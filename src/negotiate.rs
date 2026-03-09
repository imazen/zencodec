//! Pixel format negotiation.
//!
//! Provides the shared matching logic for the `preferred: &[PixelDescriptor]`
//! protocol used by [`DecodeJob::decoder`](crate::DecodeJob::decoder) and
//! related methods.

use zenpixels::{PixelDescriptor, PixelFormat};

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
/// `Some(available[0])` — the decoder's default for this image.
///
/// If `preferred` is empty, returns `Some(available[0])` immediately.
///
/// Returns `None` if `available` is empty.
///
/// # Example
///
/// ```
/// use zencodec::decode::negotiate_pixel_format;
/// use zenpixels::PixelDescriptor;
///
/// // Caller wants RGBA8, falling back to RGB8
/// let preferred = &[PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGB8_SRGB];
///
/// // This image is a JPEG — decoder can only produce RGB8
/// let available = &[PixelDescriptor::RGB8_SRGB];
///
/// let picked = negotiate_pixel_format(preferred, available).unwrap();
/// assert_eq!(picked, PixelDescriptor::RGB8_SRGB);
/// ```
pub fn negotiate_pixel_format(
    preferred: &[PixelDescriptor],
    available: &[PixelDescriptor],
) -> Option<PixelDescriptor> {
    if available.is_empty() {
        return None;
    }

    for pref in preferred {
        // Tier 1: exact match
        for avail in available {
            if *avail == *pref {
                return Some(*avail);
            }
        }
        // Tier 2: same physical pixel format, different color metadata
        for avail in available {
            if avail.pixel_format() == pref.pixel_format() {
                return Some(*avail);
            }
        }
    }

    Some(available[0])
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
/// use zenpixels::PixelDescriptor;
///
/// let source = PixelDescriptor::RGB8_SRGB;
/// let supported = &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB];
///
/// assert_eq!(best_encode_format(source, supported), Some(PixelDescriptor::RGB8_SRGB));
/// ```
pub fn best_encode_format(
    source: PixelDescriptor,
    supported: &[PixelDescriptor],
) -> Option<PixelDescriptor> {
    // Exact match first
    for s in supported {
        if *s == source {
            return Some(*s);
        }
    }
    // Same physical format, different metadata
    for s in supported {
        if s.pixel_format() == source.pixel_format() {
            return Some(*s);
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
pub fn is_format_available(target: PixelFormat, available: &[PixelDescriptor]) -> bool {
    available.iter().any(|a| a.pixel_format() == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_wins() {
        let preferred = &[PixelDescriptor::RGBA8_SRGB];
        let available = &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB];
        assert_eq!(
            negotiate_pixel_format(preferred, available),
            Some(PixelDescriptor::RGBA8_SRGB)
        );
    }

    #[test]
    fn format_match_ignores_transfer() {
        // Caller asks for sRGB, decoder produces unknown-transfer RGB8
        let preferred = &[PixelDescriptor::RGB8_SRGB];
        let available = &[PixelDescriptor::RGB8]; // Unknown transfer
        let picked = negotiate_pixel_format(preferred, available).unwrap();
        assert_eq!(picked.pixel_format(), PixelDescriptor::RGB8.pixel_format());
        assert_eq!(picked, PixelDescriptor::RGB8); // returns the available entry
    }

    #[test]
    fn preference_order_respected() {
        // Caller prefers RGBA8, but only RGB8 is available — skips to second preference
        let preferred = &[PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGB8_SRGB];
        let available = &[PixelDescriptor::RGB8_SRGB];
        assert_eq!(
            negotiate_pixel_format(preferred, available),
            Some(PixelDescriptor::RGB8_SRGB)
        );
    }

    #[test]
    fn fallback_to_first_available() {
        // No preference matches — get the decoder's default
        let preferred = &[PixelDescriptor::GRAY8_SRGB];
        let available = &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB];
        assert_eq!(
            negotiate_pixel_format(preferred, available),
            Some(PixelDescriptor::RGB8_SRGB)
        );
    }

    #[test]
    fn empty_preferred_uses_default() {
        let available = &[PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGB8_SRGB];
        assert_eq!(
            negotiate_pixel_format(&[], available),
            Some(PixelDescriptor::RGBA8_SRGB)
        );
    }

    #[test]
    fn first_preference_wins_over_later() {
        // Both preferences are available — first one wins
        let preferred = &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB];
        let available = &[PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGB8_SRGB];
        assert_eq!(
            negotiate_pixel_format(preferred, available),
            Some(PixelDescriptor::RGB8_SRGB)
        );
    }

    #[test]
    fn empty_available_returns_none() {
        assert_eq!(
            negotiate_pixel_format(&[PixelDescriptor::RGB8_SRGB], &[]),
            None
        );
    }

    #[test]
    fn best_encode_exact() {
        let supported = &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB];
        assert_eq!(
            best_encode_format(PixelDescriptor::RGBA8_SRGB, supported),
            Some(PixelDescriptor::RGBA8_SRGB)
        );
    }

    #[test]
    fn best_encode_format_match() {
        let supported = &[PixelDescriptor::RGB8]; // unknown transfer
        assert_eq!(
            best_encode_format(PixelDescriptor::RGB8_SRGB, supported),
            Some(PixelDescriptor::RGB8)
        );
    }

    #[test]
    fn best_encode_no_match() {
        let supported = &[PixelDescriptor::RGBA8_SRGB];
        assert_eq!(
            best_encode_format(PixelDescriptor::GRAY8_SRGB, supported),
            None
        );
    }

    #[test]
    fn is_format_available_found() {
        let available = &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::GRAY8_SRGB];
        assert!(is_format_available(
            PixelDescriptor::RGB8_SRGB.pixel_format(),
            available
        ));
    }

    #[test]
    fn is_format_available_not_found() {
        let available = &[PixelDescriptor::RGB8_SRGB];
        assert!(!is_format_available(
            PixelDescriptor::RGBA8_SRGB.pixel_format(),
            available
        ));
    }
}
