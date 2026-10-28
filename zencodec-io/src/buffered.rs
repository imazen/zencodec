//! Random access over a sequential stream, by buffering under a policy.

use alloc::vec::Vec;

use crate::StdSourceError;
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
    /// A compatibility heuristic that may clamp the required lookback.
    /// Use [`Self::try_for_pattern`] when the access pattern must be satisfied.
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

    /// Choose a policy whose retained history satisfies `pattern` within the cap.
    ///
    /// A required backtrack window larger than the cap is refused, never clamped.
    /// A random-access stream can still exceed its spool cap at runtime. This
    /// caps retained payload, not total process memory or caller-owned output.
    pub const fn try_for_pattern(
        pattern: AccessPattern,
        max_bytes: usize,
    ) -> Result<Self, SourceError> {
        match pattern {
            AccessPattern::Sequential => Ok(Self::Window {
                window_bytes: if max_bytes < 64 * 1024 {
                    max_bytes
                } else {
                    64 * 1024
                },
            }),
            AccessPattern::BoundedBacktrack { window_bytes } => {
                if window_bytes > max_bytes as u64 {
                    Err(SourceError::BeyondLimit {
                        needed: window_bytes,
                        limit: max_bytes as u64,
                    })
                } else {
                    Ok(Self::Window {
                        window_bytes: window_bytes as usize,
                    })
                }
            }
            _ => Ok(Self::Spool { max_bytes }),
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
    /// An EOF probe consumed a byte beyond the spool cap. It cannot be replayed.
    exceeded: Option<(u64, u64)>,
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
            exceeded: None,
        }
    }

    /// Bytes consumed from the underlying reader so far.
    pub fn watermark(&self) -> u64 {
        self.watermark
    }

    /// Payload bytes retained for later random/backtrack reads.
    /// Excludes the caller's destination and fixed stack scratch used while skipping.
    pub fn retained_bytes(&self) -> usize {
        self.buf.len()
    }

    fn check_live(&self) -> Result<(), StdSourceError> {
        if let Some((needed, limit)) = self.exceeded {
            return Err(SourceError::BeyondLimit { needed, limit }.into());
        }
        Ok(())
    }

    fn reserve_retained(&mut self, incoming: usize) -> Result<(), StdSourceError> {
        let needed = self
            .buf
            .len()
            .saturating_add(incoming)
            .min(self.policy.cap());
        if needed > self.buf.capacity() {
            self.buf
                .try_reserve_exact(needed - self.buf.len())
                .map_err(|source| StdSourceError::Allocation {
                    requested: needed,
                    source,
                })?;
        }
        Ok(())
    }

    fn remember(&mut self, bytes: &[u8]) {
        match self.policy {
            BufferPolicy::Window { window_bytes } => {
                if bytes.len() >= window_bytes {
                    self.buf.clear();
                    self.buf
                        .extend_from_slice(&bytes[bytes.len() - window_bytes..]);
                } else {
                    let drop = self
                        .buf
                        .len()
                        .saturating_add(bytes.len())
                        .saturating_sub(window_bytes);
                    self.buf.drain(..drop);
                    self.buf.extend_from_slice(bytes);
                }
                self.buf_start = self.watermark - self.buf.len() as u64;
            }
            BufferPolicy::Spool { .. } => self.buf.extend_from_slice(bytes),
        }
    }

    // Read directly into caller storage. Reserve cache capacity before consuming
    // input; a window retains only its tail even for a much larger forward read.
    fn read_more(&mut self, out: &mut [u8]) -> Result<usize, StdSourceError> {
        self.check_live()?;
        if self.eof || out.is_empty() {
            return Ok(0);
        }
        let mut want = out.len();
        if let BufferPolicy::Spool { max_bytes } = self.policy {
            want = want.min(max_bytes - self.buf.len());
            if want == 0 {
                // One fixed stack byte distinguishes exact-cap EOF from excess
                // input. A non-EOF result is terminal and never enters the cache.
                let mut probe = [0];
                let n = loop {
                    match self.reader.read(&mut probe) {
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        result => break result?,
                    }
                };
                if n == 0 {
                    self.eof = true;
                    return Ok(0);
                }
                self.watermark += 1;
                let limit = max_bytes as u64;
                self.exceeded = Some((self.watermark, limit));
                return Err(SourceError::BeyondLimit {
                    needed: self.watermark,
                    limit,
                }
                .into());
            }
        }
        self.reserve_retained(want)?;
        let n = loop {
            match self.reader.read(&mut out[..want]) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => break result?,
            }
        };
        if n == 0 {
            self.eof = true;
        } else {
            self.watermark += n as u64;
            self.remember(&out[..n]);
        }
        Ok(n)
    }

    fn advance_to(&mut self, offset: u64) -> Result<(), StdSourceError> {
        let mut scratch = [0; 16 * 1024];
        while self.watermark < offset && !self.eof {
            let want = (offset - self.watermark).min(scratch.len() as u64) as usize;
            self.read_more(&mut scratch[..want])?;
        }
        Ok(())
    }
}

impl<R: std::io::Read> ReadAt for BufferedReadAt<R> {
    type Error = StdSourceError;

