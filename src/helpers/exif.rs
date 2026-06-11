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
/// The orientation value is stored inline (SHORT or LONG), so the canonical
/// [`crate::exif::Exif`] parser locates it and the byte is overwritten in place
/// — no TIFF offsets are recomputed, so the rest of the blob is byte-identical.
/// Accepts raw TIFF bytes or a JPEG APP1 `Exif\0\0`-prefixed blob, both byte
/// orders.
///
/// Returns `None` if the blob is malformed or carries no Orientation tag — the
/// caller should then leave the blob unchanged. This is a *reconciliation*
/// primitive, so it deliberately never **adds** a tag; to insert one (authoring
/// a blob), use [`crate::exif::Exif::set_orientation`] and re-serialize. It is
/// the byte-level half of closing the double-rotation hazard: when a decoder
/// bakes orientation upright, the structured field says `Identity` but the
/// embedded blob still says e.g. `Rotate90`; rewriting the tag to `1` keeps
/// them in agreement.
///
/// Reuses the same IFD walker as [`parse_exif_orientation`] rather than a second
/// hand-rolled scanner, so the two can't diverge on prefix/byte-order/type/bounds
/// handling.
pub fn set_exif_orientation(data: &[u8], value: Orientation) -> Option<alloc::vec::Vec<u8>> {
    crate::exif::set_orientation(data, value)
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
