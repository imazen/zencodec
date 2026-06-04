//! Encode fidelity: how faithfully an encoder reproduces its input.
//!
//! [`Fidelity`] is the complete fidelity request — *exactly one of*:
//! - **lossy**, aiming at a [`LossyTarget`] (a quality dial, a perceptual
//!   distance, a metric score, or a size/bitrate budget),
//! - **near-lossless**, within a per-channel [`NearLosslessBudget`], or
//! - **mathematically lossless**.
//!
//! It is a sum type so each regime carries the parameter its own metric needs,
//! illegal states (lossy ∧ lossless) are unrepresentable, and lossless is
//! explicit rather than "quality == 100". See `docs/near-lossless-design.md`
//! for the full rationale and per-codec mapping.

/// The complete fidelity request for an encode — exactly one of three things.
///
/// Set with [`EncoderConfig::with_fidelity`](crate::encode::EncoderConfig::with_fidelity);
/// read what the codec resolved with
/// [`resolved_target_fidelity`](crate::encode::EncoderConfig::resolved_target_fidelity).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Fidelity {
    /// Lossy codestream. *What* it aims at is a [`LossyTarget`].
    Lossy(LossyTarget),
    /// Lossless codestream of pixels pre-quantized within a per-channel L∞
    /// budget. [`NearLosslessBudget::EXACT`] is mathematically lossless.
    NearLossless(NearLosslessBudget),
    /// Mathematically exact — decode reproduces the input sample-for-sample.
    Lossless,
}

impl Fidelity {
    /// Convenience constructor for lossy encoding at a 0–100 quality.
    #[must_use]
    pub const fn quality(q: f32) -> Self {
        Self::Lossy(LossyTarget::Quality(q))
    }

    /// Convenience constructor for near-lossless within `budget`.
    #[must_use]
    pub const fn near_lossless(budget: NearLosslessBudget) -> Self {
        Self::NearLossless(budget)
    }

    /// Whether this request is mathematically lossless (exact `Lossless`, or a
    /// near-lossless budget of [`NearLosslessBudget::EXACT`]).
    #[must_use]
    pub const fn is_lossless(self) -> bool {
        match self {
            Self::Lossless => true,
            Self::NearLossless(b) => b.is_exact(),
            Self::Lossy(_) => false,
        }
    }
}

/// What a lossy encode aims at.
///
/// **Non-exhaustive — the arms differ in cost and support.** `Quality` maps to a
/// native dial in a single pass on every codec; `Metric`, `TargetBytes`, and
/// `Bitrate` require *iterative* re-encoding (binary search over the quantizer)
/// that only some codecs implement; `Distance` is single-pass on JXL only. Query
/// [`EncodeCapabilities`](crate::encode::EncodeCapabilities) before requesting a
/// non-trivial target, and check
/// [`try_target_fidelity`](crate::encode::EncoderConfig::try_target_fidelity)
/// for how (or whether) it was honored.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LossyTarget {
    /// Calibrated 0–100 quality dial (the same scale as
    /// [`with_generic_quality`](crate::encode::EncoderConfig::with_generic_quality)).
    /// Single-pass on every codec. The safe default.
    Quality(f32),
    /// Butteraugli distance (JXL-native; lower is better, ~0.5–1.0 is visually
    /// lossless). Single-pass on JXL; iterative elsewhere.
    Distance(f32),
    /// Hit a quality-metric score — the codec binary-searches the quantizer.
    /// Iterative; only codecs that implement convergence honor it.
    Metric {
        /// Which metric to target.
        metric: QualityMetric,
        /// Target score on that metric's own scale.
        target: f32,
    },
    /// Hit a target encoded size in bytes (iterative).
    TargetBytes(u64),
    /// Hit a target bitrate in bits per pixel (iterative).
    Bitrate(f32),
}

/// A quality / perceptual metric a lossy encode can target.
///
/// Non-exhaustive: metrics are added as convergence support lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum QualityMetric {
    /// SSIMULACRA2 (0–100, higher is better).
    Ssimulacra2,
    /// Butteraugli (lower is better).
    Butteraugli,
    /// DSSIM (lower is better).
    Dssim,
    /// PSNR in dB (higher is better).
    Psnr,
}

/// The maximum a near-lossless encode may change **any single channel of any
/// single pixel** — the L∞-per-channel ceiling — as a fraction of that
/// channel's full range.
///
/// **Codec-agnostic and total: every value is valid for every lossless codec.**
/// A codec resolves it to the largest native setting whose *guaranteed* error
/// does not exceed the budget at its own bit depth (rounding **down**, never
/// up), and reports what it honored.
///
/// Stored as parts-per-65535 of full scale — a *fraction*, not "16-bit LSBs".
/// `255 × 257 = 65535` makes both 8-bit and 16-bit resolve exactly with integer
/// math (no float-floor trap): `from_8bit_steps(2)` is `±2` at 8-bit and `±514`
/// at 16-bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NearLosslessBudget(u16);

