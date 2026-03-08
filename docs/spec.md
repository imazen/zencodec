# zencodec-types API Specification

Shared traits and types for zen* image codecs. This is the canonical reference
for the public API surface.

`#![no_std]` + `alloc`. `#![forbid(unsafe_code)]`. Codec feature gates the
trait hierarchy; everything else is always available.

### zenpixels: use but never re-export

`zenpixels` defines the cross-crate pixel interchange types: `PixelDescriptor`,
`PixelFormat`, `PixelSlice`, `PixelSliceMut`, `PixelBuffer`, `ChannelLayout`,
`ChannelType`, `TransferFunction`, `ColorPrimaries`, `AlphaMode`, `SignalRange`,
`InterleaveFormat`.

**All crates in the zen ecosystem MUST use `zenpixels` types directly.**
zencodec-types re-exports them for convenience, but callers and codec
implementors should depend on `zenpixels` directly and use `zenpixels::` paths
in their public APIs. This ensures a single source of truth for pixel types
across all crates, avoids version-mismatch breakage from re-export chains, and
lets crates that only need pixel types (no codec traits) depend on `zenpixels`
alone without pulling in `zencodec-types`.

**zencodec-types MUST NOT re-export zenpixels types.** The re-exports that
currently exist are a migration artifact and will be removed. Codec crates
should `pub use zenpixels::PixelDescriptor` etc. in their own APIs, not
`pub use zencodec_types::PixelDescriptor`.

---

## Trait hierarchy

```text
ENCODE:
                                 ┌→ Enc (Encoder and/or EncodeRgb8, EncodeRgba8, ...)
EncoderConfig → EncodeJob<'a> ──┤
                                 └→ FrameEnc (FrameEncoder and/or FrameEncodeRgba8, ...)

DECODE:
                                 ┌→ Dec (Decode)
DecoderConfig → DecodeJob<'a> ──┤→ StreamDec (StreamingDecode)
                                 └→ FrameDec (FrameDecode)
```

Color management is **not** the codec's job. Decoders return native pixels
with ICC/CICP metadata. Encoders accept pixels as-is and embed the provided
metadata. The caller handles CMS transforms.

---

## Encode traits

### `EncoderConfig` (codec config, `Clone + Send + Sync`)

```rust
trait EncoderConfig: Clone + Send + Sync {
    type Error: core::error::Error + Send + Sync + 'static;
    type Job<'a>: EncodeJob<'a, Error = Self::Error> where Self: 'a;

    fn format() -> ImageFormat;
    fn supported_descriptors() -> &'static [PixelDescriptor];
    fn capabilities() -> &'static CodecCapabilities;    // default: EMPTY

    // Universal knobs (default no-op, check via getters)
    fn with_generic_quality(self, quality: f32) -> Self;
    fn with_generic_effort(self, effort: i32) -> Self;
    fn with_lossless(self, lossless: bool) -> Self;
    fn with_alpha_quality(self, quality: f32) -> Self;
    fn generic_quality(&self) -> Option<f32>;
    fn generic_effort(&self) -> Option<i32>;
    fn is_lossless(&self) -> Option<bool>;
    fn alpha_quality(&self) -> Option<f32>;

    fn job(&self) -> Self::Job<'_>;
}
```

### `EncodeJob<'a>` (per-operation, borrows metadata/limits/stop)

```rust
trait EncodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Enc: Sized;        // single-image encoder
    type FrameEnc: Sized;   // animation encoder

    fn with_stop(self, stop: &'a dyn Stop) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_metadata(self, meta: &'a MetadataView<'a>) -> Self;
    fn with_canvas_size(self, width: u32, height: u32) -> Self;  // default no-op
    fn with_loop_count(self, count: Option<u32>) -> Self;        // default no-op

    fn encoder(self) -> Result<Self::Enc, Self::Error>;
    fn frame_encoder(self) -> Result<Self::FrameEnc, Self::Error>;
}
```

### `Encoder` (type-erased single-image encode)

```rust
trait Encoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static + From<UnsupportedOperation>;

    fn preferred_strip_height(&self) -> u32;    // default: 0
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), Self::Error>;    // default: Err
    fn finish(self) -> Result<EncodeOutput, Self::Error>;                        // default: Err
    fn encode_from(self, source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize) -> Result<EncodeOutput, Self::Error>; // default: Err
}
```

Three mutually exclusive paths: `encode()`, `push_rows()+finish()`, `encode_from()`.

### `FrameEncoder` (type-erased animation encode)

