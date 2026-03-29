//! Codec implementation helpers.
//!
//! Free functions that codec crates use internally to implement trait methods.
//! These are not part of the consumer-facing API — they exist so codecs don't
//! have to duplicate boilerplate for common patterns.

use alloc::borrow::Cow;

use enough::Stop;
use zenpixels::{Cicp, PixelDescriptor, PixelFormat};

use crate::cost::OutputInfo;
use crate::decode::SourceColor;
use crate::sink::SinkError;
use crate::traits::{AnimationFrameDecoder, Decode, DecodeJob};

/// Implement `push_decoder` by doing a full decode and copying rows to the sink.
///
/// Most codecs that don't have a native streaming decode path can use this to
/// implement [`DecodeJob::push_decoder`] trivially:
///
/// ```rust,ignore
/// fn push_decoder(
///     self,
///     data: Cow<'a, [u8]>,
///     sink: &mut dyn DecodeRowSink,
///     preferred: &[PixelDescriptor],
/// ) -> Result<OutputInfo, Self::Error> {
///     zencodec::helpers::copy_decode_to_sink(self, data, sink, preferred, MyError::from_sink)
/// }
/// ```
pub fn copy_decode_to_sink<'a, J>(
    job: J,
    data: Cow<'a, [u8]>,
    sink: &mut dyn crate::DecodeRowSink,
    preferred: &[PixelDescriptor],
    wrap_sink_error: fn(SinkError) -> J::Error,
) -> Result<OutputInfo, J::Error>
where
    J: DecodeJob<'a>,
{
    let dec = job.decoder(data, preferred)?;
    let output = dec.decode()?;
    let ps = output.pixels();
    let desc = ps.descriptor();
    let w = ps.width();
    let h = ps.rows();

    sink.begin(w, h, desc).map_err(wrap_sink_error)?;

    let mut dst = sink
        .provide_next_buffer(0, h, w, desc)
        .map_err(wrap_sink_error)?;
    for row in 0..h {
        dst.row_mut(row).copy_from_slice(ps.row(row));
    }
    drop(dst);

    sink.finish().map_err(wrap_sink_error)?;

    let info = output.info();
    Ok(OutputInfo::full_decode(info.width, info.height, desc))
}

/// Implement `render_next_frame_to_sink` by rendering a frame and copying rows.
///
/// Codecs that implement [`AnimationFrameDecoder`] can use this to implement
/// `render_next_frame_to_sink` without duplicating the row-copy logic:
///
/// ```rust,ignore
/// fn render_next_frame_to_sink(
///     &mut self,
///     stop: Option<&dyn Stop>,
///     sink: &mut dyn DecodeRowSink,
/// ) -> Result<Option<OutputInfo>, Self::Error> {
///     zencodec::helpers::copy_frame_to_sink(self, stop, sink)
/// }
/// ```
pub fn copy_frame_to_sink<D: AnimationFrameDecoder>(
    decoder: &mut D,
    stop: Option<&dyn Stop>,
    sink: &mut dyn crate::DecodeRowSink,
) -> Result<Option<OutputInfo>, D::Error> {
    let frame = match decoder.render_next_frame(stop)? {
        Some(f) => f,
        None => return Ok(None),
    };
    let ps = frame.pixels();
    let desc = ps.descriptor();
    let w = ps.width();
    let h = ps.rows();

    sink.begin(w, h, desc).map_err(D::wrap_sink_error)?;
    let mut dst = sink
        .provide_next_buffer(0, h, w, desc)
        .map_err(D::wrap_sink_error)?;
    for row in 0..h {
        dst.row_mut(row).copy_from_slice(ps.row(row));
    }
    drop(dst);
    sink.finish().map_err(D::wrap_sink_error)?;

    Ok(Some(OutputInfo::full_decode(w, h, desc)))
}

// ── Pixel descriptor derivation ────────────────────────────────────────────

/// Derive a [`PixelDescriptor`] that accurately describes decoded pixel data.
///
/// Codecs should call this when building `DecodeOutput` or `OutputInfo` to
/// ensure the descriptor's transfer function and color primaries match the
/// actual pixel values — not a hardcoded sRGB assumption.
///
/// # Priority
///
/// 1. If `corrected_to` is `Some`, the pixels were color-managed to that
///    target during decode. The descriptor reflects the target.
/// 2. If `source_color` has CICP metadata, the descriptor uses the CICP
///    transfer function and primaries (pixels are in the source color space).
/// 3. If `source_color` has an ICC profile, the hash is checked against 22
///    known sRGB profiles. A match yields sRGB; a miss yields `Unknown`
///    transfer and primaries (honest: the pixels are in an ICC-described
///    space that doesn't map to a known CICP).
/// 4. No color metadata at all: assumes sRGB (legacy format convention).
pub fn descriptor_for_decoded_pixels(
    format: PixelFormat,
    source_color: &SourceColor,
    corrected_to: Option<&Cicp>,
) -> PixelDescriptor {
    if let Some(target) = corrected_to {
        return target.to_descriptor(format);
    }

    if let Some(cicp) = source_color.cicp {
        return cicp.to_descriptor(format);
    }

    if let Some(ref icc) = source_color.icc_profile {
        if icc_profile_is_srgb(icc) {
            return Cicp::SRGB.to_descriptor(format);
        }
        // Unknown ICC profile — can't map to CICP without a full CMS parse.
        // Be honest: Unknown transfer/primaries.
        return Cicp::SRGB
            .to_descriptor(format)
            .with_transfer(zenpixels::TransferFunction::Unknown)
            .with_primaries(zenpixels::ColorPrimaries::Unknown);
    }

    // No color metadata — assume sRGB (web/browser default).
    Cicp::SRGB.to_descriptor(format)
}

