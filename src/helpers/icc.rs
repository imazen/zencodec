//! ICC profile identification and decoded sample declarations.

use crate::decode::SourceColor;
use crate::info::AuthoritativeColor;
use zenpixels::{
    Cicp, ColorSpace, IccColorSpace, ImageSpec, Primaries, RenderingIntent, SampleLayout, Transfer,
};

/// Derive the current image spec for decoded, full-range samples.
///
/// Use [`descriptor_for_decoded_pixels_v2`] to supply a complete current color
/// space when the codec applies color correction.
#[deprecated(since = "0.1.17", note = "use descriptor_for_decoded_pixels_v2")]
#[allow(deprecated)]
pub fn descriptor_for_decoded_pixels(
    layout: SampleLayout,
    source_color: &SourceColor,
    corrected_to: Option<&Cicp>,
    _tolerance: IccMatchTolerance,
) -> ImageSpec {
    let base = super::codec_spec(layout);
    let corrected = corrected_to.map(|cicp| {
        base.clone()
            .with_primaries(
                Primaries::from_cicp(cicp.color_primaries).unwrap_or(Primaries::Unknown),
            )
            .with_transfer(
                Transfer::from_cicp(cicp.transfer_characteristics).unwrap_or(Transfer::Unknown),
            )
            .color
    });
    descriptor_for_decoded_pixels_v2(layout, source_color, corrected.as_ref())
        .unwrap_or_else(|_| super::codec_spec(layout))
}

/// Describe decoded, full-range samples and their current color meaning.
///
/// A codec that color-corrected the samples supplies the complete target
/// color. For source color, a class-compatible ICC remains authoritative;
/// a CICP claim contributes primaries and transfer after any component
/// reconstruction. The codec should build an explicit [`ImageSpec`] when
/// returning narrow-range, native component or non-RGB/gray data.
///
/// An untagged source uses the conventional sRGB transfer. A present but
/// unknown source color stays unknown.
pub fn descriptor_for_decoded_pixels_v2(
    layout: SampleLayout,
    source_color: &SourceColor,
    corrected_to: Option<&ColorSpace>,
) -> Result<ImageSpec, whereat::At<zenpixels::ColorError>> {
    let base = super::codec_spec(layout);
    let checked = |spec: ImageSpec| {
        spec.validate()?;
        Ok(spec)
    };
    if let Some(corrected) = corrected_to {
        return checked(ImageSpec::new(layout, corrected.clone()));
    }

    if let Some(AuthoritativeColor::Icc(profile)) = source_color.authoritative() {
        let icc_class = IccColorSpace::from_icc(profile);
        let order = layout.order;
        if order.is_cmyk_family()
            && icc_class == Some(IccColorSpace::Cmyk)
            && let ColorSpace::Cmyk {
                inverted, alpha, ..
            } = base.color
        {
            return checked(ImageSpec::new(
                layout,
                ColorSpace::Cmyk {
                    inverted,
                    alpha,
                    icc: Some(profile.clone()),
                },
            ));
        }
        let channels = match icc_class {
            Some(IccColorSpace::Rgb) if order.is_rgb_family() => Some(3),
            Some(IccColorSpace::Gray) if order.is_gray_family() => Some(1),
            _ => None,
        };
        if let Some(channels) = channels {
            return checked(ImageSpec::new(
                layout,
                ColorSpace::Icc {
                    profile: profile.clone(),
                    channels,
                    alpha: base.color.alpha(),
                    intent: RenderingIntent::RelativeColorimetric,
                },
            ));
        }
        if icc_class.is_some() {
            return Err(whereat::at!(zenpixels::ColorError::ColorLayoutMismatch));
        }
        return checked(
            base.with_primaries(Primaries::Unknown)
                .with_transfer(Transfer::Unknown),
        );
    }

    let (primaries, transfer) = resolve_color(source_color, None);
    let mut spec = base.with_primaries(primaries).with_transfer(transfer);
    if let Some(white) = source_color.diffuse_white {
        spec = spec.with_white(white);
    }
    checked(spec)
}

/// Metadata projection of a current or authoritative source color description.
///
/// ICC lookup identifies descriptive primaries and transfer. This result
/// cannot be used to substitute for an ICC transform.
pub fn resolve_color(
    source_color: &SourceColor,
    corrected_to: Option<&ColorSpace>,
) -> (Primaries, Transfer) {
    if let Some(color) = corrected_to {
        return describe_color(color);
    }
    match source_color.authoritative() {
        Some(AuthoritativeColor::Cicp(cicp)) => (
            Primaries::from_cicp(cicp.color_primaries).unwrap_or(Primaries::Unknown),
            Transfer::from_cicp(cicp.transfer_characteristics).unwrap_or(Transfer::Unknown),
        ),
        Some(AuthoritativeColor::Icc(profile)) => identify(profile),
        None => (Primaries::Bt709, Transfer::Srgb),
    }
}

