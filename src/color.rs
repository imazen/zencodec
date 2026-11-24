//! Internal: color-signaling emission policy (ICC profile vs CICP code points).
//!
//! This module is private — its types (`ColorEmitPolicy`, `ColorEmitFields`,
//! `ColorEmitPlan`, `IccDisposition`, `CicpEmission`) and `resolve_color_emit`
//! are re-exported at the crate root. The public overview lives on
//! `ColorEmitPolicy`; the full per-format design is in `docs/color-emit-model.md`.

use zenpixels::icc;
use zenpixels::{Cicp, ColorSpace, HlgMode, IccColorSpace, Primaries, SignalRange, Transfer};

use crate::capabilities::EncodeCapabilities;
use crate::info::{AuthoritativeColor, SourceColor};
use crate::metadata::IccRetention;

/// How an image's color *description* (ICC profile vs CICP code points) is
/// emitted when encoding or transcoding — the obvious, intent-named knob.
///
/// This is orthogonal to which *pixels* are written. Containers differ in which
/// color carriers they have and in how reliably real-world decoders honor each
/// one, so emitting "the right" color description is a per-target decision.
///
/// # Presets
///
/// Pick an intent — the same meaning whether encoding from pixels or transcoding
/// from another file:
///
/// - [`Compatibility`](ColorEmitPolicy::Compatibility) — always embed an ICC; add CICP where reliable.
/// - [`Balanced`](ColorEmitPolicy::Balanced) (**default**) — emit CICP where the format has a
///   standardized CICP carrier, drop a redundant ICC only where CICP is safe as the sole carrier
///   (JXL only: AVIF/HEIC readers such as vips and ImageMagick act only on the ICC,
///   imazen/zencodec#132) or the ICC is plain sRGB.
/// - [`Compact`](ColorEmitPolicy::Compact) — smallest: prefer CICP wherever the format carries it, drop the ICC.
/// - [`Verbatim`](ColorEmitPolicy::Verbatim) — carry the source's signals unchanged.
/// - [`Custom`](ColorEmitPolicy::Custom) — explicit [`ColorEmitFields`] for power users.
///
/// # The resolver: [`resolve_color_emit`]
///
/// [`resolve_color_emit`] reconciles a [`SourceColor`] against a target's
/// [`EncodeCapabilities`] under a `ColorEmitPolicy` and returns a [`ColorEmitPlan`] —
/// a pure description of what to emit. This crate is `no_std` and carries no
/// CMS, so the plan only describes intent ([`IccDisposition::SynthesizeFrom`],
/// etc.); the bytes are materialized one layer up.
///
/// # Lowering the plan
///
/// A codec (or the pipeline) lowers a [`ColorEmitPlan`] to the bytes it writes.
/// The lowering is **per-codec and by convention**: each codec maps the plan's
/// fields onto its container carriers. The caller supplies source metadata
/// describing the current samples and their active color model.
///
/// - [`ColorEmitPlan::cicp`] maps to the format's native CICP carrier.
/// - [`IccDisposition::KeepSource`] embeds the supplied ICC bytes unchanged.
/// - [`IccDisposition::SynthesizeFrom`] and [`IccDisposition::SynthesizeGrayFrom`]
///   require a matching RGB or GRAY profile from the converter before embedding.
/// - [`IccDisposition::Drop`] → emit no ICC.
///
/// Orientation/EXIF reconciliation is separate: when a pipeline bakes orientation
/// upright it rewrites the source EXIF orientation tag with
/// [`helpers::set_exif_orientation`](crate::helpers::set_exif_orientation) so the
/// tag and the pixels can't disagree (the double-rotation hazard).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColorEmitPolicy {
    /// Widest compatibility: always embed an ICC profile (synthesizing one from
    /// CICP when the source had none); add CICP where the format treats it as
    /// authority. Largest color overhead.
    Compatibility,
    /// **Default.** Emit CICP where it is the format's authority and drop a
    /// redundant ICC only where CICP is safe as the *sole* carrier
    /// ([`cicp_safe_sole_carrier`](EncodeCapabilities::cicp_safe_sole_carrier) —
    /// JXL only, as measured in imazen/zencodec#132) or the ICC is a plain sRGB
    /// profile. Otherwise keep the ICC.
    #[default]
    Balanced,
    /// Smallest color overhead: prefer CICP wherever the format can carry it at
    /// all, and drop the ICC whenever CICP can describe the color.
    ///
    /// Measured cost (imazen/zencodec#132): a PNG `cICP` alone is ignored by WIC,
    /// WebKit's Linux build and the Linux libraries (ImageMagick, vips, Pillow),
    /// and an AVIF `nclx` alone by vips and ImageMagick; those readers show the
    /// pixels as sRGB.
    Compact,
    /// Carry the source's color signals through unchanged — derive and strip
    /// nothing. For transcodes that must preserve exactly what was there.
    Verbatim,
    /// Explicit mechanism control.
    Custom(ColorEmitFields),
}

