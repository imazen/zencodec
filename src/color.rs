//! Internal: color-signaling emission policy (ICC profile vs CICP code points).
//!
//! This module is private — its types (`ColorEmitPolicy`, `ColorEmitFields`,
//! `ColorEmitPlan`, `IccDisposition`, `CicpEmission`) and `resolve_color_emit`
//! are re-exported at the crate root. The public overview lives on
//! `ColorEmitPolicy`; the full per-format design is in `docs/color-emit-model.md`.

use zenpixels::icc;
use zenpixels::{Cicp, ColorAuthority, ColorModel};

use crate::capabilities::EncodeCapabilities;
use crate::info::SourceColor;
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
/// fields onto its own container carriers as listed below. No type-enforced
/// atomic step ties the emitted color to the pixel buffer — the `SourceColor`
/// handed to [`resolve_color_emit`] comes from the caller's `Metadata`, a
/// separate input from the pixels, and is not cross-checked against the
/// buffer's descriptor or `ColorContext`. Keeping the two in agreement is the
/// caller's job (the pipeline builds `Metadata` from the same decode that
/// produced the pixels); a mislabeled source is emitted faithfully wrong.
///
/// - [`ColorEmitPlan::cicp`] → the format's native CICP carrier (JXL enum color,
///   AVIF/HEIC `nclx`, PNG `cICP`).
/// - [`IccDisposition::KeepSource`] → re-embed the source ICC bytes unchanged.
/// - [`IccDisposition::SynthesizeFrom`]`(cicp)` → lower through
///   `zenpixels_convert::icc_profiles::synthesize_icc_for_cicp(cicp)`, which is
///   **transfer-aware** (it won't hand a BT.2020-PQ source the SDR-TRC Rec.2020
///   profile) and returns a typed `SynthesizedIcc`: embed the bytes on `Profile`;
///   on `NotNeeded`/`NeedsCms`/`CmsUnsupported` embed no ICC and let
///   [`ColorEmitPlan::cicp`] carry the color. Coverage is best-effort — the
///   bundled `const` set (no CMS) reaches Display-P3 and SDR BT.2020; the
///   `cms-moxcms` feature generates the rest, including PQ/HLG. See
///   [`SynthesizeFrom`](IccDisposition::SynthesizeFrom) for the full contract.
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
    /// Embed an ICC synthesized from this CICP (target has no CICP carrier, or
    /// the policy wants an ICC alongside). The caller materializes the bytes —
    /// this `no_std`, CMS-less crate only states the intent.
    ///
    /// **Recommended lowering:** `zenpixels_convert::icc_profiles`'s
    /// `synthesize_icc_for_cicp(cicp)`, which is transfer-aware and returns a typed
    /// `SynthesizedIcc`:
    /// - `Profile(bytes)` — embed them (bundled `&'static`, or `cms-moxcms`-generated).
    /// - `NotNeeded` — the CICP is the sRGB/BT.709 default; embed nothing. (The
    ///   resolver won't normally emit `SynthesizeFrom(sRGB)`, so this is rare.)
    /// - `NeedsCms` — no bundled profile matches and the `cms-moxcms` feature is
    ///   off; embed nothing.
    /// - `CmsUnsupported` — the CMS can't represent this CICP; embed nothing.
    ///
    /// **Synthesis is best-effort.** Bundled coverage (no CMS) is Display-P3 and
    /// SDR BT.2020; `cms-moxcms` extends it to anything moxcms recognizes,
    /// including PQ/HLG. On any non-`Profile` outcome the lowering must **not**
    /// fabricate or mis-tag a profile — it embeds no ICC and relies on
    /// [`ColorEmitPlan::cicp`] to carry the color. For a target with no CICP
    /// carrier, that means a caller without `cms-moxcms` may lose a wide-gamut
    /// description it can't bundle; prefer a CICP-carrying format or enable the
    /// feature when exact wide-gamut/HDR color must survive.
    SynthesizeFrom(Cicp),
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

/// The CICP that describes this source's color as code points, if any.
///
/// Authority-aware (per [`ColorAuthority`]): when the **ICC** is authoritative and
/// present, the emittable CICP must *describe that ICC* — derived from its `cicp`
/// tag, then the corpus — **not** a co-existing CICP, which the spec treats as a
/// backwards-compat round-trip companion that "should NOT override the TRC." If the
/// ICC isn't identifiable as code points, this returns `None`, so the resolver keeps
/// the authoritative ICC rather than dropping it for a CICP that doesn't represent
/// it (precision loss / mis-tag on a sole-safe carrier). When CICP is authoritative
/// (or ICC is absent), the explicit CICP wins, then the ICC-derived fallback.
fn representable_cicp(src: &SourceColor) -> Option<Cicp> {
    let from_icc = |icc_bytes: &[u8]| -> Option<Cicp> {
        icc::extract_cicp(icc_bytes)
            .or_else(|| icc::identify_common(icc_bytes).and_then(|id| id.to_cicp()))
    };
    if src.color_authority == ColorAuthority::Icc
        && let Some(icc_bytes) = src.icc_profile.as_deref()
    {
        return from_icc(icc_bytes);
    }
    // CICP authoritative (or ICC declared authoritative but absent — the lenient
    // fallback the contract allows): explicit CICP first, then the ICC-derived one.
    if let Some(c) = src.cicp {
        return Some(c);
    }
    from_icc(src.icc_profile.as_deref()?)
}

