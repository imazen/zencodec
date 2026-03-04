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

use crate::format::ImageFormat;
use crate::orientation::OrientationHint;
use crate::{
    CodecCapabilities, DecodeFrame, DecodeOutput, EncodeFrame, EncodeOutput, ImageInfo,
    MetadataView, OutputInfo, ResourceLimits, Stop,
};
use rgb::{Gray, Rgb, Rgba};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

// ===========================================================================
// Encode traits
// ===========================================================================

/// Reusable encoder configuration.
///
/// Implemented by each codec's config type. Config types are `Clone + Send +
/// Sync` with no lifetimes — store them in structs, share across threads.
///
/// Universal encoding parameters (quality, effort, lossless) have default
/// no-op implementations. Use the corresponding getter to check if the
/// codec accepted a value.
///
/// The `job()` method creates a per-operation [`EncodeJob`] that borrows
/// temporary data (stop tokens, metadata, resource limits).
pub trait EncoderConfig: Clone + Send + Sync {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Per-operation job type.
    type Job<'a>: EncodeJob<'a, Error = Self::Error>
    where
        Self: 'a;

    /// The image format this encoder produces.
    fn format() -> ImageFormat;

    /// Pixel formats this encoder accepts natively (without internal conversion).
    ///
    /// Every descriptor in this list is a guarantee: the corresponding
    /// per-format encode trait is implemented and will work without format
    /// conversion. Must not be empty.
    fn supported_descriptors() -> &'static [PixelDescriptor];

    /// Codec capabilities (metadata support, cancellation, etc.).
    ///
    /// Returns a static reference describing what this codec supports.
    fn capabilities() -> &'static CodecCapabilities {
        &CodecCapabilities::EMPTY
    }

    /// Set encoding quality on a calibrated 0.0–100.0 scale.
    ///
    /// "Generic" because this is the codec-agnostic quality knob. Individual
    /// codecs may also have format-specific quality methods on their config types.
    ///
    /// Default no-op. Check [`generic_quality()`](EncoderConfig::generic_quality)
    /// for the current value.
    fn with_generic_quality(self, _quality: f32) -> Self {
        self
    }

    /// Set encoding effort (higher = slower, better compression).
    ///
    /// "Generic" because this is the codec-agnostic effort knob. Individual
    /// codecs may also have format-specific effort/speed methods.
    ///
    /// Each codec maps this to its internal effort/speed scale.
    /// Default no-op.
    fn with_generic_effort(self, _effort: i32) -> Self {
        self
    }

    /// Enable or disable lossless encoding.
    ///
    /// Default no-op. When lossless is enabled, quality is ignored.
    fn with_lossless(self, _lossless: bool) -> Self {
        self
    }

    /// Set independent alpha channel quality on a calibrated 0.0–100.0 scale.
    ///
    /// Default no-op.
    fn with_alpha_quality(self, _quality: f32) -> Self {
        self
    }

    /// Current generic quality value, or `None` if the codec has no quality tuning.
    fn generic_quality(&self) -> Option<f32> {
        None
    }

    /// Current generic effort value, or `None` if the codec has no effort tuning.
    fn generic_effort(&self) -> Option<i32> {
        None
    }

    /// Current lossless setting, or `None` if the codec doesn't support it.
    fn is_lossless(&self) -> Option<bool> {
        None
    }

    /// Current alpha quality value, or `None` if unsupported.
    fn alpha_quality(&self) -> Option<f32> {
        None
    }

    /// Create a per-operation job.
    fn job(&self) -> Self::Job<'_>;
}

