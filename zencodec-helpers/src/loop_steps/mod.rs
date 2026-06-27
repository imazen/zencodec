//! Offline picker-training-loop glue (the `loop-tools` feature).
//!
//! These modules factor out the reusable steps that were ad-hoc Python in
//! `zenmetrics/scripts/picker/` so every codec's loop runs the *identical*,
//! high-quality shape:
//!
//! | step | module | replaces |
//! |------|--------|----------|
//! | omni-merge across box sidecars | [`omni_merge`] | `omni_to_pareto.py` |
//! | `variant_name` + `size_class` | [`variant`] | `variant_of` / `size_class` |
//! | `feat_<name>` select + `log1p` | [`feature_prep`] | `picker_config_common.py` |
//! | `parse_config_name` robustness | [`parse_config`] + [`grammars`] | per-codec config parsers |
//! | train/val/test origin split | [`origin_split`] | `origin_split.py` |
//! | top-K-verify oracle gap | [`topk_eval`] | `evaluate_topk_verify` |
//!
//! Everything here is `std`. The runtime [`pick_top_k_verify`] lives outside
//! this module (the `topk-verify` feature) and does not depend on these.
//!
//! [`pick_top_k_verify`]: crate::topk_verify::pick_top_k_verify

pub mod feature_prep;
pub mod grammars;
pub mod omni_merge;
pub mod origin_split;
pub mod parse_config;
pub mod topk_eval;
pub mod variant;
