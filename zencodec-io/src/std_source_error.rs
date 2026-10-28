//! Errors from standard-library byte sources, preserving native I/O causes.

use core::fmt;
use zencodec::source::SourceError;

/// A source-contract refusal or the original standard-library I/O failure.
///
/// The native error retains its kind, OS code and custom payload. Drivers and
/// [`zencodec::source::ReadAtShim`] preserve this error in their source chain.
/// A kind does not by itself authorize retry: the source may have consumed input.
#[derive(Debug)]
#[non_exhaustive]
pub enum StdSourceError {
    /// The requested range violates the source's bounds or buffering contract.
    Source(SourceError),
    /// The original filesystem or reader error.
    Io(std::io::Error),
    /// Retained input storage could not be reserved before reading.
    Allocation {
        /// Total retained payload capacity requested, excluding allocator overhead.
        requested: usize,
        /// The allocator's original refusal.
        source: alloc::collections::TryReserveError,
    },
}

impl fmt::Display for StdSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(f),
            Self::Io(error) => write!(f, "byte source: {error}"),
            Self::Allocation { requested, source } => {
                write!(f, "reserving {requested} retained input bytes: {source}")
            }
        }
    }
}

impl core::error::Error for StdSourceError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Allocation { source, .. } => Some(source),
        }
    }
}

impl From<SourceError> for StdSourceError {
    fn from(error: SourceError) -> Self {
        Self::Source(error)
    }
}

impl From<std::io::Error> for StdSourceError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