fn identify(profile: &[u8]) -> (Primaries, Transfer) {
    zenpixels::icc::IccIdentification::from_icc(profile)
        .map_or((Primaries::Unknown, Transfer::Unknown), |id| {
            (id.primaries, id.transfer)
        })
}

fn describe_color(color: &ColorSpace) -> (Primaries, Transfer) {
    match color {
        ColorSpace::Rgb {
            primaries,
            transfer,
            ..
        } => (*primaries, *transfer),
        ColorSpace::Gray { transfer, .. } => (Primaries::Bt709, *transfer),
        ColorSpace::Icc { profile, .. }
        | ColorSpace::Cmyk {
            icc: Some(profile), ..
        } => identify(profile),
        _ => (Primaries::Unknown, Transfer::Unknown),
    }
}

// ── Deprecated shims (scheduled for removal in next minor release) ─────────

/// Maximum u16 TRC error tolerance for ICC profile identification.
///
/// **Deprecated placebo.** `zenpixels::icc::IccIdentification::from_icc` uses its
/// `Intent` tolerance internally; all variants here collapse to that.
/// At 8-bit and 10-bit output the distinction is invisible (the worst
/// case shifts a u8 by ≤0.22 of a step). Callers computing perceptual
/// metrics at 14-bit+ precision should identify profiles via a full CMS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
#[deprecated(
    since = "0.1.16",
    note = "zenpixels::icc::IccIdentification::from_icc uses Intent tolerance; sub-Intent variants are placebo"
)]
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

/// Identify a well-known ICC profile by normalized hash lookup.
///
/// Delegates to [`zenpixels::icc::IccIdentification::from_icc`]. The `tolerance`
/// argument is accepted for backwards compatibility but ignored — see
/// [`IccMatchTolerance`] for the rationale.
#[deprecated(
    since = "0.1.16",
    note = "use zenpixels::icc::IccIdentification::from_icc — returns richer IccIdentification with valid_use"
)]
#[allow(deprecated)]
pub fn identify_well_known_icc(
    icc_bytes: &[u8],
    _tolerance: IccMatchTolerance,
) -> Option<(Primaries, Transfer)> {
    let id = zenpixels::icc::IccIdentification::from_icc(icc_bytes)?;
    Some((id.primaries, id.transfer))
}

