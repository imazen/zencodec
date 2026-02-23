//! Shared traits and types for zen* image codecs.
//!
//! This crate defines the common API surface that all zen* codecs implement:
//!
//! - [`EncoderConfig`] / [`EncodeJob`] / [`Encoder`] / [`FrameEncoder`] — encode traits
//! - [`DecoderConfig`] / [`DecodeJob`] / [`Decoder`] / [`FrameDecoder`] — decode traits
//! - [`DecodeRowSink`] — zero-copy row sink for streaming decode
//! - [`EncodeOutput`] / [`DecodeOutput`] — unified output types
//! - [`PixelSlice`] / [`PixelSliceMut`] / [`PixelBuffer`] — format-erased pixel buffers
//! - [`PixelData`] — typed pixel buffer enum over `imgref::ImgVec`
//! - [`ImageInfo`] / [`MetadataView`] / [`Orientation`] — image metadata
//! - [`ImageFormat`] — format detection from magic bytes
//! - [`CodecCapabilities`] — capability flags for feature discovery
//! - [`UnsupportedOperation`] / [`HasUnsupportedOperation`] — standard unsupported operation reporting
//! - [`ResourceLimits`] — resource limit configuration
//! - [`At`] / [`AtTrace`] / [`AtTraceable`] — error location tracking (via [`whereat`])
//!
//! Individual codecs (zenjpeg, zenwebp, zengif, zenavif) implement these traits
//! on their own config types. Format-specific methods live on the concrete types,
//! not on the traits.
//!
//! `zencodecs` provides multi-format dispatch and convenience entry points.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod buffer;
mod capabilities;
mod color;
mod format;
mod gainmap;
mod info;
mod limits;
mod orientation;
mod output;
mod pixel;
mod sink;
mod traits;

pub use buffer::{
    AlphaMode, BufferError, ChannelLayout, ChannelType, PixelBuffer, PixelDescriptor, PixelSlice,
    PixelSliceMut, TransferFunction,
};
pub use capabilities::{CodecCapabilities, HasUnsupportedOperation, UnsupportedOperation};
pub use color::{ColorContext, ColorProfileSource, NamedProfile, WorkingColorSpace};
pub use format::ImageFormat;
pub use gainmap::GainMapMetadata;
pub use info::{
    Cicp, ContentLightLevel, DecodeCost, EncodeCost, ImageInfo, MasteringDisplay, MetadataView,
    OutputInfo,
};
#[allow(deprecated)]
pub use info::ImageMetadata;
pub use limits::{LimitExceeded, ResourceLimits};
pub use orientation::Orientation;
pub use output::{
    DecodeFrame, DecodeOutput, EncodeFrame, EncodeOutput, FrameBlend, FrameDisposal,
    TypedEncodeFrame,
};
pub use pixel::{GrayAlpha, PixelData};
pub use sink::DecodeRowSink;
pub use traits::{
    DecodeJob, Decoder, DecoderConfig, EncodeJob, Encoder, EncoderConfig, FrameDecoder,
    FrameEncoder,
};

/// Clamp calibrated quality to the valid 0.0–100.0 range.
///
/// Use this in codec [`EncoderConfig::with_calibrated_quality()`] and
/// [`EncoderConfig::with_alpha_quality()`] implementations to validate
/// and clamp the input value. Fires a `debug_assert` on out-of-range
/// values so callers discover mistakes during development.
///
/// # Example
///
/// ```
/// use zencodec_types::clamp_quality;
///
/// assert_eq!(clamp_quality(85.0), 85.0);
/// assert_eq!(clamp_quality(0.0), 0.0);
/// assert_eq!(clamp_quality(100.0), 100.0);
/// ```
#[inline]
pub fn clamp_quality(q: f32) -> f32 {
    debug_assert!(
        (0.0..=100.0).contains(&q),
        "calibrated quality {q} outside 0.0–100.0 range"
    );
    q.clamp(0.0, 100.0)
}

// Re-exports for codec implementors and users.
pub use enough::{Stop, Unstoppable};
pub use imgref::{Img, ImgRef, ImgRefMut, ImgVec};
pub use rgb;
pub use rgb::alt::BGRA as Bgra;
pub use rgb::{Gray, Rgb, Rgba};

// Error location tracking re-exports.
//
// Codec error types use `whereat` for file:line tracking.
// The recommended pattern (codecs depend on `thiserror` directly):
//
// ```rust,ignore
// use zencodec_types::{At, ResultAtExt};
//
// #[derive(Debug, thiserror::Error)]
// pub enum MyCodecError {
//     #[error("invalid header")]
//     InvalidHeader,
// }
//
// // In trait impl:
// type Error = At<MyCodecError>;
//
// // .at() captures file:line on error:
// fn decode(&self, data: &[u8]) -> Result<..., At<MyCodecError>> {
//     parse_header(data).at()?;
//     Ok(...)
// }
// ```
pub use whereat;
pub use whereat::{At, AtTrace, AtTraceable, ErrorAtExt, ResultAtExt};
