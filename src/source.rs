//! Random-access byte sources for pull-based decoding.
//!
//! [`ReadAt`] is the source a pull-native decoder reads from in straight-line
//! code: a file (`pread`), an object store (HTTP Range), a host callback
//! across FFI, or a slice. The codec reads exactly the ranges it needs, at
//! offsets it chooses, and never learns which transport served them.
//!
//! A decoder takes a source through
//! [`DecodeJob::push_decoder_from_source`](crate::decode::DecodeJob::push_decoder_from_source).
//! The object-safe form every job receives is [`DynReadAt`]; a concrete
//! [`ReadAt`] becomes one with [`ReadAtShim`] or `Box::<dyn DynReadAt>::from`.
//!
//! Drivers that hold a sequential stream, or run on an async runtime,
//! manufacture a source with the adapters in `zencodec-io`; the contract
//! here is deliberately the minimum a codec needs.

use alloc::boxed::Box;
use core::fmt;

use crate::traits::BoxedError;

/// How a decoder reads its input, declared so drivers can size a buffer or
/// choose a fetch strategy for unseekable streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum AccessPattern {
    /// Reads move forward; any backtrack is incidental and tiny.
    #[default]
    Sequential,
    /// Reads may revisit at most `window_bytes` behind the furthest read.
    BoundedBacktrack {
        /// Maximum bytes behind the read watermark a read may target.
        window_bytes: u64,
    },
    /// Reads may target any offset at any time (directories, tile tables,
    /// trailing indexes).
    RandomAccess,
}

impl AccessPattern {
    /// The more demanding of two patterns (`RandomAccess` over
    /// `BoundedBacktrack` over `Sequential`; the larger window between two
    /// bounded ones).
    pub const fn most_demanding(self, other: Self) -> Self {
        match (self, other) {
            (Self::RandomAccess, _) | (_, Self::RandomAccess) => Self::RandomAccess,
            (
                Self::BoundedBacktrack { window_bytes: a },
                Self::BoundedBacktrack { window_bytes: b },
            ) => Self::BoundedBacktrack {
                window_bytes: if a > b { a } else { b },
            },
            (Self::BoundedBacktrack { window_bytes }, _)
            | (_, Self::BoundedBacktrack { window_bytes }) => {
                Self::BoundedBacktrack { window_bytes }
            }
            _ => Self::Sequential,
        }
    }
}

/// Failure modes of a byte source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceError {
    /// A read targeted bytes already evicted from a windowed buffer.
    BeyondWindow {
        /// Offset the codec asked for.
        offset: u64,
        /// Oldest offset still retained.
        oldest_retained: u64,
    },
    /// A buffering policy's byte cap would be exceeded.
    BeyondLimit {
        /// Bytes the operation would have retained.
        needed: u64,
        /// Configured cap.
        limit: u64,
    },
    /// A read ran past the end of the source.
    UnexpectedEof {
        /// Offset the codec asked for.
        offset: u64,
        /// Bytes requested.
        len: u64,
        /// Total source length, if known.
        total: Option<u64>,
    },
    /// The underlying transport or filesystem failed.
    Io {
        /// Driver-supplied description of the failure.
        message: alloc::string::String,
    },
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeyondWindow {
                offset,
                oldest_retained,
            } => write!(
                f,
                "source read at {offset} is behind the buffered window (oldest retained: {oldest_retained})"
            ),
            Self::BeyondLimit { needed, limit } => write!(
                f,
                "source buffering would retain {needed} bytes, over the {limit}-byte limit"
            ),
            Self::UnexpectedEof { offset, len, total } => write!(
                f,
                "source read of {len} bytes at {offset} ran past end of source (total: {total:?})"
            ),
            Self::Io { message } => write!(f, "source I/O error: {message}"),
        }
    }
}

impl core::error::Error for SourceError {}