impl NearLosslessBudget {
    /// Exact — identical to [`Fidelity::Lossless`].
    pub const EXACT: Self = Self(0);
    /// The whole channel range (loosest possible budget).
    pub const MAX: Self = Self(u16::MAX);
    /// A sensible default (±2/255): visually transparent on photographic
    /// content, meaningfully smaller files. Use when you want "near-lossless"
    /// without choosing a number.
    pub const DEFAULT: Self = Self::from_8bit_steps(2);

    /// From the familiar 0–255 scale. `from_8bit_steps(2)` ⇒ `±2` on an 8-bit
    /// channel, and the same *fraction* (`±514`) on a 16-bit channel.
    #[must_use]
    pub const fn from_8bit_steps(n: u8) -> Self {
        // n ≤ 255 ⇒ n*257 ≤ 65535, exact in u16.
        Self(((n as u32) * 257) as u16)
    }

    /// From the 0–65535 scale, for deep content.
    #[must_use]
    pub const fn from_16bit_steps(n: u16) -> Self {
        Self(n)
    }

    /// From a fraction of full range (depth-independent). Clamped to `[0, 1]`.
    #[must_use]
    pub fn from_fraction(f: f32) -> Self {
        let v = (f.clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
        Self(if v > 65535 { 65535 } else { v as u16 })
    }

    /// Whether this is the exact (zero-error) budget.
    #[must_use]
    pub const fn is_exact(self) -> bool {
        self.0 == 0
    }

    /// The budget as a fraction of full scale (`0.0..=1.0`).
    #[must_use]
    pub fn as_fraction(self) -> f32 {
        f32::from(self.0) / 65535.0
    }

    /// The integer L∞ ceiling (in LSBs) a `depth`-bit codec may not exceed.
    /// Exact integer math; the floor *is* the "round the guarantee down" rule.
    ///
    /// `from_8bit_steps(2).max_error_at_depth(8) == 2` and
    /// `from_8bit_steps(2).max_error_at_depth(16) == 514`.
    #[must_use]
    pub const fn max_error_at_depth(self, depth: u32) -> u32 {
        let full = (1u32 << depth) - 1;
        ((self.0 as u32) * full) / 65535
    }
}

/// How a codec resolved a requested [`Fidelity`], returned by
/// [`try_target_fidelity`](crate::encode::EncoderConfig::try_target_fidelity).
///
/// A codec may quietly give you *better* fidelity than you asked
/// ([`TargetRaised`](Self::TargetRaised), [`Lossless`](Self::Lossless)) but a
/// move to *lower* fidelity than requested is always observable here (and a
/// downgrade across the lossy/lossless fence is [`Unsupported`](Self::Unsupported),
/// not a silent substitution).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum FidelityMatch {
    /// Honored exactly as requested.
    Supported,
    /// A metric / distance target was translated to the codec's native scale.
    /// The applied fidelity is in the payload.
    MetricTranslated(Fidelity),
    /// Rounded up to a higher-quality / tighter supported setting than
    /// requested (fidelity ≥ request — the contract still holds).
    TargetRaised(Fidelity),
    /// Rounded down to a lower-quality / looser supported setting than
    /// requested (still within the requested regime).
    TargetLowered(Fidelity),
    /// Resolved to exact lossless — e.g. a near-lossless budget on a codec with
    /// no ε mechanism, or an [`NearLosslessBudget::EXACT`] budget.
    Lossless,
    /// Not honorable even approximately (e.g. `Lossless` on a codec with no
    /// lossless path, or a metric target it cannot converge to).
    Unsupported,
}

impl FidelityMatch {
    /// Whether the codec will produce output for this request (anything other
    /// than [`Unsupported`](Self::Unsupported)).
    #[must_use]
    pub const fn is_honored(self) -> bool {
        !matches!(self, Self::Unsupported)
    }

    /// The applied fidelity carried by this match, when it differs from the
    /// request. `Supported` and `Unsupported` carry none.
    #[must_use]
    pub const fn resolved(self) -> Option<Fidelity> {
        match self {
            Self::MetricTranslated(f) | Self::TargetRaised(f) | Self::TargetLowered(f) => Some(f),
            Self::Lossless => Some(Fidelity::Lossless),
            Self::Supported | Self::Unsupported => None,
        }
    }
}