/// Check if an ICC profile is a known sRGB profile.
#[deprecated(
    since = "0.1.16",
    note = "use zenpixels::icc::IccIdentification::is_common_srgb"
)]
pub fn icc_profile_is_srgb(icc_bytes: &[u8]) -> bool {
    zenpixels::icc::IccIdentification::is_common_srgb(icc_bytes)
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use alloc::sync::Arc;
    use zenpixels::{AlphaMode, Cicp, Primaries, SignalRange, Transfer};

    // ── Priority 4: no metadata → sRGB assumption ──────────────────────

    #[test]
    fn no_metadata_assumes_srgb() {
        let sc = SourceColor::default();
        let desc =
            descriptor_for_decoded_pixels(SampleLayout::RGB8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt709);
    }

    #[test]
    fn no_metadata_gray_assumes_srgb() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::GRAY8,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt709);
        assert_eq!(desc.layout, SampleLayout::GRAY8);
    }

    #[test]
    fn no_metadata_rgba_assumes_srgb() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::RGBA8,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt709);
        assert_eq!(desc.layout, SampleLayout::RGBA8);
        assert_eq!(desc.color.alpha(), AlphaMode::Straight);
    }

    #[test]
    fn no_metadata_f32_assumes_srgb() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::RGBF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt709);
        assert_eq!(desc.layout, SampleLayout::RGBF32);
    }

    // ── Priority 2: CICP metadata ──────────────────────────────────────

    #[test]
    fn cicp_srgb_sets_srgb_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc =
            descriptor_for_decoded_pixels(SampleLayout::RGB8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt709);
    }

    #[test]
    fn cicp_p3_sets_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc =
            descriptor_for_decoded_pixels(SampleLayout::RGB8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::DisplayP3);
    }

    #[test]
    fn cicp_pq_sets_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::RGBAF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(describe_color(&desc.color).1, Transfer::Pq);
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt2020);
    }

    #[test]
    fn cicp_hlg_sets_descriptor() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_HLG);
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::RGBF32,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(
            describe_color(&desc.color).1,
            Transfer::Hlg(zenpixels::HlgMode::Scene)
        );
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt2020);
    }

    #[test]
    fn cicp_takes_precedence_over_icc_when_cicp_authoritative() {
        // When the codec declares CICP authoritative (AVIF/HEIF nclx, PNG
        // cICP), the CICP field wins even if an ICC blob is also present.
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default()
            .with_cicp(Cicp::DISPLAY_P3)
            .with_icc_profile(fake_icc)
            .with_color_authority(crate::ColorAuthority::Cicp);
        let desc = descriptor_for_decoded_pixels_v2(SampleLayout::RGB8, &sc, None).unwrap();
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::DisplayP3);
    }

    #[test]
    fn icc_takes_precedence_over_cicp_when_icc_authoritative() {
        // When the codec declares ICC authoritative (JPEG, PNG with iCCP,
        // WebP, TIFF), the ICC blob wins. A 64-byte zero blob doesn't
        // hash-match any known profile — expect Unknown/Unknown, not the
        // CICP Display P3 values.
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default()
            .with_cicp(Cicp::DISPLAY_P3)
            .with_icc_profile(fake_icc)
            .with_color_authority(crate::ColorAuthority::Icc);
        let desc = descriptor_for_decoded_pixels_v2(SampleLayout::RGB8, &sc, None).unwrap();
        assert_eq!(describe_color(&desc.color).1, Transfer::Unknown);
        assert_eq!(describe_color(&desc.color).0, Primaries::Unknown);
    }

    #[test]
    fn cicp_preserves_pixel_format() {
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        for fmt in [
            SampleLayout::RGB8,
            SampleLayout::RGBA8,
            SampleLayout::GRAY8,
            SampleLayout::RGBF32,
            SampleLayout::BGRA8,
        ] {
            let desc = descriptor_for_decoded_pixels(fmt, &sc, None, IccMatchTolerance::Intent);
            assert_eq!(desc.layout, fmt, "format mismatch for {fmt:?}");
        }
    }

    // ── Priority 3: ICC profile ────────────────────────────────────────

    #[test]
    fn unknown_icc_yields_unknown_descriptor() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);
        let desc =
            descriptor_for_decoded_pixels(SampleLayout::RGB8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(describe_color(&desc.color).1, Transfer::Unknown);
        assert_eq!(describe_color(&desc.color).0, Primaries::Unknown);
    }

    #[test]
    fn unknown_icc_preserves_format_and_alpha() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![99u8; 128].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);

        let desc = descriptor_for_decoded_pixels(
            SampleLayout::RGBA8,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.layout, SampleLayout::RGBA8);
        assert_eq!(desc.color.alpha(), AlphaMode::Straight);
        assert_eq!(crate::helpers::signal_range(&desc), SignalRange::Full);

        let desc =
            descriptor_for_decoded_pixels(SampleLayout::RGB8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(desc.layout, SampleLayout::RGB8);
        assert!(desc.color.alpha() == AlphaMode::None);
    }

    #[test]
    fn unknown_icc_gray_preserves_format() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![42u8; 96].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::GRAY8,
            &sc,
            None,
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.layout, SampleLayout::GRAY8);
        assert_eq!(describe_color(&desc.color).1, Transfer::Unknown);
        assert_eq!(resolve_color(&sc, None).0, Primaries::Unknown);
        assert!(matches!(
            desc.color,
            ColorSpace::Gray {
                transfer: Transfer::Unknown,
                ..
            }
        ));
    }

    #[test]
    fn empty_icc_yields_unknown() {
        let empty_icc: Arc<[u8]> = Arc::from(alloc::vec![].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(empty_icc);
        let desc =
            descriptor_for_decoded_pixels(SampleLayout::RGB8, &sc, None, IccMatchTolerance::Intent);
        assert_eq!(describe_color(&desc.color).1, Transfer::Unknown);
        assert_eq!(describe_color(&desc.color).0, Primaries::Unknown);
    }

    // ── Priority 1: corrected_to overrides everything ──────────────────

    #[test]
    fn corrected_to_overrides_source_cicp() {
        let sc = SourceColor::default().with_cicp(Cicp::DISPLAY_P3);
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::RGB8,
            &sc,
            Some(&Cicp::SRGB),
            IccMatchTolerance::Intent,
        );
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt709);
    }

    #[test]
    fn corrected_to_overrides_unknown_icc() {
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default().with_icc_profile(fake_icc);
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::RGB8,
            &sc,
            Some(&Cicp::SRGB),
            IccMatchTolerance::Intent,
        );
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt709);
    }

    #[test]
    fn corrected_to_overrides_no_metadata() {
        let sc = SourceColor::default();
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::RGB8,
            &sc,
            Some(&Cicp::SRGB),
            IccMatchTolerance::Intent,
        );
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::Bt709);
    }

    #[test]
    fn corrected_to_p3_target() {
        let sc = SourceColor::default().with_cicp(Cicp::SRGB);
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::RGB8,
            &sc,
            Some(&Cicp::DISPLAY_P3),
            IccMatchTolerance::Intent,
        );
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
        assert_eq!(describe_color(&desc.color).0, Primaries::DisplayP3);
    }

    #[test]
    fn corrected_to_preserves_format() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let desc = descriptor_for_decoded_pixels(
            SampleLayout::BGRA8,
            &sc,
            Some(&Cicp::SRGB),
            IccMatchTolerance::Intent,
        );
        assert_eq!(desc.layout, SampleLayout::BGRA8);
        assert_eq!(describe_color(&desc.color).1, Transfer::Srgb);
    }

    // ── Deprecated shim sanity ─────────────────────────────────────────

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
    fn tolerance_ordering() {
        assert!(IccMatchTolerance::Exact < IccMatchTolerance::Precise);
        assert!(IccMatchTolerance::Precise < IccMatchTolerance::Approximate);
        assert!(IccMatchTolerance::Approximate < IccMatchTolerance::Intent);
    }

    // ── Per-format decode scenarios (table-driven) ──────────────────

    fn sc_none() -> SourceColor {
        SourceColor::default()
    }
    fn sc_cicp(c: Cicp) -> SourceColor {
        SourceColor::default().with_cicp(c)
    }
    fn sc_icc(fill: u8, len: usize) -> SourceColor {
        let icc: Arc<[u8]> = Arc::from(alloc::vec![fill; len].into_boxed_slice());
        SourceColor::default().with_icc_profile(icc)
    }
    /// Helper for formats where BOTH CICP and ICC are present and CICP
    /// is authoritative (PNG with cICP chunk, AVIF/HEIF with nclx colr box).
    fn sc_cicp_icc(c: Cicp, fill: u8, len: usize) -> SourceColor {
        let icc: Arc<[u8]> = Arc::from(alloc::vec![fill; len].into_boxed_slice());
        SourceColor::default()
            .with_cicp(c)
            .with_icc_profile(icc)
            .with_color_authority(crate::ColorAuthority::Cicp)
    }

    use Primaries as CP;
    use Transfer as TF;

    type FormatScenario = (
        &'static str,
        SampleLayout,
        SourceColor,
        Option<Cicp>,
        TF,
        CP,
    );

    #[test]
    fn format_scenarios() {
        let cases: &[FormatScenario] = &[
            (
                "jpeg_no_icc",
                SampleLayout::RGB8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "jpeg_unknown_icc",
                SampleLayout::RGB8,
                sc_icc(0xCA, 3144),
                None,
                TF::Unknown,
                CP::Unknown,
            ),
            (
                "jpeg_corrected",
                SampleLayout::RGB8,
                sc_icc(0xCA, 3144),
                Some(Cicp::SRGB),
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "png_cicp_p3",
                SampleLayout::RGBA8,
                sc_cicp(Cicp::DISPLAY_P3),
                None,
                TF::Srgb,
                CP::DisplayP3,
            ),
            (
                "png_cicp_over_icc",
                SampleLayout::RGBA8,
                sc_cicp_icc(Cicp::DISPLAY_P3, 0, 100),
                None,
                TF::Srgb,
                CP::DisplayP3,
            ),
            (
                "png_no_metadata",
                SampleLayout::RGBA8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "png_hdr_pq",
                SampleLayout::RGBA16,
                sc_cicp(Cicp::BT2100_PQ),
                None,
                TF::Pq,
                CP::Bt2020,
            ),
            (
                "webp_no_icc",
                SampleLayout::RGBA8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "webp_unknown_icc",
                SampleLayout::RGBA8,
                sc_icc(0xA3, 480),
                None,
                TF::Unknown,
                CP::Unknown,
            ),
            (
                "avif_srgb",
                SampleLayout::RGBA8,
                sc_cicp(Cicp::SRGB),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "avif_hdr10",
                SampleLayout::RGBAF32,
                sc_cicp(Cicp::BT2100_PQ),
                None,
                TF::Pq,
                CP::Bt2020,
            ),
            (
                "avif_hlg",
                SampleLayout::RGBF32,
                sc_cicp(Cicp::BT2100_HLG),
                None,
                TF::Hlg(zenpixels::HlgMode::Scene),
                CP::Bt2020,
            ),
            (
                "avif_p3",
                SampleLayout::RGB8,
                sc_cicp(Cicp::DISPLAY_P3),
                None,
                TF::Srgb,
                CP::DisplayP3,
            ),
            (
                "jxl_srgb",
                SampleLayout::RGB8,
                sc_cicp(Cicp::SRGB),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "jxl_p3_pq",
                SampleLayout::RGBAF32,
                sc_cicp(Cicp::new(12, 16, 0, true)),
                None,
                TF::Pq,
                CP::DisplayP3,
            ),
            (
                "heic_p3",
                SampleLayout::RGBA8,
                sc_cicp(Cicp::DISPLAY_P3),
                None,
                TF::Srgb,
                CP::DisplayP3,
            ),
            (
                "heic_hdr10",
                SampleLayout::RGBAF32,
                sc_cicp(Cicp::BT2100_PQ),
                None,
                TF::Pq,
                CP::Bt2020,
            ),
            (
                "gif_srgb",
                SampleLayout::RGBA8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "bmp_srgb",
                SampleLayout::RGB8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "pnm_gray",
                SampleLayout::GRAY8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "tiff_unknown_icc",
                SampleLayout::RGB16,
                sc_icc(0x54, 7261),
                None,
                TF::Unknown,
                CP::Unknown,
            ),
            (
                "tiff_no_icc",
                SampleLayout::RGB16,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
        ];

        for &(name, fmt, ref sc, ref corrected, exp_tf, exp_cp) in cases {
            let desc = descriptor_for_decoded_pixels(
                fmt,
                sc,
                corrected.as_ref(),
                IccMatchTolerance::Intent,
            );
            assert_eq!(describe_color(&desc.color).1, exp_tf, "{name}: transfer");
            assert_eq!(describe_color(&desc.color).0, exp_cp, "{name}: primaries");
            assert_eq!(desc.layout, fmt, "{name}: format");
        }
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
                SampleLayout::RGB8,
                sc,
                *corrected,
                IccMatchTolerance::Intent,
            );
            assert_eq!(
                crate::helpers::signal_range(&desc),
                SignalRange::Full,
                "non-full range for {sc:?} corrected={corrected:?}"
            );
        }
    }
}

