//! Drivers that run a zencodec decoder over real input.
//!
//! zencodec defines two decoder shapes that do no I/O of their own:
//!
//! - [`IncrementalDecode`](zencodec::decode::IncrementalDecode): resumable,
//!   driven by whoever holds the bytes (`feed` / `step`).
//! - [`DecodeJob::push_decoder_from_source`](zencodec::decode::DecodeJob::push_decoder_from_source):
//!   pull-native, reading a [`DynReadAt`](zencodec::source::DynReadAt).
//!
//! This crate supplies the loops and adapters that connect either shape to
//! a slice, a random-access source, a sequential stream, or an async
//! runtime, so neither codecs nor applications write them:
//!
//! | input | resumable decoder | pull-native decoder |
//! |---|---|---|
//! | slice | [`drive_from_slice`] | [`SliceReadAt`](zencodec::source::SliceReadAt) |
//! | `ReadAt` (file, object store, FFI callback) | [`drive_from_read_at`] | direct |
//! | `std::io::Read` stream | [`pump_read_to_sink`] | [`BufferedReadAt`] |
//! | async runtime | [`tokio_io::decode_async`] | [`tokio_io::pull_decode_on_thread`] |
//!
//! Cancellation and progress share one handle: every driver takes an
//! `Option<&dyn how_far::Pulse>` (`Pulse: Stop`), checks it each step,
//! tells it the row total once the header is known and advances it per
//! strip. A caller with only a `Stop` passes `how_far::StopOnly::borrowed`.
//!
//! The crate is `no_std + alloc` by default. The `std` feature adds the
//! stream pump, file source and buffered stream adapter; `tokio` adds the
//! dedicated-thread bridge (it uses `tokio::sync` channels only and never
//! starts a runtime).

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod drive;
mod encoder_sink;
mod sinks;
mod window;

#[cfg(feature = "std")]
mod buffered;
#[cfg(feature = "std")]
mod file;
#[cfg(feature = "std")]
mod pump;
#[cfg(feature = "tokio")]
pub mod tokio_io;

pub use drive::{
    DriveError, DriveOptions, DriveStats, coalesce_byte_ranges, coalesce_into, drive_from_read_at,
    drive_from_slice,
};
pub use encoder_sink::{EncoderSink, EncoderSinkError};
pub use sinks::CollectSink;
pub use window::ProtocolViolation;

#[cfg(feature = "std")]
pub use buffered::{BufferPolicy, BufferedReadAt};
#[cfg(feature = "std")]
pub use file::FileReadAt;
#[cfg(feature = "std")]
pub use pump::{PumpError, pump_read_to_sink};
