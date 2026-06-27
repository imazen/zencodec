//! Offline **top-K-verify oracle-gap** evaluation — the metric that decides
//! the K encode budget for the runtime [`pick_top_k_verify`].
//!
//! Faithful port of `evaluate_topk_verify` in `zentrain/tools/train_hybrid.py`.
//! Given, per row (image × target), the picker's **predicted** per-cell
//! log-bytes, the **actual** per-cell log-bytes, a per-cell reachability mask
//! (cells that meet the quality target for that row), and a global allow-mask,
//! it answers: *"if we encode just the K predicted-cheapest reachable cells and
//! keep the min-actual-bytes one, how far above the per-row oracle (min actual
//! over ALL reachable cells) are we — at each K?"*
//!
//! `K=1` reproduces raw-argmin overhead; `K = n_reachable` → 0%. The loop reads
//! the per-K mean / p50 / p90 / p99 / max overhead, the hit-rate (oracle inside
//! the predicted top-K), and the mean cells verified, to choose the smallest K
//! that clears the ≤1% gap.
//!
//! This is the *training-time* twin of the runtime helper: same ranking
//! (predicted-cheapest), same finalize (min actual bytes among the verified
//! set) — so the K the loop validates here is the K the runtime uses.
//!
//! [`pick_top_k_verify`]: crate::topk_verify::pick_top_k_verify

use alloc::vec::Vec;

/// One row's per-cell vectors. All four slices index the same cell space.
pub struct Row<'a> {
    /// Picker's predicted log-bytes per cell.
    pub pred_log_bytes: &'a [f32],
    /// Measured log-bytes per cell (the ground truth).
    pub actual_log_bytes: &'a [f32],
    /// `reach[i]` = cell `i` meets the row's quality target.
    pub reach: &'a [bool],
}

/// Per-K overhead statistics, mirroring the Python dict shape.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KStats {
    pub k: usize,
    pub mean_pct: f64,
    pub p50_pct: f64,
    pub p90_pct: f64,
    pub p99_pct: f64,
    pub max_pct: f64,
    /// Fraction of rows whose oracle cell fell inside the predicted top-K.
    pub hit_rate: f64,
    /// Mean number of cells actually verified (≤ K, capped by reachable count).
    pub mean_verified: f64,
    /// Rows that contributed (had ≥1 reachable allowed cell).
    pub n_rows: usize,
}

/// Evaluate top-K-verify overhead across `rows` for each K in `ks`. `mask` is
/// the global allow-mask ANDed with each row's `reach`. Returns one [`KStats`]
/// per requested K, in the same order as `ks`.
///
/// A row with no reachable+allowed cell is skipped (doesn't count toward
/// `n_rows`), exactly like the Python `if not np.any(m): continue`.
pub fn evaluate_topk_verify(rows: &[Row<'_>], mask: &[bool], ks: &[usize]) -> Vec<KStats> {
    // Per-K accumulators: overhead fractions, hit count, verified counts.
    let mut ovh: Vec<Vec<f64>> = ks.iter().map(|_| Vec::with_capacity(rows.len())).collect();
    let mut hits: Vec<usize> = ks.iter().map(|_| 0).collect();
    let mut verified: Vec<Vec<usize>> = ks.iter().map(|_| Vec::with_capacity(rows.len())).collect();
    let mut n_used = 0usize;

    for row in rows {
        let n = row.pred_log_bytes.len();
        // Effective per-cell mask: reachable AND globally allowed.
        let allowed = |i: usize| {
            row.reach.get(i).copied().unwrap_or(false) && mask.get(i).copied().unwrap_or(false)
        };
        let n_reach = (0..n).filter(|&i| allowed(i)).count();
        if n_reach == 0 {
            continue;
        }
        n_used += 1;

        // Actual bytes (linear) over allowed cells, +inf elsewhere → never
        // chosen. The oracle is the global min-actual over allowed cells.
        let actual_lin = |i: usize| {
            if allowed(i) {
                exp_clamped(row.actual_log_bytes[i])
            } else {
                f64::INFINITY
            }
        };
        let (oracle_idx, oracle_bytes) = (0..n)
            .map(|i| (i, actual_lin(i)))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(core::cmp::Ordering::Equal))
            .expect("n_reach > 0 guarantees a finite min");

        // Stable sort of allowed cells by PREDICTED linear bytes, cheapest
        // first (unreachable sort last via +inf) — matches np.argsort stable.
        let mut order: Vec<usize> = (0..n).collect();
        let pred_lin = |i: usize| {
            if allowed(i) {
                exp_clamped(row.pred_log_bytes[i])
            } else {
                f64::INFINITY
            }
        };
        order.sort_by(|&a, &b| {
            pred_lin(a)
                .partial_cmp(&pred_lin(b))
                .unwrap_or(core::cmp::Ordering::Equal)
                .then(a.cmp(&b)) // stable: lower index breaks ties
        });

        for (ki, &k) in ks.iter().enumerate() {
            let kk = k.min(n_reach);
            let topk = &order[..kk];
            let best_actual = topk
                .iter()
                .map(|&i| actual_lin(i))
                .fold(f64::INFINITY, f64::min);
            ovh[ki].push((best_actual - oracle_bytes) / oracle_bytes);
            if topk.contains(&oracle_idx) {
                hits[ki] += 1;
            }
            verified[ki].push(kk);
        }
    }

    ks.iter()
        .enumerate()
        .map(|(ki, &k)| {
            let o = &ovh[ki];
            KStats {
                k,
                mean_pct: 100.0 * mean(o),
                p50_pct: 100.0 * percentile(o, 50.0),
                p90_pct: 100.0 * percentile(o, 90.0),
                p99_pct: 100.0 * percentile(o, 99.0),
                max_pct: 100.0 * o.iter().copied().fold(0.0_f64, f64::max),
                hit_rate: hits[ki] as f64 / n_used.max(1) as f64,
                mean_verified: mean_usize(&verified[ki]),
                n_rows: n_used,
            }
        })
        .collect()
}

