# Design Rationale

Why the API looks the way it does, and what was tried and rejected along the way.

## Three-layer pattern: Config / Job / Executor

Every codec follows Config → Job → Executor:

```text
ENCODE:  EncoderConfig → EncodeJob<'a> → Encoder / FullFrameEncoder
DECODE:  DecoderConfig → DecodeJob<'a> → Decode / StreamingDecode / FullFrameDecoder
```

**Config** is reusable settings (`Clone + Send + Sync + 'static`). A web server
keeps one `JpegEncoderConfig` at quality 85 and shares it across request threads.

**Job** borrows per-operation temporaries: cancellation token (`&dyn Stop`),
`ResourceLimits`, `MetadataView`. This is where limits are checked and headers
parsed. The job is where you bind everything that's stack-local.

**Executor** borrows pixels or file bytes. It consumes itself to produce output —
single-shot by design. This prevents use-after-encode/decode bugs at the type level.

Why three layers instead of two? Config-level methods (quality, effort, lossless)
are shared across operations. Job-level methods (limits, stop, metadata) are
per-operation. Collapsing them would force callers to re-set quality every time
they encode, or would require the config to borrow per-operation data it doesn't
need to store.

## Typed encode was tried and rejected

The original TRAIT-DESIGN.md (Feb 2025) proposed per-format encode traits:
`EncodeRgb8`, `EncodeRgba8`, `EncodeGray8`, etc. Each codec implements only the
traits for formats it accepts — JPEG implements `EncodeRgb8` and `EncodeGray8`,
not `EncodeRgba8`. Compile-time enforcement: if you pass RGBA to a JPEG encoder
that doesn't implement `EncodeRgba8`, you get a type error.

This was implemented (see commit `2c624de refactor: delete per-format encode traits`)
and then removed. The problems:

1. **Trait explosion.** 11 pixel formats × 2 (single + animation) = 22 traits
   minimum. Adding a new pixel format requires a new trait, new blanket impls for
   dyn dispatch, and updates to every codec.

2. **Dyn dispatch breaks.** The whole point of `DynEncoderConfig` is codec-agnostic
   code. Per-format traits can't be put behind a single vtable — you'd need a macro
   to try each trait and dispatch at runtime, which is exactly what the type-erased
   `Encoder::encode(PixelSlice)` already does, just less elegantly.

3. **Format negotiation already exists.** `EncoderConfig::supported_descriptors()`
   lists accepted formats. `best_encode_format()` checks compatibility at the call
   site. Runtime checking with good error messages beats compile errors that say
   "trait `EncodeRgba8` is not implemented for `JpegEncoder`" without suggesting
   what to do instead.

**Current design:** Single `Encoder::encode(PixelSlice<'_>)` method. The encoder
checks the descriptor at runtime. `supported_descriptors()` and
`best_encode_format()` let callers check before calling. Capabilities flags
provide metadata.

## Decode stays type-erased — format comes from the file

Decoders always return type-erased pixels because the output format is discovered
at runtime from the input file. A PNG might be RGB8, RGBA8, Gray8, RGB16, etc.
The caller can't know at compile time.

**Format negotiation:** The caller provides a ranked `&[PixelDescriptor]`
preference list to `decoder()`. The decoder picks the first format it can produce
without lossy conversion. Pass `&[]` for the decoder's native format. The decoder
must never do lossy conversion to satisfy a preference.

This replaces the earlier `decode_into_rgb8()` / `decode_into_rgba8()` pattern,
which required codecs to implement a method per format and forced callers to know
the output format in advance.

## Cow<[u8]> input instead of &[u8]

`DecodeJob::decoder()` takes `Cow<'a, [u8]>` instead of `&'a [u8]`. This matters
for animation decoders (`FullFrameDecoder: 'static`) that need to outlive the
job's borrow scope. With `Cow::Owned`, the caller can donate the input buffer to
the decoder. With `Cow::Borrowed`, it's zero-cost for the common case.

The earlier design used `&'a [u8]` everywhere, which made animation decoders
impossible as `'static` types — they couldn't hold a reference to the input data
past the job's lifetime. This was fixed in commit `527abaa`.

## FullFrameDecoder composites internally

The original `FrameDecode` trait yielded raw frames with compositing metadata
(`blend`, `disposal`, `frame_rect`, `required_frame`). Callers had to composite
frames themselves — handling disposal modes, blending, sub-canvas positioning,
and reference frame tracking.

This was replaced with `FullFrameDecoder`, which composites internally and yields
full-canvas frames ready for display. The decoder handles all the format-specific
compositing rules (GIF disposal modes, APNG blending, JXL reference frames) and
the caller just gets finished pixels.