```rust
trait FrameEncoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static + From<UnsupportedOperation>;

    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), Self::Error>;
    fn push_encode_frame(&mut self, frame: EncodeFrame<'_>) -> Result<(), Self::Error>;  // default: delegates to push_frame
    fn begin_frame(&mut self, duration_ms: u32) -> Result<(), Self::Error>;              // default: Err
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), Self::Error>;            // default: Err
    fn end_frame(&mut self) -> Result<(), Self::Error>;                                  // default: Err
    fn pull_frame(&mut self, duration_ms: u32, source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize) -> Result<(), Self::Error>; // default: Err
    fn with_loop_count(&mut self, count: Option<u32>);  // default no-op
    fn finish(self) -> Result<EncodeOutput, Self::Error>;
}
```

Three mutually exclusive per-frame paths: `push_frame()`/`push_encode_frame()`,
`begin_frame()+push_rows()+end_frame()`, `pull_frame()`.

### Per-format encode traits (compile-time typed)

Each trait guarantees the codec can encode that exact pixel format.
Codec implements only the formats it accepts.

```rust
trait EncodeRgb8     { type Error; fn encode_rgb8(self, pixels: PixelSlice<'_, Rgb<u8>>)   -> Result<EncodeOutput, Self::Error>; }
trait EncodeRgba8    { type Error; fn encode_rgba8(self, pixels: PixelSlice<'_, Rgba<u8>>)  -> Result<EncodeOutput, Self::Error>; }
trait EncodeGray8    { type Error; fn encode_gray8(self, pixels: PixelSlice<'_, Gray<u8>>)  -> Result<EncodeOutput, Self::Error>; }
trait EncodeRgb16    { type Error; fn encode_rgb16(self, pixels: PixelSlice<'_, Rgb<u16>>)  -> Result<EncodeOutput, Self::Error>; }
trait EncodeRgba16   { type Error; fn encode_rgba16(self, pixels: PixelSlice<'_, Rgba<u16>>)-> Result<EncodeOutput, Self::Error>; }
trait EncodeGray16   { type Error; fn encode_gray16(self, pixels: PixelSlice<'_, Gray<u16>>)-> Result<EncodeOutput, Self::Error>; }
trait EncodeRgbF16   { type Error; fn encode_rgb_f16(self, pixels: PixelSlice<'_>)          -> Result<EncodeOutput, Self::Error>; }
trait EncodeRgbaF16  { type Error; fn encode_rgba_f16(self, pixels: PixelSlice<'_>)         -> Result<EncodeOutput, Self::Error>; }
trait EncodeRgbF32   { type Error; fn encode_rgb_f32(self, pixels: PixelSlice<'_, Rgb<f32>>)-> Result<EncodeOutput, Self::Error>; }
trait EncodeRgbaF32  { type Error; fn encode_rgba_f32(self, pixels: PixelSlice<'_, Rgba<f32>>) -> Result<EncodeOutput, Self::Error>; }
trait EncodeGrayF32  { type Error; fn encode_gray_f32(self, pixels: PixelSlice<'_, Gray<f32>>) -> Result<EncodeOutput, Self::Error>; }
```

f16 traits use type-erased `PixelSlice<'_>` because `rgb` has no half-float type.

### Per-format frame encode traits

```rust
trait FrameEncodeRgb8  { type Error; fn push_frame_rgb8(&mut self, pixels: PixelSlice<'_, Rgb<u8>>, duration_ms: u32) -> Result<(), Self::Error>; fn finish_rgb8(self) -> Result<EncodeOutput, Self::Error>; }
trait FrameEncodeRgba8 { type Error; fn push_frame_rgba8(&mut self, pixels: PixelSlice<'_, Rgba<u8>>, duration_ms: u32) -> Result<(), Self::Error>; fn finish_rgba8(self) -> Result<EncodeOutput, Self::Error>; }
```

### Codec format matrix

```
              Rgb8  Rgba8  Gray8  Rgb16  Rgba16  Gray16  RgbF16  RgbaF16  RgbF32  RgbaF32  GrayF32
JPEG           ✓             ✓
WebP           ✓      ✓
GIF                   ✓
PNG            ✓      ✓      ✓      ✓       ✓      ✓
AVIF           ✓      ✓                                                    ✓        ✓
JXL            ✓      ✓      ✓      ✓       ✓      ✓      ✓        ✓      ✓        ✓        ✓
```

---

## Decode traits

### `DecoderConfig` (codec config, `Clone + Send + Sync`)

```rust
trait DecoderConfig: Clone + Send + Sync {
    type Error: core::error::Error + Send + Sync + 'static;
    type Job<'a>: DecodeJob<'a, Error = Self::Error> where Self: 'a;

    fn format() -> ImageFormat;
    fn supported_descriptors() -> &'static [PixelDescriptor];
    fn capabilities() -> &'static CodecCapabilities;    // default: EMPTY

    fn job(&self) -> Self::Job<'_>;
}
```

