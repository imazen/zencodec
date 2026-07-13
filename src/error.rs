//! Error chain helpers for codec error inspection.
//!
//! Codec errors are typically nested: `BoxedError` → `At<MyCodecError>` →
//! `LimitExceeded`. [`CodecErrorExt`] provides convenient methods to find
//! common cause types. [`find_cause`] is the generic version for arbitrary
//! error types.
//!
//! Works with `thiserror` `#[from]` variants, `whereat::At<E>` wrappers,
//! and any error type that properly implements `source()`.

use crate::{LimitExceeded, LimitKind, UnsupportedOperation};
use alloc::boxed::Box;
use enough::StopReason;
use whereat::At;

/// Coarse, codec-agnostic classification of a codec error.
///
/// A codec opts in by implementing [`CategorizedError`] on its error type,
/// mapping each variant to exactly one category. Consumers then route on the
/// category — HTTP status, retry policy, logging — without naming the concrete
/// error enum. The set is deliberately small and `#[non_exhaustive]`; reach for
/// the typed extractors ([`CodecErrorExt::limit_exceeded`], etc.) when you need
/// the underlying detail.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    /// The failure originates in the **image bytes** — corrupt, truncated, or a
    /// well-formed image using a type/feature this codec doesn't implement. A
    /// *different* codec might handle it; the caller can't fix it by changing
    /// parameters. See [`ImageError`].
    Image(ImageError),
    /// The failure originates in the **caller's request** — invalid
    /// config/buffer/call-sequence, or a well-formed request for an operation /
    /// pixel format this codec doesn't support. The caller *can* change the
    /// request. See [`RequestError`].
    Request(RequestError),
    /// A resource ceiling — a configured [`ResourceLimits`](crate::ResourceLimits)
    /// cap, or genuine allocation exhaustion. See [`ResourceError`].
    Resource(ResourceError),
    /// The input is valid and the codec *could* handle it, but a configured
    /// policy refused it. Understood and *declined*; often maps to HTTP 422.
    /// See [`PolicyKind`].
    Policy(PolicyKind),
    /// The operation was stopped via its [`Stop`](enough::Stop) token — cancelled
    /// by the caller or past its deadline. Carries the
    /// [`StopReason`](enough::StopReason) (`Cancelled` / `TimedOut`).
    Lifecycle(StopReason),
    /// An underlying I/O or output-sink operation failed. Carries a
    /// [`CodecIoKind`] — a `std::io::ErrorKind` when the `std` feature is
    /// enabled, empty under `no_std` (the variant shape is stable across builds).
    Io(CodecIoKind),
    /// An internal failure not attributable to the input or the request. See
    /// [`InternalKind`].
    Internal(InternalKind),
}

/// Which side of the encode/decode boundary a [`ErrorCategory::Policy`] rejection
/// applies to — mirrors the crate's existing
/// [`DecodePolicy`](crate::decode::DecodePolicy) /
/// [`EncodePolicy`](crate::encode::EncodePolicy) split, so a codec constructing
/// the error already knows which one at the call site.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PolicyKind {
    /// A [`DecodePolicy`](crate::decode::DecodePolicy) refused otherwise-decodable
    /// input — e.g. rejecting progressive content in strict mode.
    Decode,
    /// An [`EncodePolicy`](crate::encode::EncodePolicy) refused an
    /// otherwise-performable transform — e.g. forbidding alpha removal, or an ICC
    /// downgrade.
    Encode,
}

/// Which kind of [`ErrorCategory::Internal`] failure — a coarse split for
/// telemetry/triage, not a replacement for the downcast-recoverable detail
/// error (`CodecErrorExt::find_cause`, the `At` trace).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InternalKind {
    /// A broken invariant or assertion inside this codec's own logic. Always a
    /// code defect: never retryable, always alert-worthy, will recur on the same
    /// input.
    Bug,
    /// An error surfaced from a sub-component or foreign library that this codec
    /// hasn't (or structurally can't) classify into [`ErrorCategory::Image`] /
    /// [`ErrorCategory::Request`] / [`ErrorCategory::Resource`]. An honest
    /// "unclassified", not a permanent home — a call site that only ever produces
    /// `Dependency` is a taxonomy gap worth closing, not a fact about the world.
    Dependency,
}

/// Image-bytes-origin failure kind — the payload of [`ErrorCategory::Image`].
///
/// Everything here is "the bytes are the problem": a generic consumer treats the
/// whole [`Image`](ErrorCategory::Image) arm as a client-supplied-data fault (a
/// truncated/incomplete request is `UnexpectedEof`; the rest are 4xx-class
/// unprocessable content). This is exactly the set a truncation-conformance check
/// tolerates.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageError {
    /// The encoded bytes are invalid or corrupt — bad bitstream content.
    Malformed,
    /// Input ended before a complete image could be read (truncated / insufficient).
    UnexpectedEof,
    /// The bytes are a *well-formed* image this codec doesn't handle — an
    /// unrecognized format/profile, or an encoded feature it hasn't implemented.
    /// See [`UnsupportedImageKind`].
    Unsupported(UnsupportedImageKind),
}

/// Which image-bytes-origin "unsupported" — the payload of [`ImageError::Unsupported`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnsupportedImageKind {
    /// The format / container / codec / profile itself is not handled
    /// (e.g. "not a PNG", an unknown variant, an unsupported HEVC profile).
    Type,
    /// The format *is* handled, but the bitstream uses a feature this codec
    /// hasn't implemented (e.g. arithmetic-coded JPEG, an unsupported chunk).
    Feature,
}

/// Caller-request-origin failure kind — the payload of [`ErrorCategory::Request`].
///
/// Everything here is "the *request* is the problem, not the bytes": the caller
/// can change config, buffer, call sequence, or the operation/format it asked for.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestError {
    /// The request is malformed at the API level — bad config, pixel buffer, or
    /// call sequence. See [`InvalidKind`].
    Invalid(InvalidKind),
    /// The request is *well-formed* but asks for an API operation or pixel format
    /// this codec doesn't support — the [`UnsupportedOperation`] axis (animation,
    /// row-level, multi-image, pixel-format negotiation, …). Carries which one.
    Unsupported(UnsupportedOperation),
    /// The codec needs a colour-management transform / ICC profile it will not
    /// perform itself (CMS is the caller's job) — e.g. an encode target whose ICC
    /// profile cannot be synthesized. The caller must supply the transform.
    CmsRequired,
}

/// Which caller-request "invalid" — the payload of [`RequestError::Invalid`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidKind {
    /// Caller-supplied configuration or parameters were invalid — not the image's
    /// fault (knobs, quality, scan script, …).
    Parameters,
    /// A caller-supplied pixel buffer has an invalid layout — wrong size, stride,
    /// alignment, or [`PixelDescriptor`](zenpixels::PixelDescriptor) for the
    /// operation. Specifically the pixel-data buffer's geometry.
    Buffer,
    /// The operation was invoked in an invalid state or out of sequence — e.g.
    /// pushing rows after `finish()`, a streaming ring-buffer overflow, or using a
    /// cached reference before it was set. An API-protocol violation by the caller.
    State,
}

