//! EXIF orientation support.
//!
//! [`Orientation`] is re-exported from [`zenpixels`] — the canonical
//! definition for the zen ecosystem. [`OrientationHint`] is defined here
//! as codec-layer policy for how decoders should handle orientation.

pub use zenpixels::Orientation;

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

    /// Resolve EXIF/container orientation to [`Identity`](Orientation::Identity).
    ///
    /// The decoder coalesces this with the decode operation when possible
    /// (e.g., JPEG lossless DCT transform). The output `ImageInfo` will
    /// report `Orientation::Identity`.
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

impl OrientationHint {
    /// Whether the decoder transforms the decoded pixels (`true`) or leaves
    /// them in their stored orientation (`false`).
    ///
    /// [`Preserve`](Self::Preserve) is the only hint that leaves the pixels
    /// untouched: the decoder then reports the image's intrinsic orientation on
    /// [`ImageInfo::orientation`](crate::ImageInfo) alongside the stored
    /// dimensions, and the caller is responsible for applying it (e.g. via
    /// [`ImageInfo::display_width`](crate::ImageInfo::display_width)). Every
    /// other hint puts the decoder on the "bake" path: it transforms the pixels
    /// and reports the resulting orientation ([`Identity`](Orientation::Identity)
    /// for [`Correct`](Self::Correct)) with the display dimensions.
    ///
    /// This is the gate a codec uses to choose between the preserve path
    /// (stored dims + intrinsic tag) and the bake path (display dims + applied
    /// orientation). It deliberately does **not** say *which* transform to
    /// apply — for [`Correct`](Self::Correct),
    /// [`CorrectAndTransform`](Self::CorrectAndTransform), and
    /// [`ExactTransform`](Self::ExactTransform) the net [`Orientation`] depends
    /// on the image's intrinsic orientation and must be resolved separately.
    /// The resolved transform may itself be [`Identity`](Orientation::Identity)
    /// (e.g. `Correct` on an already-upright image), in which case the bake is a
    /// no-op but the reported orientation is still `Identity` — so a codec
    /// should drive its *reported* `ImageInfo` from the resolved orientation,
    /// not from `bakes()` alone.
    #[must_use]
    pub const fn bakes(self) -> bool {
        !matches!(self, Self::Preserve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn bakes_only_false_for_preserve() {
        assert!(!OrientationHint::Preserve.bakes());
        assert!(OrientationHint::Correct.bakes());
        assert!(OrientationHint::CorrectAndTransform(Orientation::Rotate90).bakes());
        assert!(OrientationHint::ExactTransform(Orientation::Rotate180).bakes());
        // The default (Preserve) must not bake — this is the ecosystem default.
        assert!(!OrientationHint::default().bakes());
    }
}
