//! ICC profile identification and pixel descriptor derivation.
//!
//! All identification delegates to [`zenpixels::icc`] (enabled via the `icc`
//! feature, required by zencodec), which ships a superset of the corpus:
//! 163 RGB + 18 grayscale profiles with intent-safety masks cross-validated
//! against moxcms and lcms2.
//!
//! The zencodec-specific entry point is [`descriptor_for_decoded_pixels`],
//! which applies codec-output priority rules (corrected_to → CICP → ICC → sRGB
//! default). The `identify_well_known_icc` / `icc_profile_is_srgb` /
//! `IccMatchTolerance` symbols remain as deprecated shims for 0.1.x callers —
//! scheduled for removal in the next minor release.

use crate::decode::SourceColor;
use zenpixels::{
    Cicp, ColorPrimaries, ColorProfileSource, PixelDescriptor, PixelFormat, TransferFunction,
};

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
/// 3. If `source_color` has an ICC profile, [`zenpixels::icc::identify_common`]
///    is consulted. Unrecognized profiles yield `Unknown` transfer/primaries.
/// 4. No color metadata at all: assumes sRGB (legacy format convention).
///
/// `tolerance` is accepted for API compatibility but is currently a no-op:
/// `zenpixels::icc::identify_common` uses its `Intent` tolerance internally,
/// which is indistinguishable from stricter tolerances at 8-bit and 10-bit
/// output. See [`IccMatchTolerance`].
///
/// # Deprecated
///
/// New code should use [`descriptor_for_decoded_pixels_v2`], which has the
/// same semantics without the placebo `tolerance` parameter.
#[deprecated(
    since = "0.1.17",
    note = "use descriptor_for_decoded_pixels_v2 (drops placebo IccMatchTolerance)"
)]
#[allow(deprecated)]
pub fn descriptor_for_decoded_pixels(
    format: PixelFormat,
    source_color: &SourceColor,
    corrected_to: Option<&Cicp>,
    _tolerance: IccMatchTolerance,
) -> PixelDescriptor {
    let corrected_src = corrected_to.map(|c| ColorProfileSource::Cicp(*c));
    descriptor_for_decoded_pixels_v2(format, source_color, corrected_src.as_ref())
}

/// Derive a [`PixelDescriptor`] that accurately describes decoded pixel data.
///
/// Modern replacement for [`descriptor_for_decoded_pixels`]. Drops the
/// deprecated `IccMatchTolerance` placebo parameter. `corrected_to`
/// widens from `Option<&Cicp>` to `Option<&ColorProfileSource>` so
/// callers can describe correction targets that aren't CICP-expressible
/// (arbitrary primaries+transfer, named profiles like Adobe RGB
/// v2-gamma, custom ICC profiles).
///
/// # Priority
///
/// 1. If `corrected_to` is `Some`, the pixels were color-managed to that
///    target during decode. The descriptor reflects the target.
/// 2. Otherwise resolves via [`SourceColor::to_color_context`] which
///    honors [`SourceColor::color_authority`] — drops the
///    non-authoritative field so
///    [`zenpixels::ColorContext::as_profile_source`] returns whichever
///    the codec declared canonical.
/// 3. If the resolved `ColorProfileSource` is an ICC blob,
///    [`zenpixels::icc::identify_common`] is consulted. Unrecognized
///    profiles yield `Unknown` transfer/primaries.
/// 4. No color metadata at all: assumes sRGB (legacy format convention).
///
/// # See also
///
/// - `resolve_color` (crate-private) — the underlying
///   `(ColorPrimaries, TransferFunction)` resolution without the
///   descriptor scaffolding. Used internally when callers want to
///   inspect the resolved color identity before committing to a
///   `PixelDescriptor` — e.g., to branch on Unknown/Bt709/etc. before
///   picking a `PixelFormat`.
pub fn descriptor_for_decoded_pixels_v2(
    format: PixelFormat,
    source_color: &SourceColor,
    corrected_to: Option<&ColorProfileSource<'_>>,
) -> PixelDescriptor {
    let (primaries, transfer) = resolve_color(source_color, corrected_to);
    PixelDescriptor::from_pixel_format(format)
        .with_primaries(primaries)
        .with_transfer(transfer)
}