Why:
- Compositing rules vary wildly between formats. Getting them wrong produces
  visual glitches that are hard to debug.
- The codec already knows the format-specific rules. Having the caller reimplement
  them defeats the purpose of an abstraction layer.
- `render_next_frame()` returns a `FullFrame` that borrows the decoder's internal
  canvas (zero-copy). `render_next_frame_owned()` copies for independent ownership.
  `render_next_frame_to_sink()` writes directly to a caller-owned buffer.

Types removed: `FrameBlend`, `FrameDisposal`, `DecodeFrame`, `EncodeFrame`.

## FullFrameEncoder is minimal

`FullFrameEncoder` has just two methods: `push_frame(pixels, duration_ms, stop)`
and `finish(stop)`. Full-canvas frames only.

The earlier design had `EncodeFrame` structs with sub-canvas positioning, blend
modes, and disposal hints. This was removed because:
- Sub-canvas optimization (only encoding the changed region) is a codec-internal
  concern. The encoder should figure out dirty regions itself.
- Blend modes and disposal hints are format-specific. Exposing them in the
  generic trait forces callers to understand format internals.
- Every real use case is "here are my full-canvas frames, encode them."

## Animation executors are 'static

`FullFrameEnc: 'static` and `FullFrameDec: 'static`. Animation encoders and
decoders own their data — they don't borrow from the job.

This is necessary because animation processing often outlives the job scope.
A frame decoder might be passed to another thread, stored in a struct, or iterated
lazily. The `'static` bound makes this work without lifetime gymnastics.

The cost: animation executors can't borrow the job's `&dyn Stop` token. Instead,
each method takes `stop: Option<&dyn Stop>` as a parameter. This is slightly more
verbose but avoids the alternative of requiring callers to `Arc` their stop tokens.

See `docs/lifetime.md` for the `where Self: 'a` bound explanation.

## DecodeRowSink: begin/provide_next_buffer/finish

`DecodeRowSink` uses a three-phase lifecycle instead of the earlier single
`demand()` method:

1. `begin(width, height, descriptor)` — sink pre-allocates and validates
2. `provide_next_buffer(y, height, width, descriptor)` — sink provides a buffer
   per strip, codec writes into it
3. `finish()` — sink flushes or finalizes

The earlier `demand(y, height, width, descriptor) -> PixelSliceMut` had no
lifecycle hooks — the sink couldn't reject incompatible formats early, couldn't
pre-allocate with known total dimensions, and couldn't flush after the last strip.

`begin()` and `finish()` have default no-op implementations, so minimal sinks
only need `provide_next_buffer()`.

The sink controls stride — it can return SIMD-aligned buffers (stride padded to
64 bytes) and the codec respects whatever stride the `PixelSliceMut` carries.
The trait is object-safe for use as `&mut dyn DecodeRowSink`.

## Both pull and push streaming

Two complementary streaming decode models:

**Pull (`StreamingDecode`):** Caller drives the loop, pulling batches via
`next_batch()`. Codec owns the buffer. Good for simple consumers that process
rows sequentially.

**Push (`DecodeRowSink`):** Codec drives, writing into caller-provided buffers.
Good for zero-copy pipelines where the caller controls buffer layout and lifetime.
`push_decoder()` on `DecodeJob` and `render_next_frame_to_sink()` on
`FullFrameDecoder` use this model.

Every codec gets both models via defaults:
- `push_decoder_via_full_decode()` implements push via one-shot decode + copy
- `render_frame_to_sink_via_copy()` implements sink rendering via canvas copy

Codecs with native row-level streaming override for true incremental decode.
The `row_level` capability flag distinguishes native streaming from decode-then-copy.

Strip height is codec-determined: 1 row for PNG, 8-16 rows (MCU height) for JPEG,
256 rows (group) for JXL, 512 rows (tile) for AVIF grid. Callers must not assume
a specific strip size.

## CodecErrorExt replaces HasUnsupportedOperation

The original `HasUnsupportedOperation` trait had a single method:
`unsupported_operation() -> Option<&UnsupportedOperation>`. It was implemented
manually by each codec's error type.

This was replaced with `CodecErrorExt`, a blanket-implemented trait that walks the
error's source chain automatically:

```rust
trait CodecErrorExt {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation>;
    fn limit_exceeded(&self) -> Option<&LimitExceeded>;
    fn find_cause<T: Error + 'static>(&self) -> Option<&T>;
}

impl<E: Error + 'static> CodecErrorExt for E { ... }
```

Codecs just need `impl From<UnsupportedOperation> for MyError` and chain their
error sources properly. No manual trait implementation needed.

## Capabilities are const structs, not runtime queries

`EncodeCapabilities` and `DecodeCapabilities` are const-constructible structs
returned by `capabilities()` on the config. They declare what a codec supports
before any operation happens.

