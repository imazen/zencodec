# zencodec-types Design Notes

Design discussion notes from Feb 2026. Covers the API review, decisions made, and future direction for HDR, gain maps, animation, streaming, and color management.

## The API review: 8 gaps identified

Before publishing v0.1.0, we reviewed the full trait/type surface. The traits are the hardest part — `#[non_exhaustive]` on structs and enums lets us add fields/variants in minor versions, but adding required trait methods is a semver break. So we needed to get the trait surface right before the initial publish.

Eight gaps were found:

1. **No animation in the traits.** `EncodingJob` encodes one frame. `DecodingJob::decode` returns one `DecodeOutput`. A `DecodeFrame` type existed but nothing produced or consumed it.

2. **No bit depth or channel count on `ImageInfo`.** A 10-bit AVIF and an 8-bit JPEG return identical `ImageInfo`. Callers can't know native precision without inspecting the `PixelData` variant after decode.

3. **No CICP or color space info.** `ImageInfo` carries raw ICC bytes but has no structured color space representation. No way to express sRGB vs PQ vs HLG transfer, BT.709 vs BT.2020 gamut, or whether decode output is gamma-encoded or linear.

4. **No HDR metadata.** `ImageMetadata` carries ICC/EXIF/XMP but not content light level (CEA-861.3) or mastering display color volume (SMPTE ST 2086). These are separate from ICC and needed for AVIF, JXL, and HEIF HDR content.

5. **No streaming/incremental decode.** All decode methods take `data: &[u8]` — the entire file. Known limitation for v0.1, noted for future work.

6. **No 16-bit encode paths.** `EncodingJob` has rgb8/rgba8/gray8/f32 but no rgb16/rgba16/gray16. AVIF and JXL both accept 16-bit input natively.

7. **GrayAlpha missing entirely.** The README listed `GrayAlpha8`/`GrayAlpha16` variants but they didn't exist in the code. No encode methods for grayscale+alpha either.

8. **Capabilities incomplete.** `CodecCapabilities` couldn't express animation support, 16-bit native handling, lossless mode, or HDR metadata support.

All eight were fixed on branch `api-review` (commit `42b2f19`). 101 tests pass, clippy clean.

## Decision: no default implementations

All new trait methods were added as **required**, not default-impl'd. The reasoning:

- Pre-v0.1.0, there's no semver concern — we haven't published yet.
- Required methods force every codec implementor to consciously handle every capability rather than silently inheriting a no-op.
- After v0.1.0, adding required methods is a semver break. Adding default-impl'd methods isn't, but defaults hide missing functionality. We'd rather break loudly.
- The semver escape hatch for future additions: default implementations that return `Err(Unsupported)` or similar. But we didn't need that yet.

