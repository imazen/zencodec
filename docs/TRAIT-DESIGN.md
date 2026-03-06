# zencodec-types Trait Redesign

## Motivation

The current 4-layer trait hierarchy (Config → Job → Encoder/Decoder) uses a
single type-erased `Encoder::encode(PixelSlice)` method. This requires runtime
capability checking via `CodecCapabilities` to know what pixel formats a codec
accepts. A JPEG encoder silently receives RGBA and must decide what to do.

This redesign splits encode into per-format traits: `EncodeRgb8`, `EncodeRgba8`,
etc. If a codec doesn't implement a trait, you get a compile error, not a
runtime surprise. CodecCapabilities shrinks to informational metadata.

Decoders stay type-erased because the output format is discovered at runtime
from the file. The caller provides a ranked preference list and the decoder
picks the best match.

Color management is explicitly not the codec's job. Decoders return native
pixels with ICC/CICP metadata. Callers use their CMS of choice.

## Design Principles

1. **Typed encode, erased decode.** Encoders accept concrete pixel formats via
   per-format traits. Decoders return type-erased `PixelBuffer` because the
   format comes from the file.

2. **Probe on job, not config.** Probing needs limits and cancellation context.
   The job holds those, so probe lives there.

3. **No codec-side color management.** Decoders return native pixels. Encoders
   accept pixels as-is. ICC/CICP metadata flows through `MetadataView` for
   embedding. The caller handles CMS transforms.

4. **OrientationHint for coalesced transforms.** Instead of a boolean "apply
   orientation," an enum lets callers request orientation correction plus
   additional transforms, which the decoder can coalesce with decode (e.g.,
   JPEG lossless DCT rotation).

5. **Preferred descriptors replace decode_into.** `Decode::decode()` takes a
   ranked list of preferred pixel formats. The decoder picks the first it can
   produce without lossy conversion. No caller-provided output buffer for now.

6. **Per-format traits are independently versionable.** Adding a new pixel
   format means adding a new trait. Existing codecs don't break. Old consumers
   don't notice.

## Trait Hierarchy

```text
ENCODE:
                                 ┌→ Enc (implements EncodeRgb8, EncodeGray8, ...)
EncoderConfig → EncodeJob<'a> ──┤
                                 └→ FrameEnc (implements FrameEncodeRgba8, ...)

DECODE:
                                 ┌→ Dec (implements Decode)
DecoderConfig → DecodeJob<'a> ──┤
                                 └→ FrameDec (implements FrameDecode)
```

### EncoderConfig

Reusable, `Clone + Send + Sync`. Holds quality, effort, lossless settings.

```rust
pub trait EncoderConfig: Clone + Send + Sync {
    type Error: core::error::Error + Send + Sync + 'static;
    type Job<'a>: EncodeJob<'a, Error = Self::Error> where Self: 'a;

    fn format() -> ImageFormat;
    fn supported_descriptors() -> &'static [PixelDescriptor];

    fn with_quality(self, quality: f32) -> Self;     // 0-100 calibrated
    fn with_effort(self, effort: i32) -> Self;       // codec-mapped
    fn with_lossless(self, lossless: bool) -> Self;
    fn with_alpha_quality(self, quality: f32) -> Self;

    fn quality(&self) -> Option<f32>;
    fn effort(&self) -> Option<i32>;
    fn is_lossless(&self) -> Option<bool>;
    fn alpha_quality(&self) -> Option<f32>;

    fn job(&self) -> Self::Job<'_>;
}
```

### EncodeJob

Per-operation. Binds metadata, limits, cancellation.

```rust
pub trait EncodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Enc: Sized;
    type FrameEnc: Sized;

    fn with_stop(self, stop: &'a dyn Stop) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_metadata(self, meta: &'a MetadataView<'a>) -> Self;
    fn with_canvas_size(self, width: u32, height: u32) -> Self;

    fn encoder(self) -> Result<Self::Enc, Self::Error>;
    fn frame_encoder(self) -> Result<Self::FrameEnc, Self::Error>;
}
```

### Per-Format Encode Traits

Each codec implements only the pixel formats it accepts. The trait name IS the
format contract. Compile-time enforcement.