#[cfg(test)]
mod checked_class_tests {
    use super::*;
    use zenpixels::{AlphaMode, SignalRange};
    use zenpixels_convert::icc_profiles::SynthesizedIcc;

    #[test]
    fn real_profiles_cannot_label_the_wrong_sample_model() {
        let SynthesizedIcc::Profile(rgb) = SynthesizedIcc::for_cicp(Cicp::DISPLAY_P3) else {
            panic!("RGB fixture profile unavailable");
        };
        let SynthesizedIcc::Profile(gray) = SynthesizedIcc::for_gray_cicp(Cicp::new(1, 8, 0, true))
        else {
            panic!("GRAY fixture profile unavailable");
        };
        for (profile, valid, invalid, channels) in [
            (rgb, SampleLayout::RGBA8, SampleLayout::GRAYA8, 3),
            (gray, SampleLayout::GRAYA8, SampleLayout::RGBA8, 1),
        ] {
            let source = SourceColor::default().with_icc_profile(profile.as_ref());
            let correct = descriptor_for_decoded_pixels_v2(valid, &source, None).unwrap();
            assert_eq!(correct.layout, valid);
            let ColorSpace::Icc {
                profile: actual,
                channels: count,
                alpha,
                intent,
            } = correct.color
            else {
                panic!("matching profile was discarded");
            };
            assert_eq!(actual.as_ref(), profile.as_ref());
            assert_eq!(count, channels);
            assert_eq!(alpha, AlphaMode::Straight);
            assert_eq!(intent, RenderingIntent::RelativeColorimetric);
            assert!(descriptor_for_decoded_pixels_v2(invalid, &source, None).is_err());
        }
        let wrong = ColorSpace::Gray {
            transfer: Transfer::Pq,
            range: SignalRange::Narrow,
            alpha: AlphaMode::Straight,
            white: None,
        };
        assert!(
            descriptor_for_decoded_pixels_v2(
                SampleLayout::RGBA8,
                &SourceColor::default(),
                Some(&wrong)
            )
            .is_err()
        );
        let correct = descriptor_for_decoded_pixels_v2(
            SampleLayout::GRAYA8,
            &SourceColor::default(),
            Some(&wrong),
        )
        .unwrap();
        assert_eq!(correct.color, wrong);
    }
}
