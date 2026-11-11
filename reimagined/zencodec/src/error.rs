//! One error type for the whole contract.
//!
//! Every contract method returns [`Result<T>`](Result), whose error is
//! [`At<CodecError>`](Error): a kind every caller can decide on, the codec's
//! name, and the codec's own error as the cause. The cause keeps its type:
//! [`CodecError::native_as`] and [`find_cause`] recover it.

use alloc::boxed::Box;
use core::fmt;
use whereat::At;

pub use how_far::StopReason;

/// The contract's error: a located [`CodecError`].
pub type Error = At<CodecError>;

/// The contract's result.
pub type Result<T, E = Error> = core::result::Result<T, E>;

type Native = Box<dyn core::error::Error + Send + Sync + 'static>;

/// What went wrong, in terms a caller can act on without knowing the codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The bytes are not a valid image of this format.
    Malformed,
    /// The input ended before the image did.
    Truncated,
    /// A requirement of the request cannot be met by this codec or configuration.
    Unsupported(Unsupported),
    /// A resource gate in [`ResourceLimits`](crate::ResourceLimits) refused the work.
    Limit(LimitKind),
    /// The pulse stopped the work.
    Stopped(StopReason),
    /// An allocation failed.
    OutOfMemory,
    /// The byte source failed; the cause is attached.
    Source,
    /// The row sink or byte destination failed; the cause is attached.
    Sink,
    /// The caller or driver broke the protocol (wrong rows, step after an error).
    Protocol,
    /// The codec found an inconsistency in its own state.
    Internal,
}

/// The requirement that could not be met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unsupported {
    /// The required fidelity (lossless, or a lossy target).
    Fidelity,
    /// The required effort.
    Effort,
    /// None of the required pixel formats, or this input format.
    PixelFormat,
    /// Planar rows, in or out.
    Planar,
    /// More than one frame.
    Animation,
    /// A metadata kind that had to be embedded, or had to be known before pixels.
    Metadata,
    /// Rows offered only once to an encoder that cannot hold them.
    RowSupply,
    /// Provisional passes.
    Progressive,
    /// These dimensions.
    Dimensions,
    /// The whole operation.
    Operation,
}

/// Which gate refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitKind {
    /// Image width.
    Width,
    /// Image height.
    Height,
    /// Pixels per frame.
    Pixels,
    /// Frame count.
    Frames,
    /// Input bytes.
    InputBytes,
    /// Output bytes.
    OutputBytes,
    /// Working memory, including a codec that cannot bound it.
    Memory,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => f.write_str("malformed image"),
            Self::Truncated => f.write_str("input ended early"),
            Self::Unsupported(u) => write!(f, "unsupported: {u:?}"),
            Self::Limit(l) => write!(f, "limit exceeded: {l:?}"),
            Self::Stopped(r) => write!(f, "stopped: {r:?}"),
            Self::OutOfMemory => f.write_str("out of memory"),
            Self::Source => f.write_str("source failed"),
            Self::Sink => f.write_str("sink failed"),
            Self::Protocol => f.write_str("protocol violation"),
            Self::Internal => f.write_str("internal error"),
        }
    }
}

/// A codec's own error type, classified once so `?` can lift it.
///
/// ```
/// use zencodec::{CodecError, ErrorKind, NativeError};
///
/// #[derive(Debug)]
/// struct BadChunk(u32);
/// impl core::fmt::Display for BadChunk {
///     fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
///         write!(f, "bad chunk {}", self.0)
///     }
/// }
/// impl core::error::Error for BadChunk {}
/// impl NativeError for BadChunk {
///     const CODEC: &'static str = "demo";
///     fn kind(&self) -> ErrorKind {
///         ErrorKind::Malformed
///     }
/// }
///
/// let e = CodecError::from_native(BadChunk(7));
/// assert_eq!(e.kind(), ErrorKind::Malformed);
/// assert_eq!(e.native_as::<BadChunk>().unwrap().0, 7);
/// ```
pub trait NativeError: core::error::Error + Send + Sync + 'static {
    /// The codec's name, as in [`DecodeCodec::name`](crate::decode::DecodeCodec::name).
    const CODEC: &'static str;
    /// How a caller that does not know this type should treat it.
    fn kind(&self) -> ErrorKind;
}

struct Repr {
    kind: ErrorKind,
    codec: Option<&'static str>,
    message: Option<&'static str>,
    native: Option<Native>,
}

/// A kind, the codec that raised it, and the codec's own error as the cause.
///
/// One word wide, so `Result<_, At<CodecError>>` stays in registers.
pub struct CodecError(Box<Repr>);

impl CodecError {
    /// An error of `kind` with no codec or cause.
    pub fn new(kind: ErrorKind) -> Self {
        Self(Box::new(Repr {
            kind,
            codec: None,
            message: None,
            native: None,
        }))
    }