/// Random-access byte source. Modelled on imij's `ReadAt`, so an imij-style
/// codec bridges to this trait by renaming.
///
/// `read_many_at` exists so a source that can serve ranges concurrently
/// (HTTP Range, object stores, parallel `pread`) overlaps their round
/// trips. Codecs pass every range they can ask for at once.
///
/// One decoder operation must observe one coherent byte representation. Bytes
/// already supplied and any known final length must remain consistent across
/// reads. A source over a growing stream may discover more bytes while keeping
/// the supplied prefix unchanged. Replacing an object requires a new operation;
/// it cannot silently change the meaning of an existing offset or image ID.
///
/// Establish this in the source/driver: immutable owned bytes, an immutable
/// object version, conditional reads that fail on replacement, or a stable-file
/// ownership/locking contract. Positional filesystem I/O alone is not snapshot
/// isolation. Transport validators stay outside this trait; source-specific
/// errors retain the cause when that contract cannot be maintained.
pub trait ReadAt {
    /// The source-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Total length in bytes. May be expensive: a buffered stream source
    /// consumes to the end of the stream to answer it. Callers that only
    /// read forward use [`read_at`](Self::read_at), which reports the end
    /// by returning fewer bytes, and never need this.
    fn len(&mut self) -> Result<u64, Self::Error>;

    /// Total length if it is known without reading ahead. Default `None`.
    fn len_hint(&self) -> Option<u64> {
        None
    }

    /// Whether the source is empty.
    fn is_empty(&mut self) -> Result<bool, Self::Error> {
        Ok(self.len()? == 0)
    }

    /// Fill `out` with the bytes at `offset`. Fails rather than returning
    /// a short read.
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error>;

    /// Read up to `out.len()` bytes at `offset`; returns the count, which is
    /// smaller only where the source ends (zero at or past the end).
    ///
    /// The default asks [`len`](Self::len) to bound the read; sources over
    /// streams override it to read forward without consuming to the end.
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Self::Error> {
        let len = self.len()?;
        let n = len.saturating_sub(offset).min(out.len() as u64) as usize;
        self.read_exact_at(offset, &mut out[..n])?;
        Ok(n)
    }

    /// Fill each buffer from its offset. The default serves requests in
    /// the order given; concurrent sources override to overlap them.
    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error> {
        for (offset, out) in reqs.iter_mut() {
            self.read_exact_at(*offset, out)?;
        }
        Ok(())
    }

    /// The whole source as one slice, when it is already in memory. Drivers
    /// and codecs use it to read without copying. Default `None`.
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }
}

/// [`ReadAt`] over an in-memory slice: the zero-copy source for tests,
/// embedded data and already-buffered input.
#[derive(Debug, Clone)]
pub struct SliceReadAt<'a> {
    bytes: &'a [u8],
}

impl<'a> SliceReadAt<'a> {
    /// Create a source over `bytes`.
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn eof(&self, offset: u64, len: usize) -> SourceError {
        SourceError::UnexpectedEof {
            offset,
            len: len as u64,
            total: Some(self.bytes.len() as u64),
        }
    }
}

impl ReadAt for SliceReadAt<'_> {
    type Error = SourceError;

    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(self.bytes.len() as u64)
    }

    fn len_hint(&self) -> Option<u64> {
        Some(self.bytes.len() as u64)
    }

    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Self::Error> {
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(self.bytes.len());
        let n = (self.bytes.len() - start).min(out.len());
        out[..n].copy_from_slice(&self.bytes[start..start + n]);
        Ok(n)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(self.bytes)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        let start = usize::try_from(offset).map_err(|_| self.eof(offset, out.len()))?;
        let end = start
            .checked_add(out.len())
            .ok_or_else(|| self.eof(offset, out.len()))?;
        let data = self
            .bytes
            .get(start..end)
            .ok_or_else(|| self.eof(offset, out.len()))?;
        out.copy_from_slice(data);
        Ok(())
    }
}

