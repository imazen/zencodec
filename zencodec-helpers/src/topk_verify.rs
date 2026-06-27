//! Runtime **top-K-verify** picker helper — the key gap.
//!
//! A pure content picker (raw argmin, `K=1`) leaves a residual oracle gap:
//! measured ~2.4% true-argmin on the JXL-lossy picker, where the proven
//! *narrow-by-content, finalize-by-RD-check* design reaches ~0.48% (≤1% MET)
//! at small `K`. The catch found by audit: **no per-codec picker reaches the
//! top-K path at runtime** — `zenwebp`/`zenjpeg` both call a single
//! [`argmin`](zenpredict::argmin) and use the picked config blindly. This
//! module closes that gap with one generic helper.
//!
//! ## What it does
//!
//! Given a picker's predicted per-cell costs (log-bytes, the typical zen
//! picker output), an allow-mask, and two caller-supplied closures —
//! `encode(cell) -> bytes` and `score(cell) -> quality` — it:
//!
//!   1. ranks the allowed cells by the picker's **predicted** cost (cheapest
//!      first), the same ranking the offline `evaluate_topk_verify` oracle gap
//!      uses to choose `K`;
//!   2. walks the K predicted-cheapest cells, **actually encoding** each (the
//!      verify step the offline sim only modeled);
//!   3. returns the cell with the **fewest actual bytes whose actual quality
//!      meets the target** (or, if none of the K reach the target, the best
//!      quality seen — see [`VerifyOutcome`]).
//!
//! `K` is the encode budget: `K=1` reproduces raw argmin (one encode), larger
//! `K` trades encodes for closing the RD gap. The offline oracle says small
//! `K` (2–3) already buys most of the gap.
//!
//! ## Generic by construction
//!
//! The helper knows nothing codec-specific. A codec wires it up with:
//!
//!   - its picker's output slice + the cell sub-range + an [`AllowedMask`];
//!   - a `VerifyConfig` mapping cell index → (the encode it should run, its
//!     measured byte count, its measured quality).
//!
//! The encode/score closures own all codec types; this crate stays at the
//! "pick one of N predicted-cheapest, verify by re-encoding" layer.

use zenpredict::{AllowedMask, ArgminOffsets, ScoreTransform};

/// Which way "better quality" runs for the metric the verify step scores.
///
/// Most zen perceptual scores (zensim Profile-A, SSIMULACRA2, butteraugli-as-
/// quality) are **higher-is-better**; a target of e.g. 80 means "achieved
/// quality ≥ 80". A few distance metrics (raw butteraugli JND, where lower is
/// closer to the reference) are **lower-is-better**; a target then means
/// "achieved distance ≤ target".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QualityDirection {
    /// Achieved quality must be **≥** the target (zensim / SSIM2 / …).
    HigherIsBetter,
    /// Achieved distance must be **≤** the target (butteraugli JND / …).
    LowerIsBetter,
}

impl QualityDirection {
    /// `true` when `achieved` satisfies the `target` under this direction.
    #[inline]
    pub fn meets(self, achieved: f32, target: f32) -> bool {
        // NaN never meets a target (NaN comparisons are false), so a
        // pathological score can't be mistaken for a passing encode.
        match self {
            Self::HigherIsBetter => achieved >= target,
            Self::LowerIsBetter => achieved <= target,
        }
    }
}

/// Result of one verified encode of a single candidate cell.
///
/// One is produced per candidate from the `encode` + `score` closures of
/// [`pick_top_k_verify`]. `bytes` is the encoded size in bytes; `quality` is
/// the metric the target is expressed in.
#[derive(Clone, Copy, Debug)]
pub struct Measured {
    /// Cell index in the picker's output range that produced this encode.
    pub cell: usize,
    /// Encoded size in bytes (the thing we minimize).
    pub bytes: u64,
    /// Achieved quality / distance, in the target's units.
    pub quality: f32,
}

