//! The zen codec contract, reimagined. A spike, not a release.
//!
//! This crate is only the contract between a codec and whatever drives it.
//! Registries, application conveniences, I/O drivers and metadata parsing
//! live elsewhere.
//!
//! # The whole contract
//!
//! | | decode | encode |
//! |---|---|---|
//! | A codec, shareable and erased | [`decode::DecodeCodec`] | [`encode::EncodeCodec`] |
//! | The request, as plain data | [`decode::DecodeSpec`] | [`encode::EncodeSpec`] |
//! | What will happen | [`decode::DecodePlan`] | [`encode::EncodePlan`] |
//! | The work, as a state machine | [`decode::Decoder`] | [`encode::Encoder`] |
//! | Where the product goes | [`decode::Sink`] | [`encode::OutputBuffer`] |
//!
//! Every method returns [`Result`], whose error carries a [`ErrorKind`] to
//! decide on and the codec's own error to inspect. Every trait is object
//! safe; there is no second "dyn" family. Codec authors with nothing to
//! stream start in [`simple`].
//!
//! Pixels are zenpixels types throughout and are not re-exported: import
//! `PixelSlice`, `PixelBuffer`, `PixelDescriptor`, `PlanarSlice` and
//! `ColorContext` from `zenpixels`.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

whereat::define_at_crate_info!(path = "reimagined/zencodec/");

mod bytes;
pub mod decode;
pub mod encode;
mod error;
mod format;
mod info;
mod limits;
mod quality;
mod raster;
pub mod simple;

pub use error::{
    CodecError, Error, ErrorKind, LimitKind, NativeError, Result, StopReason, Unsupported, check,
    error, error_msg, find_cause, unsupported,
};
pub use format::{FormatInfo, ImageFormat};
pub use info::{ImageInfo, Metadata, MetadataKinds, Sequence};
pub use limits::{
    Bound, LimitExceeded, ResourceEstimate, ResourceLimits, ResourceTargets, Threads,
};
pub use quality::{Demand, Effort, Fidelity, LossyTarget};
pub use raster::{FrameDuration, FrameHeader, Layout, Pass, Raster, copy_rows};

pub use bytes::{ByteRange, Input, InputRequest, OutputAccess};

/// Progress and cancellation. Steps take `Option<&dyn Pulse>`: `None` disables
/// both without a no-op observer. [`SharedPulse`] is the owned handle for work
/// that leaves the caller's thread; [`NoPulse`] remains available explicitly.
pub use how_far::{NoPulse, Pulse, SharedPulse, Stop};
/// The location-carrying wrapper behind [`Error`].
pub use whereat::At;
