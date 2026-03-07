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
//! The [`enough`] crate is re-exported for cooperative cancellation
//! (`enough::Stop`).
//!
//! ```rust,ignore
//! use enough::Stop;
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
pub use orientation::{Orientation, OrientationHint};
pub use output::{FrameBlend, FrameDisposal};

pub use capabilities::{HasUnsupportedOperation, UnsupportedOperation};

// =========================================================================
// Crate-level re-exports (qualified access, not individual types)
// =========================================================================
//
pub use enough;

// =========================================================================
// pub(crate) re-exports — keep internal `use crate::Foo` paths working
// for items that moved out of the public root into sub-modules.
// =========================================================================

pub(crate) use capabilities::{DecodeCapabilities, EncodeCapabilities};
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
///                                  ┌→ Enc (implements Encoder)
/// EncoderConfig → EncodeJob<'a> ──┤
///                                  └→ FrameEnc (implements FrameEncoder)
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
    pub use crate::traits::{EncodeJob, Encoder, EncoderConfig, FrameEncoder};

    // Object-safe dyn dispatch
    pub use crate::traits::{
        BoxedError, DynEncodeJob, DynEncoder, DynEncoderConfig, DynFrameEncoder,
    };

    // Types
    pub use crate::capabilities::EncodeCapabilities;
    pub use crate::info::EncodeCost;
    pub use crate::negotiate::best_encode_format;
    pub use crate::output::{EncodeFrame, EncodeOutput};
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

    pub use crate::negotiate::{is_format_available, negotiate_pixel_format};

    // Shared types re-exported for convenience (commonly needed alongside decode)
    pub use crate::info::{EmbeddedMetadata, SourceColor};
}