## CICP design

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cicp {
    pub color_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub full_range: bool,
}
```

Uses raw `u8` values matching ITU-T H.273 directly rather than enums. Enums would require updating for every new registered value — there are dozens of primaries and transfer characteristics, with more added periodically. Raw codes are forward-compatible.

Three constants cover the common cases:

- `Cicp::SRGB` — primaries 1, transfer 13, matrix 6, full range. The web default.
- `Cicp::BT2100_PQ` — primaries 9, transfer 16, matrix 9, full range. HDR with Perceptual Quantizer.
- `Cicp::BT2100_HLG` — primaries 9, transfer 18, matrix 9, full range. HDR with Hybrid Log-Gamma.

`Copy + Hash` enables use as map keys and cheap passing.

### Relationship to ICC

CICP and ICC serve different purposes and coexist:

- **CICP** gives quick machine-readable identification — "this is BT.2020 PQ" in 4 bytes.
- **ICC** gives the full color profile for accurate rendering, but requires parsing a binary blob.
- A file can have both. AVIF requires CICP in the container; ICC is optional in the OBU.
- For fast-path decisions (is this HDR? which transfer function?), check CICP. For color-managed rendering, use ICC.

Both are carried on `ImageInfo` and `ImageMetadata`. CICP and HDR metadata types are `Copy`, so they don't add borrowing complexity to `ImageMetadata<'a>` (which borrows ICC/EXIF/XMP as `&'a [u8]`).

## HDR metadata

Two structs following their respective standards:

- **`ContentLightLevel`** (CEA-861.3) — `max_cll` (maximum content light level in nits) and `max_fall` (maximum frame-average light level in nits). Describes the content's brightness characteristics.
- **`MasteringDisplay`** (SMPTE ST 2086) — display primaries as CIE 1931 xy coordinates, white point, and min/max luminance. Describes the environment where the content was graded.

These are deliberately separate from ICC. ICC describes color space transforms; HDR metadata describes the mastering environment and content characteristics. Both are needed for correct HDR rendering.

## Bit depth and channel count

Added to `ImageInfo`:

```rust
pub bit_depth: Option<u8>,      // 8, 10, 12, 16, 32
pub channel_count: Option<u8>,  // 1, 2, 3, 4
```

`Option` because some codecs may not report these in header-only probes. After decode, the `PixelData` variant tells you the actual format. These fields let callers make decisions before decoding (e.g., allocate a 16-bit buffer if `bit_depth` is 10+).

## GrayAlpha

We define our own `GrayAlpha<T>` struct in zencodec-types rather than using `rgb::alt::GrayAlpha`. The `rgb` crate's type is a tuple struct with accessor methods that may break — our own type has stable named fields:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct GrayAlpha<T> {
    pub v: T,
    pub a: T,
}
```

Three variants added to `PixelData`:

- `GrayAlpha8(ImgVec<GrayAlpha<u8>>)`
- `GrayAlpha16(ImgVec<GrayAlpha<u16>>)`
- `GrayAlphaF32(ImgVec<GrayAlpha<f32>>)`

Every match arm in `PixelData` was updated — `width()`, `height()`, `has_alpha()`, all conversion methods, `Debug`. Three encode methods added to `EncodingJob`.

## 16-bit transfer function conventions

### The problem

Each codec interpreted u16 differently before we defined a convention:

| Codec | u16 meaning | Range | Transfer function | Status |
|-------|-------------|-------|-------------------|--------|
| zenavif decode | gamma-encoded | ~~0–1023~~ → 0–65535 | sRGB/PQ/HLG (metadata) | ✅ Fixed |
| zenavif encode | gamma-encoded | ~~expects 0–1023~~ → accepts 0–65535 | scales to 10-bit internally | ✅ Fixed |
| zenjpeg | **linear** (`Rgb16Linear`) | 0–65535 | linearizes internally | ❌ Needs change |
| zenwebp/zengif | no u16 path | n/a | f32 linear only | ✅ No change needed |

### What AV1 actually stores

AV1 bitstream values are OETF-encoded (nonlinear). For TC=13 (sRGB), 10-bit values are sRGB-gamma-encoded — same curve as 8-bit, just more precision. For TC=16 (PQ), they're PQ-encoded. The AV1 decoder (rav1d) outputs raw values; no transfer function is applied or removed. libavif does the same — CICP is metadata for the caller.

### What libavif / PNG / image-rs do

All output gamma-encoded u16, scaled to 0–65535. No linearization. PNG 16-bit is explicitly sRGB gamma-encoded when the sRGB chunk is present. 18% gray at sRGB = ~30235, not ~11796.

### The convention (decided)

**u16 = gamma-encoded (native transfer function), full 0–65535 range.**

This matches libavif, PNG, image-rs, stb_image. The `transfer_characteristics` in CICP tells the caller what domain the values are in.

**f32 = linear light.** Already correct everywhere.

### What each codec needs to change

1. ✅ **zenavif decode**: Scaled 10-bit output to full u16 via LSB replication: `(v << 6) | (v >> 4)`. Removed dead `yuv_convert_libyuv_16bit.rs` (had 8-bit bottleneck, was never called). Also fixed latent `unpremultiply16` bug where alpha was 10-bit but divisor was 0xFFFF.

