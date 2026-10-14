//! Random access over a sequential stream, by buffering under a policy.

use alloc::string::ToString;
use alloc::vec::Vec;

use zencodec::source::{AccessPattern, ReadAt, SourceError};

/// How a [`BufferedReadAt`] retains bytes from a sequential stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BufferPolicy {
    /// Retain only the most recent `window_bytes`. A read older than the
    /// window fails with [`SourceError::BeyondWindow`]. Right for
    /// [`AccessPattern::Sequential`] and small
    /// [`AccessPattern::BoundedBacktrack`] decoders.
    Window {
        /// Bytes retained behind the read watermark.
        window_bytes: usize,
    },
    /// Retain every byte consumed, up to `max_bytes`; more fails with
    /// [`SourceError::BeyondLimit`]. Right for
    /// [`AccessPattern::RandomAccess`] decoders, with the cap sized to the
    /// largest file the driver is willing to hold in memory.
    Spool {
        /// Maximum retained bytes.
        max_bytes: usize,
    },
}

impl BufferPolicy {
    /// The cheapest policy that cannot fail for `pattern`, under `max_bytes`.
    pub const fn for_pattern(pattern: AccessPattern, max_bytes: usize) -> Self {
        match pattern {
            AccessPattern::Sequential => Self::Window {
                window_bytes: 64 * 1024,
            },
            AccessPattern::BoundedBacktrack { window_bytes } => Self::Window {
                window_bytes: if window_bytes > max_bytes as u64 {
                    max_bytes
                } else {
                    window_bytes as usize
                },
            },
            _ => Self::Spool { max_bytes },
        }
    }

    const fn cap(&self) -> usize {
        match self {
            Self::Window { window_bytes } => *window_bytes,
            Self::Spool { max_bytes } => *max_bytes,
        }
    }
}

/// [`ReadAt`] over a sequential [`std::io::Read`] stream, buffered per a
/// [`BufferPolicy`].
///
/// Buffering manufactures the seeks the stream cannot provide, at an
/// explicit price: a windowed source fails a far-back read, a spool fails
/// past its cap. It never silently grows or returns wrong bytes.
pub struct BufferedReadAt<R: std::io::Read> {
    reader: R,
    policy: BufferPolicy,
    /// Retained bytes; `buf[0]` is at `buf_start`.
    buf: Vec<u8>,
    buf_start: u64,
    /// Bytes consumed from the reader so far.
    watermark: u64,
    eof: bool,
}

impl<R: std::io::Read> BufferedReadAt<R> {
    /// Create a buffered source over `reader` with `policy`.
    pub fn new(reader: R, policy: BufferPolicy) -> Self {
        Self {
            reader,
            policy,
            buf: Vec::new(),
            buf_start: 0,
            watermark: 0,
            eof: false,
        }
    }

    /// Bytes consumed from the underlying reader so far.
    pub fn watermark(&self) -> u64 {
        self.watermark
    }

    /// Consume the stream until `watermark >= end` or EOF. Bytes at or
    /// after `protect_from` survive window eviction during the fill.
    fn fill_to(&mut self, end: u64, protect_from: u64) -> Result<(), SourceError> {
        let mut chunk = [0u8; 16 * 1024];
        while self.watermark < end && !self.eof {
            let want = ((end - self.watermark).min(chunk.len() as u64)) as usize;
            let n = self
                .reader
                .read(&mut chunk[..want.max(1)])
                .map_err(|e| SourceError::Io {
                    message: e.to_string(),
                })?;
            if n == 0 {
                self.eof = true;
                break;
            }
            self.buf.extend_from_slice(&chunk[..n]);
            self.watermark += n as u64;
            self.enforce_policy(protect_from)?;
        }
        Ok(())
    }

    fn enforce_policy(&mut self, protect_from: u64) -> Result<(), SourceError> {
        match self.policy {
            BufferPolicy::Window { window_bytes } => {
                let oldest_allowed = self
                    .watermark
                    .saturating_sub(window_bytes as u64)
                    .min(protect_from);
                if oldest_allowed > self.buf_start {
                    let drop = ((oldest_allowed - self.buf_start) as usize).min(self.buf.len());
                    self.buf.drain(..drop);
                    self.buf_start += drop as u64;
                }
                Ok(())
            }
            BufferPolicy::Spool { max_bytes } => {
                if self.buf.len() > max_bytes {
                    Err(SourceError::BeyondLimit {
                        needed: self.buf.len() as u64,
                        limit: max_bytes as u64,
                    })
                } else {
                    Ok(())
                }
            }
        }
    }
}

impl<R: std::io::Read> ReadAt for BufferedReadAt<R> {
    type Error = SourceError;

    /// Consumes the stream to EOF. Under a window policy the bytes are
    /// evicted as they pass, so the length is known but early bytes are
    /// gone; under a spool they are retained up to the cap.
    fn len(&mut self) -> Result<u64, Self::Error> {
        self.fill_to(u64::MAX, u64::MAX)?;
        Ok(self.watermark)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        if out.is_empty() {
            return Ok(());
        }
        if offset < self.buf_start {
            return Err(SourceError::BeyondWindow {
                offset,
                oldest_retained: self.buf_start,
            });
        }
        let end = offset
            .checked_add(out.len() as u64)
            .ok_or(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: None,
            })?;
        self.fill_to(end, offset)?;
        if end > self.watermark {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: self.eof.then_some(self.watermark),
            });
        }
        let start = (offset - self.buf_start) as usize;
        out.copy_from_slice(&self.buf[start..start + out.len()]);
        Ok(())
    }

    /// Reads forward only as far as `offset + out.len()`: the end of the
    /// stream is found by a short read, never by consuming it to learn the
    /// length.
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Self::Error> {
        if out.is_empty() {
            return Ok(0);
        }
        if offset < self.buf_start {
            return Err(SourceError::BeyondWindow {
                offset,
                oldest_retained: self.buf_start,
            });
        }
        let end = offset.saturating_add(out.len() as u64);
        self.fill_to(end, offset)?;
        if offset >= self.watermark {
            return Ok(0);
        }
        let n = ((self.watermark - offset) as usize).min(out.len());
        let start = (offset - self.buf_start) as usize;
        out[..n].copy_from_slice(&self.buf[start..start + n]);
        Ok(n)
    }

    fn len_hint(&self) -> Option<u64> {
        self.eof.then_some(self.watermark)
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error> {
        // Serve in stream order so the window only moves forward.
        reqs.sort_by_key(|(offset, _)| *offset);
        for (offset, out) in reqs.iter_mut() {
            self.read_exact_at(*offset, out)?;
        }
        Ok(())
    }
}

impl BufferPolicy {
    /// Byte cap of this policy (window size or spool limit).
    pub const fn max_bytes(&self) -> usize {
        self.cap()
    }
}