    /// Consumes the stream to EOF. Under a window policy the bytes are
    /// evicted as they pass, so the length is known but early bytes are
    /// gone; under a spool they are retained up to the cap. At the exact spool
    /// cap, a one-byte stack probe distinguishes EOF from excess input. Excess
    /// consumes that one byte and makes subsequent operations fail; retained
    /// payload never exceeds the cap. This is not a total-input-byte limit.
    fn len(&mut self) -> Result<u64, Self::Error> {
        self.check_live()?;
        let mut scratch = [0; 16 * 1024];
        while self.read_more(&mut scratch)? != 0 {}
        Ok(self.watermark)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        if out.is_empty() {
            return Ok(());
        }
        self.check_live()?;
        let end = offset
            .checked_add(out.len() as u64)
            .ok_or(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: None,
            })?;
        if let BufferPolicy::Spool { max_bytes } = self.policy
            && end > max_bytes as u64
        {
            return Err(SourceError::BeyondLimit {
                needed: end,
                limit: max_bytes as u64,
            }
            .into());
        }
        if self.read_at(offset, out)? != out.len() {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: self.eof.then_some(self.watermark),
            }
            .into());
        }
        Ok(())
    }

    /// Reads forward only as far as `offset + out.len()`: the end of the
    /// stream is found by a short read, never by consuming it to learn the
    /// length.
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Self::Error> {
        if out.is_empty() {
            return Ok(0);
        }
        self.check_live()?;
        offset
            .checked_add(out.len() as u64)
            .ok_or(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: self.len_hint(),
            })?;
        if offset < self.buf_start {
            return Err(SourceError::BeyondWindow {
                offset,
                oldest_retained: self.buf_start,
            }
            .into());
        }
        self.advance_to(offset)?;
        if offset > self.watermark {
            return Ok(0);
        }
        let mut filled = ((self.watermark - offset) as usize).min(out.len());
        if filled != 0 {
            let start = (offset - self.buf_start) as usize;
            out[..filled].copy_from_slice(&self.buf[start..start + filled]);
        }
        while filled < out.len() {
            let n = self.read_more(&mut out[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        Ok(filled)
    }

    fn len_hint(&self) -> Option<u64> {
        self.eof.then_some(self.watermark)
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error> {
        // Serve in stream order so the window only moves forward.
        reqs.sort_unstable_by_key(|(offset, _)| *offset);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Read};

    #[test]
    fn checked_policy_never_clamps_required_history() {
        for cap in [0, 1, 16, 65_536] {
            assert!(
                BufferPolicy::try_for_pattern(AccessPattern::Sequential, cap)
                    .unwrap()
                    .cap()
                    <= cap
            );
            for required in [0, 1, 16, 65_536, u64::MAX] {
                let result = BufferPolicy::try_for_pattern(
                    AccessPattern::BoundedBacktrack {
                        window_bytes: required,
                    },
                    cap,
                );
                if required > cap as u64 {
                    assert!(
                        matches!(result, Err(SourceError::BeyondLimit { needed, limit })
                        if needed == required && limit == cap as u64)
                    );
                } else {
                    assert_eq!(
                        result.unwrap(),
                        BufferPolicy::Window {
                            window_bytes: required as usize
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn forward_reads_retain_only_the_window_even_when_output_is_larger() {
        let input: Vec<u8> = (0..101).collect();
        for window in [0, 1, 7, 16, 128] {
            for chunk in [1, 7, 17, 64, 128] {
                let mut source = BufferedReadAt::new(
                    io::Cursor::new(&input),
                    BufferPolicy::Window {
                        window_bytes: window,
                    },
                );
                let mut output = Vec::new();
                let mut buf = alloc::vec![0; chunk];
                loop {
                    let n = source.read_at(output.len() as u64, &mut buf).unwrap();
                    output.extend_from_slice(&buf[..n]);
                    assert!(source.buf.len() <= window);
                    if n == 0 {
                        break;
                    }
                }
                assert_eq!(output, input);
                let retained = window.min(input.len());
                let offset = input.len() - retained;
                let mut back = alloc::vec![0; retained];
                source.read_exact_at(offset as u64, &mut back).unwrap();
                assert_eq!(back, input[offset..]);
                if offset != 0 {
                    assert!(matches!(
                        source.read_at(offset as u64 - 1, &mut [0; 1]),
                        Err(StdSourceError::Source(SourceError::BeyondWindow { .. }))
                    ));
                }
            }
        }
    }

    #[test]
    fn allocation_refusal_precedes_input_consumption() {
        let mut source = BufferedReadAt::new(
            io::repeat(0),
            BufferPolicy::Spool {
                max_bytes: usize::MAX,
            },
        );
        assert!(matches!(
            source.reserve_retained(usize::MAX),
            Err(StdSourceError::Allocation { .. })
        ));
        assert_eq!(source.watermark(), 0);
        assert!(source.buf.is_empty());
    }

    struct InterruptedOnce(bool);
    impl Read for InterruptedOnce {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if !self.0 {
                self.0 = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            out.fill(7);
            Ok(out.len())
        }
    }

    #[test]
    fn interrupted_reads_do_not_lose_data_or_progress() {
        let mut source = BufferedReadAt::new(
            InterruptedOnce(false),
            BufferPolicy::Window { window_bytes: 2 },
        );
        let mut out = [0; 8];
        source.read_exact_at(0, &mut out).unwrap();
        assert_eq!(out, [7; 8]);
        assert_eq!(source.watermark(), 8);
        assert_eq!(source.retained_bytes(), 2);
    }
}
