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
mod extensions;
mod format;
/// Cross-codec gain map types (ISO 21496-1).
pub mod gainmap;
/// Codec implementation helpers (not consumer API).
pub mod helpers;
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

pub use extensions::Extensions;
pub use format::{ImageFormat, ImageFormatDefinition, ImageFormatRegistry};
pub use gainmap::{GainMapChannel, GainMapDirection, GainMapInfo, GainMapParams, GainMapPresence};
pub use info::{
    Cicp, ContentLightLevel, ImageInfo, ImageSequence, MasteringDisplay, Resolution,
    ResolutionUnit, SourceColor, Supplements,
};
pub use limits::{LimitExceeded, ResourceLimits, ThreadingPolicy};
pub use metadata::Metadata;
pub use orientation::{Orientation, OrientationHint};
pub use output::{AnimationFrame, OwnedAnimationFrame};

pub use capabilities::UnsupportedOperation;
pub use detect::SourceEncodingDetails;
pub use error::{CodecErrorExt, find_cause};
pub use traits::Unsupported;

// =========================================================================
// Crate-level re-exports (qualified access, not individual types)
// =========================================================================
//
/// Owned, clonable, type-erased stop token.
///
/// Re-exported from [`almost_enough::StopToken`]. Wraps any `Stop` in an
/// enum that avoids vtable dispatch for `Stopper`/`SyncStopper`/`Unstoppable`,
/// collapses nested tokens, and is `Clone + Send + Sync + 'static`.
pub use almost_enough::StopToken;
pub use enough;
pub use enough::Unstoppable;

// =========================================================================
// pub(crate) re-exports — keep internal `use crate::Foo` paths working
// for items that moved out of the public root into sub-modules.
// =========================================================================

pub(crate) use capabilities::{DecodeCapabilities, EncodeCapabilities};
pub(crate) use cost::OutputInfo;
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
/// EncoderConfig → EncodeJob ──────┤
///                                  └→ AnimationFrameEnc (implements AnimationFrameEncoder)
/// ```
///
/// # Object-safe dyn dispatch
///
/// ```text
/// DynEncoderConfig → DynEncodeJob → DynEncoder / DynAnimationFrameEncoder
/// ```
///
/// Codec implementors implement the generic traits. Dispatch callers
/// use the `Dyn*` variants for codec-agnostic operation.
pub mod encode {
    // Traits — config, job, execution
    pub use crate::traits::{AnimationFrameEncoder, EncodeJob, Encoder, EncoderConfig};

    // Object-safe dyn dispatch
    pub use crate::traits::{
        BoxedError, DynAnimationFrameEncoder, DynEncodeJob, DynEncoder, DynEncoderConfig,
    };

    // Types
    pub use crate::capabilities::EncodeCapabilities;
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
///                                  └→ AnimationFrameDec (implements AnimationFrameDecoder)
/// ```
///
/// # Object-safe dyn dispatch
///
/// ```text
/// DynDecoderConfig → DynDecodeJob → DynDecoder / DynAnimationFrameDecoder / DynStreamingDecoder
/// ```
///
/// Codec implementors implement the generic traits. Dispatch callers
/// use the `Dyn*` variants for codec-agnostic operation.
pub mod decode {
    // Traits — config, job, execution
    pub use crate::traits::{
        AnimationFrameDecoder, Decode, DecodeJob, DecoderConfig, StreamingDecode,
    };

    // Object-safe dyn dispatch
    pub use crate::traits::{
        BoxedError, DynAnimationFrameDecoder, DynDecodeJob, DynDecoder, DynDecoderConfig,
        DynStreamingDecoder,
    };

    // Types
    pub use crate::capabilities::DecodeCapabilities;
    pub use crate::cost::OutputInfo;
    pub use crate::output::{AnimationFrame, DecodeOutput, OwnedAnimationFrame};
    pub use crate::policy::DecodePolicy;
    pub use crate::sink::{DecodeRowSink, SinkError};

    pub use crate::negotiate::{is_format_available, negotiate_pixel_format};

    // Source encoding detection
    pub use crate::detect::SourceEncodingDetails;

    // Shared types re-exported for convenience (commonly needed alongside decode)
    pub use crate::info::{EmbeddedMetadata, SourceColor};
}
