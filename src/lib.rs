//! Shared traits and types for zen* image codecs.
//!
//! This crate defines the common API surface that all zen* codecs implement:
//!
//! - [`PixelSlice`] / [`PixelSliceMut`] / [`PixelBuffer`] — format-erased pixel buffers
//! - [`ImageInfo`] / [`MetadataView`] / [`Orientation`] / [`OrientationHint`] — image metadata
//! - [`ImageFormat`] — format detection from magic bytes
//! - [`CodecCapabilities`] — capability flags for feature discovery
//! - [`UnsupportedOperation`] / [`HasUnsupportedOperation`] — standard unsupported operation reporting
//! - [`ResourceLimits`] — resource limit configuration
//! - [`At`] / [`AtTrace`] / [`AtTraceable`] — error location tracking (via [`whereat`])
//!
//! With the `codec` feature (default):
//!
//! - [`EncoderConfig`] / [`EncodeJob`] — encode configuration and job
//! - Per-format encode traits: [`EncodeRgb8`], [`EncodeRgba8`], [`EncodeGray8`], etc.
//! - Per-format frame encode traits: [`FrameEncodeRgb8`], [`FrameEncodeRgba8`]
//! - [`DecoderConfig`] / [`DecodeJob`] — decode configuration and job
//! - [`Decode`] / [`FrameDecode`] — type-erased decode with preferred format negotiation
//! - [`DecodeRowSink`] — zero-copy row sink for streaming decode
//! - [`DecodeOutput`] — decode output with typed pixel data
//!
//! Individual codecs (zenjpeg, zenwebp, zengif, zenavif) implement these traits
//! on their own config types. Format-specific methods live on the concrete types,
//! not on the traits.
//!
//! `zencodecs` provides multi-format dispatch and convenience entry points.

#![no_std]
// deny rather than forbid: bytemuck Pod/Zeroable impls for GrayAlpha<T>
// require `unsafe impl` since bytemuck derives don't support generics.
// The impls are trivially correct (#[repr(C)], two Pod fields, no padding).
#![deny(unsafe_code)]

extern crate alloc;

// Always-available modules (no external deps beyond whereat).
mod buffer;
mod capabilities;
mod color;
mod format;
mod gainmap;
mod info;
mod limits;
mod orientation;
mod output;

// Modules gated behind the `codec` feature (require rgb, imgref, enough).
#[cfg(feature = "codec")]
mod pixel;
#[cfg(feature = "codec")]
mod sink;
#[cfg(feature = "codec")]
mod traits;

// --- Always-available exports ---

pub use buffer::{
    AlphaMode, Bgrx, BufferError, ChannelLayout, ChannelType, ColorPrimaries, Pixel, PixelBuffer,
    PixelDescriptor, PixelFormat, PixelSlice, PixelSliceMut, Rgbx, SignalRange, TransferFunction,
};
pub use capabilities::{CodecCapabilities, HasUnsupportedOperation, UnsupportedOperation};
pub use color::{ColorContext, ColorProfileSource, NamedProfile, WorkingColorSpace};
pub use format::ImageFormat;
pub use gainmap::GainMapMetadata;
pub use info::{
    Cicp, ContentLightLevel, DecodeCost, EmbeddedMetadata, EncodeCost, ImageInfo, MasteringDisplay,
    Metadata, MetadataView, OutputInfo, SourceColor,
};
pub use limits::{LimitExceeded, ResourceLimits};
pub use orientation::{Orientation, OrientationHint};
pub use output::{EncodeFrame, EncodeOutput, FrameBlend, FrameDisposal};

// --- Codec-feature-gated exports ---

#[cfg(feature = "codec")]
pub use output::{DecodeFrame, DecodeOutput, TypedEncodeFrame};
#[cfg(feature = "codec")]
#[allow(deprecated)]
pub use pixel::{GrayAlpha, PixelData};
#[cfg(feature = "codec")]
pub use sink::DecodeRowSink;
#[cfg(feature = "codec")]
pub use traits::{
    Decode, DecodeJob, DecoderConfig, EncodeGray8, EncodeGray16, EncodeGrayF32, EncodeJob,
    EncodeRgb8, EncodeRgb16, EncodeRgbF16, EncodeRgbF32, EncodeRgba8, EncodeRgba16, EncodeRgbaF16,
    EncodeRgbaF32, EncoderConfig, FrameDecode, FrameEncodeRgb8, FrameEncodeRgba8,
};

// Re-exports for codec implementors and users (codec feature).
#[cfg(feature = "codec")]
pub use enough::{Stop, Unstoppable};
#[cfg(feature = "codec")]
pub use imgref::{Img, ImgRef, ImgRefMut, ImgVec};
#[cfg(feature = "codec")]
pub use rgb;
#[cfg(feature = "codec")]
pub use rgb::alt::BGRA as Bgra;
#[cfg(feature = "codec")]
pub use rgb::{Gray, Rgb, Rgba};

// Error location tracking re-exports (always available).
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
