# Streaming Decode Design

Two complementary streaming decode models: **pull** (caller drives) and **push** (codec drives into caller buffer).

All pixel types (`PixelDescriptor`, `PixelSlice`, `PixelSliceMut`, `PixelBuffer`,
etc.) come from `zenpixels` — the cross-crate interchange format. Codec crates
depend on `zenpixels` directly; zencodec-types uses but does not re-export them.

## Current State

### What exists

- **`DecodeRowSink`** (sink.rs) — push-based output primitive. Codec calls `demand(y, height, width, bpp)`, sink returns `(&mut [u8], stride)`. Object-safe, lending pattern, sink controls stride/alignment.
- **`CodecCapabilities`** — has `row_level_decode` and `row_level_frame_decode` flags.
- **`UnsupportedOperation`** — has `RowLevelDecode` and `RowLevelFrameDecode` variants.
- **`Decode` trait** — only `decode(self, data, preferred) -> DecodeOutput` (allocates internally).
- **`FrameDecode` trait** — only `next_frame(&mut self, preferred) -> Option<DecodeFrame>` (allocates internally).

### What's missing

- **Pull model:** `StreamingDecode` trait and `DecodeJob::streaming_decoder()` (defined in spec.md but not implemented).
- **Push model:** No trait method connects a decoder to a `DecodeRowSink`.

---

## Pull Model: `StreamingDecode`

Defined in spec.md. Caller drives the loop, pulling batches from the codec.

```rust
trait StreamingDecode {
    type Error: core::error::Error + Send + Sync + 'static;

    fn next_batch(&mut self, preferred: &[PixelDescriptor])
        -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error>;
    fn info(&self) -> &ImageInfo;
}
```

Created via `DecodeJob::streaming_decoder(self, data) -> Result<StreamDec, Error>`.

### Characteristics

- **Caller drives** — can pause, resume, drop early
- **Codec owns the buffer** — returns `PixelSlice` referencing internal memory
- **Easy to compose** — works naturally with iterators, async wrappers
- **Extra copy if caller needs to own the data** — must copy from `PixelSlice` into caller buffer

### Rejection type

Codecs that don't support streaming: `type StreamDec = ()` and `streaming_decoder()` returns `Err`. `impl StreamingDecode for ()` is the trivial rejection.

---

## Push Model: `decode_to_sink` / `next_frame_to_sink`

Codec drives the loop, pushing strips into a caller-owned `DecodeRowSink`.

### On `Decode`

```rust
fn decode_to_sink(
    self,
    data: &[u8],
    preferred: &[PixelDescriptor],
    sink: &mut dyn DecodeRowSink,
) -> Result<OutputInfo, Self::Error> {
    // Default: decode() + copy strips into sink
    let output = self.decode(data, preferred)?;
    copy_to_sink(&output, sink);
    Ok(output_info_from(&output))
}
```

### On `FrameDecode`

```rust
fn next_frame_to_sink(
    &mut self,
    preferred: &[PixelDescriptor],
    sink: &mut dyn DecodeRowSink,
) -> Result<Option<OutputInfo>, Self::Error> {
    // Default: next_frame() + copy strips into sink
    match self.next_frame(preferred)? {
        Some(frame) => {
            copy_frame_to_sink(&frame, sink);
            Ok(Some(output_info_from_frame(&frame)))
        }
        None => Ok(None),
    }
}
```

### Characteristics

- **Codec drives** — simpler codec implementation, natural strip boundaries
- **Zero-copy into caller buffer** — sink provides `&mut [u8]`, codec writes directly
- **Caller controls memory** — stride, alignment, allocation strategy all in the sink
- **Every codec works** — default fallback means all codecs "support" push streaming; native codecs override for zero-copy
- **Cannot pause mid-stream** — codec runs to completion or error

### Design Rationale

**Default implementations fall back to allocating path.** Every codec gets push streaming for free. Codecs that natively stream override with zero-copy implementations.

**Returns `OutputInfo`, not `DecodeOutput`.** Pixels went into the sink. `OutputInfo` carries dimensions, descriptor, and which hints were honored. Metadata available from `DecodeJob::probe()` / `output_info()` beforehand.

**`&mut dyn DecodeRowSink` not generic.** Object-safe for the same reason `Stop` is. Vtable cost is negligible vs. decode cost.

**`preferred` works identically.** Same format negotiation as `decode()`. Sink receives pixels in the negotiated format.

### Capabilities

No new flags. Existing `row_level_decode` and `row_level_frame_decode` cover both pull and push — if a codec can stream rows at all, it can do both. The flag means "native incremental streaming, not decode-then-copy."

---

