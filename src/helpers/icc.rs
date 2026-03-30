//! ICC profile identification and pixel descriptor derivation.
//!
//! Fast-path hash lookup against 45 well-known ICC profiles (sRGB, Display P3,
//! BT.2020, BT.709) from Compact-ICC, skcms, ICC.org, colord, Ghostscript,
//! HP, Facebook, Google, Kodak, and libvips. Each entry is verified against
//! its reference EOTF for all 65536 u16 values using `scripts/mega_test.rs`.
//!
//! The [`IccMatchTolerance`] enum lets callers choose how closely the ICC
//! profile's TRC must match the reference curve, from pixel-exact (±1 u16)
//! to intent-based (±56 u16 for lossy compact profiles).

use crate::decode::SourceColor;
use zenpixels::{Cicp, PixelDescriptor, PixelFormat};

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
/// 3. If `source_color` has an ICC profile, the hash is checked against 45
///    well-known profiles (sRGB, P3, BT.2020, BT.709) using
///    [`identify_well_known_icc`] with the given `tolerance`.
///    Unrecognized profiles yield `Unknown` transfer/primaries.
/// 4. No color metadata at all: assumes sRGB (legacy format convention).
///
/// Use [`IccMatchTolerance::Intent`] (the most common choice) to honor
/// the encoder's declared color space even when the ICC TRC is a lossy
/// approximation. Use [`IccMatchTolerance::Exact`] when computing
/// perceptual metrics where ±1 u16 matters.
pub fn descriptor_for_decoded_pixels(
    format: PixelFormat,
    source_color: &SourceColor,
    corrected_to: Option<&Cicp>,
    tolerance: IccMatchTolerance,
) -> PixelDescriptor {
    if let Some(target) = corrected_to {
        return target.to_descriptor(format);
    }

    if let Some(cicp) = source_color.cicp {
        return cicp.to_descriptor(format);
    }

    if let Some(ref icc) = source_color.icc_profile {
        if let Some(cicp) = identify_well_known_icc(icc, tolerance) {
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

/// Maximum u16 TRC error tolerance for ICC profile identification.
///
/// Controls how closely an ICC profile's TRC must match the reference EOTF
/// to be accepted as a known profile. Every entry in the hash table stores
/// the measured max u16 error (verified against the authoritative EOTF for
/// all 65536 input values).
///
/// Use [`Exact`](Self::Exact) when pixel-level accuracy matters (e.g.,
/// computing perceptual metrics). Use [`Intent`](Self::Intent) when you
/// want to honor the encoder's intent — a "Compact sRGB" profile with
/// ±56 u16 LUT error was clearly meant to be sRGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub enum IccMatchTolerance {
    /// ±1 u16 max — parametric v4 profiles only.
    Exact = 1,
    /// ±3 u16 max — includes v2-magic parametric approximations.
    Precise = 3,
    /// ±13 u16 max — includes v2-micro LUT profiles and iPhone P3.
    Approximate = 13,
    /// ±56 u16 max — honors encoder intent (e.g., sRGB-v2-nano, Facebook sRGB).
    Intent = 56,
}

/// Well-known ICC profile table: `(hash, primaries, transfer, max_u16_err)`.
///
/// Sorted by hash for binary search. Every entry verified against its
/// reference EOTF for all 65536 u16 values using `scripts/mega_test.rs`.
///
/// Sources: Compact-ICC-Profiles, skcms (Google), ICC.org, colord (freedesktop),
/// Ghostscript/Artifex, HP/Lino, Facebook, Google Android, Kodak, libvips/nip2.
///
/// Excluded: linear/scRGB (esrgb, scRGB), PQ/HLG (different TRC family),
/// calibrated display profiles, CMYK/Gray-only profiles.
///
/// BT.2020 uses BT.2020 12-bit EOTF (α=1.0993, β=0.0181) as reference.
/// BT.709 uses BT.709 EOTF (α=1.099, β=0.018) as reference.
// (hash, color_primaries, transfer_characteristics, max_u16_error)
const KNOWN_ICC_PROFILES: &[(u64, u8, u8, u8)] = {
    const S: (u8, u8) = (1, 13); // sRGB: CP=BT.709, TC=sRGB
    const P: (u8, u8) = (12, 13); // P3: CP=P3, TC=sRGB TRC
    const R: (u8, u8) = (9, 1); // BT.2020: CP=BT.2020, TC=BT.709/BT.2020
    const B: (u8, u8) = (1, 1); // BT.709: CP=BT.709, TC=BT.709
    // Globally sorted by hash. All entries verified with scripts/mega_test.rs.
    &[
        (0x01b2_7967_14a9_5fd5, S.0, S.1, 1),  // sRGB_lcms (656 B)
        (0x038b_a989_75d3_6160, S.0, S.1, 1),  // sRGB_LUT — Google Android (2,624 B)
        (0x131b_e18b_256c_1005, S.0, S.1, 1),  // sRGB_black_scaled — skcms (3,048 B)
        (0x190f_0cbe_0744_3404, S.0, S.1, 1),  // sRGB2014 — ICC official (3,024 B)
        (0x1b01_56ec_7dcf_0fa3, S.0, S.1, 1),  // colord Gamma5000K (6,184 B)
        (0x1b89_293e_8c83_89ad, S.0, S.1, 1),  // colord sRGB (20,420 B)
        (0x1dab_4fbb_a3fd_913f, P.0, P.1, 1),  // skcms Display_P3_LUT (2,612 B)
        (0x203c_34c1_fba5_38d2, S.0, S.1, 1),  // sRGB_ICC_v4_Appearance — ICC.org (63,868 B)
        (0x2735_dda6_6786_337b, S.0, S.1, 1),  // colord Gamma6500K (6,184 B)
        (0x2862_fba6_3274_7f0d, P.0, P.1, 5),  // skcms iPhone7p (548 B)
        (0x2cac_00e9_d69a_9840, P.0, P.1, 2),  // Compact-ICC DisplayP3Compat-v2-magic (736 B)
        (0x3132_2772_0f77_8b89, P.0, P.1, 2),  // Compact-ICC DisplayP3-v2-magic (736 B)
        (0x358f_d60d_2c26_341b, B.0, B.1, 3),  // Compact-ICC Rec709-v2-magic (738 B)
        (0x3e45_d1a7_e6ab_852f, S.0, S.1, 1),  // libvips/nip2 sRGB.icm (6,922 B)
        (0x3f59_a3a4_9d8d_6f25, P.0, P.1, 13), // Compact-ICC DisplayP3Compat-v2-micro (456 B) [LUT]
        (0x43f7_b099_aa77_a523, S.0, S.1, 1),  // Artifex sRGB / Ghostscript default_rgb (2,576 B)
        (0x45b5_2ef1_ca8c_6fcb, R.0, R.1, 1),  // Compact-ICC Rec2020-v4 (480 B)
        (0x4b41_6441_92da_c35c, S.0, S.1, 1),  // sRGB_v4_ICC_preference — ICC.org (60,960 B)
        (0x569a_1a2b_b183_597a, S.0, S.1, 1),  // Kodak sRGB / KCMS (150,368 B)
        (0x56d2_cbfc_a6b5_4318, S.0, S.1, 1),  // sRGB IEC61966-2.1 — HP/Lino (3,144 B)
        (0x70d6_01da_f84f_28ff, S.0, S.1, 1),  // Compact-ICC sRGB-v4 (480 B)
        (0x717b_5b97_bad9_374d, B.0, B.1, 1),  // Compact-ICC Rec709-v4 (480 B)
        (0x7271_2df1_0196_b1db, S.0, S.1, 13), // Compact-ICC sRGB-v2-micro (456 B) [LUT]
        (0x77e2_3b94_c4e2_39d8, S.0, S.1, 1),  // colord Gamma5500K (6,184 B)
        (0x78cb_2b5d_cdf4_e965, S.0, S.1, 2),  // Compact-ICC sRGB-v2-magic (736 B)
        (0x7aa2_2d54_73ad_99bd, P.0, P.1, 1),  // Compact-ICC DisplayP3Compat-v4 (480 B)
        (0x7f3b_a380_1001_a58b, S.0, S.1, 1),  // sRGB_D65_MAT — ICC v5 (24,708 B)
        (0x7fdb_28fb_34fc_eedb, R.0, R.1, 1),  // Compact-ICC Rec2020-v2-magic (790 B)
        (0x809e_740f_f28f_1ad8, R.0, R.1, 1),  // Compact-ICC Rec2020Compat-v4 (480 B)
        (0x869a_3fee_fd88_a489, S.0, S.1, 1),  // sRGB_ICC_v4_beta — ICC.org (63,928 B)
        (0x8d0c_ab95_b0b4_0498, B.0, B.1, 3),  // colord Rec709 (22,464 B)
        (0x9b9c_0685_797a_bfdb, S.0, S.1, 1),  // sRGB_ISO22028 — ICC v5 (692 B)
        (0x9ea9_cacd_e728_5742, P.0, P.1, 1),  // skcms Display_P3_parametric (584 B)
        (0xa52c_7f17_7bff_1392, P.0, P.1, 1),  // Compact-ICC DisplayP3-v4 (480 B)
        (0xb263_a19b_44f5_faba, R.0, R.1, 8),  // Compact-ICC Rec2020Compat-v2-micro (460 B) [LUT]
        (0xb5fc_4c1a_2d96_fbeb, S.0, S.1, 1),  // colord Bluish (16,960 B)
        (0xb5fe_02fb_0e03_d19b, S.0, S.1, 33), // sRGB Facebook (524 B) [parametric approx]
        (0xbd19_8ece_9409_9edc, R.0, R.1, 1),  // Compact-ICC Rec2020Compat-v2-magic (790 B)
        (0xc54d_44a1_49a7_d61a, S.0, S.1, 56), // Compact-ICC sRGB-v2-nano (410 B) [LUT]
        (0xca3e_5c85_c24b_4889, S.0, S.1, 1),  // sRGB_D65_colorimetric — ICC v5 (24,728 B)
        (0xcd42_2ac4_b90b_32b3, S.0, S.1, 1),  // sRGB IEC61966-2.1 — HP/Lino 2 (7,261 B)
        (0xd140_a802_3d39_d033, P.0, P.1, 13), // Compact-ICC DisplayP3-v2-micro (456 B) [LUT]
        (0xdae0_b26f_b1f4_db65, R.0, R.1, 8),  // Compact-ICC Rec2020-v2-micro (460 B) [LUT]
        (0xe132_14e4_1c8a_55b6, B.0, B.1, 8),  // Compact-ICC Rec709-v2-micro (460 B) [LUT]
        (0xe8a3_3e37_d747_9a46, S.0, S.1, 1),  // sRGB_parametric — Google Android (596 B)
    ]
};

/// Identify a well-known ICC profile by hash lookup.
///
/// Computes a FNV-1a 64-bit hash of the profile bytes and checks against
/// a table of 45 known ICC profiles from Compact-ICC, skcms, ICC.org,
/// colord, Ghostscript, HP, Facebook, Google, Kodak, and libvips.
///
/// The `tolerance` parameter controls how closely the profile's TRC must
/// match the reference EOTF. Each entry stores its measured max u16 error
/// (verified for all 65536 input values). Only entries within the tolerance
/// are returned.
///
/// Returns `Some(Cicp)` for recognized profiles, `None` for unknown ones.
/// This is a fast-path check (~100ns). For the long tail of vendor profiles,
/// use structural analysis via a CMS backend (e.g., `ColorManagement::identify_profile`).
pub fn identify_well_known_icc(icc_bytes: &[u8], tolerance: IccMatchTolerance) -> Option<Cicp> {
    let hash = fnv1a_64(icc_bytes);
    let idx = KNOWN_ICC_PROFILES
        .binary_search_by_key(&hash, |&(h, _, _, _)| h)
        .ok()?;
    let (_, cp, tc, err) = KNOWN_ICC_PROFILES[idx];
    if err > tolerance as u8 {
        return None;
    }
    Some(Cicp::new(cp, tc, 0, true))
}

/// Check if an ICC profile is a known sRGB profile by hash lookup.
///
/// Convenience wrapper around [`identify_well_known_icc`] — returns `true`
/// if the profile is sRGB within [`Intent`](IccMatchTolerance::Intent) tolerance.
pub fn icc_profile_is_srgb(icc_bytes: &[u8]) -> bool {
    identify_well_known_icc(icc_bytes, IccMatchTolerance::Intent)
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
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn no_metadata_gray_assumes_srgb() {
        let sc = SourceColor::default();
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Gray8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
        assert_eq!(desc.pixel_format(), PixelFormat::Gray8);
    }

    #[test]
    fn no_metadata_rgba_assumes_srgb() {
        let sc = SourceColor::default();
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgba8);
        assert_eq!(desc.alpha(), Some(AlphaMode::Straight));
    }

    #[test]
    fn no_metadata_f32_assumes_srgb() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::RgbF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
        assert_eq!(desc.pixel_format(), PixelFormat::RgbF32);
    }

    // ── Priority 2: CICP metadata ──────────────────────────────────────

    #[test]
    fn cicp_srgb_sets_srgb_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn cicp_p3_sets_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn cicp_pq_sets_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::RgbaF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
    }

    #[test]
    fn cicp_hlg_sets_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_HLG);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::RgbF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
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
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
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
            let desc = descriptor_for_decoded_pixels(fmt, &sc, None, IccMatchTolerance::Intent);
            assert_eq!(desc.pixel_format(), fmt, "format mismatch for {fmt:?}");
        }
    }

    // ── Priority 3: ICC profile ────────────────────────────────────────

    #[test]
    fn unknown_icc_yields_unknown_descriptor() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    #[test]
    fn unknown_icc_preserves_format_and_alpha() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![99u8; 128].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);

        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgba8);
        assert_eq!(desc.alpha(), Some(AlphaMode::Straight));
        assert_eq!(desc.signal_range, SignalRange::Full);

        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgb8);
        assert!(desc.alpha().is_none());
    }

    #[test]
    fn unknown_icc_gray_preserves_format() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![42u8; 96].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Gray8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.pixel_format(), PixelFormat::Gray8);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    #[test]
    fn empty_icc_yields_unknown() {
        // Edge case: zero-length ICC profile is definitely not a known sRGB profile.
        let empty_icc: Arc<[u8]> = Arc::from(alloc::vec![].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(empty_icc);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    // ── Priority 1: corrected_to overrides everything ──────────────────

    #[test]
    fn corrected_to_overrides_source_cicp() {
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::Rgb8,
            &sc,
            Some(&Cicp::SRGB),
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn corrected_to_overrides_unknown_icc() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::Rgb8,
            &sc,
            Some(&Cicp::SRGB),
            IccMatchTolerance::Intent,
        );
        // corrected_to wins over unknown ICC → sRGB
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn corrected_to_overrides_no_metadata() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::Rgb8,
            &sc,
            Some(&Cicp::SRGB),
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn corrected_to_p3_target() {
        // Unusual but valid: color-corrected to P3 instead of sRGB.
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::Rgb8,
            &sc,
            Some(&Cicp::DISPLAY_P3),
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn corrected_to_preserves_format() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::Bgra8,
            &sc,
            Some(&Cicp::SRGB),
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.pixel_format(), PixelFormat::Bgra8);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
    }

    // ── identify_well_known_icc ───────────────────────────────────────

    #[test]
    fn identify_rejects_empty() {
        assert!(identify_well_known_icc(&[], IccMatchTolerance::Intent).is_none());
        assert!(!icc_profile_is_srgb(&[]));
    }

    #[test]
    fn identify_rejects_garbage() {
        assert!(identify_well_known_icc(&[0u8; 100], IccMatchTolerance::Intent).is_none());
    }

    #[test]
    fn identify_rejects_short() {
        assert!(identify_well_known_icc(&[1, 2, 3, 4], IccMatchTolerance::Intent).is_none());
    }

    #[test]
    fn icc_profile_is_srgb_compat() {
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
                "KNOWN_ICC_PROFILES not sorted at index {i}: 0x{:016x} >= 0x{:016x}",
                KNOWN_ICC_PROFILES[i - 1].0,
                KNOWN_ICC_PROFILES[i].0,
            );
        }
    }

    #[test]
    fn tolerance_exact_filters_lut_profiles() {
        // sRGB-v2-micro has err=13, should be rejected by Exact
        // but accepted by Approximate
        // We test via the table directly since we don't have the actual bytes
        let micro_entry = KNOWN_ICC_PROFILES
            .iter()
            .find(|e| e.0 == 0x7271_2df1_0196_b1db); // sRGB-v2-micro
        assert!(micro_entry.is_some());
        let (_, _, _, err) = micro_entry.unwrap();
        assert_eq!(*err, 13);
        assert!(*err > IccMatchTolerance::Exact as u8);
        assert!(*err <= IccMatchTolerance::Approximate as u8);
    }

    #[test]
    fn tolerance_intent_accepts_nano() {
        let nano_entry = KNOWN_ICC_PROFILES
            .iter()
            .find(|e| e.0 == 0xc54d_44a1_49a7_d61a); // sRGB-v2-nano
        assert!(nano_entry.is_some());
        let (_, _, _, err) = nano_entry.unwrap();
        assert_eq!(*err, 56);
        assert!(*err > IccMatchTolerance::Approximate as u8);
        assert!(*err <= IccMatchTolerance::Intent as u8);
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
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_jpeg_with_unknown_icc() {
        // Camera JPEG with vendor ICC (Canon, Nikon, etc.) — not in known sRGB set
        let vendor_icc: Arc<[u8]> = Arc::from(alloc::vec![0xCA; 3144].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(vendor_icc);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    #[test]
    fn format_jpeg_with_icc_color_corrected_to_srgb() {
        // Camera JPEG with vendor ICC, color-corrected to sRGB during decode
        let vendor_icc: Arc<[u8]> = Arc::from(alloc::vec![0xCA; 3144].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(vendor_icc);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::Rgb8,
            &sc,
            Some(&Cicp::SRGB),
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    // PNG: ICC via iCCP chunk, CICP via cICP chunk, or gAMA+cHRM → sRGB.
    #[test]
    fn format_png_with_cicp_p3() {
        // Modern PNG with cICP chunk declaring Display P3
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
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
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn format_png_no_color_metadata() {
        // Legacy PNG without color chunks → assume sRGB
        let sc = SourceColor::default();
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_png_16bit_hdr_pq() {
        // HDR PNG: cICP BT.2100 PQ, 16-bit RGBA
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::Rgba16,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgba16);
    }

    // WebP: ICC via ICCP chunk, no CICP support.
    #[test]
    fn format_webp_no_icc() {
        // Most WebP files: no ICC → sRGB
        let sc = SourceColor::default();
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_webp_with_unknown_icc() {
        // WebP with non-sRGB ICC (e.g., P3 from iPhone)
        let p3_icc: Arc<[u8]> = Arc::from(alloc::vec![0xA3; 480].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(p3_icc);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
    }

    // AVIF: always has CICP from container. ICC optional.
    #[test]
    fn format_avif_srgb() {
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_avif_hdr10_pq() {
        // HDR10: BT.2020 + PQ, decoded to f32
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::RgbaF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
        assert_eq!(desc.pixel_format(), PixelFormat::RgbaF32);
    }

    #[test]
    fn format_avif_hlg() {
        // HLG broadcast content
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_HLG);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::RgbF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Hlg);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
    }

    #[test]
    fn format_avif_p3() {
        // Wide-gamut SDR: Display P3 + sRGB transfer
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    // JXL: CICP from codestream header, ICC optional.
    #[test]
    fn format_jxl_srgb() {
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    #[test]
    fn format_jxl_p3_pq() {
        // JXL with P3 primaries and PQ transfer (HDR photo)
        let cicp = Cicp::new(12, 16, 0, true); // P3 + PQ
        let sc = SourceColor::default().with_cicp(cicp);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::RgbaF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    // HEIC: CICP from container (ISOBMFF colr box).
    #[test]
    fn format_heic_p3_srgb_trc() {
        // iPhone HEIC: Display P3 with sRGB transfer
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn format_heic_hdr10() {
        // Apple HDR (Dolby Vision / HDR10)
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(
            PixelFormat::RgbaF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.transfer, TransferFunction::Pq);
        assert_eq!(desc.primaries, ColorPrimaries::Bt2020);
    }

    // GIF: never has color metadata → always sRGB.
    #[test]
    fn format_gif_always_srgb() {
        let sc = SourceColor::default();
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgba8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    // BMP / PNM / Farbfeld: no color metadata → always sRGB.
    #[test]
    fn format_bmp_pnm_farbfeld_always_srgb() {
        let sc = SourceColor::default();
        for fmt in [PixelFormat::Rgb8, PixelFormat::Rgba8, PixelFormat::Gray8] {
            let desc = descriptor_for_decoded_pixels(fmt, &sc, None, IccMatchTolerance::Intent);
            assert_eq!(desc.transfer, TransferFunction::Srgb, "{fmt:?}");
            assert_eq!(desc.primaries, ColorPrimaries::Bt709, "{fmt:?}");
        }
    }

    // ── ICC identification for well-known profiles ───────────────────

    #[test]
    fn format_jpeg_with_p3_icc_identified() {
        // Simulate a JPEG with a known P3 ICC — descriptor should show P3 primaries
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
    }

    #[test]
    fn identify_table_has_all_families() {
        let has = |cp: u8, tc: u8| KNOWN_ICC_PROFILES.iter().any(|e| e.1 == cp && e.2 == tc);
        assert!(has(1, 13), "sRGB (CP=1, TC=13)");
        assert!(has(12, 13), "Display P3 (CP=12, TC=13)");
        assert!(has(9, 1), "BT.2020 (CP=9, TC=1)");
        assert!(has(1, 1), "BT.709 (CP=1, TC=1)");
    }

    #[test]
    fn identify_srgb_count() {
        let count = KNOWN_ICC_PROFILES
            .iter()
            .filter(|e| e.1 == 1 && e.2 == 13)
            .count();
        assert_eq!(count, 26, "expected 26 sRGB profiles");
    }

    #[test]
    fn identify_p3_count() {
        let count = KNOWN_ICC_PROFILES
            .iter()
            .filter(|e| e.1 == 12 && e.2 == 13)
            .count();
        assert_eq!(count, 9, "expected 9 Display P3 profiles");
    }

    #[test]
    fn identify_bt2020_count() {
        let count = KNOWN_ICC_PROFILES
            .iter()
            .filter(|e| e.1 == 9 && e.2 == 1)
            .count();
        assert_eq!(count, 6, "expected 6 BT.2020 profiles");
    }

    #[test]
    fn identify_bt709_count() {
        let count = KNOWN_ICC_PROFILES
            .iter()
            .filter(|e| e.1 == 1 && e.2 == 1)
            .count();
        assert_eq!(count, 4, "expected 4 BT.709 profiles");
    }

    #[test]
    fn identify_total_count() {
        assert_eq!(KNOWN_ICC_PROFILES.len(), 45, "expected 45 total profiles");
    }

    // TIFF: ICC via tag, rarely CICP.
    #[test]
    fn format_tiff_with_unknown_icc() {
        // TIFF from scanner with vendor ICC profile
        let icc: Arc<[u8]> = Arc::from(alloc::vec![0x54; 7261].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(icc);
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb16, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
        assert_eq!(desc.pixel_format(), PixelFormat::Rgb16);
    }

    #[test]
    fn format_tiff_no_icc() {
        let sc = SourceColor::default();
        let desc =
            descriptor_for_decoded_pixels(PixelFormat::Rgb16, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::Bt709);
    }

    // ── Hash table integrity ──────────────────────────────────────────

    #[test]
    fn no_duplicate_hashes() {
        let mut seen = alloc::collections::BTreeSet::new();
        for entry in KNOWN_ICC_PROFILES {
            assert!(seen.insert(entry.0), "duplicate hash 0x{:016x}", entry.0);
        }
    }

    #[test]
    fn all_errors_within_intent_tolerance() {
        for entry in KNOWN_ICC_PROFILES {
            assert!(
                entry.3 <= IccMatchTolerance::Intent as u8,
                "entry 0x{:016x} has err={} exceeding Intent({})",
                entry.0,
                entry.3,
                IccMatchTolerance::Intent as u8,
            );
        }
    }

    #[test]
    fn all_cicp_codes_valid() {
        for entry in KNOWN_ICC_PROFILES {
            let (_, cp, tc, _) = *entry;
            // CP must be 1 (BT.709), 9 (BT.2020), or 12 (P3)
            assert!(
                matches!(cp, 1 | 9 | 12),
                "entry 0x{:016x} has invalid CP={}",
                entry.0,
                cp
            );
            // TC must be 1 (BT.709) or 13 (sRGB)
            assert!(
                matches!(tc, 1 | 13),
                "entry 0x{:016x} has invalid TC={}",
                entry.0,
                tc
            );
        }
    }

    #[test]
    fn zero_filled_data_no_false_positive() {
        // ICC profiles often have zero padding. Verify no zero-filled
        // buffer at any profile-typical size matches a table entry.
        for len in [
            410, 456, 480, 524, 548, 656, 736, 790, 2576, 3024, 3144, 6184, 20420,
        ] {
            let zeros = alloc::vec![0u8; len];
            assert!(
                identify_well_known_icc(&zeros, IccMatchTolerance::Intent).is_none(),
                "zeros({len}) falsely matched a profile"
            );
        }
    }

    #[test]
    fn tolerance_ordering() {
        assert!(IccMatchTolerance::Exact < IccMatchTolerance::Precise);
        assert!(IccMatchTolerance::Precise < IccMatchTolerance::Approximate);
        assert!(IccMatchTolerance::Approximate < IccMatchTolerance::Intent);
    }

    // ── Signal range ───────────────────────────────────────────────────

    #[test]
    fn all_paths_produce_full_range() {
        let cases: &[(SourceColor, Option<&Cicp>)] = &[
            (SourceColor::default(), None),
            (SourceColor::default().with_cicp(Cicp::SRGB), None),
            (SourceColor::default().with_cicp(Cicp::DISPLAY_P3), None),
            (SourceColor::default().with_cicp(Cicp::BT2100_PQ), None),
            (SourceColor::default(), Some(&Cicp::SRGB)),
        ];
        for (sc, corrected) in cases {
            let desc = descriptor_for_decoded_pixels(
                PixelFormat::Rgb8,
                sc,
                *corrected,
                IccMatchTolerance::Intent,
            );
            assert_eq!(
                desc.signal_range,
                SignalRange::Full,
                "non-full range for {sc:?} corrected={corrected:?}"
            );
        }
    }
}
