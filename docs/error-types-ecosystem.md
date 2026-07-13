# Zen ecosystem error types (non-codec) — survey

> **Note (2026-07-13).** `ErrorCategory` has since been reshaped into an
> origin-first two-level enum (`Image / Request / Resource / Policy / Stopped /
> Io / Internal` + sub-enums); flat variant names used below (`MalformedImage`,
> `PolicyRejected`, …) map per the key in
> [`error-taxonomy-inventory.md`](error-taxonomy-inventory.md). See
> [`spec.md`](spec.md#errorcategory-enum--categorizederror-trait) for the current shape.

Companion to [`error-taxonomy-inventory.md`](error-taxonomy-inventory.md)
(which covers the image **codecs**). Point-in-time snapshot **2026-06-24** of the
error types in the surrounding zen crates — pixel/colour, processing, pipeline,
compression, metrics, and infrastructure. Useful context for whether
`ErrorCategory` would ever need to classify *non-codec* failures a pipeline
surfaces, and to avoid reinventing error vocabulary.

Crate source layouts vary (nested workspaces): `zenpixels-convert` →
`zenpixels/zenpixels-convert/`, `zenfilters`/`zencodecs` → `zenpipe/`,
`zenpdf` → `zenextras/zenpdf/`, etc.

## Pixel & colour

- **zenpixels — `BufferError`** (`#[non_exhaustive]`, impls `Error`):
  `AlignmentViolation`, `InsufficientData`, `StrideTooSmall`,
  `StrideNotPixelAligned`, `InvalidDimensions`, `IncompatibleDescriptor`,
  `AllocationFailed`.
- **zenpixels-convert — `ConvertError`** (`#[non_exhaustive]`): `NoMatch`,
  `NoPath`, `BufferSize`, `InvalidWidth`, `EmptyFormatList`,
  `UnsupportedTransfer`, `AlphaNotOpaque`, `DepthReductionForbidden`,
  `AlphaRemovalForbidden`, `RgbToGray`, `AllocationFailed`,
  `Buffer(BufferError)`, `CmsError(String)`, `HdrSourceRequiresPeak`,
  **`NeedsCms`** (← the ecosystem origin of `ErrorCategory::CmsRequired`).
  Plus `CmsPluginError` (newtype over a boxed error) and
  `cms_moxcms::MoxCmsError` (Display-only).
- **linear-srgb** — *no public error type* (infallible conversions).
- **garb — `SizeError`** (`#[non_exhaustive]`): `NotPixelAligned`,
  `PixelCountMismatch`, `InvalidStride`.

## Processing

- **zenquant — `QuantizeError`** (`#[non_exhaustive]`): `ZeroDimension`,
  `DimensionMismatch`, `InvalidMaxColors`, `QualityNotMet`, `DimensionOverflow`,
  `InvalidIndex`. (zenpng wraps this → `Internal`.)
- **zenresize — `StreamingError`** (`AlreadyFinished`, `InputTooShort`,
  `RingBufferOverflow`) + **`CompositeError`** (`PremultipliedInput`).
- **zenblend** — *no public error type.*
- **zenfilters — `PipelineError`** (`UnsupportedPrimaries`, `BufferSize`,
  `Cancelled(StopReason)`) + **`ConvenienceError`** (`Pipeline`, `Convert`,
  `UnsupportedPrimaries`, `UnsupportedLayout`) + **`CubeParseError`**
  (Display-only).
- **zenlayout — `LayoutError`**: `ZeroSourceDimension`, `ZeroTargetDimension`,
  `ZeroRegionDimension`, `NonFiniteFloat`.

## Pipeline / dispatch

- **zenpipe — `PipeError`**: `FormatMismatch`, `Resize`, `DimensionMismatch`,
  `LimitExceeded`, `Cancelled`, `Op`, `Codec(Box<dyn Error>)`. Also
  `imageflow_compat::{ZenError, TranslateError}`. Result alias is
  `whereat::At<PipeError>`.
- **zencodecs — `CodecError`** (`#[non_exhaustive]`): `UnrecognizedFormat`,
  `UnsupportedFormat`, `UnsupportedOperation{format,detail}`, `DisabledFormat`,
  `InvalidInput`, `LimitExceeded`, `Cancelled`, `Oom`, `NoSuitableEncoder`,
  `ColorManagement(String)` (cfg `cms`), `Codec{format,source}`. Plus
  `exif::ExifError`. **This is the closest existing analog to `ErrorCategory`** —
  a registry-level dispatch error whose variants line up almost 1:1 with the
  taxonomy (`UnrecognizedFormat`→UnsupportedImageType, `Oom`→OutOfMemory,
  `ColorManagement`→CmsRequired, etc.). A future `impl CategorizedError for
  zencodecs::CodecError` would be a natural adopter.
- **zennode — `NodeError`**: `UnknownNode`, `UnknownParam`, `TypeMismatch`,
  `OutOfRange`, `MissingParam`, `InvalidEnumVariant`, `Other` (graph/param
  validation → maps to `InvalidParameters`).

## Compression / container

- **zenflate — `CompressionError`** (`InsufficientSpace`, `Stopped`) +
  **`DecompressionError`** (`BadData`, `InvalidHeader`, `ChecksumMismatch`,
  `InsufficientSpace`, `OutputLimitExceeded`, `StallLimitExceeded`, `Stopped`) +
  `StreamError<E>`.
- **zenzop — `Error`/`ErrorKind`** (`Interrupted`, `WriteZero`, `Cancelled`,
  `Other` — `std::io`-style shim).
- **zenraw — `RawError`** (`Decode`, `InvalidInput`, `Unsupported`,
  `LimitExceeded`, `Stopped`, `Buffer(BufferError)`).
- **zenpdf — `PdfError`**: `InvalidPdf`, `PageOutOfRange`, `DimensionOverflow`,
  `ZeroDimensions`, `TooManyPages`, `PixelLimitExceeded`, `Buffer(#[from]
  At<BufferError>)`, `Unsupported`, `Sink`, `LimitExceeded` — already wraps
  `zencodec::{UnsupportedOperation, LimitExceeded}`, so it's a near-ready
  `CategorizedError` adopter.
- **ultrahdr — `Error`** (17 variants incl. `Stopped`, `UnsupportedFormat`,
  `MissingInput`, `LimitExceeded`, `AllocationFailed`, `*Parse`, `Jpeg*`) +
  `ZenDecodeError`.
- **fax — `DecodeError<E>`** (`Reader`, `Invalid`, `Unsupported`).

## Metrics (informational — not in a codec/pipeline error path)

- **zensim — `ZensimError`** (`DimensionMismatch`, `ImageTooSmall`,
  `InvalidDataLength`, `InvalidStride`, `ImageTooLarge`, `UnsupportedPixelFormat`,
  `ModelLoadFailed`, `ModelForwardFailed`, `HdrInputRequiresPuPath`, …).
- **zenmetrics** — every GPU metric crate (`cvvdp(-gpu)`, `ssim2-gpu`,
  `dssim-gpu`, `butteraugli-gpu`, `zensim-gpu`, `iwssim(-gpu)`) defines its own
  `Error` with a recurring shape: `DimensionMismatch`, `NoCachedReference`/
  `NoWarmReference`, `InvalidImageSize`, `ModeUnsupported`, `TooBigForFull`.
  Plus fleet types (`zenfleet-*`: `CloudError`, `LedgerError`, `DashError`,
  `ContentError`, and **`ErrorClass`** = `Timeout/Oom/DecodeError/EncoderPanic/
  MetricNan/UploadFail/WorkerLost/Unknown`, a job-failure classification —
  a *parallel* idea to `ErrorCategory` but in the fleet domain).
- **fast-ssim2 — `Ssimulacra2Error`** + `LinearRgbImageError`.
- **butteraugli — `ButteraugliError`** (`#[non_exhaustive]`; incl.
  `Cancelled(StopReason)`).

## Infrastructure

- **whereat** — *no error type*; it provides `At<E>` (the location/trace wrapper)
  and the `ErrorAtExt`/`ResultAtExt` traits. The `At<E>` forwarding impl of
  `CategorizedError` is what keeps a category through these wrappers.
- **enough** — *no error type*; provides `StopReason`
  (`Cancelled`/`TimedOut`, Display-only, **not** an `Error`) — the cancellation
  signal codecs wrap. `ErrorCategory::{Cancelled,TimedOut}` mirror it.
- **archmage** — `CompileTimeGuaranteedError`, `DisableAllSimdError` (SIMD-token
  config, not an image path).

## Takeaways for the taxonomy

1. **CMS is a real, cross-ecosystem concern** — `NeedsCms`/`CmsError`
   (zenpixels-convert), `ColorManagement` (zencodecs), `IccSynthesisUnavailable`
   (zenwebp), `Icc` (zenjpeg) — which independently justifies
   `ErrorCategory::CmsRequired` beyond a single codec.
2. **`StopReason` is the one shared error-adjacent vocabulary** the whole
   ecosystem already agrees on — the taxonomy aligns with it
   (`Cancelled`/`TimedOut`).
3. **`zencodecs::CodecError` and `zenpdf::PdfError` are ready adopters** — their
   variants already line up with the categories; they'd be good first non-codec
   `CategorizedError` impls.
4. **The non-codec crates motivated three of the categories.** Conversion-policy
   refusals (`DepthReductionForbidden` / `AlphaRemovalForbidden` / `RgbToGray`) →
   `PolicyRejected`; buffer-geometry errors (`zenpixels::BufferError`,
   `garb::SizeError`, the `Buffer*`/`Stride*` variants) → `InvalidBuffer`;
   streaming/state misuse (`zenresize::{AlreadyFinished,RingBufferOverflow}`,
   metric `No*Reference`) → `InvalidState`. With those, the **17-set** covers
   everything the *codec* path surfaces.
5. **The one category a *pipeline* taxonomy would still add: `DimensionMismatch`**
   — two+ inputs that must agree in size/shape (every metric, `zenquant`,
   `ultrahdr` HDR-vs-SDR, `zenpipe`). Codecs decode one image and never hit it, so
   it's intentionally **out of** the codec-scoped `ErrorCategory`; a future
   pipeline-level taxonomy (or `zencodecs::CodecError`) is where it would live.
