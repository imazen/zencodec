//! Per-codec cell-id grammars implementing [`ConfigGrammar`].
//!
//! Each codec's plan emits cell-ids in a fixed grammar. These are the robust,
//! roundtrip-checked parsers — the Rust ports of the per-codec
//! `parse_config_name` functions in `zenmetrics/scripts/picker/configs/`, with
//! the sub-knob handling the ad-hoc parsers historically crashed on.
//!
//! Only the JXL lossy_dense grammar is ported here as the worked example (it's
//! the one with a published validation script, `test_lossy_dense_parse.py`);
//! the trait makes adding zenjpeg / zenwebp / zenavif grammars mechanical.

use alloc::string::{String, ToString};

use regex::Regex;
use std::sync::OnceLock;

use super::parse_config::{ConfigGrammar, ParsedConfig};

/// JXL **lossy_dense** cell-id grammar:
/// `vd-e{effort}_{strategy}_{knob}[-{flag}]`.
///
/// Mirrors the regex in `test_lossy_dense_parse.py`:
/// `^vd-e(\d+)_([a-z]+)_([^-]+)(?:-(.*))?$`. Axes:
///   - `effort`   (scalar) — the `e<N>` digit run;
///   - `strategy` (categorical) — a lowercase token;
///   - `knob`     (categorical) — anything up to an optional `-flag`;
///   - `flag`     (categorical) — the optional suffix, `"none"` when absent.
///
/// Baseline = `vd-e7_zen_def` (effort 7, strategy `zen`, knob `def`, no flag) —
/// the canonical config; `max_deviations = 1` matches the `--max-deviations`
/// budget the dense grid is built under.
#[derive(Clone, Copy, Debug, Default)]
pub struct JxlLossyDense;

fn lossy_dense_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^vd-e(\d+)_([a-z]+)_([^-]+)(?:-(.*))?$").expect("static regex"))
}

impl ConfigGrammar for JxlLossyDense {
    fn parse(&self, cell_id: &str) -> Option<ParsedConfig> {
        let caps = lossy_dense_re().captures(cell_id)?;
        let effort: f64 = caps.get(1)?.as_str().parse().ok()?;
        let strategy = caps.get(2)?.as_str();
        let knob = caps.get(3)?.as_str();
        let flag = caps.get(4).map(|m| m.as_str()).unwrap_or("none");
        Some(
            ParsedConfig::new()
                .with_scalar("effort", effort)
                .with_cat("strategy", strategy)
                .with_cat("knob", knob)
                .with_cat("flag", flag),
        )
    }

    fn recompose(&self, parsed: &ParsedConfig) -> Option<String> {
        let effort = *parsed.scalar.get("effort")?;
        let strategy = parsed.categorical.get("strategy")?;
        let knob = parsed.categorical.get("knob")?;
        let flag = parsed
            .categorical
            .get("flag")
            .map(String::as_str)
            .unwrap_or("none");
        let mut s = String::new();
        // effort is an integer level; format as `e{int}` to roundtrip the
        // source `e7` form (a fractional effort never appears in a cell-id).
        s.push_str("vd-e");
        s.push_str(&(effort as i64).to_string());
        s.push('_');
        s.push_str(strategy);
        s.push('_');
        s.push_str(knob);
        if flag != "none" {
            s.push('-');
            s.push_str(flag);
        }
        Some(s)
    }

    fn baseline(&self) -> ParsedConfig {
        ParsedConfig::new()
            .with_scalar("effort", 7.0)
            .with_cat("strategy", "zen")
            .with_cat("knob", "def")
            .with_cat("flag", "none")
    }

    fn max_deviations(&self) -> Option<usize> {
        Some(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_effort_strategy_knob_flag() {
        let g = JxlLossyDense;
        let p = g.parse("vd-e9_zen_k_ac_quant-g1").unwrap();
        assert_eq!(p.scalar.get("effort"), Some(&9.0));
        assert_eq!(
            p.categorical.get("strategy").map(String::as_str),
            Some("zen")
        );
        // knob is "anything up to the optional -flag" — captures underscores.
        assert_eq!(
            p.categorical.get("knob").map(String::as_str),
            Some("k_ac_quant")
        );
        assert_eq!(p.categorical.get("flag").map(String::as_str), Some("g1"));
        assert_eq!(g.recompose(&p).as_deref(), Some("vd-e9_zen_k_ac_quant-g1"));
    }

    #[test]
    fn rejects_non_matching() {
        let g = JxlLossyDense;
        assert!(g.parse("jp3_tr14_small_420").is_none()); // zenjpeg grammar, not jxl
        assert!(g.parse("vd-eX_zen_def").is_none()); // effort not numeric
    }
}
