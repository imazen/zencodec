//! # zencodec-helpers
//!
//! Shared infrastructure that makes high-quality, *consistent* per-codec picker
//! **loops** and the picker **runtime** easy across the zen* image codecs
//! (zenjpeg / zenwebp / zenjxl / zenavif / …). Two independent halves:
//!
//! ## 1. Runtime: top-K-verify picker ([`topk_verify`], `topk-verify` feature)
//!
//! The key gap an audit found: a per-codec picker's *top-K* path is unreachable
//! at runtime — every wired picker (zenwebp, zenjpeg) takes a single argmin and
//! uses the picked config blindly, leaving the residual oracle gap (~2.4% on
//! the JXL-lossy picker) on the floor. The proven fix — *narrow by content,
//! finalize by an RD check* — reached ~0.48% (≤1% MET) at K=3 in offline
//! evaluation but had no runtime home.
//!
//! [`topk_verify::pick_top_k_verify`] is that home: generic over a codec's
//! `encode(cell) -> bytes` and `score(cell) -> quality` closures, it ranks the
//! picker's predicted-cheapest cells, **encodes + scores** the K cheapest, and
//! returns the min-bytes cell meeting the quality target. [`pick_top_3_verify`]
//! is the proven K=3 specialization.
//!
//! ```no_run
//! # #[cfg(feature = "topk-verify")] {
//! use zencodec_helpers::topk_verify::{pick_top_3_verify, VerifyConfig, VerifyOutcome};
//! use zenpredict::AllowedMask;
//!
//! # let predictions: Vec<f32> = vec![0.0; 36];   // picker output (log-bytes per cell)
//! # let mask_data = vec![true; 36];
//! # fn encode_cell(_c: usize) -> u64 { 1000 }
//! # fn score_cell(_c: usize) -> f32 { 90.0 }
//! let mask = AllowedMask::new(&mask_data);
//! let cfg = VerifyConfig::log_bytes(predictions.len(), /* target */ 80.0);
//! let outcome = pick_top_3_verify(
//!     &predictions, &mask, &cfg,
//!     |cell| encode_cell(cell),   // actually encode candidate `cell`
//!     |cell| score_cell(cell),    // score that encode's quality
//! ).unwrap();
//! if let VerifyOutcome::MetTarget(best) = outcome {
//!     // re-emit / cache `best.cell`'s bitstream — it has the fewest bytes ≥ target.
//!     let _ = best;
//! }
//! # }
//! ```
//!
//! ## 2. Offline: training-loop glue ([`loop_tools`], `loop-tools` feature)
//!
//! The reusable Rust port of the per-codec picker-loop steps that were ad-hoc
//! Python in `zenmetrics/scripts/picker/`: omni-sidecar merge across boxes,
//! `variant_name`/`feat_*` prep, `parse_config_name` robustness, the canonical
//! train/val/test origin split, and the top-K-verify oracle-gap evaluation that
//! *chooses* the runtime K. Powers the `picker-loop` CLI.
//!
//! ## Dependency note (read before you depend on `topk-verify`)
//!
//! The runtime half consumes zenpredict's masked top-K argmin
//! (`argmin_masked_top_k::<K>`), which lives behind zenpredict's **`advanced`**
//! feature — present only in the in-tree **0.2.0** (crates.io still ships
//! 0.1.0 / v2-only), and explicitly "NOT YET STABILIZED" there. A concurrent
//! zenpredict PR is stabilizing exactly this API. This crate is designed
//! against the masked-top-K shape (K output indices, ascending best-first) and,
//! for local build + validation, the workspace root pins the sibling
//! `../zenanalyze/zenpredict` via `[patch.crates-io]`. Bump to the published
//! `>=0.2` once it lands. The `loop-tools` half has no such dependency.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(feature = "topk-verify")]
pub mod topk_verify;

#[cfg(feature = "topk-verify")]
pub use topk_verify::{
    Measured, QualityDirection, VerifyConfig, VerifyError, VerifyOutcome, pick_top_3_verify,
    pick_top_k_verify,
};

#[cfg(feature = "loop-tools")]
#[path = "loop_steps/mod.rs"]
pub mod loop_tools;