/// What [`pick_top_k_verify`] settled on.
#[derive(Clone, Copy, Debug)]
pub enum VerifyOutcome {
    /// At least one verified cell met the quality target; this is the
    /// **fewest-bytes** such cell. The common, happy path.
    MetTarget(Measured),
    /// None of the K verified cells met the target — the best-quality cell
    /// among them is returned so the caller can still emit *something* (and
    /// knows the target was missed). For higher-is-better that's the max
    /// quality; for lower-is-better, the min distance.
    BestEffort(Measured),
    /// The mask permitted no cells in the range (caller must relax
    /// constraints), or `K == 0` so nothing was verified.
    NoCandidate,
}

impl VerifyOutcome {
    /// The chosen measurement if any encode happened, else `None`.
    pub fn measured(&self) -> Option<Measured> {
        match self {
            Self::MetTarget(m) | Self::BestEffort(m) => Some(*m),
            Self::NoCandidate => None,
        }
    }

    /// `true` only for [`VerifyOutcome::MetTarget`].
    pub fn met_target(&self) -> bool {
        matches!(self, Self::MetTarget(_))
    }
}

/// How the top-K verify reads the picker output + ranks + bounds the encode
/// budget. The encode/score work itself is the closures passed to
/// [`pick_top_k_verify`]; this struct is the non-closure knobs.
#[derive(Clone, Copy, Debug)]
pub struct VerifyConfig<'a> {
    /// Sub-range `(start, end)` of the picker output holding the per-cell
    /// **predicted cost** (log-bytes for the typical zen picker). The cell
    /// indices the closures receive are *relative to `start`* — i.e. `0` is
    /// the first cell of the range — matching `argmin_masked_in_range`.
    pub cost_range: (usize, usize),
    /// Score transform applied to the predicted cost before ranking. Pickers
    /// that emit log-bytes pass [`ScoreTransform::Exp`] so the ranking is in
    /// linear-byte space (and any per-cell byte offsets mix correctly).
    pub transform: ScoreTransform,
    /// Optional additive cost adjustments (caller-side ICC / EXIF overhead
    /// that the model didn't see), applied in the post-transform space before
    /// ranking — same semantics as [`ArgminOffsets`].
    pub offsets: Option<&'a ArgminOffsets<'a>>,
    /// Quality target the verified encode must satisfy.
    pub target_quality: f32,
    /// Direction of the quality metric (see [`QualityDirection`]).
    pub direction: QualityDirection,
}

impl<'a> VerifyConfig<'a> {
    /// Convenience constructor: log-bytes picker (Exp transform, no offsets),
    /// higher-is-better target over the whole `0..n_cells` range.
    pub fn log_bytes(n_cells: usize, target_quality: f32) -> Self {
        Self {
            cost_range: (0, n_cells),
            transform: ScoreTransform::Exp,
            offsets: None,
            target_quality,
            direction: QualityDirection::HigherIsBetter,
        }
    }
}

/// The maximum K any caller can request. Picks are tiny (codec ranges are
/// 10s–100s of cells, K is 2–8 in practice); this caps stack arrays and
/// rejects absurd budgets.
pub const MAX_K: usize = 16;

/// Errors that abort a verify before any useful outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyError {
    /// `K` exceeded [`MAX_K`].
    KTooLarge { requested: usize, max: usize },
    /// `cost_range` was out of bounds for the predictions slice, or
    /// `start > end`.
    BadRange,
    /// The mask length didn't match the cost-range length.
    MaskLenMismatch { range_len: usize, mask_len: usize },
}

