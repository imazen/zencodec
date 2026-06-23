//! Encode fidelity: how faithfully an encoder reproduces its input.
//!
//! [`Fidelity`] is the complete fidelity request — *exactly one of*:
//! - **[`Lossless`](Fidelity::Lossless)** — mathematically exact;
//! - **[`LosslessMode`](Fidelity::LosslessMode)** — lossless *coding*
//!   (predictive: PNG, VP8L, JXL-modular, GIF — no transform, no DCT ringing) of
//!   pixels pre-quantized within a [`LosslessModeParams`] budget. Spans
//!   near-lossless through aggressive (screen content: crisp + small), all
//!   artifact-free;
//! - **[`Lossy`](Fidelity::Lossy)** — lossy *coding* (transform/DCT: JPEG, VP8,
//!   AVIF, VarDCT) aiming at a [`LossyTarget`].
//!
//! **Container is the variant.** The choice of variant *is* the coding family —
//! `Lossless`/`LosslessMode` are artifact-free predictive coding, `Lossy` is
//! transform coding — so the container the caller cares about (PNG-style vs
//! JPEG-style) is a direct, top-level choice, and illegal combinations (e.g.
//! "exact transform") are unrepresentable. Bit depth, HDR, gamut, and color stay
//! in the *input* `PixelDescriptor` and the color-emit layers — except
//! *output-encode* directives that only make sense for a lossless encode (e.g.
//! output bit depth), which the input descriptor can't express and so live in
//! [`LosslessModeParams`].
//!
//! **Scope.** The initial surface is *blind, single-pass* fidelity: a calibrated
//! target maps to a native dial in one encode, no re-encode loop. Iterative
//! ("closed-loop") targeting is intentionally not shipped yet; [`LossyTarget`]
//! reserves the names so it can be added later without renaming the one-shot
//! arms.

/// The complete fidelity request for an encode — exactly one of three things.
///
/// Set with [`EncoderConfig::with_fidelity`](crate::encode::EncoderConfig::with_fidelity);
/// read what the codec resolved with
/// [`resolved_target_fidelity`](crate::encode::EncoderConfig::resolved_target_fidelity).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Fidelity {
    /// Mathematically exact — decode reproduces the input sample-for-sample.
    /// Artifact-free predictive coding; preserves the input's depth and
    /// representation.
    Lossless,
    /// Lossless *coding* (predictive — no transform, no ringing) of pixels
    /// pre-quantized within a [`LosslessModeParams`] budget. Spans near-lossless
    /// to aggressive (screen content). Few codecs honor a precise budget
    /// natively (PNG L∞ bit-rounding, WebP near-lossless dial); others promote to
    /// exact lossless and report it via
    /// [`resolved_target_fidelity`](crate::encode::EncoderConfig::resolved_target_fidelity).
    LosslessMode(LosslessModeParams),
    /// Lossy *coding* (transform/DCT). *What* it aims at is a [`LossyTarget`].
    Lossy(LossyTarget),
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
    pub const fn butteraugli(distance: f32) -> Self {
        Self::Lossy(LossyTarget::ApproxButteraugli(distance))
    }

    /// Lossy, on the codec's own native quality scale (codec-specific meaning —
    /// see [`LossyTarget::CodecSpecificQuality`]).
    #[must_use]
    pub const fn codec_quality(q: f32) -> Self {
        Self::Lossy(LossyTarget::CodecSpecificQuality(q))
    }

    /// Near-lossless within an L∞-per-channel `budget` — a [`LosslessMode`]
    /// convenience. `near_lossless(NearLosslessBudget::EXACT)` is equivalent to
    /// [`Lossless`](Self::Lossless); prefer the plain variant for the exact case.
    #[must_use]
    pub const fn near_lossless(budget: NearLosslessBudget) -> Self {
        Self::LosslessMode(LosslessModeParams::new(LosslessBudget::MaxChannelError(
            budget,
        )))
    }

    /// Whether this request is mathematically lossless (exact `Lossless`, or a
    /// `LosslessMode` whose budget permits no loss).
    #[must_use]
    pub const fn is_lossless(self) -> bool {
        match self {
            Self::Lossless => true,
            Self::LosslessMode(p) => p.budget.is_exact(),
            Self::Lossy(_) => false,
        }
    }
}

