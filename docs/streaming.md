# Streaming Decode Design

## Current State

The building blocks exist but aren't connected:

- **`DecodeRowSink`** (sink.rs) — the output primitive. Codec calls `demand(y, height, width, bpp)`, sink returns `(&mut [u8], stride)`. Object-safe, lending pattern, sink controls stride/alignment.
- **`CodecCapabilities`** — has `row_level_decode` and `row_level_frame_decode` flags.
- **`UnsupportedOperation`** — has `RowLevelDecode` and `RowLevelFrameDecode` variants.
- **`Decode` trait** — only has `decode(self, data, preferred) -> DecodeOutput` (allocates internally).
- **`FrameDecode` trait** — only has `next_frame(&mut self, preferred) -> Option<DecodeFrame>` (allocates internally).

The gap: no trait method connects a decoder to a `DecodeRowSink`. The sink.rs doc comment references `job.decoder().decode_rows(data, &mut sink)` but that method doesn't exist on the trait.

## Proposed Trait Additions

### On `Decode`

```rust
pub trait Decode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    // Existing
    fn decode(self, data: &[u8], preferred: &[PixelDescriptor]) -> Result<DecodeOutput, Self::Error>;

    // New: streaming decode into caller-owned sink
    fn decode_rows(
        self,
        data: &[u8],
        preferred: &[PixelDescriptor],
        sink: &mut dyn DecodeRowSink,
    ) -> Result<OutputInfo, Self::Error> {
        // Default: fall back to decode() + copy into sink
        let output = self.decode(data, preferred)?;
        copy_to_sink(&output, sink);
        Ok(output.info().into())
    }
}
```

### On `FrameDecode`

```rust
pub trait FrameDecode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    // Existing
    fn next_frame(&mut self, preferred: &[PixelDescriptor]) -> Result<Option<DecodeFrame>, Self::Error>;

    // New: streaming frame decode into caller-owned sink
    fn next_frame_rows(
        &mut self,
        preferred: &[PixelDescriptor],
        sink: &mut dyn DecodeRowSink,
    ) -> Result<Option<OutputInfo>, Self::Error> {
        // Default: fall back to next_frame() + copy into sink
        match self.next_frame(preferred)? {
            Some(frame) => {
                copy_frame_to_sink(&frame, sink);
                Ok(Some(frame.info().into()))
            }
            None => Ok(None),
        }
    }
}
```

### Design Rationale

**Default implementations fall back to allocating path.** Every codec gets streaming decode for free — callers can always use `decode_rows()`. Codecs that natively stream override with zero-copy implementations. This matches the encode side where `push_rows()` and `encode_from()` have default error implementations, but the decode side is more useful with a working default.

**Returns `OutputInfo`, not `DecodeOutput`.** The pixels went into the sink, so there's no `PixelData` to return. `OutputInfo` carries dimensions, descriptor, and which hints were honored. Metadata (ICC, EXIF) is available from `DecodeJob::probe()` or `output_info()` — the caller already has it before calling `decode_rows()`.

**`&mut dyn DecodeRowSink` not generic.** Object-safe for the same reason `Stop` is — codecs shouldn't be generic over sink types. The vtable cost is negligible vs. decode cost.

**`preferred` works identically.** Same format negotiation as `decode()`. The sink receives pixels in the negotiated format.

## Per-Codec Streaming Characteristics

### PNG (zenpng)

- **Strip height:** 1 row (interlaced: 1 row per sub-image pass)
- **Buffering:** Zero — row decoder already streams natively via `RowDecoder`
- **TTFB:** First row available after IHDR + first IDAT scanline decompression. For a 4000x3000 image, ~0.03% of total work.
- **Implementation:** Wire `RowDecoder` to call `sink.demand()` per row. Nearly zero work.
- **Interlaced:** Adam7 passes deliver partial-resolution rows. The sink sees rows in pass order (1/8, 1/8, 1/4, ...). Caller must handle interlace reconstruction or request non-interlaced output.

### JPEG (zenjpeg)

