# zencodec-types API Specification

Shared traits and types for zen* image codecs. This is the canonical reference
for the public API surface.

`#![no_std]` + `alloc`. `#![forbid(unsafe_code)]`.

### zenpixels: use but never re-export

`zenpixels` defines the cross-crate pixel interchange types: `PixelDescriptor`,
`PixelFormat`, `PixelSlice`, `PixelSliceMut`, `PixelBuffer`, `ChannelLayout`,
`ChannelType`, `TransferFunction`, `ColorPrimaries`, `AlphaMode`, `SignalRange`,
`InterleaveFormat`.

**All crates in the zen ecosystem MUST use `zenpixels` types directly.**
zencodec-types uses them in trait signatures but callers and codec
implementors should depend on `zenpixels` directly and use `zenpixels::` paths
in their public APIs.

---

## Trait hierarchy

```text
ENCODE:
                                 ┌→ Enc (Encoder)
EncoderConfig → EncodeJob<'a> ──┤
                                 └→ FullFrameEnc (FullFrameEncoder, 'static)

DECODE:
                                 ┌→ Dec (Decode)
DecoderConfig → DecodeJob<'a> ──┤→ StreamDec (StreamingDecode)
                                 └→ FullFrameDec (FullFrameDecoder, 'static)
```

Each layer has object-safe `Dyn*` variants for codec-agnostic dispatch:

```text
DynEncoderConfig → DynEncodeJob → DynEncoder / DynFullFrameEncoder
DynDecoderConfig → DynDecodeJob → DynDecoder / DynStreamingDecoder / DynFullFrameDecoder
```

Blanket impls generate the dyn API automatically from the generic traits.

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
    fn capabilities() -> &'static EncodeCapabilities;    // default: EMPTY

    // Universal knobs (default no-op, codec overrides what it supports)
    fn with_generic_quality(self, quality: f32) -> Self;  // default: self
    fn with_generic_effort(self, effort: i32) -> Self;    // default: self
    fn with_lossless(self, lossless: bool) -> Self;       // default: self
    fn with_alpha_quality(self, quality: f32) -> Self;    // default: self
    fn generic_quality(&self) -> Option<f32>;   // default: None
    fn generic_effort(&self) -> Option<i32>;    // default: None
    fn is_lossless(&self) -> Option<bool>;      // default: None
    fn alpha_quality(&self) -> Option<f32>;     // default: None

    fn job(&self) -> Self::Job<'_>;
}
```

### `EncodeJob<'a>` (per-operation, borrows metadata/limits/stop)

```rust
trait EncodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Enc: Sized;                    // single-image encoder
    type FullFrameEnc: Sized + 'static; // animation encoder

    fn with_stop(self, stop: &'a dyn Stop) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_policy(self, policy: EncodePolicy) -> Self;         // default: self
    fn with_metadata(self, meta: &Metadata) -> Self;
    fn with_canvas_size(self, width: u32, height: u32) -> Self; // default: self
    fn with_loop_count(self, count: Option<u32>) -> Self;       // default: self

    // Codec-specific extensions (downcasted by callers who know the codec)
    fn extensions(&self) -> Option<&dyn Any>;          // default: None
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>; // default: None

    fn encoder(self) -> Result<Self::Enc, Self::Error>;
    fn full_frame_encoder(self) -> Result<Self::FullFrameEnc, Self::Error>;

    // Type-erased convenience (default impls via shims)
    fn dyn_encoder(self) -> Result<Box<dyn DynEncoder + 'a>, BoxedError>
        where Self: 'a, Self::Enc: Encoder;
    fn dyn_full_frame_encoder(self) -> Result<Box<dyn DynFullFrameEncoder>, BoxedError>
        where Self: 'a, Self::FullFrameEnc: FullFrameEncoder;
}
```

### `Encoder` (type-erased single-image encode)

