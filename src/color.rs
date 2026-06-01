//! Color-signaling production policy: how an image's color *description*
//! (ICC profile vs CICP code points) is emitted when encoding or transcoding.
//!
//! This is orthogonal to which *pixels* are written. Containers differ in which
//! color carriers they have and in how reliably real-world decoders honor each
//! one, so emitting "the right" color description is a per-target decision.
//!
//! # The obvious knob: [`ColorPolicy`]
//!
//! Pick an intent — the same meaning whether encoding from pixels or transcoding
//! from another file:
//!
//! - [`Compatibility`](ColorPolicy::Compatibility) — always embed an ICC; add CICP where reliable.
//! - [`Balanced`](ColorPolicy::Balanced) (**default**) — emit CICP where it's the format's authority,
//!   drop a redundant ICC only where CICP is safe as the sole carrier (JXL today) or the ICC is plain sRGB.
//! - [`Compact`](ColorPolicy::Compact) — smallest: prefer CICP wherever the format carries it, drop the ICC.
//! - [`Verbatim`](ColorPolicy::Verbatim) — carry the source's signals unchanged.
//! - [`Custom`](ColorPolicy::Custom) — explicit [`ColorFields`] for power users.
//!
//! # The resolver: [`resolve_color_emit`]
//!
//! [`resolve_color_emit`] reconciles a [`SourceColor`] against a target's
//! [`EncodeCapabilities`] under a [`ColorPolicy`] and returns a [`ColorPlan`] —
//! a pure description of what to emit. This crate is `no_std` and carries no
//! CMS, so the plan only describes intent ([`IccDisposition::SynthesizeFrom`],
//! etc.); the bytes are materialized one layer up.

use alloc::vec::Vec;

use zenpixels::icc;
use zenpixels::{Cicp, ColorModel, SignalRange};

use crate::capabilities::EncodeCapabilities;
use crate::info::SourceColor;
use crate::metadata::IccRetention;

/// How color description is emitted on encode — the obvious, intent-named knob.
///
/// See the [module docs](self) for the per-format behavior table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColorPolicy {
    /// Widest compatibility: always embed an ICC profile (synthesizing one from
    /// CICP when the source had none); add CICP where the format treats it as
    /// authority. Largest color overhead.
    Compatibility,
    /// **Default.** Emit CICP where it is the format's authority and drop a
    /// redundant ICC only where CICP is safe as the *sole* carrier
    /// ([`cicp_safe_sole_carrier`](EncodeCapabilities::cicp_safe_sole_carrier) —
    /// JXL today) or the ICC is a plain sRGB profile. Otherwise keep the ICC.
    #[default]
    Balanced,
    /// Smallest color overhead: prefer CICP wherever the format can carry it at
    /// all, and drop the ICC whenever CICP can describe the color.
    Compact,
    /// Carry the source's color signals through unchanged — derive and strip
    /// nothing. For transcodes that must preserve exactly what was there.
    Verbatim,
    /// Explicit mechanism control.
    Custom(ColorFields),
}

/// Whether CICP is emitted, behind [`ColorPolicy::Custom`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CicpEmission {
    /// Emit CICP where the format treats it as the authoritative color signal
    /// ([`cicp_is_format_authority`](EncodeCapabilities::cicp_is_format_authority)).
    #[default]
    WhereFormatAuthority,
    /// Emit CICP wherever the format has a carrier, even if not authoritative.
    WhereverSupported,
    /// Never emit CICP (ICC-only output).
    Never,
}

/// Mechanism fields behind [`ColorPolicy::Custom`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColorFields {
    /// When to drop the ICC profile.
    pub icc: IccRetention,
    /// Whether to emit CICP.
    pub cicp: CicpEmission,
}

impl Default for ColorFields {
    fn default() -> Self {
        Self {
            icc: IccRetention::DropIfCicpSafeSoleCarrier,
            cicp: CicpEmission::WhereFormatAuthority,
        }
    }
}