### `DecodeJob<'a>` (per-operation, holds limits/stop/hints)

```rust
trait DecodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Dec: Decode<Error = Self::Error>;
    type StreamDec: StreamingDecode<Error = Self::Error>;
    type FrameDec: FrameDecode<Error = Self::Error>;

    fn with_stop(self, stop: &'a dyn Stop) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;

    // Probing
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;  // default: probe()

    // Decode hints (optional, decoder may ignore)
    fn with_crop_hint(self, x: u32, y: u32, width: u32, height: u32) -> Self;  // default no-op
    fn with_scale_hint(self, max_width: u32, max_height: u32) -> Self;          // default no-op
    fn with_orientation(self, hint: OrientationHint) -> Self;                   // default no-op

    // Output prediction
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, Self::Error>;

    // Executor creation — all bind data + preferred here
    // Consistent parameter order: data, [sink], preferred
    fn decoder(self, data: &'a [u8], preferred: &[PixelDescriptor]) -> Result<Self::Dec, Self::Error>;
    fn push_decoder(self, data: &'a [u8], sink: &mut dyn DecodeRowSink, preferred: &[PixelDescriptor]) -> Result<OutputInfo, Self::Error>;  // default: decode() + copy to sink
    fn streaming_decoder(self, data: &'a [u8], preferred: &[PixelDescriptor]) -> Result<Self::StreamDec, Self::Error>;
    fn frame_decoder(self, data: &'a [u8], preferred: &[PixelDescriptor]) -> Result<Self::FrameDec, Self::Error>;
}
```

All executor creation methods bind `data` and `preferred` at the job level.
`preferred` is a ranked list of desired output formats — the decoder picks
the first it can produce without lossy conversion. Pass `&[]` for native
format. This keeps `Decode`/`StreamingDecode`/`FrameDecode` parameter-free,
and prepares for future IO-read sources.

`push_decoder` has a default implementation that creates a decoder, calls
`decode()`, and copies the result into the sink. Codecs with native row
streaming should override for zero-copy. Returns `OutputInfo` because
pixels went into the sink. Metadata is available from `probe()` /
`output_info()` before decode starts.

### `Decode` (single-image decode, returns owned pixels)

```rust
trait Decode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn decode(self) -> Result<DecodeOutput, Self::Error>;
}
```

Created by `DecodeJob::decoder(preferred, data)` with input and format
preferences already bound.

### `StreamingDecode` (scanline-batch decode, pull iterator)

```rust
trait StreamingDecode {
    type Error: core::error::Error + Send + Sync + 'static;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error>;
    fn info(&self) -> &ImageInfo;
}
```

Created by `DecodeJob::streaming_decoder(preferred, data)` with preferences
and input already bound. Yields strips of scanlines at whatever height the
decoder prefers: MCU height for JPEG, single scanline for PNG, full image
for simple formats.

`impl StreamingDecode for ()` is the trivial rejection type — codecs that
don't support streaming set `type StreamDec = ()` and return `Err` from
`streaming_decoder()`.

### `FrameDecode` (animation decode, pull iterator)

```rust
trait FrameDecode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn frame_count(&self) -> Option<u32>;      // default: None
    fn loop_count(&self) -> Option<u32>;       // default: None
    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, Self::Error>;

    // Push model: decode next frame directly into caller-owned sink
    fn next_frame_to_sink(
        &mut self,
        sink: &mut dyn DecodeRowSink,
    ) -> Result<Option<OutputInfo>, Self::Error>;  // default: next_frame() + copy to sink
}
```

### `DecodeRowSink` (zero-copy row sink, push-based)

```rust
trait DecodeRowSink {
    fn demand(&mut self, y: u32, height: u32, width: u32, descriptor: PixelDescriptor) -> PixelSliceMut<'_>;
}
```

The codec calls `demand()` per strip, writes decoded pixels via
`PixelSliceMut::row_mut()`. The sink controls the stride (can return
SIMD-aligned buffers). The returned `PixelSliceMut` carries buffer, stride,
dimensions, and pixel descriptor together. Object-safe.

---

## Pixel types (from `zenpixels`, always available)

These types are defined in the `zenpixels` crate and used throughout the zen
ecosystem as the cross-crate interchange format. All crates depend on
`zenpixels` directly. zencodec-types uses them in trait signatures but does
not re-export them.

### `PixelSlice<'a, P = ()>`