/// Resolve `(ColorPrimaries, TransferFunction)` for decoded pixel data
/// without building a [`PixelDescriptor`].
///
/// Priority matches [`descriptor_for_decoded_pixels_v2`]:
///
/// 1. `corrected_to` if `Some`.
/// 2. Authority-aware resolution via [`SourceColor::to_color_context`].
///    When an ICC blob is authoritative,
///    [`zenpixels::icc::identify_common`] extracts primaries/transfer;
///    unrecognized ICC returns `(Unknown, Unknown)`.
/// 3. Assumed sRGB (Bt709 / Srgb) when no metadata is present.
///
/// Separates color resolution from descriptor construction so callers
/// can inspect the result (e.g., refuse to encode `(Unknown, _)` without
/// user confirmation) before committing to a pixel format.
pub fn resolve_color(
    source_color: &SourceColor,
    corrected_to: Option<&ColorProfileSource<'_>>,
) -> (ColorPrimaries, TransferFunction) {
    if let Some(src) = corrected_to {
        return resolve_profile_source(src);
    }
    if let Some(src) = source_color.to_color_context().as_profile_source() {
        return resolve_profile_source(&src);
    }
    // No metadata at all — legacy web/desktop default.
    (ColorPrimaries::Bt709, TransferFunction::Srgb)
}

/// Extract `(primaries, transfer)` from any `ColorProfileSource` variant.
fn resolve_profile_source(src: &ColorProfileSource<'_>) -> (ColorPrimaries, TransferFunction) {
    match src {
        ColorProfileSource::Cicp(cicp) => (
            ColorPrimaries::from_cicp(cicp.color_primaries).unwrap_or(ColorPrimaries::Unknown),
            TransferFunction::from_cicp(cicp.transfer_characteristics)
                .unwrap_or(TransferFunction::Unknown),
        ),
        ColorProfileSource::Icc(icc) => match zenpixels::icc::identify_common(icc) {
            Some(id) => (id.primaries, id.transfer),
            None => (ColorPrimaries::Unknown, TransferFunction::Unknown),
        },
        ColorProfileSource::Named(named) => named.to_primaries_transfer(),
        ColorProfileSource::PrimariesTransferPair {
            primaries,
            transfer,
        } => (*primaries, *transfer),
        // `ColorProfileSource` is #[non_exhaustive] — future variants
        // fall through to Unknown until explicitly handled.
        _ => (ColorPrimaries::Unknown, TransferFunction::Unknown),
    }
}

// ── Deprecated shims (scheduled for removal in next minor release) ─────────

/// Maximum u16 TRC error tolerance for ICC profile identification.
///
/// **Deprecated placebo.** `zenpixels::icc::identify_common` uses its
/// `Intent` tolerance internally; all variants here collapse to that.
/// At 8-bit and 10-bit output the distinction is invisible (the worst
/// case shifts a u8 by ≤0.22 of a step). Callers computing perceptual
/// metrics at 14-bit+ precision should identify profiles via a full CMS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
#[deprecated(
    since = "0.1.16",
    note = "zenpixels::icc::identify_common uses Intent tolerance; sub-Intent variants are placebo"
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
/// Delegates to [`zenpixels::icc::identify_common`]. The `tolerance`
/// argument is accepted for backwards compatibility but ignored — see
/// [`IccMatchTolerance`] for the rationale.
#[deprecated(
    since = "0.1.16",
    note = "use zenpixels::icc::identify_common — returns richer IccIdentification with valid_use"
)]
#[allow(deprecated)]
pub fn identify_well_known_icc(
    icc_bytes: &[u8],
    _tolerance: IccMatchTolerance,
) -> Option<(ColorPrimaries, TransferFunction)> {
    let id = zenpixels::icc::identify_common(icc_bytes)?;
    Some((id.primaries, id.transfer))
}

/// Check if an ICC profile is a known sRGB profile.
#[deprecated(since = "0.1.16", note = "use zenpixels::icc::is_common_srgb")]
pub fn icc_profile_is_srgb(icc_bytes: &[u8]) -> bool {
    zenpixels::icc::is_common_srgb(icc_bytes)
}

