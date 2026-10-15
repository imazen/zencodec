//! The encoder side of streaming output: bytes produced, handed out, held.

use alloc::collections::TryReserveError;
use alloc::vec::Vec;

/// Bookkeeping for a streaming encoder's output.
///
/// An encoder that streams appends bytes as it produces them, and a driver
/// takes them with [`Encoder::pending_output`](crate::encode::Encoder::pending_output)
/// and [`consume_output`](crate::encode::Encoder::consume_output), or with
/// [`take_output`](crate::encode::Encoder::take_output). This type is that
/// buffer, so an encoder doesn't write it again: it keeps the consumed
/// prefix cheap to drop, counts every byte handed out for
/// `max_output_bytes`, clamps an over-large consume, and hands the whole
/// buffer to a caller without a copy when it can.
///
/// ```
/// use zencodec::encode::OutputBuffer;
///
/// let mut out = OutputBuffer::new();
/// out.push(b"header");
/// assert_eq!(out.pending(), b"header");
/// out.consume(3);
/// assert_eq!(out.pending(), b"der");
/// out.push(b" rows");
/// let mut sent = Vec::new();
/// out.take_into(&mut sent);
/// assert_eq!(sent, b"der rows");
/// assert_eq!(out.total(), 11);
/// assert_eq!(out.into_remaining(), b"");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputBuffer {
    buf: Vec<u8>,
    /// Bytes at the front of `buf` already handed out.
    start: usize,
    /// Bytes handed out and dropped from `buf`.
    dropped: u64,
}

/// Drop the consumed prefix once it is at least this large and at least
/// half the buffer: moving the tail then costs no more than the bytes
/// already sent.
const COMPACT_AT: usize = 64 * 1024;

impl OutputBuffer {
    /// An empty buffer.
    pub const fn new() -> Self {
        Self {
            buf: Vec::new(),
            start: 0,
            dropped: 0,
        }
    }

    /// An empty buffer with room for `capacity` bytes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
            ..Self::new()
        }
    }

    /// Reserve room for `additional` more bytes, failing instead of
    /// aborting when memory runs out.
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), TryReserveError> {
        self.buf.try_reserve(additional)
    }

    /// Append encoded bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The underlying vector, for writers that append to a `Vec<u8>`
    /// directly (a bit writer, a chunk writer that patches a length it
    /// wrote earlier).
    ///
    /// Append to it, or change bytes still pending. Bytes before
    /// [`pending`](Self::pending) have been handed out; changing or removing
    /// them corrupts the output (a removal shows up as missing pending bytes,
    /// never as a panic).
    pub fn vec_mut(&mut self) -> &mut Vec<u8> {
        &mut self.buf
    }

    /// Bytes produced and not yet handed out.
    pub fn pending(&self) -> &[u8] {
        self.buf.get(self.start..).unwrap_or(&[])
    }

    /// Number of pending bytes.
    pub fn len(&self) -> usize {
        self.pending().len()
    }

    /// Whether nothing is pending.
    pub fn is_empty(&self) -> bool {
        self.pending().is_empty()
    }

    /// Mark the first `n` pending bytes as handed out. An `n` larger than
    /// the pending length marks all of it.
    pub fn consume(&mut self, n: usize) {
        let n = n.min(self.len());
        self.start += n;
        if self.start >= self.buf.len() {
            self.dropped += self.buf.len() as u64;
            self.buf.clear();
            self.start = 0;
        } else if self.start >= COMPACT_AT && self.start >= self.buf.len() / 2 {
            self.dropped += self.start as u64;
            self.buf.drain(..self.start);
            self.start = 0;
        }
    }

    /// Append every pending byte to `into` and mark it handed out.
    ///
    /// When `into` is empty and nothing has been consumed since the last
    /// take, the two vectors are swapped instead: `into` receives this
    /// buffer's allocation, with no copy, and this buffer keeps `into`'s.
    /// A driver that hands each batch onward (a `Bytes` body chunk, a pipe
    /// segment) passes a fresh vector each time and never copies.
    pub fn take_into(&mut self, into: &mut Vec<u8>) {
        if into.is_empty() && self.start == 0 {
            self.dropped += self.buf.len() as u64;
            core::mem::swap(into, &mut self.buf);
            self.buf.clear();
        } else {
            into.extend_from_slice(self.pending());
            let n = self.len();
            self.consume(n);
        }
    }

    /// Bytes handed out so far, through [`consume`](Self::consume) or
    /// [`take_into`](Self::take_into).
    pub fn handed_out(&self) -> u64 {
        self.dropped + self.start as u64
    }

    /// Every byte produced so far, handed out or pending: what
    /// `max_output_bytes` bounds.
    pub fn total(&self) -> u64 {
        self.dropped + self.buf.len() as u64
    }

    /// The bytes not handed out: what `finish` returns.
    pub fn into_remaining(mut self) -> Vec<u8> {
        let start = self.start.min(self.buf.len());
        self.buf.drain(..start);
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn consume_clamps_and_counts() {
        let mut out = OutputBuffer::new();
        out.push(&[1, 2, 3]);
        out.consume(10);
        assert!(out.is_empty());
        assert_eq!(out.handed_out(), 3);
        assert_eq!(out.total(), 3);
        out.push(&[4]);
        assert_eq!(out.pending(), &[4]);
        assert_eq!(out.total(), 4);
    }

    #[test]
    fn take_into_swaps_when_it_can() {
        let mut out = OutputBuffer::with_capacity(16);
        out.push(&[1, 2, 3]);
        let ptr = out.pending().as_ptr();
        let mut into = Vec::new();
        out.take_into(&mut into);
        assert_eq!(into, vec![1, 2, 3]);
        assert_eq!(
            into.as_ptr(),
            ptr,
            "an empty destination takes the allocation"
        );
        assert_eq!(out.handed_out(), 3);
        assert!(out.is_empty());
    }

    #[test]
    fn take_into_appends_after_a_partial_consume() {
        let mut out = OutputBuffer::new();
        out.push(&[1, 2, 3, 4]);
        out.consume(1);
        let mut into = vec![9];
        out.take_into(&mut into);
        assert_eq!(into, vec![9, 2, 3, 4]);
        assert_eq!(out.handed_out(), 4);
        assert_eq!(out.total(), 4);
    }

    #[test]
    fn compaction_keeps_the_stream() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 + i / 251) as u8).collect();
        let mut out = OutputBuffer::new();
        let mut got = Vec::new();
        for chunk in data.chunks(10_007) {
            out.push(chunk);
            let n = out.len() * 2 / 3;
            got.extend_from_slice(&out.pending()[..n]);
            out.consume(n);
        }
        assert_eq!(out.total(), data.len() as u64);
        got.extend_from_slice(&out.into_remaining());
        assert_eq!(got, data);
    }

    #[test]
    fn a_removed_prefix_reads_as_empty() {
        let mut out = OutputBuffer::new();
        out.push(&[1, 2, 3]);
        out.consume(2);
        out.vec_mut().truncate(1);
        assert!(out.pending().is_empty());
        out.consume(1);
        assert!(out.into_remaining().is_empty());
    }
}