    /// Lift a codec's own error, keeping it as the cause.
    pub fn from_native<E: NativeError>(native: E) -> Self {
        Self::new(native.kind())
            .with_codec(E::CODEC)
            .with_native(native)
    }

    /// Lift a located native error, keeping its trace.
    pub fn lift<E: NativeError>(native: At<E>) -> Error {
        native.map_error(Self::from_native)
    }

    /// Name the codec.
    pub fn with_codec(mut self, codec: &'static str) -> Self {
        self.0.codec = Some(codec);
        self
    }

    /// Add a fixed explanation.
    pub fn with_message(mut self, message: &'static str) -> Self {
        self.0.message = Some(message);
        self
    }

    /// Attach the original error.
    pub fn with_native(mut self, native: impl core::error::Error + Send + Sync + 'static) -> Self {
        self.0.native = Some(Box::new(native));
        self
    }

    /// Locate this error at the caller.
    #[track_caller]
    pub fn at(self) -> Error {
        At::wrap(self).at()
    }

    /// What a caller should decide on.
    pub fn kind(&self) -> ErrorKind {
        self.0.kind
    }

    /// The codec that raised it.
    pub fn codec(&self) -> Option<&'static str> {
        self.0.codec
    }

    /// The fixed explanation, if any.
    pub fn message(&self) -> Option<&'static str> {
        self.0.message
    }

    /// The original error.
    pub fn native(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.0
            .native
            .as_deref()
            .map(|n| n as &(dyn core::error::Error + 'static))
    }

    /// The original error, or one of its causes, as `T`.
    pub fn native_as<T: core::error::Error + 'static>(&self) -> Option<&T> {
        self.native().and_then(find_cause::<T>)
    }

    /// Whether the pulse stopped the work.
    pub fn stop_reason(&self) -> Option<StopReason> {
        match self.0.kind {
            ErrorKind::Stopped(r) => Some(r),
            _ => None,
        }
    }
}

impl fmt::Debug for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodecError")
            .field("kind", &self.0.kind)
            .field("codec", &self.0.codec)
            .field("message", &self.0.message)
            .field("native", &self.0.native)
            .finish()
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(codec) = self.0.codec {
            write!(f, "{codec}: ")?;
        }
        write!(f, "{}", self.0.kind)?;
        if let Some(message) = self.0.message {
            write!(f, " ({message})")?;
        }
        if let Some(native) = &self.0.native {
            write!(f, ": {native}")?;
        }
        Ok(())
    }
}

impl core::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.native()
    }
}

impl From<ErrorKind> for CodecError {
    fn from(kind: ErrorKind) -> Self {
        Self::new(kind)
    }
}

impl From<StopReason> for CodecError {
    fn from(reason: StopReason) -> Self {
        Self::new(ErrorKind::Stopped(reason))
    }
}

/// A located error of `kind`.
#[track_caller]
pub fn error(kind: ErrorKind) -> Error {
    At::wrap(CodecError::new(kind)).at()
}

/// A located error of `kind` with a fixed explanation.
#[track_caller]
pub fn error_msg(kind: ErrorKind, message: &'static str) -> Error {
    At::wrap(CodecError::new(kind).with_message(message)).at()
}

/// A located refusal of one requirement.
#[track_caller]
pub fn unsupported(what: Unsupported) -> Error {
    At::wrap(CodecError::new(ErrorKind::Unsupported(what))).at()
}

/// Check the pulse, turning a stop into a located error.
#[track_caller]
pub fn check(pulse: &dyn how_far::Stop) -> Result<()> {
    match pulse.check() {
        Ok(()) => Ok(()),
        Err(reason) => Err(At::wrap(CodecError::from(reason)).at()),
    }
}

/// Find a `T` in an error's cause chain, looking through [`At`] wrappers.
///
/// Works on any erased error, so a caller holding only
/// `&dyn Error` can still reach the codec's own type.
pub fn find_cause<'a, T: core::error::Error + 'static>(
    mut err: &'a (dyn core::error::Error + 'static),
) -> Option<&'a T> {
    loop {
        if let Some(t) = err.downcast_ref::<T>() {
            return Some(t);
        }
        if let Some(at) = err.downcast_ref::<At<T>>() {
            return Some(at.error());
        }
        #[cfg(feature = "std")]
        if let Some(io) = err.downcast_ref::<std::io::Error>()
            && let Some(inner) = io.get_ref()
        {
            err = inner;
            continue;
        }
        err = err.source()?;
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for CodecError {
    /// A byte-source failure. Destinations use [`CodecError::with_native`]
    /// with [`ErrorKind::Sink`].
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorKind::Source).with_native(e)
    }
}
