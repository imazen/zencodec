//! EXIF orientation support.

/// EXIF orientation tag values.
///
/// Describes how the stored pixels should be transformed for display.
/// Values match the EXIF Orientation tag (TIFF tag 274).
///
/// When a codec applies orientation during decode, it sets orientation to
/// [`Normal`](Orientation::Normal) in the returned [`ImageInfo`](crate::ImageInfo).
/// When orientation is not applied, the caller is responsible for transforming
/// the pixel data according to this value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum Orientation {
    /// No rotation or flip needed.
    #[default]
    Normal = 1,
    /// Flip horizontally (mirror left-right).
    FlipHorizontal = 2,
    /// Rotate 180 degrees.
    Rotate180 = 3,
    /// Flip vertically (mirror top-bottom).
    FlipVertical = 4,
    /// Transpose (rotate 90 CW then flip horizontally).
    Transpose = 5,
    /// Rotate 90 degrees clockwise.
    Rotate90 = 6,
    /// Transverse (rotate 90 CCW then flip horizontally).
    Transverse = 7,
    /// Rotate 270 degrees clockwise (= 90 CCW).
    Rotate270 = 8,
}

impl Orientation {
    /// Create from EXIF orientation value (1-8).
    ///
    /// Returns [`Normal`](Orientation::Normal) for out-of-range values.
    pub fn from_exif(value: u16) -> Self {
        match value {
            1 => Self::Normal,
            2 => Self::FlipHorizontal,
            3 => Self::Rotate180,
            4 => Self::FlipVertical,
            5 => Self::Transpose,
            6 => Self::Rotate90,
            7 => Self::Transverse,
            8 => Self::Rotate270,
            _ => Self::Normal,
        }
    }

    /// EXIF tag value (1-8).
    pub fn exif_value(self) -> u16 {
        self as u16
    }

    /// Whether this orientation swaps width and height.
    ///
    /// True for orientations involving a 90 or 270 degree rotation
    /// (values 5-8).
    pub fn swaps_dimensions(self) -> bool {
        matches!(
            self,
            Self::Transpose | Self::Rotate90 | Self::Transverse | Self::Rotate270
        )
    }

    /// Compute display dimensions for the given stored dimensions.
    ///
    /// If orientation swaps dimensions (90/270 rotation), width and height
    /// are exchanged.
    pub fn display_dimensions(self, stored_width: u32, stored_height: u32) -> (u32, u32) {
        if self.swaps_dimensions() {
            (stored_height, stored_width)
        } else {
            (stored_width, stored_height)
        }
    }

    /// Whether any transformation is needed.
    pub fn is_identity(self) -> bool {
        matches!(self, Self::Normal)
    }
}

/// How the decoder should handle orientation during decode.
///
/// Replaces a simple `with_orientation_hint(Orientation)` with richer
/// semantics: the caller can request orientation correction plus
/// additional transforms, which the decoder can coalesce into a
/// single operation (e.g., JPEG lossless DCT rotation).
///
/// Pass to [`DecodeJob::with_orientation()`](crate::decode::DecodeJob::with_orientation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OrientationHint {
    /// Don't touch orientation. Report intrinsic orientation in
    /// [`ImageInfo::orientation`](crate::ImageInfo).
    #[default]
    Preserve,

    /// Resolve EXIF/container orientation to [`Normal`](Orientation::Normal).
    ///
    /// The decoder coalesces this with the decode operation when possible
    /// (e.g., JPEG lossless DCT transform). The output `ImageInfo` will
    /// report `Orientation::Normal`.
    Correct,

    /// Resolve EXIF orientation, then apply an additional transform.
    ///
    /// The decoder coalesces the combined operation when possible.
    /// For example, if EXIF says Rotate90 and the hint says Rotate180,
    /// the decoder applies Rotate270 in a single step.
    CorrectAndTransform(Orientation),

    /// Ignore EXIF orientation. Apply exactly this transform.
    ///
    /// The EXIF orientation is not consulted. The given transform is
    /// applied literally.
    ExactTransform(Orientation),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_exif_valid() {
        assert_eq!(Orientation::from_exif(1), Orientation::Normal);
        assert_eq!(Orientation::from_exif(6), Orientation::Rotate90);
        assert_eq!(Orientation::from_exif(8), Orientation::Rotate270);
    }

    #[test]
    fn from_exif_invalid() {
        assert_eq!(Orientation::from_exif(0), Orientation::Normal);
        assert_eq!(Orientation::from_exif(9), Orientation::Normal);
        assert_eq!(Orientation::from_exif(255), Orientation::Normal);
    }

    #[test]
    fn swaps_dimensions() {
        assert!(!Orientation::Normal.swaps_dimensions());
        assert!(!Orientation::FlipHorizontal.swaps_dimensions());
        assert!(!Orientation::Rotate180.swaps_dimensions());
        assert!(!Orientation::FlipVertical.swaps_dimensions());
        assert!(Orientation::Transpose.swaps_dimensions());
        assert!(Orientation::Rotate90.swaps_dimensions());
        assert!(Orientation::Transverse.swaps_dimensions());
        assert!(Orientation::Rotate270.swaps_dimensions());
    }

    #[test]
    fn display_dimensions() {
        assert_eq!(Orientation::Normal.display_dimensions(100, 200), (100, 200));
        assert_eq!(
            Orientation::Rotate90.display_dimensions(100, 200),
            (200, 100)
        );
        assert_eq!(
            Orientation::Rotate180.display_dimensions(100, 200),
            (100, 200)
        );
        assert_eq!(
            Orientation::Rotate270.display_dimensions(100, 200),
            (200, 100)
        );
    }

    #[test]
    fn exif_roundtrip() {
        for v in 1..=8u16 {
            let o = Orientation::from_exif(v);
            assert_eq!(o.exif_value(), v);
        }
    }

    #[test]
    fn identity() {
        assert!(Orientation::Normal.is_identity());
        assert!(!Orientation::Rotate90.is_identity());
    }

    #[test]
    fn default_is_normal() {
        assert_eq!(Orientation::default(), Orientation::Normal);
    }

    #[test]
    fn orientation_hint_default_is_preserve() {
        assert_eq!(OrientationHint::default(), OrientationHint::Preserve);
    }

    #[test]
    fn orientation_hint_variants() {
        let _ = OrientationHint::Preserve;
        let _ = OrientationHint::Correct;
        let _ = OrientationHint::CorrectAndTransform(Orientation::Rotate90);
        let _ = OrientationHint::ExactTransform(Orientation::Rotate180);
    }
}
