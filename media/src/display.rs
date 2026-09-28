//! Explicit display interpretation of source-encoded RGB.
//!
//! All linear components here are absolute display cd/m². HLG scene light
//! never enters a PQ or primary conversion without its luminance-coupled OOTF.
//! This module does not infer display white, black, gamma, or tone mapping.

use std::fmt;
use zenpixels::{Cicp, ColorPrimaries};

/// Policy for finite values outside the selected display's encoding domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutOfRange {
    Reject,
    /// Clip individual components. This is not tone mapping or gamut mapping.
    Clamp,
}

#[derive(Clone, Copy, Debug)]
enum Curve {
    Srgb {
        white: f32,
    },
    Pq,
    Hlg {
        peak: f32,
        gamma: f32,
        beta: f32,
    },
    Bt1886 {
        black: f32,
        white: f32,
        black_root: f32,
        span_root: f32,
    },
    Linear {
        scale: f32,
    },
}

/// A validated display interpretation, separate from raw CICP signaling.
///
/// `decode` returns absolute display-linear RGB; `encode` takes the same domain.
/// Alpha must bypass both operations. HLG is specifically BT.2020 RGB.
#[derive(Clone, Copy, Debug)]
pub struct DisplayTransfer(Curve);

impl DisplayTransfer {
    /// IEC sRGB with an explicit luminance for encoded white (not inferred
    /// from CICP or mastering-display metadata).
    pub fn srgb(white_nits: f32) -> Result<Self, DisplayError> {
        positive(white_nits)?;
        Ok(Self(Curve::Srgb { white: white_nits }))
    }

    /// ST 2084: encoded one means 10,000 cd/m² independently of display peak.
    pub const fn pq() -> Self {
        Self(Curve::Pq)
    }

    /// BT.2100 HLG display EOTF, including black lift and system gamma.
    /// Gamma is explicit: viewing environment is a caller policy. The usual
    /// 1,000 cd/m² reference uses 1.2. Requires `0 <= black < peak` and a
    /// black lift below one. A neutral zero signal decodes to `black_nits`.
    pub fn hlg(peak_nits: f32, black_nits: f32, system_gamma: f32) -> Result<Self, DisplayError> {
        positive(peak_nits)?;
        positive(system_gamma)?;
        if !black_nits.is_finite() || black_nits < 0.0 || black_nits >= peak_nits {
            return Err(DisplayError::InvalidDisplay);
        }
        let beta = (3.0 * (black_nits / peak_nits).powf(1.0 / system_gamma)).sqrt();
        if !beta.is_finite() || beta >= 1.0 {
            return Err(DisplayError::InvalidDisplay);
        }
        Ok(Self(Curve::Hlg {
            peak: peak_nits,
            gamma: system_gamma,
            beta,
        }))
    }

    /// BT.1886 display EOTF for BT.709 video, exponent 2.4 and explicit display
    /// white/black. This is deliberately not the inverse BT.709 camera OETF.
    pub fn bt1886(white_nits: f32, black_nits: f32) -> Result<Self, DisplayError> {
        positive(white_nits)?;
        if !black_nits.is_finite() || black_nits < 0.0 || black_nits >= white_nits {
            return Err(DisplayError::InvalidDisplay);
        }
        let black_root = black_nits.powf(1.0 / 2.4);
        let span_root = white_nits.powf(1.0 / 2.4) - black_root;
        if span_root <= 0.0 {
            return Err(DisplayError::InvalidDisplay);
        }
        Ok(Self(Curve::Bt1886 {
            black: black_nits,
            white: white_nits,
            black_root,
            span_root,
        }))
    }

    /// Bounded display-linear encoding: one means `scale_nits`. This does not
    /// turn unspecified scene-linear data into a display interpretation.
    pub fn linear(scale_nits: f32) -> Result<Self, DisplayError> {
        positive(scale_nits)?;
        Ok(Self(Curve::Linear { scale: scale_nits }))
    }

