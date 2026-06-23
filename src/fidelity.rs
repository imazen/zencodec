//! Encode fidelity: how faithfully an encoder reproduces its input.
//!
//! [`Fidelity`] is the complete fidelity request — *exactly one of*:
//! - **lossy**, aiming at a [`LossyTarget`] (today a SSIMULACRA2 score, a
//!   butteraugli max-norm distance, or the codec's own native quality dial),
//! - **near-lossless**, within a per-channel [`NearLosslessBudget`], or
//! - **mathematically lossless**.
//!
//! It is a sum type so each regime carries the parameter its own metric needs,
//! illegal states (lossy ∧ lossless) are unrepresentable, and lossless is
//! explicit rather than "quality == 100". See `docs/near-lossless-design.md`
//! for the full rationale and per-codec mapping.
//!
//! **Scope.** The initial surface is *blind, single-pass* fidelity: a calibrated
//! target maps to a native dial in one encode, no re-encode loop. Iterative
//! ("closed-loop") targeting — re-encoding until a *measured* metric/size is hit
//! — is intentionally not shipped yet; [`LossyTarget`] reserves the names so it
//! can be added later without renaming the one-shot arms.

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
    ///
    /// Few codecs honor a true L∞ ceiling natively (PNG exactly, WebP to the
    /// nearest power-of-two step); others promote to exact lossless or
    /// approximate perceptually. The codec reports what it did via
    /// [`resolved_target_fidelity`](crate::encode::EncoderConfig::resolved_target_fidelity).
    NearLossless(NearLosslessBudget),
    /// Mathematically exact — decode reproduces the input sample-for-sample.
    Lossless,
}

impl Fidelity {
    /// Lossy, aiming at a SSIMULACRA2 score via a single calibrated pass.
    #[must_use]
    pub const fn ssim2(score: f32) -> Self {
        Self::Lossy(LossyTarget::ApproxSsim2(score))
    }

    /// Lossy, aiming at a butteraugli **max-norm** distance via a single
    /// calibrated pass (`distance` lower is better; ≈1.0 high quality).
    #[must_use]
    pub const fn butteraugli_max(distance: f32) -> Self {
        Self::Lossy(LossyTarget::ApproxButteraugliMax(distance))
    }

