//! Bytes in and bytes out, with no I/O.
//!
//! A decoder names the bytes it wants ([`InputRequest`]) and is handed a
//! window ([`Input`]) it borrows for one step. An encoder writes into an
//! [`OutputBuffer`] it owns; the driver drains it. Neither side ever blocks,
//! awaits or calls back, which is what lets one contract serve a slice, a
//! file, an async socket and a foreign host.

use crate::error::{CodecError, ErrorKind, LimitKind, Result, error, error_msg};
use crate::limits::LimitExceeded;
use alloc::vec::Vec;

/// A span of the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ByteRange {
    /// First byte.
    pub offset: u64,
    /// Length in bytes.
    pub length: u64,
}

impl ByteRange {
    /// A span.
    pub const fn new(offset: u64, length: u64) -> Self {
        Self { offset, length }
    }

    /// One past the last byte, saturating.
    pub const fn end(&self) -> u64 {
        self.offset.saturating_add(self.length)
    }
}

/// What a decoder needs before it can continue.
///
/// A decoder may post several at once. The first must be met before the next
/// step; the rest are a forecast, so a driver with a slow source can fetch
/// them together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputRequest {
    /// Bytes from `offset`, at least `min_len` of them unless the input ends
    /// first. More is welcome.
    AtLeast {
        /// First byte wanted.
        offset: u64,
        /// Fewest bytes that allow progress.
        min_len: usize,
    },
    /// Exactly this span.
    Range(ByteRange),
    /// The complete input, contiguous, from offset zero.
    All,
    /// The total length, in [`Input::total_len`].
    Len,
}

impl InputRequest {
    /// Whether `input` meets this request.
    pub fn is_met_by(&self, input: &Input<'_>) -> bool {
        match *self {
            Self::AtLeast { offset, min_len } => {
                input.offset == offset && (input.bytes.len() >= min_len || input.reaches_end())
            }
            Self::Range(r) => {
                input.offset <= r.offset
                    && (input.end() >= r.end()
                        || (input.reaches_end() && input.total_len.is_some_and(|t| t <= r.end())))
            }
            Self::All => input.offset == 0 && input.reaches_end(),
            Self::Len => input.total_len.is_some(),
        }
    }
}

/// A window of the input, lent for one step.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    /// Offset of `bytes[0]` in the whole input.
    pub offset: u64,
    /// The bytes.
    pub bytes: &'a [u8],
    /// Total input length, once the driver knows it.
    pub total_len: Option<u64>,
}

impl<'a> Input<'a> {
    /// A window at `offset`.
    pub const fn new(offset: u64, bytes: &'a [u8]) -> Self {
        Self {
            offset,
            bytes,
            total_len: None,
        }
    }

    /// The whole input, in one piece.
    pub const fn whole(bytes: &'a [u8]) -> Self {
        Self {
            offset: 0,
            bytes,
            total_len: Some(bytes.len() as u64),
        }
    }

    /// State the total length.
    pub const fn with_total_len(mut self, total_len: Option<u64>) -> Self {
        self.total_len = total_len;
        self
    }

    /// Offset one past the window.
    pub const fn end(&self) -> u64 {
        self.offset + self.bytes.len() as u64
    }

    /// Whether the window runs to the end of the input.
    pub fn reaches_end(&self) -> bool {
        self.total_len.is_some_and(|total| self.end() >= total)
    }

    /// The window with its first `n` bytes removed.
    pub fn advanced(&self, n: usize) -> Self {
        Self {
            offset: self.offset + n as u64,
            bytes: &self.bytes[n..],
            total_len: self.total_len,
        }
    }
}

/// Whether the destination can revisit bytes it has already taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputAccess {
    /// Bytes leave in order and are gone.
    #[default]
    Forward,
    /// The destination can overwrite earlier bytes.
    Patchable,
}

/// An overwrite of bytes the destination already holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    /// Absolute offset in the encoded output.
    pub offset: u64,
    /// Replacement bytes.
    pub bytes: Vec<u8>,
}

/// An encoder's output: bytes not yet taken, and patches to bytes that were.
///
/// The encoder owns it and writes to it; the driver takes from it between
/// steps. Taking moves the allocation out, so a driver can write a chunk to
/// a socket while the encoder fills the next one, then hand the emptied
/// vector back with [`recycle`](Self::recycle).
#[derive(Debug, Default)]
pub struct OutputBuffer {
    bytes: Vec<u8>,
    /// Absolute offset of `bytes[0]`.
    base: u64,
    held: bool,
    access: OutputAccess,
    limit: Option<u64>,
    patches: Vec<Patch>,
}