/// Per-operation encode job.
///
/// Created by [`EncoderConfig::job()`]. Binds metadata, limits, and
/// cancellation for a single encode operation. Produces either an `Enc`
/// (single image via per-format traits) or a `FrameEnc` (animation via
/// per-format frame traits).
pub trait EncodeJob<'a>: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Single-image encoder type.
    ///
    /// Implements per-format encode traits (`EncodeRgb8`, `EncodeRgba8`,
    /// etc.) for each pixel format the codec accepts.
    type Enc: Sized;

    /// Animation encoder type.
    ///
    /// Implements per-format frame encode traits (`FrameEncodeRgba8`, etc.).
    type FrameEnc: Sized;

    /// Set cooperative cancellation token.
    fn with_stop(self, stop: &'a dyn Stop) -> Self;

    /// Override resource limits for this operation.
    fn with_limits(self, limits: ResourceLimits) -> Self;

    /// Set metadata (ICC, EXIF, XMP) to embed in the output.
    ///
    /// The codec embeds what the format supports, silently skips the rest.
    fn with_metadata(self, meta: &'a MetadataView<'a>) -> Self;

    /// Set animation canvas dimensions.
    ///
    /// For compositing formats (GIF, APNG, WebP), individual frames can be
    /// smaller than the canvas. Default: canvas = first frame's dimensions.
    fn with_canvas_size(self, _width: u32, _height: u32) -> Self {
        self
    }

    /// Set animation loop count.
    ///
    /// - `Some(0)` = loop forever
    /// - `Some(n)` = loop `n` times
    /// - `None` = format default
    ///
    /// Default no-op. Only meaningful before `frame_encoder()`.
    fn with_loop_count(self, _count: Option<u32>) -> Self {
        self
    }

    /// Create a one-shot encoder for a single image.
    fn encoder(self) -> Result<Self::Enc, Self::Error>;

    /// Create a frame-by-frame encoder for animation.
    fn frame_encoder(self) -> Result<Self::FrameEnc, Self::Error>;
}

// ===========================================================================
// Per-format encode traits
// ===========================================================================
//
// Each codec implements only the pixel formats it accepts. The trait name
// IS the format contract — compile-time enforcement.
//
// Codec format matrix:
//
//               Rgb8  Rgba8  Gray8  Rgb16  Rgba16  Gray16  RgbF16  RgbaF16  RgbF32  RgbaF32  GrayF32
// JPEG           ✓             ✓
// WebP           ✓      ✓
// GIF                   ✓
// PNG            ✓      ✓      ✓      ✓       ✓      ✓
// AVIF           ✓      ✓                                                    ✓        ✓
// JXL            ✓      ✓      ✓      ✓       ✓      ✓      ✓        ✓      ✓        ✓        ✓

