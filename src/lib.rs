//! Shared traits and types for zen* image codecs.
//!
//! This crate defines the common API surface that all zen* codecs implement.
//!
//! # Module organization
//!
//! - [`encode`] — encoder traits, per-format encode traits, dyn dispatch, output types
//! - [`decode`] — decoder traits, streaming/animation decode, dyn dispatch, output types
//! - Root — shared types used by both encode and decode paths
//!
//! # Shared types (root)
//!
//! - [`ImageFormat`] — format detection from magic bytes
//! - [`ImageInfo`] / [`MetadataView`] / [`Orientation`] / [`OrientationHint`] — image metadata
//! - [`ResourceLimits`] / [`ThreadingPolicy`] — resource limit and threading configuration
//! - [`UnsupportedOperation`] / [`HasUnsupportedOperation`] — standard unsupported operation reporting
//!
//! # Re-exported crates
//!
//! Pixel types, cancellation, and error tracking crates are re-exported at
//! crate level for qualified access: [`rgb`], [`imgref`], [`enough`], [`whereat`].
//!
//! ```rust,ignore
//! use rgb::Rgba;
//! use imgref::ImgVec;
//! use enough::Stop;
//! use whereat::At;
//! ```
//!
//! Individual codecs (zenjpeg, zenwebp, zengif, zenavif) implement the
//! [`encode`] and [`decode`] traits on their own config types.
//! Format-specific methods live on the concrete types, not on the traits.
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
mod negotiate;
mod orientation;
mod output;
mod policy;
mod sink;
mod traits;

// =========================================================================
// Public root: shared types used by both encode and decode
// =========================================================================

pub use color::{ColorContext, ColorProfileSource, NamedProfile};
pub use convert::{
    AlphaPolicy, ConvertError, ConvertOptions, DepthPolicy, GrayExpand, LumaCoefficients,
    PixelSliceConvertExt,
};
pub use format::ImageFormat;
pub use gainmap::GainMapMetadata;
pub use info::{Cicp, ContentLightLevel, ImageInfo, MasteringDisplay, Metadata, MetadataView};
pub use limits::{LimitExceeded, ResourceLimits, ThreadingPolicy};
pub use negotiate::{best_encode_format, is_format_available, negotiate_pixel_format};
pub use orientation::{Orientation, OrientationHint};
pub use output::{FrameBlend, FrameDisposal};

pub use capabilities::{HasUnsupportedOperation, UnsupportedOperation};

// Re-export PixelBufferConvertExt so codec crates get to_rgb8() etc. automatically.
pub use zenpixels_convert::ext::PixelBufferConvertExt;

// =========================================================================
// Crate-level re-exports (qualified access, not individual types)
// =========================================================================
//
// Use `rgb::Rgba`, `imgref::ImgVec`, `enough::Stop`, `whereat::At`.

pub use enough;
pub use imgref;
pub use rgb;
pub use whereat;

// =========================================================================
// pub(crate) re-exports — keep internal `use crate::Foo` paths working
// for items that moved out of the public root into sub-modules.
// =========================================================================

pub(crate) use capabilities::{DecodeCapabilities, EncodeCapabilities};
pub(crate) use enough::Stop;
pub(crate) use info::{DecodeCost, EncodeCost, OutputInfo};
pub(crate) use output::{DecodeFrame, DecodeOutput, EncodeFrame, EncodeOutput};
pub(crate) use policy::{DecodePolicy, EncodePolicy};
pub(crate) use sink::DecodeRowSink;

// =========================================================================
// Public sub-modules
// =========================================================================

/// Encode traits, types, and configuration.
///
/// # Trait hierarchy
///
/// ```text
///                                  ┌→ Enc (implements Encoder and/or EncodeRgb8, EncodeRgba8, ...)
/// EncoderConfig → EncodeJob<'a> ──┤
///                                  └→ FrameEnc (implements FrameEncoder and/or FrameEncodeRgba8, ...)
/// ```
///
/// # Object-safe dyn dispatch
///
/// ```text
/// DynEncoderConfig → DynEncodeJob → DynEncoder / DynFrameEncoder
/// ```
///
/// Codec implementors implement the generic traits. Dispatch callers
/// use the `Dyn*` variants for codec-agnostic operation.
pub mod encode {
    // Traits — config, job, execution
    pub use crate::traits::{
        EncodeJob, Encoder, EncoderConfig, FrameEncoder,
    };

    // Per-format typed encode traits
    pub use crate::traits::{
        EncodeGray8, EncodeGray16, EncodeGrayF32, EncodeRgb8, EncodeRgb16, EncodeRgbF16,
        EncodeRgbF32, EncodeRgba8, EncodeRgba16, EncodeRgbaF16, EncodeRgbaF32,
    };

    // Per-format frame encode traits (animation)
    pub use crate::traits::{FrameEncodeRgb8, FrameEncodeRgba8};

    // Object-safe dyn dispatch
    pub use crate::traits::{
        BoxedError, DynEncodeJob, DynEncoder, DynEncoderConfig, DynFrameEncoder,
    };

    // Types
    pub use crate::capabilities::EncodeCapabilities;
    pub use crate::info::EncodeCost;
    pub use crate::output::{EncodeFrame, EncodeOutput, TypedEncodeFrame};
    pub use crate::policy::EncodePolicy;
}

/// Decode traits, types, and configuration.
///
/// # Trait hierarchy
///
/// ```text
///                                  ┌→ Dec (implements Decode)
/// DecoderConfig → DecodeJob<'a> ──┤→ StreamDec (implements StreamingDecode)
///                                  └→ FrameDec (implements FrameDecode)
/// ```
///
/// # Object-safe dyn dispatch
///
/// ```text
/// DynDecoderConfig → DynDecodeJob → DynDecoder / DynFrameDecoder / DynStreamingDecoder
/// ```
///
/// Codec implementors implement the generic traits. Dispatch callers
/// use the `Dyn*` variants for codec-agnostic operation.
pub mod decode {
    // Traits — config, job, execution
    pub use crate::traits::{
        Decode, DecodeJob, DecoderConfig, FrameDecode, StreamingDecode,
    };

    // Object-safe dyn dispatch
    pub use crate::traits::{
        BoxedError, DynDecodeJob, DynDecoder, DynDecoderConfig, DynFrameDecoder,
        DynStreamingDecoder,
    };

    // Types
    pub use crate::capabilities::DecodeCapabilities;
    pub use crate::info::{DecodeCost, OutputInfo};
    pub use crate::output::{DecodeFrame, DecodeOutput};
    pub use crate::policy::DecodePolicy;
    pub use crate::sink::DecodeRowSink;

    // Shared types re-exported for convenience (commonly needed alongside decode)
    pub use crate::info::{EmbeddedMetadata, SourceColor};
}

// Error location tracking re-exports.
//
// Codec error types use `whereat` for file:line tracking.
// The recommended pattern (codecs depend on `thiserror` directly):
//
// ```rust,ignore
// use whereat::{At, ResultAtExt};
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
