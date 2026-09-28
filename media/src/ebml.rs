//! EBML primitives for WebM/Matroska: element IDs and VINT sizes over blocking
//! `Read`/`Write`, with explicit unknown-size handling and bounded allocation.
//!
//! Element ID and size fields share a length-prefix encoding: the number of
//! leading zero bits before the marker bit gives the field width. IDs keep the
//! marker bit in the value; sizes do not (a size field of all data-bits-set is
//! the "unknown size" sentinel used by streaming Segments and Clusters).

use crate::track::MediaError;
use std::io::{Read, Write};

/// Largest element ID/size field in bytes.
pub const MAX_VINT_LEN: usize = 8;

/// Decoded element header: raw ID (marker bit retained) and payload size.
/// `size == None` is EBML unknown-size (only legal for master elements in
/// streaming profiles — Segment and Cluster here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementHeader {
    pub id: u64,
    pub size: Option<u64>,
    /// Total header bytes consumed (id field + size field).
    pub header_len: u64,
}

/// Read one VINT field: returns (value-without-marker, encoded length).
/// `keep_marker` yields the field with the marker bit retained (element IDs).
pub(crate) fn read_vint<R: Read>(r: &mut R, keep_marker: bool) -> Result<(u64, usize), MediaError> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first).map_err(MediaError::Io)?;
    let b = first[0];
    if b == 0 {
        return Err(MediaError::Format("vint field with zero first byte"));
    }
    let len = b.leading_zeros() as usize + 1;
    if len > MAX_VINT_LEN {
        return Err(MediaError::Format("vint length > 8"));
    }
    let mut value = if keep_marker {
        b as u64
    } else {
        // u64 mask: for len==8 the shift yields 0 (every bit is the marker).
        (b as u64) & (0xFFu64 >> len)
    };
    if len > 1 {
        let mut rest = [0u8; MAX_VINT_LEN - 1];
        r.read_exact(&mut rest[..len - 1]).map_err(MediaError::Io)?;
        for &x in &rest[..len - 1] {
            value = (value << 8) | x as u64;
        }
    }
    Ok((value, len))
}

/// Read one element header (ID + size). `size == None` when the field is the
/// all-ones unknown-size sentinel.
pub fn read_element_header<R: Read>(r: &mut R) -> Result<ElementHeader, MediaError> {
    let (id, id_len) = read_vint(r, true)?;
    let (size_raw, size_len) = read_vint(r, false)?;
    let unknown = size_raw == (1u64 << (7 * size_len)) - 1;
    Ok(ElementHeader {
        id,
        size: if unknown { None } else { Some(size_raw) },
        header_len: (id_len + size_len) as u64,
    })
}

/// Encode `id` as an element-ID field. IDs are written at their natural width
/// (the width of their marker bit) — all IDs we emit are spec-defined.
pub fn write_id<W: Write>(w: &mut W, id: u64) -> std::io::Result<()> {
    debug_assert!(id > 0);
    // The marker bit of an n-byte ID sits at bit 7n (n=1 → bit 7, n=4 → bit 28).
    // Width is the position of the top set bit, NOT the value's magnitude —
    // e.g. 0x4286 is a 2-byte ID even though 0x4286 > 0x3FFF.
    let top_bit = 63 - id.leading_zeros() as usize;
    let len = (top_bit / 7).max(1);
    w.write_all(&id.to_be_bytes()[8 - len..])
}

/// Encode a payload size at the smallest width that can represent it.
/// The all-ones value is reserved (unknown-size sentinel), so a `v` that
/// collides with it is promoted to the next width. `None` writes the 8-byte
/// unknown-size sentinel (only for Segment/Cluster in streaming output).
pub fn write_size<W: Write>(w: &mut W, size: Option<u64>) -> std::io::Result<()> {
    let Some(v) = size else {
        w.write_all(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])?;
        return Ok(());
    };
    let mut len = 1usize;
    while len < 8 && v >= (1u64 << (7 * len)) - 1 {
        len += 1;
    }
    // At len 8 the largest legal value is 2^56-2; 2^56-1 is the sentinel.
    if v >= (1u64 << 56) - 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "size exceeds 56-bit EBML limit",
        ));
    }
    // Field = marker bit at position 7*len OR'd over the big-endian value.
    let field = (1u64 << (7 * len)) | v;
    w.write_all(&field.to_be_bytes()[8 - len..])
}

/// Write a complete element (id + size + payload).
pub fn write_element<W: Write>(w: &mut W, id: u64, payload: &[u8]) -> std::io::Result<()> {
    write_id(w, id)?;
    write_size(w, Some(payload.len() as u64))?;
    w.write_all(payload)
}

