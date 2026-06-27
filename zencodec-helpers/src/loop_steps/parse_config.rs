//! `parse_config_name` robustness — decomposing a sweep's opaque cell-id
//! string into its categorical + scalar axes, with a roundtrip + max-deviation
//! self-check that catches grammar drift.
//!
//! Each codec's plan emits cell-ids in its own grammar (`vd-e7_zen_def` for JXL
//! lossy_dense, `jp3_tr14.75cpl+1cl1_small_420` for zenjpeg scalar_dense, …).
//! The ad-hoc Python parsers historically **crashed** on sub-knobs they didn't
//! anticipate (see the zenjpeg config comment: "Robust to the cpl/blur/bracket
//! sub-knobs the old parser crashed on"). This module factors out the robust
//! shape: a [`ConfigGrammar`] trait that parses to a [`ParsedConfig`] and can
//! recompose it, plus a [`validate_cell_ids`] sweep that flags parse failures,
//! roundtrip failures, and max-deviation violations across a whole sweep's
//! cell-id set — the exact checks `test_lossy_dense_parse.py` runs.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

use hashbrown_or_std::Map;

/// A parsed cell-id: its categorical axes (string-valued, the picker treats
/// them as opaque categories) and its scalar axes (float-valued, the picker
/// learns / interpolates). Mirrors the dict `parse_config_name` returns in
/// `train_hybrid.py`, which partitions keys into categorical vs scalar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedConfig {
    /// Categorical axis name → value (e.g. `"strategy" -> "zen"`).
    pub categorical: Map<String, String>,
    /// Scalar axis name → value (e.g. `"effort" -> 7.0`).
    pub scalar: Map<String, f64>,
}

impl ParsedConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: add a categorical axis.
    pub fn with_cat(mut self, key: &str, value: &str) -> Self {
        self.categorical.insert(key.to_owned(), value.to_owned());
        self
    }

    /// Builder: add a scalar axis.
    pub fn with_scalar(mut self, key: &str, value: f64) -> Self {
        self.scalar.insert(key.to_owned(), value);
        self
    }

    /// Number of axes that deviate from a baseline config — used by the
    /// `--max-deviations` plan budget. An axis deviates when its value differs
    /// from the baseline's value for that key (categorical: string !=; scalar:
    /// not bit-equal after the codec's own rounding — here exact `!=`, since
    /// cell-ids encode discretized values).
    pub fn deviations_from(&self, baseline: &ParsedConfig) -> usize {
        let mut n = 0;
        for (k, v) in &self.categorical {
            if baseline.categorical.get(k).map(String::as_str) != Some(v.as_str()) {
                n += 1;
            }
        }
        for (k, v) in &self.scalar {
            match baseline.scalar.get(k) {
                Some(b) if b == v => {}
                _ => n += 1,
            }
        }
        n
    }
}

/// A codec's cell-id grammar: parse a cell-id string to a [`ParsedConfig`] and
/// recompose it back. Implementors should make `recompose(parse(id)) == id` for
/// every well-formed id (the roundtrip the validator checks).
pub trait ConfigGrammar {
    /// Parse a cell-id; `None` if it doesn't match the grammar.
    fn parse(&self, cell_id: &str) -> Option<ParsedConfig>;

    /// Recompose a parsed config back to its canonical cell-id string. Used by
    /// the roundtrip check; a faithful grammar roundtrips losslessly.
    fn recompose(&self, parsed: &ParsedConfig) -> Option<String>;

    /// The codec's baseline config — the all-default cell that deviations are
    /// counted against (for the max-deviation check). Default: empty (every
    /// axis counts as a deviation), which codecs override.
    fn baseline(&self) -> ParsedConfig {
        ParsedConfig::new()
    }

    /// Plan-budget invariant: the maximum number of axes a single cell may
    /// deviate from [`Self::baseline`]. `None` = unbounded (no check).
    fn max_deviations(&self) -> Option<usize> {
        None
    }
}

/// Outcome of validating a sweep's full cell-id set against a grammar — the
/// Rust analog of `test_lossy_dense_parse.py`'s report.
#[derive(Clone, Debug, Default)]
pub struct ValidationReport {
    /// Total distinct cell-ids checked.
    pub n_cells: usize,
    /// Cell-ids the grammar failed to parse.
    pub parse_failures: Vec<String>,
    /// `(cell_id, recomposed)` where `recompose(parse(id)) != id`.
    pub roundtrip_failures: Vec<(String, String)>,
    /// `(cell_id, deviations)` exceeding the grammar's `max_deviations`.
    pub max_deviation_violations: Vec<(String, usize)>,
}