```rust
trait Encoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn reject(op: UnsupportedOperation) -> Self::Error;
    fn preferred_strip_height(&self) -> u32;    // default: 0
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;

    // Hot path: encode from mutable sRGB RGBA8 buffer (encoder may modify in-place)
    fn encode_srgba8(
        self, data: &mut [u8], make_opaque: bool,
        width: u32, height: u32, stride_pixels: u32,
    ) -> Result<EncodeOutput, Self::Error>;  // default: wraps encode()

    // Row-level push (mutually exclusive with encode)
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), Self::Error>;  // default: Err
    fn finish(self) -> Result<EncodeOutput, Self::Error>;                       // default: Err

    // Pull from source callback
    fn encode_from(
        self, source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, Self::Error>;  // default: Err
}
```

Three mutually exclusive paths: `encode()`/`encode_srgba8()`, `push_rows()+finish()`, `encode_from()`.

### `FullFrameEncoder` (animation encode)

```rust
trait FullFrameEncoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn reject(op: UnsupportedOperation) -> Self::Error;
    fn push_frame(
        &mut self, pixels: PixelSlice<'_>, duration_ms: u32, stop: Option<&dyn Stop>,
    ) -> Result<(), Self::Error>;
    fn finish(self, stop: Option<&dyn Stop>) -> Result<EncodeOutput, Self::Error>;
}
```

Full-canvas frames only. Animation encoder is `'static` — it owns its data.
Codecs without animation set `type FullFrameEnc = ()` (unit implements
`FullFrameEncoder` with all methods returning `Err`).

---

## Decode traits

### `DecoderConfig` (codec config, `Clone + Send + Sync`)

```rust
trait DecoderConfig: Clone + Send + Sync {
    type Error: core::error::Error + Send + Sync + 'static;
    type Job<'a>: DecodeJob<'a, Error = Self::Error> where Self: 'a;

    fn formats() -> &'static [ImageFormat]; // may return multiple
    fn supported_descriptors() -> &'static [PixelDescriptor];
    fn capabilities() -> &'static DecodeCapabilities;  // default: EMPTY

    fn job(&self) -> Self::Job<'_>;
}
```

### `DecodeJob<'a>` (per-operation, holds limits/stop/hints)

```rust
trait DecodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Dec: Decode<Error = Self::Error>;
    type StreamDec: StreamingDecode<Error = Self::Error>;
    type FullFrameDec: FullFrameDecoder<Error = Self::Error> + 'static;

    fn with_stop(self, stop: &'a dyn Stop) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_policy(self, policy: DecodePolicy) -> Self;  // default: self

    // Probing (needs limits + stop context)
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;      // header only
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>; // default: probe()

    // Decode hints (optional, decoder may ignore)
    fn with_crop_hint(self, x: u32, y: u32, width: u32, height: u32) -> Self;  // default: self
    fn with_orientation(self, hint: OrientationHint) -> Self;                   // default: self
    fn with_start_frame_index(self, index: u32) -> Self;                        // default: self

    // Codec-specific extensions
    fn extensions(&self) -> Option<&dyn Any>;          // default: None
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>; // default: None

    // Output prediction
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, Self::Error>;

    // Executor creation — all bind data + preferred here
    // data is Cow<'a, [u8]> — pass Cow::Borrowed for zero-copy, Cow::Owned to donate
    fn decoder(self, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
        -> Result<Self::Dec, Self::Error>;
    fn push_decoder(self, data: Cow<'a, [u8]>, sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor]) -> Result<OutputInfo, Self::Error>;
    fn streaming_decoder(self, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
        -> Result<Self::StreamDec, Self::Error>;
    fn full_frame_decoder(self, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
        -> Result<Self::FullFrameDec, Self::Error>;

    // Type-erased convenience (default impls via shims)
    fn dyn_decoder(...) -> Result<Box<dyn DynDecoder + 'a>, BoxedError>;
    fn dyn_full_frame_decoder(...) -> Result<Box<dyn DynFullFrameDecoder>, BoxedError>;
    fn dyn_streaming_decoder(...) -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError>;
}
```