    fn transfer_code(self) -> u8 {
        match self.0 {
            Curve::Srgb { .. } => 13,
            Curve::Pq => 16,
            Curve::Hlg { .. } => 18,
            Curve::Bt1886 { .. } => 1,
            Curve::Linear { .. } => 8,
        }
    }

    pub fn decode(self, rgb: [f32; 3], policy: OutOfRange) -> Result<[f32; 3], DisplayError> {
        let rgb = bounded(rgb, policy)?;
        let output = match self.0 {
            Curve::Srgb { white } => rgb.map(|x| linear_srgb::iec::srgb_to_linear(x) * white),
            Curve::Pq => rgb.map(|x| {
                // The polynomial is an approximation; pin the normative ends.
                if x == 1.0 {
                    10000.0
                } else {
                    linear_srgb::tf::pq_to_linear(x) * 10000.0
                }
            }),
            Curve::Hlg { peak, gamma, beta } => {
                let scene = rgb.map(|x| linear_srgb::tf::hlg_to_linear((1.0 - beta) * x + beta));
                zentone::hlg::hlg_ootf(scene, gamma).map(|x| x * peak)
            }
            Curve::Bt1886 {
                black,
                white,
                black_root,
                span_root,
            } => rgb.map(|x| {
                // Preserve physical black/white exactly; powf at an endpoint
                // can drift outside its own declared display domain.
                if x == 0.0 {
                    black
                } else if x == 1.0 {
                    white
                } else {
                    (x * span_root + black_root).powf(2.4).clamp(black, white)
                }
            }),
            Curve::Linear { scale } => rgb.map(|x| x * scale),
        };
        finite(output)
    }

    pub fn encode(self, nits: [f32; 3], policy: OutOfRange) -> Result<[f32; 3], DisplayError> {
        let mut nits = finite(nits)?;
        if nits.iter().any(|&x| x < 0.0) {
            if policy == OutOfRange::Reject {
                return Err(DisplayError::OutOfRange);
            }
            nits = nits.map(|x| x.max(0.0));
        }
        let signal = match self.0 {
            Curve::Srgb { white } => {
                // The underlying IEC function clamps, so validate first.
                bounded(nits.map(|x| x / white), policy)?.map(linear_srgb::iec::linear_to_srgb)
            }
            Curve::Pq => bounded(nits.map(|x| x / 10000.0), policy)?.map(|x| {
                if x == 1.0 {
                    1.0
                } else {
                    linear_srgb::tf::linear_to_pq(x)
                }
            }),
            Curve::Hlg { peak, gamma, beta } => {
                let scene = zentone::hlg::hlg_inverse_ootf(nits.map(|x| x / peak), gamma);
                scene.map(|x| (linear_srgb::tf::linear_to_hlg(x) - beta) / (1.0 - beta))
            }
            Curve::Bt1886 {
                black,
                white,
                black_root,
                span_root,
            } => {
                if policy == OutOfRange::Reject && nits.iter().any(|&x| x < black || x > white) {
                    return Err(DisplayError::OutOfRange);
                }
                nits.map(|x| {
                    let x = x.clamp(black, white);
                    if x == black {
                        0.0
                    } else if x == white {
                        1.0
                    } else {
                        ((x.powf(1.0 / 2.4) - black_root) / span_root).clamp(0.0, 1.0)
                    }
                })
            }
            Curve::Linear { scale } => nits.map(|x| x / scale),
        };
        bounded(signal, policy)
    }
}

/// Prepared transfer → primary matrix → inverse-transfer conversion. Both
/// CICP descriptions must describe full-range RGB; use `YuvToRgb` first for
/// native components. No ICC interpretation, rendering intent, or tone map is
/// silently selected. Unsupported/unspecified color fails construction.
#[derive(Clone, Debug)]
pub struct DisplayConversion {
    source: DisplayTransfer,
    target: DisplayTransfer,
    matrix: [[f32; 3]; 3],
    policy: OutOfRange,
    input: Cicp,
    output: Cicp,
}

