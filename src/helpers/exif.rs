//! Minimal EXIF orientation parser.
//!
//! Parses the EXIF Orientation tag (0x0112 / TIFF tag 274) from TIFF-structured
//! EXIF data. Handles both raw TIFF bytes and JPEG APP1 style (`Exif\0\0` prefix).
//!
//! Spec references:
//! - TIFF 6.0 specification (Adobe, 1992): IFD structure, byte order, tag 274
//! - EXIF 2.32 (CIPA DC-008-Translation-2019): Orientation tag semantics
//! - TIFF/EP (ISO 12234-2): Same orientation tag definition
//!
//! # Design
//!
//! This parser is intentionally minimal — it extracts only the orientation tag.
//! For full EXIF parsing (make, model, GPS, dates), use `zencodecs::exif::parse_exif`.
//!
//! Safety properties:
//! - Every byte read is bounds-checked (returns `None` on truncation)
//! - IFD entry count capped at 1000 to prevent DoS from malformed data
//! - No recursion, no heap allocation, `no_std` compatible
//! - Handles both big-endian (Motorola/MM) and little-endian (Intel/II)
//! - Accepts TIFF SHORT (type 3) and LONG (type 4) for the orientation value
//! - Validates orientation value is in 1..=8
//! - Does NOT follow EXIF sub-IFD pointers (orientation is always in IFD0)

use zenpixels::Orientation;

/// EXIF Orientation tag (TIFF tag 274 / 0x0112).
const TAG_ORIENTATION: u16 = 0x0112;
/// TIFF type SHORT (unsigned 16-bit integer).
const TIFF_SHORT: u16 = 3;
/// TIFF type LONG (unsigned 32-bit integer).
const TIFF_LONG: u16 = 4;
/// Maximum IFD entries to scan before giving up (DoS protection).
const MAX_IFD_ENTRIES: u16 = 1000;
/// Minimum TIFF header size: byte order (2) + magic (2) + IFD0 offset (4).
const TIFF_HEADER_SIZE: usize = 8;

