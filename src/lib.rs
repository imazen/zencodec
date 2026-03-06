//! Shared traits and types for zen* image codecs.
//!
//! This crate defines the common API surface that all zen* codecs implement:
//!
//! - `PixelSlice` / `PixelSliceMut` / `PixelBuffer` — format-erased pixel buffers (from [`zenpixels`])
//! - [`ImageInfo`] / [`MetadataView`] / [`Orientation`] / [`OrientationHint`] — image metadata
//! - [`ImageFormat`] — format detection from magic bytes
//! - [`CodecCapabilities`] — capability flags for feature discovery
//! - [`UnsupportedOperation`] / [`HasUnsupportedOperation`] — standard unsupported operation reporting
//! - [`ResourceLimits`] — resource limit configuration
//! - [`EncoderConfig`] / [`EncodeJob`] — encode configuration and job
//! - Per-format encode traits: [`EncodeRgb8`], [`EncodeRgba8`], [`EncodeGray8`], etc.
//! - Per-format frame encode traits: [`FrameEncodeRgb8`], [`FrameEncodeRgba8`]
//! - [`DecoderConfig`] / [`DecodeJob`] — decode configuration and job
//! - [`Decode`] / [`FrameDecode`] — type-erased decode with preferred format negotiation
//! - [`DecodeRowSink`] — zero-copy row sink for streaming decode
//! - [`DecodeOutput`] — decode output with typed pixel data
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

mod capabilities;
mod color;
mod convert;
mod format;
mod gainmap;
mod info;
mod limits;
mod orientation;
mod output;
mod sink;
mod traits;

pub use capabilities::{CodecCapabilities, HasUnsupportedOperation, UnsupportedOperation};
pub use color::{ColorContext, ColorProfileSource, NamedProfile};
pub use convert::{
    AlphaPolicy, ConvertError, ConvertOptions, DepthPolicy, GrayExpand, LumaCoefficients,
    PixelSliceConvertExt,
};
pub use format::ImageFormat;
pub use gainmap::GainMapMetadata;
pub use info::{
    Cicp, ContentLightLevel, DecodeCost, EmbeddedMetadata, EncodeCost, ImageInfo, MasteringDisplay,
    Metadata, MetadataView, OutputInfo, SourceColor,
};
pub use limits::{LimitExceeded, ResourceLimits};
pub use orientation::{Orientation, OrientationHint};
pub use output::{
    DecodeFrame, DecodeOutput, EncodeFrame, EncodeOutput, FrameBlend, FrameDisposal,
    TypedEncodeFrame,
};
pub use sink::DecodeRowSink;
pub use traits::{
    Decode, DecodeJob, DecoderConfig, EncodeGray8, EncodeGray16, EncodeGrayF32, EncodeJob,
    EncodeRgb8, EncodeRgb16, EncodeRgbF16, EncodeRgbF32, EncodeRgba8, EncodeRgba16, EncodeRgbaF16,
    EncodeRgbaF32, Encoder, EncoderConfig, FrameDecode, FrameEncodeRgb8, FrameEncodeRgba8,
    FrameEncoder, StreamingDecode,
};

// Re-export PixelBufferConvertExt so codec crates get to_rgb8() etc. automatically.
pub use zenpixels_convert::ext::PixelBufferConvertExt;

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
