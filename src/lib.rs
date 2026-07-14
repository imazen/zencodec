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
//! - [`CodecSet`] — multi-codec registry: register configs once, then
//!   detect-and-decode or encode-by-format through one shared `&self` handle
//! - [`ImageInfo`] / [`Metadata`] / [`Orientation`] / [`OrientationHint`] — image metadata
//! - [`ResourceLimits`] / [`ThreadingPolicy`] — resource limit and threading configuration
//! - [`UnsupportedOperation`] / [`CodecErrorExt`] — standard unsupported operation reporting and error chain inspection
//! - [`prelude`] — one-import bundle of all encode/decode traits
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

// Pulled in only by the `std` feature, solely so [`CodecIoKind`] can carry a
// `std::io::ErrorKind` on the I/O category. The crate stays `#![no_std]`; when
// `core::io::ErrorKind` stabilizes this can drop.
#[cfg(feature = "std")]
extern crate std;

whereat::define_at_crate_info!();

mod capabilities;
/// Cross-codec color-signaling emission policy (ICC vs CICP). See
/// `docs/color-emit-model.md`.
mod color;
mod cost;
mod detect;
mod error;
pub mod estimate;
/// Structured EXIF/TIFF parsing, pruning, and serialization.
pub mod exif;
mod extensions;
mod fidelity;
mod format;
/// Cross-codec gain map types (ISO 21496-1).
pub mod gainmap;
/// Codec implementation helpers (not consumer API).
pub mod helpers;
/// Lightweight ICC profile inspection (tag extraction, no full parse).
// Kept `pub` — privatizing it removes `zencodec::icc` from the public API, a
// breaking change. The removal stays in CHANGELOG's QUEUED BREAKING CHANGES for
// a future major; do not ship it in a patch/minor.
pub mod icc;
mod info;
mod limits;
mod metadata;
mod negotiate;
mod orientation;
mod output;
mod policy;
mod set;
mod sink;
mod traits;

// =========================================================================
// Public root: shared types used by both encode and decode
// =========================================================================

pub use color::{
    CicpEmission, ColorEmitFields, ColorEmitPlan, ColorEmitPolicy, IccDisposition,
    resolve_color_emit,
};
// `ByteOrder` is intentionally NOT re-exported at the root: it is a TIFF/EXIF
// header detail used only within the `exif` module, and the bare name is too
// generic for the crate root. Reach it as `exif::ByteOrder`.
pub use exif::{Exif, ExifPolicy, Retention, TextEncoding};
pub use extensions::Extensions;
pub use format::{ImageFormat, ImageFormatDefinition, ImageFormatRegistry};
pub use gainmap::{
    GainMapChannel, GainMapDirection, GainMapInfo, GainMapParams, GainMapPresence, GainMapRender,
    ISO_21496_1_PRIMARY_APP2_BODY, ISO_21496_1_URN, Iso21496Format,
};
#[allow(deprecated)]
pub use icc::icc_extract_cicp;
pub use info::{
    Cicp, ContentLightLevel, ImageInfo, ImageSequence, MasteringDisplay, Resolution,
    ResolutionUnit, SourceColor, Supplements,
};
pub use limits::{AllocPreference, LimitExceeded, LimitKind, ResourceLimits, ThreadingPolicy};
pub use metadata::{IccRetention, Metadata, MetadataFields, MetadataPolicy};
pub use orientation::{Orientation, OrientationHint};
pub use output::{AnimationFrame, OwnedAnimationFrame};
pub use set::{CodecSet, CodecSetError};
pub use zenpixels::ColorAuthority;

pub use capabilities::UnsupportedOperation;
pub use detect::SourceEncodingDetails;
pub use error::{
    CategorizedError, CodecError, CodecErrorExt, CodecIoKind, ErrorCategory, ImageError,
    InternalKind, InvalidKind, PolicyKind, RequestError, ResourceError, StreamOffset,
    UnsupportedImageKind, find_cause,
};
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

/// Location-tracing error wrapper, re-exported from [`whereat`].
///
/// `whereat::At` is already part of this crate's public surface (the blanket
/// `impl CategorizedError for At<E>`), so re-exporting it adds no new coupling.
/// It is re-exported because [`At<CodecError>`](crate::CodecError) is the
/// recommended codec `Error` type: a codec can name
/// `zencodec::At<zencodec::CodecError>` and reach `start_at()` / `.at()` (via the
/// re-exported [`ErrorAtExt`] / [`ResultAtExt`]) without depending on `whereat`
/// directly.
pub use whereat;
pub use whereat::{At, ErrorAtExt, ResultAtExt};

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
    pub use crate::fidelity::{Fidelity, LossyTarget};
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

    // Gain-map decode intent + decoded payloads (the math stays in `ultrahdr-core`).
    pub use crate::gainmap::{DecodedGainMap, GainMapRender};
}

/// One-import trait bundle: `use zencodec::prelude::*;`.
///
/// Brings every encode/decode trait into scope so `.job()`, `.decoder()`,
/// `.encode()`, `.next_batch()`, and the other trait methods resolve without
/// hunting down individual `use` lines. Types are *not* included — import
/// those from the crate root or the [`encode`]/[`decode`] modules.
pub mod prelude {
    pub use crate::decode::{
        AnimationFrameDecoder, Decode, DecodeJob, DecoderConfig, DynAnimationFrameDecoder,
        DynDecodeJob, DynDecoder, DynDecoderConfig, DynStreamingDecoder, StreamingDecode,
    };
    pub use crate::encode::{
        AnimationFrameEncoder, DynAnimationFrameEncoder, DynEncodeJob, DynEncoder,
        DynEncoderConfig, EncodeJob, Encoder, EncoderConfig,
    };
}