/// Parse the EXIF orientation from TIFF-structured EXIF data.
///
/// Accepts either:
/// - **Raw TIFF bytes** starting with byte order mark (`II` or `MM`)
/// - **JPEG APP1 style** with `Exif\0\0` prefix followed by TIFF data
/// - **HEIF EXIF item** with 4-byte offset header — strip this before calling
///
/// Returns the [`Orientation`] if the tag is found and valid (1-8),
/// or `None` for missing/invalid/truncated data.
///
/// # Spec compliance
///
/// - Validates TIFF byte order mark and magic number (42)
/// - Walks IFD0 entries up to [`MAX_IFD_ENTRIES`] (1000)
/// - Accepts both SHORT (2-byte) and LONG (4-byte) orientation values,
///   per TIFF 6.0 which recommends SHORT but doesn't forbid LONG
/// - Exploits IFD tag sort order for early exit (tags are sorted ascending)
/// - Correctly handles the IFD value/offset field: values ≤4 bytes are
///   stored inline at the entry's value field, not at an external offset
///
/// # Examples
///
/// ```
/// use zencodec::helpers::parse_exif_orientation;
/// use zenpixels::Orientation;
///
/// // Minimal valid TIFF with orientation tag (little-endian)
/// let mut tiff = vec![
///     b'I', b'I',           // byte order: little-endian
///     42, 0,                 // TIFF magic
///     8, 0, 0, 0,           // IFD0 offset = 8
///     1, 0,                  // 1 IFD entry
///     0x12, 0x01,            // tag = 0x0112 (Orientation)
///     3, 0,                  // type = SHORT
///     1, 0, 0, 0,           // count = 1
///     6, 0, 0, 0,           // value = 6 (Rotate90)
/// ];
/// assert_eq!(
///     parse_exif_orientation(&tiff),
///     Some(Orientation::Rotate90),
/// );
///
/// // Also works with Exif\0\0 prefix (JPEG APP1 style)
/// let mut app1 = b"Exif\0\0".to_vec();
/// app1.extend_from_slice(&tiff);
/// assert_eq!(
///     parse_exif_orientation(&app1),
///     Some(Orientation::Rotate90),
/// );
/// ```
pub fn parse_exif_orientation(data: &[u8]) -> Option<Orientation> {
    // Strip optional Exif\0\0 prefix (JPEG APP1 style).
    let tiff = if data.len() >= 6 && data[..6] == *b"Exif\0\0" {
        &data[6..]
    } else {
        data
    };

    if tiff.len() < TIFF_HEADER_SIZE {
        return None;
    }

    // Determine byte order from TIFF header.
    let be = match [tiff[0], tiff[1]] {
        [b'M', b'M'] => true,  // Motorola byte order (big-endian)
        [b'I', b'I'] => false, // Intel byte order (little-endian)
        _ => return None,
    };

    // Verify TIFF magic number (42).
    if rd16(tiff, 2, be)? != 42 {
        return None;
    }

    // Read IFD0 offset and validate.
    let ifd0 = rd32(tiff, 4, be)? as usize;
    let entry_count = rd16(tiff, ifd0, be)?;

    // Cap entry count to prevent DoS from malformed data.
    if entry_count > MAX_IFD_ENTRIES {
        return None;
    }

    let entries_start = ifd0.checked_add(2)?;

    // Walk IFD0 entries looking for orientation tag.
    for i in 0..entry_count as usize {
        let off = entries_start.checked_add(i.checked_mul(12)?)?;

        // Each IFD entry is 12 bytes: tag(2) + type(2) + count(4) + value(4)
        if off.checked_add(12)? > tiff.len() {
            break;
        }

        let tag = rd16(tiff, off, be)?;

        // IFD entries are sorted by tag number (TIFF 6.0 §2).
        // If we've passed 0x0112, it's not here.
        if tag > TAG_ORIENTATION {
            break;
        }
        if tag != TAG_ORIENTATION {
            continue;
        }

        let type_id = rd16(tiff, off + 2, be)?;
        let count = rd32(tiff, off + 4, be)?;

        // Orientation must be a single value.
        if count < 1 {
            return None;
        }

        // Read the value. Per TIFF 6.0 §2: if the value fits in 4 bytes,
        // it's stored inline at offset+8. Orientation is SHORT (2 bytes)
        // or occasionally LONG (4 bytes) — both fit inline.
        let raw = match type_id {
            TIFF_SHORT => rd16(tiff, off + 8, be)? as u32,
            TIFF_LONG => rd32(tiff, off + 8, be)?,
            _ => return None,
        };

        // Orientation values are 1-8.
        if raw > 8 {
            return None;
        }
        return Orientation::from_exif(raw as u8);
    }

    None
}

/// Read a u16 from `data` at `offset` with bounds checking.
fn rd16(data: &[u8], offset: usize, big_endian: bool) -> Option<u16> {
    let b = data.get(offset..offset + 2)?;
    Some(if big_endian {
        u16::from_be_bytes([b[0], b[1]])
    } else {
        u16::from_le_bytes([b[0], b[1]])
    })
}