`preferred` is a ranked list of desired output formats — the decoder picks the
first it can produce without lossy conversion. Pass `&[]` for native format.

### `Decode` (single-image decode, returns owned pixels)

```rust
trait Decode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    fn decode(self) -> Result<DecodeOutput, Self::Error>;
}
```

### `StreamingDecode` (scanline-batch decode, pull iterator)

```rust
trait StreamingDecode {
    type Error: core::error::Error + Send + Sync + 'static;
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error>;
    fn info(&self) -> &ImageInfo;
}
```

`impl StreamingDecode for ()` is the rejection stub — set `type StreamDec = ()`
for codecs that don't support streaming.

### `FullFrameDecoder` (animation decode, composited full-canvas frames)

```rust
trait FullFrameDecoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn wrap_sink_error(err: SinkError) -> Self::Error;
    fn info(&self) -> &ImageInfo;
    fn frame_count(&self) -> Option<u32>;       // default: None
    fn loop_count(&self) -> Option<u32>;        // default: None

    fn render_next_frame(&mut self, stop: Option<&dyn Stop>)
        -> Result<Option<FullFrame<'_>>, Self::Error>;
    fn render_next_frame_owned(&mut self, stop: Option<&dyn Stop>)
        -> Result<Option<OwnedFullFrame>, Self::Error>;   // default: copies from render_next_frame
    fn render_next_frame_to_sink(&mut self, stop: Option<&dyn Stop>,
        sink: &mut dyn DecodeRowSink) -> Result<Option<OutputInfo>, Self::Error>;
}
```

Use `Unsupported<E>` as the associated type for codecs without animation support.

### `DecodeRowSink` (zero-copy row sink, push-based)

```rust
trait DecodeRowSink {
    fn begin(&mut self, width: u32, height: u32, descriptor: PixelDescriptor)
        -> Result<(), SinkError>;  // default: Ok(())
    fn provide_next_buffer(&mut self, y: u32, height: u32, width: u32,
        descriptor: PixelDescriptor) -> Result<PixelSliceMut<'_>, SinkError>;
    fn finish(&mut self) -> Result<(), SinkError>;  // default: Ok(())
}
```

The codec calls `begin()`, then `provide_next_buffer()` per strip, writes
decoded pixels via `PixelSliceMut::row_mut()`, then calls `finish()`. The sink
controls stride (can return SIMD-aligned buffers). Object-safe.

`SinkError = Box<dyn core::error::Error + Send + Sync>`

---

## Dyn dispatch traits

### Encode side

```rust
trait DynEncoderConfig: Send + Sync {
    fn as_any(&self) -> &dyn Any;  // downcast to concrete config
    fn format(&self) -> ImageFormat;
    fn supported_descriptors(&self) -> &'static [PixelDescriptor];
    fn capabilities(&self) -> &'static EncodeCapabilities;
    fn dyn_job(&self) -> Box<dyn DynEncodeJob<'_> + '_>;
}

trait DynEncodeJob<'a> {
    fn set_stop(&mut self, stop: &'a dyn Stop);
    fn set_limits(&mut self, limits: ResourceLimits);
    fn set_policy(&mut self, policy: EncodePolicy);
    fn set_metadata(&mut self, meta: &Metadata);
    fn set_canvas_size(&mut self, width: u32, height: u32);
    fn set_loop_count(&mut self, count: Option<u32>);
    fn extensions(&self) -> Option<&dyn Any>;
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>;
    fn into_encoder(self: Box<Self>) -> Result<Box<dyn DynEncoder + 'a>, BoxedError>;
    fn into_full_frame_encoder(self: Box<Self>) -> Result<Box<dyn DynFullFrameEncoder>, BoxedError>;
}

trait DynEncoder {
    fn preferred_strip_height(&self) -> u32;
    fn encode(self: Box<Self>, pixels: PixelSlice<'_>) -> Result<EncodeOutput, BoxedError>;
    fn encode_srgba8(self: Box<Self>, data: &mut [u8], make_opaque: bool,
        width: u32, height: u32, stride_pixels: u32) -> Result<EncodeOutput, BoxedError>;
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), BoxedError>;
    fn finish(self: Box<Self>) -> Result<EncodeOutput, BoxedError>;
    fn encode_from(self: Box<Self>,
        source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize) -> Result<EncodeOutput, BoxedError>;
}

trait DynFullFrameEncoder {
    fn as_any(&self) -> &dyn Any;       // downcast to concrete encoder
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32,
        stop: Option<&dyn Stop>) -> Result<(), BoxedError>;
    fn finish(self: Box<Self>, stop: Option<&dyn Stop>) -> Result<EncodeOutput, BoxedError>;
}
```