/// The `K` lowest-scoring indices of `predictions` that `mask` permits,
/// ascending (best first); slots past the number of allowed entries are `None`.
///
/// Self-contained masked top-K selection — this is the piece the runtime needs
/// that zenpredict only exposes behind a gated feature, so we own it here and
/// depend on zenpredict's **default** API alone. Score for index `i` is
/// `transform(predictions[i]) + uniform + per_output[i]`, identical to
/// zenpredict's `argmin_masked` / `ArgminOffsets` semantics, so a picker's
/// log-bytes output ranks the same way under [`ScoreTransform::Exp`].
///
/// `K` is a const generic so the result buffer is stack-allocated (alloc-free).
/// In practice `K` is 2–8.
///
/// # Contract (matches `zenpredict::argmin::argmin_masked`)
///
/// - **Mask length:** `mask.len() >= predictions.len()` is required; a shorter
///   mask **panics** (debug + release) rather than silently denying high-index
///   cells.
/// - **NaN scores:** any index whose post-transform/offset score is NaN is
///   **silently skipped** (`NaN < x` is false), so a NaN cell is never picked.
/// - **Tie-breaking:** equal scores break to the **lower index** (deterministic,
///   first-encountered), via a strict `<` insertion test.
///
/// ```
/// use zencodec_helpers::topk_verify::select_top_k;
/// use zenpredict::{AllowedMask, ScoreTransform};
///
/// let scores = [3.0_f32, 1.0, 4.0, 1.5];
/// let allow = [true, false, true, true]; // index 1 (the 1.0) masked out
/// let mask = AllowedMask::new(&allow);
/// let top = select_top_k::<2>(&scores, &mask, ScoreTransform::Identity, None);
/// assert_eq!(top, [Some(3), Some(0)]); // 1.5 then 3.0 (index 1 excluded)
/// ```
pub fn select_top_k<const K: usize>(
    predictions: &[f32],
    mask: &AllowedMask<'_>,
    transform: ScoreTransform,
    offsets: Option<&ArgminOffsets<'_>>,
) -> [Option<usize>; K] {
    assert!(
        mask.len() >= predictions.len(),
        "select_top_k: mask.len() ({}) < predictions.len() ({}) — short masks \
         used to silently deny high-index cells",
        mask.len(),
        predictions.len(),
    );

    // Insertion-sorted buffer of the K best `(score, idx)` seen so far,
    // ascending by score (same shape as a masked top-K insertion sort). We
    // guard NaN explicitly below so a NaN cell is never selected, per the
    // documented contract.
    let mut top: [(f32, usize); K] = [(f32::INFINITY, usize::MAX); K];
    let mut count: usize = 0;

    for (i, &raw) in predictions.iter().enumerate() {
        if !mask.is_allowed(i) {
            continue;
        }
        let score = score_at(raw, i, transform, offsets);
        // NaN scores are silently skipped (the contract): a NaN cell must never
        // be selected. The `else if score < …` replace path already excludes
        // NaN (`NaN < x` is false), but the `count < K` fill path would insert
        // it unguarded — so guard NaN explicitly here.
        if score.is_nan() {
            continue;
        }
        if count < K {
            let mut j = count;
            while j > 0 && top[j - 1].0 > score {
                top[j] = top[j - 1];
                j -= 1;
            }
            top[j] = (score, i);
            count += 1;
        } else if K > 0 && score < top[K - 1].0 {
            let mut j = K - 1;
            while j > 0 && top[j - 1].0 > score {
                top[j] = top[j - 1];
                j -= 1;
            }
            top[j] = (score, i);
        }
    }

    let mut out: [Option<usize>; K] = [None; K];
    for (slot, dst) in out.iter_mut().enumerate().take(count) {
        *dst = Some(top[slot].1);
    }
    out
}

/// Score one cell: `transform(raw) + uniform + per_output[idx]`. Matches
/// zenpredict's internal `score_at` so ranking is identical.
#[inline]
fn score_at(
    raw: f32,
    idx: usize,
    transform: ScoreTransform,
    offsets: Option<&ArgminOffsets<'_>>,
) -> f32 {
    let mut s = transform.apply(raw);
    if let Some(o) = offsets {
        s += o.uniform;
        if let Some(po) = o.per_output
            && let Some(&v) = po.get(idx)
        {
            s += v;
        }
    }
    s
}