impl OutputBuffer {
    /// An empty buffer for a destination with this access and size gate.
    pub const fn new(access: OutputAccess, max_output_bytes: Option<u64>) -> Self {
        Self {
            bytes: Vec::new(),
            base: 0,
            held: false,
            access,
            limit: max_output_bytes,
            patches: Vec::new(),
        }
    }

    // ── encoder side ──

    /// Absolute offset of the next byte written.
    pub fn position(&self) -> u64 {
        self.base + self.bytes.len() as u64
    }

    /// Keep bytes back until [`release`](Self::release). An encoder that must
    /// revisit its output, on a destination that cannot be patched, holds.
    pub fn hold(&mut self) {
        self.held = true;
    }

    /// Let held bytes go.
    pub fn release(&mut self) {
        self.held = false;
    }

    /// Append bytes.
    #[track_caller]
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.admit(data.len())?;
        if self.bytes.try_reserve(data.len()).is_err() {
            return Err(error(ErrorKind::OutOfMemory));
        }
        self.bytes.extend_from_slice(data);
        Ok(())
    }

    /// Append an owned buffer, adopting its allocation when nothing is pending.
    #[track_caller]
    pub fn write_owned(&mut self, data: Vec<u8>) -> Result<()> {
        if self.bytes.is_empty() {
            self.admit(data.len())?;
            self.bytes = data;
            Ok(())
        } else {
            self.write(&data)
        }
    }

    /// Overwrite bytes already written, at an absolute offset.
    ///
    /// Bytes still here are changed in place. Bytes already taken become a
    /// [`Patch`] when the destination is [`Patchable`](OutputAccess::Patchable)
    /// and an error when it is not: an encoder that will patch must
    /// [`hold`](Self::hold) on a forward destination.
    #[track_caller]
    pub fn patch(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        let end = offset
            .checked_add(data.len() as u64)
            .filter(|end| *end <= self.position())
            .ok_or_else(|| error_msg(ErrorKind::Internal, "patch past the written output"))?;
        // The part that has left, then the part still here.
        let gone = (self.base.saturating_sub(offset) as usize).min(data.len());
        if gone > 0 {
            if self.access != OutputAccess::Patchable {
                return Err(error_msg(
                    ErrorKind::Internal,
                    "patch to bytes a forward destination already took",
                ));
            }
            self.patches.push(Patch {
                offset,
                bytes: data[..gone].to_vec(),
            });
        }
        if end > self.base {
            let start = (offset.max(self.base) - self.base) as usize;
            self.bytes[start..start + data.len() - gone].copy_from_slice(&data[gone..]);
        }
        Ok(())
    }

    #[track_caller]
    fn admit(&self, more: usize) -> Result<()> {
        let total = self.position() + more as u64;
        match self.limit {
            Some(limit) if total > limit => {
                Err(CodecError::new(ErrorKind::Limit(LimitKind::OutputBytes))
                    .with_native(LimitExceeded {
                        kind: LimitKind::OutputBytes,
                        limit,
                        actual: Some(total),
                    })
                    .at())
            }
            _ => Ok(()),
        }
    }

    // ── driver side ──

    /// Bytes ready to leave.
    pub fn pending(&self) -> &[u8] {
        if self.held { &[] } else { &self.bytes }
    }

    /// Take every pending byte, with its allocation.
    pub fn take(&mut self) -> Vec<u8> {
        if self.held {
            return Vec::new();
        }
        self.base += self.bytes.len() as u64;
        core::mem::take(&mut self.bytes)
    }

    /// Give back a vector from [`take`](Self::take) so its capacity is reused.
    pub fn recycle(&mut self, mut spare: Vec<u8>) {
        if self.bytes.capacity() == 0 {
            spare.clear();
            self.bytes = spare;
        }
    }

    /// Take the patches recorded so far, in the order they must be applied.
    pub fn take_patches(&mut self) -> Vec<Patch> {
        core::mem::take(&mut self.patches)
    }

    /// Whether any patch is waiting.
    pub fn has_patches(&self) -> bool {
        !self.patches.is_empty()
    }
}