Format-erased pixel buffer view. `P = ()` is type-erased (runtime descriptor),
`P = Rgb<u8>` etc. is compile-time typed. `From<PixelSlice<'a, P>>` converts
typed → erased.

```rust
fn data(&self) -> &[u8];
fn descriptor(&self) -> PixelDescriptor;
fn width(&self) -> u32;
fn height(&self) -> u32;
fn stride(&self) -> usize;
fn rows(&self) -> u32;   // alias for height
```

### `PixelSliceMut<'a, P = ()>`

Mutable version of `PixelSlice`.

### `PixelBuffer`

Owned pixel buffer (`Vec<u8>` backing).

### `PixelDescriptor`

Describes pixel format: channel layout, channel type, signal range, transfer
function, color primaries, alpha mode. Provides named constants:

```rust
PixelDescriptor::RGB8_SRGB
PixelDescriptor::RGBA8_SRGB
PixelDescriptor::GRAY8_SRGB
PixelDescriptor::RGB16_LINEAR
PixelDescriptor::RGBA16_LINEAR
PixelDescriptor::RGBF32_LINEAR
PixelDescriptor::RGBAF32_LINEAR
PixelDescriptor::GRAYF32_LINEAR
// ... etc.
```

### `PixelFormat`

Enum: `Rgb8`, `Rgba8`, `Gray8`, `Rgb16`, `Rgba16`, `Gray16`, `RgbF16`,
`RgbaF16`, `RgbF32`, `RgbaF32`, `GrayF32`, `GrayAlpha8`, `GrayAlpha16`,
`GrayAlphaF32`, `Bgra8`, `Bgrx8`, `Rgbx8`.

### Supporting enums

- `ChannelLayout` — RGB, RGBA, Gray, GrayAlpha, BGRA, BGRX, RGBX
- `ChannelType` — U8, U16, F16, F32
- `SignalRange` — Full, Limited
- `TransferFunction` — Srgb, Linear, Pq, Hlg
- `ColorPrimaries` — Bt709, Bt2020, DisplayP3, AdobeRgb, ...
- `AlphaMode` — Straight, Premultiplied, None
- `InterleaveFormat` — Interleaved, Planar

---

## Image metadata

### `ImageInfo`

Full image metadata from probing/decoding:

```rust
fn width(&self) -> u32;
fn height(&self) -> u32;
fn descriptor(&self) -> PixelDescriptor;
fn orientation(&self) -> Orientation;
fn metadata(&self) -> &Metadata;
fn source_color(&self) -> Option<&SourceColor>;
```

### `Metadata`

```rust
fn icc_profile(&self) -> Option<&[u8]>;
fn exif(&self) -> Option<&[u8]>;
fn xmp(&self) -> Option<&[u8]>;
fn iptc(&self) -> Option<&[u8]>;
fn cicp(&self) -> Option<&Cicp>;
```

### `MetadataView<'a>`

Borrowed metadata for encoding. Same accessors as `Metadata` but borrowed.

### `OutputInfo`

Predicted decoder output (dimensions, format, which hints were honored).

### `SourceColor`

Source color information for embedded ICC profile tracking.

### `Orientation` / `OrientationHint`

EXIF orientation (1-8) and decode-time orientation handling strategy.

### `Cicp`

Color primaries, transfer characteristics, matrix coefficients, video full
range flag (ITU-T H.273).

### `EmbeddedMetadata`

Container for ICC, EXIF, XMP, IPTC byte blobs.

---

## Output types

### `EncodeOutput`

Encoded image bytes + metadata. Supports typed extras for codec-specific data.

```rust
fn data(&self) -> &[u8];
fn into_vec(self) -> Vec<u8>;
fn format(&self) -> ImageFormat;
fn with_extras<T: Any + Send>(self, extras: T) -> Self;
fn extras<T: Any + Send>(&self) -> Option<&T>;
fn take_extras<T: Any + Send>(&mut self) -> Option<T>;
```

Clone drops extras. PartialEq/Eq skip extras.

### `DecodeOutput` (codec feature)

Decoded image with owned pixel data. Supports typed extras for codec-specific data.

```rust
fn pixels(&self) -> PixelSlice<'_>;
fn info(&self) -> &ImageInfo;
fn into_buffer(self) -> PixelBuffer;
fn with_extras<T: Any + Send>(self, extras: T) -> Self;
fn extras<T: Any + Send>(&self) -> Option<&T>;
fn take_extras<T: Any + Send>(&mut self) -> Option<T>;
```

### `OwnedFullFrame` (codec feature)

Owned animation frame with pixel data. Supports typed extras.

