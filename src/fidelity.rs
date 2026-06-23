//! Encode fidelity: how faithfully an encoder reproduces its input.
//!
//! [`Fidelity`] is the encode-fidelity request. Two variants ship today:
//! - **[`Lossless`](Fidelity::Lossless)** — mathematically exact.
//! - **[`Lossy`](Fidelity::Lossy)** — aiming at a one-shot [`LossyTarget`] (a
//!   SSIMULACRA2 score, a butteraugli max-norm distance, or the codec's own
//!   native quality dial).
//!
//! A third variant — **`LosslessMode`**: lossless *coding* (predictive, no DCT
//! ringing) of pixels pre-quantized within a budget — is **designed but
//! deferred** until its budget model is concrete (the L∞-vs-perceptual question
//! isn't settled: the perceptual budget is PNG-only today, and VP8L / JXL-modular
//! need sweeps). See the reserved-design block at the bottom of this file and
//! `docs/near-lossless-design.md`. When it lands it makes the *container* the
//! variant (predictive vs transform), so the screen-content path — crisp + small
//! in a lossless container — becomes a direct, top-level choice.
//!
//! **Scope.** Blind, single-pass: a calibrated target maps to a native dial in
//! one encode, no re-encode loop. Closed-loop targeting is reserved (see
//! [`LossyTarget`]).

/// The encode-fidelity request.
///
/// Set with [`EncoderConfig::with_fidelity`](crate::encode::EncoderConfig::with_fidelity);
/// read what the codec resolved with
/// [`resolved_target_fidelity`](crate::encode::EncoderConfig::resolved_target_fidelity).
///
/// `#[non_exhaustive]`: a `LosslessMode` variant is reserved (see the
/// deferred-design block at the bottom of `fidelity.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Fidelity {
    /// Mathematically exact — decode reproduces the input sample-for-sample.
    Lossless,
    /// Lossy coding, aiming at a [`LossyTarget`].
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

    /// Whether this request is mathematically lossless.
    #[must_use]
    pub const fn is_lossless(self) -> bool {
        match self {
            Self::Lossless => true,
            Self::Lossy(_) => false,
        }
    }
}

/// How a codec resolved a [`Fidelity`] request, returned by
/// [`try_with_fidelity`](crate::encode::EncoderConfig::try_with_fidelity).
///
/// The contract: a codec may resolve to *equal or greater* fidelity silently,
/// but a resolution to *less* fidelity across the lossy↔lossless fence is always
/// observable as [`Unsupported`](Self::Unsupported) — never a silent downgrade.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum FidelityMatch {
    /// Honored exactly as requested.
    Exact,
    /// Honored in the same (or a more faithful) regime, but resolved to a
    /// *different* fidelity — a metric/quality target rounded or translated to
    /// the codec's native scale, or a request promoted to exact lossless. The
    /// applied fidelity is in the payload; it is never *less* faithful than
    /// requested across the lossy↔lossless fence.
    Approximated(Fidelity),
    /// The codec cannot meet the requested regime without crossing to *less*
    /// fidelity (e.g. `Lossless` on a lossy-only codec), or has no fidelity
    /// control at all. [`with_fidelity`](crate::encode::EncoderConfig::with_fidelity)
    /// still produces best-effort output; the request is not met — pick a codec
    /// whose [capabilities](crate::EncodeCapabilities) cover it.
    Unsupported,
}

impl FidelityMatch {
    /// Whether the codec meets the request (anything but
    /// [`Unsupported`](Self::Unsupported)).
    #[must_use]
    pub const fn is_honored(self) -> bool {
        !matches!(self, Self::Unsupported)
    }

    /// The fidelity the codec resolved to when it differs from the request
    /// ([`Approximated`](Self::Approximated)); `None` for `Exact` / `Unsupported`.
    #[must_use]
    pub const fn resolved(self) -> Option<Fidelity> {
        match self {
            Self::Approximated(f) => Some(f),
            Self::Exact | Self::Unsupported => None,
        }
    }
}