impl ColorPolicy {
    /// Resolve a preset to its mechanism fields.
    pub const fn fields(&self) -> ColorFields {
        match self {
            Self::Compatibility => ColorFields {
                icc: IccRetention::Keep,
                cicp: CicpEmission::WhereFormatAuthority,
            },
            Self::Balanced => ColorFields {
                icc: IccRetention::DropIfCicpSafeSoleCarrier,
                cicp: CicpEmission::WhereFormatAuthority,
            },
            Self::Compact => ColorFields {
                icc: IccRetention::DropIfCicpRepresentable,
                cicp: CicpEmission::WhereverSupported,
            },
            Self::Verbatim => ColorFields {
                icc: IccRetention::Keep,
                cicp: CicpEmission::WhereFormatAuthority,
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
    /// the policy wants an ICC alongside). The caller materializes the bytes.
    SynthesizeFrom(Cicp),
    /// Emit no ICC profile.
    Drop,
}

/// What to do with an attached HDR gain map.
///
/// Dropping a gain map is **not** tone-mapping: a gain-map image's base is a
/// complete rendition. See [`crate::gainmap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum GainMapDisposition {
    /// No gain map present.
    #[default]
    None,
    /// Target carries gain maps: re-wrap the payload into the target encoder's
    /// own canonical box and re-encode the gain-map image.
    Rewrap,
    /// `BaseIsSdr` (common) and the target can't carry a gain map: drop the map
    /// and keep the base verbatim — it is already the SDR rendition. No pixel
    /// math, no signaling change.
    DropKeepSdrBase,
    /// Rare `BaseIsHdr`/subtractive map with an SDR-only target: apply the stored
    /// gain ratio to recover the SDR alternate (not tone-mapping), then drop.
    ApplyToRecoverSdr,
}

/// What to do with static HDR metadata (CLLI / MDCV).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum HdrStaticDisposition {
    /// Carry CLLI / MDCV through unchanged.
    #[default]
    Keep,
    /// Drop CLLI / MDCV — the pixels were tone-mapped from transfer-function HDR
    /// (PQ/HLG) to SDR, so the HDR luminance metadata no longer describes them.
    DropForSdr,
}

/// ICC rendering intent (ICC profile header bytes 64..68).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderingIntent {
    /// Perceptual (0) — the common default for images.
    Perceptual,
    /// Media-relative colorimetric (1).
    RelativeColorimetric,
    /// Saturation (2).
    Saturation,
    /// ICC-absolute colorimetric (3).
    AbsoluteColorimetric,
}

impl RenderingIntent {
    /// Parse from the ICC intent code (0..3).
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Perceptual),
            1 => Some(Self::RelativeColorimetric),
            2 => Some(Self::Saturation),
            3 => Some(Self::AbsoluteColorimetric),
            _ => None,
        }
    }
}

/// A non-fatal observation about a [`ColorPlan`], for the (future) encode-side
/// warnings channel. Surfacing these is how lossy/degenerate color handling
/// becomes visible instead of silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColorNote {
    /// Grayscale color has no CICP encoding — the ICC was kept, CICP suppressed.
    /// Emitting an RGB CICP over gray pixels would recolor them.
    GrayNotCicpRepresentable,
    /// CMYK is not describable by CICP — the ICC was kept; CICP is inapplicable.
    CmykCicpInapplicable,
    /// A non-default ICC rendering intent was dropped along with the profile
    /// (CICP has no rendering-intent slot).
    RenderingIntentDropped,
    /// A limited/narrow signal range could not be carried by the synthesized ICC.
    RangeDroppedOnSynthesis,
    /// The source ICC could not be reduced to CICP (unrecognized, no `cicp` tag),
    /// so it was kept even though the target prefers CICP.
    IccKeptCicpUnrepresentable,
}

/// A resolved plan for emitting an image's color description on encode.
///
/// Produced by [`resolve_color_emit`]. `gain_map` and `hdr_static` default to
/// no-op here (they need the gain-map/HDR state from
/// [`ImageInfo`](crate::ImageInfo), not just [`SourceColor`]); a higher-level
/// resolver fills them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorPlan {
    /// CICP to write to the target's native carrier, if any.
    pub cicp: Option<Cicp>,
    /// Disposition of the ICC profile channel.
    pub icc: IccDisposition,
    /// Signal range to carry (so a synthesized ICC / target can't silently lose
    /// limited-range and crush blacks). `None` when unknown.
    pub range: Option<SignalRange>,
    /// Rendering intent recovered from the source ICC, if any.
    pub rendering_intent: Option<RenderingIntent>,
    /// Disposition of static HDR metadata (CLLI / MDCV).
    pub hdr_static: HdrStaticDisposition,
    /// Disposition of an attached gain map.
    pub gain_map: GainMapDisposition,
    /// Non-fatal observations (lossy/degenerate handling) for the warnings channel.
    pub notes: Vec<ColorNote>,
}

/// Read the ICC rendering-intent field (header bytes 64..68, big-endian u32).
fn read_icc_intent(icc_bytes: &[u8]) -> Option<RenderingIntent> {
    let b: [u8; 4] = icc_bytes.get(64..68)?.try_into().ok()?;
    RenderingIntent::from_code(u32::from_be_bytes(b))
}

