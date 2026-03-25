//! Common codec traits.
//!
//! ```text
//! ENCODE:
//!                                  ┌→ Enc (implements Encoder)
//! EncoderConfig → EncodeJob<'a> ──┤
//!                                  └→ AnimationFrameEnc (implements AnimationFrameEncoder)
//!
//! DECODE:
//!                                  ┌→ Dec (implements Decode)
//! DecoderConfig → DecodeJob<'a> ──┤→ StreamDec (implements StreamingDecode)
//!                                  └→ AnimationFrameDec (implements AnimationFrameDecoder)
//! ```
//!
//! Encoding and decoding are both **type-erased**: encoders accept any pixel
//! format at runtime via [`PixelSlice`] and dispatch internally based on the
//! descriptor. Decoders return native pixels; the caller provides a ranked
//! preference list of [`PixelDescriptor`](crate::PixelDescriptor)s.
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
mod unsupported;

#[allow(deprecated)]
pub use decoder::{AnimationFrameDecoder, Decode, StreamingDecode, render_frame_to_sink_via_copy};
#[allow(deprecated)]
pub use decoding::{DecodeJob, DecoderConfig, push_decoder_via_full_decode};
pub use dyn_decoding::{
    DynAnimationFrameDecoder, DynDecodeJob, DynDecoder, DynDecoderConfig, DynStreamingDecoder,
};
pub use dyn_encoding::{DynAnimationFrameEncoder, DynEncodeJob, DynEncoder, DynEncoderConfig};
pub use encoder::{AnimationFrameEncoder, Encoder};
pub use encoding::{EncodeJob, EncoderConfig};
pub use unsupported::Unsupported;

use alloc::boxed::Box;

/// Boxed error type for type-erased codec operations.
///
/// Used by [`EncodeJob::dyn_encoder`], [`DecodeJob::dyn_decoder`], and
/// related methods that erase the concrete codec type.
pub type BoxedError = Box<dyn core::error::Error + Send + Sync>;
