//! Media contracts for zen codecs: what a video (and later audio) codec, a
//! container and a pipeline say to each other.
//!
//! `no_std + alloc`, no I/O, no async, no codec implementations. The image
//! contract in [`zencodec`] stays as it is; this crate adds the pieces video
//! needs and reuses the image side's shapes wherever they fit:
//!
//! - [`packet`]: compressed access units with optional timestamps, container
//!   random-access evidence and configuration epochs.
//! - [`track`]: codec identity separate from the container, validated track
//!   descriptions.
//! - [`frame`]: decoded frames as owned, refcounted handles over backend
//!   pictures, with planar views and an immutable per-frame interpretation.
//! - [`session`]: codec sessions with explicit backpressure (`send` /
//!   `receive`), drain at end of input, and discard-only reset.
//! - [`container`]: a sans-I/O [`Demuxer`](container::Demuxer) speaking the
//!   image decoders' `Input`/`InputRequest` dialogue (so every `zencodec-io`
//!   driver serves it), and a [`Muxer`](container::Muxer) whose bytes are
//!   pulled like a streaming image encoder's.
//! - [`adapt`]: the animation traits as a layer over demuxer + decoder and
//!   encoder + muxer.
//!
//! Time comes from [`zencodec::time`]. Design and rationale:
//! `docs/media-design.md` in the repository; the rules every implementation
//! follows: `docs/media-contract.md`.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod adapt;
pub mod container;
pub mod frame;
pub mod limits;
/// Checked synchronous output drivers (requires `std`).
#[cfg(feature = "std")]
pub mod output;
pub mod packet;
pub mod policy;
pub mod session;
/// Owned finite video encoding with admitted compressed staging (requires `std`).
#[cfg(feature = "std")]
pub mod staged;
/// Explicit memory/disk compressed packet staging (requires `std`).
#[cfg(feature = "std")]
pub mod staging;
pub mod timeline;
pub mod track;