impl ValidationReport {
    /// `true` iff there were no failures of any kind.
    pub fn ok(&self) -> bool {
        self.parse_failures.is_empty()
            && self.roundtrip_failures.is_empty()
            && self.max_deviation_violations.is_empty()
    }
}

/// Validate every cell-id against `grammar`, collecting parse / roundtrip /
/// max-deviation failures. This is the gate the per-codec sweep should run on
/// its omni TSV's distinct cell-ids before training — a parser that crashes (or
/// silently mis-parses) on a sub-knob produces a poisoned categorical axis.
pub fn validate_cell_ids<G, I>(grammar: &G, cell_ids: I) -> ValidationReport
where
    G: ConfigGrammar,
    I: IntoIterator<Item = String>,
{
    let baseline = grammar.baseline();
    let max_dev = grammar.max_deviations();
    let mut report = ValidationReport::default();
    for cid in cell_ids {
        report.n_cells += 1;
        let Some(parsed) = grammar.parse(&cid) else {
            report.parse_failures.push(cid);
            continue;
        };
        if let Some(limit) = max_dev {
            let dev = parsed.deviations_from(&baseline);
            if dev > limit {
                report.max_deviation_violations.push((cid.clone(), dev));
            }
        }
        match grammar.recompose(&parsed) {
            Some(re) if re == cid => {}
            Some(re) => report.roundtrip_failures.push((cid, re)),
            None => report.roundtrip_failures.push((cid, String::new())),
        }
    }
    report
}

// --- Tiny std/no_std map shim ------------------------------------------------
// loop-tools is always `std`, but keep the import path uniform so the modules
// read the same whether or not a future no_std consumer appears.
mod hashbrown_or_std {
    pub use std::collections::BTreeMap as Map;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loop_tools::grammars::JxlLossyDense;

    #[test]
    fn jxl_lossy_dense_parses_and_roundtrips_canonical_ids() {
        let g = JxlLossyDense;
        // vd-e{effort}_{strategy}_{knob}[-{flag}]
        let p = g.parse("vd-e7_zen_def").unwrap();
        assert_eq!(p.scalar.get("effort"), Some(&7.0));
        assert_eq!(
            p.categorical.get("strategy").map(String::as_str),
            Some("zen")
        );
        assert_eq!(p.categorical.get("knob").map(String::as_str), Some("def"));
        assert_eq!(p.categorical.get("flag").map(String::as_str), Some("none"));
        assert_eq!(g.recompose(&p).as_deref(), Some("vd-e7_zen_def"));

        // with a flag suffix
        let p2 = g.parse("vd-e9_zen_epf2-g1").unwrap();
        assert_eq!(p2.categorical.get("flag").map(String::as_str), Some("g1"));
        assert_eq!(g.recompose(&p2).as_deref(), Some("vd-e9_zen_epf2-g1"));
    }

    #[test]
    fn jxl_lossy_dense_validation_clean_set_passes() {
        let g = JxlLossyDense;
        let ids = [
            "vd-e7_zen_def".to_owned(),    // baseline
            "vd-e9_zen_def".to_owned(),    // effort deviates (1)
            "vd-e7_zen_epf2".to_owned(),   // knob deviates (1)
            "vd-e7_alt_def".to_owned(),    // strategy deviates (1)
            "vd-e7_zen_def-g1".to_owned(), // flag deviates (1)
        ];
        let r = validate_cell_ids(&g, ids);
        assert!(r.ok(), "clean ≤1-deviation set should pass: {r:?}");
        assert_eq!(r.n_cells, 5);
    }

    #[test]
    fn jxl_lossy_dense_flags_unparseable_and_overbudget() {
        let g = JxlLossyDense;
        let ids = [
            "garbage_cell".to_owned(),      // parse failure
            "vd-e9_alt_epf2-g1".to_owned(), // 4 deviations > max_deviations(1)
        ];
        let r = validate_cell_ids(&g, ids);
        assert_eq!(r.parse_failures, ["garbage_cell"]);
        assert_eq!(
            r.max_deviation_violations,
            [("vd-e9_alt_epf2-g1".to_owned(), 4)]
        );
    }

    #[test]
    fn deviation_count_partitions_categorical_and_scalar() {
        let base = ParsedConfig::new()
            .with_cat("strategy", "zen")
            .with_scalar("effort", 7.0);
        let same = base.clone();
        assert_eq!(same.deviations_from(&base), 0);
        let two = ParsedConfig::new()
            .with_cat("strategy", "alt") // deviates
            .with_scalar("effort", 9.0); // deviates
        assert_eq!(two.deviations_from(&base), 2);
    }
}