/// Resource-origin failure kind — the payload of [`ErrorCategory::Resource`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceError {
    /// A [`ResourceLimits`](crate::ResourceLimits) cap was (or would be) exceeded.
    /// Carries which [`LimitKind`].
    Limits(LimitKind),
    /// A memory allocation failed — distinct from a configured cap (retry with a
    /// smaller input / more RAM may help; a cap is a policy the caller set).
    OutOfMemory,
}

// Ergonomic lifts so a codec's `category()` map reads `ImageError::Malformed.into()`
// / `RequestError::Invalid(InvalidKind::Buffer).into()` instead of spelling the
// outer wrapper at every arm.
impl From<ImageError> for ErrorCategory {
    #[inline]
    fn from(e: ImageError) -> Self {
        Self::Image(e)
    }
}
impl From<RequestError> for ErrorCategory {
    #[inline]
    fn from(e: RequestError) -> Self {
        Self::Request(e)
    }
}
impl From<ResourceError> for ErrorCategory {
    #[inline]
    fn from(e: ResourceError) -> Self {
        Self::Resource(e)
    }
}
impl From<StopReason> for ErrorCategory {
    #[inline]
    fn from(r: StopReason) -> Self {
        Self::Lifecycle(r)
    }
}
impl From<UnsupportedImageKind> for ErrorCategory {
    #[inline]
    fn from(k: UnsupportedImageKind) -> Self {
        Self::Image(ImageError::Unsupported(k))
    }
}
impl From<InvalidKind> for ErrorCategory {
    #[inline]
    fn from(k: InvalidKind) -> Self {
        Self::Request(RequestError::Invalid(k))
    }
}
impl From<PolicyKind> for ErrorCategory {
    #[inline]
    fn from(k: PolicyKind) -> Self {
        Self::Policy(k)
    }
}
impl From<InternalKind> for ErrorCategory {
    #[inline]
    fn from(k: InternalKind) -> Self {
        Self::Internal(k)
    }
}
impl From<LimitKind> for ErrorCategory {
    #[inline]
    fn from(k: LimitKind) -> Self {
        Self::Resource(ResourceError::Limits(k))
    }
}

impl core::fmt::Display for ErrorCategory {
    /// A short human phrase — used by [`CodecError`]'s `Display` when there is no
    /// detail error to render.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Image(e) => write!(f, "{e}"),
            Self::Request(e) => write!(f, "{e}"),
            Self::Resource(e) => write!(f, "{e}"),
            Self::Policy(k) => write!(f, "{k}"),
            Self::Lifecycle(e) => write!(f, "{e}"),
            Self::Io(_) => f.write_str("I/O error"),
            Self::Internal(k) => write!(f, "{k}"),
        }
    }
}

impl core::fmt::Display for PolicyKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Decode => f.write_str("rejected by decode policy"),
            Self::Encode => f.write_str("rejected by encode policy"),
        }
    }
}

impl core::fmt::Display for InternalKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bug => f.write_str("internal error (bug)"),
            Self::Dependency => f.write_str("internal error (unclassified dependency failure)"),
        }
    }
}

impl core::fmt::Display for ImageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Malformed => f.write_str("malformed image"),
            Self::UnexpectedEof => f.write_str("unexpected end of input"),
            Self::Unsupported(UnsupportedImageKind::Type) => f.write_str("unsupported image type"),
            Self::Unsupported(UnsupportedImageKind::Feature) => {
                f.write_str("unsupported image feature")
            }
        }
    }
}

impl core::fmt::Display for RequestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Invalid(InvalidKind::Parameters) => f.write_str("invalid parameters"),
            Self::Invalid(InvalidKind::Buffer) => f.write_str("invalid pixel buffer"),
            Self::Invalid(InvalidKind::State) => f.write_str("invalid state"),
            // `UnsupportedOperation`'s own Display already reads "unsupported
            // operation: <op>" (and "no acceptable pixel format" for PixelFormat).
            Self::Unsupported(op) => write!(f, "{op}"),
            Self::CmsRequired => f.write_str("colour-management transform required"),
        }
    }
}

impl core::fmt::Display for ResourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Limits(kind) => write!(f, "resource limit exceeded ({kind:?})"),
            Self::OutOfMemory => f.write_str("out of memory"),
        }
    }
}

/// The kind of an I/O failure carried by [`ErrorCategory::Io`].
///
/// `core::io::ErrorKind` does not exist yet, and `std::io::ErrorKind` is
/// unavailable under `no_std`. So this is a thin newtype that carries a
/// `std::io::ErrorKind` **only when the `std` feature is enabled**, and is empty
/// otherwise. The [`ErrorCategory::Io`] variant shape stays stable across builds
/// — only this payload's internals are feature-gated — so matching `Io(_)` is
/// portable. When `core::io::ErrorKind` stabilizes the `cfg` drops and this works
/// under `no_std` too, with no API change.
///
// The `kind` accessor only exists under `std`, so gate the doc line that links
// to it — otherwise a `no_std` doc build (which is what docs.rs runs here, since
// `std` is not a default feature) hits an unresolved intra-doc link and fails.
#[cfg_attr(
    feature = "std",
    doc = "`std` consumers read the [`kind`](Self::kind) accessor when present."
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CodecIoKind {
    #[cfg(feature = "std")]
    kind: Option<std::io::ErrorKind>,
}

impl CodecIoKind {
    /// An I/O failure with no further classification — the only form available
    /// under `no_std`, and the `None`-kind form under `std`.
    #[inline]
    pub const fn opaque() -> Self {
        Self {
            #[cfg(feature = "std")]
            kind: None,
        }
    }

    /// The underlying `std::io::ErrorKind`, when the `std` feature is enabled and
    /// one was recorded. Always `None`'s analogue (absent) under `no_std`.
    #[cfg(feature = "std")]
    #[inline]
    pub const fn kind(&self) -> Option<std::io::ErrorKind> {
        self.kind
    }
}

#[cfg(feature = "std")]
impl From<std::io::ErrorKind> for CodecIoKind {
    #[inline]
    fn from(kind: std::io::ErrorKind) -> Self {
        Self { kind: Some(kind) }
    }
}

#[cfg(feature = "std")]
impl From<&std::io::Error> for CodecIoKind {
    #[inline]
    fn from(e: &std::io::Error) -> Self {
        Self {
            kind: Some(e.kind()),
        }
    }
}