```rust
pub trait EncodeRgb8    { type Error; fn encode_rgb8(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeRgba8   { type Error; fn encode_rgba8(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeGray8   { type Error; fn encode_gray8(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeRgb16   { type Error; fn encode_rgb16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeRgba16  { type Error; fn encode_rgba16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeGray16  { type Error; fn encode_gray16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeRgbF16  { type Error; fn encode_rgb_f16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeRgbaF16 { type Error; fn encode_rgba_f16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeRgbF32  { type Error; fn encode_rgb_f32(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeRgbaF32 { type Error; fn encode_rgba_f32(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
pub trait EncodeGrayF32 { type Error; fn encode_gray_f32(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>; }
```

### Per-Format Frame Encode Traits

```rust
pub trait FrameEncodeRgb8 {
    type Error;
    fn push_frame_rgb8(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), Self::Error>;
    fn finish_rgb8(self) -> Result<EncodeOutput, Self::Error>;
}
pub trait FrameEncodeRgba8 {
    type Error;
    fn push_frame_rgba8(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), Self::Error>;
    fn finish_rgba8(self) -> Result<EncodeOutput, Self::Error>;
}
// Additional frame encode traits as needed.
```

### Codec Format Matrix

```
ENCODE (single image):
              Rgb8  Rgba8  Gray8  Rgb16  Rgba16  Gray16  RgbF16  RgbaF16  RgbF32  RgbaF32  GrayF32
JPEG           ✓             ✓
WebP           ✓      ✓
GIF                   ✓
PNG            ✓      ✓      ✓      ✓       ✓      ✓
AVIF           ✓      ✓                                                    ✓        ✓
JXL            ✓      ✓      ✓      ✓       ✓      ✓      ✓        ✓      ✓        ✓        ✓

ENCODE (animation):
              FrameRgba8  FrameRgb8
GIF                ✓
WebP               ✓          ✓
```

### DecoderConfig

Reusable, `Clone + Send + Sync`. No data-dependent methods.

```rust
pub trait DecoderConfig: Clone + Send + Sync {
    type Error: core::error::Error + Send + Sync + 'static;
    type Job<'a>: DecodeJob<'a, Error = Self::Error> where Self: 'a;

    fn format() -> ImageFormat;
    fn supported_descriptors() -> &'static [PixelDescriptor];

    fn job(&self) -> Self::Job<'_>;
}
```

### DecodeJob

Per-operation. Holds limits, cancellation, hints. Probing lives here.

```rust
pub trait DecodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Dec: Decode<Error = Self::Error>;
    type FrameDec: FrameDecode<Error = Self::Error>;

    fn with_stop(self, stop: &'a dyn Stop) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;

    // Probing (needs limits + stop context)
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;

    // Decode hints (optional, decoder may ignore)
    fn with_crop_hint(self, x: u32, y: u32, w: u32, h: u32) -> Self;
    fn with_scale_hint(self, max_width: u32, max_height: u32) -> Self;
    fn with_orientation(self, hint: OrientationHint) -> Self;

    // Output prediction
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, Self::Error>;

    fn decoder(self) -> Result<Self::Dec, Self::Error>;
    fn frame_decoder(self, data: &'a [u8]) -> Result<Self::FrameDec, Self::Error>;
}
```

### Decode

Single-image decode. Returns owned pixels. Takes preferred descriptor list.

```rust
pub trait Decode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn decode(
        self,
        data: &[u8],
        preferred: &[PixelDescriptor],
    ) -> Result<DecodeOutput, Self::Error>;
}
```

The decoder picks the first descriptor from `preferred` that it can produce
without lossy conversion. Empty slice = decoder's native format.

### FrameDecode

Animation. Returns owned frames.

```rust
pub trait FrameDecode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn frame_count(&self) -> Option<u32>;
    fn loop_count(&self) -> Option<u32>;
    fn next_frame(&mut self, preferred: &[PixelDescriptor]) -> Result<Option<DecodeFrame>, Self::Error>;
}
```

## OrientationHint

Replaces `with_orientation_hint(Orientation)`. Allows coalescing additional
transforms with the decode operation.