```rust
fn pixels(&self) -> PixelSlice<'_>;
fn duration_ms(&self) -> u32;
fn frame_index(&self) -> u32;
fn with_extras<T: Any + Send>(self, extras: T) -> Self;
fn extras<T: Any + Send>(&self) -> Option<&T>;
fn take_extras<T: Any + Send>(&mut self) -> Option<T>;
```

### `EncodeFrame<'a>` / `DecodeFrame`

Animation frame with positioning, timing, blend/disposal modes.

```rust
// EncodeFrame
fn pixels(&self) -> PixelSlice<'_>;
fn duration_ms(&self) -> u32;
fn x(&self) -> u32;
fn y(&self) -> u32;
fn blend(&self) -> FrameBlend;
fn disposal(&self) -> FrameDisposal;

// DecodeFrame
fn pixels(&self) -> PixelSlice<'_>;
fn info(&self) -> &ImageInfo;
fn duration_ms(&self) -> u32;
```

### `FrameBlend` / `FrameDisposal`

Animation compositing: `Source` / `Over` and `None` / `Background` / `Previous`.

---

## Format detection

### `ImageFormat`

```rust
enum ImageFormat {
    Jpeg, Png, Gif, WebP, Avif, Jxl, Heif, Bmp, Tiff, Ico, Pnm, Farbfeld, Qoi, Unknown,
}

fn from_magic(data: &[u8]) -> Self;    // detect from first bytes
fn mime_type(&self) -> &'static str;
fn extension(&self) -> &'static str;
```

---

## Capabilities

### `CodecCapabilities`

Const-constructible struct with builder pattern. Returned by
`EncoderConfig::capabilities()` / `DecoderConfig::capabilities()`.

Flags: `encode_icc`, `encode_exif`, `encode_xmp`, `decode_icc`, `decode_exif`,
`decode_xmp`, `encode_cancel`, `decode_cancel`, `native_gray`, `cheap_probe`,
`encode_animation`, `decode_animation`, `native_16bit`, `lossless`, `lossy`,
`hdr`, `encode_cicp`, `decode_cicp`, `enforces_max_pixels`,
`enforces_max_memory`, `enforces_max_file_size`, `native_f32`, `native_alpha`,
`decode_into`, `row_level_encode`, `pull_encode`, `row_level_decode`,
`row_level_frame_encode`, `pull_frame_encode`, `frame_decode_into`,
`row_level_frame_decode`.

Ranges: `effort_range() -> Option<[i32; 2]>`, `quality_range() -> Option<[f32; 2]>`.

`row_level_decode` means the codec streams rows natively (not decode-then-copy)
for both pull (`StreamingDecode::next_batch`) and push (`Decode::decode_to_sink`).
Same for `row_level_frame_decode` covering `FrameDecode::next_frame_to_sink`.

### `UnsupportedOperation`

Enum for codecs to report which operations they don't support. Implements
`core::error::Error` + `Display`.

### `HasUnsupportedOperation`

Trait for codec errors that can report unsupported operations without
downcasting.

---

## Resource limits

### `ResourceLimits`

```rust
fn max_pixels(&self) -> Option<u64>;
fn max_memory_bytes(&self) -> Option<u64>;
fn max_file_size(&self) -> Option<u64>;
```

### `LimitExceeded`

Error type when a resource limit is exceeded.

---

## Color types

### `ColorContext` / `ColorProfileSource` / `NamedProfile`

Color context for pipeline tracking (ICC profile bytes, named profiles,
CICP parameters).

### `GainMapMetadata`

HDR gain map metadata (ISO 21496-1).

---

## Conversion types (always available)

### `ConvertOptions` / `ConvertError`

Options and error type for pixel format conversion.

### `AlphaPolicy` / `DepthPolicy` / `GrayExpand` / `LumaCoefficients`

Policies for alpha handling, bit depth changes, grayscale expansion, and
luma coefficient selection during conversion.

### `PixelSliceConvertExt`

Extension trait on `PixelSlice` for in-place and allocating format conversions.

---

## Error tracking (always available, re-exported from `whereat`)

```rust
pub use whereat::{At, AtTrace, AtTraceable, ErrorAtExt, ResultAtExt};
```

Pattern: `type Error = At<MyCodecError>;` then `.at()` captures file:line.

---

## Re-exports (codec feature)

```rust
pub use enough::{Stop, Unstoppable};
pub use imgref::{Img, ImgRef, ImgRefMut, ImgVec};
pub use rgb::{self, Gray, Rgb, Rgba};
pub use rgb::alt::BGRA as Bgra;
pub use zenpixels_convert::ext::PixelBufferConvertExt;
```