/// Encode from 8-bit RGB pixels.
pub trait EncodeRgb8 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGB8 pixels. Consumes self (single-shot).
    fn encode_rgb8(self, pixels: PixelSlice<'_, Rgb<u8>>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 8-bit RGBA pixels.
pub trait EncodeRgba8 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGBA8 pixels. Consumes self (single-shot).
    fn encode_rgba8(self, pixels: PixelSlice<'_, Rgba<u8>>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 8-bit grayscale pixels.
pub trait EncodeGray8 {
    /// The codec-specific error type.
    type Error;
    /// Encode Gray8 pixels. Consumes self (single-shot).
    fn encode_gray8(self, pixels: PixelSlice<'_, Gray<u8>>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 16-bit RGB pixels.
pub trait EncodeRgb16 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGB16 pixels. Consumes self (single-shot).
    fn encode_rgb16(self, pixels: PixelSlice<'_, Rgb<u16>>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 16-bit RGBA pixels.
pub trait EncodeRgba16 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGBA16 pixels. Consumes self (single-shot).
    fn encode_rgba16(self, pixels: PixelSlice<'_, Rgba<u16>>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 16-bit grayscale pixels.
pub trait EncodeGray16 {
    /// The codec-specific error type.
    type Error;
    /// Encode Gray16 pixels. Consumes self (single-shot).
    fn encode_gray16(self, pixels: PixelSlice<'_, Gray<u16>>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from half-precision (f16) RGB pixels.
///
/// Uses type-erased `PixelSlice` because the `rgb` crate has no half-float type.
pub trait EncodeRgbF16 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGB f16 pixels. Consumes self (single-shot).
    fn encode_rgb_f16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from half-precision (f16) RGBA pixels.
///
/// Uses type-erased `PixelSlice` because the `rgb` crate has no half-float type.
pub trait EncodeRgbaF16 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGBA f16 pixels. Consumes self (single-shot).
    fn encode_rgba_f16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 32-bit float RGB pixels.
pub trait EncodeRgbF32 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGB f32 pixels. Consumes self (single-shot).
    fn encode_rgb_f32(self, pixels: PixelSlice<'_, Rgb<f32>>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 32-bit float RGBA pixels.
pub trait EncodeRgbaF32 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGBA f32 pixels. Consumes self (single-shot).
    fn encode_rgba_f32(
        self,
        pixels: PixelSlice<'_, Rgba<f32>>,
    ) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 32-bit float grayscale pixels.
pub trait EncodeGrayF32 {
    /// The codec-specific error type.
    type Error;
    /// Encode Gray f32 pixels. Consumes self (single-shot).
    fn encode_gray_f32(
        self,
        pixels: PixelSlice<'_, Gray<f32>>,
    ) -> Result<EncodeOutput, Self::Error>;
}

// ===========================================================================
// Per-format frame encode traits (animation)
// ===========================================================================

/// Encode animation frames from 8-bit RGB pixels.
pub trait FrameEncodeRgb8 {
    /// The codec-specific error type.
    type Error;
    /// Push one RGB8 animation frame.
    fn push_frame_rgb8(
        &mut self,
        pixels: PixelSlice<'_, Rgb<u8>>,
        duration_ms: u32,
    ) -> Result<(), Self::Error>;
    /// Finalize and return the encoded animation.
    fn finish_rgb8(self) -> Result<EncodeOutput, Self::Error>;
}

/// Encode animation frames from 8-bit RGBA pixels.
pub trait FrameEncodeRgba8 {
    /// The codec-specific error type.
    type Error;
    /// Push one RGBA8 animation frame.
    fn push_frame_rgba8(
        &mut self,
        pixels: PixelSlice<'_, Rgba<u8>>,
        duration_ms: u32,
    ) -> Result<(), Self::Error>;
    /// Finalize and return the encoded animation.
    fn finish_rgba8(self) -> Result<EncodeOutput, Self::Error>;
}

// ===========================================================================
// Type-erased encode traits
// ===========================================================================

/// Type-erased single-image encoder.
///
/// Accepts any pixel format at runtime via [`PixelSlice`] (type-erased).
/// The encoder dispatches internally based on the pixel descriptor.
///
/// Three mutually exclusive usage paths:
/// - [`encode()`](Encoder::encode) — all at once, consumes self
/// - [`push_rows()`](Encoder::push_rows) + [`finish()`](Encoder::finish) — caller pushes rows
/// - [`encode_from()`](Encoder::encode_from) — encoder pulls rows from a callback
///
/// Codecs that need full-frame data (e.g. AV1) may buffer internally
/// when rows are pushed or pulled incrementally.
///
/// Codecs may implement this alongside per-format traits like [`EncodeRgb8`].
pub trait Encoder: Sized {
    /// The codec-specific error type.
    ///
    /// Must implement `From<UnsupportedOperation>` so default method
    /// implementations can return proper errors for unimplemented paths.
    type Error: core::error::Error + Send + Sync + 'static + From<crate::UnsupportedOperation>;

    /// Suggested strip height for optimal row-level encoding.
    ///
    /// For JPEG, typically the MCU height (8 or 16 rows).
    /// For PNG, typically 1 (row-at-a-time filtering).
    ///
    /// Returns 0 if the codec has no preference or doesn't support
    /// row-level encoding.
    fn preferred_strip_height(&self) -> u32 {
        0
    }

    /// Encode a complete image at once (consumes self).
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;

    /// Push scanline rows incrementally.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelEncode`](crate::UnsupportedOperation::RowLevelEncode).
    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelEncode.into())
    }

    /// Finalize after push_rows. Returns encoded output.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelEncode`](crate::UnsupportedOperation::RowLevelEncode).
    fn finish(self) -> Result<EncodeOutput, Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelEncode.into())
    }

    /// Encode by pulling rows from a source callback.
    ///
    /// The encoder calls `source` repeatedly with the row index and a
    /// mutable buffer slice. The callback fills the buffer and returns
    /// the number of rows written. Returns `0` to signal end of image.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::PullEncode`](crate::UnsupportedOperation::PullEncode).
    fn encode_from(
        self,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, Self::Error> {
        Err(crate::UnsupportedOperation::PullEncode.into())
    }
}

/// Type-erased animation encoder.
///
/// Accepts any pixel format at runtime via [`PixelSlice`] (type-erased).
///
/// Three mutually exclusive per-frame paths:
/// - [`push_frame()`](FrameEncoder::push_frame) /
///   [`push_encode_frame()`](FrameEncoder::push_encode_frame) — complete frame at once
/// - [`begin_frame()`](FrameEncoder::begin_frame) +
///   [`push_rows()`](FrameEncoder::push_rows) +
///   [`end_frame()`](FrameEncoder::end_frame) — caller pushes rows
/// - [`pull_frame()`](FrameEncoder::pull_frame) — encoder pulls rows from a callback
///
/// Codecs may implement this alongside per-format frame traits like [`FrameEncodeRgba8`].
pub trait FrameEncoder: Sized {
    /// The codec-specific error type.
    ///
    /// Must implement `From<UnsupportedOperation>` so default method
    /// implementations can return proper errors for unimplemented paths.
    type Error: core::error::Error + Send + Sync + 'static + From<crate::UnsupportedOperation>;

    /// Push a complete full-canvas frame.
    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), Self::Error>;

    /// Push a frame with sub-canvas positioning and compositing.
    ///
    /// Default: delegates to [`push_frame()`](FrameEncoder::push_frame).
    fn push_encode_frame(&mut self, frame: EncodeFrame<'_>) -> Result<(), Self::Error> {
        self.push_frame(frame.pixels, frame.duration_ms)
    }

    /// Begin a new frame (for row-level building).
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelFrameEncode`](crate::UnsupportedOperation::RowLevelFrameEncode).
    fn begin_frame(&mut self, _duration_ms: u32) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelFrameEncode.into())
    }

    /// Push rows into the current frame (after begin_frame).
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelFrameEncode`](crate::UnsupportedOperation::RowLevelFrameEncode).
    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelFrameEncode.into())
    }

    /// End the current frame (after pushing all rows).
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::RowLevelFrameEncode`](crate::UnsupportedOperation::RowLevelFrameEncode).
    fn end_frame(&mut self) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelFrameEncode.into())
    }

    /// Encode a frame by pulling rows from a source callback.
    ///
    /// # Errors
    ///
    /// Default returns [`UnsupportedOperation::PullFrameEncode`](crate::UnsupportedOperation::PullFrameEncode).
    fn pull_frame(
        &mut self,
        _duration_ms: u32,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<(), Self::Error> {
        Err(crate::UnsupportedOperation::PullFrameEncode.into())
    }

    /// Set animation loop count.
    ///
    /// - `Some(0)` = loop forever
    /// - `Some(n)` = loop `n` times
    /// - `None` = format default
    ///
    /// Default no-op.
    fn with_loop_count(&mut self, _count: Option<u32>) {}

    /// Finalize animation. Returns encoded output.
    fn finish(self) -> Result<EncodeOutput, Self::Error>;
}

// ===========================================================================
// Decode traits
// ===========================================================================

/// Reusable decoder configuration.
///
/// Implemented by each codec's config type. Config types are `Clone + Send +
/// Sync` with no lifetimes.
///
/// Probing lives on [`DecodeJob`], not here, because probing needs limits
/// and cancellation context.
pub trait DecoderConfig: Clone + Send + Sync {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Per-operation job type.
    type Job<'a>: DecodeJob<'a, Error = Self::Error>
    where
        Self: 'a;

    /// The image format this decoder handles.
    fn format() -> ImageFormat;

    /// Pixel formats this decoder can produce natively.
    ///
    /// Every descriptor is a guarantee: the decoder can produce this format
    /// without lossy conversion. Must not be empty.
    fn supported_descriptors() -> &'static [PixelDescriptor];

    /// Codec capabilities (metadata support, cancellation, etc.).
    ///
    /// Returns a static reference describing what this codec supports.
    fn capabilities() -> &'static CodecCapabilities {
        &CodecCapabilities::EMPTY
    }

    /// Create a per-operation job.
    fn job(&self) -> Self::Job<'_>;
}

/// Per-operation decode job.
///
/// Created by [`DecoderConfig::job()`]. Holds limits, cancellation, and
/// decode hints. Probing lives here because it needs the limits/stop context.
///
/// # Decode hints
///
/// Hints let the caller request spatial transforms (crop, scale, orientation)
/// that the decoder may apply during decode. The decoder is free to ignore
/// any hint. Call [`output_info()`](DecodeJob::output_info) after setting
/// hints to learn what the decoder will actually produce.
pub trait DecodeJob<'a>: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Single-image decoder type.
    type Dec: Decode<Error = Self::Error>;

    /// Streaming decoder type.
    ///
    /// Implements [`StreamingDecode`] for batch/scanline-level decode.
    /// Set to `()` if the codec does not support streaming decode.
    type StreamDec: StreamingDecode<Error = Self::Error>;

    /// Animation decoder type.
    type FrameDec: FrameDecode<Error = Self::Error>;

    /// Set cooperative cancellation token.
    fn with_stop(self, stop: &'a dyn Stop) -> Self;

    /// Override resource limits for this operation.
    fn with_limits(self, limits: ResourceLimits) -> Self;

    // --- Probing (needs limits + stop context) ---

    /// Probe image metadata cheaply (header parse only).
    ///
    /// O(header), not O(pixels). Parses container headers to extract
    /// dimensions, format, and basic metadata. May not return frame
    /// counts or data requiring a full parse.
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;

    /// Probe image metadata with a full parse.
    ///
    /// May be expensive (e.g., parsing all GIF frames to count them).
    /// Returns complete metadata including frame counts.
    ///
    /// Default: delegates to [`probe()`](DecodeJob::probe).
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, Self::Error> {
        self.probe(data)
    }

    // --- Decode hints (optional, decoder may ignore) ---

    /// Hint: crop to this region in source coordinates.
    ///
    /// The decoder may adjust for block alignment (JPEG MCU boundaries).
    fn with_crop_hint(self, _x: u32, _y: u32, _width: u32, _height: u32) -> Self {
        self
    }

    /// Hint: target output dimensions for prescaling.
    ///
    /// Some codecs decode at reduced resolution cheaply (JPEG 1/2/4/8).
    fn with_scale_hint(self, _max_width: u32, _max_height: u32) -> Self {
        self
    }

    /// Set orientation handling strategy.
    ///
    /// See [`OrientationHint`] for the available strategies.
    /// Default: [`OrientationHint::Preserve`].
    fn with_orientation(self, _hint: OrientationHint) -> Self {
        self
    }

    // --- Output prediction ---

    /// Predict what the decoder will produce given current hints.
    ///
    /// Returns dimensions, pixel format, and which hints were honored.
    /// Call after setting hints, before creating a decoder.
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, Self::Error>;

    // --- Executor creation ---
    //
    // All executors bind `data` here so the DecodeJob is the single
    // place where input is provided. This keeps Decode/StreamingDecode/
    // FrameDecode free of data parameters, and prepares for future
    // IO-read sources (the job can bind a reader instead of a slice).
    //
    // Consistent parameter order: data, [sink], preferred.

    /// Create a one-shot decoder bound to `data`.
    ///
    /// The returned `Dec` borrows `data` for the duration of decoding.
    /// Call [`Decode::decode()`] on the result to get pixels.
    ///
    /// `preferred` is a ranked list of desired output formats. The decoder
    /// picks the first it can produce without lossy conversion. Pass `&[]`
    /// for the decoder's native format.
    fn decoder(
        self,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Self::Dec, Self::Error>;

    /// Decode directly into a caller-owned sink (push model).
    ///
    /// Decodes and pushes strips into `sink` via
    /// [`DecodeRowSink::demand()`]. Returns [`OutputInfo`] describing
    /// what was produced (pixels went into the sink, not a return value).
    ///
    /// `preferred` is a ranked list of desired output formats.
    ///
    /// Default implementation creates a [`decoder()`](DecodeJob::decoder),
    /// calls [`Decode::decode()`], then copies the result into the sink
    /// strip by strip. Codecs with native row streaming should override
    /// this for zero-copy.
    fn push_decoder(
        self,
        data: &'a [u8],
        sink: &mut dyn crate::DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, Self::Error> {
        let dec = self.decoder(data, preferred)?;
        let output = dec.decode()?;
        let ps = output.pixels().as_slice();
        let desc = ps.descriptor();
        let w = ps.width();
        let h = ps.rows();

        // Push all rows into the sink as a single strip
        let mut dst = sink.demand(0, h, w, desc);
        for row in 0..h {
            dst.row_mut(row).copy_from_slice(ps.row(row));
        }

        let info = output.info();
        Ok(OutputInfo::full_decode(info.width, info.height, desc))
    }

    /// Create a streaming decoder that yields scanline batches.
    ///
    /// Binds `data` — the decoder borrows the input for the duration
    /// of streaming. Returns an error if the codec does not support
    /// streaming decode.
    ///
    /// `preferred` is a ranked list of desired output formats.
    ///
    /// See [`StreamingDecode`] for the batch pull API.
    fn streaming_decoder(
        self,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Self::StreamDec, Self::Error>;

    /// Create a frame-by-frame animation decoder.
    ///
    /// Binds `data` — the decoder parses the container upfront.
    ///
    /// `preferred` is a ranked list of desired output formats.
    fn frame_decoder(
        self,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Self::FrameDec, Self::Error>;
}

/// Single-image decode. Returns owned pixels.
///
/// Created by [`DecodeJob::decoder()`] with input data and format
/// preferences already bound.
pub trait Decode: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Decode to owned pixels.
    ///
    /// Input data and format preferences were bound when the decoder
    /// was created via [`DecodeJob::decoder()`].
    fn decode(self) -> Result<DecodeOutput, Self::Error>;
}

/// Streaming scanline-batch decode.
///
/// The decoder yields strips of scanlines at whatever height it prefers:
/// MCU height for JPEG, full image for simple formats, single scanline
/// for PNG, etc. The caller pulls batches until `None` is returned.
///
/// Created by [`DecodeJob::streaming_decoder()`] with input data and
/// format preferences already bound.
///
/// # Usage
///
/// ```text
/// let job = config.job();
/// let info = job.output_info(data)?;
/// let mut dec = job.streaming_decoder(&[], data)?;
/// while let Some((y, strip)) = dec.next_batch()? {
///     // process strip.rows() scanlines starting at row y
/// }
/// ```
pub trait StreamingDecode {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Pull the next batch of scanlines.
    ///
    /// Returns `Ok(Some((y, strip)))` with the row offset and pixel data,
    /// or `Ok(None)` when the image is fully decoded.
    ///
    /// Format preferences were bound at construction. The format remains
    /// consistent across all batches.
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error>;

    /// Image metadata, available after construction.
    fn info(&self) -> &ImageInfo;
}

/// Trivial rejection impl — codecs that don't support streaming set
/// `type StreamDec = ()` and `streaming_decoder()` returns an error.
impl StreamingDecode for () {
    type Error = crate::UnsupportedOperation;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error> {
        Err(crate::UnsupportedOperation::RowLevelDecode)
    }

    fn info(&self) -> &ImageInfo {
        // This is unreachable — streaming_decoder() returns Err before
        // the caller can call info(). But we need the impl for Sized.
        panic!("StreamingDecode not supported");
    }
}

/// Animation decode. Returns owned frames.
///
/// Created by [`DecodeJob::frame_decoder()`] with input data and
/// format preferences already bound.
pub trait FrameDecode: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Number of frames, if known without decoding.
    fn frame_count(&self) -> Option<u32> {
        None
    }

    /// Animation loop count from the container.
    ///
    /// - `Some(0)` = loop forever
    /// - `Some(n)` = loop `n` times
    /// - `None` = unknown or not specified
    fn loop_count(&self) -> Option<u32> {
        None
    }

    /// Pull next frame. Returns `None` when all frames consumed.
    ///
    /// Format preferences were bound at construction.
    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, Self::Error>;

    /// Decode next frame directly into a caller-owned sink (push model).
    ///
    /// Returns `Ok(Some(info))` with frame metadata, or `Ok(None)` when
    /// all frames are consumed.
    ///
    /// Default implementation calls [`next_frame()`](FrameDecode::next_frame)
    /// and copies the result into the sink. Codecs with native row streaming
    /// should override for zero-copy.
    fn next_frame_to_sink(
        &mut self,
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, Self::Error> {
        let frame = match self.next_frame()? {
            Some(f) => f,
            None => return Ok(None),
        };
        let ps = frame.pixels().as_slice();
        let desc = ps.descriptor();
        let w = ps.width();
        let h = ps.rows();

        let mut dst = sink.demand(0, h, w, desc);
        for row in 0..h {
            dst.row_mut(row).copy_from_slice(ps.row(row));
        }

        let info = frame.info();
        Ok(Some(OutputInfo::full_decode(info.width, info.height, desc)))
    }
}

// ===========================================================================
// Deprecated aliases
// ===========================================================================

/// Deprecated alias for [`Decode`].
#[deprecated(since = "0.2.0", note = "renamed to Decode")]
pub trait Decoder: Decode {}
#[allow(deprecated)]
impl<T: Decode> Decoder for T {}

/// Deprecated alias for [`FrameDecode`].
#[deprecated(since = "0.2.0", note = "renamed to FrameDecode")]
pub trait FrameDecoder: FrameDecode {}
#[allow(deprecated)]
impl<T: FrameDecode> FrameDecoder for T {}