### Decode side

```rust
trait DynDecoderConfig: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn formats(&self) -> &'static [ImageFormat];
    fn supported_descriptors(&self) -> &'static [PixelDescriptor];
    fn capabilities(&self) -> &'static DecodeCapabilities;
    fn dyn_job(&self) -> Box<dyn DynDecodeJob<'_> + '_>;
}

trait DynDecodeJob<'a> {
    fn set_stop(&mut self, stop: &'a dyn Stop);
    fn set_limits(&mut self, limits: ResourceLimits);
    fn set_policy(&mut self, policy: DecodePolicy);
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, BoxedError>;
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, BoxedError>;
    fn set_crop_hint(&mut self, x: u32, y: u32, width: u32, height: u32);
    fn set_orientation(&mut self, hint: OrientationHint);
    fn set_start_frame_index(&mut self, index: u32);
    fn extensions(&self) -> Option<&dyn Any>;
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>;
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, BoxedError>;
    fn into_decoder(self: Box<Self>, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynDecoder + 'a>, BoxedError>;
    fn push_decode(self: Box<Self>, data: Cow<'a, [u8]>,
        sink: &mut dyn DecodeRowSink, preferred: &[PixelDescriptor])
        -> Result<OutputInfo, BoxedError>;
    fn into_streaming_decoder(self: Box<Self>, data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError>;
    fn into_full_frame_decoder(self: Box<Self>, data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynFullFrameDecoder>, BoxedError>;
}

trait DynDecoder {
    fn decode(self: Box<Self>) -> Result<DecodeOutput, BoxedError>;
}

trait DynFullFrameDecoder {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
    fn info(&self) -> &ImageInfo;
    fn frame_count(&self) -> Option<u32>;
    fn loop_count(&self) -> Option<u32>;
    fn render_next_frame_owned(&mut self, stop: Option<&dyn Stop>)
        -> Result<Option<OwnedFullFrame>, BoxedError>;
    fn render_next_frame_to_sink(&mut self, stop: Option<&dyn Stop>,
        sink: &mut dyn DecodeRowSink) -> Result<Option<OutputInfo>, BoxedError>;
}

trait DynStreamingDecoder {
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, BoxedError>;
    fn info(&self) -> &ImageInfo;
}
```

### Downcasting rules

- `DynEncoderConfig`, `DynDecoderConfig`: `as_any()` — configs are `'static`
- `DynFullFrameEncoder`, `DynFullFrameDecoder`: `as_any()`, `as_any_mut()`, `into_any()` — frame decoders/encoders are `'static`
- `DynEncoder`, `DynDecoder`, `DynStreamingDecoder`: **no downcasting** — they borrow `'a` data

Use `extensions()`/`extensions_mut()` on jobs for codec-specific access through the dyn pipeline.

---

## Pixel types (from `zenpixels`)

These types are defined in `zenpixels` and used throughout the zen ecosystem.
All crates depend on `zenpixels` directly. See `zenpixels` documentation.

