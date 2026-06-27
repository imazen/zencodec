//! # zencodec-helpers
//!
//! A generic runtime **top-K-verify** picker helper for the zen\* image codecs
//! (zenjpeg / zenwebp / zenjxl / zenavif / …).
//!
//! ## The gap this fills
//!
//! An audit found that a per-codec picker's *top-K* path is unreachable at
//! runtime — every wired picker (zenwebp, zenjpeg) takes a single argmin and
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
//! ```
//!
//! ## Dependency note — zenpredict DEFAULT API only
//!
//! This crate uses zenpredict's **default** API only — no gated feature
//! (`topk` / `advanced`). It calls [`zenpredict::Predictor::predict`] /
//! `predict_transformed` (the per-cell predicted-cost forward pass) and reuses
//! the default decision-math primitives [`zenpredict::AllowedMask`],
//! [`zenpredict::ScoreTransform`], [`zenpredict::ArgminOffsets`]. The masked
//! top-K *selection* is implemented **here** ([`topk_verify::select_top_k`])
//! over that output slice, with the same NaN / tie-break / mask-length contract
//! as zenpredict's `argmin_masked` — so nothing needs adding to zenpredict and
//! the proven ≤1% path lives entirely in the consumer.
//!
//! zenpredict **0.2** is unpublished today (crates.io still ships 0.1.0 /
//! v2-only), so the crate is excluded from the zencodec workspace and pins the
//! sibling `../zenanalyze/zenpredict` (at `main`) via a build-local
//! `[patch.crates-io]`. Bump to the published `>=0.2` once it lands — the dep
//! stays default-features-only either way.

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
