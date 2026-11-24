//! Pixel format negotiation.
//!
//! Provides the shared matching logic for the `preferred: &[ImageSpec]`
//! protocol used by [`DecodeJob::decoder`](crate::DecodeJob::decoder) and
//! related methods.

#[cfg(test)]
use crate::helpers::codec_spec;
use zenpixels::{ImageSpec, SampleLayout};

/// Select the best output pixel format from available options.
///
/// Given a caller's ranked preference list and the formats the decoder can
/// produce for this image, returns the best match. Decoders call this inside
/// their [`DecodeJob::decoder`](crate::decode::DecodeJob::decoder) (and similar)
/// implementations to resolve the `preferred` parameter consistently.
///
/// # Matching strategy
///
/// For each preferred descriptor in priority order:
/// 1. **Exact match** — all fields identical (format, transfer, alpha,
///    primaries, signal range)
/// 2. **Format match** — same [`PixelFormat`] (channel type + layout),
///    ignoring transfer function, alpha mode, primaries, and signal range
///
/// Returns the first match found (the *available* entry, not the *preferred*
/// entry, so the descriptor accurately describes what the decoder produces).
///
/// If no preferred descriptor matches any available format, returns
/// `Some(available[0].clone())` — the decoder's default for this image.
///
/// If `preferred` is empty, returns `Some(available[0].clone())` immediately.
///
/// Returns `None` if `available` is empty.
///
/// # Example
///
/// ```
/// use zencodec::decode::negotiate_pixel_format;
/// use zenpixels::ImageSpec;
///
/// // Caller wants RGBA8, falling back to RGB8
/// let preferred = &[ImageSpec::rgba8_srgb(), ImageSpec::rgb8_srgb()];
///
/// // This image is a JPEG — decoder can only produce RGB8
/// let available = &[ImageSpec::rgb8_srgb()];
///
/// let picked = negotiate_pixel_format(preferred, available).unwrap();
/// assert_eq!(picked, ImageSpec::rgb8_srgb());
/// ```
pub fn negotiate_pixel_format(
    preferred: &[ImageSpec],
    available: &[ImageSpec],
) -> Option<ImageSpec> {
    if available.is_empty() {
        return None;
    }

    for pref in preferred {
        // Tier 1: exact match
        for avail in available {
            if *avail == *pref {
                return Some(avail.clone());
            }
        }
        // Tier 2: same physical pixel format, different color metadata
        for avail in available {
            if avail.layout == pref.layout {
                return Some(avail.clone());
            }
        }
    }

    Some(available[0].clone())
}

/// Select the best encode format for given pixel data.
///
/// Returns the first `supported` descriptor whose [`PixelFormat`] matches
/// `source`, or `None` if no supported format is layout-compatible.
///
/// Encoders can use this to check whether they can accept the caller's
/// pixel data without conversion.
///
/// # Example
///
/// ```
/// use zencodec::encode::best_encode_format;
/// use zenpixels::ImageSpec;
///
/// let source = ImageSpec::rgb8_srgb();
/// let supported = &[ImageSpec::rgb8_srgb(), ImageSpec::rgba8_srgb()];
///
/// assert_eq!(best_encode_format(source, supported), Some(ImageSpec::rgb8_srgb()));
/// ```
pub fn best_encode_format(source: ImageSpec, supported: &[ImageSpec]) -> Option<ImageSpec> {
    // Exact match first
    for s in supported {
        if *s == source {
            return Some(s.clone());
        }
    }
    // Same physical format, different metadata
    for s in supported {
        if s.layout == source.layout {
            return Some(s.clone());
        }
    }
    None
}