/// Declares a codec error's coarse [`ErrorCategory`].
///
/// **Opt-in and additive.** Implement it on your error type — mapping each
/// variant to a category — so a generic consumer can route on the category
/// without naming the type. Unlike [`CodecErrorExt`] it is *not* blanket-
/// implemented, so each codec declares (and can refine) its own mapping.
///
/// The trait requires [`Any`](core::any::Any) (so every implementor is
/// `'static`, which an error type always is), letting a `dyn CategorizedError`
/// be downcast to its concrete type via trait upcasting. Recovery after type
/// erasure still goes through the concrete [`CodecError`] envelope, not a
/// `dyn CategorizedError`.
///
/// A blanket impl forwards through [`whereat::At<E>`], so wrapping an error with
/// a location trace keeps its category. zencodec's own cause types implement it
/// too ([`LimitExceeded`], [`UnsupportedOperation`], [`enough::StopReason`]), so
/// a codec can delegate those arms — e.g. `MyError::Cancelled(r) => r.category()`.
///
/// # Example
///
/// ```rust
/// use zencodec::{CategorizedError, ErrorCategory, ImageError, UnsupportedOperation};
///
/// #[derive(Debug)]
/// enum MyError {
///     Corrupt,
///     Truncated,
///     Unsupported(UnsupportedOperation),
/// }
///
/// impl CategorizedError for MyError {
///     fn codec_name(&self) -> Option<&'static str> { Some("mycodec") } // declared once
///     fn category(&self) -> ErrorCategory {
///         match self {
///             MyError::Corrupt => ErrorCategory::Image(ImageError::Malformed),
///             MyError::Truncated => ErrorCategory::Image(ImageError::UnexpectedEof),
///             MyError::Unsupported(op) => op.category(), // delegate to the zencodec arm
///         }
///     }
/// }
/// assert_eq!(MyError::Corrupt.category(), ErrorCategory::Image(ImageError::Malformed));
/// assert_eq!(MyError::Corrupt.codec_name(), Some("mycodec"));
/// ```
pub trait CategorizedError: core::any::Any {
    /// The originating codec's name (e.g. `Some("zenjpeg")`). A codec returns a
    /// constant here on its error type so [`CodecError::from_native`] /
    /// [`of`](CodecError::of) can tag the envelope from the value — no separate
    /// argument. **Required**: there is deliberately no default, so every
    /// implementor must answer it. The zencodec cause types ([`LimitExceeded`],
    /// [`UnsupportedOperation`], [`enough::StopReason`]) implement it as `None`
    /// since they aren't codecs.
    ///
    /// It is a `&self` method (not an associated `const`) so the trait stays
    /// [dyn-compatible](https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility)
    /// — an associated const would forbid `dyn CategorizedError` entirely. With
    /// the [`Any`](core::any::Any) supertrait, a `dyn CategorizedError` can be
    /// formed *and* downcast to its concrete type.
    fn codec_name(&self) -> Option<&'static str>;

    /// This error's coarse [`ErrorCategory`].
    fn category(&self) -> ErrorCategory;
}

/// A located error keeps both the category and the codec name of the error it
/// wraps — [`At`] is transparent.
impl<E: CategorizedError> CategorizedError for At<E> {
    #[inline]
    fn codec_name(&self) -> Option<&'static str> {
        self.error().codec_name()
    }
    #[inline]
    fn category(&self) -> ErrorCategory {
        self.error().category()
    }
}

impl CategorizedError for StopReason {
    #[inline]
    fn codec_name(&self) -> Option<&'static str> {
        None
    }
    #[inline]
    fn category(&self) -> ErrorCategory {
        // The stop reason IS the payload — no lossy collapse; a future
        // `#[non_exhaustive]` reason flows through unchanged.
        ErrorCategory::Lifecycle(*self)
    }
}

impl CategorizedError for UnsupportedOperation {
    #[inline]
    fn codec_name(&self) -> Option<&'static str> {
        None
    }
    #[inline]
    fn category(&self) -> ErrorCategory {
        // The whole operation axis (including `PixelFormat`) is a caller-request
        // fault. Carry *which* op through as the payload instead of flattening it
        // to a single coarse category — the audit found all delegators lost it.
        ErrorCategory::Request(RequestError::Unsupported(*self))
    }
}

impl CategorizedError for LimitExceeded {
    #[inline]
    fn codec_name(&self) -> Option<&'static str> {
        None
    }
    #[inline]
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Resource(ResourceError::Limits(self.kind()))
    }
}

/// The error a zen codec returns: a coarse [`ErrorCategory`] for routing, the
/// name of the originating codec, and (optionally) the codec's own detailed error.
///
/// Codecs return it as **`whereat::At<CodecError>`** — the `At` carries the
/// location trace (extended with `.at()` as the error propagates up the codec's
/// call tree and the caller's), while `CodecError` carries:
/// - [`category`](Self::category) — the coarse routing axis, fixed when the error
///   is *created* (from the native error's [`CategorizedError`] impl, or given
///   explicitly), so it is correct and total, never re-derived from an opaque chain.
/// - [`codec`](Self::codec) — the originating codec's name (e.g. `Some("zenjpeg")`),
///   so a consumer can tell codecs apart without downcasting the detail.
/// - [`detail`](Self::detail) — the codec's native error, *optional*: a codec with
///   no error enum of its own builds a `CodecError` from a category alone
///   (see [`new`](Self::new)).
///
/// The handle is one word (the fields live behind a `Box`), so `At<CodecError>`
/// is two — small enough to return in registers, keeping every
/// `Result<_, At<CodecError>>` a codec threads through `?` off the stack.
///
/// Because `At<CodecError>` is a single *concrete* type, a consumer recovers all
/// of this after **any** erasure — a `Box<dyn Error>`, an `anyhow::Error`, or a
/// mapped wrapper — by downcasting to it (or via [`CodecErrorExt::codec_error`] /
/// [`error_category`](CodecErrorExt::error_category)). A trait-only classifier
/// would be invisible through those: erasure yields a `dyn Error`, not a
/// `dyn CategorizedError`.
///
/// `source()` returns the [`detail`](Self::detail) when present, so the typed
/// extractors ([`CodecErrorExt::limit_exceeded`], `find_cause::<T>()`, …) still
/// reach the underlying cause. `Display` is `"{codec}: {detail-or-category}"`.
///
/// # Example
///
/// ```rust
/// use zencodec::{
///     At, CategorizedError, CodecError, ErrorAtExt, ErrorCategory, ImageError, RequestError,
///     UnsupportedOperation,
/// };
///
/// // A codec's native error declares its name once and maps its variants:
/// #[derive(Debug)]
/// struct JpegError(UnsupportedOperation);
/// # impl core::fmt::Display for JpegError {
/// #     fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, "{}", self.0) }
/// # }
/// # impl core::error::Error for JpegError {}
/// impl CategorizedError for JpegError {
///     fn codec_name(&self) -> Option<&'static str> { Some("zenjpeg") }
///     fn category(&self) -> ErrorCategory { self.0.category() }
/// }
///
/// // `of` takes an already-located `At<E>`, keeps the trace on the outside, and
/// // reads the category AND the codec name from the type — no codec arg:
/// let e: At<CodecError> =
///     CodecError::of(JpegError(UnsupportedOperation::AnimationEncode).start_at());
/// assert_eq!(e.category(), ErrorCategory::Request(RequestError::Unsupported(UnsupportedOperation::AnimationEncode))); // via At's CategorizedError
/// assert_eq!(e.error().codec(), Some("zenjpeg"));
///
/// // `new` is the fundamental form — codec name passed, no detail:
/// let bare = CodecError::new(Some("zenjpeg"), ErrorCategory::Image(ImageError::Malformed));
/// assert!(bare.detail().is_none());
///
/// // Everything survives erasure to a trait object — recover by concrete downcast:
/// let boxed: Box<dyn core::error::Error + Send + Sync> = Box::new(e);
/// let recovered = boxed.downcast_ref::<At<CodecError>>().unwrap();
/// assert_eq!(recovered.category(), ErrorCategory::Request(RequestError::Unsupported(UnsupportedOperation::AnimationEncode)));
/// assert_eq!(recovered.error().codec(), Some("zenjpeg"));
/// ```
pub struct CodecError(Box<Repr>);