/// Parameters for a [`LosslessMode`](Fidelity::LosslessMode) encode — the loss
/// budget, plus room for *output-encode* directives that only make sense for a
/// lossless encode and that the *input* [`PixelDescriptor`](zenpixels::PixelDescriptor)
/// cannot express.
///
/// A **struct** (not a bare budget) on purpose, matching the load-bearing
/// descriptive structs elsewhere in the stack: it grows additively as codecs
/// gain directives. Fields are private — construct via [`new`](Self::new) and
/// the builders, read via the getters — so every new field is a non-breaking
/// addition. **Reserved next fields** (added when a codec actually honors them):
/// output bit depth (16-bit input → 12-bit lossless output, which PNG/JXL can
/// honor — distinct from the *input* descriptor's depth) and lossless
/// representation choices (reversible color transform, palette vs truecolor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LosslessModeParams {
    budget: LosslessBudget,
    // Reserved — add when a codec honors it (each is additive on this struct):
    //   output_depth: Option<OutputDepth>,   // PNG/JXL: encode at a chosen depth
    //   representation: ReprChoice,           // RCT / palette / truecolor
}

impl LosslessModeParams {
    /// New params for the given `budget`, with codec defaults for everything
    /// else.
    #[must_use]
    pub const fn new(budget: LosslessBudget) -> Self {
        Self { budget }
    }

    /// The loss budget for this lossless-coding encode.
    #[must_use]
    pub const fn budget(&self) -> LosslessBudget {
        self.budget
    }
}

/// The bound a [`LosslessMode`](Fidelity::LosslessMode) encode must respect — the
/// kind of "near" in near-lossless.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LosslessBudget {
    /// L∞-per-channel ceiling: no channel of any pixel may change by more than
    /// this. PNG LSB-rounding (exact), WebP near-lossless (capped). Broadly
    /// supported — the one near-lossless contract every lossless codec can honor.
    MaxChannelError(NearLosslessBudget),
    //
    // ─── Reserved ────────────────────────────────────────────────────────────
    // Perceptual(f32) — bounded SSIMULACRA2 within a lossless container (PNG's
    //   zenquant path, calibrated on 27k libjpeg-turbo + 1992 MPE↔SSIM2
    //   measurements). PNG-only today; add (capability-gated) once VP8L /
    //   JXL-modular-lossy are swept so the cross-codec promise is real.
}

impl LosslessBudget {
    /// Whether the budget permits no loss (≡ [`Fidelity::Lossless`]).
    #[must_use]
    pub const fn is_exact(self) -> bool {
        match self {
            Self::MaxChannelError(b) => b.is_exact(),
        }
    }
}

/// What a lossy encode aims at.
///
/// Three things we can target **today**, each in a single blind pass (no
/// re-encode):
/// - [`ApproxSsim2`](Self::ApproxSsim2) — a SSIMULACRA2 score.
/// - [`ApproxButteraugli`](Self::ApproxButteraugli) — a butteraugli
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
    /// one-shot; a closed-loop `ButteraugliLoop` can be added later. Named
    /// without a norm suffix — max-norm is the standardized butteraugli target;
    /// the 3-norm aggregate is reserved as its own arm.
    ApproxButteraugli(f32),
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
    //     ButteraugliLoop(f32),
    //     TargetBytes(u64),               // hit an encoded-size budget
    //     Bitrate(f32),                   // hit a bits-per-pixel budget
}

/// The maximum a near-lossless encode may change **any single channel of any
/// single pixel** — the L∞-per-channel ceiling — as a fraction of that
/// channel's full range. The payload of [`LosslessBudget::MaxChannelError`].
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
pub struct NearLosslessBudget {
    /// L∞-per-channel ceiling, as parts-per-65535 of full scale. Private — set
    /// via the constructors, read via [`as_fraction`](Self::as_fraction) /
    /// [`max_error_at_depth`](Self::max_error_at_depth).
    max_channel_error_per65535: u16,
}

