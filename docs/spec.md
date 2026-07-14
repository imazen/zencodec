# zencodec API Specification

Shared traits and types for zen* image codecs. This is the canonical reference
for the public API surface.

`#![no_std]` + `alloc`. `#![forbid(unsafe_code)]`.

### zenpixels: use but never re-export

`zenpixels` defines the cross-crate pixel interchange types: `PixelDescriptor`,
`PixelFormat`, `PixelSlice`, `PixelSliceMut`, `PixelBuffer`, `ChannelLayout`,
`ChannelType`, `TransferFunction`, `ColorPrimaries`, `AlphaMode`, `SignalRange`,
`InterleaveFormat`.

**All crates in the zen ecosystem MUST use `zenpixels` types directly.**
zencodec uses them in trait signatures but callers and codec
implementors should depend on `zenpixels` directly and use `zenpixels::` paths
in their public APIs.

---

## Trait hierarchy

```text
ENCODE:
                                 ┌→ Enc (Encoder)
EncoderConfig → EncodeJob ──────┤
                                 └→ AnimationFrameEnc (AnimationFrameEncoder, 'static)

DECODE:
                                 ┌→ Dec (Decode)
DecoderConfig → DecodeJob<'a> ──┤→ StreamDec (StreamingDecode)
                                 └→ AnimationFrameDec (AnimationFrameDecoder, 'static)
```

Each layer has object-safe `Dyn*` variants for codec-agnostic dispatch:

```text
DynEncoderConfig → DynEncodeJob → DynEncoder / DynAnimationFrameEncoder
DynDecoderConfig → DynDecodeJob → DynDecoder / DynStreamingDecoder / DynAnimationFrameDecoder
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
    type Job: EncodeJob<Error = Self::Error>;

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

    fn job(self) -> Self::Job;

    // Provided one-shot (default body; requires Job::Enc: Encoder<Error = Self::Error>):
    // job() → encoder() → encode(pixels) with default job settings.
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}
```

### `EncodeJob` (per-operation, owns stop token and metadata)

```rust
trait EncodeJob: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Enc: Sized + 'static;                    // single-image encoder
    type AnimationFrameEnc: Sized + Send + 'static; // animation encoder

    fn with_stop(self, stop: StopToken) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_policy(self, policy: EncodePolicy) -> Self;         // default: self
    // Blessed metadata path: filters via Metadata::filtered(&policy) before the
    // codec sees the record, then routes through with_metadata. (default provided)
    fn with_metadata_policy(self, meta: Metadata, policy: MetadataPolicy) -> Self;
    #[deprecated] // embeds without a retention policy; use with_metadata_policy.
    fn with_metadata(self, meta: Metadata) -> Self; // primitive — codecs impl this
    fn with_canvas_size(self, width: u32, height: u32) -> Self; // default: self
    fn with_loop_count(self, count: Option<u32>) -> Self;       // default: self

    // Codec-specific extensions (downcasted by callers who know the codec)
    fn extensions(&self) -> Option<&dyn Any>;          // default: None
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>; // default: None

    fn encoder(self) -> Result<Self::Enc, Self::Error>;
    fn animation_frame_encoder(self) -> Result<Self::AnimationFrameEnc, Self::Error>;

    // Type-erased convenience (default impls via shims)
    fn dyn_encoder(self) -> Result<Box<dyn DynEncoder>, BoxedError>
        where Self::Enc: Encoder;
    fn dyn_animation_frame_encoder(self) -> Result<Box<dyn DynAnimationFrameEncoder>, BoxedError>
        where Self::AnimationFrameEnc: AnimationFrameEncoder;
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

### `AnimationFrameEncoder` (animation encode)

```rust
trait AnimationFrameEncoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn reject(op: UnsupportedOperation) -> Self::Error;
    fn push_frame(
        &mut self, pixels: PixelSlice<'_>, duration_ms: u32, stop: Option<&dyn Stop>,
    ) -> Result<(), Self::Error>;
    fn finish(self, stop: Option<&dyn Stop>) -> Result<EncodeOutput, Self::Error>;
}
```

Full-canvas frames only. Animation encoder is `'static` — it owns its data.
Codecs without animation set `type AnimationFrameEnc = ()` (unit implements
`AnimationFrameEncoder` with all methods returning `Err`).

---

## Decode traits

### `DecoderConfig` (codec config, `Clone + Send + Sync`)

```rust
trait DecoderConfig: Clone + Send + Sync {
    type Error: core::error::Error + Send + Sync + 'static;
    type Job<'a>: DecodeJob<'a, Error = Self::Error> + 'static;

    fn formats() -> &'static [ImageFormat]; // may return multiple
    fn supported_descriptors() -> &'static [PixelDescriptor];
    fn capabilities() -> &'static DecodeCapabilities;  // default: EMPTY

    fn job<'a>(self) -> Self::Job<'a>;

    // Provided one-shots (default bodies) with default job settings:
    fn decode(self, data: &[u8]) -> Result<DecodeOutput, Self::Error>;  // native pixel format
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;     // header parse only
}
```

### `DecodeJob<'a>` (per-operation, holds limits/stop/hints)

```rust
trait DecodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Dec: Decode<Error = Self::Error>;
    type StreamDec: StreamingDecode<Error = Self::Error>;
    type AnimationFrameDec: AnimationFrameDecoder<Error = Self::Error> + 'static;

    fn with_stop(self, stop: StopToken) -> Self;
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
    fn animation_frame_decoder(self, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
        -> Result<Self::AnimationFrameDec, Self::Error>;

    // Type-erased convenience (default impls via shims)
    fn dyn_decoder(...) -> Result<Box<dyn DynDecoder + 'a>, BoxedError>;
    fn dyn_animation_frame_decoder(...) -> Result<Box<dyn DynAnimationFrameDecoder>, BoxedError>;
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

### `AnimationFrameDecoder` (animation decode, composited full-canvas frames)

```rust
trait AnimationFrameDecoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn wrap_sink_error(err: SinkError) -> Self::Error;
    fn info(&self) -> &ImageInfo;
    fn frame_count(&self) -> Option<u32>;       // default: None
    fn loop_count(&self) -> Option<u32>;        // default: None

    fn render_next_frame(&mut self, stop: Option<&dyn Stop>)
        -> Result<Option<AnimationFrame<'_>>, Self::Error>;
    fn render_next_frame_owned(&mut self, stop: Option<&dyn Stop>)
        -> Result<Option<OwnedAnimationFrame>, Self::Error>;   // default: copies from render_next_frame
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
    fn dyn_job(&self) -> Box<dyn DynEncodeJob + 'static>;
}