```rust
pub enum OrientationHint {
    /// Don't touch. Report intrinsic orientation in ImageInfo.
    Preserve,
    /// Resolve EXIF/container orientation to Normal.
    /// Decoder coalesces with decode (JPEG: lossless DCT transform).
    Correct,
    /// Resolve EXIF, then apply additional transform.
    /// Decoder coalesces the combined operation.
    CorrectAndTransform(Orientation),
    /// Ignore EXIF. Apply exactly this transform.
    ExactTransform(Orientation),
}
```

## Metadata Flow

### Decode → ImageInfo

Decoders populate `ImageInfo` with everything they find:
- `icc_profile: Option<Arc<[u8]>>` — ICC bytes (Arc for cheap sharing)
- `exif: Option<Vec<u8>>` — raw EXIF TIFF blob
- `xmp: Option<Vec<u8>>` — raw XMP XML
- `cicp: Option<Cicp>` — ITU-T H.273 color description
- `content_light_level: Option<ContentLightLevel>` — CEA-861.3
- `mastering_display: Option<MasteringDisplay>` — SMPTE ST 2086
- `orientation: Orientation` — intrinsic (Normal if decoder applied it)
- `has_gain_map: bool`, `gain_map_metadata: Option<GainMapMetadata>`

### ImageInfo → MetadataView → Encode

`ImageInfo::metadata()` borrows a `MetadataView<'a>`. Pass to
`EncodeJob::with_metadata()`. Codec embeds what it supports, silently skips
the rest.

```
          ICC   EXIF   XMP   CICP   HDR   GainMap
JPEG       ✓     ✓      ✓     —      —    ✓(xmp)
WebP       ✓     ✓      ✓     —      —     —
PNG        ✓     ✓      ✓     ✓      ✓     —
AVIF       ✓     ✓      ✓     ✓      ✓   planned
JXL        ✓     ✓      ✓     ✓      ✓   planned
GIF        —     —      —     —      —     —
```

### No Codec-Side Color Management

Pixels come out of the decoder in the codec's native color space. `ImageInfo`
carries ICC and/or CICP so the caller can build a CMS transform. The encode
side accepts pixels as-is and embeds the provided ICC/CICP from
`MetadataView`.

If the caller converts pixels (e.g., wide-gamut to sRGB), they update the
ICC/CICP in the `MetadataView` before passing to encode. The codec never
modifies pixel values for color management.

## What This Replaces

| Old | New | Notes |
|-----|-----|-------|
| `Encoder::encode(PixelSlice)` | `EncodeRgb8::encode_rgb8(PixelSlice)` etc. | Compile-time format safety |
| `CodecCapabilities` for encode formats | Trait existence | Type system does the work |
| `DecoderConfig::probe_header()` | `DecodeJob::probe()` | Needs limits/stop context |
| `Decoder::decode(data)` | `Decode::decode(data, &preferred)` | Caller states format preference |
| `Decoder::decode_into(data, dst)` | Deferred | Owned output only for now |
| `with_orientation_hint(Orientation)` | `with_orientation(OrientationHint)` | Coalesced transforms |
| `clamp_quality()` | Removed | Unused |
| `ImageMetadata` (deprecated alias) | Removed | Use `MetadataView` |

## What's Deferred

| Item | Reason |
|------|--------|
| Generic `PixelSlice<'a, P>` | Needs `Pixel` trait design; per-format traits work without it |
| `Decode::decode_into()` | Owned output first; caller buffer is an optimization |
| Type-erased `Encode` trait | Build on concrete traits first; add dispatch macro later |
| Row-level streaming encode/decode | Get one-shot right first |
| Auto-format selection | Needs encode traits proven across codecs |
| `half` crate for typed f16 pixels | `EncodeRgbF16` works with untyped `PixelSlice` for now |
| Migrate `DecodeOutput` from `PixelData` to `PixelBuffer` | Requires all decoders updated |
| Migrate `DecodeFrame` similarly | Same |
| Macro for generating type-erased `Encode` from per-format traits | After pattern stabilizes |
| `DecodeRowSink` integration | Depends on streaming design |
| Update 12+ downstream codec implementations | Separate effort per codec |
