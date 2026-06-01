//! Lightweight EXIF orientation accessor.
//!
//! A thin convenience over the structured [`crate::exif`] parser: extracts just
//! the Orientation tag (0x0112) from a TIFF/EXIF blob. For anything richer
//! (copyright, GPS, thumbnail, pruning, re-serialization) use [`crate::exif::Exif`].

use zenpixels::Orientation;

/// Parse the EXIF Orientation tag (0x0112) from a TIFF/EXIF blob.
///
/// Accepts raw TIFF bytes or a JPEG APP1 `Exif\0\0`-prefixed blob, both byte
/// orders, and SHORT or LONG values. Delegates to [`crate::exif::Exif`], so it
/// is fully bounds-checked and never panics on malformed input. Returns `None`
/// if the blob is malformed or carries no valid Orientation tag.
///
/// # Examples
///
/// ```
/// use zencodec::helpers::parse_exif_orientation;
/// use zenpixels::Orientation;
///
/// let tiff = vec![
///     b'I', b'I', 42, 0, 8, 0, 0, 0, // header: LE, magic 42, IFD0 @ 8
///     1, 0,                          // 1 entry
///     0x12, 0x01, 3, 0, 1, 0, 0, 0,  // tag 0x0112, SHORT, count 1
///     6, 0, 0, 0,                    // value 6 (Rotate90)
///     0, 0, 0, 0,                    // next IFD = 0
/// ];
/// assert_eq!(parse_exif_orientation(&tiff), Some(Orientation::Rotate90));
/// ```
pub fn parse_exif_orientation(data: &[u8]) -> Option<Orientation> {
    crate::exif::Exif::parse(data)?.orientation()
}