/// The CICP that describes this source's color as code points, if any:
/// the explicit CICP, else derived from the ICC (`cicp` tag, then corpus).
fn representable_cicp(src: &SourceColor) -> Option<Cicp> {
    if let Some(c) = src.cicp {
        return Some(c);
    }
    let icc_bytes = src.icc_profile.as_ref()?;
    icc::extract_cicp(icc_bytes)
        .or_else(|| icc::identify_common(icc_bytes).and_then(|id| id.to_cicp()))
}

/// Reconcile a source's color description against a target's capabilities under
/// a [`ColorPolicy`], returning what to emit.
///
/// Pure and `no_std`. Handles the ICC/CICP/range/intent axis and the grayscale /
/// CMYK terminal states (where CICP is inapplicable and the ICC must be kept).
/// `gain_map` and `hdr_static` are left at their no-op defaults — fill them from
/// the [`ImageInfo`](crate::ImageInfo) gain-map state at a higher layer.
pub fn resolve_color_emit(
    src: &SourceColor,
    target: &EncodeCapabilities,
    policy: ColorPolicy,
) -> ColorPlan {
    let fields = policy.fields();
    let src_has_icc = src.icc_profile.is_some();
    let intent = src.icc_profile.as_deref().and_then(read_icc_intent);
    let range = src.cicp.map(|c| {
        if c.full_range {
            SignalRange::Full
        } else {
            SignalRange::Narrow
        }
    });

    // Grayscale / CMYK: CICP is RGB-centric and cannot describe these. Keep the
    // ICC (the only valid color description) and suppress CICP.
    let model = src
        .icc_profile
        .as_deref()
        .and_then(icc::profile_color_space);
    let is_gray = matches!(model, Some(ColorModel::Gray)) || src.channel_count == Some(1);
    let is_cmyk = matches!(model, Some(ColorModel::Cmyk));
    if is_gray || is_cmyk {
        let mut notes = Vec::new();
        notes.push(if is_cmyk {
            ColorNote::CmykCicpInapplicable
        } else {
            ColorNote::GrayNotCicpRepresentable
        });
        return ColorPlan {
            cicp: None,
            icc: if src_has_icc {
                IccDisposition::KeepSource
            } else {
                IccDisposition::Drop
            },
            range,
            rendering_intent: intent,
            hdr_static: HdrStaticDisposition::Keep,
            gain_map: GainMapDisposition::None,
            notes,
        };
    }

    let repr_cicp = representable_cicp(src);
    let cicp_represents = repr_cicp.is_some();
    let has_carrier = target.cicp();
    let is_authority = target.cicp_is_format_authority();
    let sole_safe = target.cicp_safe_sole_carrier();
    let icc_is_srgb = src.icc_profile.as_deref().is_some_and(icc::is_common_srgb);

    // Whether to emit CICP.
    let emit_cicp = match policy {
        ColorPolicy::Verbatim => has_carrier && src.cicp.is_some(),
        _ => match fields.cicp {
            CicpEmission::Never => false,
            CicpEmission::WhereFormatAuthority => has_carrier && is_authority && cicp_represents,
            CicpEmission::WhereverSupported => has_carrier && cicp_represents,
        },
    };
    let cicp_out = if emit_cicp {
        if policy == ColorPolicy::Verbatim {
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
        ColorPolicy::Balanced => drop_by_rule || (emit_cicp && icc_is_srgb),
        _ => drop_by_rule,
    };

    let mut notes = Vec::new();
    let icc = if src_has_icc {
        if drop_icc {
            if matches!(
                intent,
                Some(RenderingIntent::Saturation | RenderingIntent::AbsoluteColorimetric)
            ) {
                notes.push(ColorNote::RenderingIntentDropped);
            }
            IccDisposition::Drop
        } else {
            // Kept. If the target prefers CICP but we couldn't derive one, flag it.
            if !emit_cicp && has_carrier && is_authority && !cicp_represents {
                notes.push(ColorNote::IccKeptCicpUnrepresentable);
            }
            IccDisposition::KeepSource
        }
    } else if !emit_cicp && cicp_represents && policy != ColorPolicy::Verbatim {
        // No source ICC and CICP isn't carrying the color (target is ICC-only,
        // or policy is Compatibility): synthesize an ICC so color isn't lost.
        if matches!(range, Some(SignalRange::Narrow)) {
            notes.push(ColorNote::RangeDroppedOnSynthesis);
        }
        IccDisposition::SynthesizeFrom(repr_cicp.expect("cicp_represents"))
    } else if matches!(policy, ColorPolicy::Compatibility) && cicp_represents {
        // Compatibility always wants an ICC present.
        IccDisposition::SynthesizeFrom(repr_cicp.expect("cicp_represents"))
    } else {
        IccDisposition::Drop
    };

    ColorPlan {
        cicp: cicp_out,
        icc,
        range,
        rendering_intent: intent,
        hdr_static: HdrStaticDisposition::Keep,
        gain_map: GainMapDisposition::None,
        notes,
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
            .with_cicp_is_format_authority(true)
            .with_cicp_safe_sole_carrier(true)
    }
    fn caps_avif() -> EncodeCapabilities {
        EncodeCapabilities::new()
            .with_icc(true)
            .with_cicp(true)
            .with_cicp_is_format_authority(true)
            .with_cicp_safe_sole_carrier(false)
    }
    fn caps_jpeg() -> EncodeCapabilities {
        // No CICP carrier at all.
        EncodeCapabilities::new().with_icc(true)
    }

    fn src_cicp(c: Cicp) -> SourceColor {
        SourceColor::default()
            .with_cicp(c)
            .with_color_authority(ColorAuthority::Cicp)
            .with_channel_count(3)
    }

    #[test]
    fn jxl_balanced_strips_representable_icc() {
        // JXL (sole-safe): CICP present + an ICC whose color CICP represents →
        // emit CICP, drop the ICC (matches libjxl's want_icc=false default).
        let src = SourceColor::default()
            .with_cicp(Cicp::SRGB)
            .with_icc_profile(alloc::vec![0u8; 132])
            .with_channel_count(3);
        let plan = resolve_color_emit(&src, &caps_jxl(), ColorPolicy::Balanced);
        assert_eq!(plan.cicp, Some(Cicp::SRGB));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn avif_balanced_keeps_nonsrgb_icc_alongside_cicp() {
        // AVIF (not sole-safe): a non-sRGB ICC is kept alongside CICP. (The
        // redundant-sRGB drop needs a corpus-recognized profile and is covered
        // by the conformance suite, which has a real sRGB profile via `cms`.)
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(alloc::vec![0u8; 132]);
        let plan = resolve_color_emit(&p3, &caps_avif(), ColorPolicy::Balanced);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::KeepSource);
    }

    #[test]
    fn jpeg_synthesizes_icc_from_cicp() {
        // CICP-only source → JPEG (no CICP carrier): synthesize an ICC.
        let src = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(&src, &caps_jpeg(), ColorPolicy::Balanced);
        assert_eq!(plan.cicp, None);
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn compact_strips_icc_on_avif() {
        // Compact drops the ICC wherever CICP represents the color, even on AVIF.
        let p3 = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(alloc::vec![0u8; 132]);
        let plan = resolve_color_emit(&p3, &caps_avif(), ColorPolicy::Compact);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::Drop);
    }

    #[test]
    fn compatibility_always_keeps_or_synthesizes_icc() {
        // CICP-only source, AVIF, Compatibility → CICP emitted AND an ICC synthesized.
        let src = src_cicp(Cicp::DISPLAY_P3);
        let plan = resolve_color_emit(&src, &caps_avif(), ColorPolicy::Compatibility);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::SynthesizeFrom(Cicp::DISPLAY_P3));
    }

    #[test]
    fn grayscale_keeps_icc_suppresses_cicp() {
        // A 1-channel source: CICP is inapplicable; keep ICC, note it.
        let src = SourceColor::default()
            .with_icc_profile(alloc::vec![0u8; 132])
            .with_channel_count(1);
        let plan = resolve_color_emit(&src, &caps_avif(), ColorPolicy::Balanced);
        assert_eq!(plan.cicp, None);
        assert_eq!(plan.icc, IccDisposition::KeepSource);
        assert!(plan.notes.contains(&ColorNote::GrayNotCicpRepresentable));
    }

    #[test]
    fn verbatim_passes_source_through() {
        // Verbatim keeps both, derives nothing.
        let src = src_cicp(Cicp::DISPLAY_P3).with_icc_profile(alloc::vec![0u8; 132]);
        let plan = resolve_color_emit(&src, &caps_avif(), ColorPolicy::Verbatim);
        assert_eq!(plan.cicp, Some(Cicp::DISPLAY_P3));
        assert_eq!(plan.icc, IccDisposition::KeepSource);
    }

    #[test]
    fn default_policy_is_balanced() {
        assert_eq!(ColorPolicy::default(), ColorPolicy::Balanced);
    }
}