/// `exp` with the same [-30, 30] input clamp `zenpredict::ScoreTransform::Exp`
/// and the Python (`np.clip(..., -30, 30)`) use, so the linear-space ranking is
/// identical to the runtime path.
#[inline]
fn exp_clamped(x: f32) -> f64 {
    f64::from(x.clamp(-30.0, 30.0)).exp()
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn mean_usize(xs: &[usize]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<usize>() as f64 / xs.len() as f64
}

/// Linear-interpolated percentile matching numpy's default ('linear') method,
/// so the reported p50/p90/p99 line up with the Python harness.
fn percentile(xs: &[f64], pct: f64) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f64> = xs.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    if s.len() == 1 {
        return s[0];
    }
    let rank = (pct / 100.0) * (s.len() as f64 - 1.0);
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        s[lo]
    } else {
        let frac = rank - lo as f64;
        s[lo] * (1.0 - frac) + s[hi] * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k1_overhead_equals_raw_argmin_gap() {
        // One row, 3 cells, all reachable+allowed. Predicted-cheapest = cell 0
        // (pred 1.0). Actual bytes: cell 0 is NOT the oracle — cell 2 is
        // cheaper actually. So K=1 has a gap; K=3 reaches the oracle (0%).
        let pred = [1.0_f32, 2.0, 3.0];
        let actual = [
            (300.0_f64).ln() as f32, // cell 0 actual 300
            (250.0_f64).ln() as f32, // cell 1 actual 250
            (200.0_f64).ln() as f32, // cell 2 actual 200 (oracle)
        ];
        let reach = [true, true, true];
        let row = Row {
            pred_log_bytes: &pred,
            actual_log_bytes: &actual,
            reach: &reach,
        };
        let stats = evaluate_topk_verify(&[row], &[true; 3], &[1, 2, 3]);
        // K=1: only cell 0 verified → 300 vs oracle 200 → 50% overhead.
        assert!(
            (stats[0].mean_pct - 50.0).abs() < 1e-3,
            "K=1 {}",
            stats[0].mean_pct
        );
        assert_eq!(stats[0].hit_rate, 0.0); // oracle (cell 2) not in top-1
        // K=3: all verified → oracle inside → 0% overhead, hit_rate 1.0.
        assert!(stats[2].mean_pct.abs() < 1e-6, "K=3 should be 0%");
        assert_eq!(stats[2].hit_rate, 1.0);
        assert_eq!(stats[2].n_rows, 1);
    }

    #[test]
    fn unreachable_cells_excluded_and_row_skipped_when_none_reach() {
        // Row A: only cell 1 reachable. Row B: nothing reachable → skipped.
        let pred_a = [1.0_f32, 2.0];
        let act_a = [(100.0_f64).ln() as f32, (500.0_f64).ln() as f32];
        let reach_a = [false, true]; // cell 0 unreachable
        let row_a = Row {
            pred_log_bytes: &pred_a,
            actual_log_bytes: &act_a,
            reach: &reach_a,
        };
        let pred_b = [1.0_f32, 2.0];
        let act_b = [0.0_f32, 0.0];
        let reach_b = [false, false]; // none reachable
        let row_b = Row {
            pred_log_bytes: &pred_b,
            actual_log_bytes: &act_b,
            reach: &reach_b,
        };
        let stats = evaluate_topk_verify(&[row_a, row_b], &[true; 2], &[1]);
        // Only row A counts. Its sole reachable cell is the oracle → 0% gap.
        assert_eq!(stats[0].n_rows, 1);
        assert!(stats[0].mean_pct.abs() < 1e-6);
        assert!(
            (stats[0].mean_verified - 1.0).abs() < 1e-9,
            "1 cell reachable"
        );
    }

    #[test]
    fn global_mask_intersects_reach() {
        // Cell 0 reachable but globally masked out → falls back to cell 1.
        let pred = [1.0_f32, 2.0];
        let act = [(100.0_f64).ln() as f32, (200.0_f64).ln() as f32];
        let reach = [true, true];
        let row = Row {
            pred_log_bytes: &pred,
            actual_log_bytes: &act,
            reach: &reach,
        };
        let stats = evaluate_topk_verify(&[row], &[false, true], &[1]);
        // Only cell 1 is allowed → it's the oracle of the allowed set → 0%.
        assert!(stats[0].mean_pct.abs() < 1e-6);
        assert_eq!(stats[0].n_rows, 1);
    }

    #[test]
    fn percentile_matches_numpy_linear() {
        // numpy.percentile([1,2,3,4], 50) == 2.5 (linear interpolation).
        let xs = [1.0, 2.0, 3.0, 4.0];
        assert!((percentile(&xs, 50.0) - 2.5).abs() < 1e-9);
        assert!((percentile(&xs, 0.0) - 1.0).abs() < 1e-9);
        assert!((percentile(&xs, 100.0) - 4.0).abs() < 1e-9);
    }
}