    /// Lossy, on the codec's own native quality scale (codec-specific meaning —
    /// see [`LossyTarget::CodecSpecificQuality`]).
    #[must_use]
    pub const fn codec_quality(q: f32) -> Self {
        Self::Lossy(LossyTarget::CodecSpecificQuality(q))
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
/// Three things we can target **today**, each in a single blind pass (no
/// re-encode):
/// - [`ApproxSsim2`](Self::ApproxSsim2) — a SSIMULACRA2 score.
/// - [`ApproxButteraugliMax`](Self::ApproxButteraugliMax) — a butteraugli
///   **max-norm** distance (worst-region; lower is better).
/// - [`CodecSpecificQuality`](Self::CodecSpecificQuality) — the codec's own
///   native quality dial, honest that its meaning differs per codec.
///
/// There is deliberately **no generic `Quality`** arm: the codec-agnostic
/// `generic_quality` scale is not yet standardized (we have no agreed
/// cross-codec meaning for "quality 75"), so exposing it as a `Fidelity` target
/// would promise a standard we don't have. It is reserved (commented below)
/// until standardized.
///
/// The reserved arms split **one-shot** perceptual targets (`Approx*`, a single
/// calibrated pass) from **closed-loop** targets (re-encode until a *measured*
/// value is hit), so loop targeting can be added later without renaming the
/// one-shot arms. We target the butteraugli **max-norm** here; the **3-norm**
/// aggregate is reserved as a separate arm (the two norms differ — a bare
/// `Distance(f32)` is ambiguous and is never an arm). zensim is deferred — no
/// reliable metric yet.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LossyTarget {
    /// Aim for a SSIMULACRA2 score (≈0–100, higher is better) in a single
    /// calibrated pass — no re-encode. "Approx" marks it as blind one-shot; a
    /// closed-loop `Ssim2` variant can be added later without renaming this.
    ApproxSsim2(f32),
    /// Aim for a butteraugli **max-norm** distance (the worst-region p-norm,
    /// p→∞) in a single calibrated pass — no re-encode. Lower is better; ≈1.0
    /// is high quality, ≈0.5 near-visually-lossless. "Approx" marks it blind
    /// one-shot; a closed-loop `ButteraugliMaxLoop` can be added later.
    ApproxButteraugliMax(f32),
    /// The codec's **native** quality dial, on its own scale. The meaning is
    /// codec-specific — there is no cross-codec standard here (unlike a metric
    /// target). Use when you know the codec and want its raw knob.
    CodecSpecificQuality(f32),
    //
    // ─── Reserved: blind one-shot perceptual targets ─────────────────────────
    // Single calibrated pass (target → native dial), no re-encode. The 3-norm
    // aggregate complements the active max-norm arm above:
    //     ApproxButteraugli3Norm(f32),    // 3-norm / pnorm (aggregate)
    //
    // ─── Reserved: standardized generic quality ──────────────────────────────
    // A cross-codec quality scale, once `generic_quality` has an agreed meaning:
    //     Quality(f32),
    //
    // ─── Reserved: closed-loop targets ───────────────────────────────────────
    // Re-encode until a *measured* value is hit. Deferred — no closed-loop
    // machinery is wired yet:
    //     Ssim2Loop(f32),
    //     ButteraugliMaxLoop(f32),
    //     TargetBytes(u64),               // hit an encoded-size budget
    //     Bitrate(f32),                   // hit a bits-per-pixel budget
}

/// Coarse butteraugli-distance → 0–100 quality fallback used by the default
/// [`with_fidelity`](crate::encode::EncoderConfig::with_fidelity) when a codec
/// has not implemented native butteraugli targeting. Inverse of the de-facto
/// jpegli curve `d ≈ 0.1 + (100 − q)·0.09`, clamped. Codecs with native
/// butteraugli targeting override `with_fidelity` and never reach this.
pub(crate) fn butteraugli_max_distance_to_quality(distance: f32) -> f32 {
    (100.0 - (distance - 0.1) / 0.09).clamp(0.0, 100.0)
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
        assert!(!Fidelity::ssim2(90.0).is_lossless());
        assert!(!Fidelity::butteraugli_max(1.0).is_lossless());
        assert!(!Fidelity::codec_quality(90.0).is_lossless());
    }

    #[test]
    fn fidelity_constructors() {
        assert_eq!(
            Fidelity::ssim2(90.0),
            Fidelity::Lossy(LossyTarget::ApproxSsim2(90.0))
        );
        assert_eq!(
            Fidelity::butteraugli_max(1.0),
            Fidelity::Lossy(LossyTarget::ApproxButteraugliMax(1.0))
        );
        assert_eq!(
            Fidelity::codec_quality(85.0),
            Fidelity::Lossy(LossyTarget::CodecSpecificQuality(85.0))
        );
    }

    #[test]
    fn butteraugli_fallback_curve_is_monotone_and_clamped() {
        // d ≈ 1.0 → q ≈ 90 (the de-facto jpegli anchor); lower d → higher q.
        assert!((butteraugli_max_distance_to_quality(1.0) - 90.0).abs() < 0.01);
        assert_eq!(butteraugli_max_distance_to_quality(0.1), 100.0);
        assert!(
            butteraugli_max_distance_to_quality(0.5) > butteraugli_max_distance_to_quality(2.0)
        );
        // far ends clamp to [0, 100].
        assert_eq!(butteraugli_max_distance_to_quality(-5.0), 100.0);
        assert_eq!(butteraugli_max_distance_to_quality(100.0), 0.0);
    }
}
