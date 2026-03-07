//! Common codec traits.
//!
//! These traits define the execution interface for image codecs:
//!
//! ```text
//! ENCODE:
//!                                  ┌→ Enc (implements Encoder and/or EncodeRgb8, EncodeRgba8, ...)
//! EncoderConfig → EncodeJob<'a> ──┤
//!                                  └→ FrameEnc (implements FrameEncoder and/or FrameEncodeRgba8, ...)
//!
//! DECODE:
//!                                  ┌→ Dec (implements Decode)
//! DecoderConfig → DecodeJob<'a> ──┤
//!                                  └→ FrameDec (implements FrameDecode)
//! ```
//!
//! # Encoding: two complementary approaches
//!
//! **Type-erased** ([`Encoder`], [`FrameEncoder`]): The encoder accepts any
//! pixel format at runtime via [`PixelSlice`]. It dispatches internally based
//! on the descriptor. Good for generic pipelines and codecs that handle many
//! formats uniformly (e.g. PNM, BMP).
//!
//! **Per-format typed** ([`EncodeRgb8`], [`EncodeRgba8`], etc.): Each trait
//! is a compile-time guarantee that the codec can encode that exact format.
//! No runtime dispatch needed. Good for codecs with format-specific paths.
//!
//! A codec can implement both: type-erased for generic callers, per-format
//! for callers that know the pixel type statically.
//!
//! # Decoding
//!
//! Decoding is **type-erased**: the output format is discovered at runtime
//! from the file. The caller provides a ranked preference list of
//! [`PixelDescriptor`](crate::PixelDescriptor)s and the decoder picks the
//! best match it can produce without lossy conversion.
//!
//! Color management is explicitly **not** the codec's job. Decoders return
//! native pixels with ICC/CICP metadata. Encoders accept pixels as-is and
//! embed the provided metadata. The caller handles CMS transforms.

mod decoder;
mod decoding;
mod dyn_decoding;
mod dyn_encoding;
mod encoder;
mod encoding;

pub use decoder::{Decode, FrameDecode, StreamingDecode};
pub use decoding::{DecodeJob, DecoderConfig};
pub use dyn_decoding::{
    DynDecodeJob, DynDecoder, DynDecoderConfig, DynFrameDecoder, DynStreamingDecoder,
};
pub use dyn_encoding::{DynEncodeJob, DynEncoder, DynEncoderConfig, DynFrameEncoder};
pub use encoder::{Encoder, FrameEncoder};
pub use encoding::{
    EncodeGray8, EncodeGray16, EncodeGrayF32, EncodeJob, EncodeRgb8, EncodeRgb16, EncodeRgbF16,
    EncodeRgbF32, EncodeRgba8, EncodeRgba16, EncodeRgbaF16, EncodeRgbaF32, EncoderConfig,
    FrameEncodeRgb8, FrameEncodeRgba8,
};

use alloc::boxed::Box;

/// Boxed error type for type-erased codec operations.
///
/// Used by [`EncodeJob::dyn_encoder`], [`DecodeJob::dyn_decoder`], and
/// related methods that erase the concrete codec type.
pub type BoxedError = Box<dyn core::error::Error + Send + Sync>;