```rust
static MY_CAPS: EncodeCapabilities = EncodeCapabilities::new()
    .with_lossy(true)
    .with_icc(true)
    .with_quality_range(0.0, 100.0);
```

The earlier `CodecCapabilities` was a single struct for both encode and decode.
Splitting it allows encode-specific flags (quality range, effort range, pull
encode) and decode-specific flags (cheap probe, streaming) without confusion.

`capabilities().supports(UnsupportedOperation::AnimationEncode)` checks a
specific operation. Check before calling instead of catching errors.

## No codec-side color management

Decoders return native pixels with ICC/CICP metadata. Encoders accept pixels
as-is and embed the provided metadata. The caller handles CMS transforms.

Why:
- Color management is complex and application-specific. Some apps want perceptual
  intent, others colorimetric. Some want to preserve out-of-gamut colors.
- ICC profiles can be megabytes. Parsing and applying them is a significant
  dependency (lcms2, littlecms, etc.) that image codecs shouldn't force.
- Most codec libraries (libjpeg, libpng, libwebp) don't do CMS either.
  The ones that do (ImageMagick) have a full CMS engine, not a half-baked one.

CICP (`Cicp` struct) uses raw u8 values matching ITU-T H.273. This is
forward-compatible — new CICP values work without updating the enum. Three
named constants cover common cases: `Cicp::SRGB`, `Cicp::BT2100_PQ`,
`Cicp::BT2100_HLG`. CICP and ICC coexist: CICP for machine-readable color
space identification, ICC for full profile precision.

## Metadata flows through MetadataView

`ImageInfo::metadata()` borrows a `MetadataView<'a>`. Pass to
`EncodeJob::with_metadata()` for lossless metadata roundtrip.

`MetadataView` carries ICC, EXIF, XMP, CICP, HDR metadata (content light level,
mastering display), orientation, and resolution. The encoder embeds what it
supports and what the policy allows, silently skipping the rest.

`Metadata` (owned) exists for cross-boundary transfer when the borrow can't be
maintained. `From<MetadataView<'_>>` and `From<&ImageInfo>` conversions.

## Policies control security-relevant behavior

`EncodePolicy` and `DecodePolicy` use three-valued logic (`None` / `Some(true)` /
`Some(false)`). `None` means "use codec default."

Why three-valued instead of booleans: codec defaults vary. JPEG's default for
progressive might be different from PNG's. `None` defers to the codec rather than
imposing a blanket policy.

`strict()` constructor restricts everything. `none()` defers everything.
`permissive()` allows everything. Individual flags override.

## Extensions for codec-specific dyn access

`extensions()` / `extensions_mut()` on `EncodeJob` and `DecodeJob` return
`Option<&dyn Any>` for codec-specific configuration through the dyn pipeline.

Without this, callers using `&dyn DynEncodeJob` can set generic options (quality,
limits, stop) but can't access codec-specific settings. Extensions let them
downcast to the concrete job type's extension struct.

This is less clean than typed access but preserves the object-safety contract.
The alternative — adding every possible codec-specific option to the generic
trait — would bloat the trait and couple it to specific codecs.

## OrientationHint for coalesced transforms

Instead of a boolean "apply orientation," `OrientationHint` is an enum:

- `Preserve` — don't touch, report intrinsic orientation
- `Correct` — resolve EXIF/container orientation to Normal
- `CorrectAndTransform(Orientation)` — coalesce EXIF correction with an
  additional transform
- `ExactTransform(Orientation)` — ignore EXIF, apply exactly this transform

JPEG decoders can coalesce orientation correction with DCT-domain lossless
rotation, avoiding a pixel-domain transform. The `CorrectAndTransform` variant
lets callers request "correct orientation AND rotate 90 degrees" as a single
coalesced operation.

## Unsupported<E> rejection stub

Codecs that don't support streaming or animation use `Unsupported<E>` as the
associated type: `type StreamDec = Unsupported<At<MyError>>`. This implements
the trait with unreachable bodies — the job's constructor returns an error before
the stub is ever created.

The unit type `()` also implements `FullFrameEncoder` and `StreamingDecode` as
rejection stubs. `Unsupported<E>` exists for cases where the error type matters
(dyn dispatch blanket impls need matching error types).

## Dyn dispatch is blanket-implemented

Codec authors implement the generic traits (`EncoderConfig`, `EncodeJob`,
`Encoder`, etc.). Blanket impls automatically generate the object-safe `Dyn*`
variants. No extra code from codec authors.

