# Codec error inventory → `ErrorCategory` mapping

> **Superseded shape (2026-07-13).** `ErrorCategory` was reshaped from the flat
> 17-variant set below into an **origin-first two-level** enum —
> `Image(ImageError) / Request(RequestError) / Resource(ResourceError) / Policy /
> Stopped(StopReason) / Io(CodecIoKind) / Internal`, with sub-enums
> (`ImageError`, `RequestError`, `InvalidKind`, `UnsupportedImageKind`,
> `ResourceError`) — before the 0.1.26 freeze. See [`spec.md`](spec.md#errorcategory-enum--categorizederror-trait)
> for the current canonical shape. The old→new key: `MalformedImage`→`Image(Malformed)`,
> `UnexpectedEof`→`Image(UnexpectedEof)`, `UnsupportedImageType/Feature`→
> `Image(Unsupported(Type/Feature))`, `UnsupportedPixelFormat/Operation`→
> `Request(Unsupported(op))`, `CmsRequired`→`Request(CmsRequired)`,
> `InvalidParameters/Buffer/State`→`Request(Invalid(Parameters/Buffer/State))`,
> `PolicyRejected`→`Policy`, `Cancelled/TimedOut`→`Stopped(StopReason)`,
> `LimitsExceeded(k)`→`Resource(Limits(k))`, `OutOfMemory`→`Resource(OutOfMemory)`,
> `Io`/`Internal` unchanged. The per-codec mapping table below is retained as the
> **historical basis** for the category set (the audit that justified each
> category); the per-codec `category()` maps themselves are being rewritten to
> the two-level form in the codec crates (Phases 2–3 of the reshape).

Point-in-time reference (snapshot **2026-06-24**) backing the
[`ErrorCategory`](../src/error.rs) / `CategorizedError` design (issue #99). It
records the **actual error enum of every zen image codec**, maps every variant
to a category, and evaluates whether the 17-variant set is right-sized.

Other repos' error enums drift — re-run the inventory (grep `pub enum .*Error`
+ the `#[error(...)]` lines under each codec's `src/`) if validating afresh.

## The categories (17)

`MalformedImage`, `UnexpectedEof`, `UnsupportedImageType`,
`UnsupportedImageFeature`, `UnsupportedPixelFormat`, `UnsupportedOperation`,
`CmsRequired`, `PolicyRejected`, `Cancelled`, `TimedOut`,
`LimitsExceeded(LimitKind)`, `OutOfMemory`, `Io`, `InvalidParameters`,
`InvalidBuffer`, `InvalidState`, `Internal`.

## Carrying the category

The mapping below says *which* category each variant is. *How* a codec carries
it to a consumer is a separate choice:

- **Native enum + `CategorizedError`** — `type Error = MyError`. The category is
  reachable on the typed path (`e.category()` / forwarded through `At<MyError>`),
  but **not** after the error is erased to `Box<dyn Error>` (the erased value is a
  `dyn Error`, not a `dyn CategorizedError`).
- **The `CodecError` envelope** — `type Error = whereat::At<CodecError>`. Since
  that is one concrete type, the category survives erasure: a consumer recovers
  it from any `Box<dyn Error>` / `anyhow` via `CodecErrorExt::error_category()`.
  Recommended for codecs driven through dyn dispatch. The per-codec mapping in
  this doc is unchanged either way — it just feeds `CodecError::from_native`/`of`
  instead of being read off the typed enum. See [`spec.md`](spec.md) (`CodecError`) for the
  one-impl adoption recipe.

---

## Per-codec error enums (verbatim variant inventory)

### zenjpeg — `Error(pub whereat::At<ErrorKind>)`
`zenjpeg/zenjpeg/src/` (nested workspace). Public `Error` newtypes `At<ErrorKind>`;
there are layered kinds (top-level `error.rs`, plus `encoder/error.rs`,
`decoder/error.rs`, `recompress/error.rs`). Distinct variants across them:
- `InvalidDimensions{w,h,reason}`, `InvalidColorFormat`,
  `InvalidJpegData`, `TruncatedData`, `InvalidMarker`, `InvalidHuffmanTable`,
  `InvalidQuantTable`, `DecodeError`, `InvalidScanScript` → **MalformedImage** /
  (truncation → **UnexpectedEof**)
- `UnsupportedFeature{feature}` → **UnsupportedImageFeature**
- `UnsupportedPixelFormat{format}` → **UnsupportedPixelFormat**
- `ImageTooLarge{pixels,limit}`, `TooManyScans` → **LimitsExceeded**
- `AllocationFailed{bytes,context}` → **OutOfMemory**
- `SizeOverflow{context}`, `InternalError{reason}`, `Internal{reason}` → **Internal**
- `IoError{reason}` → **Io**
- `IccError(String)` / `Icc(String)` → **CmsRequired**
- `InvalidBufferSize{expected,actual}`, `StrideTooSmall{width,stride}` → **InvalidBuffer**
- `TooManyRows`, `IncompleteImage` → **InvalidState**
- `InvalidQuality`, `InvalidConfig`, `TargetOutOfRange` → **InvalidParameters**
- `Cancelled(enough::StopReason)` → **Cancelled** / **TimedOut**

### zenpng — `PngError` (`At<PngError>` at the boundary)
- `Decode(String)` → **MalformedImage**
- `InvalidInput(String)` → **MalformedImage** / **InvalidParameters** (context-dependent; also carries `UnsupportedOperation::PixelFormat` → **UnsupportedPixelFormat**)
- `LimitExceeded(String)` → **LimitsExceeded**
- `Stopped(enough::StopReason)` → **Cancelled**/**TimedOut**
- `Quantize(#[from] zenquant::QuantizeError)` → **Internal**
- (`ProbeError`: `TooShort`/`Truncated` → **UnexpectedEof**, `NotPng` → **UnsupportedImageType**)

### zenwebp — `DecodeError` / `EncodeError` / `MuxError` / `ValidationError`
- decode malformed set (`RiffSignatureInvalid`, `WebpSignatureInvalid`, `ChunkMissing`, `ChunkHeaderInvalid`, `InvalidAlphaPreprocessing`, `InvalidCompressionMethod`, `AlphaChunkSizeMismatch`, `FrameOutsideImage`, `LosslessSignatureInvalid`, `VersionNumberInvalid`, `InvalidColorCacheBits`, `HuffmanError`, `BitStreamError`, `TransformError`, `Vp8MagicInvalid`, `ColorSpaceInvalid`, `*PredictionModeInvalid`, `InconsistentImageSizes`, `InvalidChunkSize`) → **MalformedImage**
- `NotEnoughInitData` → **UnexpectedEof**
- `UnsupportedFeature(String)` → **UnsupportedImageFeature**
- `UnsupportedOperation(#[from] zencodec::UnsupportedOperation)` → **UnsupportedOperation** (+`PixelFormat` arm → **UnsupportedPixelFormat**)
- `ImageTooLarge`, `MemoryLimitExceeded`, `LimitExceeded`, `Partition0Overflow` → **LimitsExceeded**
- `Cancelled(enough::StopReason)` → **Cancelled**/**TimedOut**
- `IoError(#[from] std::io::Error)` → **Io**
- `IccSynthesisUnavailable(String)` → **CmsRequired**
- `InvalidBufferSize` → **InvalidBuffer**
- `InvalidParameter`, `InvalidDimensions`, `TargetZensimUnsupportedLayout`, all of `ValidationError::*OutOfRange` → **InvalidParameters**
- `NoMoreFrames` → animation-iteration end (**UnexpectedEof**, control-flow)

### zengif — `GifError`
- `InvalidHeader`, `InvalidScreenDescriptor`, `InvalidFrameBounds`, `ZeroDimensionFrame`, `MissingPalette`, `InvalidDisposalMethod`, `MalformedLzw`, `InvalidMinCodeSize`, `FrameDimensionMismatch`, `GifCrate` → **MalformedImage**
- `UnsupportedVersion` → **UnsupportedImageType**
- `UnexpectedEof` → **UnexpectedEof**
- `DimensionsTooLarge`, `TotalPixelsTooLarge`, `TooManyFrames`, `FileTooLarge`, `MemoryLimitExceeded`, `DecompressionRatioExceeded`, `AnimationTooLong`, `OutputTooLarge` → **LimitsExceeded**
- `AllocationFailed` → **OutOfMemory**
- `Io` → **Io**
- `Cancelled` → **Cancelled**
- `UnsupportedOperation(zencodec::UnsupportedOperation)` → **UnsupportedOperation**
- `InvalidEncoderState` → **InvalidState**
- `QuantizationFailed` → **Internal**

### zenavif — `Error`
- `Parse(#[from] zenavif_parse::Error)` → **MalformedImage**
- `Unsupported(&str)` → **UnsupportedImageFeature**
- `ImageTooLarge`, `ResourceLimit(String)` → **LimitsExceeded**
- `OutOfMemory` → **OutOfMemory**
- `Cancelled(StopReason)` → **Cancelled**/**TimedOut**
- `UnsupportedOperation(#[from] zencodec::UnsupportedOperation)` → **UnsupportedOperation**
- `Decode{code,msg}`, `Encode(String)`, `ColorConversion(#[from] yuv::YuvError)` → **Internal**

### zenjxl — `JxlError`
- `Decode(#[from] jxl::api::Error)` → **MalformedImage**
- `InvalidInput(String)` → **MalformedImage**/**InvalidParameters**
- `ProgressiveRejected` → **PolicyRejected** (rejected by decode policy)
- `LimitExceeded(String)` → **LimitsExceeded**
- `Cancelled(enough::StopReason)` → **Cancelled**/**TimedOut**
- `UnsupportedOperation(#[from] zencodec::UnsupportedOperation)` → **UnsupportedOperation**
- `Sink(zencodec::decode::SinkError)` → **Io**
- `Encode(#[from] jxl_encoder::EncodeError)` → **Internal**

### zenbitmaps — `BitmapError` (`At<BitmapError>` at the boundary)
- `InvalidHeader`, `InvalidData` → **MalformedImage**
- `UnrecognizedFormat` → **UnsupportedImageType**
- `UnsupportedVariant(String)` → **UnsupportedImageType** (or **UnsupportedImageFeature**)
- `UnexpectedEof` → **UnexpectedEof**
- `DimensionsTooLarge`, `LimitExceeded` → **LimitsExceeded**
- `BufferTooSmall{needed,actual}`, `LayoutMismatch{expected,actual}` → **InvalidBuffer**
- `Cancelled(StopReason)` → **Cancelled**/**TimedOut**
- `UnsupportedOperation(#[from] zencodec::UnsupportedOperation)` → **UnsupportedOperation**

### heic — `HeicError` (+ `HevcError`, `ProbeError`); `type Error = At<HeicError>`
- `InvalidContainer`, `InvalidData`, `InvalidBitstream`, `InvalidNalUnit`, `MissingParameterSet`, `InvalidParameterSet`, `CabacError`, `ProbeError::{InvalidFormat,Corrupt}` → **MalformedImage**
- `ProbeError::NeedMoreData` → **UnexpectedEof**
- `UnsupportedCodec`, `UnsupportedProfile` → **UnsupportedImageType**
- `Unsupported(&str)` → **UnsupportedImageFeature**
- `BufferTooSmall` → **InvalidBuffer**
- `LimitExceeded`, `DimensionOverflow` → **LimitsExceeded**
- `OutOfMemory`, `AllocationFailed` → **OutOfMemory**
- `Cancelled(StopReason)` → **Cancelled**/**TimedOut**
- `Sink(Box<dyn Error>)` → **Io**
- `HevcDecode`, `DecodingError`, `NoPrimaryImage`, `NoBackendSelected`, `AllBackendsFailed` → **Internal**

### zentiff — not checked out locally
Per the crate index it wraps `image-tiff`; its error type would forward
`image_tiff`'s decode errors (→ **MalformedImage** / **UnsupportedImageFeature**)
plus the standard `UnsupportedOperation` / `LimitsExceeded` / `Cancelled` arms.
Confirm when adopting.

---

## Design evaluation — is the 17-set right-sized?

**Coverage.** Every variant across all eight inventoried codecs maps to exactly
one category. No variant is left unclassifiable.

**No dead categories.** Each category is backed by multiple real variants across
multiple codecs:

| category | representative backers |
|---|---|
| MalformedImage | every decoder (dozens of variants) |
| UnexpectedEof | zengif, zenbitmaps, zenjpeg, zenwebp, heic |
| UnsupportedImageType | zenpng, zenwebp, zengif, zenbitmaps, heic |
| UnsupportedImageFeature | **zenjpeg (`UnsupportedFeature`)**, zenwebp, zenavif, heic |
| UnsupportedPixelFormat | **zenjpeg (`UnsupportedPixelFormat`)**, `UnsupportedOperation::PixelFormat` (zenpng/zenjxl/zenavif) |
| UnsupportedOperation | `zencodec::UnsupportedOperation` (every codec) |
| CmsRequired | zenwebp `IccSynthesisUnavailable`, zenjpeg `Icc`, + ecosystem (`zenpixels_convert::{NeedsCms,CmsError}`, `zencodecs::ColorManagement`) |
| Cancelled / TimedOut | every codec (`StopReason`) |
| LimitsExceeded | every codec (many variants) |
| OutOfMemory | zenjpeg, zengif, zenavif, heic (`AllocationFailed`/`OutOfMemory`) |
| Io | zenwebp, zenjpeg, sinks |
| InvalidParameters | zenwebp `ValidationError` (15+), zenjpeg `InvalidQuality`/`InvalidConfig`, zenavif |
| InvalidBuffer | zenjpeg `InvalidBufferSize`/`StrideTooSmall`, zenwebp `InvalidBufferSize`, zenbitmaps `BufferTooSmall`/`LayoutMismatch`, heic `BufferTooSmall`, + ecosystem `zenpixels::BufferError`, `garb::SizeError` |
| InvalidState | zengif `InvalidEncoderState`, zenjpeg `TooManyRows`/`IncompleteImage`, + ecosystem `zenresize::{AlreadyFinished,RingBufferOverflow}`, metric `NoCachedReference`/`NoWarmReference` |
| Internal | zenavif `ColorConversion`, zengif `QuantizationFailed`, heic backends, zenjxl encode, `zenquant::QuantizeError` |
| PolicyRejected | `zenjxl::ProgressiveRejected`; pipeline-side `zenpixels_convert::{DepthReductionForbidden, AlphaRemovalForbidden, RgbToGray}` |

The split the review asked for is **vindicated by zenjpeg specifically**, which
independently distinguishes `UnsupportedFeature` *and* `UnsupportedPixelFormat`
as separate variants — exactly the `UnsupportedImageFeature` /
`UnsupportedPixelFormat` distinction in the taxonomy.

**Policy rejection (`PolicyRejected`).** A distinct cluster means "the input is
valid and the codec *can* handle it, but a configured policy refused":
`zenjxl::ProgressiveRejected` ("rejected by decode policy"), and on the pipeline
side `zenpixels_convert::{DepthReductionForbidden, AlphaRemovalForbidden,
RgbToGray}`. This is neither malformed input nor a codec limitation — the request
was understood and *declined* — so it gets its own category (HTTP 422-ish),
keeping it out of the `MalformedImage`/`UnsupportedImageFeature` buckets where it
would mislead a router.

**Prior art in the workspace** (different domains, no conflict):
`zenfleet-core::ErrorClass` (`Timeout/Oom/DecodeError/EncoderPanic/MetricNan/…`)
classifies *fleet-job* failures; `zensim`'s `ErrorCategory`
(`Identical/RoundingError/ChannelSwap/…`) classifies *pixel diffs*. Neither is a
codec-error taxonomy; the name reuse is incidental.

**Buffer & state (`InvalidBuffer`, `InvalidState`).** Added from the cross-codec
+ ecosystem inventory. `InvalidBuffer` is a wrong-geometry pixel buffer (size /
stride / alignment / descriptor) — recurring as `BufferTooSmall` /
`InvalidBufferSize` / `StrideTooSmall` / `LayoutMismatch` and `zenpixels::
BufferError`; previously these scattered across `InvalidParameters` /
`LimitsExceeded` / `UnsupportedPixelFormat`. `InvalidState` is API misuse /
wrong-sequence (`InvalidEncoderState`, `TooManyRows`, `IncompleteImage`,
`zenresize::{AlreadyFinished,RingBufferOverflow}`, metric `No*Reference`) —
previously folded into `Internal`. Both pull a real, recurring cluster out of the
overloaded `InvalidParameters` / `Internal` buckets.

**Conclusion: right-sized at 17.** Complete coverage, every category
load-bearing, no over-granular dead variants. `PolicyRejected` / `InvalidBuffer`
/ `InvalidState` each pull a distinct, recurring cluster out of the
malformed/unsupported/params/internal catch-alls.

---

## Discrimination gaps — codec variants that can't map *precisely*

The 17 categories are complete, but a category map is only as good as the
**source** error's granularity. Several codec variants are *insufficiently
discriminatory*: a single variant bundles cases that belong in different
categories, so a faithful `category()` impl has to guess. These are the variants
worth **expanding in the codec crates** so the mapping is exact (the work lands in
each codec repo, not here). Three failure modes:

### (a) Stringly-typed catch-alls (split into discrete variants)
A `Variant(String)` / `Variant(&str)` that spans several categories:

| codec | variant | spans | should split into |
|---|---|---|---|
| zenpng | `Decode(String)` | malformed vs truncated-EOF vs unsupported-feature | `MalformedImage` / `UnexpectedEof` / `UnsupportedImageFeature` |
| zenpng | `InvalidInput(String)` | malformed vs bad-params vs pixel-format | `MalformedImage` / `InvalidParameters` / `UnsupportedPixelFormat` |
| zenavif | `Encode(String)` | bad-params vs internal vs limits | `InvalidParameters` / `Internal` / `LimitsExceeded` |
| zenavif / heic | `Unsupported(&str)` | image-type vs feature vs pixel-format vs operation | the four `Unsupported*` categories |
| zenjxl | `InvalidInput(String)` | malformed vs bad-params | `MalformedImage` / `InvalidParameters` |
| zenbitmaps | `UnsupportedVariant(String)` | whole-format-type vs in-format-feature | `UnsupportedImageType` / `UnsupportedImageFeature` |
| zengif | `GifCrate{message}` | malformed vs io vs internal (opaque wrap) | discrete variants per cause |

### (b) Stringly-typed limits (lose `LimitKind`)
`LimitExceeded(String)` maps to `LimitsExceeded(..)` but **cannot fill
`LimitKind`** — the *which-cap* discrimination is gone. Affects **zenpng**,
**zenwebp** (encode), **zenavif** (`ResourceLimit`), **zenjxl**, **zenbitmaps**,
**heic**. Fix: carry the cap kind (a field/enum) or embed `zencodec::LimitExceeded`
(which already has the kind). Contrast the codecs that *do* it well — zengif and
zenjpeg use discrete `*TooLarge` / `ImageTooLarge` / `TooManyScans` variants that
map straight onto `LimitKind`.

### (c) Opaque wrapped sub-errors (delegate, don't collapse)
A `#[from] <dep>::Error` mapped to one category throws away the sub-error's own
discrimination: `zenjxl::Decode(jxl::api::Error)`,
`zenavif::Parse(zenavif_parse::Error)`, `zenavif::ColorConversion(yuv::YuvError)`,
`heic::HevcDecode(HevcError)`, `*::Quantize(zenquant::QuantizeError)`. Fix: have
the inner error implement `CategorizedError` and **delegate** (`e.0.category()`),
rather than the wrapper hard-coding `Internal`/`MalformedImage`. `HevcError` and
`QuantizeError` are already structured enough to classify per-variant.

### Scorecard
- **Best discriminated (adopt as-is):** **zenjpeg** (distinct variants for nearly
  every category), **zengif** (discrete limits, now `InvalidState`), **zenwebp
  decode** (one variant per malformed case).
- **Needs expansion before a precise map:** **zenpng** (`Decode`/`InvalidInput`/
  `LimitExceeded` all stringly-typed), **zenjxl** (opaque `Decode`/`Encode` wraps +
  string limit), **zenavif** (`Unsupported`/`Encode`/`ResourceLimit` strings +
  opaque `Decode`/`Parse`), **heic** (`Unsupported(&str)` + `LimitExceeded(&str)`).

None of this blocks the taxonomy — coarse codecs still map to *a* category; they
just can't reach the *finest* one until the codec splits the variant. The 17
categories are the target the codec error enums should be refactored toward.