/// Rewrite the EXIF Orientation tag (0x0112) in a TIFF/EXIF blob to `value`,
/// returning a new blob.
///
/// The orientation value is stored inline (SHORT or LONG), so this overwrites it
/// in place without recomputing any TIFF offsets. Accepts raw TIFF bytes or a
/// JPEG APP1 `Exif\0\0`-prefixed blob, both byte orders, fully bounds-checked.
///
/// Returns `None` if the blob is malformed or carries no Orientation tag — the
/// caller should then leave the blob unchanged. This is the byte-level half of
/// closing the double-rotation hazard: when a decoder bakes orientation upright,
/// the structured field says `Identity` but the embedded blob still says e.g.
/// `Rotate90`; rewriting the tag to `1` keeps them in agreement.
pub fn set_exif_orientation(data: &[u8], value: Orientation) -> Option<alloc::vec::Vec<u8>> {
    // Optional "Exif\0\0" prefix; TIFF offsets are relative to the TIFF header.
    let tiff_start = if data.len() >= 6 && &data[0..4] == b"Exif" && data[4] == 0 && data[5] == 0 {
        6
    } else {
        0
    };
    let tiff = data.get(tiff_start..)?;
    if tiff.len() < 8 {
        return None;
    }
    let be = match &tiff[0..2] {
        b"II" => false,
        b"MM" => true,
        _ => return None,
    };
    let r16 = |o: usize| -> Option<u16> {
        let s = tiff.get(o..o + 2)?;
        Some(if be {
            u16::from_be_bytes([s[0], s[1]])
        } else {
            u16::from_le_bytes([s[0], s[1]])
        })
    };
    let r32 = |o: usize| -> Option<u32> {
        let s = tiff.get(o..o + 4)?;
        Some(if be {
            u32::from_be_bytes([s[0], s[1], s[2], s[3]])
        } else {
            u32::from_le_bytes([s[0], s[1], s[2], s[3]])
        })
    };
    if r16(2)? != 42 {
        return None;
    }
    let ifd = r32(4)? as usize;
    let count = r16(ifd)? as usize;
    if count > 4096 {
        return None; // DoS cap on IFD entry count
    }
    let v = value as u8;
    for i in 0..count {
        let entry = ifd + 2 + i * 12;
        if r16(entry)? != 0x0112 {
            continue;
        }
        let type_id = r16(entry + 2)?;
        // Value field is the last 4 bytes of the 12-byte entry, absolute in `data`.
        let off = tiff_start + entry + 8;
        let mut out = data.to_vec();
        match type_id {
            3 => {
                let b = if be {
                    (v as u16).to_be_bytes()
                } else {
                    (v as u16).to_le_bytes()
                };
                *out.get_mut(off)? = b[0];
                *out.get_mut(off + 1)? = b[1];
            }
            4 => {
                let b = if be {
                    (v as u32).to_be_bytes()
                } else {
                    (v as u32).to_le_bytes()
                };
                for (k, byte) in b.iter().enumerate() {
                    *out.get_mut(off + k)? = *byte;
                }
            }
            _ => return None,
        }
        return Some(out);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// Minimal TIFF with one orientation entry (SHORT or LONG), either order.
    fn tiff(order_be: bool, value: u32, type_id: u16) -> Vec<u8> {
        let mut v = Vec::new();
        let w16 = |v: &mut Vec<u8>, x: u16| {
            v.extend_from_slice(&if order_be {
                x.to_be_bytes()
            } else {
                x.to_le_bytes()
            })
        };
        let w32 = |v: &mut Vec<u8>, x: u32| {
            v.extend_from_slice(&if order_be {
                x.to_be_bytes()
            } else {
                x.to_le_bytes()
            })
        };
        v.extend_from_slice(if order_be { b"MM" } else { b"II" });
        w16(&mut v, 42);
        w32(&mut v, 8);
        w16(&mut v, 1); // 1 entry
        w16(&mut v, 0x0112);
        w16(&mut v, type_id);
        w32(&mut v, 1);
        // Inline value: SHORT occupies the leading bytes of the 4-byte field.
        if type_id == 3 {
            w16(&mut v, value as u16);
            w16(&mut v, 0);
        } else {
            w32(&mut v, value);
        }
        w32(&mut v, 0); // next IFD
        v
    }

    #[test]
    fn short_little_endian() {
        assert_eq!(
            parse_exif_orientation(&tiff(false, 6, 3)),
            Some(Orientation::Rotate90)
        );
    }

    #[test]
    fn short_big_endian() {
        assert_eq!(
            parse_exif_orientation(&tiff(true, 6, 3)),
            Some(Orientation::Rotate90)
        );
    }

    #[test]
    fn long_type_both_orders() {
        assert_eq!(
            parse_exif_orientation(&tiff(false, 8, 4)),
            Some(Orientation::Rotate270)
        );
        assert_eq!(
            parse_exif_orientation(&tiff(true, 8, 4)),
            Some(Orientation::Rotate270)
        );
    }

    #[test]
    fn with_exif_prefix() {
        let mut blob = b"Exif\0\0".to_vec();
        blob.extend_from_slice(&tiff(false, 6, 3));
        assert_eq!(parse_exif_orientation(&blob), Some(Orientation::Rotate90));
    }

    #[test]
    fn invalid_inputs_return_none() {
        assert_eq!(parse_exif_orientation(b"garbage"), None);
        assert_eq!(parse_exif_orientation(&[]), None);
        assert_eq!(parse_exif_orientation(&[0u8; 7]), None);
        // Orientation value out of range (9) → no valid orientation.
        assert_eq!(parse_exif_orientation(&tiff(false, 9, 3)), None);
        assert_eq!(parse_exif_orientation(&tiff(false, 0, 3)), None);
    }

    #[test]
    fn set_orientation_roundtrips_all_orders_and_types() {
        for be in [false, true] {
            for &type_id in &[3u16, 4] {
                // Start at Rotate90 (6), rewrite to Identity (1), read back.
                let blob = tiff(be, 6, type_id);
                assert_eq!(parse_exif_orientation(&blob), Some(Orientation::Rotate90));
                let rewritten =
                    set_exif_orientation(&blob, Orientation::Identity).expect("tag present");
                assert_eq!(rewritten.len(), blob.len()); // offsets unchanged
                assert_eq!(
                    parse_exif_orientation(&rewritten),
                    Some(Orientation::Identity)
                );
            }
        }
    }

    #[test]
    fn set_orientation_with_exif_prefix() {
        let mut blob = b"Exif\0\0".to_vec();
        blob.extend_from_slice(&tiff(false, 6, 3));
        let out = set_exif_orientation(&blob, Orientation::Rotate180).expect("tag present");
        assert_eq!(parse_exif_orientation(&out), Some(Orientation::Rotate180));
    }

    #[test]
    fn set_orientation_absent_tag_or_garbage_is_none() {
        // No 0x0112 entry: a minimal IFD with a different tag.
        let mut v = b"II".to_vec();
        v.extend_from_slice(&42u16.to_le_bytes());
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // 1 entry
        v.extend_from_slice(&0x010Fu16.to_le_bytes()); // Make tag, not orientation
        v.extend_from_slice(&3u16.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(set_exif_orientation(&v, Orientation::Identity), None);
        assert_eq!(
            set_exif_orientation(b"garbage", Orientation::Identity),
            None
        );
        assert_eq!(set_exif_orientation(&[], Orientation::Identity), None);
    }
}