- **Strip height:** MCU row height (8 rows for 4:4:4, 16 rows for 4:2:0)
- **Buffering:** Zero — `ScanlineReader` already yields MCU rows
- **TTFB:** After SOS marker + first MCU row. Typically <1% of input for a large JPEG.
- **Implementation:** Wire scanline reader output to `sink.demand()` per MCU row. Nearly zero work.
- **Progressive JPEG:** Each scan refines the entire image. True row streaming only works on sequential JPEGs. Progressive requires buffering the full DCT coefficient matrix, then emitting rows after the final scan. `decode_rows()` can still work (emit rows at the end) but TTFB = full decode time. Mark in `OutputInfo` whether streaming was truly incremental.

### AVIF (zenavif)

- **Strip height:** Tile height (typically 512 pixels, configurable per-image)
- **Buffering:** One tile row (all tiles in the row decoded independently, then emitted)
- **TTFB:** After parsing the ISOBMFF container + decoding the first tile row. For a grid image, this is 1/N of the total pixel decode work (N = tile rows).
- **Implementation:** `decode_grid_to_sink()` already exists in zenavif. Wire it to the trait. Moderate work — need to handle the AV1 OBU parsing per tile.
- **Non-grid AVIF:** Single tile, no streaming benefit. Falls back to decode-then-copy (the default impl is fine).
- **HEIC:** Same architecture. Tiles are independently coded HEVC, no cross-tile loop filtering. Future zenavif extension.

### JXL (zenjxl)

- **Strip height:** Group height (256 pixels in VarDCT, 256 in Modular)
- **Buffering:** 2-3 group rows (EPF edge-preserving filter needs ±3 pixel border from adjacent groups)
- **TTFB:** After parsing frame header + decoding first 2-3 group rows. For a 4000x3000 image with 256px groups, that's 2-3/12 = ~20% of pixel work. Meaningful but not dramatic.
- **Implementation:** jxl-rs already has `low_memory_pipeline` that manages `RowBuffer` for group-row streaming. The plumbing exists but isn't exposed through zencodec-types traits. Moderate-to-high work.
- **Modular (lossless):** Groups are fully independent (no EPF). Buffer 1 group row. Better TTFB than VarDCT.

### WebP (zenwebp)

- **VP8 (lossy):** Loop filter has row dependencies but could theoretically stream MCU rows with 1-row look-behind. Not implemented in any WebP decoder we use. Low priority — WebP lossy images are typically small.
- **VP8L (lossless):** Backward-reference entropy coding. Cannot stream — the entire image must be decoded to resolve references. No streaming possible.
- **Verdict:** Use the default fallback (decode-then-copy). Not worth implementing native streaming.

### GIF (zengif)

- **Strip height:** 1 row (LZW decompresses to a raster scanline)
- **Buffering:** Zero — already decoded row-by-row
- **TTFB:** After GIF header + first row of first frame LZW decompression
- **Implementation:** Wire existing row-by-row decode to sink. Low work.
- **Interlaced GIF:** Rows arrive in 4-pass interlace order (0, 8, 16... then 4, 12, 20... then 2, 6, 10... then 1, 3, 5...). The `y` parameter in `demand()` handles this — the sink sees non-sequential y values.

## TTFB Analysis

| Codec | Native Streaming | TTFB (first strip) | Full-Image Overhead | Verdict |
|-------|-----------------|--------------------|--------------------|---------|
| PNG (sequential) | Yes | ~0.03% of total | None | Clear win |
| PNG (interlaced) | Partial | Pass 1 of 7 | Interlace reconstruction | Moderate win (progressive display) |
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

1. **Image proxies** — Start sending HTTP chunked-transfer response before the full image is decoded. For PNG/JPEG/GIF this is a major TTFB win. For AVIF grid images, meaningful. For WebP, irrelevant.

2. **Decode-to-encode pipelines** — The encoder can start compressing rows while the decoder is still producing them. PNG-to-PNG and JPEG-to-JPEG pipelines benefit enormously (encoder push_rows + decoder decode_rows with a shared sink).

3. **Memory reduction** — The caller never holds the full decoded image. For a 4000x3000 RGBA8 image, that's 48MB. With streaming, peak memory drops to strip_height * stride bytes in the sink, plus whatever the codec buffers internally.

4. **Display pipelines** — Progressive rendering on screen while decoding.

**Where it doesn't help:**

