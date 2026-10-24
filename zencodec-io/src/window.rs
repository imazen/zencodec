//! The driver-owned input buffer and the protocol checks every driver
//! shares.

use alloc::vec::Vec;
use core::fmt;

use zencodec::decode::{Input, Request};

/// Bytes the driver holds: `buf[head..end]` starts at absolute `base`.
///
/// `buf` is zeroed once as it grows and then reused: reads go straight into
/// `buf[end..]`, so no read pays for a memset, and live bytes move to the
/// front only when the tail has no room for the next read.
#[derive(Debug, Default)]
pub(crate) struct Window {
    buf: Vec<u8>,
    head: usize,
    end: usize,
    base: u64,
    /// Total source length, once known.
    pub(crate) total: Option<u64>,
}

impl Window {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn base(&self) -> u64 {
        self.base
    }

    pub(crate) fn len(&self) -> usize {
        self.end - self.head
    }

    pub(crate) fn end(&self) -> u64 {
        self.base + self.len() as u64
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.buf[self.head..self.end]
    }

    pub(crate) fn input(&self) -> Input<'_> {
        Input::new(self.base, self.bytes()).with_total_len_opt(self.total)
    }

    /// Position at `offset`, keeping the longest held prefix there. A fetched
    /// range can become the sequential window without copying its allocation.
    /// Returns whether any bytes were retained; it never fills a hole.
    pub(crate) fn position(&mut self, offset: u64, fetched: &mut Vec<(u64, Vec<u8>)>) -> bool {
        let active = offset >= self.base && offset < self.end();
        let active_end = if active { self.end() } else { offset };
        if let Some((i, _)) = fetched
            .iter()
            .enumerate()
            .filter(|(_, (start, bytes))| {
                offset >= *start && offset < start.saturating_add(bytes.len() as u64)
            })
            .max_by_key(|(_, (start, bytes))| start.saturating_add(bytes.len() as u64))
            .filter(|(_, (start, bytes))| start.saturating_add(bytes.len() as u64) > active_end)
        {
            let (start, bytes) = fetched.swap_remove(i);
            self.adopt(start, bytes);
            self.consume((offset - start) as usize);
            return true;
        }
        if active {
            self.consume((offset - self.base) as usize);
            true
        } else {
            self.reset_to(offset);
            false
        }
    }

    /// Drop `n` bytes from the front (the decoder took them).
    pub(crate) fn consume(&mut self, n: usize) {
        let n = n.min(self.len());
        self.head += n;
        self.base += n as u64;
        if self.head == self.end {
            self.head = 0;
            self.end = 0;
        }
    }

    /// Forget everything and start at `offset`.
    pub(crate) fn reset_to(&mut self, offset: u64) {
        self.head = 0;
        self.end = 0;
        self.base = offset;
    }

    /// Room for `n` more bytes after the held bytes, to read into; follow
    /// with [`commit`](Self::commit). The contents are stale, not zeroed.
    pub(crate) fn spare(&mut self, n: usize) -> &mut [u8] {
        if self.buf.len() - self.end < n && self.head > 0 {
            self.buf.copy_within(self.head..self.end, 0);
            self.end -= self.head;
            self.head = 0;
        }
        if self.buf.len() - self.end < n {
            // Grow geometrically so a stream of reads zeroes O(total) bytes.
            let want = (self.end + n).max(self.buf.len().saturating_mul(2));
            self.buf.resize(want, 0);
        }
        &mut self.buf[self.end..self.end + n]
    }

    /// Keep `n` of the bytes last handed out by [`spare`](Self::spare).
    pub(crate) fn commit(&mut self, n: usize) {
        self.end += n;
    }

    /// Take ownership of `buf`, holding bytes from `offset`. The old
    /// contents are dropped; no bytes are copied.
    #[cfg_attr(not(feature = "tokio"), allow(dead_code))]
    pub(crate) fn adopt(&mut self, offset: u64, buf: Vec<u8>) {
        self.end = buf.len();
        self.buf = buf;
        self.head = 0;
        self.base = offset;
    }

    /// Append bytes received from elsewhere.
    #[allow(dead_code)]
    pub(crate) fn extend(&mut self, bytes: &[u8]) {
        self.spare(bytes.len()).copy_from_slice(bytes);
        self.commit(bytes.len());
    }
}