## Pull vs. Push: When to Use Each

| Use case | Pull (`StreamingDecode`) | Push (`decode_to_sink`) |
|----------|-------------------------|------------------------|
| Decode-to-encode pipeline | Possible but extra copy | Direct — sink feeds encoder |
| HTTP chunked response | Natural — pull rows, write chunks | Works but codec drives timing |
| Display/render pipeline | Pull rows into GPU texture | Push into mmap'd texture |
| Compositing with other sources | Pull lets you interleave | Push is fire-and-forget |
| Memory-constrained decode | Codec owns buffer (small) | Sink controls buffer (flexible) |
| Simple "decode into my buffer" | Pull + copy loop | One call, zero-copy |

**Pull** is better when the caller needs control over pacing or interleaving with other work.

**Push** is better for "decode this image into my buffer" with minimal code and zero unnecessary copies.

---

## Per-Codec Streaming Characteristics

### PNG (zenpng)

- **Strip height:** 1 row (interlaced: 1 row per sub-image pass)
- **Buffering:** Zero — row decoder already streams natively via `RowDecoder`
- **TTFB:** First row available after IHDR + first IDAT scanline decompression. For a 4000x3000 image, ~0.03% of total work.
- **Pull impl:** `next_batch()` returns one decompressed/unfiltered row.
- **Push impl:** Wire `RowDecoder` to call `sink.demand()` per row.
- **Interlaced:** Adam7 passes deliver partial-resolution rows. Rows arrive out-of-order (y is non-sequential). Sink must handle random y values.

### JPEG (zenjpeg)

- **Strip height:** MCU row height (8 rows for 4:4:4, 16 rows for 4:2:0)
- **Buffering:** Zero — `ScanlineReader` already yields MCU rows
- **TTFB:** After SOS marker + first MCU row. Typically <1% of input for a large JPEG.
- **Pull impl:** `next_batch()` returns one MCU row (8 or 16 scanlines).
- **Push impl:** Wire scanline reader to `sink.demand()` per MCU row.
- **Progressive JPEG:** Must buffer full DCT coefficient matrix, then emit rows after final scan. TTFB = full decode time. Both pull and push work but aren't truly incremental.

### AVIF (zenavif)

- **Strip height:** Tile height (typically 512 pixels, configurable per-image)
- **Buffering:** One tile row (all tiles in the row decoded independently, then emitted)
- **TTFB:** After ISOBMFF container parse + first tile row decode. For grid images, 1/N of total work (N = tile rows).
- **Pull impl:** `next_batch()` returns one tile row (all horizontal tiles decoded and stitched).
- **Push impl:** `decode_grid_to_sink()` already exists. Wire to trait.
- **Non-grid AVIF:** Single tile, no streaming benefit. Falls back to decode-then-copy.
- **HEIC:** Same architecture. Tiles are independently coded HEVC, no cross-tile loop filtering.

### JXL (zenjxl)

- **Strip height:** Group height (256 pixels in VarDCT, 256 in Modular)
- **Buffering:** 2-3 group rows (EPF edge-preserving filter needs ±3 pixel border from adjacent groups)
- **TTFB:** After frame header + first 2-3 group rows. For 4000x3000 with 256px groups, ~20% of pixel work.
- **Pull impl:** `next_batch()` returns one group row after EPF is complete for that row.
- **Push impl:** Plumb `low_memory_pipeline`'s `RowBuffer` to `sink.demand()`.
- **Modular (lossless):** Groups fully independent (no EPF). Buffer 1 group row. Better TTFB.

### WebP (zenwebp)

- **VP8 (lossy):** Could theoretically stream MCU rows with 1-row look-behind for loop filter. Not implemented. Low priority.
- **VP8L (lossless):** Backward-reference entropy coding. Cannot stream.
- **Verdict:** Use default fallback (decode-then-copy) for both models. `row_level_decode` = false.

### GIF (zengif)

- **Strip height:** 1 row (LZW decompresses to scanlines)
- **Buffering:** Zero — already row-by-row
- **TTFB:** After GIF header + first LZW-decoded row
- **Pull impl:** `next_batch()` returns one scanline.
- **Push impl:** Wire existing row decode to `sink.demand()`.
- **Interlaced GIF:** Rows arrive in 4-pass order (0, 8, 16... then 4, 12, 20...). Non-sequential y values.

---

## TTFB Analysis

