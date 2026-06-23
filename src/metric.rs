//! Approximate conversion between perceptual quality metrics.
//!
//! A codec or pipeline often has a target expressed in one metric — say a
//! SSIMULACRA2 score — but a knob calibrated in another, such as a butteraugli
//! distance (the unit [`LossyTarget::ApproxButteraugli`] takes). This module
//! provides:
//!
//! - [`interpolate`] — a generic piecewise-linear table lookup (clamping, no
//!   extrapolation). Use it directly with any `&[(key, value)]` table, including
//!   the published [`SSIM2_TO_BUTTERAUGLI_MAX`] / [`SSIM2_TO_BUTTERAUGLI_PNORM3`]
//!   calibration tables.
//! - [`Metric`] + [`convert`] — an enum-driven front end that converts a value
//!   between any two metrics, routing through SSIMULACRA2 as the pivot.
//!
//! The calibration tables are codec-agnostic central estimates fit from the
//! zenmetrics omni fleet sweep (779k cells across zenjpeg / zenwebp / zenavif,
//! 2026-06-23; provenance: `zenwebp/benchmarks/codec_metric_to_q_2026-06-23.md`),
//! taking the per-codec median at each ssim2 level and enforcing monotonicity.
//!
//! # Accuracy & caveats
//!
//! These are **approximate** cross-metric conversions, not exact functions — two
//! metrics measure different things and the mapping has real spread by content
//! and codec. Treat a converted value as a calibrated estimate.
//!
//! - **3-norm butteraugli ([`Metric::ButteraugliPnorm3`]) is the most
//!   codec-agnostic** — per-codec medians agree to within ~0.2 over the
//!   ssim2 65–90 band. Prefer it when you need a stable cross-codec relationship.
//! - **Max-norm butteraugli ([`Metric::ButteraugliMax`]) is codec-dependent at
//!   low quality** — it tracks the single worst block, which differs 2–4× by
//!   codec below ssim2 ~55, so the table is only a central estimate there (the
//!   band is wide). It is the norm [`LossyTarget::ApproxButteraugli`] targets.
//! - SSIMULACRA2 **saturates near 100**, so conversions above ssim2 ~95 are
//!   ill-defined; the tables anchor ssim2 100 → butteraugli 0.
//!
//! ```
//! use zencodec::metric::{convert, interpolate, Metric, SSIM2_TO_BUTTERAUGLI_MAX};
//!
//! // Generic conversion between two metrics.
//! let d = convert(85.0, Metric::Ssim2, Metric::ButteraugliMax).unwrap();
//! assert!((d - 2.3).abs() < 0.2);
//!
//! // Or interpolate a table directly.
//! let d2 = interpolate(SSIM2_TO_BUTTERAUGLI_MAX, 85.0);
//! assert!((d - d2).abs() < f32::EPSILON);
//! ```
//!
//! [`LossyTarget::ApproxButteraugli`]: crate::encode::LossyTarget::ApproxButteraugli

/// A perceptual quality metric with calibrated cross-conversion tables.
///
/// `#[non_exhaustive]` — more metrics may be added; match with a `_` arm or use
/// [`convert`], which returns `None` for any pair without a calibration table.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Metric {
    /// SSIMULACRA2 — roughly 0..=100, **higher is better**. The pivot metric
    /// all conversions route through.
    Ssim2,
    /// Butteraugli **max-norm** (p→∞, worst region) — `>= 0`, **lower is
    /// better**. The norm [`LossyTarget::ApproxButteraugli`] targets;
    /// codec-dependent at low quality.
    ///
    /// [`LossyTarget::ApproxButteraugli`]: crate::encode::LossyTarget::ApproxButteraugli
    ButteraugliMax,
    /// Butteraugli **3-norm** (aggregate) — `>= 0`, **lower is better**. The
    /// most codec-agnostic of the three.
    ButteraugliPnorm3,
}

/// Piecewise-linear interpolation of `table` at `x`, clamping to the endpoints.
///
/// `table` is a set of `(key, value)` points **sorted ascending by key**. For
/// `x` inside the key range the value is linearly interpolated between the two
/// bracketing points; outside it, the nearest endpoint's value is returned (no
/// extrapolation). An empty `table` returns `x` unchanged.
///
/// This is the primitive behind [`convert`]; it works with any monotone or
/// non-monotone table, including the published calibration tables in this module.
#[must_use]
pub fn interpolate(table: &[(f32, f32)], x: f32) -> f32 {
    let Some(&(first_x, first_y)) = table.first() else {
        return x;
    };
    if x <= first_x {
        return first_y;
    }
    let &(last_x, last_y) = &table[table.len() - 1];
    if x >= last_x {
        return last_y;
    }
    for w in table.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if x <= x1 {
            // x0 < x <= x1; keys are ascending so x1 > x0 (no divide-by-zero on
            // the calibration tables, which are strictly increasing in key).
            let span = x1 - x0;
            if span <= 0.0 {
                return y1;
            }
            return y0 + (x - x0) / span * (y1 - y0);
        }
    }
    last_y
}