/// Reconcile a source's color description against a target's capabilities under
/// a [`ColorEmitPolicy`], returning what to emit.
///
/// Pure and `no_std`. Decides ICC vs CICP emission, including the grayscale /
/// CMYK terminal states (where CICP is inapplicable and the ICC must be kept).
pub fn resolve_color_emit(
    src: &SourceColor,
    target: &EncodeCapabilities,
    policy: ColorEmitPolicy,
) -> ColorEmitPlan {
    let fields = policy.fields();
    let src_has_icc = src.icc_profile.is_some();

    // Grayscale / CMYK: CICP is RGB-centric and cannot describe these. Keep the
    // ICC (the only valid color description) and suppress CICP — emitting an RGB
    // CICP over gray/CMYK pixels would recolor them.
    let model = src
        .icc_profile
        .as_deref()
        .and_then(icc::profile_color_space);
    let is_gray = matches!(model, Some(ColorModel::Gray)) || src.channel_count == Some(1);
    let is_cmyk = matches!(model, Some(ColorModel::Cmyk));
    if is_gray || is_cmyk {
        return ColorEmitPlan {
            cicp: None,
            icc: if src_has_icc {
                IccDisposition::KeepSource
            } else {
                IccDisposition::Drop
            },
        };
    }

    let repr_cicp = representable_cicp(src);
    let cicp_represents = repr_cicp.is_some();
    let has_carrier = target.cicp();
    let is_valid_carrier = target.cicp_is_valid_carrier();
    let sole_safe = target.cicp_safe_sole_carrier();
    let icc_is_srgb = src.icc_profile.as_deref().is_some_and(icc::is_common_srgb);

    // Whether to emit CICP.
    let emit_cicp = match policy {
        ColorEmitPolicy::Verbatim => has_carrier && src.cicp.is_some(),
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
    let synth_worthwhile = repr_cicp.is_some_and(|c| !is_assumed_default(&c));

    let icc = if src_has_icc {
        if drop_icc {
            IccDisposition::Drop
        } else {
            IccDisposition::KeepSource
        }
    } else if !emit_cicp && synth_worthwhile && policy != ColorEmitPolicy::Verbatim {
        // No source ICC and CICP isn't carrying the color (target is ICC-only):
        // synthesize an ICC so the (non-default) color isn't lost.
        IccDisposition::SynthesizeFrom(repr_cicp.expect("synth_worthwhile"))
    } else if synth_worthwhile
        && (matches!(policy, ColorEmitPolicy::Compatibility)
            || (matches!(policy, ColorEmitPolicy::Balanced) && !sole_safe))
    {
        // Compatibility always wants an ICC alongside CICP; Balanced synthesizes a
        // companion when the CICP carrier isn't sole-safe (PNG cICP, AVIF/HEIC nclx)
        // so the color survives decoders that ignore the carrier — symmetric with
        // keeping a source ICC there. Compact accepts CICP-only (smallest); Verbatim
        // derives nothing. (non-sRGB only — see `synth_worthwhile`.)
        IccDisposition::SynthesizeFrom(repr_cicp.expect("synth_worthwhile"))
    } else {
        IccDisposition::Drop
    };

    ColorEmitPlan {
        cicp: cicp_out,
        icc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenpixels::ColorAuthority;

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

    #[test]
    fn jxl_balanced_strips_representable_icc() {
        // JXL (sole-safe) with CICP declared authoritative + a redundant ICC
        // companion → emit CICP, drop the ICC (matches libjxl's want_icc=false
        // default). Dropping is safe because CICP is the *declared* truth here.
        let src = SourceColor::default()
            .with_cicp(Cicp::SRGB)
            .with_icc_profile(alloc::vec![0u8; 132])
            .with_color_authority(ColorAuthority::Cicp)
            .with_channel_count(3);
        let plan = resolve_color_emit(&src, &caps_jxl(), ColorEmitPolicy::Balanced);
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
        let src = SourceColor::default()
            .with_cicp(Cicp::SRGB)
            .with_icc_profile(alloc::vec![0u8; 132]) // present, unidentifiable, authoritative
            .with_color_authority(ColorAuthority::Icc)
            .with_channel_count(3);
        let plan = resolve_color_emit(&src, &caps_jxl(), ColorEmitPolicy::Balanced);
        assert_eq!(
            plan.icc,
            IccDisposition::KeepSource,
            "authoritative ICC must survive"
        );
        assert_eq!(
            plan.cicp, None,
            "no CICP can represent the unidentifiable authoritative ICC"
        );
    }

    #[test]
    fn avif_balanced_keeps_wide_gamut_icc_next_to_nclx() {
        // AVIF nclx is not sole-safe (vips and ImageMagick act only on the ICC,
        // imazen/zencodec#132): a non-sRGB ICC is kept next to the nclx under
        // Balanced. The sole-safe drop is JXL's (`jxl_balanced_strips_representable_icc`).
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(alloc::vec![0u8; 132]);
        let plan = resolve_color_emit(&p3, &caps_avif(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::KeepSource);
    }

    #[test]
    fn jpeg_synthesizes_icc_from_cicp() {
        // CICP-only source → JPEG (no CICP carrier): synthesize an ICC.
        let src = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(&src, &caps_jpeg(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.cicp, None);
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn compact_strips_icc_on_avif() {
        // Compact drops the ICC wherever CICP represents the color, even on AVIF
        // (where vips and ImageMagick then show sRGB: the documented cost).
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(alloc::vec![0u8; 132]);
        let plan = resolve_color_emit(&p3, &caps_avif(), ColorEmitPolicy::Compact);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn compatibility_always_keeps_or_synthesizes_icc() {
        // CICP-only source, AVIF, Compatibility → CICP emitted AND an ICC synthesized.
        let src = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(&src, &caps_avif(), ColorEmitPolicy::Compatibility);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn grayscale_keeps_icc_suppresses_cicp() {
        // A 1-channel source: CICP is inapplicable; keep ICC, suppress CICP.
        let src = SourceColor::default()
            .with_icc_profile(alloc::vec![0u8; 132])
            .with_channel_count(1);
        let plan = resolve_color_emit(&src, &caps_avif(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.cicp, None);
        assert_eq!(plan.icc, IccDisposition::KeepSource);
    }

    #[test]
    fn verbatim_passes_source_through() {
        // Verbatim keeps both, derives nothing.
        let src = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(alloc::vec![0u8; 132]);
        let plan = resolve_color_emit(&src, &caps_avif(), ColorEmitPolicy::Verbatim);
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
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(alloc::vec![0u8; 132]);
        let plan = resolve_color_emit(&p3, &caps_png(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::KeepSource);
    }

    #[test]
    fn srgb_only_source_does_not_synthesize_redundant_icc() {
        // CICP-only sRGB → JPEG (no carrier): sRGB is the assumed default and the
        // canned table has no sRGB profile → drop, never SynthesizeFrom(sRGB).
        let src = src_cicp(Cicp::SRGB);
        let plan = resolve_color_emit(&src, &caps_jpeg(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.cicp, None);
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn bt709_transfer_is_not_the_srgb_default() {
        // (1, 1): BT.709 primaries with the BT.709 transfer is a different curve
        // from sRGB, so a target with no CICP carrier needs a profile for it.
        // (1, 13) and (2, 1) stay the assumed default.
        let bt709 = src_cicp(Cicp::new(1, 1, 0, true));
        let plan = resolve_color_emit(&bt709, &caps_jpeg(), ColorEmitPolicy::Balanced);
        assert_eq!(
            plan.icc,
            IccDisposition::SynthesizeFrom(Cicp::new(1, 1, 0, true))
        );
        for c in [
            Cicp::new(1, 13, 0, true),
            Cicp::new(2, 1, 0, true),
            Cicp::new(2, 2, 0, true),
        ] {
            let plan = resolve_color_emit(&src_cicp(c), &caps_jpeg(), ColorEmitPolicy::Balanced);
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
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(alloc::vec![0u8; 132]);
        let plan = resolve_color_emit(&p3, &caps_avif(), policy);
        assert_eq!(plan.cicp, None); // CicpEmission::Never
        assert_eq!(plan.icc, IccDisposition::KeepSource); // IccRetention::Keep
    }

    #[test]
    fn png_balanced_synthesizes_icc_companion_for_cicp_only() {
        // CICP-only (no source ICC) → PNG under Balanced: cICP is a valid but NOT
        // sole-safe carrier, so synthesize a companion ICC rather than ship cICP alone.
        let p3 = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(&p3, &caps_png(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn avif_balanced_cicp_only_synthesizes_a_companion_icc() {
        // AVIF nclx alone is ignored by vips and ImageMagick (imazen/zencodec#132),
        // so a CICP-only source gets a synthesized ICC next to the nclx, as PNG
        // does. Only JXL ships CICP alone (`jxl_balanced_cicp_only_needs_no_companion`).
        let p3 = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(&p3, &caps_avif(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn png_compact_stays_cicp_only_for_cicp_only() {
        // Compact is "smallest": it accepts cICP-only even on a non-sole-safe carrier,
        // so it must NOT synthesize a companion (guards against over-synth).
        let p3 = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(&p3, &caps_png(), ColorEmitPolicy::Compact);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn srgb_cicp_only_png_balanced_does_not_synthesize() {
        // sRGB is the assumed default and the canned table has no sRGB ICC, so even on
        // a non-sole-safe carrier Balanced must not synthesize a redundant companion.
        let srgb = src_cicp(Cicp::SRGB);
        let plan = resolve_color_emit(&srgb, &caps_png(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn jxl_balanced_cicp_only_needs_no_companion() {
        // JXL enum color IS sole-safe → a CICP-only source needs no companion ICC.
        let p3 = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(&p3, &caps_jxl(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }
}