// ── sRGB ICC hash detection ────────────────────────────────────────────────

/// FNV-1a 64-bit hash. Deterministic across all platforms.
const fn fnv1a_64(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    let mut i = 0;
    while i < data.len() {
        hash ^= data[i] as u64;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

/// Known sRGB ICC profile FNV-1a 64-bit hashes, sorted for binary search.
///
/// Covers canonical profiles from ICC, HP/Lino, Apple, Google, Facebook,
/// lcms, Ghostscript/Artifex, libjxl Compact-ICC, and ICC.org v4/v5.
const KNOWN_SRGB_HASHES: [u64; 22] = {
    let h = [
        0x01b2_7967_14a9_5fd5, // sRGB_lcms (656 B)
        0x038b_a989_75d3_6160, // sRGB_LUT — Google Android (2,624 B)
        0x131b_e18b_256c_1005, // sRGB_black_scaled (3,048 B)
        0x190f_0cbe_0744_3404, // sRGB2014 — ICC official (3,024 B)
        0x1b89_293e_8c83_89ad, // colord sRGB — freedesktop/colord (20,420 B)
        0x203c_34c1_fba5_38d2, // sRGB_ICC_v4_Appearance (63,868 B)
        0x43f7_b099_aa77_a523, // Artifex sRGB — Ghostscript (2,576 B)
        0x4b41_6441_92da_c35c, // sRGB_v4_ICC_preference (60,960 B)
        0x569a_1a2b_b183_597a, // Kodak sRGB / KCMS (150,368 B)
        0x56d2_cbfc_a6b5_4318, // sRGB IEC61966-2.1 — HP/Lino (3,144 B)
        0x70d6_01da_f84f_28ff, // Compact-ICC sRGB-v4 (480 B)
        0x7271_2df1_0196_b1db, // Compact-ICC sRGB-v2-micro (456 B)
        0x78cb_2b5d_cdf4_e965, // Compact-ICC sRGB-v2-magic (736 B)
        0x7f3b_a380_1001_a58b, // sRGB_D65_MAT — ICC v5 (24,708 B)
        0x869a_3fee_fd88_a489, // sRGB_ICC_v4_beta (63,928 B)
        0x9b9c_0685_797a_bfdb, // sRGB_ISO22028 — ICC v5 (692 B)
        0xb5fe_02fb_0e03_d19b, // sRGB Facebook (524 B)
        0xbd30_9056_9601_1a32, // Artifex esRGB (12,840 B)
        0xc54d_44a1_49a7_d61a, // Compact-ICC sRGB-v2-nano (410 B)
        0xca3e_5c85_c24b_4889, // sRGB_D65_colorimetric — ICC v5 (24,728 B)
        0xcd42_2ac4_b90b_32b3, // sRGB IEC61966-2.1 — HP/Lino large (7,261 B)
        0xe8a3_3e37_d747_9a46, // sRGB_parametric — Google Android (596 B)
    ];
    // Compile-time assertion: array is sorted
    let mut i = 1;
    while i < h.len() {
        assert!(h[i - 1] < h[i], "KNOWN_SRGB_HASHES must be sorted");
        i += 1;
    }
    h
};

/// Check if an ICC profile is a known sRGB profile by hash lookup.
///
/// Computes a FNV-1a 64-bit hash of the full profile bytes and checks
/// against a table of 22 known sRGB ICC profiles from ICC, HP, Apple,
/// Google, Facebook, lcms, Ghostscript, and libjxl Compact-ICC.
///
/// This is a fast-path check (~50-100ns) that catches the vast majority
/// of real-world sRGB images. Returns `false` for unrecognized profiles —
/// use structural analysis (primaries/TRC comparison) for the long tail.
pub fn icc_profile_is_srgb(icc_bytes: &[u8]) -> bool {
    let hash = fnv1a_64(icc_bytes);
    KNOWN_SRGB_HASHES.binary_search(&hash).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;
    use zenpixels::{AlphaMode, Cicp, ColorPrimaries, SignalRange, TransferFunction};

    // ── Priority 4: no metadata → sRGB assumption ──────────────────────

    #[test]
    fn no_metadata_assumes_srgb() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn no_metadata_gray_assumes_srgb() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::Gray8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
        assert_eq!(desc.pixel_format(), PixelFormat::Gray8);
    }

    #[test]
    fn no_metadata_rgba_assumes_srgb() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgba8);
        assert_eq!(desc.alpha(), Some(AlphaMode::Straight));
    }

    #[test]
    fn no_metadata_f32_assumes_srgb() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::RgbF32, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
        assert_eq!(desc.pixel_format(), PixelFormat::RgbF32);
    }

    // ── Priority 2: CICP metadata ──────────────────────────────────────

    #[test]
    fn cicp_srgb_sets_srgb_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn cicp_p3_sets_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn cicp_pq_sets_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(PixelFormat::RgbaF32, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
    }

    #[test]
    fn cicp_hlg_sets_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_HLG);
        let desc = descriptor_for_decoded_pixels(PixelFormat::RgbF32, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Hlg);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
    }

    #[test]
    fn cicp_takes_precedence_over_icc() {
        // When both CICP and ICC are present, CICP wins (per AVIF/HEIF spec).
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default()
            .with_cicp(Cicp::DISPLAY_P3)
            .with_icc_profile(fake_icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn cicp_preserves_pixel_format() {
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        for fmt in [
            PixelFormat::Rgb8,
            PixelFormat::Rgba8,
            PixelFormat::Gray8,
            PixelFormat::RgbF32,
            PixelFormat::Bgra8,
        ] {
            let desc = descriptor_for_decoded_pixels(fmt, &sc, None);
            assert_eq!(desc.pixel_format(), fmt, "format mismatch for {fmt:?}");
        }
    }

    // ── Priority 3: ICC profile ────────────────────────────────────────

    #[test]
    fn unknown_icc_yields_unknown_descriptor() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    #[test]
    fn unknown_icc_preserves_format_and_alpha() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![99u8; 128].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);

        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgba8);
        assert_eq!(desc.alpha(), Some(AlphaMode::Straight));
        assert_eq!(desc.signal_range, SignalRange::Full);

        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgb8);
        assert!(desc.alpha().is_none());
    }

    #[test]
    fn unknown_icc_gray_preserves_format() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![42u8; 96].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Gray8, &sc, None);
        assert_eq!(desc.pixel_format(), PixelFormat::Gray8);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    #[test]
    fn empty_icc_yields_unknown() {
        // Edge case: zero-length ICC profile is definitely not a known sRGB profile.
        let empty_icc: Arc<[u8]> = Arc::from(alloc::vec![].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(empty_icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    // ── Priority 1: corrected_to overrides everything ──────────────────

    #[test]
    fn corrected_to_overrides_source_cicp() {
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, Some(&Cicp::SRGB));
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn corrected_to_overrides_unknown_icc() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, Some(&Cicp::SRGB));
        // corrected_to wins over unknown ICC → sRGB
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn corrected_to_overrides_no_metadata() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, Some(&Cicp::SRGB));
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn corrected_to_p3_target() {
        // Unusual but valid: color-corrected to P3 instead of sRGB.
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, Some(&Cicp::DISPLAY_P3));
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn corrected_to_preserves_format() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Bgra8, &sc, Some(&Cicp::SRGB));
        assert_eq!(desc.pixel_format(), PixelFormat::Bgra8);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
    }

    // ── icc_profile_is_srgb ────────────────────────────────────────────

    #[test]
    fn srgb_hash_rejects_empty() {
        assert!(!icc_profile_is_srgb(&[]));
    }

    #[test]
    fn srgb_hash_rejects_garbage() {
        assert!(!icc_profile_is_srgb(&[0u8; 100]));
        assert!(!icc_profile_is_srgb(&[0xFF; 3144])); // same size as HP/Lino sRGB
    }

    #[test]
    fn srgb_hash_rejects_short() {
        assert!(!icc_profile_is_srgb(&[1, 2, 3, 4]));
    }

    #[test]
    fn fnv1a_deterministic() {
        // Same input always produces same hash.
        let data = b"sRGB IEC61966-2.1";
        assert_eq!(fnv1a_64(data), fnv1a_64(data));
    }

    #[test]
    fn fnv1a_distinct_inputs() {
        assert_ne!(fnv1a_64(b"abc"), fnv1a_64(b"abd"));
    }

    #[test]
    fn known_srgb_hashes_sorted() {
        // Redundant with the const assertion, but exercises at runtime too.
        for i in 1..KNOWN_SRGB_HASHES.len() {
            assert!(
                KNOWN_SRGB_HASHES[i - 1] < KNOWN_SRGB_HASHES[i],
                "hash table not sorted at index {i}"
            );
        }
    }

    // ── Signal range ───────────────────────────────────────────────────

    #[test]
    fn all_paths_produce_full_range() {
        // All decode paths should produce full-range descriptors.
        let cases: &[(SourceColor, Option<&Cicp>)] = &[
            (SourceColor::default(), None),
            (SourceColor::default().with_cicp(Cicp::SRGB), None),
            (SourceColor::default().with_cicp(Cicp::DISPLAY_P3), None),
            (SourceColor::default().with_cicp(Cicp::BT2100_PQ), None),
            (SourceColor::default(), Some(&Cicp::SRGB)),
        ];
        for (sc, corrected) in cases {
            let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, sc, *corrected);
            assert_eq!(
                desc.signal_range,
                SignalRange::Full,
                "non-full range for {sc:?} corrected={corrected:?}"
            );
        }
    }
}
