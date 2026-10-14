//! The driver-owned input buffer and the protocol checks every driver
//! shares.

use alloc::vec::Vec;
use core::fmt;

use zencodec::decode::{Input, Request};

/// Bytes the driver holds: `buf[head..]` starts at absolute `base`.
#[derive(Debug, Default)]
pub(crate) struct Window {
    buf: Vec<u8>,
    head: usize,
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
        self.buf.len() - self.head
    }

    pub(crate) fn end(&self) -> u64 {
        self.base + self.len() as u64
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.buf[self.head..]
    }

    pub(crate) fn input(&self) -> Input<'_> {
        Input::new(self.base, self.bytes()).with_total_len_opt(self.total)
    }

    /// Drop `n` bytes from the front (the decoder took them).
    pub(crate) fn consume(&mut self, n: usize) {
        let n = n.min(self.len());
        self.head += n;
        self.base += n as u64;
        if self.head == self.buf.len() {
            self.buf.clear();
            self.head = 0;
        } else if self.head > self.buf.len() / 2 && self.head >= 64 * 1024 {
            self.buf.drain(..self.head);
            self.head = 0;
        }
    }

    /// Forget everything and start at `offset`.
    pub(crate) fn reset_to(&mut self, offset: u64) {
        self.buf.clear();
        self.head = 0;
        self.base = offset;
    }

    /// Spare capacity of `n` zeroed bytes after the held bytes, to read into;
    /// follow with [`commit`](Self::commit).
    pub(crate) fn spare(&mut self, n: usize) -> &mut [u8] {
        if self.head > 0 && self.head >= self.buf.len() / 2 {
            self.buf.drain(..self.head);
            self.head = 0;
        }
        let start = self.buf.len();
        self.buf.resize(start + n, 0);
        &mut self.buf[start..]
    }

    /// Keep `n` of the bytes last handed out by [`spare`](Self::spare).
    pub(crate) fn commit(&mut self, spare: usize, n: usize) {
        let keep = self.buf.len() - spare + n;
        self.buf.truncate(keep);
    }

    /// Take ownership of `buf`, holding bytes from `offset`. The old
    /// contents are dropped; no bytes are copied.
    #[cfg_attr(not(feature = "tokio"), allow(dead_code))]
    pub(crate) fn adopt(&mut self, offset: u64, buf: Vec<u8>) {
        self.buf = buf;
        self.head = 0;
        self.base = offset;
    }

    /// Append bytes received from elsewhere.
    #[allow(dead_code)]
    pub(crate) fn extend(&mut self, bytes: &[u8]) {
        self.spare(bytes.len()).copy_from_slice(bytes);
    }
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
        w.commit(4, 2);
        assert_eq!(w.bytes(), &[2, 3, 4, 5, 6]);
        w.consume(5);
        assert_eq!((w.base(), w.len()), (6, 0));
        w.reset_to(100);
        assert_eq!((w.base(), w.end()), (100, 100));
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