2. ✅ **zenavif encode**: Accepts 0–65535, scales to 10-bit via `((v + 32) >> 6).min(1023)`. All 4 encode functions (rgb16, rgba16, animation_rgb16, animation_rgba16) updated.

3. ❌ **zenjpeg**: Its `encode_rgb16` implementation currently accepts linear u16 via `Rgb16Linear`. It should accept sRGB gamma u16 (0–65535) per the convention, and linearize internally before its JPEG pipeline. The `linear-srgb` crate's `LinearTable16` (256KB LUT, ~450-820 Melem/s) is the right tool. The internal `Rgb16Linear` type becomes an implementation detail — the trait method semantics override it.

4. ✅ **zenwebp/zengif**: No changes needed. They don't have u16 paths, and their f32 paths are already linear.

### Performance of linearization (for codecs that need it internally)

A 65536-entry u16→f32 LUT fits in 256KB (L2 cache). The `linear-srgb` crate already provides this via `LinearTable16`. Throughput: ~450-820 Melem/s. For a 4K image (~25M channel values), linearization takes ~30-55ms.

For PQ content, a LUT is essential — the PQ EOTF requires two `pow()` operations per value (~200 cycles). A 1024-entry LUT (10-bit) or 4096-entry LUT (12-bit) eliminates this.

## 16-bit encode paths

Three methods added to `EncodingJob`:

```rust
fn encode_rgb16(self, img: ImgRef<'_, Rgb<u16>>) -> Result<EncodeOutput, Self::Error>;
fn encode_rgba16(self, img: ImgRef<'_, Rgba<u16>>) -> Result<EncodeOutput, Self::Error>;
fn encode_gray16(self, img: ImgRef<'_, Gray<u16>>) -> Result<EncodeOutput, Self::Error>;
```

Input is gamma-encoded (native transfer function), 0–65535 range. Codecs without native 16-bit support dither or truncate.

## Capability flags

Seven flags added to `CodecCapabilities`:

| Flag | Meaning |
|------|---------|
| `encode_animation` | Can encode multi-frame animation |
| `decode_animation` | Can decode multi-frame animation |
| `native_16bit` | Handles 16-bit data without downsampling to 8 |
| `lossless` | Supports mathematically lossless encoding |
| `hdr` | Supports HDR metadata (CICP, CLLI, MDCV) |
| `encode_cicp` | Preserves CICP through encode |
| `decode_cicp` | Extracts CICP on decode |

Encode and decode flags are separate because asymmetry is common — a codec might decode animations but not encode them. All flags follow the existing const builder pattern.

## Animation API

### EncodeFrame

```rust
pub struct EncodeFrame<'a, Pixel> {
    pub image: ImgRef<'a, Pixel>,
    pub duration_ms: u32,
}
```

Borrows the pixel data. `duration_ms` is per-frame delay. The generic `Pixel` parameter means `EncodeFrame<'_, Rgb<u8>>` and `EncodeFrame<'_, Rgba<u8>>` are distinct types — no runtime pixel format confusion.

### Trait methods

```rust
// EncodingJob
fn encode_animation_rgb8(self, frames: &[EncodeFrame<'_, Rgb<u8>>]) -> Result<EncodeOutput, Self::Error>;
fn encode_animation_rgba8(self, frames: &[EncodeFrame<'_, Rgba<u8>>]) -> Result<EncodeOutput, Self::Error>;

// DecodingJob
fn decode_animation(self, data: &[u8]) -> Result<Vec<DecodeFrame>, Self::Error>;
```

`decode_animation` returns `Vec<DecodeFrame>` — the existing `DecodeFrame` type already has `delay_ms` and `index` fields. This buffers all frames in memory, which is the known limitation. Streaming animation decode is future work.

### What this doesn't cover

- Streaming animation decode (frame-by-frame iterator)
- Animation encoding from a streaming source
- Variable frame dimensions (all frames assumed same size)
- Animation-specific metadata (loop count, etc.)

