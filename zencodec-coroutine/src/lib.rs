//! Write a zencodec incremental decoder as straight-line `async` code.
//!
//! A resumable decoder ([`IncrementalDecode`]) must suspend whenever it
//! lacks bytes and resume where it left off. Writing that by hand means a
//! state enum per parser stage. This crate repurposes `async`/`await` as a
//! compile-time coroutine generator instead: the codec author writes
//! ordinary sequential code against a [`CoroutineContext`], and `rustc`
//! synthesises the state machine. [`CoroutineDecoder`] polls it and
//! presents the standard synchronous [`IncrementalDecode`] contract.
//!
//! No runtime, no executor, no threads, no `block_on`: `.await` here means
//! "yield to the driver until bytes arrive or the sink has taken the rows".
//! The crate is `no_std + alloc` and `forbid(unsafe_code)`.
//!
//! ```ignore
//! let dec = CoroutineDecoder::new(|mut ctx: CoroutineContext| async move {
//!     let mut hdr = [0u8; 16];
//!     ctx.read_exact_at(0, &mut hdr).await?;
//!     ctx.set_info(info_from(&hdr));
//!     ctx.set_output_descriptor(PixelDescriptor::RGBA8_SRGB);
//!     ctx.announce(MetadataUpdate::HeaderReady).await?;
//!     for strip in strips {
//!         let mut buf = vec![0u8; strip.len];
//!         ctx.read_exact_at(strip.offset, &mut buf).await?;
//!         ctx.write_rows(BatchMeta::final_rows(strip.y), &pixels(&buf)).await?;
//!     }
//!     Ok(OutputInfo::full_decode(w, h, PixelDescriptor::RGBA8_SRGB))
//! });
//! ```
//!
//! # Cost model
//!
//! Each `.await` that suspends costs one `step` round trip through the
//! driver (no thread, no allocation). The driver owns the input; the
//! decoder copies into its store only the offered bytes an outstanding read
//! needs, under a byte cap, and releases them once read. Gap bytes between
//! requested ranges are consumed but never stored, and a single read larger
//! than the cap is an error, not a stall. Rows are copied once more, from
//! the codec's strip into the sink. A hand-written decoder can avoid both
//! copies by decoding from the driver's input straight into the sink, which
//! is why bulk pixel paths are better hand-written and container walks
//! suit this crate.

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;

mod admitted_input;
mod context;
mod decoder;
mod input;

pub use context::{CoroutineContext, CoroutineError};
pub use decoder::{BoxFuture, BoxedCoroutineDecoder, CoroutineDecoder, DEFAULT_INPUT_CAP};

#[doc(no_inline)]
pub use zencodec::decode::IncrementalDecode;
