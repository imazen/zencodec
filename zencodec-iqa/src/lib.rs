//! Map resolved zencodec outputs into the still-image zenpsychovisual-api contract.
//!
//! This companion owns no metric traits, kernels, conversion or video sessions.
//! The complete codec-independent contract is re-exported as [`crate::iqa`].
#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
/// Codec-independent still-image metric and result contracts.
pub use zenpsychovisual_api as iqa;
mod codec;
pub use codec::score_outputs;