Key types: `PixelSlice<'a>`, `PixelSliceMut<'a>`, `PixelBuffer`, `PixelDescriptor`,
`PixelFormat`, `ChannelLayout`, `ChannelType`, `SignalRange`, `TransferFunction`,
`ColorPrimaries`, `AlphaMode`.

---

## Image metadata

### `ImageInfo`

Image metadata from probing or decoding. `#[non_exhaustive]`, `Clone + Debug + PartialEq`.

Fields: `width`, `height`, `format: ImageFormat`, `has_alpha`, `has_animation`,
`frame_count: Option<u32>`, `orientation: Orientation`,
`source_color: SourceColor`, `embedded_metadata: EmbeddedMetadata`,
`has_gain_map`, `gain_map_metadata: Option<GainMapMetadata>`,
`source_encoding: Option<Arc<dyn SourceEncodingDetails>>`,
`warnings: Vec<String>`.

Builder pattern: `ImageInfo::new(w, h, format).with_alpha(true).with_cicp(...)`.

Key methods: `display_width()`, `display_height()` (orientation-corrected),
`transfer_function()`, `color_primaries()`, `color_profile_source()`,
`color_context()`, `metadata() -> Metadata`,
`source_encoding_details() -> Option<&dyn SourceEncodingDetails>`.