/// Classify how a `resolved` fidelity relates to the `requested` one.
///
/// Used by the default [`try_target_fidelity`](crate::encode::EncoderConfig::try_target_fidelity)
/// implementation. The generic default can classify the common cases (exact,
/// quality raised/lowered, metric-translated-to-quality, promoted-to-lossless,
/// unsupported); codecs override `try_target_fidelity` for fully precise
/// reporting that knows their native quantization.
pub(crate) fn classify_fidelity_match(
    requested: Fidelity,
    resolved: Option<Fidelity>,
) -> FidelityMatch {
    let Some(resolved) = resolved else {
        return FidelityMatch::Unsupported;
    };
    if resolved == requested {
        return FidelityMatch::Supported;
    }
    match (requested, resolved) {
        (_, Fidelity::Lossless) => FidelityMatch::Lossless,
        (Fidelity::Lossy(req), Fidelity::Lossy(LossyTarget::Quality(rq))) => match req {
            LossyTarget::Quality(reqq) if rq > reqq => FidelityMatch::TargetRaised(resolved),
            LossyTarget::Quality(_) => FidelityMatch::TargetLowered(resolved),
            // requested a non-Quality lossy target, got a plain quality back
            _ => FidelityMatch::MetricTranslated(resolved),
        },
        // anything else changed but the direction isn't generically knowable;
        // report the conservative "not better than asked" so callers inspect it.
        _ => FidelityMatch::TargetLowered(resolved),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_exact_round_trips_both_depths() {
        let b = NearLosslessBudget::from_8bit_steps(2);
        assert_eq!(b.max_error_at_depth(8), 2, "±2 at 8-bit");
        assert_eq!(b.max_error_at_depth(16), 514, "same fraction at 16-bit");
        assert!(!b.is_exact());
        assert!(NearLosslessBudget::EXACT.is_exact());
        assert_eq!(NearLosslessBudget::EXACT.max_error_at_depth(8), 0);
    }

    #[test]
    fn budget_max_is_full_range() {
        assert_eq!(NearLosslessBudget::MAX.max_error_at_depth(8), 255);
        assert_eq!(NearLosslessBudget::MAX.max_error_at_depth(16), 65535);
    }

    #[test]
    fn budget_default_and_steps() {
        assert_eq!(
            NearLosslessBudget::DEFAULT,
            NearLosslessBudget::from_8bit_steps(2)
        );
        // from_8bit_steps(1) is exactly one 8-bit LSB.
        assert_eq!(
            NearLosslessBudget::from_8bit_steps(1).max_error_at_depth(8),
            1
        );
        assert_eq!(
            NearLosslessBudget::from_8bit_steps(255),
            NearLosslessBudget::MAX
        );
    }

    #[test]
    fn budget_from_fraction_is_clamped() {
        assert_eq!(
            NearLosslessBudget::from_fraction(-1.0),
            NearLosslessBudget::EXACT
        );
        assert_eq!(
            NearLosslessBudget::from_fraction(2.0),
            NearLosslessBudget::MAX
        );
        // ~2/255 ≈ 0.00784 → 2 at 8-bit.
        assert_eq!(
            NearLosslessBudget::from_fraction(2.0 / 255.0).max_error_at_depth(8),
            2
        );
    }

    #[test]
    fn fidelity_is_lossless() {
        assert!(Fidelity::Lossless.is_lossless());
        assert!(Fidelity::NearLossless(NearLosslessBudget::EXACT).is_lossless());
        assert!(!Fidelity::NearLossless(NearLosslessBudget::DEFAULT).is_lossless());
        assert!(!Fidelity::quality(90.0).is_lossless());
    }

    #[test]
    fn classify_exact_and_unsupported() {
        let q = Fidelity::quality(85.0);
        assert_eq!(
            classify_fidelity_match(q, Some(q)),
            FidelityMatch::Supported
        );
        assert_eq!(classify_fidelity_match(q, None), FidelityMatch::Unsupported);
    }

    #[test]
    fn classify_promote_to_lossless() {
        let nl = Fidelity::NearLossless(NearLosslessBudget::DEFAULT);
        assert_eq!(
            classify_fidelity_match(nl, Some(Fidelity::Lossless)),
            FidelityMatch::Lossless
        );
    }

    #[test]
    fn classify_quality_raised_and_lowered() {
        let req = Fidelity::quality(83.0);
        assert_eq!(
            classify_fidelity_match(req, Some(Fidelity::quality(85.0))),
            FidelityMatch::TargetRaised(Fidelity::quality(85.0))
        );
        assert_eq!(
            classify_fidelity_match(req, Some(Fidelity::quality(80.0))),
            FidelityMatch::TargetLowered(Fidelity::quality(80.0))
        );
    }

    #[test]
    fn classify_metric_translated() {
        let req = Fidelity::Lossy(LossyTarget::Metric {
            metric: QualityMetric::Ssimulacra2,
            target: 90.0,
        });
        let got = Fidelity::quality(88.0);
        assert_eq!(
            classify_fidelity_match(req, Some(got)),
            FidelityMatch::MetricTranslated(got)
        );
    }

    #[test]
    fn fidelity_match_resolved_and_honored() {
        assert!(FidelityMatch::Supported.is_honored());
        assert!(!FidelityMatch::Unsupported.is_honored());
        assert_eq!(FidelityMatch::Lossless.resolved(), Some(Fidelity::Lossless));
        assert_eq!(FidelityMatch::Supported.resolved(), None);
    }
}