/// Build an [`AllowedMask`]-ready bool vector from a **caller-supplied** per-cell
/// effort/compute-tier slice + a `max_tier` ceiling: cell `i` is allowed iff
/// `tiers[i] <= max_tier`.
///
/// The codec knows its cells' effort tiers from its own config grammar, so it
/// passes them in — this crate does **not** read zenpredict metadata for it
/// (and zenpredict needs no accessor added). AND the result with any other
/// constraint mask before calling [`pick_top_k_verify`].
///
/// Returns one bool per `tiers` entry. (Convenience-only: a codec that already
/// has its bool mask doesn't need this.)
///
/// ```
/// use zencodec_helpers::topk_verify::tier_mask;
/// // per-cell effort tiers; allow tiers <= 2.
/// let tiers = [0u8, 1, 2, 3, 2];
/// assert_eq!(tier_mask(&tiers, 2), [true, true, true, false, true]);
/// ```
pub fn tier_mask(tiers: &[u8], max_tier: u8) -> alloc::vec::Vec<bool> {
    tiers.iter().map(|&t| t <= max_tier).collect()
}

/// Predict the top-K cheapest cells, **encode + score each**, and return the
/// min-bytes cell that meets the quality target.
///
/// This is the runtime realization of the proven top-K-verify design. It is
/// generic over a codec via two closures:
///
///   - `encode(cell) -> u64` — actually encode candidate `cell` (index
///     relative to `cost_range.0`) and return its byte count. The codec owns
///     the encoded bytes; only the length flows back here. (Codecs that want
///     to *keep* the winning bitstream should cache it inside the closure
///     keyed on `cell`, then re-read after the pick — encoding is the
///     expensive step, so caching avoids a redundant final encode.)
///   - `score(cell) -> f32` — score that encode's quality in the target's
///     units. Invoked once per verified cell, right after `encode(cell)`.
///
/// Both closures are called **at most `K` times**, in predicted-cheapest
/// order, and only for mask-allowed cells. Ranking uses this crate's
/// [`select_top_k`] — a self-contained masked top-K selection over the
/// `predict_transformed` output slice (the same thing the offline oracle's
/// `np.argsort` models), so this crate needs only zenpredict's **default** API.
///
/// # Errors
///
/// [`VerifyError`] for a bad K / range / mask before any encode runs. A run
/// that encodes but finds nothing meeting the target is **not** an error — it
/// returns [`VerifyOutcome::BestEffort`].
///
/// # Determinism
///
/// Ties in predicted cost break to the lower cell index (see [`select_top_k`]).
/// Among verified cells that meet the target, ties in actual bytes break to the
/// one encountered first in predicted-cheapest order.
///
/// # K must be a const
///
/// `K` is a const generic so the candidate buffer is stack-allocated and the
/// helper stays alloc-free. `K=3` is the proven sweet spot
/// (`pick_top_3_verify`).
pub fn pick_top_k_verify<const K: usize, Enc, Score>(
    predictions: &[f32],
    mask: &AllowedMask<'_>,
    config: &VerifyConfig<'_>,
    mut encode: Enc,
    mut score: Score,
) -> Result<VerifyOutcome, VerifyError>
where
    Enc: FnMut(usize) -> u64,
    Score: FnMut(usize) -> f32,
{
    if K == 0 {
        return Ok(VerifyOutcome::NoCandidate);
    }
    if K > MAX_K {
        return Err(VerifyError::KTooLarge {
            requested: K,
            max: MAX_K,
        });
    }
    let (start, end) = config.cost_range;
    if start > end || end > predictions.len() {
        return Err(VerifyError::BadRange);
    }
    let range_len = end - start;
    if mask.len() != range_len {
        return Err(VerifyError::MaskLenMismatch {
            range_len,
            mask_len: mask.len(),
        });
    }

    // Rank the allowed cells by predicted cost (cheapest first), entirely in
    // this crate — `select_top_k` over the sub-range slice. Indices are
    // relative to the sub-range, which is exactly what the closures expect.
    let slice = &predictions[start..end];
    let ranked: [Option<usize>; K] =
        select_top_k::<K>(slice, mask, config.transform, config.offsets);

    // Verify each candidate in predicted order: encode → score → track the
    // min-bytes target-meeter, and (separately) the best-quality fallback for
    // the BestEffort branch when none meet the target.
    let mut best_meeting: Option<Measured> = None;
    let mut best_effort: Option<Measured> = None;

    for slot in ranked.into_iter() {
        let Some(cell) = slot else { break }; // None slots come after all
        // allowed cells; nothing more to verify.
        let bytes = encode(cell);
        let quality = score(cell);
        let m = Measured {
            cell,
            bytes,
            quality,
        };

        if config.direction.meets(quality, config.target_quality) {
            // Keep the fewest-bytes cell that meets the target. `<` (not `<=`)
            // preserves the predicted-order tie-break: the first-encountered
            // wins on a byte tie.
            match best_meeting {
                Some(prev) if prev.bytes <= m.bytes => {}
                _ => best_meeting = Some(m),
            }
        }

        // Track best-effort fallback regardless. "Best" = closest to passing.
        best_effort = Some(match best_effort {
            None => m,
            Some(prev) => match config.direction {
                QualityDirection::HigherIsBetter => {
                    if m.quality > prev.quality {
                        m
                    } else {
                        prev
                    }
                }
                QualityDirection::LowerIsBetter => {
                    if m.quality < prev.quality {
                        m
                    } else {
                        prev
                    }
                }
            },
        });
    }

    Ok(match (best_meeting, best_effort) {
        (Some(m), _) => VerifyOutcome::MetTarget(m),
        (None, Some(m)) => VerifyOutcome::BestEffort(m),
        (None, None) => VerifyOutcome::NoCandidate,
    })
}