`PartialEq` skips `source_encoding` (trait objects aren't comparable).

### `SourceColor`

Source color description. Fields: `cicp: Option<Cicp>`,
`icc_profile: Option<Arc<[u8]>>`, `bit_depth: Option<u8>`,
`channel_count: Option<u8>`, `content_light_level: Option<ContentLightLevel>`,
`mastering_display: Option<MasteringDisplay>`.

### `EmbeddedMetadata`

Non-color metadata blobs. Fields: `exif: Option<Vec<u8>>`, `xmp: Option<Vec<u8>>`.

### `Metadata`

Owned metadata for encode/decode roundtrip. Fields: `icc_profile`, `exif`, `xmp`
(`Option<Arc<[u8]>>`), `cicp`, `content_light_level`, `mastering_display` (Copy),
`orientation`, `resolution`. `#[non_exhaustive]`.

Methods: builder pattern (`with_icc()`, etc.), `transfer_function()`,
`color_primaries()`, `is_empty()`. `From<&ImageInfo>` conversion.

### `OutputInfo`

Predicted decoder output. Fields: `width`, `height`, `native_format: PixelDescriptor`,
`has_alpha`, `orientation_applied: Orientation`, `crop_applied: Option<[u32; 4]>`.

Methods: `full_decode()`, `buffer_size()`, `pixel_count()`.

### `Cicp`

ITU-T H.273 color description. Re-exported from `zenpixels`. Constants:
`SRGB`, `BT2100_PQ`, `BT2100_HLG`, `LINEAR_SRGB`, `DISPLAY_P3`, `DISPLAY_P3_PQ`.

### `ContentLightLevel` / `MasteringDisplay`

HDR metadata types (CEA-861.3 / SMPTE ST 2086).

### `Resolution` / `ResolutionUnit`

Physical resolution (DPI / pixels-per-cm / pixels-per-meter).

### `Orientation` / `OrientationHint`

EXIF orientation (1-8 enum) and decode-time orientation strategy
(`Preserve`, `Correct`, `CorrectAndTransform`, `ExactTransform`).

---

## Output types

### `EncodeOutput`

Encoded image bytes. `#[non_exhaustive]`.

Fields: `data: Vec<u8>`, `format: ImageFormat`, `mime_type`, `extension`,
`extras: Option<Box<dyn Any + Send>>`.

Methods: `new()`, `data()`, `into_vec()`, `format()`, `mime_type()`, `extension()`,
`with_extras<T>()`, `extras<T>()`, `take_extras<T>()`.

Clone drops extras. PartialEq/Eq skip extras.

### `DecodeOutput`

Decoded image with owned pixels. `#[non_exhaustive]`.

Fields: `pixels: PixelBuffer`, `info: ImageInfo`,
`source_encoding: Option<Box<dyn SourceEncodingDetails>>`,
`extras: Option<Box<dyn Any + Send>>`.

Methods: `pixels()`, `into_buffer()`, `info()`, `width()`, `height()`,
`has_alpha()`, `descriptor()`, `format()`, `color_context()`, `metadata()`,
`with_source_encoding_details<T>()`, `source_encoding_details()`,
`take_source_encoding_details()`,
`with_extras<T>()`, `extras<T>()`, `take_extras<T>()`.

### `FullFrame<'a>`

Borrowed animation frame. Fields: `pixels: PixelSlice<'a>`, `duration_ms: u32`,
`frame_index: u32`. Method: `to_owned_frame()`.

### `OwnedFullFrame`

Owned animation frame. Fields: `pixels: PixelBuffer`, `duration_ms: u32`,
`frame_index: u32`, `extras: Option<Box<dyn Any + Send>>`.

Methods: `pixels()`, `into_buffer()`, `as_full_frame()`,
`with_extras<T>()`, `extras<T>()`, `take_extras<T>()`.

---

## Format detection

### `ImageFormat`

```rust
enum ImageFormat {
    Jpeg, Png, Gif, WebP, Avif, Jxl, Heic, Bmp, Tiff, Ico, Pnm, Farbfeld, Qoi, Unknown,
    Custom(&'static ImageFormatDefinition),
}
```

Methods: `from_magic(data)`, `definition()`, `mime_type()`, `extension()`,
`display_name()`, `supports_alpha()`, `supports_animation()`, etc.

### `ImageFormatDefinition`

Metadata for a format: name, extensions, MIME types, capability flags, detection function.

### `ImageFormatRegistry`

Thread-safe registry for custom formats. `common()` returns built-in formats.

---

## Capabilities

### `EncodeCapabilities` / `DecodeCapabilities`

Const-constructible structs with builder pattern. Returned by config `capabilities()`.

**`EncodeCapabilities` flags:** `icc`, `exif`, `xmp`, `cicp`, `cancel`, `animation`,
`row_level`, `pull`, `lossy`, `lossless`, `hdr`, `native_gray`, `native_16bit`,
`native_f32`, `native_alpha`, `enforces_max_pixels`, `enforces_max_memory`,
`effort_range`, `quality_range`, `threads_supported_range`.

**`DecodeCapabilities` flags:** `icc`, `exif`, `xmp`, `cicp`, `cancel`, `animation`,
`cheap_probe`, `decode_into`, `row_level`, `hdr`, `native_gray`, `native_16bit`,
`native_f32`, `native_alpha`, `enforces_max_pixels`, `enforces_max_memory`,
`enforces_max_input_bytes`, `threads_supported_range`.

Method: `supports(UnsupportedOperation) -> bool`.

### `UnsupportedOperation`

```rust
enum UnsupportedOperation {
    RowLevelEncode, PullEncode, AnimationEncode,
    DecodeInto, RowLevelDecode, AnimationDecode,
    PixelFormat,
}
```

---

## Resource limits

### `ResourceLimits` (`Copy + Clone + Debug + PartialEq + Eq`)

Fields: `max_pixels`, `max_memory_bytes`, `max_output_bytes`, `max_width`,
`max_height`, `max_input_bytes`, `max_frames`, `max_animation_ms`,
`threading: ThreadingPolicy`.

Validation methods: `check_dimensions()`, `check_memory()`, `check_image_info()`,
`check_output_info()`, `check_decode_cost()`, `check_encode_cost()`.

### `LimitExceeded`

Error enum: `Width`, `Height`, `Pixels`, `Memory`, `InputSize`, `OutputSize`,
`Frames`, `Duration` — each carries `actual` and `max`.

### `ThreadingPolicy`

```rust
enum ThreadingPolicy {
    SingleThread,
    LimitOrSingle { max_threads: u16 },
    LimitOrAny { preferred_max_threads: u16 },
    Balanced,
    Unlimited,  // #[default]
}
```

---

## Security policies

### `DecodePolicy` / `EncodePolicy`

Const-constructible structs controlling what metadata to extract/embed,
what features to allow.

**`DecodePolicy` flags:** `allow_icc`, `allow_exif`, `allow_xmp`, `allow_progressive`,
`allow_animation`, `allow_truncated`, `strict`.

**`EncodePolicy` flags:** `embed_icc`, `embed_exif`, `embed_xmp`.

`DecodePolicy` constructors: `none()`, `strict()`, `permissive()`.
`EncodePolicy` constructors: `none()`, `strip_all()`, `preserve_all()`.

---

## Cost estimation

### `DecodeCost` / `EncodeCost`

Resource cost estimates for pre-decode/pre-encode budget checks.

`DecodeCost`: `output_bytes`, `pixel_count`, `peak_memory: Option<u64>`.
Constructor: `from_output_info(&OutputInfo)`.

`EncodeCost`: `input_bytes`, `pixel_count`, `peak_memory: Option<u64>`.
Constructor: `for_input(width, height, descriptor)`.

---

## Color types

### `ColorContext` / `ColorProfileSource` / `NamedProfile`

Color context for pipeline tracking (ICC profile bytes + CICP parameters).
Re-exported from `zenpixels`.

### `GainMapMetadata`

HDR gain map metadata (ISO 21496-1). Fields: per-channel gain map parameters,
HDR capacity range, base/alternate rendition flags.

---

## Error utilities

### `CodecErrorExt` (trait)

Extension trait for inspecting error chains without downcasting:

```rust
trait CodecErrorExt {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation>;
    fn limit_exceeded(&self) -> Option<&LimitExceeded>;
    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T>;
}
```

### `find_cause<T>(err) -> Option<&T>`

Walk an error chain looking for a specific cause type.

### `Unsupported<E>`

Generic stub type for unsupported decode modes. Implements `StreamingDecode`
and `FullFrameDecoder` with unreachable bodies. Use as `type StreamDec = Unsupported<E>`.

---

## Source encoding detection

### `SourceEncodingDetails` (trait)

Codec-agnostic interface for querying how an image was encoded. Each codec's
probe type (e.g. `JpegProbe`, `WebPProbe`) implements this trait.

```rust
trait SourceEncodingDetails: Any + Send + Sync {
    fn source_generic_quality(&self) -> Option<f32>;
    fn is_lossless(&self) -> bool;  // default: false
}

impl dyn SourceEncodingDetails {
    fn codec_details<T: SourceEncodingDetails + 'static>(&self) -> Option<&T>;
}
```

`source_generic_quality()` returns a 0.0–100.0 estimate on the same scale as
`EncoderConfig::with_generic_quality()`. Returns `None` for lossless encodings
or when quality can't be determined from headers. Approximate (±5).

The trait intentionally has very few methods — only properties meaningful across
all image formats. Codec-specific details (color type, bit depth, palette size,
chroma subsampling, encoder family, quantizer tables) belong on the concrete
probe struct and are accessed via `codec_details::<T>()` downcast.

Available on both `ImageInfo` (from probe or decode) and `DecodeOutput`. Codec
implementors populate it when the codec can detect source encoding properties
from headers.

---

## Re-exports

```rust
pub use enough;             // cooperative cancellation (Stop trait)
pub use enough::Unstoppable;
```

---

## Helpers

### `push_decoder_via_full_decode()`

Fallback `push_decoder` implementation via one-shot decode + copy to sink.
For codecs that can't stream natively.

### `render_frame_to_sink_via_copy()`

Fallback `render_next_frame_to_sink` implementation via `render_next_frame` + copy.

### `negotiate_pixel_format()` / `best_encode_format()` / `is_format_available()`

Format negotiation helpers for matching preferred descriptors to codec capabilities.