| Codec | Native Streaming | TTFB (first strip) | Full-Image Overhead | Verdict |
|-------|-----------------|--------------------|--------------------|---------|
| PNG (sequential) | Yes | ~0.03% of total | None | Clear win |
| PNG (interlaced) | Partial | Pass 1 of 7 | Interlace reconstruction | Moderate win |
| JPEG (sequential) | Yes | ~0.5% of total | None | Clear win |
| JPEG (progressive) | No | ~100% of total | None (buffered internally) | No TTFB benefit |
| AVIF (grid) | Yes | ~1/tile_rows of total | None | Win for large images |
| AVIF (single) | No | ~100% of total | None (fallback) | No benefit |
| JXL (VarDCT) | Yes | ~20% of total | 2-3 group row buffer | Moderate win |
| JXL (Modular) | Yes | ~8% of total | 1 group row buffer | Good win |
| WebP (lossy) | No | ~100% | None (fallback) | No benefit |
| WebP (lossless) | No | ~100% | None (fallback) | No benefit |
| GIF | Yes | ~0.03% of total | None | Clear win |

**Where streaming matters most:**

1. **Image proxies** — Start sending HTTP chunked-transfer response before full decode. Major TTFB win for PNG/JPEG/GIF, meaningful for AVIF grids.
2. **Decode-to-encode pipelines** — Encoder starts compressing rows while decoder still producing them. Push model with shared sink is ideal.
3. **Memory reduction** — Never hold full decoded image. 4000x3000 RGBA8 = 48MB. Streaming peak = strip_height * stride + codec internal buffers.
4. **Display pipelines** — Progressive rendering on screen while decoding.

**Where it doesn't help:**