/// The proven K=3 specialization. Encodes at most the 3 predicted-cheapest
/// reachable cells; the JXL-lossy picker reached val 0.52% / test 0.42%
/// top-3-verify gap (≤1% target MET). See [`pick_top_k_verify`] for the
/// generic form and the contract.
#[inline]
pub fn pick_top_3_verify<Enc, Score>(
    predictions: &[f32],
    mask: &AllowedMask<'_>,
    config: &VerifyConfig<'_>,
    encode: Enc,
    score: Score,
) -> Result<VerifyOutcome, VerifyError>
where
    Enc: FnMut(usize) -> u64,
    Score: FnMut(usize) -> f32,
{
    pick_top_k_verify::<3, _, _>(predictions, mask, config, encode, score)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- select_top_k: the self-contained masked top-K selection -------------

    #[test]
    fn select_top_k_ascending_best_first() {
        let scores = [3.0_f32, 1.0, 4.0, 1.5, 9.0];
        let allow = [true; 5];
        let mask = AllowedMask::new(&allow);
        // 3 smallest ascending: 1.0(idx1), 1.5(idx3), 3.0(idx0).
        let top = select_top_k::<3>(&scores, &mask, ScoreTransform::Identity, None);
        assert_eq!(top, [Some(1), Some(3), Some(0)]);
    }

    #[test]
    fn select_top_k_respects_mask_and_fills_none() {
        let scores = [3.0_f32, 1.0, 4.0, 1.5];
        let allow = [true, false, true, true]; // mask out the 1.0
        let mask = AllowedMask::new(&allow);
        // Only 3 allowed; ask for K=4 → last slot None. Order: 1.5,3.0,4.0.
        let top = select_top_k::<4>(&scores, &mask, ScoreTransform::Identity, None);
        assert_eq!(top, [Some(3), Some(0), Some(2), None]);
    }

    #[test]
    fn select_top_k_nan_scores_skipped() {
        let scores = [f32::NAN, 2.0, f32::NAN, 1.0];
        let allow = [true; 4];
        let mask = AllowedMask::new(&allow);
        // NaN cells never selected; only 1.0(idx3),2.0(idx1) qualify.
        let top = select_top_k::<3>(&scores, &mask, ScoreTransform::Identity, None);
        assert_eq!(top, [Some(3), Some(1), None]);
    }

    #[test]
    fn select_top_k_ties_break_to_lower_index() {
        let scores = [5.0_f32, 5.0, 5.0];
        let allow = [true; 3];
        let mask = AllowedMask::new(&allow);
        let top = select_top_k::<3>(&scores, &mask, ScoreTransform::Identity, None);
        assert_eq!(
            top,
            [Some(0), Some(1), Some(2)],
            "equal scores → lower index first"
        );
    }

    #[test]
    fn select_top_k_exp_transform_and_offsets() {
        // Log-bytes picker: Exp ranks in linear space. e^3.0≈20.09, e^3.1≈22.20.
        // A +5 per-output offset on idx0 flips it: 25.09 > 22.20 → idx1 first.
        let scores = [3.0_f32, 3.1];
        let allow = [true; 2];
        let mask = AllowedMask::new(&allow);
        let per_output = [5.0_f32, 0.0];
        let offsets = ArgminOffsets {
            uniform: 0.0,
            per_output: Some(&per_output),
        };
        let top = select_top_k::<2>(&scores, &mask, ScoreTransform::Exp, Some(&offsets));
        assert_eq!(top, [Some(1), Some(0)]);
    }

    #[test]
    fn select_top_k_matches_zenpredict_argmin_masked_winner() {
        // The K=1 winner of select_top_k must equal zenpredict's (default-API)
        // argmin_masked — proving our self-contained selection agrees with the
        // upstream primitive on the single-best pick.
        let scores = [3.0_f32, 1.0, 4.0, 1.5, 9.0];
        let allow = [true, false, true, true, true];
        let mask = AllowedMask::new(&allow);
        let ours = select_top_k::<1>(&scores, &mask, ScoreTransform::Identity, None)[0];
        let upstream = zenpredict::argmin_masked(&scores, &mask, ScoreTransform::Identity, None);
        assert_eq!(ours, upstream, "select_top_k::<1> winner == argmin_masked");
        assert_eq!(ours, Some(3));
    }

    #[test]
    #[should_panic(expected = "select_top_k")]
    fn select_top_k_panics_on_short_mask() {
        let scores = [1.0_f32, 2.0, 3.0];
        let allow = [true, true]; // shorter than predictions
        let mask = AllowedMask::new(&allow);
        let _ = select_top_k::<2>(&scores, &mask, ScoreTransform::Identity, None);
    }

    // --- tier_mask -----------------------------------------------------------

    #[test]
    fn tier_mask_allows_at_or_below_max() {
        let tiers = [0u8, 1, 2, 3, 2];
        assert_eq!(tier_mask(&tiers, 2), [true, true, true, false, true]);
        assert_eq!(tier_mask(&tiers, 0), [true, false, false, false, false]);
        assert_eq!(tier_mask(&tiers, 9), [true; 5]);
    }

    // A tiny synthetic picker: `n` cells, caller supplies predicted log-bytes,
    // actual bytes, and actual quality per cell. Lets us assert the helper's
    // pick against a hand-computed oracle without a real codec.
    struct Fixture {
        pred_log_bytes: Vec<f32>,
        actual_bytes: Vec<u64>,
        actual_quality: Vec<f32>,
        encodes: std::cell::RefCell<Vec<usize>>,
    }

    impl Fixture {
        fn n(&self) -> usize {
            self.pred_log_bytes.len()
        }
        fn encode(&self, cell: usize) -> u64 {
            self.encodes.borrow_mut().push(cell);
            self.actual_bytes[cell]
        }
        fn score(&self, cell: usize) -> f32 {
            self.actual_quality[cell]
        }
    }

    fn run<const K: usize>(
        fx: &Fixture,
        mask_data: &[bool],
        target: f32,
        dir: QualityDirection,
    ) -> Result<VerifyOutcome, VerifyError> {
        let mask = AllowedMask::new(mask_data);
        let cfg = VerifyConfig {
            cost_range: (0, fx.n()),
            transform: ScoreTransform::Exp,
            offsets: None,
            target_quality: target,
            direction: dir,
        };
        pick_top_k_verify::<K, _, _>(
            &fx.pred_log_bytes,
            &mask,
            &cfg,
            |c| fx.encode(c),
            |c| fx.score(c),
        )
    }

    #[test]
    fn picks_min_bytes_among_target_meeters_in_top_k() {
        // 5 cells. Predicted-cheapest order: 0,1,2,3,4 (already ascending).
        // With K=3 we verify cells {0,1,2}. Qualities: only cells 1 and 2 meet
        // target 80. Actual bytes 1→500, 2→400 → pick cell 2 (fewer bytes),
        // even though cell 0 is predicted-cheapest (it misses quality).
        let fx = Fixture {
            pred_log_bytes: vec![1.0, 2.0, 3.0, 4.0, 5.0],
            actual_bytes: vec![100, 500, 400, 300, 200],
            actual_quality: vec![60.0, 85.0, 90.0, 95.0, 99.0],
            encodes: Default::default(),
        };
        let out = run::<3>(&fx, &[true; 5], 80.0, QualityDirection::HigherIsBetter).unwrap();
        match out {
            VerifyOutcome::MetTarget(m) => {
                assert_eq!(m.cell, 2, "min-bytes target-meeter in top-3");
                assert_eq!(m.bytes, 400);
            }
            other => panic!("expected MetTarget(cell 2), got {other:?}"),
        }
        // Only the 3 predicted-cheapest cells were ever encoded.
        let enc = fx.encodes.borrow();
        assert_eq!(*enc, vec![0, 1, 2], "exactly the top-3 cheapest encoded");
    }

    #[test]
    fn k1_reproduces_raw_argmin_one_encode() {
        // K=1: verify only the predicted-cheapest cell. Even though it misses
        // the target, that's all we encode (raw-argmin behavior), and the
        // result is BestEffort on that single cell.
        let fx = Fixture {
            pred_log_bytes: vec![1.0, 2.0, 3.0],
            actual_bytes: vec![100, 200, 300],
            actual_quality: vec![60.0, 85.0, 90.0],
            encodes: Default::default(),
        };
        let out = run::<1>(&fx, &[true; 3], 80.0, QualityDirection::HigherIsBetter).unwrap();
        match out {
            VerifyOutcome::BestEffort(m) => assert_eq!(m.cell, 0),
            other => panic!("expected BestEffort(cell 0), got {other:?}"),
        }
        assert_eq!(
            *fx.encodes.borrow(),
            vec![0],
            "K=1 encodes exactly one cell"
        );
    }

    #[test]
    fn best_effort_returns_highest_quality_when_none_meet() {
        // No cell meets target 99.5 within the top-3. BestEffort returns the
        // highest-quality verified cell (cell 2 at 90), not the cheapest.
        let fx = Fixture {
            pred_log_bytes: vec![1.0, 2.0, 3.0, 4.0],
            actual_bytes: vec![100, 200, 300, 400],
            actual_quality: vec![60.0, 80.0, 90.0, 99.9],
            encodes: Default::default(),
        };
        let out = run::<3>(&fx, &[true; 4], 99.5, QualityDirection::HigherIsBetter).unwrap();
        match out {
            VerifyOutcome::BestEffort(m) => {
                assert_eq!(m.cell, 2, "best-effort = highest verified quality");
                assert_eq!(m.quality, 90.0);
            }
            other => panic!("expected BestEffort(cell 2), got {other:?}"),
        }
    }

    #[test]
    fn mask_excludes_cells_from_verification() {
        // Mask out the two predicted-cheapest (0,1). Top-3 over the remaining
        // {2,3,4} → verify 2,3,4. Cell 3 meets target 90 at fewest bytes.
        let fx = Fixture {
            pred_log_bytes: vec![1.0, 2.0, 3.0, 4.0, 5.0],
            actual_bytes: vec![10, 20, 900, 300, 800],
            actual_quality: vec![99.0, 99.0, 70.0, 92.0, 95.0],
            encodes: Default::default(),
        };
        let out = run::<3>(
            &fx,
            &[false, false, true, true, true],
            90.0,
            QualityDirection::HigherIsBetter,
        )
        .unwrap();
        match out {
            VerifyOutcome::MetTarget(m) => assert_eq!(m.cell, 3),
            other => panic!("expected MetTarget(cell 3), got {other:?}"),
        }
        let enc = fx.encodes.borrow();
        assert!(
            !enc.contains(&0) && !enc.contains(&1),
            "masked cells not encoded"
        );
    }

    #[test]
    fn lower_is_better_direction() {
        // Distance metric (butteraugli JND): target ≤ 1.5. Cells 1,2 meet it
        // (1.2, 0.8). Cell 1 has fewer bytes (200 < 400) → pick cell 1.
        let fx = Fixture {
            pred_log_bytes: vec![1.0, 2.0, 3.0],
            actual_bytes: vec![100, 200, 400],
            actual_quality: vec![3.0, 1.2, 0.8], // distances; lower better
            encodes: Default::default(),
        };
        let out = run::<3>(&fx, &[true; 3], 1.5, QualityDirection::LowerIsBetter).unwrap();
        match out {
            VerifyOutcome::MetTarget(m) => {
                assert_eq!(m.cell, 1, "min-bytes cell whose distance ≤ target");
            }
            other => panic!("expected MetTarget(cell 1), got {other:?}"),
        }
    }

    #[test]
    fn no_candidate_when_mask_empty() {
        let fx = Fixture {
            pred_log_bytes: vec![1.0, 2.0],
            actual_bytes: vec![1, 2],
            actual_quality: vec![1.0, 2.0],
            encodes: Default::default(),
        };
        let out = run::<3>(&fx, &[false, false], 0.0, QualityDirection::HigherIsBetter).unwrap();
        assert!(matches!(out, VerifyOutcome::NoCandidate));
        assert!(
            fx.encodes.borrow().is_empty(),
            "no encodes when nothing allowed"
        );
    }

    #[test]
    fn k0_is_no_candidate_no_encode() {
        let fx = Fixture {
            pred_log_bytes: vec![1.0, 2.0],
            actual_bytes: vec![1, 2],
            actual_quality: vec![99.0, 99.0],
            encodes: Default::default(),
        };
        let out = run::<0>(&fx, &[true, true], 0.0, QualityDirection::HigherIsBetter).unwrap();
        assert!(matches!(out, VerifyOutcome::NoCandidate));
        assert!(fx.encodes.borrow().is_empty());
    }

    #[test]
    fn rejects_bad_range_and_mask_len() {
        let preds = [1.0_f32, 2.0, 3.0];
        let mask3 = [true; 3];
        // mask length must match the range length, not the whole slice.
        let cfg_bad_mask = VerifyConfig {
            cost_range: (0, 2),
            transform: ScoreTransform::Exp,
            offsets: None,
            target_quality: 0.0,
            direction: QualityDirection::HigherIsBetter,
        };
        let err = pick_top_k_verify::<2, _, _>(
            &preds,
            &AllowedMask::new(&mask3),
            &cfg_bad_mask,
            |_| 0,
            |_| 0.0,
        )
        .unwrap_err();
        assert_eq!(
            err,
            VerifyError::MaskLenMismatch {
                range_len: 2,
                mask_len: 3
            }
        );

        // Out-of-bounds range.
        let cfg_oob = VerifyConfig {
            cost_range: (0, 99),
            ..cfg_bad_mask
        };
        assert_eq!(
            pick_top_k_verify::<2, _, _>(
                &preds,
                &AllowedMask::new(&mask3),
                &cfg_oob,
                |_| 0,
                |_| 0.0
            )
            .unwrap_err(),
            VerifyError::BadRange
        );
    }

    #[test]
    fn top_3_alias_matches_generic() {
        let fx = Fixture {
            pred_log_bytes: vec![1.0, 2.0, 3.0, 4.0],
            actual_bytes: vec![500, 400, 300, 200],
            actual_quality: vec![85.0, 86.0, 70.0, 60.0],
            encodes: Default::default(),
        };
        let mask = AllowedMask::new(&[true; 4]);
        let cfg = VerifyConfig::log_bytes(4, 80.0);
        let a = pick_top_3_verify(
            &fx.pred_log_bytes,
            &mask,
            &cfg,
            |c| fx.encode(c),
            |c| fx.score(c),
        )
        .unwrap();
        // top-3 = {0,1,2}; meeters {0,1}; min bytes → cell 1 (400 < 500).
        match a {
            VerifyOutcome::MetTarget(m) => assert_eq!(m.cell, 1),
            other => panic!("expected MetTarget(cell 1), got {other:?}"),
        }
    }
}