#[cfg(test)]
#[allow(deprecated)]
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
    fn cicp_takes_precedence_over_icc_when_cicp_authoritative() {
        // When the codec declares CICP authoritative (AVIF/HEIF nclx, PNG
        // cICP), the CICP field wins even if an ICC blob is also present.
        let fake_icc: Arc<[u8]> = Arc::from(alloc::vec![0u8; 64].into_boxed_slice());
        let sc = SourceColor::default()
            .with_cicp(Cicp::DISPLAY_P3)
            .with_icc_profile(fake_icc)
            .with_color_authority(zenpixels::ColorAuthority::Cicp);
        let desc = descriptor_for_decoded_pixels_v2(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Srgb);
        assert_eq!(desc.primaries, ColorPrimaries::DisplayP3);
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
            .with_color_authority(zenpixels::ColorAuthority::Icc);
        let desc = descriptor_for_decoded_pixels_v2(PixelFormat::Rgb8, &sc, None);
        assert_eq!(desc.transfer, TransferFunction::Unknown);
        assert_eq!(desc.primaries, ColorPrimaries::Unknown);
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
            .with_color_authority(zenpixels::ColorAuthority::Cicp)
    }

    use ColorPrimaries as CP;
    use TransferFunction as TF;

    type FormatScenario = (&'static str, PixelFormat, SourceColor, Option<Cicp>, TF, CP);

    #[test]
    fn format_scenarios() {
        let cases: &[FormatScenario] = &[
            (
                "jpeg_no_icc",
                PixelFormat::Rgb8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "jpeg_unknown_icc",
                PixelFormat::Rgb8,
                sc_icc(0xCA, 3144),
                None,
                TF::Unknown,
                CP::Unknown,
            ),
            (
                "jpeg_corrected",
                PixelFormat::Rgb8,
                sc_icc(0xCA, 3144),
                Some(Cicp::SRGB),
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "png_cicp_p3",
                PixelFormat::Rgba8,
                sc_cicp(Cicp::DISPLAY_P3),
                None,
                TF::Srgb,
                CP::DisplayP3,
            ),
            (
                "png_cicp_over_icc",
                PixelFormat::Rgba8,
                sc_cicp_icc(Cicp::DISPLAY_P3, 0, 100),
                None,
                TF::Srgb,
                CP::DisplayP3,
            ),
            (
                "png_no_metadata",
                PixelFormat::Rgba8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "png_hdr_pq",
                PixelFormat::Rgba16,
                sc_cicp(Cicp::BT2100_PQ),
                None,
                TF::Pq,
                CP::Bt2020,
            ),
            (
                "webp_no_icc",
                PixelFormat::Rgba8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "webp_unknown_icc",
                PixelFormat::Rgba8,
                sc_icc(0xA3, 480),
                None,
                TF::Unknown,
                CP::Unknown,
            ),
            (
                "avif_srgb",
                PixelFormat::Rgba8,
                sc_cicp(Cicp::SRGB),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "avif_hdr10",
                PixelFormat::RgbaF32,
                sc_cicp(Cicp::BT2100_PQ),
                None,
                TF::Pq,
                CP::Bt2020,
            ),
            (
                "avif_hlg",
                PixelFormat::RgbF32,
                sc_cicp(Cicp::BT2100_HLG),
                None,
                TF::Hlg,
                CP::Bt2020,
            ),
            (
                "avif_p3",
                PixelFormat::Rgb8,
                sc_cicp(Cicp::DISPLAY_P3),
                None,
                TF::Srgb,
                CP::DisplayP3,
            ),
            (
                "jxl_srgb",
                PixelFormat::Rgb8,
                sc_cicp(Cicp::SRGB),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "jxl_p3_pq",
                PixelFormat::RgbaF32,
                sc_cicp(Cicp::new(12, 16, 0, true)),
                None,
                TF::Pq,
                CP::DisplayP3,
            ),
            (
                "heic_p3",
                PixelFormat::Rgba8,
                sc_cicp(Cicp::DISPLAY_P3),
                None,
                TF::Srgb,
                CP::DisplayP3,
            ),
            (
                "heic_hdr10",
                PixelFormat::RgbaF32,
                sc_cicp(Cicp::BT2100_PQ),
                None,
                TF::Pq,
                CP::Bt2020,
            ),
            (
                "gif_srgb",
                PixelFormat::Rgba8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "bmp_srgb",
                PixelFormat::Rgb8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "pnm_gray",
                PixelFormat::Gray8,
                sc_none(),
                None,
                TF::Srgb,
                CP::Bt709,
            ),
            (
                "tiff_unknown_icc",
                PixelFormat::Rgb16,
                sc_icc(0x54, 7261),
                None,
                TF::Unknown,
                CP::Unknown,
            ),
            (
                "tiff_no_icc",
                PixelFormat::Rgb16,
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
            assert_eq!(desc.transfer, exp_tf, "{name}: transfer");
            assert_eq!(desc.primaries, exp_cp, "{name}: primaries");
            assert_eq!(desc.pixel_format(), fmt, "{name}: format");
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