/// Whether CICP is emitted, behind [`ColorEmitPolicy::Custom`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CicpEmission {
    /// Emit CICP where the format has a standardized, real-world-honored CICP
    /// carrier ([`cicp_is_valid_carrier`](EncodeCapabilities::cicp_is_valid_carrier)):
    /// JXL/AVIF/HEIC `nclx`, and PNG `cICP`. The default. Distinct from
    /// "drop the ICC" — a valid carrier (PNG) still keeps the ICC alongside.
    #[default]
    WhereValidCarrier,
    /// Emit CICP wherever the format has *any* carrier slot, even a non-standard
    /// or emergent one.
    WhereverSupported,
    /// Never emit CICP (ICC-only output).
    Never,
}

/// Mechanism fields behind [`ColorEmitPolicy::Custom`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColorEmitFields {
    /// When to drop the ICC profile.
    pub icc: IccRetention,
    /// Whether to emit CICP.
    pub cicp: CicpEmission,
}

impl Default for ColorEmitFields {
    fn default() -> Self {
        Self {
            icc: IccRetention::DropIfCicpSafeSoleCarrier,
            cicp: CicpEmission::WhereValidCarrier,
        }
    }
}

impl ColorEmitFields {
    /// Construct explicit color-emission fields for [`ColorEmitPolicy::Custom`].
    ///
    /// `ColorEmitFields` is `#[non_exhaustive]`, so downstream crates cannot build it
    /// with a struct literal — use this constructor (or [`Default`]) so
    /// [`ColorEmitPolicy::Custom`] is actually reachable.
    pub const fn new(icc: IccRetention, cicp: CicpEmission) -> Self {
        Self { icc, cicp }
    }
}

impl ColorEmitPolicy {
    /// Resolve a preset to its mechanism fields.
    pub const fn fields(&self) -> ColorEmitFields {
        match self {
            Self::Compatibility => ColorEmitFields {
                icc: IccRetention::Keep,
                cicp: CicpEmission::WhereValidCarrier,
            },
            Self::Balanced => ColorEmitFields {
                icc: IccRetention::DropIfCicpSafeSoleCarrier,
                cicp: CicpEmission::WhereValidCarrier,
            },
            Self::Compact => ColorEmitFields {
                icc: IccRetention::DropIfCicpRepresentable,
                cicp: CicpEmission::WhereverSupported,
            },
            Self::Verbatim => ColorEmitFields {
                icc: IccRetention::Keep,
                cicp: CicpEmission::WhereValidCarrier,
            },
            Self::Custom(f) => *f,
        }
    }
}

/// What to do with the ICC profile channel for one encode.
///
/// The bytes are materialized by the codec adapter / CMS layer, not here.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IccDisposition {
    /// Embed the source ICC bytes verbatim.
    KeepSource,
    /// Materialize an RGB profile with `SynthesizedIcc::for_cicp`.
    /// A missing profile requires a valid emitted CICP or a checked refusal.
    SynthesizeFrom(Cicp),
    /// Materialize a GRAY profile with `SynthesizedIcc::for_gray_cicp`.
    SynthesizeGrayFrom(Cicp),
    /// Emit no ICC profile.
    Drop,
}

/// A resolved plan for emitting an image's color description on encode.
///
/// Produced by [`resolve_color_emit`]. Deliberately minimal: it carries the
/// ICC/CICP decision, which is what current transcode needs. `#[non_exhaustive]`
/// so range/rendering-intent/HDR/gain-map dispositions and a warnings channel
/// can be added back additively when a consumer needs them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorEmitPlan {
    /// CICP to write to the target's native carrier, if any.
    pub cicp: Option<Cicp>,
    /// Disposition of the ICC profile channel.
    pub icc: IccDisposition,
}

fn icc_as_cicp(profile: &[u8]) -> Option<Cicp> {
    Cicp::from_icc(profile).or_else(|| icc::IccIdentification::from_icc(profile)?.to_cicp())
}

/// The available carriers and policy cannot express the current color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColorEmitError {
    /// The profile does not match the active color model.
    ProfileModel,
    /// No supported carrier can express the supplied color under this policy.
    Unrepresentable,
}

impl core::fmt::Display for ColorEmitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::ProfileModel => "ICC profile does not match the active color model",
            Self::Unrepresentable => "target carriers cannot express the supplied color",
        })
    }
}

impl core::error::Error for ColorEmitError {}