// Boxed so the `CodecError` handle is a single non-null word: `At<CodecError>`
// is then two words (handle + trace) — small enough to return in registers,
// keeping `Result<_, At<CodecError>>` off the stack on the `?` path. The detail
// (a fat `Box<dyn Error>`) and the rest live behind the one box. Cold-path: one
// allocation per error, which is fine for an error type.
struct Repr {
    category: ErrorCategory,
    codec: Option<&'static str>,
    detail: Option<Box<dyn core::error::Error + Send + Sync>>,
}

impl core::fmt::Debug for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CodecError")
            .field("category", &self.0.category)
            .field("codec", &self.0.codec)
            .field("detail", &self.0.detail)
            .finish()
    }
}

impl CodecError {
    /// A fundamental codec error: a [`category`](Self::category) and the
    /// originating `codec` name, with **no** detail — for a codec that has no
    /// error enum of its own. `Display` renders the category's phrase, prefixed by
    /// the codec when present. Pass the codec name as `Some("mycodec")` (or `None`
    /// for an unidentified error); the codec then locates it with `.start_at()` to
    /// reach its `At<CodecError>` `type Error`.
    #[inline]
    pub fn new(codec: Option<&'static str>, category: ErrorCategory) -> Self {
        CodecError(Box::new(Repr {
            category,
            codec,
            detail: None,
        }))
    }

    /// Build the envelope from a codec's native error, capturing both its
    /// [`category()`](CategorizedError::category) and its codec name
    /// ([`codec_name()`](CategorizedError::codec_name)) from the value — no codec
    /// argument. Returns a bare `CodecError`; pair it with whereat's
    /// [`map_err_at`](whereat::ResultAtExt::map_err_at) to locate and convert a
    /// `Result<_, E>` in one step (`r.map_err_at(CodecError::from_native)?`), or
    /// prefer [`of`](Self::of) when you already hold an `At<E>`.
    #[inline]
    pub fn from_native<E>(detail: E) -> Self
    where
        E: CategorizedError + core::error::Error + Send + Sync + 'static,
    {
        // The codec name comes from the value's `codec_name()`. A shared cause type
        // (e.g. `UnsupportedOperation`, `LimitExceeded`) keeps the default `None`,
        // so building an envelope from one directly is unidentified — wrap it in
        // your codec's own error type first, or use `from_parts` with an explicit
        // name. This catches that (and a codec that forgot `codec_name`) in dev; it
        // is a no-op in release.
        debug_assert!(
            detail.codec_name().is_some(),
            "CodecError built from a type with no codec_name ({}); wrap it in \
             your codec's error type, or use from_parts with an explicit name",
            core::any::type_name::<E>(),
        );
        CodecError(Box::new(Repr {
            category: detail.category(),
            codec: detail.codec_name(),
            detail: Some(Box::new(detail)),
        }))
    }

    /// Wrap a codec's **already-located** native error as `At<CodecError>`,
    /// preserving the location trace.
    ///
    /// Taking `At<E>` (not a bare `E`) makes location *mandatory at the type
    /// level*: a codec that skipped whereat cannot call `of`, so the omission is a
    /// compile error, not a silently trace-less error. The `At` stays on the
    /// **outside** (`At<CodecError>` — never an `At<E>` buried in the detail), so
    /// `.at()` / [`contexts`](whereat::At::contexts) keep working on the envelope.
    /// Typical use: `inner().map_err(CodecError::of)?`.
    #[inline]
    pub fn of<E>(located: At<E>) -> At<CodecError>
    where
        E: CategorizedError + core::error::Error + Send + Sync + 'static,
    {
        located.map_error(CodecError::from_native::<E>)
    }

    /// Build from an explicit category and an already-boxed detail error — for
    /// codecs whose native error does not implement [`CategorizedError`]. `codec`
    /// is the originating codec's name (`Some("mycodec")`), or `None` if unset.
    #[inline]
    pub fn from_parts(
        codec: Option<&'static str>,
        category: ErrorCategory,
        detail: Box<dyn core::error::Error + Send + Sync>,
    ) -> Self {
        CodecError(Box::new(Repr {
            category,
            codec,
            detail: Some(detail),
        }))
    }

    /// Set (or clear) the originating codec name on an existing envelope.
    ///
    /// Since the codec name is optional and defaults to `None`, this is the way to
    /// stamp it after the fact — for a `CodecError` built without one (e.g.
    /// [`from_parts`](Self::from_parts) with `None`, or a generic helper that
    /// wrapped a foreign error) that a codec then wants to attribute to itself.
    /// Builder form, so it chains:
    /// `CodecError::from_parts(None, cat, e).with_codec(Some("zenjpeg"))`. For the
    /// located form, apply it before locating, or via
    /// `at.map_error(|e| e.with_codec(Some("zenjpeg")))`.
    #[inline]
    #[must_use]
    pub fn with_codec(mut self, codec: Option<&'static str>) -> Self {
        self.0.codec = codec;
        self
    }

    /// The coarse category — fixed at construction, total, allocation-free.
    #[inline]
    pub fn category(&self) -> ErrorCategory {
        self.0.category
    }

    /// The originating codec's name (e.g. `Some("zenjpeg")`), or `None` if unset.
    #[inline]
    pub fn codec(&self) -> Option<&'static str> {
        self.0.codec
    }

    /// The codec's native error (its message and its own `source()` chain), if any.
    #[inline]
    pub fn detail(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.0
            .detail
            .as_deref()
            .map(|d| d as &(dyn core::error::Error + 'static))
    }
}

/// The envelope is itself categorized: `category()` and `codec_name()` return its
/// stored fields, so a bare `CodecError` — and `At<CodecError>` via the blanket
/// [`At`] impl — both answer [`CategorizedError`]. (Here the codec name really is
/// per-instance, read from [`codec`](CodecError::codec), since one envelope type
/// fronts every codec.)
impl CategorizedError for CodecError {
    #[inline]
    fn codec_name(&self) -> Option<&'static str> {
        self.0.codec
    }
    #[inline]
    fn category(&self) -> ErrorCategory {
        self.0.category
    }
}

impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // "{codec}: {detail-or-category}". The category is a separate axis; it is
        // rendered as text only when there is no detail message to show.
        if let Some(codec) = self.0.codec {
            write!(f, "{codec}: ")?;
        }
        match &self.0.detail {
            Some(detail) => core::fmt::Display::fmt(detail, f),
            None => core::fmt::Display::fmt(&self.0.category, f),
        }
    }
}

impl core::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.0
            .detail
            .as_deref()
            .map(|d| d as &(dyn core::error::Error + 'static))
    }
}

/// A byte offset into the **encoded input**, for attaching to an error's
/// location trace as retrievable context.
///
/// The most useful diagnostic a decoder can give is *where* in the input it
/// failed — and decoders already track this internally. Rather than grow a
/// per-locus field on [`CodecError`], a codec attaches the offset to its
/// `At<CodecError>` trace as typed context with
/// [`At::at_data`](whereat::At::at_data). It is a shared type so a generic
/// consumer can recover it by [`downcast_ref`](whereat::AtContextRef::downcast_ref)
/// **without naming any codec-specific type** — the cross-codec convention for
/// "where did it fail". The unit is bytes from the start of the encoded input.
///
/// ```rust
/// use zencodec::{At, CodecError, ErrorAtExt, ErrorCategory, ImageError, StreamOffset};
///
/// // A codec attaches the offset where parsing failed to its error's trace:
/// let err: At<CodecError> = CodecError::new(Some("zenjpeg"), ErrorCategory::Image(ImageError::Malformed))
///     .start_at()
///     .at_data(|| StreamOffset(42));
///
/// // A generic consumer recovers it — even after erasure to `Box<dyn Error>`:
/// let boxed: Box<dyn core::error::Error + Send + Sync> = Box::new(err);
/// let at = boxed.downcast_ref::<At<CodecError>>().unwrap();
/// let offset = at.contexts().find_map(|c| c.downcast_ref::<StreamOffset>().copied());
/// assert_eq!(offset, Some(StreamOffset(42)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StreamOffset(pub u64);

impl core::fmt::Display for StreamOffset {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "byte offset {}", self.0)
    }
}

/// Extension trait for inspecting codec errors.
///
/// Blanket-implemented for all `core::error::Error + 'static` types.
/// Walks the [`source()`](core::error::Error::source) chain to find
/// common codec error causes without knowing the concrete error type.
///
/// Works through any wrapper that delegates `source()`:
/// `thiserror` `#[from]` variants, `whereat::At<E>`, `Box<dyn Error>`, etc.
/// For coarse routing prefer a codec's [`CategorizedError`] impl; these typed
/// extractors recover the underlying cause when you need its detail.
///
/// # Example
///
/// ```rust,ignore
/// use zencodec::CodecErrorExt;
///
/// let result = dyn_decoder.decode();
/// if let Err(ref e) = result {
///     if let Some(limit) = e.limit_exceeded() {
///         eprintln!("limit exceeded: {limit}");
///     } else if let Some(op) = e.unsupported_operation() {
///         eprintln!("not supported: {op}");
///     }
/// }
/// ```
pub trait CodecErrorExt {
    /// Find an [`UnsupportedOperation`] in this error's cause chain.
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation>;

    /// Find a [`LimitExceeded`] in this error's cause chain.
    fn limit_exceeded(&self) -> Option<&LimitExceeded>;

    /// Recover the shared [`CodecError`] envelope from this error's chain — after
    /// **any** erasure (`Box<dyn Error>`, `anyhow::Error`, a mapped wrapper) — by
    /// downcasting to the concrete `At<CodecError>` (or a bare `CodecError`),
    /// tolerating a single `Box` layer (`Box<At<CodecError>>` etc., so a codec may
    /// use an 8-byte boxed `type Error`). It gives the
    /// [`category`](CodecError::category) *and* the originating
    /// [`codec`](CodecError::codec) name, so a consumer can tell codecs apart
    /// without naming any codec-specific type.
    ///
    /// `None` only when the error did not originate from a zen codec via the
    /// envelope. The provided default returns `None`; the blanket impl over
    /// `core::error::Error` and the `dyn Error` impls override it with the real
    /// lookup.
    fn codec_error(&self) -> Option<&CodecError> {
        None
    }

    /// Shortcut for
    /// [`codec_error()`](Self::codec_error)`.map(CodecError::category)` — the
    /// coarse axis app/lib code routes on.
    fn error_category(&self) -> Option<ErrorCategory> {
        self.codec_error().map(CodecError::category)
    }

    /// Find a cause of arbitrary type `T` in this error's cause chain.
    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T>;
}

/// Recover the shared [`CodecError`] envelope from an error chain, tolerating a
/// single `Box` layer in either position. Backs [`CodecErrorExt::codec_error`].
///
/// Recovery is downcast-based, so it must name the concrete shapes it accepts: the
/// canonical `At<CodecError>` (and bare `CodecError`), plus a one-deep `Box` a
/// caller may have applied — `Box<At<CodecError>>`, `At<Box<CodecError>>`, and
/// `Box<CodecError>`. (`CodecError` is already a one-word handle, so a codec's
/// `type Error = At<CodecError>` needs no extra boxing; these probes cover a
/// consumer that boxed anyway.) A `Box`'s `source()` forwards *past* the envelope,
/// so each boxed shape needs its own probe rather than falling out of the chain
/// walk; deeper nesting (`Box<Box<…>>`) is not covered (and shouldn't occur).
fn recover_codec_error<'a>(err: &'a (dyn core::error::Error + 'static)) -> Option<&'a CodecError> {
    if let Some(at) = find_cause::<At<CodecError>>(err) {
        return Some(at.error());
    }
    if let Some(ce) = find_cause::<CodecError>(err) {
        return Some(ce);
    }
    if let Some(b) = find_cause::<Box<At<CodecError>>>(err) {
        return Some(b.error());
    }
    if let Some(at) = find_cause::<At<Box<CodecError>>>(err) {
        return Some(&**at.error());
    }
    if let Some(b) = find_cause::<Box<CodecError>>(err) {
        return Some(&**b);
    }
    None
}

impl<E: core::error::Error + 'static> CodecErrorExt for E {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation> {
        find_cause::<UnsupportedOperation>(self)
    }

    fn limit_exceeded(&self) -> Option<&LimitExceeded> {
        find_cause::<LimitExceeded>(self)
    }

    fn codec_error(&self) -> Option<&CodecError> {
        recover_codec_error(self)
    }

    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T> {
        find_cause::<T>(self)
    }
}