/// Write an unsigned-integer element (EBML uint is big-endian, minimal width;
/// zero may be written as a zero-length field but one byte is safer for readers).
pub fn write_uint<W: Write>(w: &mut W, id: u64, value: u64) -> std::io::Result<()> {
    let bytes = value.to_be_bytes();
    let mut start = 0;
    while start < 7 && bytes[start] == 0 {
        start += 1;
    }
    write_element(w, id, &bytes[start..])
}

/// Write a signed-integer element (minimal-width two's complement: a leading
/// byte may be dropped only when it is the sign extension of the next byte).
pub fn write_int<W: Write>(w: &mut W, id: u64, value: i64) -> std::io::Result<()> {
    let bytes = value.to_be_bytes();
    let mut start = 0;
    while start < 7
        && ((bytes[start] == 0x00 && bytes[start + 1] & 0x80 == 0)
            || (bytes[start] == 0xFF && bytes[start + 1] & 0x80 != 0))
    {
        start += 1;
    }
    write_element(w, id, &bytes[start..])
}

/// Write a UTF-8 string element.
pub fn write_utf8<W: Write>(w: &mut W, id: u64, s: &str) -> std::io::Result<()> {
    write_element(w, id, s.as_bytes())
}

/// Well-known element IDs used by the WebM subset.
pub mod id {
    pub const EBML: u64 = 0x1A45DFA3;
    pub const EBML_VERSION: u64 = 0x4286;
    pub const EBML_READ_VERSION: u64 = 0x42F7;
    pub const EBML_MAX_ID_LENGTH: u64 = 0x42F2;
    pub const EBML_MAX_SIZE_LENGTH: u64 = 0x42F3;
    pub const DOC_TYPE: u64 = 0x4282;
    pub const DOC_TYPE_VERSION: u64 = 0x4287;
    pub const DOC_TYPE_READ_VERSION: u64 = 0x4285;
    pub const VOID: u64 = 0xEC;
    pub const CRC32: u64 = 0xBF;

    pub const SEGMENT: u64 = 0x18538067;
    pub const SEEK_HEAD: u64 = 0x114D9B74;
    pub const INFO: u64 = 0x1549A966;
    pub const TIMESTAMP_SCALE: u64 = 0x2AD7B1;
    pub const DURATION: u64 = 0x4489;
    pub const MUXING_APP: u64 = 0x4D80;
    pub const WRITING_APP: u64 = 0x5741;

    pub const TRACKS: u64 = 0x1654AE6B;
    pub const TRACK_ENTRY: u64 = 0xAE;
    pub const TRACK_NUMBER: u64 = 0xD7;
    pub const TRACK_UID: u64 = 0x73C5;
    pub const TRACK_TYPE: u64 = 0x83;
    pub const FLAG_ENABLED: u64 = 0xB9;
    pub const FLAG_DEFAULT: u64 = 0x88;
    pub const FLAG_FORCED: u64 = 0x55AA;
    pub const FLAG_LACING: u64 = 0x9C;
    pub const DEFAULT_DURATION: u64 = 0x23E383;
    pub const CODEC_ID: u64 = 0x86;
    pub const CODEC_PRIVATE: u64 = 0x63A2;
    pub const CODEC_DELAY: u64 = 0x56AA;
    pub const SEEK_PRE_ROLL: u64 = 0x56BB;
    pub const VIDEO: u64 = 0xE0;
    pub const PIXEL_WIDTH: u64 = 0xB0;
    pub const PIXEL_HEIGHT: u64 = 0xBA;
    pub const AUDIO: u64 = 0xE1;
    pub const SAMPLING_FREQUENCY: u64 = 0xB5;
    pub const CHANNELS: u64 = 0x9F;
    pub const BIT_DEPTH: u64 = 0x6264;

    pub const CLUSTER: u64 = 0x1F43B675;
    pub const TIMESTAMP: u64 = 0xE7;
    pub const SIMPLE_BLOCK: u64 = 0xA3;
    pub const BLOCK_GROUP: u64 = 0xA0;
    pub const BLOCK: u64 = 0xA1;
    pub const BLOCK_DURATION: u64 = 0x9B;
    pub const REFERENCE_BLOCK: u64 = 0xFB;
    pub const DISCARD_PADDING: u64 = 0x75A2;

    pub const CUES: u64 = 0x1C53BB6B;
    pub const TAGS: u64 = 0x1254C367;
    pub const CHAPTERS: u64 = 0x1043A770;
    pub const ATTACHMENTS: u64 = 0x1941A469;
}
