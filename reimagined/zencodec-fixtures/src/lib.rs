//! Three small codecs written against the reimagined contract.
//!
//! They are not image formats anyone should use. Each exists to exercise a
//! different shape of codec through the same four traits:
//!
//! | codec | shape | exercises |
//! |---|---|---|
//! | [`rows::RowsCodec`] (`ZRW1`) | streams rows as bytes arrive | borrowed-row delivery, animation, planar strips, streaming output, a finish patch |
//! | [`rle::RleCodec`] (`ZWH1`) | whole file in, whole image out | the [`zencodec::simple`] adapters, owned-frame delivery |
//! | [`bias::BiasCodec`] (`ZTP1`) | two passes over the pixels | row replay, lent-row delivery, metadata after the pixels and a range read to reach it early |
//!
//! Every sample they write is 8-bit and every one of them is lossless.

#![forbid(unsafe_code)]

pub mod bias;
pub mod document;
pub mod rle;
pub mod rows;

use core::fmt;
use zencodec::{ErrorKind, Layout, NativeError};
use zenpixels::PixelDescriptor;

/// The fixtures' own error type, kept as the cause of contract errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixtureError {
    /// The file does not start with the format's magic.
    BadMagic,
    /// The layout byte is not one the format defines.
    BadLayout(u8),
    /// A header field is impossible.
    BadHeader(&'static str),
    /// The compressed stream is inconsistent.
    BadStream(&'static str),
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("bad magic"),
            Self::BadLayout(b) => write!(f, "unknown layout byte {b}"),
            Self::BadHeader(what) => write!(f, "bad header: {what}"),
            Self::BadStream(what) => write!(f, "bad stream: {what}"),
        }
    }
}

impl core::error::Error for FixtureError {}

impl NativeError for FixtureError {
    const CODEC: &'static str = "zencodec-fixtures";
    fn kind(&self) -> ErrorKind {
        ErrorKind::Malformed
    }
}

/// The packed layouts the fixtures store, by layout byte.
pub const PACKED: [PixelDescriptor; 3] = [
    PixelDescriptor::GRAY8_SRGB,
    PixelDescriptor::RGB8_SRGB,
    PixelDescriptor::RGBA8_SRGB,
];

pub(crate) fn packed_from_byte(byte: u8) -> Result<PixelDescriptor, FixtureError> {
    PACKED
        .get(byte as usize)
        .copied()
        .ok_or(FixtureError::BadLayout(byte))
}

pub(crate) fn packed_to_byte(descriptor: PixelDescriptor) -> Option<u8> {
    PACKED
        .iter()
        .position(|d| *d == descriptor)
        .map(|i| i as u8)
}

pub(crate) fn packed_of(layout: &Layout) -> Option<PixelDescriptor> {
    layout.descriptor().filter(|d| PACKED.contains(d))
}

pub(crate) fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub(crate) fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(b)
}