// Manual impl for trait objects — the blanket impl requires Sized.
impl CodecErrorExt for dyn core::error::Error + Send + Sync + 'static {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation> {
        find_cause::<UnsupportedOperation>(self)
    }

    fn limit_exceeded(&self) -> Option<&LimitExceeded> {
        find_cause::<LimitExceeded>(self)
    }

    fn codec_error(&self) -> Option<&CodecError> {
        recover_codec_error(self)
    }

    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T> {
        find_cause::<T>(self)
    }
}

impl CodecErrorExt for dyn core::error::Error + Send + 'static {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation> {
        find_cause::<UnsupportedOperation>(self)
    }

    fn limit_exceeded(&self) -> Option<&LimitExceeded> {
        find_cause::<LimitExceeded>(self)
    }

    fn codec_error(&self) -> Option<&CodecError> {
        recover_codec_error(self)
    }

    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T> {
        find_cause::<T>(self)
    }
}

impl CodecErrorExt for dyn core::error::Error + 'static {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation> {
        find_cause::<UnsupportedOperation>(self)
    }

    fn limit_exceeded(&self) -> Option<&LimitExceeded> {
        find_cause::<LimitExceeded>(self)
    }

    fn codec_error(&self) -> Option<&CodecError> {
        recover_codec_error(self)
    }

    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T> {
        find_cause::<T>(self)
    }
}