- WebP (can't stream)
- Single-tile AVIF (nothing to stream incrementally)
- Progressive JPEG (must buffer all scans)
- Any codec where the caller needs the full image anyway (resizing, quantization, format conversion that needs spatial context)

## Memory Model

```
Caller                    Codec
  |                         |
  |  create sink            |
  |  (owns buffer)          |
  |                         |
  |  decode_rows(data, preferred, &mut sink)
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

The sink has full control over memory:

- **Fixed-buffer sink:** Single buffer, processes each strip before returning it for the next. Peak memory = 1 strip.
- **Accumulating sink:** Grows a Vec, appending each strip. Peak memory = full image. (This is what the default fallback does.)
- **Ring-buffer sink:** Two strip buffers, ping-pong. Allows overlapped processing.
- **mmap sink:** Writes directly into a memory-mapped file or GPU texture.

## Error Handling and Cancellation

- If the codec encounters an error mid-stream, `decode_rows()` returns `Err`. The sink may have partially-written data in the last demanded buffer. The caller should discard it.
- `Stop` cancellation works the same as `decode()` — the codec checks the stop token between strips and returns a cancellation error.
- The sink cannot signal errors back to the codec. If the sink has a problem (e.g., mmap failure), it should set an internal flag and the caller checks after `decode_rows()` returns. This keeps the trait simple and avoids `Result` return from `demand()`.

## What Not To Do

**Don't add `Read`-style incremental input.** Streaming *input* (feeding bytes incrementally to the decoder) is a different problem from streaming *output* (emitting rows incrementally). Input streaming requires fundamentally different decoder architecture (state machines, resumable parsing). The existing `data: &[u8]` parameter means the full compressed data is available. This is fine for our use case — image proxies typically have the full response body buffered.

**Don't make the sink generic on the trait.** `&mut dyn DecodeRowSink` is right. The decode cost dominates the vtable dispatch.

**Don't add async.** These are CPU-bound operations. Async would add complexity for zero benefit. The caller can spawn the decode on a blocking thread if needed.

**Don't force codecs to implement it.** The default fallback means every codec "supports" streaming. Codecs opt into native streaming for performance. The `row_level_decode` capability flag tells callers whether it's truly incremental.

## Implementation Order

1. Add `decode_rows()` to `Decode` with default impl (decode + copy to sink)
2. Add `next_frame_rows()` to `FrameDecode` with default impl
3. Add helper `copy_to_sink()` / `copy_frame_to_sink()` in zencodec-types
4. Wire zenpng (easiest — RowDecoder already streams)
5. Wire zenjpeg (ScanlineReader already streams)
6. Wire zengif (already row-by-row)
7. Wire zenavif grid path (decode_grid_to_sink exists)
8. Wire zenjxl (low_memory_pipeline exists but needs plumbing)
9. WebP stays on default fallback

## Open Questions

1. **Should `decode_rows` consume `self` or take `&mut self`?** Currently `decode()` consumes self (single-shot). For consistency, `decode_rows` should too. But if we want to decode multiple frames with row streaming, `FrameDecode::next_frame_rows()` needs `&mut self` (already the case for `next_frame`).

2. **Interlaced images.** PNG Adam7 and interlaced GIF deliver rows out of order. Options:
   - (a) Let the sink receive out-of-order `y` values — sink must handle random access
   - (b) Codec deinterlaces internally, sink always gets sequential rows — but this requires a full-image buffer in the codec, defeating the point
   - (c) Capability flag `sequential_rows` — when false, sink must handle random y

   Recommend (c). Most sinks writing to a pre-allocated buffer handle random y trivially.

3. **Strip height negotiation.** The codec picks strip height based on its internal structure (MCU rows, tile height, group height). Should the caller be able to request a preferred strip height? The `Encoder` trait has `preferred_strip_height()` for encode. We could add a similar query for decode.

   Propose: `OutputInfo::strip_height() -> u32` — tells the caller what strip height to expect (0 = unknown/variable). Not a negotiation, just information. The codec emits whatever strips it naturally produces.

4. **Metadata delivery timing.** With `decode()`, metadata comes in `DecodeOutput`. With `decode_rows()`, the caller needs metadata *before* calling decode_rows (to set up the sink). This is already solved: `DecodeJob::probe()` and `output_info()` provide metadata and dimensions before decode starts. No change needed.
