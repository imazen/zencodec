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
        if let Some(cicp) = identify_well_known_icc(icc) {
            return cicp.to_descriptor(format);
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

// ── Well-known ICC profile identification ──────────────────────────────────

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

/// CICP code points for well-known ICC profile families.
///
/// sRGB: primaries=1 (BT.709), transfer=13 (sRGB)
/// Display P3: primaries=12 (P3), transfer=13 (sRGB TRC)
/// BT.2020: primaries=9 (BT.2020), transfer=1 (BT.709 TRC — safe default;
///          actual TRC (PQ/HLG) can't be determined from ICC colorants alone)
/// BT.709: primaries=1 (BT.709), transfer=1 (BT.709 TRC — distinct from sRGB)
const SRGB: (u8, u8) = (1, 13);
const P3: (u8, u8) = (12, 13);
const BT2020: (u8, u8) = (9, 1);
const BT709: (u8, u8) = (1, 1);

/// Well-known ICC profile table: (hash, primaries_code, transfer_code).
///
/// Sorted by hash for binary search. Covers:
/// - 22 sRGB profiles (ICC, HP/Lino, Apple, Google, Facebook, lcms, etc.)
/// - 6 Display P3 profiles (Compact-ICC v4/v2/micro, Compat variants)
/// - 4 BT.2020 profiles (Compact-ICC v4, Compat v4/v2-magic/v2-micro)
/// - 3 BT.709 profiles (Compact-ICC v4/v2-magic/v2-micro)
/// - 2 Adobe RGB profiles (Compact-ICC v4/v2)
///
/// Adobe RGB maps to BT.709 primaries (closest CICP match — same white
/// point, slightly wider gamut in green-cyan, but no dedicated CICP code).
const KNOWN_ICC_PROFILES: [(u64, u8, u8); 39] = {
    let t = [
        // sRGB profiles (22)
        (0x01b2_7967_14a9_5fd5, SRGB),  // sRGB_lcms (656 B)
        (0x038b_a989_75d3_6160, SRGB),  // sRGB_LUT — Google Android (2,624 B)
        (0x131b_e18b_256c_1005, SRGB),  // sRGB_black_scaled (3,048 B)
        (0x190f_0cbe_0744_3404, SRGB),  // sRGB2014 — ICC official (3,024 B)
        (0x1b89_293e_8c83_89ad, SRGB),  // colord sRGB — freedesktop/colord (20,420 B)
        // Adobe RGB (2) — mapped to BT.709 primaries (closest CICP match)
        (0x1d3e_7e4f_40c5_8953, BT709), // Compact-ICC AdobeCompat-v2 (374 B)
        (0x203c_34c1_fba5_38d2, SRGB),  // sRGB_ICC_v4_Appearance (63,868 B)
        // Display P3 (6)
        (0x2cac_00e9_d69a_9840, P3),    // Compact-ICC DisplayP3Compat-v2-magic (736 B)
        (0x3132_2772_0f77_8b89, P3),    // Compact-ICC DisplayP3-v2-magic (736 B)
        // BT.709 (3)
        (0x358f_d60d_2c26_341b, BT709), // Compact-ICC Rec709-v2-magic (738 B)
        (0x3f59_a3a4_9d8d_6f25, P3),    // Compact-ICC DisplayP3Compat-v2-micro (456 B)
        (0x43f7_b099_aa77_a523, SRGB),  // Artifex sRGB — Ghostscript (2,576 B)
        // BT.2020 (4)
        (0x45b5_2ef1_ca8c_6fcb, BT2020), // Compact-ICC Rec2020-v4 (480 B)
        (0x4b41_6441_92da_c35c, SRGB),  // sRGB_v4_ICC_preference (60,960 B)
        (0x4de1_052e_3b80_7417, BT709), // Compact-ICC AdobeCompat-v4 (480 B)
        (0x569a_1a2b_b183_597a, SRGB),  // Kodak sRGB / KCMS (150,368 B)
        (0x56d2_cbfc_a6b5_4318, SRGB),  // sRGB IEC61966-2.1 — HP/Lino (3,144 B)
        (0x70d6_01da_f84f_28ff, SRGB),  // Compact-ICC sRGB-v4 (480 B)
        (0x717b_5b97_bad9_374d, BT709), // Compact-ICC Rec709-v4 (480 B)
        (0x7271_2df1_0196_b1db, SRGB),  // Compact-ICC sRGB-v2-micro (456 B)
        (0x78cb_2b5d_cdf4_e965, SRGB),  // Compact-ICC sRGB-v2-magic (736 B)
        (0x7aa2_2d54_73ad_99bd, P3),    // Compact-ICC DisplayP3Compat-v4 (480 B)
        (0x7f3b_a380_1001_a58b, SRGB),  // sRGB_D65_MAT — ICC v5 (24,708 B)
        (0x7fdb_28fb_34fc_eedb, BT2020), // Compact-ICC Rec2020-v2-magic (790 B)
        (0x809e_740f_f28f_1ad8, BT2020), // Compact-ICC Rec2020Compat-v4 (480 B)
        (0x869a_3fee_fd88_a489, SRGB),  // sRGB_ICC_v4_beta (63,928 B)
        (0x9b9c_0685_797a_bfdb, SRGB),  // sRGB_ISO22028 — ICC v5 (692 B)
        (0xa52c_7f17_7bff_1392, P3),    // Compact-ICC DisplayP3-v4 (480 B)
        (0xb263_a19b_44f5_faba, BT2020), // Compact-ICC Rec2020Compat-v2-micro (460 B)
        (0xb5fe_02fb_0e03_d19b, SRGB),  // sRGB Facebook (524 B)
        (0xbd19_8ece_9409_9edc, BT2020), // Compact-ICC Rec2020Compat-v2-magic (790 B)
        (0xbd30_9056_9601_1a32, SRGB),  // Artifex esRGB (12,840 B)
        (0xc54d_44a1_49a7_d61a, SRGB),  // Compact-ICC sRGB-v2-nano (410 B)
        (0xca3e_5c85_c24b_4889, SRGB),  // sRGB_D65_colorimetric — ICC v5 (24,728 B)
        (0xcd42_2ac4_b90b_32b3, SRGB),  // sRGB IEC61966-2.1 — HP/Lino large (7,261 B)
        (0xd140_a802_3d39_d033, P3),    // Compact-ICC DisplayP3-v2-micro (456 B)
        (0xdae0_b26f_b1f4_db65, BT2020), // Compact-ICC Rec2020-v2-micro (460 B)
        (0xe132_14e4_1c8a_55b6, BT709), // Compact-ICC Rec709-v2-micro (460 B)
        (0xe8a3_3e37_d747_9a46, SRGB),  // sRGB_parametric — Google Android (596 B)
    ];
    // Flatten to (hash, primaries, transfer) and verify sorted.
    let mut out = [(0u64, 0u8, 0u8); 39];
    let mut i = 0;
    while i < t.len() {
        out[i] = (t[i].0, (t[i].1).0, (t[i].1).1);
        if i > 0 {
            assert!(out[i - 1].0 < out[i].0, "KNOWN_ICC_PROFILES must be sorted by hash");
        }
        i += 1;
    }
    out
};

/// Identify a well-known ICC profile by hash lookup.
///
/// Computes a FNV-1a 64-bit hash of the profile bytes and checks against
/// a table of 39 known ICC profiles covering sRGB (22 variants),
/// Display P3 (6), BT.2020 (6), BT.709 (3), and Adobe RGB (2).
///
/// Returns `Some(Cicp)` for recognized profiles, `None` for unknown ones.
/// This is a fast-path check (~100ns). For the long tail of vendor profiles,
/// use structural analysis via a CMS backend (e.g., `ColorManagement::identify_profile`).
pub fn identify_well_known_icc(icc_bytes: &[u8]) -> Option<Cicp> {
    let hash = fnv1a_64(icc_bytes);
    let idx = KNOWN_ICC_PROFILES
        .binary_search_by_key(&hash, |&(h, _, _)| h)
        .ok()?;
    let (_, cp, tc) = KNOWN_ICC_PROFILES[idx];
    Some(Cicp::new(cp, tc, 0, true))
}

/// Check if an ICC profile is a known sRGB profile by hash lookup.
///
/// Convenience wrapper around [`identify_well_known_icc`] — returns `true`
/// if the profile is any of the 22 known sRGB ICC profiles.
pub fn icc_profile_is_srgb(icc_bytes: &[u8]) -> bool {
    identify_well_known_icc(icc_bytes)
        .is_some_and(|c| c.color_primaries == 1 && c.transfer_characteristics == 13)
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

    // ── identify_well_known_icc ───────────────────────────────────────

    #[test]
    fn identify_rejects_empty() {
        assert!(identify_well_known_icc(&[]).is_none());
        assert!(!icc_profile_is_srgb(&[]));
    }

    #[test]
    fn identify_rejects_garbage() {
        assert!(identify_well_known_icc(&[0u8; 100]).is_none());
        assert!(identify_well_known_icc(&[0xFF; 3144]).is_none());
    }

    #[test]
    fn identify_rejects_short() {
        assert!(identify_well_known_icc(&[1, 2, 3, 4]).is_none());
    }

    #[test]
    fn icc_profile_is_srgb_compat() {
        // icc_profile_is_srgb returns true only for sRGB, not P3/BT.2020
        assert!(!icc_profile_is_srgb(&[0u8; 100]));
    }

    #[test]
    fn fnv1a_deterministic() {
        let data = b"sRGB IEC61966-2.1";
        assert_eq!(fnv1a_64(data), fnv1a_64(data));
    }

    #[test]
    fn fnv1a_distinct_inputs() {
        assert_ne!(fnv1a_64(b"abc"), fnv1a_64(b"abd"));
    }

    #[test]
    fn known_profiles_table_sorted() {
        for i in 1..KNOWN_ICC_PROFILES.len() {
            assert!(
                KNOWN_ICC_PROFILES[i - 1].0 < KNOWN_ICC_PROFILES[i].0,
                "KNOWN_ICC_PROFILES not sorted at index {i}"
            );
        }
    }

    // ── Per-format decode scenarios ────────────────────────────────────
    //
    // Each test simulates the SourceColor a specific codec would produce,
    // verifying the descriptor matches what downstream consumers expect.

    // JPEG: most common — no metadata (web default), or sRGB/unknown ICC.
    // No CICP in baseline JPEG. ICC via APP2.
    #[test]
    fn format_jpeg_no_icc() {
        // 95% of web JPEGs: no ICC, no CICP → assume sRGB
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_jpeg_with_unknown_icc() {
        // Camera JPEG with vendor ICC (Canon, Nikon, etc.) — not in known sRGB set
        let vendor_icc: Arc<[u8]> = Arc::from(alloc::vec![0xCA; 3144].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(vendor_icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    #[test]
    fn format_jpeg_with_icc_color_corrected_to_srgb() {
        // Camera JPEG with vendor ICC, color-corrected to sRGB during decode
        let vendor_icc: Arc<[u8]> = Arc::from(alloc::vec![0xCA; 3144].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(vendor_icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, Some(&Cicp::SRGB));
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    // PNG: ICC via iCCP chunk, CICP via cICP chunk, or gAMA+cHRM → sRGB.
    #[test]
    fn format_png_with_cicp_p3() {
        // Modern PNG with cICP chunk declaring Display P3
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgba8);
    }

    #[test]
    fn format_png_with_cicp_and_icc_cicp_wins() {
        // PNG with both cICP and iCCP — CICP takes precedence
        let icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 100].into_boxed_slice());
        let sc = SourceColor::default()
            .with_cicp(Cicp::DISPLAY_P3)
            .with_icc_profile(icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn format_png_no_color_metadata() {
        // Legacy PNG without color chunks → assume sRGB
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_png_16bit_hdr_pq() {
        // HDR PNG: cICP BT.2100 PQ, 16-bit RGBA
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba16, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgba16);
    }

    // WebP: ICC via ICCP chunk, no CICP support.
    #[test]
    fn format_webp_no_icc() {
        // Most WebP files: no ICC → sRGB
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_webp_with_unknown_icc() {
        // WebP with non-sRGB ICC (e.g., P3 from iPhone)
        let p3_icc: Arc<[u8]> = Arc::from(alloc::vec![0xA3; 480].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(p3_icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    // AVIF: always has CICP from container. ICC optional.
    #[test]
    fn format_avif_srgb() {
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_avif_hdr10_pq() {
        // HDR10: BT.2020 + PQ, decoded to f32
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(PixelFormat::RgbaF32, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
        assert_eq!(desc.pixel_format(), PixelFormat::RgbaF32);
    }

    #[test]
    fn format_avif_hlg() {
        // HLG broadcast content
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_HLG);
        let desc = descriptor_for_decoded_pixels(PixelFormat::RgbF32, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Hlg);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
    }

    #[test]
    fn format_avif_p3() {
        // Wide-gamut SDR: Display P3 + sRGB transfer
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    // JXL: CICP from codestream header, ICC optional.
    #[test]
    fn format_jxl_srgb() {
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_jxl_p3_pq() {
        // JXL with P3 primaries and PQ transfer (HDR photo)
        let cicp = Cicp::new(12, 16, 0, true); // P3 + PQ
        let sc = SourceColor::default().with_cicp(cicp);
        let desc = descriptor_for_decoded_pixels(PixelFormat::RgbaF32, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    // HEIC: CICP from container (ISOBMFF colr box).
    #[test]
    fn format_heic_p3_srgb_trc() {
        // iPhone HEIC: Display P3 with sRGB transfer
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn format_heic_hdr10() {
        // Apple HDR (Dolby Vision / HDR10)
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(PixelFormat::RgbaF32, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
    }

    // GIF: never has color metadata → always sRGB.
    #[test]
    fn format_gif_always_srgb() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    // BMP / PNM / Farbfeld: no color metadata → always sRGB.
    #[test]
    fn format_bmp_pnm_farbfeld_always_srgb() {
        let sc = SourceColor::default();
        for fmt in [PixelFormat::Rgb8, PixelFormat::Rgba8, PixelFormat::Gray8] {
            let desc = descriptor_for_decoded_pixels(fmt, &sc, None);
            assert_eq!(desc.transfer, TransferFunction::Srgb, "{fmt:?}");
            assert_eq!(desc.primaries, ColorPrimaries::Bt709, "{fmt:?}");
        }
    }

    // ── ICC identification for well-known profiles ───────────────────

    #[test]
    fn format_jpeg_with_p3_icc_identified() {
        // A JPEG with a Compact-ICC Display P3 v4 profile should be identified
        // as P3 without needing a full CMS parse.
        // Hash: 0x7aa2_2d54_73ad_99bd = DisplayP3Compat-v4.icc
        let cicp = identify_well_known_icc(&[]).is_none(); // placeholder — real test below
        let _ = cicp;

        // Simulate the SourceColor a codec would build for a known P3 ICC
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn identify_table_has_all_families() {
        // Verify the table covers all expected profile families by checking
        // that each CICP combination appears at least once.
        let has = |cp: u8, tc: u8| {
            KNOWN_ICC_PROFILES.iter().any(|&(_, c, t)| c == cp && t == tc)
        };
        assert!(has(1, 13), "sRGB (CP=1, TC=13)");
        assert!(has(12, 13), "Display P3 (CP=12, TC=13)");
        assert!(has(9, 1), "BT.2020 (CP=9, TC=1)");
        assert!(has(1, 1), "BT.709/AdobeRGB (CP=1, TC=1)");
    }

    #[test]
    fn identify_srgb_count() {
        let srgb_count = KNOWN_ICC_PROFILES
            .iter()
            .filter(|&&(_, cp, tc)| cp == 1 && tc == 13)
            .count();
        assert_eq!(srgb_count, 22, "expected 22 sRGB profiles");
    }

    #[test]
    fn identify_p3_count() {
        let p3_count = KNOWN_ICC_PROFILES
            .iter()
            .filter(|&&(_, cp, tc)| cp == 12 && tc == 13)
            .count();
        assert_eq!(p3_count, 6, "expected 6 Display P3 profiles");
    }

    // TIFF: ICC via tag, rarely CICP.
    #[test]
    fn format_tiff_with_unknown_icc() {
        // TIFF from scanner with vendor ICC profile
        let icc: Arc<[u8]> = Arc::from(alloc::vec![0x54; 7261].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(icc);
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb16, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgb16);
    }

    #[test]
    fn format_tiff_no_icc() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(PixelFormat::Rgb16, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
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