/// Approximately convert `value` from metric `from` into metric `to`.
///
/// Conversions route through SSIMULACRA2 as the pivot. Returns the input
/// unchanged when `from == to`, and `None` only for a `(from, to)` pair with no
/// calibration table (all currently-defined [`Metric`] pairs are supported).
///
/// The result is a calibrated estimate — see the [module docs](self) for the
/// per-metric accuracy caveats (max-norm is codec-dependent at low quality;
/// ssim2 saturates near 100).
#[must_use]
pub fn convert(value: f32, from: Metric, to: Metric) -> Option<f32> {
    if from == to {
        return Some(value);
    }
    let ssim2 = to_ssim2(value, from)?;
    from_ssim2(ssim2, to)
}

/// Map `value` (in metric `from`) onto the SSIMULACRA2 pivot scale.
fn to_ssim2(value: f32, from: Metric) -> Option<f32> {
    Some(match from {
        Metric::Ssim2 => value,
        Metric::ButteraugliMax => interpolate(BUTTERAUGLI_MAX_TO_SSIM2, value),
        Metric::ButteraugliPnorm3 => interpolate(BUTTERAUGLI_PNORM3_TO_SSIM2, value),
    })
}

/// Map a SSIMULACRA2 pivot value onto metric `to`.
fn from_ssim2(ssim2: f32, to: Metric) -> Option<f32> {
    Some(match to {
        Metric::Ssim2 => ssim2,
        Metric::ButteraugliMax => interpolate(SSIM2_TO_BUTTERAUGLI_MAX, ssim2),
        Metric::ButteraugliPnorm3 => interpolate(SSIM2_TO_BUTTERAUGLI_PNORM3, ssim2),
    })
}

// ===========================================================================
// Calibration tables — codec-agnostic central estimates, ssim2 key 0..=100.
// Fit from the zenmetrics omni fleet sweep (zenjpeg/zenwebp/zenavif, 779k cells,
// 2026-06-23), per-codec median per ssim2 level, monotonized. See module docs +
// zenwebp/benchmarks/codec_metric_to_q_2026-06-23.md.
// ===========================================================================

/// SSIMULACRA2 → butteraugli **max-norm** distance (ssim2 ascending; the value
/// descends, since higher ssim2 = lower distance). Codec-dependent below
/// ssim2 ~55 (see module docs).
pub const SSIM2_TO_BUTTERAUGLI_MAX: &[(f32, f32)] = &[
    (0.0, 25.733),
    (5.0, 18.423),
    (10.0, 15.419),
    (15.0, 15.068),
    (20.0, 14.410),
    (25.0, 10.968),
    (30.0, 9.186),
    (35.0, 9.185),
    (40.0, 9.184),
    (45.0, 8.712),
    (50.0, 8.711),
    (55.0, 8.497),
    (60.0, 7.707),
    (65.0, 6.669),
    (70.0, 5.479),
    (75.0, 4.240),
    (80.0, 3.427),
    (85.0, 2.324),
    (90.0, 1.336),
    (92.0, 0.867),
    (94.0, 0.692),
    (96.0, 0.438),
    (98.0, 0.194),
    (100.0, 0.000),
];

/// SSIMULACRA2 → butteraugli **3-norm** distance (ssim2 ascending). The most
/// codec-agnostic of the tables.
pub const SSIM2_TO_BUTTERAUGLI_PNORM3: &[(f32, f32)] = &[
    (0.0, 8.261),
    (5.0, 6.145),
    (10.0, 5.468),
    (15.0, 5.382),
    (20.0, 5.096),
    (25.0, 3.807),
    (30.0, 3.788),
    (35.0, 3.752),
    (40.0, 3.493),
    (45.0, 3.271),
    (50.0, 2.948),
    (55.0, 2.755),
    (60.0, 2.487),
    (65.0, 2.115),
    (70.0, 1.744),
    (75.0, 1.417),
    (80.0, 1.154),
    (85.0, 0.840),
    (90.0, 0.513),
    (92.0, 0.311),
    (94.0, 0.291),
    (96.0, 0.136),
    (98.0, 0.047),
    (100.0, 0.000),
];