impl DisplayConversion {
    pub fn new(
        source: Cicp,
        source_display: DisplayTransfer,
        target: Cicp,
        target_display: DisplayTransfer,
        policy: OutOfRange,
    ) -> Result<Self, DisplayError> {
        for (color, display) in [(source, source_display), (target, target_display)] {
            if color.matrix_coefficients != 0
                || !color.full_range
                || color.transfer_characteristics != display.transfer_code()
            {
                return Err(DisplayError::ColorMismatch);
            }
            if matches!(display.0, Curve::Hlg { .. }) && color.color_primaries != 9 {
                return Err(DisplayError::HlgPrimaries);
            }
        }
        let from = ColorPrimaries::from_cicp(source.color_primaries)
            .ok_or(DisplayError::UnknownPrimaries)?;
        let to = ColorPrimaries::from_cicp(target.color_primaries)
            .ok_or(DisplayError::UnknownPrimaries)?;
        let matrix = from
            .gamut_matrix_to(to)
            .ok_or(DisplayError::UnknownPrimaries)?;
        Ok(Self {
            source: source_display,
            target: target_display,
            matrix,
            policy,
            input: source,
            output: target,
        })
    }

    pub fn input_color(&self) -> Cicp {
        self.input
    }

    pub fn output_color(&self) -> Cicp {
        self.output
    }

    pub fn convert(&self, rgb: [f32; 3]) -> Result<[f32; 3], DisplayError> {
        let light = self.source.decode(rgb, self.policy)?;
        let target = self
            .matrix
            .map(|row| row[0] * light[0] + row[1] * light[1] + row[2] * light[2]);
        self.target.encode(target, self.policy)
    }

    /// Convert a tightly packed RGB row. Validation and a dry run precede
    /// writes, so a rejected pixel leaves the entire destination unchanged.
    /// This scalar path performs no allocations; source/destination widths
    /// must match. Alpha is carried separately by the enclosing image.
    pub fn write_row(
        &self,
        source: &[[f32; 3]],
        destination: &mut [[f32; 3]],
    ) -> Result<(), DisplayError> {
        if source.len() != destination.len() {
            return Err(DisplayError::OutputWidth);
        }
        for &rgb in source {
            self.convert(rgb)?;
        }
        for (&rgb, out) in source.iter().zip(destination) {
            *out = self.convert(rgb)?;
        }
        Ok(())
    }
}

fn positive(x: f32) -> Result<(), DisplayError> {
    if x.is_finite() && x > 0.0 {
        Ok(())
    } else {
        Err(DisplayError::InvalidDisplay)
    }
}

fn finite(rgb: [f32; 3]) -> Result<[f32; 3], DisplayError> {
    if rgb.iter().all(|x| x.is_finite()) {
        Ok(rgb)
    } else {
        Err(DisplayError::NonFinite)
    }
}

fn bounded(rgb: [f32; 3], policy: OutOfRange) -> Result<[f32; 3], DisplayError> {
    let rgb = finite(rgb)?;
    if policy == OutOfRange::Reject && rgb.iter().any(|x| !(0.0..=1.0).contains(x)) {
        return Err(DisplayError::OutOfRange);
    }
    Ok(rgb.map(|x| x.clamp(0.0, 1.0)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DisplayError {
    InvalidDisplay,
    NonFinite,
    OutOfRange,
    ColorMismatch,
    HlgPrimaries,
    UnknownPrimaries,
    OutputWidth,
}

impl fmt::Display for DisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidDisplay => "display white, black or gamma is invalid",
            Self::NonFinite => "color conversion produced or received a nonfinite component",
            Self::OutOfRange => "component lies outside the selected display encoding",
            Self::ColorMismatch => "display interpretation disagrees with full-range RGB CICP",
            Self::HlgPrimaries => "HLG display interpretation requires BT.2020 primaries",
            Self::UnknownPrimaries => "color primaries have no supported conversion matrix",
            Self::OutputWidth => "source and destination RGB row widths differ",
        })
    }
}
impl std::error::Error for DisplayError {}
