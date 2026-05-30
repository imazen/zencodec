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
}