/// Check whether a pixel format can be produced by selecting from available
/// formats, considering lossless layout-compatible reinterpretation.
///
/// This is a looser check than [`negotiate_pixel_format`] — it returns `true`
/// if the bytes could be reinterpreted as the target format (same channel type
/// and channel count), regardless of color metadata differences.
pub fn is_format_available(target: SampleLayout, available: &[ImageSpec]) -> bool {
    available.iter().any(|a| a.layout == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_wins() {
        let preferred = &[ImageSpec::rgba8_srgb()];
        let available = &[ImageSpec::rgb8_srgb(), ImageSpec::rgba8_srgb()];
        assert_eq!(
            negotiate_pixel_format(preferred, available),
            Some(ImageSpec::rgba8_srgb())
        );
    }

    #[test]
    fn format_match_ignores_transfer() {
        // Caller asks for sRGB, decoder produces unknown-transfer RGB8
        let preferred = &[ImageSpec::rgb8_srgb()];
        let available = &[codec_spec(SampleLayout::RGB8)]; // Unknown transfer
        let picked = negotiate_pixel_format(preferred, available).unwrap();
        assert_eq!(picked.layout, codec_spec(SampleLayout::RGB8).layout);
        assert_eq!(picked, codec_spec(SampleLayout::RGB8)); // returns the available entry
    }

    #[test]
    fn preference_order_respected() {
        // Caller prefers RGBA8, but only RGB8 is available — skips to second preference
        let preferred = &[ImageSpec::rgba8_srgb(), ImageSpec::rgb8_srgb()];
        let available = &[ImageSpec::rgb8_srgb()];
        assert_eq!(
            negotiate_pixel_format(preferred, available),
            Some(ImageSpec::rgb8_srgb())
        );
    }

    #[test]
    fn fallback_to_first_available() {
        // No preference matches — get the decoder's default
        let preferred = &[ImageSpec::gray8_srgb()];
        let available = &[ImageSpec::rgb8_srgb(), ImageSpec::rgba8_srgb()];
        assert_eq!(
            negotiate_pixel_format(preferred, available),
            Some(ImageSpec::rgb8_srgb())
        );
    }

    #[test]
    fn empty_preferred_uses_default() {
        let available = &[ImageSpec::rgba8_srgb(), ImageSpec::rgb8_srgb()];
        assert_eq!(
            negotiate_pixel_format(&[], available),
            Some(ImageSpec::rgba8_srgb())
        );
    }

    #[test]
    fn first_preference_wins_over_later() {
        // Both preferences are available — first one wins
        let preferred = &[ImageSpec::rgb8_srgb(), ImageSpec::rgba8_srgb()];
        let available = &[ImageSpec::rgba8_srgb(), ImageSpec::rgb8_srgb()];
        assert_eq!(
            negotiate_pixel_format(preferred, available),
            Some(ImageSpec::rgb8_srgb())
        );
    }

    #[test]
    fn empty_available_returns_none() {
        assert_eq!(negotiate_pixel_format(&[ImageSpec::rgb8_srgb()], &[]), None);
    }

    #[test]
    fn best_encode_exact() {
        let supported = &[ImageSpec::rgb8_srgb(), ImageSpec::rgba8_srgb()];
        assert_eq!(
            best_encode_format(ImageSpec::rgba8_srgb(), supported),
            Some(ImageSpec::rgba8_srgb())
        );
    }

    #[test]
    fn best_encode_format_match() {
        let supported = &[codec_spec(SampleLayout::RGB8)]; // unknown transfer
        assert_eq!(
            best_encode_format(ImageSpec::rgb8_srgb(), supported),
            Some(codec_spec(SampleLayout::RGB8))
        );
    }

    #[test]
    fn best_encode_no_match() {
        let supported = &[ImageSpec::rgba8_srgb()];
        assert_eq!(best_encode_format(ImageSpec::gray8_srgb(), supported), None);
    }

    #[test]
    fn is_format_available_found() {
        let available = &[ImageSpec::rgb8_srgb(), ImageSpec::gray8_srgb()];
        assert!(is_format_available(
            ImageSpec::rgb8_srgb().layout,
            available
        ));
    }

    #[test]
    fn is_format_available_not_found() {
        let available = &[ImageSpec::rgb8_srgb()];
        assert!(!is_format_available(
            ImageSpec::rgba8_srgb().layout,
            available
        ));
    }
}