trait DynEncodeJob {
    fn set_stop(&mut self, stop: StopToken);
    fn set_limits(&mut self, limits: ResourceLimits);
    fn set_policy(&mut self, policy: EncodePolicy);
    fn set_metadata_policy(&mut self, meta: Metadata, policy: MetadataPolicy); // blessed
    #[deprecated] // use set_metadata_policy
    fn set_metadata(&mut self, meta: Metadata);
    fn set_canvas_size(&mut self, width: u32, height: u32);
    fn set_loop_count(&mut self, count: Option<u32>);
    fn extensions(&self) -> Option<&dyn Any>;
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>;
    fn into_encoder(self: Box<Self>) -> Result<Box<dyn DynEncoder>, BoxedError>;
    fn into_animation_frame_encoder(self: Box<Self>) -> Result<Box<dyn DynAnimationFrameEncoder>, BoxedError>;
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

trait DynAnimationFrameEncoder {
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
    fn set_stop(&mut self, stop: StopToken);
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
    fn into_animation_frame_decoder(self: Box<Self>, data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynAnimationFrameDecoder>, BoxedError>;
}

trait DynDecoder {
    fn decode(self: Box<Self>) -> Result<DecodeOutput, BoxedError>;
}

trait DynAnimationFrameDecoder {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
    fn info(&self) -> &ImageInfo;
    fn frame_count(&self) -> Option<u32>;
    fn loop_count(&self) -> Option<u32>;
    fn render_next_frame_owned(&mut self, stop: Option<&dyn Stop>)
        -> Result<Option<OwnedAnimationFrame>, BoxedError>;
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
- `DynAnimationFrameEncoder`, `DynAnimationFrameDecoder`: `as_any()`, `as_any_mut()`, `into_any()` — frame decoders/encoders are `'static`
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
`has_gain_map`,
`source_encoding: Option<Arc<dyn SourceEncodingDetails>>`,
`warnings: Vec<String>`.

Builder pattern: `ImageInfo::new(w, h, format).with_alpha(true).with_cicp(...)`.

Key methods: `display_width()`, `display_height()` (orientation-corrected),
`transfer_function()`, `color_primaries()`,
`metadata() -> Metadata`,
`source_encoding_details() -> Option<&dyn SourceEncodingDetails>`.

`PartialEq` skips `source_encoding` (trait objects aren't comparable).

### `SourceColor`

Source color description. Fields: `cicp: Option<Cicp>`,
`icc_profile: Option<Arc<[u8]>>`, `bit_depth: Option<u8>`,
`channel_count: Option<u8>`, `content_light_level: Option<ContentLightLevel>`,
`mastering_display: Option<MasteringDisplay>`.

Builder pattern: `SourceColor::default().with_cicp(...).with_icc_profile(...)`.

Methods: `transfer_function()`, `color_primaries()`.

Note: sRGB detection (`is_srgb()`) lives in zencodecs as `SourceColorExt`,
not here — zencodec stores color metadata but doesn't classify it.

### `EmbeddedMetadata`

Non-color metadata blobs. Fields: `exif: Option<Vec<u8>>`, `xmp: Option<Vec<u8>>`.

### `Metadata`

Owned metadata for encode/decode roundtrip. Fields: `icc_profile`, `exif`, `xmp`
(`Option<Arc<[u8]>>`), `cicp`, `content_light_level`, `mastering_display` (Copy),
and `orientation`. `#[non_exhaustive]`. Carries no retention state — a policy is
chosen *transiently* at embed time (see below), not stored here.

Methods: builder pattern (`with_icc()`, `with_exif()`, `with_xmp()`, etc.),
`with_copyright(&str)` / `with_artist(&str)` (build-or-merge the rights tag into
the EXIF blob, ASCII), `transfer_function()`, `color_primaries()`, `is_empty()`,
`filtered(&MetadataPolicy) -> Metadata`. `From<&ImageInfo>` conversion.

**Embed-time policy (explicit privacy decision, compile-time enforced).**
Retention is decided when metadata is handed to the encoder, via
`EncodeJob::with_metadata_policy(meta, policy)` (or
`DynEncodeJob::set_metadata_policy`), which applies `meta.filtered(&policy)`
*before* the record reaches the codec — so a codec only ever embeds what the
policy kept. The plain `with_metadata` / `set_metadata` are `#[deprecated]`:
they embed without a retention choice, so the compiler **warns at the call site**
(a nudge, not a semver break — they still work, and codecs still *implement*
`with_metadata` as the primitive `with_metadata_policy` routes through;
deprecation warns callers, not implementors). The raw `exif`/`xmp`/`icc_profile`
bytes stay untouched until that filter runs, so an inspect / bring-your-own
EXIF-library round-trip still sees the originals. `MetadataPolicy` has **no
`Default`** — name one explicitly; `Web` is the recommended privacy-safe choice,
`PreserveExact` embeds verbatim.

### `MetadataPolicy` / `MetadataFields` / `IccRetention`

Field-level retention policy for `Metadata::filtered()` — the shared metadata
filter for re-encode / recompress pipelines.

`MetadataPolicy` (`#[non_exhaustive]`, **no `Default`** — retention is a privacy
decision the caller must make explicitly; `Web` is the recommended choice):
- `PreserveExact` — keep everything, byte-faithfully (incl. a redundant sRGB ICC).
- `Preserve` — keep everything, but drop a redundant sRGB ICC.
- `Web` (recommended) — ICC (unless redundant sRGB) + EXIF orientation/rights +
  CICP/HDR; drop the rest of EXIF (GPS, timestamps, camera, thumbnail) and XMP.
- `ColorAndRotation` — only what places pixels: ICC (non-sRGB) + CICP/HDR +
  EXIF orientation. Drops attribution, XMP, other EXIF.
- `Custom(MetadataFields)` — explicit per-field control.

`MetadataFields` (`Copy`, `#[non_exhaustive]`, `with_*` builders + `KEEP_ALL` /
`DISCARD_ALL` consts): `icc: IccRetention`, `exif: ExifPolicy`, and `xmp` /
`cicp` / `hdr: Retention`. `MetadataPolicy::fields()` resolves a policy.

`IccRetention`: `Drop` / `KeepNonSrgb` (drop only a redundant sRGB,
`zenpixels::icc::is_common_srgb`) / `Keep` (byte-faithful).

CICP / HDR are color *signaling* (dropping them changes displayed pixels), so
the presets keep them; only a `Custom` policy can drop them. Gain maps are not
part of `Metadata` (they live at the encode-request layer) and are unaffected.

### `exif::Exif` / `ExifPolicy` / `Retention`

Structured EXIF model (`zencodec::exif`). `Exif<'a>` (`parse` **or** `new` →
`filtered` / edit → `to_bytes`) borrows the source — entry values and the
thumbnail are never copied (entry values are `Cow`, borrowed on parse, owned when
injected by an edit). `Exif::new(TextEncoding)` (and `Default`, which uses `Ascii`)
starts an empty little-endian tree for building from scratch — e.g. stamp a
Copyright on an image that had no EXIF: `Exif::new(TextEncoding::Ascii)` →
`set_copyright(…)` → `to_bytes()` (raw TIFF; the codec adds the APP1 `Exif\0\0`
framing). The `TextEncoding` is **required** at `new` — it's the Exif 2.x ASCII
(type 2) vs Exif 3.0 UTF-8 (type 129) compat choice, a blob property used by all
string writes (type 129 is read by almost nothing today, so it can't be a silent
default). Read accessors: `orientation()`, `copyright()` / `artist()` (lossy-UTF-8
text *view*, borrowing `&self`), `copyright_bytes()` / `artist_bytes()` (raw
field bytes), `has_thumbnail()`, `has_gps()`. Edit accessors: `set_copyright(&str)`
/ `set_artist(&str)` insert-or-replace the IFD0 tag using the blob's `TextEncoding`
(materialized on the next `to_bytes`); `set_orientation(Orientation)`
insert-or-replaces the Orientation tag (an existing SHORT/LONG entry keeps its
TIFF type; a malformed non-integer carrier is replaced by the canonical 1-count
SHORT; the serializer writes IFDs tag-sorted, so insertion order is
immaterial). `to_bytes()` re-serializes a valid TIFF
with recomputed offsets, preserving byte order and `Exif\0\0` framing; it is a
byte-exact fixpoint, so filtering and editing stay idempotent.

`TextEncoding` (`#[non_exhaustive]`) — the EXIF text convention a write uses:
`Ascii` (Exif 2.x, TIFF type 2; carries UTF-8 bytes de-facto — most compatible,
the recommended default) or `Utf8` (Exif 3.0 / CIPA DC-008-2023, TIFF type 129;
spec-conformant Unicode, thin reader support). Both write the same UTF-8 bytes,
NUL-terminated; they differ only in the declared TIFF type. Re-exported at the
crate root.

Encoding (read side): Copyright/Artist may be ASCII (type 2, 7-bit) **or UTF-8
(type 129, Exif 3.0)**; non-ASCII bytes stuffed into a type-2 field are the
non-conformant-but-common case. zencodec reads both — `copyright` / `artist`
give a lossy-UTF-8 display view, `*_bytes` give the exact bytes. A pruning
rewrite **never transcodes**: it preserves the value bytes **and TIFF type**
verbatim (a field is neither corrupted nor "corrected"). Writing is the only
path that mints new bytes, and the caller picks the type via `TextEncoding`.

`ExifPolicy` (`Copy`, `#[non_exhaustive]`, `with_*` builders) — seven keep/drop
categories of `Retention`: `orientation`, `rights` (copyright + artist),
`thumbnail`, `gps`, `datetimes`, `camera`, `other`. Consts: `KEEP_ALL`,
`DISCARD_ALL`, `ATTRIBUTED_ORIENTATION`, `ORIENTATION_ONLY`.

`Retention` (`Keep` / `Discard`) — explicit per-field intent.

`exif::retain(&[u8], &ExifPolicy) -> Option<Cow<[u8]>>` — `Cow::Borrowed` when
nothing is dropped (so `Metadata::filtered` is a cheap `Arc` clone),
`Cow::Owned` on a rewrite, `None` when all EXIF is discarded.

`helpers::parse_exif_orientation` is a lightweight orientation accessor that
delegates here. Limitation: a partial rewrite that *keeps* `MakerNote` (0x927C)
relocates it without fixing its maker-specific internal offsets — keep all EXIF
(no prune) for byte-exact MakerNote.

Privacy (partial-strip policies): `MakerNote` is dropped whenever `gps` **or**
`camera` is stripped (it's opaque and can embed GPS/serials); `SubIFDs` (0x014A,
an unmodeled sub-IFD pointer) is dropped on a rewrite rather than left dangling;
IFD1 (thumbnail directory) entries are filtered by the same per-category rules as
IFD0, so a keep-thumbnail policy doesn't leak the Make/Model/DateTime it carries.
The `Web`/`ColorAndRotation` presets drop `gps`/`camera`/`thumbnail`/`other`, so
they were already safe; these close the gaps for hand-rolled `Custom` policies.
Cross-carrier caveat: XMP can duplicate GPS/identity — a policy that keeps XMP
ships it even when the EXIF copy is stripped.

Hardening: bounds-checked, no panics on untrusted input (32M+ fuzz executions);
the serializer dedups aliased out-of-line values to prevent rewrite
memory-amplification; ASCII accessors require the ASCII/UTF-8 TIFF type; thumbnail
length is read as SHORT or LONG; under a stripping policy `retain` fails **safe**
— unparseable or >4 GiB blobs are dropped, never passed through unfiltered.
Validated by differential tests vs `kamadak-exif`, libFuzzer targets, and a
1 KiB–1 MiB-thumbnail zero-copy benchmark.

#### EXIF write / edit path

A blob can be authored from scratch (`Exif::new(TextEncoding)` → setters →
`to_bytes`) or edited after a parse. Setters: `set_copyright` / `set_artist`
insert-or-replace the IFD0 string tag; `set_orientation` insert-or-replaces the
Orientation tag (an existing SHORT/LONG entry keeps its TIFF type, a malformed
non-integer carrier is replaced by the canonical 1-count SHORT, and a tag-less
blob gains one). `to_bytes` re-serializes through the canonical serializer
(offsets recomputed, fixpoint preserved). Mechanism: `Entry.value` is
`Cow<'a, [u8]>`, so parsed entries stay borrowed (zero-copy) while injected
ones are owned.

The caller picks the TIFF type explicitly via `TextEncoding` (`Ascii` = type 2,
`Utf8` = type 129) rather than the writer auto-upgrading to type 129 for
non-ASCII. This is deliberate: type 129 has thin reader support (ExifTool reads
it; kamadak / Pillow / most do not), so an auto-upgrade would silently produce
copyright strings most tools can't read. `Ascii` writes the string's UTF-8 bytes
into the type-2 field (the de-facto interchange form — maximally compatible);
`Utf8` is the spec-conformant choice when the consumer is known to handle it.
Both are NUL-terminated with the count including the NUL.

Still planned (additive, semver-minor; deferred until a concrete consumer):

- Setters for further fields (datetimes, software, …) as consumers appear.
- For broad copyright readability, also writing XMP `dc:rights` (universally
  UTF-8) is the most portable option and a likely companion feature.

Related: the byte-level, offset-preserving rewrite
(`helpers::set_exif_orientation`) remains the *reconciliation* path used by
`Metadata::filtered` to align a baked-upright buffer's embedded tag with the
authoritative `Metadata::orientation` field — it edits an existing tag only and
deliberately never adds one. `Exif::set_orientation` is the *authoring* path.

### `OutputInfo`

Predicted decoder output. Fields: `width`, `height`, `native_format: PixelDescriptor`,
`has_alpha`, `orientation_applied: Orientation`, `crop_applied: Option<[u32; 4]>`.

Methods: `full_decode()`, `buffer_size()`, `pixel_count()`.

### `Cicp`

ITU-T H.273 color description. Re-exported from `zenpixels`. Constants:
`SRGB`, `BT2100_PQ`, `BT2100_HLG`, `LINEAR_SRGB`, `DISPLAY_P3`, `DISPLAY_P3_PQ`.

### `ContentLightLevel` / `MasteringDisplay`

HDR metadata types (CEA-861.3 / SMPTE ST 2086). Re-exported from `zenpixels`.

### `Orientation` / `OrientationHint`

EXIF orientation (1-8 enum) and decode-time orientation strategy
(`Preserve`, `Correct`, `CorrectAndTransform`, `ExactTransform`).

---

## Output types

### `EncodeOutput`

Encoded image bytes. `#[non_exhaustive]`.

Fields: `data: Vec<u8>`, `format: ImageFormat`, `mime_type`, `extension`,
`extensions: Extensions` (type-map, see Extensions section below).

Methods: `new()`, `data()`, `into_vec()`, `format()`, `mime_type()`, `extension()`,
`with_extras<T>()`, `extras<T>()`, `take_extras<T>()`.

Clone drops extras. PartialEq/Eq skip extras.

### `DecodeOutput`

Decoded image with owned pixels. `#[non_exhaustive]`.

Fields: `pixels: PixelBuffer`, `info: ImageInfo`,
`source_encoding: Option<Arc<dyn SourceEncodingDetails>>`,
`extensions: Extensions` (type-map, see Extensions section below).

Methods: `pixels()`, `into_buffer()`, `info()`, `width()`, `height()`,
`has_alpha()`, `descriptor()`, `format()`, `metadata()`,
`with_source_encoding_details<T>()`, `source_encoding_details()`,
`take_source_encoding_details()`,
`with_extras<T>()`, `extras<T>()`, `take_extras<T>()`.

### `AnimationFrame<'a>`

Borrowed animation frame. Fields: `pixels: PixelSlice<'a>`, `duration_ms: u32`,
`frame_index: u32`. Method: `to_owned_frame()`.

### `OwnedAnimationFrame`

Owned animation frame. Fields: `pixels: PixelBuffer`, `duration_ms: u32`,
`frame_index: u32`, `extensions: Extensions` (type-map).

Methods: `pixels()`, `into_buffer()`, `as_animation_frame()`,
`with_extras<T>()`, `extras<T>()`, `take_extras<T>()`.

### Extensions type-map

`DecodeOutput`, `EncodeOutput`, and `OwnedAnimationFrame` use an `Extensions`
type-map (not a single `Box<dyn Any>`). Multiple independently-typed values
can be stored simultaneously, keyed by `TypeId`. At most one value per
concrete type. Values are `Arc`-wrapped for cheap cloning.

```rust
output.with_extras(gain_map).with_extras(depth_map) // both stored
output.extras::<DecodedGainMap>()  // access gain map
output.extras::<DecodedDepthMap>() // access depth map (independent)
```

### Supplementary decode data conventions

Decode outputs carry three layers of information:

1. **`ImageInfo`** — structured, cross-codec metadata available from probe and
   decode. Always populated. Includes `supplements` flags for discovery.

2. **`SourceEncodingDetails`** — cross-codec encoding analysis (quality estimate,
   lossless detection). Accessed via `source_encoding_details()` and
   `codec_details::<T>()` for the concrete probe struct.

3. **`Extensions` type-map** — supplementary decoded data. Accessed via
   `extras::<T>()`. Multiple types coexist.

#### Discovery, opt-in, then access

Supplement data flows through three stages:

1. **Detection (always, cheap):** `ImageInfo.supplements` and `GainMapPresence`
   are populated during probe/decode from container metadata. No pixel
   decoding occurs. This tells the caller what's available.

2. **Opt-in (caller decides):** Supplement pixel decode is **never automatic**.
   Gain maps and depth maps require explicit opt-in because decoding them
   is expensive (full AV1/HEVC decode at 1/4-1/8 primary resolution).
   Opt-in happens via:
   - Decode node params (e.g., `extract_gain_map: true` on `heic.decode`)
   - Job-level extensions on `DecodeJob`
   - Dedicated codec methods (e.g., `decode_gain_map()`)

3. **Access (typed):** After opt-in decode, supplement pixels appear in
   `extras()`:

```rust
// Stage 1: discover (cheap, always available)
if info.supplements.gain_map {
    // Stage 2: opt-in (caller must request decode separately)
    // Stage 3: access decoded pixels
    let gm = output.extras::<DecodedGainMap>().expect("opted in and decoded");
}
```

**Default behavior:** `Decode::decode()` decodes primary image pixels only.
Supplements are detected but not pixel-decoded unless explicitly requested.
Codecs that currently decode supplements unconditionally should be migrated
to opt-in behavior.

#### Normalized supplement types

Cross-codec supplement types (defined in zencodec) that all codecs should use:

| Type | When to use | Producers |
|------|-------------|-----------|
| `DecodedGainMap` | Decoded gain map pixels + ISO 21496-1 metadata | JPEG (UltraHDR), AVIF (tmap), JXL (jhgm), HEIC (Apple) |
| `DecodedDepthMap` | Decoded depth map pixels | HEIC (Apple), JXL (extra channel) |

When a codec produces a gain map, it MUST use `DecodedGainMap` (not a
codec-specific type) so that consumers work codec-agnostically.

#### Codec-specific extras

Genuinely per-codec data that cannot be normalized belongs in `extras()` with
the codec's own type. Examples:

- JPEG `DecodedExtras` — DCT coefficients, quantization tables, raw APP markers
- HEIC `HeicAuxiliaryInfo` — auxiliary image type list (URNs)
- TIFF `TiffPageInfo` — multi-page IFD metadata

These require `extras::<zenjpeg::DecodedExtras>()` — the caller must know the
codec type. This is intentional: the data is codec-specific by nature.

#### What goes where

| Data | Location | Access |
|------|----------|--------|
| Dimensions, alpha, format, progressive | `ImageInfo` fields | Direct field access |
| ICC, EXIF, XMP, CICP, HDR metadata | `ImageInfo.source_color` / `embedded_metadata` | Direct field access |
| Orientation, resolution, supplements flags | `ImageInfo` fields | Direct field access |
| Quality estimate, lossless detection | `SourceEncodingDetails` trait | `source_encoding_details()` |
| Codec-specific probe data (encoder family, DQT tables, chroma subsampling) | Concrete probe struct | `codec_details::<JpegProbe>()` |
| Gain map pixels + metadata | `DecodedGainMap` in extensions | `extras::<DecodedGainMap>()` |
| Depth map pixels | `DecodedDepthMap` in extensions | `extras::<DecodedDepthMap>()` |
| Codec-specific decode artifacts | Codec's own type in extensions | `extras::<CodecSpecificType>()` |

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

## CodecSet (multi-codec registry)

Runtime set of registered codec configs with one entry point per operation.
`Send + Sync + 'static`, `Clone`, `Debug`, `Default`; every operation takes
`&self`, so one instance can be shared app-wide (`LazyLock` / `OnceLock` /
`Arc`, or `Box::leak` in `no_std`).

```rust
struct CodecSet { /* private */ }

impl CodecSet {
    fn new() -> Self;

    // Registration — self-describing via DecoderConfig::formats() /
    // EncoderConfig::format(); first registered wins per format.
    fn with_decoder(self, config: impl DecoderConfig + 'static) -> Self;
    fn with_encoder<C>(self, config: C) -> Self
        where C: EncoderConfig + 'static,
              <C::Job as EncodeJob>::Enc: Encoder + Send,
              <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder;

    // Defaults stamped onto every job the set creates.
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_stop(self, stop: StopToken) -> Self;
    fn with_decode_policy(self, policy: DecodePolicy) -> Self;
    fn with_encode_policy(self, policy: EncodePolicy) -> Self;

    // Queries.
    fn detect(&self, data: &[u8]) -> Option<ImageFormat>;
    fn can_decode(&self, format: ImageFormat) -> bool;
    fn can_encode(&self, format: ImageFormat) -> bool;
    fn decoder_for(&self, format: ImageFormat) -> Option<&dyn DynDecoderConfig>;
    fn encoder_for(&self, format: ImageFormat) -> Option<&dyn DynEncoderConfig>;
    fn decodable_formats(&self) -> impl Iterator<Item = ImageFormat> + '_;
    fn encodable_formats(&self) -> impl Iterator<Item = ImageFormat> + '_;

    // Decode: detect → stamped job → run.
    fn probe<'a>(&'a self, data: &'a [u8]) -> Result<ImageInfo, CodecSetError>;
    fn decode<'a>(&'a self, data: &'a [u8]) -> Result<DecodeOutput, CodecSetError>;
    fn decode_preferring<'a>(&'a self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<DecodeOutput, CodecSetError>;
    fn decode_as<'a>(&'a self, format: ImageFormat, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<DecodeOutput, CodecSetError>;
    fn push_decode<'a>(&'a self, data: &'a [u8], sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor]) -> Result<OutputInfo, CodecSetError>;
    fn animation_decoder<'a>(&'a self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynAnimationFrameDecoder>, CodecSetError>;   // 'static result
    fn streaming_decoder<'a>(&'a self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynStreamingDecoder + 'a>, CodecSetError>;   // borrows set + data
    fn decode_job<'a>(&'a self, format: ImageFormat)
        -> Result<Box<dyn DynDecodeJob<'a> + 'a>, CodecSetError>;      // escape hatch (hints, etc.)

    // Encode: format-keyed; the registered config is a template.
    fn encode(&self, format: ImageFormat, pixels: PixelSlice<'_>)
        -> Result<EncodeOutput, CodecSetError>;
    fn encode_with(&self, format: ImageFormat, fidelity: Fidelity, pixels: PixelSlice<'_>)
        -> Result<EncodeOutput, CodecSetError>;                        // clones the template
    fn encode_job(&self, format: ImageFormat) -> Result<Box<dyn DynEncodeJob>, CodecSetError>;
    fn encode_job_with(&self, format: ImageFormat, fidelity: Fidelity)
        -> Result<Box<dyn DynEncodeJob>, CodecSetError>;

    // Resource estimation: forward to the registered codec's cost model
    // (unknown() if it has none); NoEncoder / NoDecoder if unregistered.
    fn estimate_encode(&self, format: ImageFormat, image: &ImageCharacteristics,
        compute: &ComputeEnvironment) -> Result<ResourceEstimate, CodecSetError>;
    fn estimate_decode(&self, format: ImageFormat, image: &ImageCharacteristics,
        compute: &ComputeEnvironment) -> Result<ResourceEstimate, CodecSetError>;
    // Bytes-based: probe → estimate_decode at the decoder's native output.
    fn estimate_decode_of(&self, data: &[u8], compute: &ComputeEnvironment)
        -> Result<ResourceEstimate, CodecSetError>;
}

enum CodecSetError {
    UnrecognizedFormat,        // detect() matched nothing registered
    NoDecoder(ImageFormat),    // known format, nothing registered for it
    NoEncoder(ImageFormat),
    Codec(BoxedError),         // codec failure; source() exposes the chain
}
```

Semantics:

- **Detection** consults only formats with a registered decoder: built-ins in
  `ImageFormatRegistry::common()` priority order (so AVIF-before-HEIC and
  DNG-before-TIFF disambiguation is preserved regardless of registration
  order), then `Custom` formats in registration order.
- **Encoder templates**: codec-specific options are set on the concrete config
  before registration; `encode_with` / `encode_job_with` clone the template
  and apply a per-call `Fidelity` to the clone.
- **Job escape hatches**: `decode_job` / `encode_job` return the stamped
  `Dyn*Job` for per-operation control (decode hints, metadata, canvas/loop
  settings) before running an executor.

## Prelude

`zencodec::prelude::*` imports every encode/decode trait (generic and dyn
variants) so `.job()`, `.decoder()`, `.encode()`, `.next_batch()`, … resolve
with one `use`. Types are not included.

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

**`EncodePolicy` fields:** `color: Option<ColorEmitPolicy>` (ICC-vs-CICP carrier,
read by the codec via `resolve_color`) plus the coarse, best-effort per-channel
embed gates `embed_icc`, `embed_exif`, `embed_xmp`. The embed gates are *not* the
reliable retention control — for field-level privacy use
`EncodeJob::with_metadata_policy`.

`DecodePolicy` constructors: `none()`, `strict()`, `permissive()`.
`EncodePolicy` constructors: `none()`, `strip_all()`, `preserve_all()`;
builder `with_color()`.

---

## Color types

`Cicp`, `ContentLightLevel`, and `MasteringDisplay` are re-exported from `zenpixels`.
See `zenpixels` documentation for field details.

---

## Color emission

`resolve_color_emit(&SourceColor, &EncodeCapabilities, ColorEmitPolicy) -> ColorEmitPlan`
— a pure, `no_std`, CMS-free decision of which color carriers an *encode* writes for
a target. Crate-root re-exports (`CicpEmission`, `ColorEmitFields`, `ColorEmitPlan`,
`ColorEmitPolicy`, `IccDisposition`, `resolve_color_emit`).

```rust
pub fn resolve_color_emit(
    source: &SourceColor,
    caps: &EncodeCapabilities,
    policy: ColorEmitPolicy,
) -> ColorEmitPlan;

#[non_exhaustive]
pub enum ColorEmitPolicy { Compatibility, Balanced, Compact, Verbatim, Custom(ColorEmitFields) }

pub struct ColorEmitPlan { pub cicp: Option<Cicp>, pub icc: IccDisposition }

#[non_exhaustive]
pub enum IccDisposition { KeepSource, SynthesizeFrom(Cicp), Drop }
```

- **`ColorEmitPolicy`** — `Balanced` (default) writes CICP where it's a *safe sole
  carrier* and keeps a synthesized ICC companion otherwise (e.g. PNG `cICP`);
  `Compatibility` favors the widest reader support; `Compact` prefers CICP and drops
  the ICC; `Verbatim` carries the source's signals unchanged; `Custom(ColorEmitFields)`
  is explicit (`ColorEmitFields::new`).
- **`EncodeCapabilities`** carrier methods: `cicp_is_valid_carrier` (the format has a
  standardized CICP slot — JXL/AVIF/HEIC `nclx`, PNG `cICP`) and `cicp_safe_sole_carrier`
  (CICP alone is spec-mandated and reader-authoritative — JXL, AVIF, HEIC).
- **`IccDisposition`** — `KeepSource` re-embeds the source ICC; `Drop` emits none;
  `SynthesizeFrom(cicp)` asks the caller to materialize bytes (this crate carries no
  CMS) via `zenpixels_convert`'s transfer-aware `synthesize_icc_for_cicp` — a bundled
  `const` profile or a CMS-generated one, never a mis-tagged TRC. The plan never emits
  a redundant `SynthesizeFrom(sRGB)`.

The emit-direction names can't be confused with the decode-side `SourceColor`. Design
and rejected alternatives: `docs/color-emit-model.md`; how the framework resolves
color/orientation/metadata before a codec runs: `docs/correctness-model.md`.

---

## Error utilities

### `ErrorCategory` (enum) + `CategorizedError` (trait)

Coarse, codec-agnostic error classification for routing (HTTP status, retry,
logging) without naming the concrete error enum.

```rust
#[non_exhaustive]
enum ErrorCategory {
    Image(ImageError),        // the bytes are the problem
    Request(RequestError),    // the caller's request is the problem
    Resource(ResourceError),  // a cap was hit, or allocation failed
    Policy(PolicyKind),        // valid input a configured policy declined
    Stopped(StopReason),    // stopped via the Stop token (Cancelled / TimedOut)
    Io(CodecIoKind),          // an I/O or output-sink failure
    Internal(InternalKind),   // a bug / broken invariant / unclassified dependency error
}

#[non_exhaustive] enum ImageError { Malformed, UnexpectedEof, Unsupported(UnsupportedImageKind) }
#[non_exhaustive] enum UnsupportedImageKind { Type, Feature }
#[non_exhaustive] enum RequestError { Invalid(InvalidKind), Unsupported(UnsupportedOperation), CmsRequired }
#[non_exhaustive] enum InvalidKind { Parameters, Buffer, State }
#[non_exhaustive] enum ResourceError { Limits(LimitKind), OutOfMemory }
#[non_exhaustive] enum PolicyKind { Decode, Encode }      // mirrors DecodePolicy / EncodePolicy
#[non_exhaustive] enum InternalKind { Bug, Dependency }   // Bug = our defect; Dependency = unclassified foreign error
// StopReason is enough::StopReason { Cancelled, TimedOut } — reused, not re-defined.

trait CategorizedError: core::any::Any {
    fn codec_name(&self) -> Option<&'static str>;  // required; cause types return None
    fn category(&self) -> ErrorCategory;
}
```

**Origin-first shape.** The top level splits by *who owns the fault*, so a generic
consumer can route on the outer arm and only destructure when a sub-kind changes
the answer. `Image(_)` is "the bytes are the problem" (a *different* codec might
handle them; the caller can't fix it by changing parameters) — this whole arm is
the client-supplied-data / incomplete-input set a truncation check tolerates.
`Request(_)` is "the *request* is the problem" (the caller can change config,
buffer, call sequence, or the operation/format asked for).

The "unsupported" axis is split by origin: `Image(Unsupported(Type))` (the format
isn't handled at all), `Image(Unsupported(Feature))` (a bitstream feature within a
handled format), and — on the request side — `Request(Unsupported(op))` (an API
operation this codec doesn't do, including its `PixelFormat` arm when negotiation
found no common `PixelDescriptor`). `Request(CmsRequired)` flags a
colour-management transform the codec won't perform itself.
`Policy(kind)` is valid input the codec *could* handle but a configured policy
declined — `PolicyKind` mirrors the crate's existing `DecodePolicy` / `EncodePolicy`
split (e.g. progressive content rejected by a decode policy, or alpha removal
forbidden by an encode policy), so the call site already knows which one.
`Internal(kind)` splits similarly for telemetry/triage, not routing: `Bug` is a
broken invariant in the codec's own logic (never retryable, always alert-worthy);
`Dependency` is an error surfaced from a sub-component/foreign library the codec
hasn't classified into `Image`/`Request`/`Resource` — an honest "unclassified",
not a permanent home. Both `Internal` variants still mean "500" for routing; the
split only pays off when telemetry carries the category forward without a
per-codec downcast.
`Request(Invalid(Buffer))` is a wrong-geometry pixel buffer
(size/stride/alignment/descriptor) and `Request(Invalid(State))` is API misuse
(called out of sequence) — both distinct from `Request(Invalid(Parameters))`
(config/knobs). `Resource(Limits(kind))` is a configured cap; `Resource(OutOfMemory)`
is genuine allocation exhaustion. `Io` carries a `CodecIoKind` — a
`std::io::ErrorKind` when the `std` feature is enabled, empty under `no_std` (the
variant shape is stable either way, so matching `Io(_)` is portable), anticipating
a future `core::io::ErrorKind`.
The set is distilled from a per-codec inventory; see
[`error-taxonomy-inventory.md`](error-taxonomy-inventory.md) for the
variant→category mapping and right-sizing rationale, and
[`error-types-ecosystem.md`](error-types-ecosystem.md) for the wider ecosystem.

Sub-enums carry `From` shortcuts into `ErrorCategory` (`ImageError`, `RequestError`,
`ResourceError`, `StopReason`, and the leaf kinds `UnsupportedImageKind` /
`InvalidKind` / `LimitKind`), so a codec's `category()` arm reads
`ImageError::Malformed.into()` or `InvalidKind::Buffer.into()` rather than spelling
the outer wrapper each time.

`CategorizedError` is **opt-in** (not blanket-implemented): a codec implements it
on its error type, mapping each variant to one category. It is **not** required
by the `EncoderConfig`/`DecoderConfig::Error` bound, so adopting it is additive
and back-compatible. A blanket `impl<E: CategorizedError> CategorizedError for
whereat::At<E>` forwards to the inner error, so a located error keeps its
category. zencodec's own cause types implement it (`LimitExceeded` →
`Resource(Limits(kind))`, `UnsupportedOperation` → `Request(Unsupported(self))`,
`enough::StopReason` → `Stopped(self)` — the reason IS the payload, no lossy
collapse), so a codec's mapping is usually a one-line delegation per arm.
`LimitKind` is the value-free discriminant of `LimitExceeded`
(`LimitExceeded::kind()`).

The `codec_name()` method is where a codec declares its name —
`fn codec_name(&self) -> Option<&'static str> { Some("zenjpeg") }` — so
[`CodecError::from_native`] / [`of`] tag the envelope from the value instead of
taking a codec argument. It is **required** (no default), so every implementor
answers it; the cause types return `None` (they aren't codecs); `At<E>` forwards
the inner `codec_name()`. It is a `&self` method rather than an associated const
specifically so the trait stays **dyn-compatible**: with the `Any` supertrait a
`dyn CategorizedError` can be formed *and* downcast to its concrete type — an
associated const would forbid the trait object outright.

### `CodecError` (struct)

The shared error envelope: a coarse `ErrorCategory`, the originating codec's name,
and (optionally) the codec's own detail error. A codec returns it as
`whereat::At<CodecError>` — the recommended (and only needed) form, with the
cleanest `?` / `.at()` ergonomics.

`CodecError` is a **one-word handle** — its fields live behind a `Box` — so
`At<CodecError>` is **two words** (handle + trace, 16 bytes on 64-bit) and every
`Result<_, At<CodecError>>` a codec threads through `?` is two words too (the box
pointer's niche absorbs the `Result` discriminant). That is small enough to return
in registers rather than spill to the stack — the reason for the box: the detail is
a *fat* `Box<dyn Error>` (16 bytes alone), so an unboxed envelope would exceed the
16-byte ABI threshold and spill regardless of how thin the other fields are. The
trade is one cold-path allocation per error, fine for an error type; `new` is
therefore no longer `const`. Recovery (`codec_error()` / `error_category()`) is
downcast-based and additionally tolerates a single consumer-applied `Box` layer in
either position (`Box<At<CodecError>>`, `At<Box<CodecError>>`, `Box<CodecError>`);
deeper nesting isn't covered.

The codec name is an `Option<&'static str>` (`None` when unset — honest, no
sentinel). `from_native` / `of` read it from the detail's
[`codec_name()`](#errorcategory-enum--categorizederror-trait); `new` / `from_parts`
(no typed detail) take it directly. `codec()` reads it back.

```rust
struct CodecError(Box<Repr>);  // Repr { category, codec: Option<&'static str>, detail: Option<Box<dyn Error + Send + Sync>> }

impl CodecError {
    fn new(codec: Option<&'static str>, category: ErrorCategory) -> Self;                     // no detail
    fn from_native<E: CategorizedError + Error + Send + Sync + 'static>(detail: E) -> Self;    // bare; name from detail.codec_name()
    fn of<E: CategorizedError + Error + Send + Sync + 'static>(located: At<E>) -> At<CodecError>;  // located; trace preserved
    fn from_parts(codec: Option<&'static str>, category: ErrorCategory, detail: Box<dyn Error + Send + Sync>) -> Self;
    fn with_codec(self, codec: Option<&'static str>) -> Self;  // builder: stamp/clear the codec name on an existing envelope
    fn category(&self) -> ErrorCategory;             // total, fixed at construction
    fn codec(&self) -> Option<&'static str>;         // which codec produced it (None if unset)
    fn detail(&self) -> Option<&(dyn Error + 'static)>;
}
```

Two ways to surface a codec error, both routable by category:

- **Native enum + `CategorizedError`** (`type Error = MyError`): `category()`
  classifies on the *typed* path. Once erased to `Box<dyn Error>` the category is
  unreachable — the erased value is a `dyn Error`, not a `dyn CategorizedError`.
- **The envelope** (`type Error = whereat::At<CodecError>`): because
  `At<CodecError>` is one *concrete* type, the category **and** the codec name
  survive erasure. A consumer recovers them from any `Box<dyn Error>` /
  `anyhow::Error` / mapped wrapper by downcast — see `CodecErrorExt::codec_error()`
  / `error_category()`.

The **`codec` name** (`&'static str`, e.g. `"zenjpeg"`) is how a consumer tells
codecs apart without naming any codec-specific type — useful when several codecs
feed one pipeline. The **`detail` is optional**: `CodecError::new(codec, category)`
is a complete error for a codec that has no error enum of its own.

`from_native` / `of` read the category *and* the codec name from the detail's
`CategorizedError` impl at construction, so both are total and never re-derived
from an opaque chain. `of` takes an **already-located** `At<E>` (not a bare `E`):
location is mandatory at the type level — a codec that skipped whereat can't call
it — and the trace stays on the *outside* (`At<CodecError>`, mapping the inner
error via whereat's trace-preserving `map_error`), never buried in the detail.
`Display` is `"{codec}: {detail-or-category}"`; `source()` is the detail (when
present), so the typed extractors below still reach the underlying cause.

**Adoption is one impl.** A codec keeping a native `MyError: CategorizedError`
adds `impl From<MyError> for At<CodecError>` (body: `CodecError::of(e.start_at())`,
with `fn codec_name(&self) { Some("mycodec") }` on the error's `CategorizedError`
impl), after which `?` on any `Result<_, MyError>` auto-wraps into the envelope —
existing fallible internals need no rewrite. (Or convert a bare `Result<_, MyError>`
in one step with whereat's `map_err_at(CodecError::from_native)`.) Reject stubs use `Unsupported<At<CodecError>>`. `At` and the
`start_at()` / `.at()` ext traits are **re-exported** from the crate root
(`zencodec::{At, ErrorAtExt, ResultAtExt}`), so a codec can name
`zencodec::At<zencodec::CodecError>` without depending on `whereat` directly. The
testkit's `minimal` codec is a worked example; `reference` keeps the native-enum
form for contrast.

#### Attaching structural locus (`StreamOffset`)

Per-variant detail (dimensions, expected/actual, table indices) stays in the
native error — reachable via `detail` / `find_cause`. The one piece of context
worth carrying *generically* is **where in the input** the failure was: best-in-
class decoders report a byte offset, and zen decoders already track it internally.
Rather than grow a per-locus field on `CodecError`, a codec attaches it to the
`At<CodecError>` trace as typed context, and a generic consumer recovers it by
downcast — no codec-specific type named:

```rust
// codec, at the failure site (offset = bytes consumed):
Err(at!(CodecError::new(Some("zenjpeg"), ErrorCategory::Image(ImageError::Malformed)))
    .at_data(|| StreamOffset(reader.position())))

// consumer, even after erasure to Box<dyn Error>:
let off = err.contexts().find_map(|c| c.downcast_ref::<StreamOffset>().copied());
```

`StreamOffset(pub u64)` is a shared, downcast-able newtype (bytes from the start
of the encoded input) so the "where" convention is cross-codec. Richer loci
(marker, box fourcc, NAL type, frame index) ride the same `at_data` / `at_str`
channel; only the offset is standardized as a type today.

### `CodecErrorExt` (trait)

Extension trait for inspecting error chains without downcasting — the *detail*
layer beneath `ErrorCategory` (which cap, which operation, which cause), plus
`codec_error()` / `error_category()` which recover the `CodecError` envelope (and
hence the category + codec name) from any erased error (`Box<dyn Error>`,
`anyhow`):

```rust
trait CodecErrorExt {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation>;
    fn limit_exceeded(&self) -> Option<&LimitExceeded>;
    fn codec_error(&self) -> Option<&CodecError>;       // recover the envelope post-erasure
    fn error_category(&self) -> Option<ErrorCategory>;  // = codec_error().map(|c| c.category())
    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T>;
}
```

### `find_cause<T>(err) -> Option<&T>`

Walk an error chain looking for a specific cause type.

### `Unsupported<E>`

Generic stub type for unsupported decode modes. Implements `StreamingDecode`
and `AnimationFrameDecoder` with unreachable bodies. Use as `type StreamDec = Unsupported<E>`.

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
pub use almost_enough::StopToken;  // owned, cloneable, type-erased stop token
```

---

## Helpers

### `copy_decode_to_sink()`

Fallback `push_decoder` implementation via one-shot decode + copy to sink.
For codecs that can't stream natively.

### `copy_frame_to_sink()`

Fallback `render_next_frame_to_sink` implementation via `render_next_frame` + copy.

### `negotiate_pixel_format()` / `best_encode_format()` / `is_format_available()`

Format negotiation helpers for matching preferred descriptors to codec capabilities.
