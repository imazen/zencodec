//! Encoder configuration, encode jobs, and per-format encode traits.

use alloc::boxed::Box;

use crate::format::ImageFormat;
use crate::{EncodeCapabilities, EncodeOutput, MetadataView, ResourceLimits, Stop};
use rgb::{Gray, Rgb, Rgba};
use zenpixels::{PixelDescriptor, PixelSlice};

use super::dyn_encoding::{DynEncoder, DynFrameEncoder, FrameEncoderShim};
use super::encoder::{Encoder, FrameEncoder};
use super::BoxedError;

// ===========================================================================
// Encoder configuration
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

    /// Encoder capabilities (metadata support, cancellation, etc.).
    ///
    /// Returns a static reference describing what this encoder supports.
    fn capabilities() -> &'static EncodeCapabilities {
        &EncodeCapabilities::EMPTY
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

// ===========================================================================
// Encode job
// ===========================================================================

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

    /// Set encode security policy (controls metadata embedding, etc.).
    ///
    /// Default no-op. Codecs that support policy check the flags in
    /// [`EncodePolicy`](crate::EncodePolicy) to decide what to embed.
    fn with_policy(self, _policy: crate::EncodePolicy) -> Self {
        self
    }

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

    // --- Type-erased convenience methods ---

    /// Create a type-erased one-shot encoder.
    ///
    /// Returns a boxed [`DynEncoder`] that accepts any [`PixelSlice`]
    /// (type-erased) and produces encoded output. All configuration —
    /// both universal ([`EncoderConfig::with_generic_quality`]) and
    /// codec-specific (methods on the concrete config type) — is
    /// applied *before* this call.
    ///
    /// Only available when `Enc` implements [`Encoder`].
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Codec-specific options on the concrete type
    /// let config = JpegConfig::new()
    ///     .set_chroma_subsampling(ChromaSubsampling::Yuv444)
    ///     .with_generic_quality(92.0);
    ///
    /// // Erase the codec type
    /// let encode = config.job()
    ///     .with_metadata(&meta)
    ///     .dyn_encoder()?;
    ///
    /// // No generics from here on
    /// let output = encode.encode(pixels)?;
    /// ```
    fn dyn_encoder(self) -> Result<Box<dyn DynEncoder + 'a>, BoxedError>
    where
        Self: 'a,
        Self::Enc: Encoder,
    {
        let enc = self.encoder().map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(super::dyn_encoding::EncoderShim(enc)))
    }

    /// Create a type-erased frame-by-frame encoder.
    ///
    /// Only available when `FrameEnc` implements [`FrameEncoder`].
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut enc = config.job()
    ///     .with_loop_count(Some(0))
    ///     .dyn_frame_encoder()?;
    ///
    /// enc.push_encode_frame(frame1)?;
    /// enc.push_encode_frame(frame2)?;
    /// let output = enc.finish()?;
    /// ```
    fn dyn_frame_encoder(self) -> Result<Box<dyn DynFrameEncoder + 'a>, BoxedError>
    where
        Self: 'a,
        Self::FrameEnc: FrameEncoder,
    {
        let enc = self
            .frame_encoder()
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(FrameEncoderShim(enc)))
    }
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
