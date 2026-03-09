//! Shared traits and types for zen* image codecs.
//!
//! This crate defines the common API surface that all zen* codecs implement.
//!
//! # Module organization
//!
//! - [`encode`] — encoder traits, dyn dispatch, output types
//! - [`decode`] — decoder traits, streaming/animation decode, dyn dispatch, output types
//! - Root — shared types used by both encode and decode paths
//!
//! # Shared types (root)
//!
//! - [`ImageFormat`] — format detection from magic bytes
//! - [`ImageInfo`] / [`Metadata`] / [`Orientation`] / [`OrientationHint`] — image metadata
//! - [`ResourceLimits`] / [`ThreadingPolicy`] — resource limit and threading configuration
//! - [`UnsupportedOperation`] / [`CodecErrorExt`] — standard unsupported operation reporting and error chain inspection
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

whereat::define_at_crate_info!();

mod capabilities;
mod cost;
mod detect;
mod error;
mod format;
mod gainmap;
mod info;
mod limits;
mod metadata;
mod negotiate;
mod orientation;
mod output;
mod policy;
mod sink;
mod traits;

// =========================================================================
// Public root: shared types used by both encode and decode
// =========================================================================

// Color profile types re-exported from zenpixels for convenience.
// These appear in our public API (ImageInfo, SourceColor, etc.) so callers
// don't need to add zenpixels as a direct dependency just for these.
pub use format::{ImageFormat, ImageFormatDefinition, ImageFormatRegistry};
pub use gainmap::GainMapMetadata;
pub use info::{Cicp, ContentLightLevel, ImageInfo, MasteringDisplay, Resolution, ResolutionUnit};
pub use limits::{LimitExceeded, ResourceLimits, ThreadingPolicy};
pub use metadata::Metadata;
pub use orientation::{Orientation, OrientationHint};
pub use output::{FullFrame, OwnedFullFrame};
pub use zenpixels::{ColorContext, ColorProfileSource, NamedProfile};

pub use capabilities::UnsupportedOperation;
pub use detect::SourceEncodingDetails;
pub use error::{CodecErrorExt, find_cause};
pub use traits::Unsupported;

// =========================================================================
// Crate-level re-exports (qualified access, not individual types)
// =========================================================================
//
pub use enough;
pub use enough::Unstoppable;

// =========================================================================
// pub(crate) re-exports — keep internal `use crate::Foo` paths working
// for items that moved out of the public root into sub-modules.
// =========================================================================

pub(crate) use capabilities::{DecodeCapabilities, EncodeCapabilities};
pub(crate) use cost::{DecodeCost, EncodeCost, OutputInfo};
pub(crate) use output::{DecodeOutput, EncodeOutput};
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
///                                  └→ FullFrameEnc (implements FullFrameEncoder)
/// ```
///
/// # Object-safe dyn dispatch
///
/// ```text
/// DynEncoderConfig → DynEncodeJob → DynEncoder / DynFullFrameEncoder
/// ```
///
/// Codec implementors implement the generic traits. Dispatch callers
/// use the `Dyn*` variants for codec-agnostic operation.
pub mod encode {
    // Traits — config, job, execution
    pub use crate::traits::{EncodeJob, Encoder, EncoderConfig, FullFrameEncoder};

    // Object-safe dyn dispatch
    pub use crate::traits::{
        BoxedError, DynEncodeJob, DynEncoder, DynEncoderConfig, DynFullFrameEncoder,
    };

    // Types
    pub use crate::capabilities::EncodeCapabilities;
    pub use crate::cost::EncodeCost;
    pub use crate::negotiate::best_encode_format;
    pub use crate::output::EncodeOutput;
    pub use crate::policy::EncodePolicy;
}

/// Decode traits, types, and configuration.
///
/// # Trait hierarchy
///
/// ```text
///                                  ┌→ Dec (implements Decode)
/// DecoderConfig → DecodeJob<'a> ──┤→ StreamDec (implements StreamingDecode)
///                                  └→ FullFrameDec (implements FullFrameDecoder)
/// ```
///
/// # Object-safe dyn dispatch
///
/// ```text
/// DynDecoderConfig → DynDecodeJob → DynDecoder / DynFullFrameDecoder / DynStreamingDecoder
/// ```
///
/// Codec implementors implement the generic traits. Dispatch callers
/// use the `Dyn*` variants for codec-agnostic operation.
pub mod decode {
    // Traits — config, job, execution
    pub use crate::traits::{
        Decode, DecodeJob, DecoderConfig, FullFrameDecoder, StreamingDecode,
        push_decoder_via_full_decode, render_frame_to_sink_via_copy,
    };

    // Object-safe dyn dispatch
    pub use crate::traits::{
        BoxedError, DynDecodeJob, DynDecoder, DynDecoderConfig, DynFullFrameDecoder,
        DynStreamingDecoder,
    };

    // Types
    pub use crate::capabilities::DecodeCapabilities;
    pub use crate::cost::{DecodeCost, OutputInfo};
    pub use crate::output::{DecodeOutput, FullFrame, OwnedFullFrame};
    pub use crate::policy::DecodePolicy;
    pub use crate::sink::{DecodeRowSink, SinkError};

    pub use crate::negotiate::{is_format_available, negotiate_pixel_format};

    // Source encoding detection
    pub use crate::detect::SourceEncodingDetails;

    // Shared types re-exported for convenience (commonly needed alongside decode)
    pub use crate::info::{EmbeddedMetadata, SourceColor};
}