/// Default classification for
/// [`try_with_fidelity`](crate::encode::EncoderConfig::try_with_fidelity): compare
/// the `requested` fidelity against what the codec `resolved` to.
pub(crate) fn classify_fidelity_match(
    requested: Fidelity,
    resolved: Option<Fidelity>,
) -> FidelityMatch {
    match resolved {
        // No fidelity control at all → can't claim to meet the request.
        None => FidelityMatch::Unsupported,
        Some(r) if r == requested => FidelityMatch::Exact,
        // Requested exact but resolved to lossy → demoted across the fence.
        Some(r) if requested.is_lossless() && !r.is_lossless() => FidelityMatch::Unsupported,
        // Same regime (or promoted to lossless) → honored, just not identical.
        Some(r) => FidelityMatch::Approximated(r),
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

// ═════════════════════════════════════════════════════════════════════════════
// DEFERRED DESIGN — `LosslessMode`: lossless *coding* with a loss budget.
//
// Deferred until the budget model (`LosslessBudget`) is concrete. The open
// question is L∞-vs-perceptual: the L∞ ceiling is broadly supported, but the
// perceptual budget is PNG-only today (the zenquant path, calibrated on 27k
// libjpeg-turbo + 1992 MPE↔SSIM2 measurements) — VP8L / JXL-modular-lossy need
// sweeps before a cross-codec perceptual promise is honest.
//
// When it lands, `LosslessMode` becomes a third `Fidelity` variant, making the
// *container* the variant: `Lossless`/`LosslessMode` are artifact-free predictive
// coding (PNG, VP8L, JXL-modular, GIF), `Lossy` is transform/DCT. That turns the
// container the caller cares about (PNG-style vs JPEG-style) into a direct,
// top-level choice, makes illegal combos ("exact transform") unrepresentable, and
// opens the screen-content path: lossy fidelity in a lossless container (crisp +
// small). Full rationale: `docs/near-lossless-design.md`. Full prior impl in git
// history (commit d36bff5).
//
//   enum Fidelity { Lossless, LosslessMode(LosslessModeParams), Lossy(LossyTarget) }
//
//   // Load-bearing struct (private fields + builders → additive): carries the
//   // budget now, plus reserved room for *output-encode* directives the input
//   // PixelDescriptor can't express (output bit depth — PNG/JXL can encode
//   // 16-bit input at a chosen depth; lossless representation — RCT, palette).
//   struct LosslessModeParams { budget: LosslessBudget, /* output_depth, repr… */ }
//
//   enum LosslessBudget {
//       MaxChannelError(NearLosslessBudget),  // L∞-per-channel ceiling (broad)
//       // Perceptual(f32),                   // bounded SSIMULACRA2 (PNG-only; reserved)
//   }
//
//   // L∞ ceiling as parts-per-65535 of full scale; resolves exactly at any depth.
//   struct NearLosslessBudget { max_channel_error_per65535: u16 }
//   //   EXACT / MAX / DEFAULT, from_8bit_steps / from_16bit_steps / from_fraction,
//   //   is_exact / as_fraction / max_error_at_depth(depth) → integer LSB ceiling.
//
//   Fidelity::near_lossless(b) -> LosslessMode(MaxChannelError(b))   // EXACT ≡ Lossless
//   with_fidelity default: Lossless | LosslessMode(_) -> with_lossless(true)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fidelity_is_lossless() {
        assert!(Fidelity::Lossless.is_lossless());
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
    fn classify_exact_approximated_unsupported() {
        let q = Fidelity::codec_quality(85.0);
        let got = Fidelity::codec_quality(80.0);
        // exact match
        assert_eq!(classify_fidelity_match(q, Some(q)), FidelityMatch::Exact);
        // no fidelity control → unsupported
        assert_eq!(classify_fidelity_match(q, None), FidelityMatch::Unsupported);
        // same regime, different value → approximated
        assert_eq!(
            classify_fidelity_match(q, Some(got)),
            FidelityMatch::Approximated(got)
        );
        // metric request resolved to a native quality (same lossy regime)
        assert_eq!(
            classify_fidelity_match(Fidelity::ssim2(90.0), Some(got)),
            FidelityMatch::Approximated(got)
        );
        // lossless requested, resolved lossy → demoted across the fence
        assert_eq!(
            classify_fidelity_match(Fidelity::Lossless, Some(got)),
            FidelityMatch::Unsupported
        );
        // lossy requested, promoted to lossless → more faithful → approximated
        assert_eq!(
            classify_fidelity_match(q, Some(Fidelity::Lossless)),
            FidelityMatch::Approximated(Fidelity::Lossless)
        );
    }

    #[test]
    fn fidelity_match_helpers() {
        assert!(FidelityMatch::Exact.is_honored());
        assert!(FidelityMatch::Approximated(Fidelity::Lossless).is_honored());
        assert!(!FidelityMatch::Unsupported.is_honored());
        assert_eq!(FidelityMatch::Exact.resolved(), None);
        assert_eq!(FidelityMatch::Unsupported.resolved(), None);
        assert_eq!(
            FidelityMatch::Approximated(Fidelity::Lossless).resolved(),
            Some(Fidelity::Lossless)
        );
    }
}
