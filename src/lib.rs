//! Shared traits and types for zen* image codecs.
//!
//! This crate defines the common API surface that all zen* codecs implement:
//!
//! - [`EncoderConfig`] / [`EncodeJob`] / [`Encoder`] / [`FrameEncoder`] — encode traits
//! - [`DecoderConfig`] / [`DecodeJob`] / [`Decoder`] / [`FrameDecoder`] — decode traits
//! - [`EncodeOutput`] / [`DecodeOutput`] — unified output types
//! - [`PixelSlice`] / [`PixelSliceMut`] / [`PixelBuffer`] — format-erased pixel buffers
//! - [`PixelData`] — typed pixel buffer enum over `imgref::ImgVec`
//! - [`ImageInfo`] / [`ImageMetadata`] / [`Orientation`] — image metadata
//! - [`ImageFormat`] — format detection from magic bytes
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
mod info;
mod limits;
mod orientation;
mod output;
mod pixel;
mod pixel_format;
mod traits;

pub use buffer::{
    AlphaMode, BufferError, ChannelLayout, ChannelType, PixelBuffer, PixelDescriptor, PixelSlice,
    PixelSliceMut, TransferFunction,
};
pub use capabilities::CodecCapabilities;
pub use color::{ColorProfileSource, NamedProfile};
pub use format::ImageFormat;
pub use info::{
    Cicp, ContentLightLevel, DecodeCost, EncodeCost, ImageInfo, ImageMetadata, MasteringDisplay,
    OutputInfo,
};
pub use limits::{LimitExceeded, ResourceLimits};
pub use orientation::Orientation;
pub use output::{
    DecodeFrame, DecodeOutput, EncodeFrame, EncodeOutput, FrameBlend, FrameDisposal,
    TypedEncodeFrame,
};
pub use pixel::{GrayAlpha, PixelData};
pub use pixel_format::{ByteOrder, ColorModel, PixelFormat, Subsampling, YuvMatrix};
pub use traits::{
    DecodeJob, Decoder, DecoderConfig, EncodeJob, Encoder, EncoderConfig, FrameDecoder,
    FrameEncoder,
};

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
