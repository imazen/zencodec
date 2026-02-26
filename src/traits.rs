//! Common codec traits.
//!
//! These traits define the execution interface for image codecs:
//!
//! ```text
//! ENCODE:
//!                                  ┌→ Enc (implements EncodeRgb8, EncodeRgba8, ...)
//! EncoderConfig → EncodeJob<'a> ──┤
//!                                  └→ FrameEnc (implements FrameEncodeRgba8, ...)
//!
//! DECODE:
//!                                  ┌→ Dec (implements Decode)
//! DecoderConfig → DecodeJob<'a> ──┤
//!                                  └→ FrameDec (implements FrameDecode)
//! ```
//!
//! Encoding is **typed per-format**: each pixel format the codec accepts is a
//! separate trait (`EncodeRgb8`, `EncodeRgba8`, etc.). If a codec doesn't
//! implement a trait, you get a compile error, not a runtime surprise.
//!
//! Decoding is **type-erased**: the output format is discovered at runtime from
//! the file. The caller provides a ranked preference list of
//! [`PixelDescriptor`](crate::PixelDescriptor)s and the decoder picks the best
//! match it can produce without lossy conversion.
//!
//! Color management is explicitly **not** the codec's job. Decoders return
//! native pixels with ICC/CICP metadata. Encoders accept pixels as-is and
//! embed the provided metadata. The caller handles CMS transforms.

use crate::format::ImageFormat;
use crate::orientation::OrientationHint;
use crate::{
    DecodeFrame, DecodeOutput, EncodeOutput, ImageInfo, MetadataView, OutputInfo, PixelDescriptor,
    PixelSlice, ResourceLimits, Stop,
};

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

    /// Set encoding quality on a calibrated 0.0–100.0 scale.
    ///
    /// Default no-op. Check [`quality()`](EncoderConfig::quality) for the
    /// current value.
    fn with_quality(self, _quality: f32) -> Self {
        self
    }

    /// Set encoding effort (higher = slower, better compression).
    ///
    /// Each codec maps this to its internal effort/speed scale.
    /// Default no-op.
    fn with_effort(self, _effort: i32) -> Self {
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

    /// Current quality value, or `None` if the codec has no quality tuning.
    fn quality(&self) -> Option<f32> {
        None
    }

    /// Current effort value, or `None` if the codec has no effort tuning.
    fn effort(&self) -> Option<i32> {
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
    fn encode_rgb8(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 8-bit RGBA pixels.
pub trait EncodeRgba8 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGBA8 pixels. Consumes self (single-shot).
    fn encode_rgba8(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 8-bit grayscale pixels.
pub trait EncodeGray8 {
    /// The codec-specific error type.
    type Error;
    /// Encode Gray8 pixels. Consumes self (single-shot).
    fn encode_gray8(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 16-bit RGB pixels.
pub trait EncodeRgb16 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGB16 pixels. Consumes self (single-shot).
    fn encode_rgb16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 16-bit RGBA pixels.
pub trait EncodeRgba16 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGBA16 pixels. Consumes self (single-shot).
    fn encode_rgba16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 16-bit grayscale pixels.
pub trait EncodeGray16 {
    /// The codec-specific error type.
    type Error;
    /// Encode Gray16 pixels. Consumes self (single-shot).
    fn encode_gray16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from half-precision (f16) RGB pixels.
pub trait EncodeRgbF16 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGB f16 pixels. Consumes self (single-shot).
    fn encode_rgb_f16(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from half-precision (f16) RGBA pixels.
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
    fn encode_rgb_f32(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 32-bit float RGBA pixels.
pub trait EncodeRgbaF32 {
    /// The codec-specific error type.
    type Error;
    /// Encode RGBA f32 pixels. Consumes self (single-shot).
    fn encode_rgba_f32(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}

/// Encode from 32-bit float grayscale pixels.
pub trait EncodeGrayF32 {
    /// The codec-specific error type.
    type Error;
    /// Encode Gray f32 pixels. Consumes self (single-shot).
    fn encode_gray_f32(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
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
        pixels: PixelSlice<'_>,
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
        pixels: PixelSlice<'_>,
        duration_ms: u32,
    ) -> Result<(), Self::Error>;
    /// Finalize and return the encoded animation.
    fn finish_rgba8(self) -> Result<EncodeOutput, Self::Error>;
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

    /// Create a one-shot decoder.
    fn decoder(self) -> Result<Self::Dec, Self::Error>;

    /// Create a frame-by-frame animation decoder.
    ///
    /// Binds `data` — the decoder parses the container upfront.
    fn frame_decoder(self, data: &'a [u8]) -> Result<Self::FrameDec, Self::Error>;
}

/// Single-image decode. Returns owned pixels.
///
/// The caller provides a ranked preference list of pixel descriptors.
/// The decoder picks the first it can produce without lossy conversion.
/// Empty slice = decoder's native format.
pub trait Decode: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Decode to owned pixels.
    ///
    /// `preferred` is a ranked list of desired output formats. The decoder
    /// picks the first it can produce without lossy conversion. Pass `&[]`
    /// for the decoder's native format.
    fn decode(
        self,
        data: &[u8],
        preferred: &[PixelDescriptor],
    ) -> Result<DecodeOutput, Self::Error>;
}

/// Animation decode. Returns owned frames.
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
    /// `preferred` is a ranked list of desired output formats (same
    /// semantics as [`Decode::decode()`]).
    fn next_frame(
        &mut self,
        preferred: &[PixelDescriptor],
    ) -> Result<Option<DecodeFrame>, Self::Error>;
}