/// Reconcile the current samples' color description and model with a target.
pub fn resolve_color_emit(
    src: &SourceColor,
    color: &ColorSpace,
    target: &EncodeCapabilities,
    policy: ColorEmitPolicy,
) -> Result<ColorEmitPlan, ColorEmitError> {
    let fields = policy.fields();
    let profile = src.icc_profile.as_deref();
    let model = match color {
        ColorSpace::Rgb { .. } => IccColorSpace::Rgb,
        ColorSpace::Gray { .. } => IccColorSpace::Gray,
        ColorSpace::Cmyk { .. } => IccColorSpace::Cmyk,
        ColorSpace::Icc {
            profile: active, ..
        } => {
            if profile != Some(active.as_ref()) {
                return Err(ColorEmitError::ProfileModel);
            }
            IccColorSpace::from_icc(active).ok_or(ColorEmitError::ProfileModel)?
        }
        _ => return Err(ColorEmitError::Unrepresentable),
    };
    let is_gray = model == IccColorSpace::Gray;
    let active_icc = matches!(
        color,
        ColorSpace::Icc { .. } | ColorSpace::Cmyk { icc: Some(_), .. }
    );
    if let ColorSpace::Cmyk {
        icc: Some(active), ..
    } = color
        && profile != Some(active.as_ref())
    {
        return Err(ColorEmitError::ProfileModel);
    }
    // CICP describes the sample coding. Diffuse white lives outside the tuple.
    let repr_cicp = match color {
        ColorSpace::Rgb {
            transfer: Transfer::Hlg(HlgMode::Display(_)),
            ..
        }
        | ColorSpace::Gray {
            transfer: Transfer::Hlg(HlgMode::Display(_)),
            ..
        } => None,
        ColorSpace::Rgb {
            primaries: Primaries::Unknown,
            range,
            ..
        }
        | ColorSpace::Rgb {
            transfer: Transfer::Unknown,
            range,
            ..
        } => match src.authoritative() {
            Some(AuthoritativeColor::Cicp(c))
                if c.matrix_coefficients == 0 && c.full_range == (*range == SignalRange::Full) =>
            {
                Some(c)
            }
            _ => None,
        },
        ColorSpace::Rgb {
            primaries,
            transfer,
            range,
            alpha,
            ..
        } => ColorSpace::Rgb {
            primaries: *primaries,
            transfer: *transfer,
            range: *range,
            alpha: *alpha,
            white: None,
        }
        .try_cicp()
        .ok(),
        ColorSpace::Gray {
            transfer, range, ..
        } => transfer
            .to_cicp()
            .map(|tc| Cicp::new(1, tc, 0, *range == SignalRange::Full)),
        ColorSpace::Icc { .. } if model == IccColorSpace::Rgb => profile.and_then(icc_as_cicp),
        _ => None,
    };
    let src_has_icc = profile.is_some_and(|profile| {
        IccColorSpace::from_icc(profile) == Some(model)
            && (active_icc
                || match repr_cicp {
                    Some(cicp) => {
                        icc_as_cicp(profile).is_some_and(|icc| cicp_equivalent(icc, cicp))
                    }
                    None => icc_matches_active_color(profile, color),
                })
    });
    let cicp_represents = repr_cicp.is_some();
    let has_carrier = target.cicp() && model == IccColorSpace::Rgb;
    let is_valid_carrier = target.cicp_is_valid_carrier();
    let sole_safe = target.cicp_safe_sole_carrier();
    let icc_is_srgb = src
        .icc_profile
        .as_deref()
        .is_some_and(icc::IccIdentification::is_common_srgb);

    // Whether to emit CICP.
    let emit_cicp = match policy {
        ColorEmitPolicy::Verbatim => {
            has_carrier
                && src
                    .cicp
                    .zip(repr_cicp)
                    .is_some_and(|(src, repr)| cicp_equivalent(src, repr))
        }
        _ => match fields.cicp {
            CicpEmission::Never => false,
            CicpEmission::WhereValidCarrier => has_carrier && is_valid_carrier && cicp_represents,
            CicpEmission::WhereverSupported => has_carrier && cicp_represents,
        },
    };
    let cicp_out = if emit_cicp {
        if policy == ColorEmitPolicy::Verbatim {
            src.cicp
        } else {
            repr_cicp
        }
    } else {
        None
    };

    // Whether to drop the ICC.
    let drop_by_rule = match fields.icc {
        IccRetention::Drop => true,
        IccRetention::Keep => false,
        IccRetention::KeepNonSrgb => icc_is_srgb,
        IccRetention::DropIfCicpRepresentable => emit_cicp && cicp_represents,
        IccRetention::DropIfCicpSafeSoleCarrier => emit_cicp && sole_safe && cicp_represents,
    };
    // Balanced additionally sheds a redundant sRGB ICC even where CICP isn't the
    // sole carrier (the most common pure-weight case).
    let drop_icc = match policy {
        ColorEmitPolicy::Balanced => drop_by_rule || (emit_cicp && icc_is_srgb),
        _ => drop_by_rule,
    };

    // The sRGB default has no ICC to synthesize: the lowering
    // (`zenpixels_convert::icc_profiles::synthesize_icc_for_cicp`) returns
    // `NotNeeded` for primaries ∈ {BT.709(1), unspecified(2)} with transfer ∈
    // {unspecified(2), sRGB(13)}, and for unspecified primaries with the BT.709
    // transfer (nothing to state about the gamut). Mirror that predicate here so
    // we never emit a `SynthesizeFrom` directive that would lower to nothing.
    // BT.709 primaries with the BT.709 transfer (1, 1) is not the sRGB default:
    // the BT.709 OETF and the sRGB curve differ across the whole range (encoded
    // 0.5 is linear 0.2140 under sRGB, 0.2597 under BT.709), so an untagged
    // (1, 1) image is read with the wrong curve (imazen/zenpixels#88).
    let is_assumed_default = |c: &Cicp| {
        (matches!(c.color_primaries, 1 | 2) && matches!(c.transfer_characteristics, 2 | 13))
            || (c.color_primaries == 2 && c.transfer_characteristics == 1)
    };
    let can_synthesize = |c: Cicp| {
        // ICC export describes full-range RGB or gray samples. CICP's matrix and
        // narrow-range coding cannot be baked into that profile.
        c.matrix_coefficients == 0
            && c.full_range
            && (is_gray || c.color_primaries != 10)
            && (matches!(
                c.color_primaries,
                1 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 22
            ) || (is_gray && c.color_primaries == 2))
            && matches!(c.transfer_characteristics, 1 | 4..=17)
            && !is_assumed_default(&c)
    };
    let synthesize = repr_cicp.filter(|&c| {
        target.icc()
            && fields.icc != IccRetention::Drop
            && policy != ColorEmitPolicy::Verbatim
            && can_synthesize(c)
            && (!emit_cicp
                || policy == ColorEmitPolicy::Compatibility
                || (policy == ColorEmitPolicy::Balanced && !sole_safe))
    });
    let icc = if target.icc() && src_has_icc && !drop_icc {
        IccDisposition::KeepSource
    } else if let Some(c) = synthesize {
        if is_gray {
            IccDisposition::SynthesizeGrayFrom(c)
        } else {
            IccDisposition::SynthesizeFrom(c)
        }
    } else {
        IccDisposition::Drop
    };

    let cicp_carries_color = cicp_out
        .zip(repr_cicp)
        .is_some_and(|(out, repr)| cicp_equivalent(out, repr));
    let color_is_default = repr_cicp
        .is_some_and(|c| c.full_range && c.matrix_coefficients == 0 && is_assumed_default(&c));
    if !cicp_carries_color && icc == IccDisposition::Drop && !color_is_default {
        return Err(ColorEmitError::Unrepresentable);
    }
    Ok(ColorEmitPlan {
        cicp: cicp_out,
        icc,
    })
}