Downcasting rules:
- `DynEncoderConfig`, `DynDecoderConfig`: `as_any()` — configs are `'static`
- `DynFullFrameEncoder`, `DynFullFrameDecoder`: `as_any()`, `as_any_mut()`,
  `into_any()` — animation executors are `'static`
- `DynEncoder`, `DynDecoder`, `DynStreamingDecoder`: **no downcasting** — they
  borrow `'a` data, can't safely cast to concrete types

The dyn API uses `set_*()` methods (mutation) instead of `with_*()` (builder)
because `Box<dyn Trait>` can't return `Self`.

## SourceEncodingDetails: available from both probe and decode

`SourceEncodingDetails` lives on both `ImageInfo` (available from probe) and
`DecodeOutput` (available from decode). It describes how the image was *encoded*
(quality estimate, encoder fingerprint, chroma subsampling).

`ImageInfo` uses `Option<Arc<dyn SourceEncodingDetails>>` — the `Arc` makes it
`Clone`-compatible, and `PartialEq` skips the field (trait objects aren't
comparable). This is the same pattern used for other non-comparable metadata.

Codecs that can cheaply detect encoding properties from headers (JPEG
quantization tables, WebP VP8 header) populate it during `probe()`. Codecs
that need deeper parsing populate it only during `decode()`. Either way, the
caller gets a uniform interface via `source_encoding_details()`.

Why a trait instead of struct fields:
- Each codec has different encoding properties. JPEG has quantization tables and
  subsampling. WebP has VP8 quantizer and partition type. PNG has filter strategy
  and compression level. A struct would either be too generic (just quality) or
  too bloated (every codec's fields).
- The trait gives a generic interface (`source_generic_quality()`, `is_lossless()`)
  plus downcast access to the full codec-specific type.

The trait is intentionally minimal — only properties meaningful across **all**
image formats belong on it. Codec-specific data (color type, bit depth, palette
size, chroma subsampling, encoder family, etc.) belongs as fields or methods on
the concrete probe struct, accessed via `codec_details::<T>()`. Adding methods to
a shared trait because two or three codecs share a concept creates a combinatorial
explosion and couples the trait to specific codecs.

This is a general zen* design principle that applies wherever traits meet concrete
types: `EncoderConfig` traits define cross-codec settings (quality, effort);
concrete configs expose codec-specific knobs. `EncodeOutput` has `extras::<T>()`
for codec-specific output data. The trait is the stable contract; the concrete
type carries the richness.

## I/O abstraction: deferred

The current API requires `&[u8]` (or `Cow<[u8]>`) — the full file must be in
memory. A positioned-read trait (`ByteSource`) was explored:

```rust
trait ByteSource {
    fn len(&self) -> Option<u64>;
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, Error>;
    fn as_contiguous(&self) -> Option<&[u8]> { None }
}
```

Stateless (`&self`, no cursor), matches image codec random-access patterns.
Zero-cost for `&[u8]` via `as_contiguous()`.

This was deferred because:
- It needs validation on 2-3 codecs before becoming a trait-level API
- Object-safety constraints make generic trait methods tricky
- The `_from()` method pattern on concrete codec types works as an interim solution

## 16-bit convention

u16 = gamma-encoded (native transfer function), full 0-65535 range.
f32 = linear light.

This matches libavif, PNG spec, and image-rs. u16 sRGB is not linear — it's
the same gamma curve as u8 but with more precision.

## HDR metadata

`ContentLightLevel` (CEA-861.3) and `MasteringDisplay` (SMPTE ST 2086) are
separate types from ICC and CICP. They describe display characteristics, not
color space.

Three HDR workflows exist:
1. Gain map preservation (carry the data through encode/decode)
2. HDR reconstruction from gain map (needs PQ EOTF)
3. HDR→SDR+GainMap creation (subjective, deferred)

`GainMapMetadata` (ISO 21496-1) is included for workflow 1.

## What got renamed and why

| Old name | Current name | Reason |
|----------|-------------|--------|
| `FrameEncoder` | `FullFrameEncoder` | Clarifies it handles full-canvas composited frames |
| `FrameDecode` | `FullFrameDecoder` | Same — composites internally, yields full canvas |
| `frame_encoder()` | `full_frame_encoder()` | Consistency with type name |
| `frame_decoder()` | `full_frame_decoder()` | Consistency with type name |
| `HasUnsupportedOperation` | `CodecErrorExt` | Broader scope (also finds `LimitExceeded`) |
| `CodecCapabilities` | `EncodeCapabilities` / `DecodeCapabilities` | Split for encode-specific vs decode-specific flags |
| `demand()` | `provide_next_buffer()` | Better describes the call direction (sink provides, codec fills) |
| `DecoderConfig::format()` | `DecoderConfig::formats()` | Returns `&[ImageFormat]` — some decoders handle multiple formats |
