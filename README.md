# zencodec-types

Shared traits and types for the zen\* image codec family.

This crate defines the common interface that all zen\* codecs implement. It contains no codec logic — just traits, types, pixel format descriptors, and format negotiation helpers. `no_std` compatible (requires `alloc`), `forbid(unsafe_code)`.

## Crates in the zen\* family

| Crate | Format | Repo |
|-------|--------|------|
| `zenjpeg` | JPEG | [imazen/zenjpeg](https://github.com/imazen/zenjpeg) |
| `zenwebp` | WebP | [imazen/zenwebp](https://github.com/imazen/zenwebp) |
| `zenpng` | PNG | [imazen/zenpng](https://github.com/imazen/zenpng) |
| `zengif` | GIF | [imazen/zengif](https://github.com/imazen/zengif) |
| `zenavif` | AVIF | [imazen/zenavif](https://github.com/imazen/zenavif) |
| `zenjxl` | JPEG XL | [imazen/zenjxl](https://github.com/imazen/zenjxl) |
| `zencodecs` | Multi-format dispatch | [imazen/zencodecs](https://github.com/imazen/zencodecs) |

## Architecture: Config → Job → Executor

The trait hierarchy splits into three lifetime layers that map to real usage
patterns:

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
                              ┌→ Enc (Encoder and/or EncodeRgb8, EncodeRgba8, ...)
EncoderConfig → EncodeJob<'a> ┤
                              └→ FrameEnc (FrameEncoder and/or FrameEncodeRgba8, ...)

DECODE:
                              ┌→ Dec (Decode)
DecoderConfig → DecodeJob<'a> ┤→ StreamDec (StreamingDecode)
                              └→ FrameDec (FrameDecode)
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
    fn capabilities() -> &'static CodecCapabilities;

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
functions** (no `&self`). They're compile-time constants — a JPEG encoder always
produces JPEG, always supports the same pixel formats. `capabilities()` returns
`&'static CodecCapabilities` for zero runtime cost.

### `EncodeJob<'a>` — per-operation setup

```rust
trait EncodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Enc: Sized;       // NO trait bounds
    type FrameEnc: Sized;  // NO trait bounds

    fn with_stop(self, stop: &'a dyn Stop) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_policy(self, policy: EncodePolicy) -> Self;
    fn with_metadata(self, meta: &'a MetadataView<'a>) -> Self;
    fn with_canvas_size(self, width: u32, height: u32) -> Self;
    fn with_loop_count(self, count: Option<u32>) -> Self;

    fn encoder(self) -> Result<Self::Enc, Self::Error>;
    fn frame_encoder(self) -> Result<Self::FrameEnc, Self::Error>;

    // Type-erased convenience (provided default implementations):
    fn dyn_encoder(self) -> Result<DynEncoder<'a>, BoxedError>
        where Self: 'a, Self::Enc: Encoder;
    fn dyn_frame_encoder(self) -> Result<DynFrameEncoder<'a>, BoxedError>
        where Self: 'a, Self::FrameEnc: FrameEncoder;
}
```

**`Enc` and `FrameEnc` have NO trait bounds.** This is deliberate. The codec's
encoder type implements whichever combination of traits makes sense:

- Just `Encoder` (type-erased) for simple codecs
- Just `EncodeRgb8 + EncodeGray8` (per-format) for callers who know the pixel type
- Both, for maximum flexibility

The caller decides which trait to use on the returned encoder. The job doesn't
constrain it.

### `Encoder` — type-erased single-image encode

```rust
trait Encoder: Sized {
    type Error: ... + From<UnsupportedOperation>;

    fn preferred_strip_height(&self) -> u32;
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), Self::Error>;
    fn finish(self) -> Result<EncodeOutput, Self::Error>;
    fn encode_from(self, source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize)
        -> Result<EncodeOutput, Self::Error>;
}
```

Three mutually exclusive paths:

1. **`encode()`** — all at once, consumes self. Simplest, most common.
2. **`push_rows()` + `finish()`** — caller pushes strips. Good for streaming from disk.
3. **`encode_from()`** — encoder pulls from a callback. Good when the codec has a
   preferred strip height (e.g. JPEG MCUs).

The `From<UnsupportedOperation>` bound on `Error` makes the defaults work —
`push_rows` defaults to `Err(UnsupportedOperation::RowLevelEncode.into())`.
Check `CodecCapabilities::row_level_encode()` before calling, or handle the error.

### Per-format encode traits — compile-time typed

Each trait is a single-method contract that the codec can encode that exact
pixel format. No runtime dispatch. The caller knows at compile time what the
codec accepts.

```rust
trait EncodeRgb8  { type Error; fn encode_rgb8(self, pixels: PixelSlice<'_, Rgb<u8>>)  -> Result<EncodeOutput, Self::Error>; }
trait EncodeRgba8 { type Error; fn encode_rgba8(self, pixels: PixelSlice<'_, Rgba<u8>>) -> Result<EncodeOutput, Self::Error>; }
trait EncodeGray8 { type Error; fn encode_gray8(self, pixels: PixelSlice<'_, Gray<u8>>) -> Result<EncodeOutput, Self::Error>; }
// ... EncodeRgb16, EncodeRgba16, EncodeGray16, EncodeRgbF16, EncodeRgbaF16,
//     EncodeRgbF32, EncodeRgbaF32, EncodeGrayF32
```

f16 traits use type-erased `PixelSlice<'_>` because the `rgb` crate has no
half-float type.

**Choosing between the two approaches:**

- Use per-format when you know your pixel type at compile time (most app code)
- Use `Encoder` for generic pipelines that handle any format (e.g. `zencodecs` dispatch)
- Check `EncoderConfig::supported_descriptors()` to know which per-format traits
  are implemented

Codec format matrix:

```
              Rgb8  Rgba8  Gray8  Rgb16  Rgba16  Gray16  RgbF16  RgbaF16  RgbF32  RgbaF32  GrayF32
JPEG           ✓             ✓
WebP           ✓      ✓
GIF                   ✓
PNG            ✓      ✓      ✓      ✓       ✓      ✓
AVIF           ✓      ✓                                                    ✓        ✓
JXL            ✓      ✓      ✓      ✓       ✓      ✓      ✓        ✓      ✓        ✓        ✓
```

### `FrameEncoder` — type-erased animation encode

```rust
trait FrameEncoder: Sized {
    type Error: ... + From<UnsupportedOperation>;

    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), Self::Error>;
    fn push_encode_frame(&mut self, frame: EncodeFrame<'_>) -> Result<(), Self::Error>;
    fn begin_frame(&mut self, duration_ms: u32) -> Result<(), Self::Error>;
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), Self::Error>;
    fn end_frame(&mut self) -> Result<(), Self::Error>;
    fn pull_frame(&mut self, duration_ms: u32,
        source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize) -> Result<(), Self::Error>;
    fn with_loop_count(&mut self, count: Option<u32>);
    fn finish(self) -> Result<EncodeOutput, Self::Error>;
}
```

Same three paths per frame: whole-frame push, row push, or pull.
`push_encode_frame` adds sub-canvas positioning via `EncodeFrame` (carrying
`frame_rect`, `blend`, `disposal`).

Per-format frame traits (`FrameEncodeRgb8`, `FrameEncodeRgba8`) are simpler —
just `push_frame_rgb8` + `finish_rgb8`.

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

    // Type-erased convenience (provided default implementations):
    fn dyn_decoder(self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<DynDecoder<'a>, BoxedError> where Self: 'a;
    fn dyn_frame_decoder(self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<DynFrameDecoder<'a>, BoxedError> where Self: 'a;
}
```

**Key difference from encode: `Dec`, `StreamDec`, `FrameDec` DO have trait
bounds** (`Decode<Error = Self::Error>`, etc.). Decode output format is always
type-erased — you don't know the pixel format until you probe the file.

**Format negotiation via `preferred`:** The caller passes a ranked
`&[PixelDescriptor]` list. The decoder picks the first it can produce without
lossy conversion. Pass `&[]` for native format.

**Data binding at the job level:** All executor constructors take `data: &'a [u8]`
here, not on the executor. This means `Decode::decode()` takes no arguments —
data was already bound. This simplifies the executor interface and prepares for
future IO-read sources.

**`push_decoder`** has a default implementation that creates a decoder, decodes,
then copies rows into the sink. Codecs with native row streaming override for
zero-copy.

### `Decode` — single-image decode

```rust
trait Decode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    fn decode(self) -> Result<DecodeOutput, Self::Error>;
}
```

Returns `DecodeOutput` which wraps a `PixelBuffer` + `ImageInfo`. The output
has convenience methods: `as_rgb8()` → `Option<ImgRef<'_, Rgb<u8>>>` for
zero-copy typed access, `to_rgb8()` for converting access.

### `StreamingDecode` — scanline-batch pull iterator

```rust
trait StreamingDecode {
    type Error: core::error::Error + Send + Sync + 'static;
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error>;
    fn info(&self) -> &ImageInfo;
}
```

Yields `(start_row, strip_pixels)`. Strip height is codec-determined: MCU height
for JPEG, single scanline for PNG, full image for simple formats.

`impl StreamingDecode for ()` is the trivial rejection type — codecs that don't
support streaming set `type StreamDec = ()` and return `Err` from
`streaming_decoder()`. Zero-cost at the type level.

### `FrameDecode` — animation pull iterator

```rust
trait FrameDecode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn frame_count(&self) -> Option<u32>;
    fn loop_count(&self) -> Option<u32>;
    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, Self::Error>;
    fn next_frame_to_sink(&mut self, sink: &mut dyn DecodeRowSink)
        -> Result<Option<OutputInfo>, Self::Error>;
}
```

`DecodeFrame` carries `PixelBuffer` + `Arc<ImageInfo>` (shared across frames) +
compositing info (`blend`, `disposal`, `frame_rect`, `required_frame`).

### `DecodeRowSink` — zero-copy row sink

```rust
trait DecodeRowSink {
    fn demand(&mut self, y: u32, height: u32, width: u32, descriptor: PixelDescriptor)
        -> PixelSliceMut<'_>;
}
```

Lending pattern: the sink provides mutable buffers, the codec fills them. The
sink controls stride — it can return SIMD-aligned buffers (stride padded to 64
bytes). Object-safe (`&mut dyn DecodeRowSink`).

---

## Type erasure via `dyn_*` methods

The `dyn_*` default methods on `EncodeJob` and `DecodeJob` erase the concrete
codec type into boxed closures. All codec-specific and universal configuration
happens *before* the erasure point — the only line that changes is
`encoder()` → `dyn_encoder()`.

**Type aliases:**

| Alias | Shape |
|-------|-------|
| `BoxedError` | `Box<dyn Error + Send + Sync>` |
| `DynEncoder<'a>` | `Box<dyn FnOnce(PixelSlice<'_>) -> Result<EncodeOutput, BoxedError> + 'a>` |
| `DynDecoder<'a>` | `Box<dyn FnOnce() -> Result<DecodeOutput, BoxedError> + 'a>` |
| `DynFrameEncoder<'a>` | `Box<dyn FnMut(Option<EncodeFrame<'_>>) -> Result<Option<EncodeOutput>, BoxedError> + 'a>` |
| `DynFrameDecoder<'a>` | `Box<dyn FnMut() -> Result<Option<DecodeFrame>, BoxedError> + 'a>` |

**Concrete vs erased — one line changes:**

```rust
// ── CONCRETE ──
let output = JpegConfig::new()
    .set_chroma_subsampling(Yuv444)  // codec-specific
    .with_generic_quality(92.0)       // universal trait
    .job()
    .with_metadata(&meta)
    .encoder()?                       // returns JpegEncoder
    .encode(pixels)?;

// ── ERASED ──
let output = JpegConfig::new()
    .set_chroma_subsampling(Yuv444)  // identical
    .with_generic_quality(92.0)       // identical
    .job()
    .with_metadata(&meta)
    .dyn_encoder()?                   // returns DynEncoder<'_>
    (pixels)?;                        // call the closure
```

**Multi-format dispatch (the real payoff):**

```rust
fn encode_to_format(
    format: ImageFormat,
    pixels: PixelSlice<'_>,
    quality: f32,
) -> Result<EncodeOutput, BoxedError> {
    match format {
        ImageFormat::Jpeg => JpegConfig::new()
            .with_generic_quality(quality)
            .job().dyn_encoder()?(pixels),
        ImageFormat::WebP => WebpConfig::new()
            .with_generic_quality(quality)
            .job().dyn_encoder()?(pixels),
        ImageFormat::Png => PngConfig::new()
            .with_lossless(true)
            .job().dyn_encoder()?(pixels),
        _ => todo!(),
    }
}
```

No generics in the function signature. Each arm uses its concrete config type
for codec-specific tuning, then erases at `dyn_encoder()`.

**Frame encoder** uses an option protocol: `Some(frame)` pushes, `None`
finalizes:

```rust
let mut enc = config.job().dyn_frame_encoder()?;
enc(Some(frame1))?;
enc(Some(frame2))?;
let output = enc(None)?.unwrap();
```

**Frame decoder** is a pull iterator:

```rust
let mut next = config.job().dyn_frame_decoder(data, &[])?;
while let Some(frame) = next()? {
    // process frame
}
```

---

## Format negotiation

The `preferred: &[PixelDescriptor]` parameter on decode methods is a ranked
list — the caller's wish list of output formats. The decoder picks the first
it can produce without lossy conversion. Pass `&[]` for the decoder's native
format.

Three shared helpers standardize the matching logic so every codec behaves
consistently:

```rust
// Decode side: pick best output format from caller's preferences
let format = negotiate_pixel_format(preferred, &available_for_this_image);

// Encode side: check if encoder accepts the caller's pixel data
if let Some(fmt) = best_encode_format(source_descriptor, supported) {
    // encoder can handle this format
}

// Quick existence check
if is_format_available(PixelFormat::Rgba8, &supported_descriptors) {
    // codec supports RGBA8
}
```

`negotiate_pixel_format` matching tiers (per preference, in order):
1. **Exact match** — all fields identical
2. **Format match** — same `PixelFormat` (channel type + layout), ignoring
   transfer function, primaries, alpha mode, and signal range

If nothing matches, returns `available[0]` (the decoder's default).

Decoder implementations call `negotiate_pixel_format` inside their
`decoder()` / `frame_decoder()` / etc. methods. Callers construct preference
lists from `PixelDescriptor` constants:

```rust
// "I want RGBA8 for compositing, accept RGB8 as fallback"
let preferred = &[PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGB8_SRGB];

let output = config.job()
    .decoder(data, preferred)?
    .decode()?;

// Check what the decoder actually produced
let actual = output.pixels().descriptor();
```

---

## Cross-cutting types

### `CodecCapabilities`

Const-constructible struct with ~30 boolean flags and range fields. Returned as
`&'static CodecCapabilities`. Callers discover behavior before calling methods
that might be unsupported.

```rust
static CAPS: CodecCapabilities = CodecCapabilities::new()
    .with_encode_icc(true)
    .with_encode_exif(true)
    .with_encode_cancel(true)
    .with_lossy(true)
    .with_lossless(true)
    .with_quality_range(0.0, 100.0)
    .with_effort_range(1, 9);
```

Flags: `encode_icc`, `encode_exif`, `encode_xmp`, `decode_icc`, `decode_exif`,
`decode_xmp`, `encode_cancel`, `decode_cancel`, `native_gray`, `cheap_probe`,
`encode_animation`, `decode_animation`, `native_16bit`, `lossless`, `lossy`,
`hdr`, `encode_cicp`, `decode_cicp`, `enforces_max_pixels`,
`enforces_max_memory`, `enforces_max_file_size`, `native_f32`, `native_alpha`,
`decode_into`, `row_level_encode`, `pull_encode`, `row_level_decode`,
`row_level_frame_encode`, `pull_frame_encode`, `frame_decode_into`,
`row_level_frame_decode`.

### `ResourceLimits`

Optional caps on pixels, memory, file size, frames, duration. Builder pattern,
`Copy`. Has `check_*` methods for early rejection:

```rust
let limits = ResourceLimits::none()
    .with_max_pixels(100_000_000)
    .with_max_memory(512 * 1024 * 1024);

let info = job.probe(data)?;
limits.check_image_info(&info)?;
```

### `DecodePolicy` / `EncodePolicy`

Per-job security flags. Three-valued (`None`/`Some(true)`/`Some(false)`) with
`resolve_*()` methods that take a codec default. Named levels: `strict()`,
`permissive()`, `none()`.

```rust
let policy = DecodePolicy::strict().with_allow_icc(true);
// allow ICC for color management, deny everything else
```

### `UnsupportedOperation` / `HasUnsupportedOperation`

Standard error reporting for unsupported paths. The `From<UnsupportedOperation>`
bound on `Encoder::Error` / `FrameEncoder::Error` enables default method impls
to return proper errors.

### Error tracking (`whereat`)

```rust
pub use whereat::{At, AtTrace, AtTraceable, ErrorAtExt, ResultAtExt};
```

Pattern: `type Error = At<MyCodecError>` then `.at()` captures file:line.

---

## Output types

### `EncodeOutput`

Encoded bytes (`Vec<u8>`) + `ImageFormat`. `into_vec()`, `data()`, `AsRef<[u8]>`.

### `DecodeOutput`

Decoded pixels (`PixelBuffer`) + `ImageInfo` + optional type-erased extras
(`Box<dyn Any + Send>`). Zero-copy typed access: `as_rgb8()` →
`Option<ImgRef<'_, Rgb<u8>>>`. Converting access: `to_rgb8()` →
`PixelBuffer<Rgb<u8>>`.

### `DecodeFrame`

Animation frame with `PixelBuffer`, `Arc<ImageInfo>` (shared across frames),
delay, index, and compositing metadata (`blend`, `disposal`, `frame_rect`,
`required_frame`).

### `EncodeFrame<'a>`

Animation frame for encoding with `PixelSlice`, duration, and compositing
parameters (`frame_rect`, `blend`, `disposal`).

---

## Pixel types (from `zenpixels`)

All pixel interchange types come from the `zenpixels` crate. Codec crates
depend on `zenpixels` directly. zencodec-types uses these types in trait
signatures but does not re-export them.

- **`PixelSlice<'a, P = ()>`** — format-erased pixel buffer view. `P = ()` is
  type-erased (runtime descriptor), `P = Rgb<u8>` is compile-time typed.
  `From<PixelSlice<'a, P>>` converts typed → erased.
- **`PixelSliceMut<'a, P = ()>`** — mutable version.
- **`PixelBuffer`** — owned pixel buffer (`Vec<u8>` backing).
- **`PixelDescriptor`** — describes pixel format: channel layout, type, signal
  range, transfer function, color primaries, alpha mode. Named constants:
  `RGB8_SRGB`, `RGBA8_SRGB`, `RGBAF32_LINEAR`, etc.

---

## Example: type-erased end-to-end (no generics)

This shows a concrete codec implementing and using the traits without any
generic type parameters. Every type is named, every step is explicit.

```rust
// ============================================================
// CODEC IMPLEMENTOR SIDE — defining a minimal "FooCodec"
// ============================================================
use std::sync::Arc;
use zencodec_types::*;
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice, PixelSliceMut};

// --- Error type ---

#[derive(Debug, thiserror::Error)]
enum FooError {
    #[error("unsupported: {0}")]
    Unsupported(#[from] UnsupportedOperation),
    #[error("invalid input")]
    InvalidInput,
}

// --- Config (Layer 1: 'static, Clone, Send, Sync) ---

#[derive(Clone)]
struct FooEncoderConfig {
    quality: f32,
}

#[derive(Clone)]
struct FooDecoderConfig;

// --- Job (Layer 2: borrows per-operation data) ---

struct FooEncodeJob<'a> {
    quality: f32,
    metadata: Option<&'a MetadataView<'a>>,
    limits: ResourceLimits,
}

struct FooDecodeJob<'a> {
    limits: ResourceLimits,
    _life: std::marker::PhantomData<&'a ()>,
}

// --- Executor (Layer 3: borrows input, consumes self) ---

struct FooEncoder {
    quality: f32,
}

struct FooDecoder<'a> {
    data: &'a [u8],
}

// --- Trait implementations ---

impl EncoderConfig for FooEncoderConfig {
    type Error = At<FooError>;
    type Job<'a> = FooEncodeJob<'a> where Self: 'a;

    fn format() -> ImageFormat { ImageFormat::Unknown }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB]
    }

    fn with_generic_quality(mut self, quality: f32) -> Self {
        self.quality = quality;
        self
    }

    fn generic_quality(&self) -> Option<f32> { Some(self.quality) }

    fn job(&self) -> FooEncodeJob<'_> {
        FooEncodeJob {
            quality: self.quality,
            metadata: None,
            limits: ResourceLimits::none(),
        }
    }
}

impl<'a> EncodeJob<'a> for FooEncodeJob<'a> {
    type Error = At<FooError>;
    type Enc = FooEncoder;        // implements Encoder + EncodeRgb8
    type FrameEnc = FooEncoder;   // reuse (or a separate type)

    fn with_stop(self, _stop: &'a dyn Stop) -> Self { self }
    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits; self
    }
    fn with_metadata(mut self, meta: &'a MetadataView<'a>) -> Self {
        self.metadata = Some(meta); self
    }
    fn encoder(self) -> Result<FooEncoder, At<FooError>> {
        Ok(FooEncoder { quality: self.quality })
    }
    fn frame_encoder(self) -> Result<FooEncoder, At<FooError>> {
        Err(FooError::Unsupported(UnsupportedOperation::AnimationEncode)).at()
    }
}

// Type-erased encode: accepts any pixel format at runtime
impl Encoder for FooEncoder {
    type Error = At<FooError>;

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, At<FooError>> {
        // Real codec would compress pixels.data() here.
        // Descriptor tells us the format at runtime.
        let _desc = pixels.descriptor();
        let _w = pixels.width();
        let _h = pixels.height();
        let fake_output = vec![0xFF; 64]; // placeholder compressed bytes
        Ok(EncodeOutput::new(fake_output, ImageFormat::Unknown))
    }
}

// Per-format typed encode: compile-time guarantee of RGB8 support
impl EncodeRgb8 for FooEncoder {
    type Error = At<FooError>;

    fn encode_rgb8(self, pixels: PixelSlice<'_, Rgb<u8>>)
        -> Result<EncodeOutput, At<FooError>>
    {
        // pixels is statically known to be Rgb<u8> — no runtime check needed.
        // Convert typed → erased and delegate:
        self.encode(pixels.into())
    }
}

impl DecoderConfig for FooDecoderConfig {
    type Error = At<FooError>;
    type Job<'a> = FooDecodeJob<'a> where Self: 'a;

    fn format() -> ImageFormat { ImageFormat::Unknown }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB]
    }

    fn job(&self) -> FooDecodeJob<'_> {
        FooDecodeJob { limits: ResourceLimits::none(), _life: Default::default() }
    }
}

impl<'a> DecodeJob<'a> for FooDecodeJob<'a> {
    type Error = At<FooError>;
    type Dec = FooDecoder<'a>;
    type StreamDec = ();  // no streaming support
    type FrameDec = FooDecoder<'a>;  // reuse for frames (or a separate type)

    fn with_stop(self, _stop: &'a dyn Stop) -> Self { self }
    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits; self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, At<FooError>> {
        // Real codec would parse headers here
        Ok(ImageInfo::new(64, 64, ImageFormat::Unknown))
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, At<FooError>> {
        Ok(OutputInfo::full_decode(64, 64, PixelDescriptor::RGB8_SRGB))
    }

    fn decoder(self, data: &'a [u8], _preferred: &[PixelDescriptor])
        -> Result<FooDecoder<'a>, At<FooError>>
    {
        Ok(FooDecoder { data })
    }

    fn streaming_decoder(self, _data: &'a [u8], _preferred: &[PixelDescriptor])
        -> Result<(), At<FooError>>
    {
        Err(FooError::Unsupported(UnsupportedOperation::RowLevelDecode)).at()
    }

    fn frame_decoder(self, data: &'a [u8], _preferred: &[PixelDescriptor])
        -> Result<FooDecoder<'a>, At<FooError>>
    {
        Ok(FooDecoder { data })
    }
}

impl<'a> Decode for FooDecoder<'a> {
    type Error = At<FooError>;

    fn decode(self) -> Result<DecodeOutput, At<FooError>> {
        // Real codec would decompress self.data here.
        let img = imgref::ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 64 * 64], 64, 64);
        let pixels: PixelBuffer = PixelBuffer::from_imgvec(img).into();
        let info = ImageInfo::new(64, 64, ImageFormat::Unknown);
        Ok(DecodeOutput::new(pixels, info))
    }
}

impl<'a> FrameDecode for FooDecoder<'a> {
    type Error = At<FooError>;

    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, At<FooError>> {
        Ok(None) // single-frame codec, no animation
    }
}

// ============================================================
// CALLER SIDE — using the codec with concrete types
// ============================================================
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Encode ---

    // 1. Create config (reusable, thread-safe)
    let config = FooEncoderConfig { quality: 85.0 };

    // 2. Create job (borrows per-operation data)
    let limits = ResourceLimits::none().with_max_pixels(100_000_000);
    let job: FooEncodeJob<'_> = config
        .job()                          // FooEncodeJob<'_>
        .with_limits(limits);

    // 3. Create executor
    let encoder: FooEncoder = job.encoder()?;  // FooEncoder

    // 4. Encode — type-erased path (any pixel format)
    let img = imgref::ImgVec::new(vec![Rgb { r: 128u8, g: 64, b: 32 }; 64 * 64], 64, 64);
    let typed: PixelSlice<'_, Rgb<u8>> = PixelSlice::from(img.as_ref());
    let erased: PixelSlice<'_> = typed.into();  // typed → erased via From
    let output: EncodeOutput = encoder.encode(erased)?;
    println!("Encoded {} bytes", output.len());

    // Or: per-format typed path (compile-time guarantee)
    let encoder2: FooEncoder = config.job().encoder()?;
    let typed_pixels: PixelSlice<'_, Rgb<u8>> = PixelSlice::from(img.as_ref());
    let output2: EncodeOutput = encoder2.encode_rgb8(typed_pixels)?;

    // --- Decode ---

    let dec_config = FooDecoderConfig;
    let fake_data: &[u8] = &[0u8; 256];

    // 1. Probe (cheap header parse)
    let job: FooDecodeJob<'_> = dec_config.job();
    let info: ImageInfo = job.probe(fake_data)?;
    println!("Image: {}x{}", info.width, info.height);

    // 2. Check limits before decoding
    let limits = ResourceLimits::none().with_max_pixels(10_000_000);
    limits.check_image_info(&info)?;

    // 3. Create decoder with format preference
    let job: FooDecodeJob<'_> = dec_config.job().with_limits(limits);
    let preferred = &[PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGB8_SRGB];
    let decoder: FooDecoder<'_> = job.decoder(fake_data, preferred)?;

    // 4. Decode
    let output: DecodeOutput = decoder.decode()?;
    println!("Decoded: {}x{}, {:?}", output.width(), output.height(), output.descriptor());

    // Zero-copy typed access (returns None if format doesn't match)
    if let Some(rgb_view) = output.as_rgb8() {
        println!("Got RGB8: {}x{}", rgb_view.width(), rgb_view.height());
    }

    // Converting access (always succeeds)
    let rgba_buf: PixelBuffer<Rgba<u8>> = output.to_rgba8();

    // --- Decode with push sink (zero-copy row streaming) ---

    struct CollectSink { buf: Vec<u8> }

    impl DecodeRowSink for CollectSink {
        fn demand(&mut self, _y: u32, height: u32, width: u32, desc: PixelDescriptor)
            -> PixelSliceMut<'_>
        {
            let stride = width as usize * desc.bytes_per_pixel();
            self.buf.resize(height as usize * stride, 0);
            PixelSliceMut::new(&mut self.buf, width, height, stride, desc)
                .expect("valid buffer")
        }
    }

    let mut sink = CollectSink { buf: Vec::new() };
    let job = dec_config.job();
    let info: OutputInfo = job.push_decoder(fake_data, &mut sink, &[])?;
    println!("Push-decoded into sink: {}x{}", info.width, info.height);

    Ok(())
}
```

Every variable has an explicit type annotation to show exactly what flows
through the pipeline. In practice you'd let type inference handle most of
these — the concrete types are spelled out here so you can see the full
chain: `FooEncoderConfig` → `FooEncodeJob<'_>` → `FooEncoder` →
`EncodeOutput`.

---

## Design rationale

### Why three layers?

A single `encode(config, pixels)` function can't handle:
- Sharing config across threads (needs `Clone + Send + Sync`, no lifetimes)
- Borrowing a cancellation token for one operation (needs `'a`)
- Binding input data for streaming decode (needs another `'a`)

Three layers give each concern its own lifetime scope.

### Why unbounded `Enc`/`FrameEnc`?

If `EncodeJob::Enc` required `Encoder`, every codec's encoder would have to
implement the type-erased path. But JPEG only needs `EncodeRgb8 + EncodeGray8`.
By leaving `Enc: Sized` with no trait bounds, codecs implement whichever
traits make sense. The caller picks which trait to use.

The tradeoff: in generic code over `T: EncoderConfig`, you can't write
`T::Job::Enc: EncodeRgb8` as a bound without propagating it. The `dyn_encoder()`
default method addresses this — it requires `Self::Enc: Encoder` only where
called, not in the trait definition, and erases the concrete type into a
`DynEncoder<'a>` closure.

### Why bounded `Dec`/`StreamDec`/`FrameDec`?

Decode output format is always discovered at runtime (from the file). There's no
per-format decode trait like `DecodeRgb8` — the caller doesn't know the pixel
format until after probing. So `Dec: Decode` is always the right bound.

### Why `impl StreamingDecode for ()`?

Codecs that don't support streaming set `type StreamDec = ()`. The `()`
impl returns `Err(UnsupportedOperation::RowLevelDecode)` from every method.
This avoids boxing, `Option` wrapping, or feature flags — it's zero-cost
rejection at the type level.

### Why `preferred: &[PixelDescriptor]` for format negotiation?

The decoder picks the first format from the caller's ranked list that it can
produce without lossy conversion. This avoids a two-step "probe format, then
convert" dance. Pass `&[]` for the decoder's native format. The shared
`negotiate_pixel_format()` function standardizes the matching logic so every
codec behaves consistently.

### Why `DecodeRowSink` lending pattern?

The codec writes directly into caller-owned, potentially SIMD-aligned buffers.
No intermediate allocation. The `PixelSliceMut` carries stride so the codec
respects the sink's layout. Object-safe for use with `&mut dyn DecodeRowSink`.

### Why `Arc<ImageInfo>` in `DecodeFrame`?

Container metadata (ICC profiles, EXIF, XMP) is shared across all frames of
an animation. `Arc` sharing avoids cloning megabytes of ICC data per frame.

## License

Apache-2.0 OR MIT