These are punted to post-v0.1.0. The current API handles the common case: decode all frames, encode a sequence of same-sized frames.

## Current state of HDR support across zen*

The parsing layer is solid. The pixel math doesn't exist.

**What we have:**

- **zenavif-parse** — Full ISO 21496-1 gain map parsing: `GainMapMetadata`, `GainMapChannel`, `parse_tone_map_image()`, gain map AV1 bitstream extraction, `tmap` item extraction.
- **zenjpeg** — UltraHDR gain map extraction from MPF secondary images. `GainMapHandling::Decode` decodes the embedded JPEG gain map to pixels.
- **zencodec-types** — `Cicp`, `ContentLightLevel`, `MasteringDisplay`. These flow through encode/decode.
- **jxl-encoder-rs** — `TransferFunction` enum with `Pq`, `Hlg`, `Srgb`, `Linear`, `Bt709`, `Dci` variants. `intensity_target`/`min_nits` fields. But `tone_mapping` is hardcoded as `all_default` — only 255-nit SDR. No EOTF/OETF curve math.
- **linear-srgb** — SIMD-accelerated `srgb_to_linear`/`linear_to_srgb`. No PQ, no HLG.

**What's missing:**

- PQ (ST 2084) / HLG (ARIB STD-B67) transfer function implementations
- Gain map application (ISO 21496-1 math: SDR + gain map -> HDR)
- Gain map computation (HDR/SDR pair -> gain map)
- Gamut mapping (BT.709 <-> BT.2020 <-> Display P3)
- Tone mapping operators (HDR -> SDR display adaptation)
- JXL non-default tone mapping bundle

## Three HDR workflows

1. **Gain map preservation** (JPEG UltraHDR <-> AVIF <-> JXL): Decode SDR base + gain map + metadata from one format, encode into another. No pixel math — just carrying data across. The easy win.

2. **HDR reconstruction** (SDR + gain map -> HDR pixels): Apply the ISO 21496-1 formula. Needs PQ EOTF at minimum. The formulas are small and well-specified.

3. **HDR->SDR+GainMap creation** (full HDR -> SDR base + computed gain map): Tone map to SDR, derive gain map as inverse. Hardest problem — needs a subjective tone mapping operator. Defer this.

## Proposed gain map types

### GainMapMetadata