/// Read a u32 from `data` at `offset` with bounds checking.
fn rd32(data: &[u8], offset: usize, big_endian: bool) -> Option<u32> {
    let b = data.get(offset..offset + 4)?;
    Some(if big_endian {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Build a minimal TIFF with one IFD entry for orientation.
    fn make_tiff(big_endian: bool, orientation: u16, type_id: u16) -> Vec<u8> {
        let mut buf = Vec::new();
        let w16 = |buf: &mut Vec<u8>, v: u16| {
            if big_endian {
                buf.extend_from_slice(&v.to_be_bytes());
            } else {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        };
        let w32 = |buf: &mut Vec<u8>, v: u32| {
            if big_endian {
                buf.extend_from_slice(&v.to_be_bytes());
            } else {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        };

        // Header
        if big_endian {
            buf.extend_from_slice(b"MM");
        } else {
            buf.extend_from_slice(b"II");
        }
        w16(&mut buf, 42); // magic
        w32(&mut buf, 8); // IFD0 offset

        // IFD0
        w16(&mut buf, 1); // 1 entry
        w16(&mut buf, TAG_ORIENTATION); // tag
        w16(&mut buf, type_id); // type
        w32(&mut buf, 1); // count
        if type_id == TIFF_LONG {
            w32(&mut buf, orientation as u32); // value (LONG)
        } else {
            w16(&mut buf, orientation); // value (SHORT)
            w16(&mut buf, 0); // padding
        }

        buf
    }

    // ── Basic parsing ──────────────────────────────────────────────────

    #[test]
    fn identity_little_endian() {
        let tiff = make_tiff(false, 1, TIFF_SHORT);
        assert_eq!(parse_exif_orientation(&tiff), Some(Orientation::Identity));
    }

    #[test]
    fn rotate90_big_endian() {
        let tiff = make_tiff(true, 6, TIFF_SHORT);
        assert_eq!(parse_exif_orientation(&tiff), Some(Orientation::Rotate90));
    }

    #[test]
    fn all_orientations() {
        let expected = [
            (1, Orientation::Identity),
            (2, Orientation::FlipH),
            (3, Orientation::Rotate180),
            (4, Orientation::FlipV),
            (5, Orientation::Transpose),
            (6, Orientation::Rotate90),
            (7, Orientation::Transverse),
            (8, Orientation::Rotate270),
        ];
        for (val, orient) in expected {
            let tiff = make_tiff(false, val, TIFF_SHORT);
            assert_eq!(parse_exif_orientation(&tiff), Some(orient), "value={val}");
        }
    }

    // ── APP1 prefix handling ───────────────────────────────────────────

    #[test]
    fn with_exif_prefix() {
        let tiff = make_tiff(false, 6, TIFF_SHORT);
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        assert_eq!(parse_exif_orientation(&app1), Some(Orientation::Rotate90));
    }

    #[test]
    fn raw_tiff_without_prefix() {
        let tiff = make_tiff(true, 3, TIFF_SHORT);
        assert_eq!(parse_exif_orientation(&tiff), Some(Orientation::Rotate180));
    }

    // ── LONG type support ──────────────────────────────────────────────

    #[test]
    fn orientation_as_long() {
        let tiff = make_tiff(false, 8, TIFF_LONG);
        assert_eq!(parse_exif_orientation(&tiff), Some(Orientation::Rotate270));
    }

    #[test]
    fn orientation_as_long_big_endian() {
        let tiff = make_tiff(true, 5, TIFF_LONG);
        assert_eq!(parse_exif_orientation(&tiff), Some(Orientation::Transpose));
    }

    // ── Invalid/edge cases ─────────────────────────────────────────────

    #[test]
    fn empty_input() {
        assert_eq!(parse_exif_orientation(&[]), None);
    }

    #[test]
    fn too_short() {
        assert_eq!(parse_exif_orientation(&[0x49, 0x49, 42, 0]), None);
    }

    #[test]
    fn bad_byte_order() {
        let mut tiff = make_tiff(false, 1, TIFF_SHORT);
        tiff[0] = b'X';
        assert_eq!(parse_exif_orientation(&tiff), None);
    }

    #[test]
    fn bad_magic() {
        let mut tiff = make_tiff(false, 1, TIFF_SHORT);
        tiff[2] = 0;
        tiff[3] = 0; // magic = 0 instead of 42
        assert_eq!(parse_exif_orientation(&tiff), None);
    }

    #[test]
    fn orientation_value_0_invalid() {
        let tiff = make_tiff(false, 0, TIFF_SHORT);
        assert_eq!(parse_exif_orientation(&tiff), None);
    }

    #[test]
    fn orientation_value_9_invalid() {
        let tiff = make_tiff(false, 9, TIFF_SHORT);
        assert_eq!(parse_exif_orientation(&tiff), None);
    }

    #[test]
    fn orientation_value_255_invalid() {
        let tiff = make_tiff(false, 255, TIFF_SHORT);
        assert_eq!(parse_exif_orientation(&tiff), None);
    }

    #[test]
    fn wrong_type_rejected() {
        // TIFF type 2 (ASCII) is not valid for orientation
        let tiff = make_tiff(false, 6, 2);
        assert_eq!(parse_exif_orientation(&tiff), None);
    }

    #[test]
    fn ifd_offset_beyond_data() {
        let mut tiff = make_tiff(false, 1, TIFF_SHORT);
        // Set IFD0 offset to beyond data length
        tiff[4] = 0xFF;
        tiff[5] = 0xFF;
        tiff[6] = 0;
        tiff[7] = 0;
        assert_eq!(parse_exif_orientation(&tiff), None);
    }

    #[test]
    fn truncated_ifd_entry() {
        let tiff = make_tiff(false, 6, TIFF_SHORT);
        // Truncate to just after IFD entry count, before the entry data
        assert_eq!(parse_exif_orientation(&tiff[..12]), None);
    }

    // ── Tag sorting / early exit ───────────────────────────────────────

    #[test]
    fn orientation_after_other_tags() {
        // Build TIFF with ImageWidth (0x0100) before Orientation (0x0112)
        let mut buf = Vec::new();
        buf.extend_from_slice(b"II"); // little-endian
        buf.extend_from_slice(&42u16.to_le_bytes());
        buf.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset

        buf.extend_from_slice(&2u16.to_le_bytes()); // 2 entries

        // Entry 1: ImageWidth (0x0100) = 640
        buf.extend_from_slice(&0x0100u16.to_le_bytes());
        buf.extend_from_slice(&TIFF_SHORT.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&640u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());

        // Entry 2: Orientation (0x0112) = 6
        buf.extend_from_slice(&TAG_ORIENTATION.to_le_bytes());
        buf.extend_from_slice(&TIFF_SHORT.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&6u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());

        assert_eq!(parse_exif_orientation(&buf), Some(Orientation::Rotate90));
    }

    #[test]
    fn early_exit_on_higher_tag() {
        // Build TIFF with only ImageDescription (0x010E) — past 0x0112 in sort order? No, 0x010E < 0x0112.
        // Use XResolution (0x011A) which is > 0x0112 to test early exit.
        let mut buf = Vec::new();
        buf.extend_from_slice(b"II");
        buf.extend_from_slice(&42u16.to_le_bytes());
        buf.extend_from_slice(&8u32.to_le_bytes());

        buf.extend_from_slice(&1u16.to_le_bytes()); // 1 entry

        // Entry: XResolution (0x011A) — tag > TAG_ORIENTATION
        buf.extend_from_slice(&0x011Au16.to_le_bytes());
        buf.extend_from_slice(&TIFF_SHORT.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&72u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());

        // Should return None (orientation not present, early exit)
        assert_eq!(parse_exif_orientation(&buf), None);
    }

    // ── DoS protection ─────────────────────────────────────────────────

    #[test]
    fn excessive_entry_count_rejected() {
        let mut tiff = make_tiff(false, 6, TIFF_SHORT);
        // Set entry count to 1001 (> MAX_IFD_ENTRIES)
        tiff[8] = 0xE9;
        tiff[9] = 0x03; // 1001 in LE
        assert_eq!(parse_exif_orientation(&tiff), None);
    }

    #[test]
    fn max_entry_count_accepted() {
        let mut tiff = make_tiff(false, 6, TIFF_SHORT);
        // Set entry count to 1000 (= MAX_IFD_ENTRIES) — accepted but
        // will break on bounds check since we don't have 1000 entries
        tiff[8] = 0xE8;
        tiff[9] = 0x03; // 1000 in LE
        // Won't find orientation (entry is at index 0 but tag bytes are
        // now part of the "count" field area) — just shouldn't panic
        let _ = parse_exif_orientation(&tiff);
    }

    // ── Exif\0\0 prefix edge cases ────────────────────────────────────

    #[test]
    fn exif_prefix_only_no_tiff() {
        assert_eq!(parse_exif_orientation(b"Exif\0\0"), None);
    }

    #[test]
    fn exif_prefix_truncated() {
        assert_eq!(parse_exif_orientation(b"Exif\0"), None);
    }

    #[test]
    fn exif_prefix_with_garbage() {
        let data = b"Exif\0\0GARBAGE".to_vec();
        assert_eq!(parse_exif_orientation(&data), None);
    }
}