/// How many bytes to hold before stepping a decoder that asked for
/// `min_len` contiguous bytes: `min_len` rounded up to a whole multiple of
/// itself no smaller than `read_size`.
///
/// A decoder that consumes `min_len` per step (a strip, a row) then leaves
/// nothing behind, so the window empties and restarts at its front instead
/// of moving a leftover tail on every read; one stray header in front of
/// the strips would otherwise misalign every read for the whole image.
/// Decoders that ask for little (`min_len` of 1) still get `read_size`.
pub(crate) fn read_target(min_len: usize, read_size: usize) -> usize {
    let unit = min_len.max(1);
    let read_size = read_size.max(unit);
    read_size.div_ceil(unit).saturating_mul(unit)
}

/// A decoder broke the [`IncrementalDecode`](zencodec::decode::IncrementalDecode)
/// protocol. Drivers report this instead of looping.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolViolation {
    /// `Need` with no requests.
    EmptyRequest,
    /// The input satisfied one of the decoder's requests, yet it took
    /// nothing and asked again.
    NoProgress(Request),
    /// The decoder asked for bytes past the end it was told about, instead
    /// of reporting truncated input.
    NeedPastEnd {
        /// The request.
        request: Request,
        /// The total length the decoder had been given.
        total: u64,
    },
}

impl fmt::Display for ProtocolViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRequest => write!(f, "decoder returned Need with no requests"),
            Self::NoProgress(r) => write!(
                f,
                "decoder took nothing from an input that satisfied its request {r:?}"
            ),
            Self::NeedPastEnd { request, total } => write!(
                f,
                "decoder requested {request:?} past the end of a {total}-byte source"
            ),
        }
    }
}

impl core::error::Error for ProtocolViolation {}

/// Check a `Need` against the input that was just offered: requests must
/// exist, must not ask past a known end, and an input that satisfied one
/// must have been consumed from.
pub(crate) fn check_need(
    requests: &[Request],
    offered: &Input<'_>,
    consumed: usize,
) -> Result<(), ProtocolViolation> {
    if requests.is_empty() {
        return Err(ProtocolViolation::EmptyRequest);
    }
    if let Some(total) = offered.total_len {
        for r in requests {
            let past = match *r {
                Request::AtLeast { offset, .. } => offset > total,
                Request::Range(range) => range.end() > total,
                _ => false,
            };
            if past {
                return Err(ProtocolViolation::NeedPastEnd { request: *r, total });
            }
        }
    }
    if consumed == 0
        && let Some(r) = requests.iter().find(|r| r.is_satisfied_by(offered))
    {
        return Err(ProtocolViolation::NoProgress(*r));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_consumes_by_head_and_reads_into_spare() {
        let mut w = Window::new();
        w.extend(&[1, 2, 3, 4]);
        w.consume(1);
        assert_eq!((w.base(), w.bytes()), (1, &[2u8, 3, 4][..]));
        let s = w.spare(4);
        s[..2].copy_from_slice(&[5, 6]);
        w.commit(2);
        assert_eq!(w.bytes(), &[2, 3, 4, 5, 6]);
        w.consume(5);
        assert_eq!((w.base(), w.len()), (6, 0));
        w.reset_to(100);
        assert_eq!((w.base(), w.end()), (100, 100));
    }

    #[test]
    fn read_target_rounds_to_whole_requests() {
        assert_eq!(read_target(1, 65536), 65536);
        assert_eq!(read_target(0, 4096), 4096);
        assert_eq!(read_target(65536, 65536), 65536);
        assert_eq!(read_target(20_000, 65536), 80_000);
        assert_eq!(read_target(100_000, 65536), 100_000);
    }

    #[test]
    fn need_checks_catch_spins_and_overreach() {
        let bytes = [0u8; 8];
        let full = Input::new(0, &bytes).with_total_len(8);
        assert_eq!(
            check_need(&[], &full, 0),
            Err(ProtocolViolation::EmptyRequest)
        );
        let r = Request::at_least(0, 4);
        assert_eq!(
            check_need(&[r], &full, 0),
            Err(ProtocolViolation::NoProgress(r))
        );
        assert_eq!(check_need(&[r], &full, 1), Ok(()));
        let past = Request::range(6, 4);
        assert!(matches!(
            check_need(&[past], &full, 0),
            Err(ProtocolViolation::NeedPastEnd { .. })
        ));
        let short = Input::new(0, &bytes[..2]);
        assert_eq!(check_need(&[r], &short, 0), Ok(()));
    }
}
