//! Encode fidelity: how faithfully an encoder reproduces its input.
//!
//! [`Fidelity`] is the encode-fidelity request. Two variants ship today:
//! - **[`Lossless`](Fidelity::Lossless)** — mathematically exact.
//! - **[`Lossy`](Fidelity::Lossy)** — aiming at a one-shot [`LossyTarget`] (a
//!   SSIMULACRA2 score, a butteraugli max-norm distance, a zensim score, or the
//!   codec's own native quality dial).
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

    /// Lossy, aiming at a zensim score (`zensim::ZensimProfile::B`) via a single
    /// calibrated pass.
    #[must_use]
    pub const fn zensim_b(score: f32) -> Self {
        Self::Lossy(LossyTarget::ApproxZensimB(score))
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

/// Why a configuration did not accept the requested fidelity target unchanged.
/// This describes target selection, not a measured pixel-fidelity result.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum FidelityTargetError {
    /// A floating-point target is NaN or infinite.
    NonFinite,
    /// The configuration cannot report what target it selected.
    Unreported,
    /// The legacy setter selected a different target or value.
    Substituted { resolved: Fidelity },
}

impl core::fmt::Display for FidelityTargetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonFinite => f.write_str("fidelity target must be finite"),
            Self::Unreported => f.write_str("encoder cannot confirm the selected fidelity target"),
            Self::Substituted { resolved } => write!(f, "encoder substituted {resolved:?}"),
        }
    }
}
impl core::error::Error for FidelityTargetError {}

/// One rejected entry in an ordered fidelity preference list.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct FidelityRejection {
    pub index: usize,
    pub target: Fidelity,
    pub reason: FidelityTargetError,
}

/// Failure to select a fidelity target. Unsupported targets are reported in
/// caller order; selection never retries an encode or hides an encode failure.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum FidelitySelectionError {
    Empty,
    /// Invalid requests are caller errors, not a reason to downgrade fidelity.
    InvalidPreference {
        index: usize,
        reason: FidelityTargetError,
    },
    Rejected(alloc::vec::Vec<FidelityRejection>),
    /// Recording rejection reasons failed; no encoder has been run.
    AllocationFailed,
}
impl core::fmt::Display for FidelitySelectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str("fidelity preference list is empty"),
            Self::InvalidPreference { index, reason } => {
                write!(f, "invalid fidelity preference {index}: {reason}")
            }
            Self::Rejected(attempts) => write!(
                f,
                "all {} fidelity preferences were rejected: {attempts:?}",
                attempts.len()
            ),
            Self::AllocationFailed => f.write_str("could not allocate fidelity rejection report"),
        }
    }
}
impl core::error::Error for FidelitySelectionError {}

impl Fidelity {
    pub(crate) fn validate_target(self) -> Result<(), FidelityTargetError> {
        let value = match self {
            Self::Lossless => return Ok(()),
            Self::Lossy(
                LossyTarget::ApproxSsim2(v)
                | LossyTarget::ApproxButteraugli(v)
                | LossyTarget::ApproxZensimB(v)
                | LossyTarget::CodecSpecificQuality(v),
            ) => v,
        };
        if value.is_finite() {
            Ok(())
        } else {
            Err(FidelityTargetError::NonFinite)
        }
    }
}

pub(crate) fn select_target<T>(
    preferences: &[Fidelity],
    mut attempt: impl FnMut(Fidelity) -> Result<T, FidelityTargetError>,
) -> Result<(usize, T), FidelitySelectionError> {
    if preferences.is_empty() {
        return Err(FidelitySelectionError::Empty);
    }
    // Validate the entire request before trying candidates, even entries after
    // a supported one. A malformed preference must not be silently ignored.
    for (index, target) in preferences.iter().enumerate() {
        target
            .validate_target()
            .map_err(|reason| FidelitySelectionError::InvalidPreference { index, reason })?;
    }
    let mut rejected = alloc::vec::Vec::new();
    for (index, &target) in preferences.iter().enumerate() {
        match attempt(target) {
            Ok(value) => return Ok((index, value)),
            Err(reason) => {
                rejected
                    .try_reserve(1)
                    .map_err(|_| FidelitySelectionError::AllocationFailed)?;
                rejected.push(FidelityRejection {
                    index,
                    target,
                    reason,
                });
            }
        }
    }
    Err(FidelitySelectionError::Rejected(rejected))
}

// ═════════════════════════════════════════════════════════════════════════════
// DEFERRED — `try_with_fidelity` / `FidelityMatch` (insufficiently settled).
//
// A fail-fast counterpart to `with_fidelity` that reports how the codec resolved
// the request. Deferred: the raised/lowered/translated + content-dependent
// semantics aren't settled — a *config-time* verdict can't express GIF's
// content-dependent lossless, nor the HDR/bit-depth "is it really lossless?"
// question. Design + open questions: imazen/zencodec#104. Full prior impl in git
// history (commit 44fe22e).
//
//   enum FidelityMatch { Exact, RaisedTo(Fidelity), LoweredTo(Fidelity),
//                        Translated(Fidelity), Unsupported }
//   //   is_honored() / meets_or_exceeds() / resolved()
//   EncoderConfig::try_with_fidelity(&mut self, Fidelity) -> FidelityMatch
//
// What ships meanwhile: `with_fidelity` (infallible best-effort) +
// `resolved_target_fidelity` (the simple `Option<Fidelity>` resolved report).
// ═════════════════════════════════════════════════════════════════════════════

/// What a lossy encode aims at.
///
/// Four things we can target **today**, each in a single blind pass (no
/// re-encode):
/// - [`ApproxSsim2`](Self::ApproxSsim2) — a SSIMULACRA2 score.
/// - [`ApproxButteraugli`](Self::ApproxButteraugli) — a butteraugli
///   **max-norm** distance (worst-region; lower is better).
/// - [`ApproxZensimB`](Self::ApproxZensimB) — a zensim score on the
///   `ZensimProfile::B` scale.
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
/// `Distance(f32)` is ambiguous and is never an arm).
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
    /// Aim for a zensim score (≈0–100, higher is better) in a single calibrated
    /// pass — no re-encode. Pinned to `zensim::ZensimProfile::B` specifically
    /// (not "whichever profile is latest"): `B` is a **deterministic linear**
    /// profile — a closed-form lasso fit, byte-reproducible with no training
    /// seed — so a target expressed against it is stable across zensim patch
    /// releases the way this crate's other metric targets are. "Approx" marks
    /// it blind one-shot, matching
    /// `ApproxSsim2` / `ApproxButteraugli`; a closed-loop variant can be added
    /// later without renaming this. If a future zensim generation supersedes
    /// `B` as the recommended profile, that becomes a new sibling arm here —
    /// this one keeps meaning "generation-B" for as long as it exists.
    ApproxZensimB(f32),
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
        assert!(!Fidelity::zensim_b(90.0).is_lossless());
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
            Fidelity::zensim_b(90.0),
            Fidelity::Lossy(LossyTarget::ApproxZensimB(90.0))
        );
        assert_eq!(
            Fidelity::codec_quality(85.0),
            Fidelity::Lossy(LossyTarget::CodecSpecificQuality(85.0))
        );
    }
}
