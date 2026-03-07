# zencodec-types

Shared traits and types for the zen\* image codec family.

This crate defines the common interface that all zen\* codecs implement. It contains no codec logic — just traits, types, and format negotiation helpers. `no_std` compatible (requires `alloc`), `forbid(unsafe_code)`.

**Lib name:** `zc` — use `zc::` in imports, `zencodec-types` on crates.io.

## Crates in the zen\* family

| Crate | Format | Repo |
|-------|--------|------|
| `zenjpeg` | JPEG | [imazen/zenjpeg](https://github.com/imazen/zenjpeg) |
| `zenwebp` | WebP | [imazen/zenwebp](https://github.com/imazen/zenwebp) |
| `zenpng` | PNG | [imazen/zenpng](https://github.com/imazen/zenpng) |
| `zengif` | GIF | [imazen/zengif](https://github.com/imazen/zengif) |
| `zenavif` | AVIF | [imazen/zenavif](https://github.com/imazen/zenavif) |
| `zenjxl` | JPEG XL | [imazen/zenjxl](https://github.com/imazen/zenjxl) |
| `zenbitmaps` | PNM/BMP/Farbfeld | [imazen/zenbitmaps](https://github.com/imazen/zenbitmaps) |
| `zencodecs` | Multi-format dispatch | [imazen/zencodecs](https://github.com/imazen/zencodecs) |

## Architecture: Config → Job → Executor

The trait hierarchy has three lifetime layers:

```text
Layer 1: Config     (Clone + Send + Sync, 'static, reusable)
Layer 2: Job        (borrows config + per-op data, short-lived)
Layer 3: Executor   (borrows job's data + input, consumes self)
```

**Config** lives in a struct, gets shared across threads, cloned freely. A web
server keeps one `JpegEncoderConfig` at quality 85 for all requests.

**Job** borrows temporary per-operation data: a `&Stop` cancellation token,
`ResourceLimits`, `&MetadataView`. These are stack-local and die after the
encode/decode call.

**Executor** borrows the actual pixel data or file bytes. It consumes itself to
produce output (single-shot encoding/decoding).

```text
ENCODE:
                              ┌→ Enc (implements Encoder)
EncoderConfig → EncodeJob<'a> ┤
                              └→ FrameEnc (implements FrameEncoder)

DECODE:
                              ┌→ Dec (implements Decode)
DecoderConfig → DecodeJob<'a> ┤→ StreamDec (implements StreamingDecode)
                              └→ FrameDec (implements FrameDecode)
```

Color management is **not** the codec's job. Decoders return native pixels with
ICC/CICP metadata. Encoders accept pixels as-is and embed the provided metadata.
The caller handles CMS transforms.

---

## Encode traits

### `EncoderConfig` — reusable config

```rust
trait EncoderConfig: Clone + Send + Sync {
    type Error: core::error::Error + Send + Sync + 'static;
    type Job<'a>: EncodeJob<'a, Error = Self::Error> where Self: 'a;

    fn format() -> ImageFormat;
    fn supported_descriptors() -> &'static [PixelDescriptor];
    fn capabilities() -> &'static EncodeCapabilities;

    // Universal knobs (all default no-op, check via getters):
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

Builder-pattern config. The `with_*` methods consume and return `Self`. The
getters (`generic_quality()` → `Option<f32>`) let you check whether the codec
actually accepted a value — a codec without quality tuning returns `None`.

`format()`, `supported_descriptors()`, and `capabilities()` are **associated
functions** (no `&self`). They're compile-time constants.

### `EncodeJob<'a>` — per-operation setup

```rust
trait EncodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Enc: Sized;
    type FrameEnc: Sized;

    fn with_stop(self, stop: &'a dyn Stop) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_policy(self, policy: EncodePolicy) -> Self;
    fn with_metadata(self, meta: &'a MetadataView<'a>) -> Self;
    fn with_canvas_size(self, width: u32, height: u32) -> Self;
    fn with_loop_count(self, count: Option<u32>) -> Self;

    fn encoder(self) -> Result<Self::Enc, Self::Error>;
    fn frame_encoder(self) -> Result<Self::FrameEnc, Self::Error>;

    // Type-erased convenience (provided defaults):
    fn dyn_encoder(self) -> Result<Box<dyn DynEncoder + 'a>, BoxedError>
        where Self::Enc: Encoder;
    fn dyn_frame_encoder(self) -> Result<Box<dyn DynFrameEncoder + 'a>, BoxedError>
        where Self::FrameEnc: FrameEncoder;
}
```

### `Encoder` — type-erased single-image encode

```rust
trait Encoder: Sized {
    type Error: ... + From<UnsupportedOperation>;

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
    fn encode_srgba8(self, data: &mut [u8], make_opaque: bool,
        width: u32, height: u32, stride_pixels: u32) -> Result<EncodeOutput, Self::Error>;
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), Self::Error>;
    fn finish(self) -> Result<EncodeOutput, Self::Error>;
    fn encode_from(self, source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize)
        -> Result<EncodeOutput, Self::Error>;
}
```

Three mutually exclusive paths:

1. **`encode()`** — all at once, consumes self. Simplest, most common.
2. **`push_rows()` + `finish()`** — caller pushes strips.
3. **`encode_from()`** — encoder pulls from a callback.

Unsupported paths return `Err(UnsupportedOperation::*.into())` by default.

### `FrameEncoder` — animation encode

Same three paths per frame: whole-frame push, row push, or pull.
`push_encode_frame` adds sub-canvas positioning via `EncodeFrame` (carrying
`frame_rect`, `blend`, `disposal`).

---

## Decode traits

### `DecoderConfig` — reusable config

Mirrors `EncoderConfig`. Same static functions: `format()`,
`supported_descriptors()`, `capabilities()`. Creates a `Job<'a>`.

### `DecodeJob<'a>` — per-operation setup

```rust
trait DecodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Dec: Decode<Error = Self::Error>;
    type StreamDec: StreamingDecode<Error = Self::Error>;
    type FrameDec: FrameDecode<Error = Self::Error>;

    fn with_stop(self, stop: &'a dyn Stop) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_policy(self, policy: DecodePolicy) -> Self;

    // Probing
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;

    // Decode hints (optional, decoder may ignore)
    fn with_crop_hint(self, x: u32, y: u32, width: u32, height: u32) -> Self;
    fn with_scale_hint(self, max_width: u32, max_height: u32) -> Self;
    fn with_orientation(self, hint: OrientationHint) -> Self;

    // Output prediction
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, Self::Error>;

    // Executor creation — all bind data + preferred here
    fn decoder(self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<Self::Dec, Self::Error>;
    fn push_decoder(self, data: &'a [u8], sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor]) -> Result<OutputInfo, Self::Error>;
    fn streaming_decoder(self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<Self::StreamDec, Self::Error>;
    fn frame_decoder(self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<Self::FrameDec, Self::Error>;

    // Type-erased convenience (provided defaults):
    fn dyn_decoder(...) -> Result<Box<dyn DynDecoder + 'a>, BoxedError>;
    fn dyn_frame_decoder(...) -> Result<Box<dyn DynFrameDecoder + 'a>, BoxedError>;
    fn dyn_streaming_decoder(...) -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError>;
}
```

**Format negotiation via `preferred`:** The caller passes a ranked
`&[PixelDescriptor]` list. The decoder picks the first it can produce without
lossy conversion. Pass `&[]` for native format.

**Data binding at the job level:** All executor constructors take `data: &'a [u8]`
here, not on the executor. This means `Decode::decode()` takes no arguments —
data was already bound.

### `Decode` — single-image decode

```rust
trait Decode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    fn decode(self) -> Result<DecodeOutput, Self::Error>;
}
```

Returns `DecodeOutput` which wraps a `PixelBuffer` + `ImageInfo`.

### `StreamingDecode` — scanline-batch pull iterator

Yields `(start_row, strip_pixels)`. Strip height is codec-determined.
Codecs that don't support streaming set `type StreamDec = ()`.

### `FrameDecode` — animation pull iterator

`DecodeFrame` carries `PixelBuffer`, `Arc<ImageInfo>` (shared across frames),
delay, index, and compositing metadata (`blend`, `disposal`, `frame_rect`,
`required_frame`).

### `DecodeRowSink` — zero-copy row sink

```rust
trait DecodeRowSink {
    fn demand(&mut self, y: u32, height: u32, width: u32, descriptor: PixelDescriptor)
        -> PixelSliceMut<'_>;
}
```

Lending pattern: the sink provides mutable buffers, the codec fills them. The
sink controls stride (can return SIMD-aligned buffers). Object-safe.

---

## Dyn dispatch traits

The `Dyn*` traits mirror the generic hierarchy with object-safe interfaces:

```text
DynEncoderConfig → DynEncodeJob → DynEncoder / DynFrameEncoder
DynDecoderConfig → DynDecodeJob → DynDecoder / DynFrameDecoder / DynStreamingDecoder
```

Blanket impls automatically implement `DynEncoderConfig` for any `EncoderConfig`
(and `DynDecoderConfig` for any `DecoderConfig`). This enables fully
codec-agnostic dispatch with no generic parameters:

```rust
fn encode(config: &dyn DynEncoderConfig, pixels: PixelSlice<'_>)
    -> Result<Vec<u8>, BoxedError>
{
    let enc = config.dyn_job().into_encoder()?;
    Ok(enc.encode(pixels)?.into_vec())
}

// Works with any codec:
encode(&jpeg_config, pixels)?;
encode(&webp_config, pixels)?;
```

The `dyn_encoder()` / `dyn_decoder()` convenience methods on `EncodeJob` /
`DecodeJob` provide a shortcut for the common case.

---

## Format negotiation

```rust
use zc::decode::{negotiate_pixel_format, is_format_available};
use zc::encode::best_encode_format;

// Decode: pick best output format from caller's preferences
let format = negotiate_pixel_format(preferred, &available_for_this_image);

// Encode: check if encoder accepts the caller's pixel data
if let Some(fmt) = best_encode_format(source_descriptor, supported) { ... }

// Quick existence check
if is_format_available(PixelFormat::Rgba8, &supported_descriptors) { ... }
```

---

## Cross-cutting types

### `EncodeCapabilities` / `DecodeCapabilities`

Const-constructible structs with boolean flags and range fields. Returned as
`&'static` references. Callers discover behavior before calling methods.

```rust
static CAPS: EncodeCapabilities = EncodeCapabilities::new()
    .with_icc(true)
    .with_exif(true)
    .with_cancel(true)
    .with_lossy(true)
    .with_lossless(true)
    .with_quality_range(0.0, 100.0)
    .with_effort_range(1, 9);
```

### `ResourceLimits`

Optional caps on pixels, memory, file size, frames, duration. Builder pattern,
`Copy`. Has `check_*` methods for early rejection.

### `DecodePolicy` / `EncodePolicy`

Per-job security flags. Three-valued (`None`/`Some(true)`/`Some(false)`) with
`resolve_*()` methods. Named levels: `strict()`, `permissive()`, `none()`.

### `UnsupportedOperation` / `HasUnsupportedOperation`

Standard error reporting for unsupported paths. The `From<UnsupportedOperation>`
bound on `Encoder::Error` enables default method impls to return proper errors.

---

## Output types

### `EncodeOutput`

Encoded bytes (`Vec<u8>`) + `ImageFormat`. `into_vec()`, `data()`, `AsRef<[u8]>`.

### `DecodeOutput`

Decoded pixels (`PixelBuffer`) + `ImageInfo` + optional type-erased extras
(`Box<dyn Any + Send>`). Use `into_buffer()` to take the `PixelBuffer`,
`pixels()` to borrow as `PixelSlice`.

### `DecodeFrame`

Animation frame: `PixelBuffer` + `Arc<ImageInfo>` (shared across frames) +
delay, index, compositing metadata (`blend`, `disposal`, `frame_rect`,
`required_frame`).

### `EncodeFrame<'a>`

Animation frame for encoding: `PixelSlice` + duration + compositing parameters.

---

## Pixel types (from `zenpixels`)

All pixel interchange types come from the `zenpixels` crate:

- **`PixelSlice<'a>`** — format-erased pixel buffer view with runtime descriptor
- **`PixelSliceMut<'a>`** — mutable version
- **`PixelBuffer`** — owned pixel buffer (`Vec<u8>` backing)
- **`PixelDescriptor`** — describes pixel format: channel layout, type, signal
  range, transfer function, color primaries, alpha mode. Named constants:
  `RGB8_SRGB`, `RGBA8_SRGB`, `GRAY8_SRGB`, etc.

## License

Apache-2.0 OR MIT