/// Walk an error's [`source()`](core::error::Error::source) chain to find
/// a cause of type `T`.
///
/// Starts with the error itself, then follows `source()` links. Returns
/// the first match.
///
/// Prefer [`CodecErrorExt`] methods for common types. Use this for
/// codec-specific error types not covered by the extension trait.
pub fn find_cause<'a, T: core::error::Error + 'static>(
    mut err: &'a (dyn core::error::Error + 'static),
) -> Option<&'a T> {
    loop {
        if let Some(t) = err.downcast_ref::<T>() {
            return Some(t);
        }
        err = err.source()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use alloc::string::String;
    use core::fmt;

    // A codec error that opts into CategorizedError and exposes typed causes
    // via source() — exactly the pattern a real codec follows.
    #[derive(Debug)]
    enum TestCodecError {
        Limit(LimitExceeded),
        Unsupported(UnsupportedOperation),
        Cancelled(StopReason),
        Malformed(String),
        // Valid input the codec could handle, refused by a configured policy.
        RejectedByPolicy,
        // Caller-supplied pixel buffer has the wrong geometry.
        BadBuffer,
        // API called out of sequence (e.g. push after finish).
        WrongState,
    }

    impl fmt::Display for TestCodecError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Limit(e) => write!(f, "limit: {e}"),
                Self::Unsupported(e) => write!(f, "unsupported: {e}"),
                Self::Cancelled(r) => write!(f, "cancelled: {r}"),
                Self::Malformed(s) => write!(f, "malformed: {s}"),
                Self::RejectedByPolicy => write!(f, "rejected by policy"),
                Self::BadBuffer => write!(f, "bad buffer"),
                Self::WrongState => write!(f, "wrong state"),
            }
        }
    }

    impl core::error::Error for TestCodecError {
        fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
            match self {
                Self::Limit(e) => Some(e),
                Self::Unsupported(e) => Some(e),
                // StopReason is not an Error, so it isn't a source link.
                Self::Cancelled(_)
                | Self::Malformed(_)
                | Self::RejectedByPolicy
                | Self::BadBuffer
                | Self::WrongState => None,
            }
        }
    }

    impl CategorizedError for TestCodecError {
        fn codec_name(&self) -> Option<&'static str> {
            Some("test-codec")
        }
        fn category(&self) -> ErrorCategory {
            match self {
                Self::Limit(e) => e.category(),
                Self::Unsupported(e) => e.category(),
                Self::Cancelled(r) => r.category(),
                Self::Malformed(_) => ErrorCategory::Image(ImageError::Malformed),
                Self::RejectedByPolicy => ErrorCategory::Policy(PolicyKind::Decode),
                Self::BadBuffer => {
                    ErrorCategory::Request(RequestError::Invalid(InvalidKind::Buffer))
                }
                Self::WrongState => {
                    ErrorCategory::Request(RequestError::Invalid(InvalidKind::State))
                }
            }
        }
    }

    #[test]
    fn ext_limit_exceeded_direct() {
        let err = LimitExceeded::Width {
            actual: 5000,
            max: 4096,
        };
        assert_eq!(err.limit_exceeded(), Some(&err));
    }

    #[test]
    fn ext_limit_exceeded_through_source_chain() {
        let inner = LimitExceeded::Pixels {
            actual: 100_000_000,
            max: 50_000_000,
        };
        let err = TestCodecError::Limit(inner.clone());
        assert_eq!(err.limit_exceeded(), Some(&inner));
    }

    #[test]
    fn ext_unsupported_through_source_chain() {
        let err = TestCodecError::Unsupported(UnsupportedOperation::AnimationEncode);
        assert_eq!(
            err.unsupported_operation(),
            Some(&UnsupportedOperation::AnimationEncode)
        );
    }

    #[test]
    fn ext_returns_none_when_absent() {
        let err = TestCodecError::Malformed("something else".into());
        assert!(err.limit_exceeded().is_none());
        assert!(err.unsupported_operation().is_none());
    }

    #[test]
    fn ext_through_boxed_error() {
        let inner = LimitExceeded::Memory {
            actual: 1_000_000_000,
            max: 512_000_000,
        };
        let err = TestCodecError::Limit(inner.clone());
        let boxed: Box<dyn core::error::Error + Send + Sync> = Box::new(err);
        assert_eq!(boxed.limit_exceeded(), Some(&inner));
    }

    #[test]
    fn ext_find_cause_generic() {
        let err = TestCodecError::Unsupported(UnsupportedOperation::DecodeInto);
        let found: Option<&UnsupportedOperation> = err.find_cause();
        assert_eq!(found, Some(&UnsupportedOperation::DecodeInto));
    }

    // find_cause free function still works
    #[test]
    fn find_cause_free_fn() {
        let err = LimitExceeded::Width {
            actual: 5000,
            max: 4096,
        };
        let found = find_cause::<LimitExceeded>(&err);
        assert_eq!(found, Some(&err));
    }

    // ---- CategorizedError ----

    #[test]
    fn category_maps_each_codec_variant() {
        assert_eq!(
            TestCodecError::Malformed("x".into()).category(),
            ErrorCategory::Image(ImageError::Malformed)
        );
        assert_eq!(
            TestCodecError::RejectedByPolicy.category(),
            ErrorCategory::Policy(PolicyKind::Decode)
        );
        assert_eq!(
            TestCodecError::BadBuffer.category(),
            ErrorCategory::Request(RequestError::Invalid(InvalidKind::Buffer))
        );
        assert_eq!(
            TestCodecError::WrongState.category(),
            ErrorCategory::Request(RequestError::Invalid(InvalidKind::State))
        );
        assert_eq!(
            TestCodecError::Unsupported(UnsupportedOperation::AnimationEncode).category(),
            ErrorCategory::Request(RequestError::Unsupported(
                UnsupportedOperation::AnimationEncode
            ))
        );
        assert_eq!(
            TestCodecError::Cancelled(StopReason::Cancelled).category(),
            ErrorCategory::Lifecycle(StopReason::Cancelled)
        );
        assert_eq!(
            TestCodecError::Cancelled(StopReason::TimedOut).category(),
            ErrorCategory::Lifecycle(StopReason::TimedOut)
        );
        assert_eq!(
            TestCodecError::Limit(LimitExceeded::Pixels { actual: 9, max: 4 }).category(),
            ErrorCategory::Resource(ResourceError::Limits(LimitKind::Pixels))
        );
    }

    #[test]
    fn zencodec_cause_types_categorize() {
        assert_eq!(
            StopReason::Cancelled.category(),
            ErrorCategory::Lifecycle(StopReason::Cancelled)
        );
        assert_eq!(
            StopReason::TimedOut.category(),
            ErrorCategory::Lifecycle(StopReason::TimedOut)
        );
        // The operation axis splits: a plain operation vs the pixel-format arm.
        assert_eq!(
            UnsupportedOperation::AnimationDecode.category(),
            ErrorCategory::Request(RequestError::Unsupported(
                UnsupportedOperation::AnimationDecode
            ))
        );
        assert_eq!(
            UnsupportedOperation::PixelFormat.category(),
            ErrorCategory::Request(RequestError::Unsupported(UnsupportedOperation::PixelFormat))
        );
        assert_eq!(
            LimitExceeded::Memory { actual: 2, max: 1 }.category(),
            ErrorCategory::Resource(ResourceError::Limits(LimitKind::Memory))
        );
    }

    #[test]
    fn category_is_preserved_through_at() {
        // A located error (the form heic/zenbitmaps return) keeps its category,
        // and the inner error is still reachable for its detail.
        let located = At::wrap(TestCodecError::Cancelled(StopReason::TimedOut));
        assert_eq!(
            located.category(),
            ErrorCategory::Lifecycle(StopReason::TimedOut)
        );
        assert!(matches!(
            located.error(),
            TestCodecError::Cancelled(StopReason::TimedOut)
        ));
    }

    // ---- CodecError envelope ----

    #[test]
    fn codec_error_from_native_and_of_capture_category_and_codec() {
        // `from_native` reads both the category and the codec name from the type.
        let e = CodecError::from_native(TestCodecError::Malformed("bad".into()));
        assert_eq!(e.category(), ErrorCategory::Image(ImageError::Malformed));
        assert_eq!(e.codec(), Some("test-codec")); // TestCodecError::codec_name
        assert!(e.detail().is_some());
        // Display is "{codec}: {detail message}".
        assert_eq!(alloc::format!("{e}"), "test-codec: malformed: bad");
        // `of` is the located form: takes At<E>, keeps the trace on the outside.
        let located: At<CodecError> =
            CodecError::of(At::wrap(TestCodecError::Malformed("bad".into())));
        assert_eq!(
            located.category(),
            ErrorCategory::Image(ImageError::Malformed)
        );
        assert_eq!(located.error().codec(), Some("test-codec"));
        // from_parts takes an explicit category + codec name (no typed detail):
        let e2 = CodecError::from_parts(
            Some("zenjpeg"),
            ErrorCategory::Io(CodecIoKind::opaque()),
            Box::new(TestCodecError::Malformed("x".into())),
        );
        assert_eq!(e2.category(), ErrorCategory::Io(CodecIoKind::opaque()));
        assert_eq!(e2.codec(), Some("zenjpeg"));
    }

    #[test]
    fn codec_error_new_is_detail_free() {
        // The fundamental form: a codec with no error enum of its own.
        let e = CodecError::new(Some("zenpng"), ErrorCategory::Image(ImageError::Malformed));
        assert_eq!(e.category(), ErrorCategory::Image(ImageError::Malformed));
        assert_eq!(e.codec(), Some("zenpng"));
        assert!(e.detail().is_none());
        // With no detail, Display falls back to the category's phrase.
        assert_eq!(alloc::format!("{e}"), "zenpng: malformed image");
    }

    #[test]
    fn codec_error_recovers_through_box_dyn_error() {
        // The dyn-dispatch path: At<CodecError> erased to Box<dyn Error>. Both the
        // category AND the originating codec name survive erasure.
        let located = CodecError::of(At::wrap(TestCodecError::Cancelled(StopReason::Cancelled)));
        let boxed: Box<dyn core::error::Error + Send + Sync> = Box::new(located);
        assert_eq!(
            boxed.error_category(),
            Some(ErrorCategory::Lifecycle(StopReason::Cancelled))
        );
        assert_eq!(
            boxed.codec_error().and_then(CodecError::codec),
            Some("test-codec")
        );
        // A bare CodecError (no At) recovers too.
        let bare: Box<dyn core::error::Error + Send + Sync> =
            Box::new(CodecError::from_native(TestCodecError::WrongState));
        assert_eq!(
            bare.error_category(),
            Some(ErrorCategory::Request(RequestError::Invalid(
                InvalidKind::State
            )))
        );
        assert_eq!(
            bare.codec_error().and_then(CodecError::codec),
            Some("test-codec")
        );
        // A non-codec error has no envelope.
        let other: Box<dyn core::error::Error + Send + Sync> =
            Box::new(TestCodecError::Malformed("not wrapped".into()));
        assert_eq!(other.error_category(), None);
        assert!(other.codec_error().is_none());
    }

    #[test]
    fn codec_error_recovers_through_a_wrapping_error() {
        // The "map to own type / anyhow" path: a consumer error whose source()
        // is the At<CodecError>. codec_error() walks the chain and downcasts to it.
        #[derive(Debug)]
        struct Wrap(At<CodecError>);
        impl fmt::Display for Wrap {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "wrap: {}", self.0)
            }
        }
        impl core::error::Error for Wrap {
            fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let wrapped = Wrap(CodecError::of(At::wrap(TestCodecError::Limit(
            LimitExceeded::Pixels { actual: 9, max: 4 },
        ))));
        assert_eq!(
            wrapped.error_category(),
            Some(ErrorCategory::Resource(ResourceError::Limits(
                LimitKind::Pixels
            )))
        );
        assert_eq!(
            wrapped.codec_error().and_then(CodecError::codec),
            Some("test-codec")
        );
    }

    #[test]
    fn codec_error_typed_cause_still_findable() {
        // The native cause stays reachable via source()/find_cause for detail,
        // alongside the coarse category.
        let inner = LimitExceeded::Memory { actual: 9, max: 4 };
        let located = CodecError::of(At::wrap(TestCodecError::Limit(inner.clone())));
        assert_eq!(located.limit_exceeded(), Some(&inner));
        assert_eq!(
            located.error_category(),
            Some(ErrorCategory::Resource(ResourceError::Limits(
                LimitKind::Memory
            )))
        );
    }

    #[test]
    fn codec_error_at_trace_is_present() {
        // `.at()` builds the trace on the outer At<CodecError>; Debug renders it.
        let e = CodecError::of(At::wrap(TestCodecError::Malformed("x".into()))).at();
        let dbg = alloc::format!("{e:?}");
        assert!(
            dbg.contains("at "),
            "trace frame should render in Debug: {dbg}"
        );
    }

    #[test]
    fn stream_offset_rides_the_trace_through_erasure() {
        // The shared locus convention: a byte offset attached as trace context,
        // recovered generically (no codec-specific type named) after erasure.
        let err = At::wrap(CodecError::new(
            Some("zenjpeg"),
            ErrorCategory::Image(ImageError::Malformed),
        ))
        .at_data(|| StreamOffset(1234));
        let boxed: Box<dyn core::error::Error + Send + Sync> = Box::new(err);
        let at = boxed
            .downcast_ref::<At<CodecError>>()
            .expect("downcast to the concrete envelope");
        let offset = at
            .contexts()
            .find_map(|c| c.downcast_ref::<StreamOffset>().copied());
        assert_eq!(offset, Some(StreamOffset(1234)));
        // The category recovers alongside the locus.
        assert_eq!(
            boxed.error_category(),
            Some(ErrorCategory::Image(ImageError::Malformed))
        );
    }

    #[test]
    fn error_types_stay_small() {
        use core::mem::size_of;
        let word = size_of::<usize>();
        // `CodecError` is a boxed handle — one non-null word — so `At<CodecError>`
        // is two (handle + trace), and every `Result<_, At<CodecError>>` a codec
        // threads through `?` is two words too (the box pointer's niche absorbs the
        // discriminant): small enough to return in registers rather than spill to
        // the stack. Guards against the handle un-boxing or the trace growing.
        // (Word-relative so it holds on 32-bit too, e.g. i686.)
        assert!(
            size_of::<CodecError>() <= word,
            "CodecError = {} bytes (expected <= {})",
            size_of::<CodecError>(),
            word
        );
        assert!(
            size_of::<At<CodecError>>() <= 2 * word,
            "At<CodecError> = {} bytes (expected <= {})",
            size_of::<At<CodecError>>(),
            2 * word
        );
        assert!(
            size_of::<Result<(), At<CodecError>>>() <= 2 * word,
            "Result<(), At<CodecError>> = {} bytes (expected <= {})",
            size_of::<Result<(), At<CodecError>>>(),
            2 * word
        );
    }
    #[test]
    fn recovery_tolerates_a_single_box_layer() {
        // Recovery is downcast-based, but probes a single `Box` layer in either
        // position too, so a consumer that boxed the (already one-word) envelope is
        // still classifiable.
        let mk = || CodecError::new(Some("zenjpeg"), ErrorCategory::Image(ImageError::Malformed));

        // Canonical `At<CodecError>`.
        let canonical: Box<dyn core::error::Error + Send + Sync> = Box::new(At::wrap(mk()));
        assert_eq!(
            canonical.error_category(),
            Some(ErrorCategory::Image(ImageError::Malformed))
        );
        assert_eq!(
            canonical.codec_error().and_then(CodecError::codec),
            Some("zenjpeg")
        );

        // `Box<At<CodecError>>` — a consumer-applied box.
        let boxed_at: Box<dyn core::error::Error + Send + Sync> =
            Box::new(Box::new(At::wrap(mk())) as Box<At<CodecError>>);
        assert_eq!(
            boxed_at.error_category(),
            Some(ErrorCategory::Image(ImageError::Malformed))
        );
        assert_eq!(
            boxed_at.codec_error().and_then(CodecError::codec),
            Some("zenjpeg")
        );

        // `At<Box<CodecError>>`.
        let at_boxed: Box<dyn core::error::Error + Send + Sync> =
            Box::new(At::wrap(Box::new(mk()) as Box<CodecError>));
        assert_eq!(
            at_boxed.error_category(),
            Some(ErrorCategory::Image(ImageError::Malformed))
        );

        // `Box<CodecError>` (no `At`).
        let bare_boxed: Box<dyn core::error::Error + Send + Sync> =
            Box::new(Box::new(mk()) as Box<CodecError>);
        assert_eq!(
            bare_boxed.error_category(),
            Some(ErrorCategory::Image(ImageError::Malformed))
        );

        // The fallback is one layer deep — a pathological double box is not covered.
        let double: Box<dyn core::error::Error + Send + Sync> =
            Box::new(Box::new(Box::new(At::wrap(mk())) as Box<At<CodecError>>));
        assert_eq!(double.error_category(), None);
    }

    #[test]
    fn io_kind_variant_is_portable_and_carries_kind_under_std() {
        // The variant shape is stable across builds; matching `Io(_)` is portable.
        let cat = ErrorCategory::Io(CodecIoKind::opaque());
        assert!(matches!(cat, ErrorCategory::Io(_)));
        #[cfg(feature = "std")]
        {
            let k: CodecIoKind = std::io::ErrorKind::UnexpectedEof.into();
            assert_eq!(k.kind(), Some(std::io::ErrorKind::UnexpectedEof));
            assert_eq!(CodecIoKind::opaque().kind(), None);
        }
    }

    #[test]
    fn categorized_error_is_dyn_downcastable_via_any() {
        // `CategorizedError: Any` lets a `dyn CategorizedError` upcast to `dyn Any`
        // and downcast to the concrete type (recovery still prefers the envelope).
        use core::any::Any;
        let err = TestCodecError::WrongState;
        let dynamic: &dyn CategorizedError = &err;
        assert_eq!(
            dynamic.category(),
            ErrorCategory::Request(RequestError::Invalid(InvalidKind::State))
        );
        let any: &dyn Any = dynamic; // trait upcasting (Rust 1.86+)
        assert!(any.downcast_ref::<TestCodecError>().is_some());
    }

    #[test]
    fn with_codec_stamps_or_clears_the_name() {
        // A foreign error wrapped with no codec name, then stamped via the builder.
        let e = CodecError::from_parts(
            None,
            ErrorCategory::Internal(InternalKind::Bug),
            Box::new(TestCodecError::Malformed("x".into())),
        );
        assert_eq!(e.codec(), None);
        let stamped = e.with_codec(Some("zenjpeg"));
        assert_eq!(stamped.codec(), Some("zenjpeg"));
        // It can also clear the name back to None.
        assert_eq!(stamped.with_codec(None).codec(), None);
    }
}