zenavif-parse and zenjpeg both parse ISO 21496-1 into their own types. A shared type avoids making transcoders depend on both:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rational { pub n: i32, pub d: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct URational { pub n: u32, pub d: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GainMapChannel {
    pub gain_map_min: Rational,
    pub gain_map_max: Rational,
    pub gamma: URational,
    pub base_offset: Rational,
    pub alternate_offset: Rational,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GainMapMetadata {
    pub is_multichannel: bool,
    pub use_base_colour_space: bool,
    pub base_hdr_headroom: URational,
    pub alternate_hdr_headroom: URational,
    pub channels: [GainMapChannel; 3],
}
```

### GainMapImage

The dual-layer interchange type:

```rust
pub struct GainMapImage {
    pub base: PixelData,
    pub gain_map: PixelData,
    pub metadata: GainMapMetadata,
    pub base_cicp: Cicp,
    pub alternate_cicp: Cicp,
    pub content_light_level: Option<ContentLightLevel>,
    pub mastering_display: Option<MasteringDisplay>,
}
```

### Trait methods for gain map round-tripping

```rust
// DecodingJob
fn decode_gain_map(self, data: &[u8]) -> Result<Option<GainMapImage>, Self::Error>;

// EncodingJob
fn encode_gain_map(self, image: &GainMapImage) -> Result<EncodeOutput, Self::Error>;
```

Codecs that don't support gain maps return `Ok(None)` / error. A `gain_map: bool` flag on `CodecCapabilities` tells callers upfront.

### Transfer function math

Extend `linear-srgb`:

```rust
pub trait TransferFn {
    fn to_linear(v: f32) -> f32;       // EOTF
    fn from_linear(v: f32) -> f32;     // inverse EOTF
    fn to_linear_slice(buf: &mut [f32]);
    fn from_linear_slice(buf: &mut [f32]);
}

pub struct Srgb;    // already exists
pub struct Pq;      // ST 2084, [0,1] -> [0, 10000] nits
pub struct Hlg;     // ARIB STD-B67
pub struct Bt709;
```

### Gain map application

The ISO 21496-1 formula:
```
hdr_pixel = (sdr_pixel + offset) * pow(gain_map_pixel, mix(headroom)) + offset
```

```rust
pub fn apply_gain_map(
    base: &PixelData,
    gain_map: &PixelData,
    metadata: &GainMapMetadata,
    target_headroom: f32,
) -> PixelData;

pub fn compute_gain_map(
    sdr: &PixelData,
    hdr: &PixelData,
    base_cicp: Cicp,
    hdr_cicp: Cicp,
) -> (PixelData, GainMapMetadata);
```

### Gamut mapping

Primaries conversion is a 3x3 matrix multiply in linear light:

```rust
pub fn convert_primaries(pixels: &mut [f32], from: ColorPrimaries, to: ColorPrimaries);
```

## The streaming problem

The whole-image API buffers everything. An 8K HDR image with gain map needs ~1GB just to hold pixels. Not acceptable for production pipelines.

### Why single rows don't work

- JPEG encodes in MCU rows (8 or 16 lines)
- AV1 works on superblocks (64x64)
- JXL works on groups (256x256)
- Resize kernels need N neighbor rows (lanczos3 = 6)
- Chroma upsampling for 4:2:0 needs row pairs
- SIMD benefits from processing multiple rows

### Strips: batches of rows

The natural processing unit is a **strip** — a horizontal band of rows. Each processing stage declares how many rows it needs; the pipeline picks a chunk size that works for everyone.

```rust
pub trait StripReader {
    type Error;
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn channels(&self) -> u8;
    fn preferred_strip_height(&self) -> u32;
    fn read_rows(&mut self, buf: &mut [f32], max_rows: u32) -> Result<u32, Self::Error>;
}

pub trait StripWriter {
    type Error;
    fn preferred_strip_height(&self) -> u32;
    fn write_rows(&mut self, rows: &[f32], num_rows: u32) -> Result<(), Self::Error>;
    fn finish(self) -> Result<EncodeOutput, Self::Error>;
}
```

Pointwise processing functions take `&mut [f32]` of any length — they don't care about row boundaries:

```rust
pub fn pq_to_linear(strip: &mut [f32]);
pub fn linear_to_pq(strip: &mut [f32]);
pub fn srgb_to_linear(strip: &mut [f32]);
pub fn convert_primaries(strip: &mut [f32], channels: u8, from: Primaries, to: Primaries);
```

Pipeline composition — caller negotiates strip height:

```rust
let strip_h = lcm(decoder.preferred_strip_height(), encoder.preferred_strip_height()).min(64);
let row_len = width as usize * channels as usize;
let mut strip = vec![0f32; row_len * strip_h as usize];
let mut out_strip = vec![0f32; row_len * strip_h as usize];

while y < height {
    let n = decoder.read_rows(&mut strip, strip_h)?;
    if n == 0 { break; }
    srgb_to_linear(&mut strip[..row_len * n as usize]);
    convert_primaries(&mut strip[..], channels, Primaries::Bt709, Primaries::Bt2020);
    gain_map.apply_strip(&strip, &mut out_strip, y, n, &mut gm_reader)?;
    linear_to_pq(&mut out_strip[..row_len * n as usize]);
    encoder.write_rows(&out_strip[..row_len * n as usize], n)?;
    y += n;
}
```

Peak memory for 8K RGBA with 64-row strips: ~16MB instead of 800MB.

### Stages that need row context

Gain map application with resolution mismatch needs a 2-row sliding window for bilinear interpolation of the lower-resolution gain map:

```rust
pub struct GainMapApplicator {
    params: GainMapParams,
    base_width: u32, base_height: u32,
    gm_width: u32, gm_height: u32,
    x_lut: Vec<(u32, f32)>,
    gm_row_buf: [Vec<f32>; 2],
    gm_rows_loaded: (u32, u32),
}
```

Resizing needs a sliding window matching the kernel radius:

```rust
pub struct StripResizer {
    kernel: ResizeKernel,
    window: Vec<Vec<f32>>,     // kernel_radius * 2 rows
    window_start_y: u32,
}
```

## zenjpeg's delayed row pattern

zenjpeg already implements streaming decode with multi-row context. This is the real-world model for how delayed output works.

### StripProcessor

Manages MCU-row-sized strips with explicit lookbehind, lookahead, and deferred output:

```rust
pub(super) struct StripProcessor {
    // Current MCU row buffers
    y_strip: Vec<i16>,
    cb_strip: Vec<i16>,
    cr_strip: Vec<i16>,
    cb_upsampled: Vec<i16>,
    cr_upsampled: Vec<i16>,

    // Lookbehind: previous MCU's last chroma row
    prev_cb_row: Vec<i16>,
    prev_cr_row: Vec<i16>,
    has_prev_context: bool,

    // Lookahead: next MCU's first chroma row
    next_cb_row: Vec<i16>,
    next_cr_row: Vec<i16>,
    has_next_context: bool,

    // Deferred output: bottom row corrected with proper vertical context
    deferred_y_row: Vec<i16>,
    deferred_cb_row: Vec<i16>,
    deferred_cr_row: Vec<i16>,
    has_deferred_bottom: bool,
}
```

The deferred row buffers delay output by one MCU row. The previous MCU's bottom row can't be finalized until the next MCU's first chroma row is decoded — that row is the vertical neighbor for the triangle filter. So it gets stored in `deferred_*_row` and transparently substituted when the caller reads that position.

### Algorithm

1. **Decode current MCU row** — IDCT into y/cb/cr strip buffers.
2. **Fix top boundary** — If `has_prev_context`, recompute output row 0 using `prev_cb_row`/`prev_cr_row` instead of edge-clamped duplicates.
3. **Save lookbehind** — Copy current MCU's last chroma row into `prev_cb_row`/`prev_cr_row`.
4. **Fix bottom boundary** — Pre-decode next MCU's first chroma row. Recompute current MCU's last row using that lookahead. Store in `deferred_*_row`.
5. **Transparent substitution** — `row_planes()` returns deferred rows when the caller reads the last row position.

### Triangle filter kernel

```rust
// Separable 3:1 interpolation:
// Horizontal: (3*curr + neighbor) / 4
// Vertical:   (3*curr + neighbor) / 4
// Combined:   (9*curr + 3*h + 3*v + hv + 8) >> 4
output[out_x]     = ((9 * curr + 3 * left  + 3 * v_curr + v_left  + 8) >> 4) as i16;
output[out_x + 1] = ((9 * curr + 3 * right + 3 * v_curr + v_right + 8) >> 4) as i16;
```

The vertical neighbor is passed explicitly — the entire lookbehind/lookahead machinery exists to provide that neighbor correctly across MCU boundaries.

### ScanlineReader

```rust
pub struct ScanlineReader<'a> {
    strip: StripProcessor,
    current_row: usize,
    current_mcu_row: usize,
    row_in_mcu: usize,
    mcu_row_decoded: bool,
    next_mcu_preloaded: bool,
}
```

Caller requests rows via `read_rows_rgb8()`. Rows come from current MCU with deferred substitution at the bottom. When MCU is exhausted: decode next MCU, compute deferred bottom for previous, advance.

### Encode side: double-buffered pending blocks

AQ (adaptive quantization) runs on strip N but applies to strip N-1. `PendingBuffers` swaps between two buffers:

```rust
struct PendingBuffers {
    y: [Vec<Block8x8f>; 2],
    cb: [Vec<Block8x8f>; 2],
    cr: [Vec<Block8x8f>; 2],
    current: bool,
}
```

1. Process strip N -> DCT -> store in current buffer
2. AQ runs on strip N, returns quantization strengths
3. Quantize previous buffer using those strengths
4. Swap buffers

### Memory model

4K image, 4:2:0 subsampling, streaming decode: ~500 KB total.

- Y strip: 128 KB (4096 x 16 x 2 bytes)
- Cb/Cr strip: 64 KB
- Cb/Cr upsampled: 256 KB
- prev/next/deferred rows: ~40 KB

### Design insights

1. **Separation of concerns** — IDCT fills strip buffers; upsampling is a separate phase.
2. **Lazy fixup** — Row corrections only happen when context is available.
3. **Transparent API** — Caller doesn't know about deferred rows.
4. **Zero-copy** — Row slices are direct references into strip buffers.
5. **SIMD-friendly** — Strides aligned to 32-pixel boundaries.
6. **No trait abstraction** — All concrete structs within the codec.

## Where streaming belongs (and doesn't)

The zenjpeg pattern argues against putting strip-based streaming traits in zencodec-types.

**Why not here:**

- Strip sizes are codec-determined. JPEG uses 8/16-row MCUs. AVIF uses tiles (64x64+). PNG is row-by-row. GIF is frame-by-frame. There's no universal strip height.
- Delay semantics are codec-specific. JPEG defers 1 row for chroma. A resizer defers N rows for its kernel. These are implementation details, not trait contracts.
- The pipeline composition problem is a separate crate's job.

**What zencodec-types should provide:**

- Data types that flow between stages: `PixelData`, `EncodeFrame`, `DecodeFrame`, `ImageInfo`
- Metadata types: `Cicp`, `ContentLightLevel`, `MasteringDisplay`, `GainMapMetadata`
- Codec capability declarations
- The Config -> Job -> Output pattern for whole-image encode/decode

**What belongs elsewhere:**

| Component | Crate | Notes |
|-----------|-------|-------|
| `GainMapMetadata`, `GainMapChannel`, `Rational` | zencodec-types | Format-agnostic interchange types |
| `GainMapImage` | zencodec-types | Dual-layer interchange |
| `decode_gain_map` / `encode_gain_map` | zencodec-types traits | Codec contract |
| PQ/HLG EOTF/OETF | linear-srgb | Extend existing crate |
| Primaries matrices | linear-srgb | Pure math |
| `apply_gain_map` / `compute_gain_map` | zen-gainmap | Uses transfer fns |
| Tone mapping operators | zen-tonemap | Subjective, defer |
| Strip pipeline orchestration | zencodec-pipeline or zencodecs | Pipeline composition |
| `StripReader` / `StripWriter` | zencodec-pipeline or zencodecs | Not a codec contract |

Each codec implements streaming internally (like zenjpeg's `ScanlineReader`) with its own strip sizes and delay semantics. A pipeline crate adapts between them.

## Open questions

- **Rational vs f64 for gain map params?** zenavif-parse uses its own rational type; zenjpeg extracts differently. Rational is lossless but harder to work with.
- **Gain map resolution mismatch?** The applicator needs to resample gain map rows to match the base. That's a resize operation — should it share resize infrastructure?
- **Strip height adaptation?** If JPEG produces 16-row strips and AVIF wants 64-row strips, something buffers the difference. Who owns that buffer?
- **linear-srgb naming?** Should it become `zen-color` or `zen-transfer` when we add PQ/HLG, or keep the name and expand scope?
- **JXL tone mapping?** Write non-default support in jxl-encoder-rs, or define a shared `ToneMappingConfig` that multiple codecs use?
- **Animation streaming?** Current API buffers all frames. For large animations (GIF, animated AVIF/WebP), a frame-by-frame iterator or callback API will be needed. How does this compose with the strip-based pipeline?
- **Gain map as trait method timing?** Should `decode_gain_map`/`encode_gain_map` be added before v0.1.0 (as required methods), or deferred to a later version (as default-impl'd methods)?