/// Inverse of [`SSIM2_TO_BUTTERAUGLI_MAX`] (butteraugli max-norm key ascending).
const BUTTERAUGLI_MAX_TO_SSIM2: &[(f32, f32)] = &[
    (0.000, 100.0),
    (0.194, 98.0),
    (0.438, 96.0),
    (0.692, 94.0),
    (0.867, 92.0),
    (1.336, 90.0),
    (2.324, 85.0),
    (3.427, 80.0),
    (4.240, 75.0),
    (5.479, 70.0),
    (6.669, 65.0),
    (7.707, 60.0),
    (8.497, 55.0),
    (8.711, 50.0),
    (8.712, 45.0),
    (9.184, 40.0),
    (9.185, 35.0),
    (9.186, 30.0),
    (10.968, 25.0),
    (14.410, 20.0),
    (15.068, 15.0),
    (15.419, 10.0),
    (18.423, 5.0),
    (25.733, 0.0),
];

/// Inverse of [`SSIM2_TO_BUTTERAUGLI_PNORM3`] (butteraugli 3-norm key ascending).
const BUTTERAUGLI_PNORM3_TO_SSIM2: &[(f32, f32)] = &[
    (0.000, 100.0),
    (0.047, 98.0),
    (0.136, 96.0),
    (0.291, 94.0),
    (0.311, 92.0),
    (0.513, 90.0),
    (0.840, 85.0),
    (1.154, 80.0),
    (1.417, 75.0),
    (1.744, 70.0),
    (2.115, 65.0),
    (2.487, 60.0),
    (2.755, 55.0),
    (2.948, 50.0),
    (3.271, 45.0),
    (3.493, 40.0),
    (3.752, 35.0),
    (3.788, 30.0),
    (3.807, 25.0),
    (5.096, 20.0),
    (5.382, 15.0),
    (5.468, 10.0),
    (6.145, 5.0),
    (8.261, 0.0),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_clamps_and_lerps() {
        let t = &[(0.0_f32, 10.0_f32), (10.0, 20.0), (20.0, 0.0)];
        // Clamp below / above.
        assert_eq!(interpolate(t, -5.0), 10.0);
        assert_eq!(interpolate(t, 99.0), 0.0);
        // Exact knots.
        assert_eq!(interpolate(t, 0.0), 10.0);
        assert_eq!(interpolate(t, 10.0), 20.0);
        assert_eq!(interpolate(t, 20.0), 0.0);
        // Midpoints (linear).
        assert!((interpolate(t, 5.0) - 15.0).abs() < 1e-5);
        assert!((interpolate(t, 15.0) - 10.0).abs() < 1e-5);
        // Empty table → identity.
        assert_eq!(interpolate(&[], 7.0), 7.0);
    }

    #[test]
    fn convert_identity_and_known_points() {
        assert_eq!(convert(73.0, Metric::Ssim2, Metric::Ssim2), Some(73.0));
        // ssim2 85 ≈ butteraugli max-norm 2.32, 3-norm 0.84.
        let bmax = convert(85.0, Metric::Ssim2, Metric::ButteraugliMax).unwrap();
        assert!((bmax - 2.324).abs() < 1e-3, "{bmax}");
        let b3 = convert(85.0, Metric::Ssim2, Metric::ButteraugliPnorm3).unwrap();
        assert!((b3 - 0.840).abs() < 1e-3, "{b3}");
    }

    #[test]
    fn convert_roundtrips_through_pivot() {
        // ssim2 → bmax → ssim2 recovers the input on a strictly-monotone knot.
        for &s in &[20.0_f32, 50.0, 70.0, 85.0, 90.0] {
            let d = convert(s, Metric::Ssim2, Metric::ButteraugliMax).unwrap();
            let back = convert(d, Metric::ButteraugliMax, Metric::Ssim2).unwrap();
            assert!((back - s).abs() < 0.5, "ssim2 {s} → {d} → {back}");
        }
    }

    #[test]
    fn convert_between_butteraugli_norms_via_pivot() {
        // max-norm 2.324 ≈ ssim2 85 ≈ 3-norm 0.84.
        let b3 = convert(2.324, Metric::ButteraugliMax, Metric::ButteraugliPnorm3).unwrap();
        assert!((b3 - 0.840).abs() < 0.1, "{b3}");
    }

    #[test]
    fn tables_are_strictly_monotone_in_key() {
        for t in [
            SSIM2_TO_BUTTERAUGLI_MAX,
            SSIM2_TO_BUTTERAUGLI_PNORM3,
            BUTTERAUGLI_MAX_TO_SSIM2,
            BUTTERAUGLI_PNORM3_TO_SSIM2,
        ] {
            for w in t.windows(2) {
                assert!(w[1].0 > w[0].0, "key not strictly ascending: {w:?}");
            }
        }
    }

    #[test]
    fn lower_quality_means_higher_distance() {
        // butteraugli is "lower = better", so distance must fall as ssim2 rises.
        let lo = convert(60.0, Metric::Ssim2, Metric::ButteraugliMax).unwrap();
        let hi = convert(90.0, Metric::Ssim2, Metric::ButteraugliMax).unwrap();
        assert!(lo > hi, "ssim2 60 dist {lo} should exceed ssim2 90 dist {hi}");
    }
}