fn cicp_equivalent(a: Cicp, b: Cicp) -> bool {
    let is_bt709_transfer = |code| matches!(code, 1 | 6 | 14 | 15);
    a.color_primaries == b.color_primaries
        && a.matrix_coefficients == b.matrix_coefficients
        && a.full_range == b.full_range
        && (a.transfer_characteristics == b.transfer_characteristics
            || (is_bt709_transfer(a.transfer_characteristics)
                && is_bt709_transfer(b.transfer_characteristics)))
}

fn icc_matches_active_color(profile: &[u8], color: &ColorSpace) -> bool {
    let Some(id) = icc::IccIdentification::from_icc(profile) else {
        return false;
    };
    match color {
        ColorSpace::Rgb {
            primaries,
            transfer,
            range: SignalRange::Full,
            ..
        } => id.primaries == *primaries && id.transfer == *transfer,
        ColorSpace::Gray {
            transfer,
            range: SignalRange::Full,
            ..
        } => id.transfer == *transfer,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColorAuthority;

    // Capability fixtures matching the 2026 reliability findings.
    fn caps_jxl() -> EncodeCapabilities {
        EncodeCapabilities::new()
            .with_icc(true)
            .with_cicp(true)
            .with_cicp_is_valid_carrier(true)
            .with_cicp_safe_sole_carrier(true)
    }
    fn caps_avif() -> EncodeCapabilities {
        // AVIF nclx is a valid carrier but NOT sole-safe: vips and ImageMagick
        // render an nclx-only Display P3 AVIF as sRGB (reader-compatibility
        // matrix, imazen/zencodec#132). HEIC has identical caps.
        EncodeCapabilities::new()
            .with_icc(true)
            .with_cicp(true)
            .with_cicp_is_valid_carrier(true)
            .with_cicp_safe_sole_carrier(false)
    }
    fn caps_jpeg() -> EncodeCapabilities {
        // No CICP carrier at all.
        EncodeCapabilities::new().with_icc(true)
    }
    fn caps_png() -> EncodeCapabilities {
        // PNG cICP: a standardized-but-emergent carrier — valid, not sole-safe.
        EncodeCapabilities::new()
            .with_icc(true)
            .with_cicp(true)
            .with_cicp_is_valid_carrier(true)
            .with_cicp_safe_sole_carrier(false)
    }

    fn src_cicp(c: Cicp) -> SourceColor {
        SourceColor::default()
            .with_cicp(c)
            .with_color_authority(ColorAuthority::Cicp)
            .with_channel_count(3)
    }

    fn current_color(source: &SourceColor) -> ColorSpace {
        match source.authoritative() {
            Some(AuthoritativeColor::Icc(profile)) => ColorSpace::Icc {
                profile: profile.clone(),
                channels: 3,
                alpha: zenpixels::AlphaMode::None,
                intent: zenpixels::RenderingIntent::RelativeColorimetric,
            },
            Some(AuthoritativeColor::Cicp(c)) => c
                .to_color_space(zenpixels::AlphaMode::None)
                .unwrap_or_else(|_| {
                    crate::helpers::codec_spec(zenpixels::SampleLayout::RGB8).color
                }),
            None => ColorSpace::SRGB,
        }
    }

    fn rgb_profile(cicp: Cicp) -> alloc::vec::Vec<u8> {
        use zenpixels_convert::icc_profiles::SynthesizedIcc;
        let SynthesizedIcc::Profile(profile) = SynthesizedIcc::for_cicp(cicp) else {
            panic!("test requires a real RGB profile");
        };
        profile.into_owned()
    }

    #[test]
    fn decoded_png_pq_with_diffuse_white_emits_avif_cicp() {
        use zenpixels::{DiffuseWhite, SampleLayout};
        let mut source = src_cicp(Cicp::BT2100_PQ_RGB);
        source.diffuse_white = Some(DiffuseWhite::BT2408);
        let spec =
            crate::helpers::descriptor_for_decoded_pixels_v2(SampleLayout::RGB16, &source, None)
                .unwrap();
        assert_eq!(spec.color.white(), Some(DiffuseWhite::BT2408));
        let plan = resolve_color_emit(&source, &spec.color, &caps_avif(), ColorEmitPolicy::Compact)
            .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::new(9, 16, 0, true)));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn reconstructed_gain_map_linear_color_emits_avif_cicp() {
        use zenpixels::{AlphaMode, DiffuseWhite, ImageSpec, PixelBuf, SampleLayout};
        use zenpixels_convert::{DualLayerHdrView, GainMapApplyConfig, apply_gain_map_with};

        let base = PixelBuf::new(1, 1, ImageSpec::rgb8_srgb()).unwrap();
        let gain = PixelBuf::new(
            1,
            1,
            ImageSpec::new(
                SampleLayout::GRAY8,
                ColorSpace::Untagged {
                    channels: 1,
                    alpha: AlphaMode::None,
                },
            ),
        )
        .unwrap();
        let dual = DualLayerHdrView::new(
            base.as_view(),
            gain.as_view(),
            zenpixels::GainMapMetadata::symmetric(3.0),
            None,
            zenpixels::Orientation::Identity,
        )
        .unwrap();
        let hdr = apply_gain_map_with(
            &dual,
            &GainMapApplyConfig::alternate_rendition(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(hdr.spec().color.white(), Some(DiffuseWhite::BT2408));
        let plan = resolve_color_emit(
            &src_cicp(Cicp::SRGB),
            &hdr.spec().color,
            &caps_avif(),
            ColorEmitPolicy::Compact,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::new(1, 8, 0, true)));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn anchored_gray_color_can_synthesize_gray_icc() {
        let gray = ColorSpace::Gray {
            transfer: Transfer::Linear,
            range: SignalRange::Full,
            alpha: zenpixels::AlphaMode::None,
            white: Some(zenpixels::DiffuseWhite::BT2408),
        };
        let plan = resolve_color_emit(
            &SourceColor::default(),
            &gray,
            &caps_png(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, None);
        assert_eq!(
            plan.icc,
            IccDisposition::SynthesizeGrayFrom(Cicp::new(1, 8, 0, true))
        );
    }

    #[test]
    fn jpeg_keeps_matching_adobe_rgb_profile() {
        use zenpixels_convert::icc_profiles::ADOBE_RGB;
        let source = SourceColor::default().with_icc_profile(ADOBE_RGB);
        let jpeg_plan = |transfer, range| {
            resolve_color_emit(
                &source,
                &ColorSpace::Rgb {
                    primaries: Primaries::AdobeRgb,
                    transfer,
                    range,
                    alpha: zenpixels::AlphaMode::None,
                    white: None,
                },
                &caps_jpeg(),
                ColorEmitPolicy::Balanced,
            )
        };
        let plan = jpeg_plan(Transfer::AdobeRgb, SignalRange::Full).unwrap();
        assert_eq!(plan.icc, IccDisposition::KeepSource);
        assert_eq!(plan.cicp, None);
        assert_eq!(
            jpeg_plan(Transfer::Linear, SignalRange::Full),
            Err(ColorEmitError::Unrepresentable)
        );
        assert_eq!(
            jpeg_plan(Transfer::AdobeRgb, SignalRange::Narrow),
            Err(ColorEmitError::Unrepresentable)
        );
    }

    #[test]
    fn verbatim_preserves_bt709_transfer_aliases() {
        let active = Cicp::new(9, 1, 0, true)
            .to_color_space(zenpixels::AlphaMode::None)
            .unwrap();
        let plan = |raw| {
            resolve_color_emit(
                &src_cicp(raw),
                &active,
                &caps_avif(),
                ColorEmitPolicy::Verbatim,
            )
        };
        for tc in [1, 6, 14, 15] {
            let raw = Cicp::new(9, tc, 0, true);
            assert_eq!(plan(raw).unwrap().cicp, Some(raw));
        }
        for raw in [
            Cicp::new(1, 14, 0, true),
            Cicp::new(9, 14, 9, true),
            Cicp::new(9, 14, 0, false),
            Cicp::new(9, 13, 0, true),
        ] {
            assert_eq!(plan(raw), Err(ColorEmitError::Unrepresentable));
        }
    }

    #[test]
    fn jxl_balanced_strips_representable_icc() {
        // JXL (sole-safe) with CICP declared authoritative + a redundant ICC
        // companion → emit CICP, drop the ICC (matches libjxl's want_icc=false
        // default). Dropping is safe because CICP is the *declared* truth here.
        let src = SourceColor::default()
            .with_cicp(Cicp::SRGB)
            .with_icc_profile(rgb_profile(Cicp::DISPLAY_P3))
            .with_color_authority(ColorAuthority::Cicp)
            .with_channel_count(3);
        let plan = resolve_color_emit(
            &src,
            &current_color(&src),
            &caps_jxl(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::SRGB));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn authoritative_icc_not_dropped_for_roundtrip_cicp() {
        // Bug guard (review HIGH #2): an authoritative ICC (the default authority)
        // co-existing with a CICP the spec treats as a round-trip companion must
        // NOT be dropped on a sole-safe target in favor of that CICP — doing so
        // mis-tags / precision-loses when the CICP doesn't represent the ICC. Here
        // the ICC isn't identifiable as code points, so no CICP can stand in for it.
        let mut opaque_profile = alloc::vec![0; 132];
        opaque_profile[16..20].copy_from_slice(b"RGB ");
        opaque_profile[36..40].copy_from_slice(b"acsp");
        let src = SourceColor::default()
            .with_cicp(Cicp::SRGB)
            .with_icc_profile(opaque_profile) // present, unidentifiable, authoritative
            .with_color_authority(ColorAuthority::Icc)
            .with_channel_count(3);
        let plan = resolve_color_emit(
            &src,
            &current_color(&src),
            &caps_jxl(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(
            plan.icc,
            IccDisposition::KeepSource,
            "authoritative ICC must survive"
        );
        assert_eq!(
            plan.cicp, None,
            "an unidentified authoritative profile has no equivalent CICP"
        );
    }

    #[test]
    fn avif_balanced_keeps_wide_gamut_icc_next_to_nclx() {
        // AVIF nclx is not sole-safe (vips and ImageMagick act only on the ICC,
        // imazen/zencodec#132): a non-sRGB ICC is kept next to the nclx under
        // Balanced. The sole-safe drop is JXL's (`jxl_balanced_strips_representable_icc`).
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(rgb_profile(Cicp::DISPLAY_P3));
        let plan = resolve_color_emit(
            &p3,
            &current_color(&p3),
            &caps_avif(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::KeepSource);
    }

    #[test]
    fn jpeg_synthesizes_icc_from_cicp() {
        // CICP-only source → JPEG (no CICP carrier): synthesize an ICC.
        let src = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(
            &src,
            &current_color(&src),
            &caps_jpeg(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, None);
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn compact_strips_icc_on_avif() {
        // Compact drops the ICC wherever CICP represents the color, even on AVIF
        // (where vips and ImageMagick then show sRGB: the documented cost).
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(rgb_profile(Cicp::DISPLAY_P3));
        let plan = resolve_color_emit(
            &p3,
            &current_color(&p3),
            &caps_avif(),
            ColorEmitPolicy::Compact,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn compatibility_always_keeps_or_synthesizes_icc() {
        // CICP-only source, AVIF, Compatibility → CICP emitted AND an ICC synthesized.
        let src = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(
            &src,
            &current_color(&src),
            &caps_avif(),
            ColorEmitPolicy::Compatibility,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn gray_and_gray_alpha_require_gray_profiles() {
        use zenpixels::{AlphaMode, Transfer};
        use zenpixels_convert::icc_profiles::SynthesizedIcc;
        let source = src_cicp(Cicp::new(1, 8, 0, true));
        for alpha in [AlphaMode::None, AlphaMode::Straight] {
            let color = ColorSpace::Gray {
                transfer: Transfer::Linear,
                alpha,
                range: SignalRange::Full,
                white: None,
            };
            let plan = resolve_color_emit(&source, &color, &caps_png(), ColorEmitPolicy::Balanced)
                .unwrap();
            assert_eq!(plan.cicp, None);
            let IccDisposition::SynthesizeGrayFrom(cicp) = plan.icc else {
                panic!("expected GRAY profile")
            };
            let SynthesizedIcc::Profile(icc) = SynthesizedIcc::for_gray_cicp(cicp) else {
                panic!("GRAY profile unavailable")
            };
            assert_eq!(&icc[16..20], b"GRAY");
        }
    }

    #[test]
    fn unsupported_carriers_require_refusal_or_native_cicp() {
        use zenpixels_convert::icc_profiles::SynthesizedIcc;
        let no_carrier = EncodeCapabilities::new();
        for policy in [
            ColorEmitPolicy::Balanced,
            ColorEmitPolicy::Compact,
            ColorEmitPolicy::Compatibility,
            ColorEmitPolicy::Verbatim,
        ] {
            for source in [
                src_cicp(Cicp::DISPLAY_P3),
                src_cicp(Cicp::DISPLAY_P3).with_icc_profile(rgb_profile(Cicp::DISPLAY_P3)),
            ] {
                assert_eq!(
                    resolve_color_emit(&source, &current_color(&source), &no_carrier, policy),
                    Err(ColorEmitError::Unrepresentable)
                );
            }
        }
        let hlg = src_cicp(Cicp::BT2100_HLG_RGB);
        assert_eq!(
            SynthesizedIcc::for_cicp(Cicp::BT2100_HLG_RGB),
            SynthesizedIcc::CmsUnsupported
        );
        assert_eq!(
            resolve_color_emit(
                &hlg,
                &current_color(&hlg),
                &caps_jpeg(),
                ColorEmitPolicy::Balanced
            ),
            Err(ColorEmitError::Unrepresentable)
        );
        let plan = resolve_color_emit(
            &hlg,
            &current_color(&hlg),
            &caps_png(),
            ColorEmitPolicy::Compatibility,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::BT2100_HLG_RGB));
        assert_eq!(plan.icc, IccDisposition::Drop);
        let drop = ColorEmitPolicy::Custom(ColorEmitFields::new(
            IccRetention::Drop,
            CicpEmission::Never,
        ));
        assert_eq!(
            resolve_color_emit(
                &src_cicp(Cicp::DISPLAY_P3),
                &current_color(&src_cicp(Cicp::DISPLAY_P3)),
                &caps_png(),
                drop
            ),
            Err(ColorEmitError::Unrepresentable)
        );

        let plan = resolve_color_emit(
            &src_cicp(Cicp::DISPLAY_P3),
            &current_color(&src_cicp(Cicp::DISPLAY_P3)),
            &caps_jpeg(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        let IccDisposition::SynthesizeFrom(cicp) = plan.icc else {
            panic!("expected RGB profile")
        };
        let SynthesizedIcc::Profile(icc) = SynthesizedIcc::for_cicp(cicp) else {
            panic!("RGB profile unavailable")
        };
        assert_eq!(&icc[16..20], b"RGB ");
    }

    #[test]
    fn corrected_pixels_do_not_reuse_conflicting_source_profile() {
        let source = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(rgb_profile(Cicp::DISPLAY_P3));
        let plan = resolve_color_emit(
            &source,
            &ColorSpace::SRGB,
            &caps_png(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::SRGB));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn verbatim_passes_source_through() {
        // Verbatim keeps both, derives nothing.
        let src = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(rgb_profile(Cicp::DISPLAY_P3));
        let plan = resolve_color_emit(
            &src,
            &current_color(&src),
            &caps_avif(),
            ColorEmitPolicy::Verbatim,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::KeepSource);
    }

    #[test]
    fn default_policy_is_balanced() {
        assert_eq!(ColorEmitPolicy::default(), ColorEmitPolicy::Balanced);
    }

    #[test]
    fn png_emits_cicp_keeps_icc_under_balanced() {
        // PNG: standardized cICP carrier but not sole-safe → emit cICP AND keep
        // iCCP. Regression for the missing valid-carrier tier — a non-authority
        // carrier must still emit CICP under Balanced.
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(rgb_profile(Cicp::DISPLAY_P3));
        let plan = resolve_color_emit(
            &p3,
            &current_color(&p3),
            &caps_png(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::KeepSource);
    }

    #[test]
    fn srgb_only_source_does_not_synthesize_redundant_icc() {
        // CICP-only sRGB → JPEG (no carrier): sRGB is the assumed default and the
        // canned table has no sRGB profile → drop, never SynthesizeFrom(sRGB).
        let src = src_cicp(Cicp::SRGB);
        let plan = resolve_color_emit(
            &src,
            &current_color(&src),
            &caps_jpeg(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, None);
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn bt709_transfer_is_not_the_srgb_default() {
        // (1, 1): BT.709 primaries with the BT.709 transfer is a different curve
        // from sRGB, so a target with no CICP carrier needs a profile for it.
        // (1, 13) and (2, 1) stay the assumed default.
        let bt709 = src_cicp(Cicp::new(1, 1, 0, true));
        let plan = resolve_color_emit(
            &bt709,
            &current_color(&bt709),
            &caps_jpeg(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(
            plan.icc,
            IccDisposition::SynthesizeFrom(Cicp::new(1, 1, 0, true))
        );
        for c in [
            Cicp::new(1, 13, 0, true),
            Cicp::new(2, 1, 0, true),
            Cicp::new(2, 2, 0, true),
        ] {
            let plan = resolve_color_emit(
                &src_cicp(c),
                &current_color(&src_cicp(c)),
                &caps_jpeg(),
                ColorEmitPolicy::Balanced,
            )
            .unwrap();
            assert_eq!(plan.icc, IccDisposition::Drop, "{c:?}");
        }
    }

    #[test]
    fn custom_policy_is_constructible() {
        // ColorEmitFields::new makes ColorEmitPolicy::Custom reachable from downstream.
        let policy = ColorEmitPolicy::Custom(ColorEmitFields::new(
            IccRetention::Keep,
            CicpEmission::Never,
        ));
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(rgb_profile(Cicp::DISPLAY_P3));
        let plan = resolve_color_emit(&p3, &current_color(&p3), &caps_avif(), policy).unwrap();
        assert_eq!(plan.cicp, None); // CicpEmission::Never
        assert_eq!(plan.icc, IccDisposition::KeepSource); // IccRetention::Keep
    }

    #[test]
    fn png_balanced_synthesizes_icc_companion_for_cicp_only() {
        // CICP-only (no source ICC) → PNG under Balanced: cICP is a valid but NOT
        // sole-safe carrier, so synthesize a companion ICC rather than ship cICP alone.
        let p3 = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(
            &p3,
            &current_color(&p3),
            &caps_png(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn avif_balanced_cicp_only_synthesizes_a_companion_icc() {
        // AVIF nclx alone is ignored by vips and ImageMagick (imazen/zencodec#132),
        // so a CICP-only source gets a synthesized ICC next to the nclx, as PNG
        // does. Only JXL ships CICP alone (`jxl_balanced_cicp_only_needs_no_companion`).
        let p3 = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(
            &p3,
            &current_color(&p3),
            &caps_avif(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn png_compact_stays_cicp_only_for_cicp_only() {
        // Compact is "smallest": it accepts cICP-only even on a non-sole-safe carrier,
        // so it must NOT synthesize a companion (guards against over-synth).
        let p3 = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(
            &p3,
            &current_color(&p3),
            &caps_png(),
            ColorEmitPolicy::Compact,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn srgb_cicp_only_png_balanced_does_not_synthesize() {
        // sRGB is the assumed default and the canned table has no sRGB ICC, so even on
        // a non-sole-safe carrier Balanced must not synthesize a redundant companion.
        let srgb = src_cicp(Cicp::SRGB);
        let plan = resolve_color_emit(
            &srgb,
            &current_color(&srgb),
            &caps_png(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn jxl_balanced_cicp_only_needs_no_companion() {
        // JXL enum color IS sole-safe → a CICP-only source needs no companion ICC.
        let p3 = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(
            &p3,
            &current_color(&p3),
            &caps_jxl(),
            ColorEmitPolicy::Balanced,
        )
        .unwrap();
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }
}