/// Object-safe [`ReadAt`]. Jobs receive this; codecs wrap its boxed errors
/// with their own source-error constructor.
pub trait DynReadAt: Send {
    /// Total length in bytes (may be expensive; see [`ReadAt::len`]).
    fn len(&mut self) -> Result<u64, BoxedError>;
    /// Total length if known without reading ahead.
    fn len_hint(&self) -> Option<u64> {
        None
    }
    /// Whether the source holds no bytes.
    fn is_empty(&mut self) -> Result<bool, BoxedError> {
        Ok(self.len()? == 0)
    }
    /// Fill `out` with the bytes at `offset`.
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BoxedError>;
    /// Read up to `out.len()` bytes at `offset`; see [`ReadAt::read_at`].
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
        let len = self.len()?;
        let n = len.saturating_sub(offset).min(out.len() as u64) as usize;
        self.read_exact_at(offset, &mut out[..n])?;
        Ok(n)
    }
    /// Fill each buffer from its offset, concurrently where possible.
    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
        for (offset, out) in reqs.iter_mut() {
            self.read_exact_at(*offset, out)?;
        }
        Ok(())
    }
    /// The whole source as one slice, when already in memory.
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }
}

impl<S: DynReadAt + ?Sized> DynReadAt for Box<S> {
    fn len(&mut self) -> Result<u64, BoxedError> {
        (**self).len()
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BoxedError> {
        (**self).read_exact_at(offset, out)
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
        (**self).read_many_at(reqs)
    }

    fn len_hint(&self) -> Option<u64> {
        (**self).len_hint()
    }

    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
        (**self).read_at(offset, out)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        (**self).as_slice()
    }
}

impl<S: DynReadAt + ?Sized> DynReadAt for &mut S {
    fn len(&mut self) -> Result<u64, BoxedError> {
        (**self).len()
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BoxedError> {
        (**self).read_exact_at(offset, out)
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
        (**self).read_many_at(reqs)
    }

    fn len_hint(&self) -> Option<u64> {
        (**self).len_hint()
    }

    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
        (**self).read_at(offset, out)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        (**self).as_slice()
    }
}

/// Shim presenting a concrete [`ReadAt`] as a [`DynReadAt`].
pub struct ReadAtShim<S>(pub S);

impl<S: ReadAt + Send> DynReadAt for ReadAtShim<S> {
    fn len(&mut self) -> Result<u64, BoxedError> {
        self.0.len().map_err(|e| Box::new(e) as BoxedError)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BoxedError> {
        self.0
            .read_exact_at(offset, out)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
        self.0
            .read_many_at(reqs)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn len_hint(&self) -> Option<u64> {
        self.0.len_hint()
    }

    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
        self.0
            .read_at(offset, out)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        self.0.as_slice()
    }
}

impl<S: ReadAt + Send + 'static> From<S> for Box<dyn DynReadAt> {
    fn from(source: S) -> Self {
        Box::new(ReadAtShim(source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_reads_and_fails_past_end() {
        let data = [1u8, 2, 3, 4, 5];
        let mut src = SliceReadAt::new(&data);
        assert_eq!(src.len().unwrap(), 5);
        let mut two = [0u8; 2];
        src.read_exact_at(3, &mut two).unwrap();
        assert_eq!(two, [4, 5]);
        let mut three = [0u8; 3];
        assert!(matches!(
            src.read_exact_at(3, &mut three),
            Err(SourceError::UnexpectedEof {
                offset: 3,
                len: 3,
                total: Some(5)
            })
        ));
        assert!(src.read_exact_at(u64::MAX, &mut two).is_err());
    }

    #[test]
    fn shim_and_box_dispatch() {
        static DATA: [u8; 3] = [9, 8, 7];
        let mut boxed: Box<dyn DynReadAt> = SliceReadAt::new(&DATA).into();
        let mut a = [0u8; 1];
        let mut b = [0u8; 2];
        boxed
            .read_many_at(&mut [(2, &mut a[..]), (0, &mut b[..])])
            .unwrap();
        assert_eq!((a, b), ([7], [9, 8]));
        let by_ref: &mut dyn DynReadAt = &mut *boxed;
        assert_eq!(by_ref.len().unwrap(), 3);
    }
}