- WebP (can't stream)
- Single-tile AVIF (nothing to stream incrementally)
- Progressive JPEG (must buffer all scans)
- Callers that need the full image anyway (resizing, quantization)

---

## Push Model Memory Diagram

```
Caller                    Codec
  |                         |
  |  create sink            |
  |  (owns buffer)          |
  |                         |
  |  decode_to_sink(data, preferred, &mut sink)
  |                         |
  |    <-- demand(y=0, h=8, w, bpp)
  |  return (&mut buf, stride)
  |                         |
  |    (codec writes 8 rows)|
  |                         |
  |    <-- demand(y=8, h=8, w, bpp)
  |  (caller can process    |
  |   rows 0-7 now)         |
  |  return (&mut buf, stride)
  |                         |
  |    (codec writes 8 rows)|
  |         ...             |
  |                         |
  |  <-- Ok(OutputInfo)     |
  |  (last strip written)   |
```

Sink strategies:

- **Fixed-buffer:** Single buffer, process each strip before next `demand()`. Peak = 1 strip.
- **Accumulating:** Grows a Vec. Peak = full image (what default fallback does).
- **Ring-buffer:** Two strips, ping-pong. Overlapped processing.
- **mmap:** Direct write to memory-mapped file or GPU texture.

---

## Error Handling and Cancellation

- If the codec errors mid-stream, the method returns `Err`. The sink may have partial data in the last demanded buffer — caller discards it.
- `Stop` cancellation: codec checks the stop token between strips/batches.
- Push model: the sink cannot signal errors back to the codec. Sink sets an internal flag; caller checks after return. Keeps `demand()` simple.
- Pull model: caller just stops calling `next_batch()`. Natural cancellation.

---

## What Not To Do

**Don't add `Read`-style incremental input.** Streaming *input* (feeding bytes incrementally) is a different problem from streaming *output* (emitting rows incrementally). Input streaming needs state machines and resumable parsing. The `data: &[u8]` parameter means full compressed data is available.

**Don't make the sink generic on the trait.** `&mut dyn DecodeRowSink` is right. Vtable cost is negligible vs. decode cost.

**Don't add async.** CPU-bound operations. Caller can spawn on a blocking thread.

**Don't force codecs to implement streaming natively.** Default fallbacks make both models universally available. The `row_level_decode` capability flag tells callers whether it's truly incremental.

---

## Implementation Order

1. Add `StreamingDecode` trait and `impl StreamingDecode for ()` rejection type
2. Add `type StreamDec` + `streaming_decoder()` to `DecodeJob`
3. Add `decode_to_sink()` to `Decode` with default impl
4. Add `next_frame_to_sink()` to `FrameDecode` with default impl
5. Add helpers `copy_to_sink()` / `copy_frame_to_sink()` in zencodec-types
6. Wire zenpng (easiest — RowDecoder already streams)
7. Wire zenjpeg (ScanlineReader already streams)
8. Wire zengif (already row-by-row)
9. Wire zenavif grid path (decode_grid_to_sink exists)
10. Wire zenjxl (low_memory_pipeline exists but needs plumbing)
11. WebP stays on default fallback for push, `type StreamDec = ()` for pull

---

## Expected Streaming Support Per Codec

Which codecs should implement native streaming (override the default fallback)
vs. which should rely on the decode-then-copy default.

| Codec | Pull (`StreamingDecode`) | Push (`decode_to_sink`) | Push frames (`next_frame_to_sink`) | `row_level_decode` | Priority |
|-------|-------------------------|------------------------|------------------------------------|--------------------|----------|
| **zenpng** | Native | Native | Native (APNG) | **true** | P0 — trivial |
| **zenjpeg** | Native | Native | N/A (no animation) | **true** | P0 — trivial |
| **zengif** | Native | Native | Native | **true** | P1 — easy |
| **zenavif** (grid) | Native | Native | N/A | **true** | P1 — moderate |
| **zenavif** (single) | Default (= `()`) | Default (decode+copy) | N/A | false | — |
| **zenjxl** | Native | Native | N/A | **true** | P2 — high effort |
| **zenwebp** | Default (= `()`) | Default (decode+copy) | Default (decode+copy) | false | — |

### Rationale

**zenpng (P0):** `RowDecoder` already streams one scanline at a time. Both pull and push are trivial wrappers — pull returns the row as `PixelSlice`, push writes it into `sink.demand()`. APNG frame streaming via `next_frame_to_sink` also straightforward since each frame decodes independently with the same row decoder.

**zenjpeg (P0):** `ScanlineReader` already yields MCU rows (8 or 16 scanlines). Same story as PNG — trivial wrappers. No animation support. Progressive JPEG: the streaming decoder returns the full image as a single batch after all scans are buffered — not truly incremental but still works through the interface. The `row_level_decode` flag could be conditional on whether the JPEG is sequential, but for simplicity just set it true and document the progressive caveat.

**zengif (P1):** LZW decompresses to scanlines. Row-level decode is natural. Frame streaming via `next_frame_to_sink` is valuable for large animated GIFs — each frame decoded row-by-row into the sink without allocating the full composited frame. Interlaced GIF delivers non-sequential y values; sink handles this.

**zenavif grid (P1):** `decode_grid_to_sink()` already exists. Grid tiles are independently coded AV1 frames — decode one tile row, emit those scanlines, repeat. Pull model returns tile-height strips. Moderate work to wire the existing tile pipeline to both interfaces. Single-tile AVIF has nothing to stream — default fallback is correct.

**zenjxl (P2):** `low_memory_pipeline` with `RowBuffer` already manages group-row streaming internally. The plumbing exists but isn't exposed through traits. EPF complicates things — need 2-3 group rows buffered before the first can be emitted. Modular (lossless) is simpler (groups fully independent). Highest implementation effort, but also the biggest memory win for large JXL images (256px groups = substantial internal buffering otherwise).

**zenwebp (skip):** VP8L (lossless) fundamentally cannot stream due to backward references. VP8 (lossy) could theoretically stream MCU rows but the effort isn't justified — WebP images are typically small and our decoder doesn't have the infrastructure. `type StreamDec = ()`, default fallback for push. Not worth the investment.

### What "Native" means

A codec with native streaming support:
- **Pull:** Implements `StreamingDecode` with real batches (not one giant batch = whole image)
- **Push:** Overrides `decode_to_sink` to call `sink.demand()` per strip during decode, never allocating the full image
- **Capability:** Sets `row_level_decode = true` (and `row_level_frame_decode` if applicable)

A codec on default fallback:
- **Pull:** `type StreamDec = ()`, `streaming_decoder()` returns `Err`
- **Push:** `decode_to_sink` calls `decode()` internally, then copies strips to sink — works but allocates the full image temporarily
- **Capability:** `row_level_decode = false`

---

## Open Questions

1. **Interlaced images.** PNG Adam7 and interlaced GIF deliver rows out of order. Both models see non-sequential y values. Options:
   - (a) Sink/caller handles random y — most sinks writing to a pre-allocated buffer handle this trivially
   - (b) Codec deinterlaces internally — requires full-image buffer, defeats streaming
   - (c) Capability flag `sequential_rows` — when false, y may be non-sequential

   Recommend (a) with (c) as documentation. Most sinks are pre-allocated buffers where random y is free.

2. **Strip height information.** The codec picks strip height based on internal structure. Callers benefit from knowing it ahead of time (buffer pre-allocation). Propose: `OutputInfo::strip_height() -> u32` — 0 means unknown/variable. Informational, not negotiation.

3. **Pull model buffer lifetime.** `next_batch()` returns `PixelSlice<'_>` borrowing from the streaming decoder. The borrow ends when `next_batch()` is called again. This is the same lending pattern as `DecodeRowSink::demand()`. Callers must copy or process before the next call.