impl NearLosslessBudget {
    /// Exact — `MaxChannelError(EXACT)` is equivalent to [`Fidelity::Lossless`].
    pub const EXACT: Self = Self {
        max_channel_error_per65535: 0,
    };
    /// The whole channel range (loosest possible budget).
    pub const MAX: Self = Self {
        max_channel_error_per65535: u16::MAX,
    };
    /// A sensible default (±2/255): visually transparent on photographic
    /// content, meaningfully smaller files. Use when you want "near-lossless"
    /// without choosing a number.
    pub const DEFAULT: Self = Self::from_8bit_steps(2);

    /// From the familiar 0–255 scale. `from_8bit_steps(2)` ⇒ `±2` on an 8-bit
    /// channel, and the same *fraction* (`±514`) on a 16-bit channel.
    #[must_use]
    pub const fn from_8bit_steps(n: u8) -> Self {
        // n ≤ 255 ⇒ n*257 ≤ 65535, exact in u16.
        Self {
            max_channel_error_per65535: ((n as u32) * 257) as u16,
        }
    }

    /// From the 0–65535 scale, for deep content.
    #[must_use]
    pub const fn from_16bit_steps(n: u16) -> Self {
        Self {
            max_channel_error_per65535: n,
        }
    }

    /// From a fraction of full range (depth-independent). Clamped to `[0, 1]`.
    #[must_use]
    pub fn from_fraction(f: f32) -> Self {
        let v = (f.clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
        Self {
            max_channel_error_per65535: if v > 65535 { 65535 } else { v as u16 },
        }
    }

    /// Whether this is the exact (zero-error) budget.
    #[must_use]
    pub const fn is_exact(self) -> bool {
        self.max_channel_error_per65535 == 0
    }

    /// The budget as a fraction of full scale (`0.0..=1.0`).
    #[must_use]
    pub fn as_fraction(self) -> f32 {
        f32::from(self.max_channel_error_per65535) / 65535.0
    }

    /// The integer L∞ ceiling (in LSBs) a `depth`-bit codec may not exceed.
    /// Exact integer math; the floor *is* the "round the guarantee down" rule.
    ///
    /// `from_8bit_steps(2).max_error_at_depth(8) == 2` and
    /// `from_8bit_steps(2).max_error_at_depth(16) == 514`.
    #[must_use]
    pub const fn max_error_at_depth(self, depth: u32) -> u32 {
        let full = (1u32 << depth) - 1;
        ((self.max_channel_error_per65535 as u32) * full) / 65535
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
        assert_eq!(
            NearLosslessBudget::from_fraction(2.0 / 255.0).max_error_at_depth(8),
            2
        );
    }

    #[test]
    fn fidelity_is_lossless() {
        assert!(Fidelity::Lossless.is_lossless());
        // near_lossless(EXACT) is a lossless spelling of LosslessMode.
        assert!(Fidelity::near_lossless(NearLosslessBudget::EXACT).is_lossless());
        assert!(!Fidelity::near_lossless(NearLosslessBudget::DEFAULT).is_lossless());
        assert!(!Fidelity::ssim2(90.0).is_lossless());
        assert!(!Fidelity::butteraugli(1.0).is_lossless());
        assert!(!Fidelity::codec_quality(90.0).is_lossless());
    }

    #[test]
    fn fidelity_constructors() {
        assert_eq!(
            Fidelity::ssim2(90.0),
            Fidelity::Lossy(LossyTarget::ApproxSsim2(90.0))
        );
        assert_eq!(
            Fidelity::butteraugli(1.0),
            Fidelity::Lossy(LossyTarget::ApproxButteraugli(1.0))
        );
        assert_eq!(
            Fidelity::codec_quality(85.0),
            Fidelity::Lossy(LossyTarget::CodecSpecificQuality(85.0))
        );
    }

    #[test]
    fn near_lossless_builds_lossless_mode_with_budget() {
        let f = Fidelity::near_lossless(NearLosslessBudget::DEFAULT);
        let Fidelity::LosslessMode(p) = f else {
            panic!("expected LosslessMode");
        };
        assert_eq!(
            p.budget(),
            LosslessBudget::MaxChannelError(NearLosslessBudget::DEFAULT)
        );
        assert!(!p.budget().is_exact());
    }
}
