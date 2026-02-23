//! Common codec traits.
//!
//! These traits define the execution interface for image codecs, split into
//! four layers per side (encode and decode):
//!
//! ```text
//!                               ┌→ Encoder (one-shot or row-level push)
//! EncoderConfig → EncodeJob<'a> ┤
//!                               └→ FrameEncoder (animation: push frames or rows-per-frame)
//!
//!                               ┌→ Decoder (one-shot, decode-into, or row callback)
//! DecoderConfig → DecodeJob<'a> ┤
//!                               └→ FrameDecoder (animation: pull frames)
//! ```
//!
//! Universal encoding parameters (quality, effort, lossless) are on
//! [`EncoderConfig`]. Format-specific settings live on each codec's
//! concrete types. The traits handle execution, metadata, cancellation,
//! and resource limits.
//!
//! # Transfer function
//!
//! The convenience methods (e.g. `encode_rgb8`, `decode_into_rgba_f32`)
//! do **not** perform any transfer function conversion. The pixel data's
//! transfer function depends on the source — check `ImageInfo::cicp` or
//! the ICC profile. See [`PixelData`](crate::PixelData) for details.

use alloc::vec::Vec;

use imgref::{ImgRef, ImgRefMut, ImgVec};
use rgb::alt::BGRA;
use rgb::{Gray, Rgb, Rgba};

use crate::format::ImageFormat;
use crate::output::{EncodeFrame, TypedEncodeFrame};
use crate::pixel::{GrayAlpha, PixelData};
use crate::{
    CodecCapabilities, DecodeFrame, DecodeOutput, EncodeOutput, ImageInfo, MetadataView,
    PixelSlice, PixelSliceMut, ResourceLimits, Stop,
};

// ===========================================================================
// Encode traits (4)
// ===========================================================================

/// Common interface for encode configurations.
///
/// Implemented by each codec's config type (e.g. `zenjpeg::EncoderConfig`).
/// Config types are reusable (`Clone`) and have no lifetimes — they can be
/// stored in structs and shared across threads.
///
/// Universal encoding parameters — [`with_effort()`](EncoderConfig::with_effort),
/// [`with_calibrated_quality()`](EncoderConfig::with_calibrated_quality),
/// [`with_lossless()`](EncoderConfig::with_lossless) — are on the trait
/// with default no-op implementations. Getters ([`effort()`](EncoderConfig::effort),
/// [`calibrated_quality()`](EncoderConfig::calibrated_quality),
/// [`is_lossless()`](EncoderConfig::is_lossless)) return `Option` so callers
/// can detect whether a codec supports a parameter.
/// Quality uses a calibrated 0.0–100.0 scale where 85 matches libjpeg-turbo
/// quality 85 (each codec maintains its own calibration table).
/// Format-specific settings beyond these live on the concrete config type.
///
/// The `job()` method creates a per-operation [`EncodeJob`] that can borrow
/// temporary data (stop tokens, metadata, resource limits).
pub trait EncoderConfig: Clone + Send + Sync {
    /// The image format this encoder produces (e.g. JPEG, WebP, AVIF).
    ///
    /// Static — the format is known at the type level.
    fn format() -> ImageFormat;

    /// Pixel formats this encoder accepts natively (without internal conversion).
    ///
    /// Every descriptor in this list is a guarantee: calling `encode()` or
    /// `push_rows()` with a `PixelSlice` matching one of these descriptors
    /// **must** work without any format conversion. The codec processes the
    /// data directly.
    ///
    /// The encoder may also accept other formats via internal conversion,
    /// but these are the zero-overhead path. Callers use this to pick the
    /// best pixel format before encoding.
    ///
    /// Must not be empty — every codec can natively accept at least one format.
    fn supported_descriptors() -> &'static [crate::PixelDescriptor];
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Per-operation job type, created by [`job()`](EncoderConfig::job).
    type Job<'a>: EncodeJob<'a, Error = Self::Error>
    where
        Self: 'a;

    /// Codec capabilities (metadata support, cancellation, etc.).
    ///
    /// Returns a static reference describing what this codec supports.
    /// Use this to check before calling methods that may be no-ops.
    fn capabilities() -> &'static CodecCapabilities;

    // --- Universal encoding parameters ---
    //
    // These have default no-op implementations so codecs that don't support
    // a parameter aren't forced to stub it out. Use the corresponding getter
    // (e.g. `calibrated_quality()`) to check if the codec accepted the value.
    // `CodecCapabilities` reports supported ranges.

    /// Set encoding effort on a signed scale.
    ///
    /// Higher values = slower / better compression. Each codec maps this
    /// to its internal effort/speed scale. Values outside the codec's
    /// meaningful range are clamped.
    ///
    /// Default no-op. Check [`effort()`](EncoderConfig::effort) to see
    /// the current value, or [`capabilities().effort_range()`](CodecCapabilities::effort_range)
    /// for the supported range (`None` = no effort tuning).
    fn with_effort(self, _effort: i32) -> Self {
        self
    }

    /// Set lossy quality on a calibrated 0.0–100.0 scale.
    ///
    /// The scale is calibrated to match libjpeg-turbo output: quality 85
    /// on any codec targets the same butteraugli / SSIM2 score as
    /// libjpeg-turbo quality 85. Each codec maps this to its internal
    /// parameters via a calibration table.
    ///
    /// Values are clamped to \[0.0, 100.0\]. Default no-op. Check
    /// [`calibrated_quality()`](EncoderConfig::calibrated_quality) to see
    /// the current value, or [`capabilities().quality_range()`](CodecCapabilities::quality_range)
    /// for the supported range (`None` = lossless-only codec).
    fn with_calibrated_quality(self, _quality: f32) -> Self {
        self
    }

    /// Enable or disable lossless encoding.
    ///
    /// Default no-op. Check [`is_lossless()`](EncoderConfig::is_lossless)
    /// to see the current value, or
    /// [`capabilities().lossless()`](CodecCapabilities::lossless) for support.
    /// When lossless is enabled, [`with_calibrated_quality()`](EncoderConfig::with_calibrated_quality)
    /// is ignored.
    fn with_lossless(self, _lossless: bool) -> Self {
        self
    }

    /// Current effort value, or `None` if the codec has no effort tuning.
    fn effort(&self) -> Option<i32> {
        None
    }

    /// Current calibrated quality value, or `None` if the codec is lossless-only.
    fn calibrated_quality(&self) -> Option<f32> {
        None
    }

    /// Current lossless setting, or `None` if the codec doesn't support lossless.
    fn is_lossless(&self) -> Option<bool> {
        None
    }

    /// Set independent alpha channel quality on a calibrated 0.0–100.0 scale.
    ///
    /// Useful for codecs like AVIF, WebP, and JXL where the alpha plane
    /// can be encoded at a different quality than the color planes.
    /// Values are clamped to \[0.0, 100.0\].
    ///
    /// Default no-op. Check [`alpha_quality()`](EncoderConfig::alpha_quality)
    /// to see the current value.
    fn with_alpha_quality(self, _quality: f32) -> Self {
        self
    }

    /// Current alpha quality value, or `None` if the codec doesn't
    /// support independent alpha quality.
    fn alpha_quality(&self) -> Option<f32> {
        None
    }

    /// Create a per-operation job for this config.
    ///
    /// The job borrows the config and can accept temporary references
    /// (stop tokens, metadata, resource limits) before creating an
    /// encoder or frame encoder.
    fn job(&self) -> Self::Job<'_>;

    /// Convenience: encode RGB8 with default job settings.
    fn encode_rgb8(&self, img: ImgRef<'_, Rgb<u8>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode RGBA8 with default job settings.
    fn encode_rgba8(&self, img: ImgRef<'_, Rgba<u8>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode Gray8 with default job settings.
    fn encode_gray8(&self, img: ImgRef<'_, Gray<u8>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode BGRA8 with default job settings.
    fn encode_bgra8(&self, img: ImgRef<'_, BGRA<u8>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode BGRX8 (opaque BGRA, padding byte ignored) with default job settings.
    fn encode_bgrx8(&self, img: ImgRef<'_, BGRA<u8>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode RGB f32 with default job settings.
    fn encode_rgb_f32(&self, img: ImgRef<'_, Rgb<f32>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode RGBA f32 with default job settings.
    fn encode_rgba_f32(&self, img: ImgRef<'_, Rgba<f32>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode grayscale f32 with default job settings.
    fn encode_gray_f32(&self, img: ImgRef<'_, Gray<f32>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode RGB16 with default job settings.
    fn encode_rgb16(&self, img: ImgRef<'_, Rgb<u16>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode RGBA16 with default job settings.
    fn encode_rgba16(&self, img: ImgRef<'_, Rgba<u16>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode Gray16 with default job settings.
    fn encode_gray16(&self, img: ImgRef<'_, Gray<u16>>) -> Result<EncodeOutput, Self::Error> {
        self.job().encoder().encode(PixelSlice::from(img))
    }

    /// Convenience: encode GrayAlpha8 with default job settings.
    ///
    /// Routes through [`PixelData`] conversion since `GrayAlpha` lacks
    /// `ComponentBytes` (not zero-copy).
    fn encode_gray_alpha8(
        &self,
        img: ImgRef<'_, GrayAlpha<u8>>,
    ) -> Result<EncodeOutput, Self::Error> {
        let pixels: Vec<_> = img.pixels().collect();
        let owned = ImgVec::new(pixels, img.width(), img.height());
        self.encode_pixel_data(&PixelData::GrayAlpha8(owned))
    }

    /// Convenience: encode GrayAlpha16 with default job settings.
    ///
    /// Routes through [`PixelData`] conversion since `GrayAlpha` lacks
    /// `ComponentBytes` (not zero-copy).
    fn encode_gray_alpha16(
        &self,
        img: ImgRef<'_, GrayAlpha<u16>>,
    ) -> Result<EncodeOutput, Self::Error> {
        let pixels: Vec<_> = img.pixels().collect();
        let owned = ImgVec::new(pixels, img.width(), img.height());
        self.encode_pixel_data(&PixelData::GrayAlpha16(owned))
    }

    /// Convenience: encode GrayAlpha f32 with default job settings.
    ///
    /// Routes through [`PixelData`] conversion since `GrayAlpha` lacks
    /// `ComponentBytes` (not zero-copy).
    fn encode_gray_alpha_f32(
        &self,
        img: ImgRef<'_, GrayAlpha<f32>>,
    ) -> Result<EncodeOutput, Self::Error> {
        let pixels: Vec<_> = img.pixels().collect();
        let owned = ImgVec::new(pixels, img.width(), img.height());
        self.encode_pixel_data(&PixelData::GrayAlphaF32(owned))
    }

    /// Convenience: encode RGB8 animation with default job settings.
    ///
    /// Compositing fields on each [`TypedEncodeFrame`] (frame_rect, blend,
    /// disposal) are forwarded to the encoder.
    fn encode_animation_rgb8(
        &self,
        frames: &[TypedEncodeFrame<'_, Rgb<u8>>],
    ) -> Result<EncodeOutput, Self::Error> {
        let mut enc = self.job().frame_encoder()?;
        for frame in frames {
            enc.push_encode_frame(EncodeFrame {
                pixels: PixelSlice::from(frame.image),
                duration_ms: frame.duration_ms,
                frame_rect: frame.frame_rect,
                blend: frame.blend,
                disposal: frame.disposal,
            })?;
        }
        enc.finish()
    }

    /// Convenience: encode RGBA8 animation with default job settings.
    fn encode_animation_rgba8(
        &self,
        frames: &[TypedEncodeFrame<'_, Rgba<u8>>],
    ) -> Result<EncodeOutput, Self::Error> {
        let mut enc = self.job().frame_encoder()?;
        for frame in frames {
            enc.push_encode_frame(EncodeFrame {
                pixels: PixelSlice::from(frame.image),
                duration_ms: frame.duration_ms,
                frame_rect: frame.frame_rect,
                blend: frame.blend,
                disposal: frame.disposal,
            })?;
        }
        enc.finish()
    }

    /// Convenience: encode 16-bit RGB animation with default job settings.
    fn encode_animation_rgb16(
        &self,
        frames: &[TypedEncodeFrame<'_, Rgb<u16>>],
    ) -> Result<EncodeOutput, Self::Error> {
        let mut enc = self.job().frame_encoder()?;
        for frame in frames {
            enc.push_encode_frame(EncodeFrame {
                pixels: PixelSlice::from(frame.image),
                duration_ms: frame.duration_ms,
                frame_rect: frame.frame_rect,
                blend: frame.blend,
                disposal: frame.disposal,
            })?;
        }
        enc.finish()
    }

    /// Convenience: encode 16-bit RGBA animation with default job settings.
    fn encode_animation_rgba16(
        &self,
        frames: &[TypedEncodeFrame<'_, Rgba<u16>>],
    ) -> Result<EncodeOutput, Self::Error> {
        let mut enc = self.job().frame_encoder()?;
        for frame in frames {
            enc.push_encode_frame(EncodeFrame {
                pixels: PixelSlice::from(frame.image),
                duration_ms: frame.duration_ms,
                frame_rect: frame.frame_rect,
                blend: frame.blend,
                disposal: frame.disposal,
            })?;
        }
        enc.finish()
    }

    /// Convenience: encode from a [`PixelData`] value with default job settings.
    ///
    /// Dispatches to the correct typed encode method based on the variant.
    fn encode_pixel_data(&self, pixels: &PixelData) -> Result<EncodeOutput, Self::Error> {
        match pixels {
            PixelData::Rgb8(img) => self.encode_rgb8(img.as_ref()),
            PixelData::Rgba8(img) => self.encode_rgba8(img.as_ref()),
            PixelData::Gray8(img) => self.encode_gray8(img.as_ref()),
            PixelData::Bgra8(img) => self.encode_bgra8(img.as_ref()),
            PixelData::Rgb16(img) => self.encode_rgb16(img.as_ref()),
            PixelData::Rgba16(img) => self.encode_rgba16(img.as_ref()),
            PixelData::Gray16(img) => self.encode_gray16(img.as_ref()),
            PixelData::RgbF32(img) => self.encode_rgb_f32(img.as_ref()),
            PixelData::RgbaF32(img) => self.encode_rgba_f32(img.as_ref()),
            PixelData::GrayF32(img) => self.encode_gray_f32(img.as_ref()),
            PixelData::GrayAlpha8(img) => self.encode_gray_alpha8(img.as_ref()),
            PixelData::GrayAlpha16(img) => self.encode_gray_alpha16(img.as_ref()),
            PixelData::GrayAlphaF32(img) => self.encode_gray_alpha_f32(img.as_ref()),
        }
    }
}

/// Per-operation encode job.
///
/// Created by [`EncoderConfig::job()`]. Borrows temporary data (stop token,
/// metadata, resource limits) and produces either an [`Encoder`] (single image)
/// or a [`FrameEncoder`] (animation).
///
/// Every codec must accept a stop token and metadata. The codec embeds
/// whatever metadata the format supports and periodically checks the
/// stop token for cooperative cancellation.
///
/// Check [`EncoderConfig::capabilities()`] to see which metadata types and
/// cancellation are actually supported.
pub trait EncodeJob<'a>: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Single-image encoder type.
    type Encoder: Encoder<Error = Self::Error>;

    /// Animation encoder type.
    type FrameEncoder: FrameEncoder<Error = Self::Error>;

    /// Set cooperative cancellation token.
    ///
    /// The codec periodically calls `stop.check()` and returns an error
    /// if the operation should be cancelled. No-op if the codec doesn't
    /// support cancellation (check [`capabilities().encode_cancel()`](CodecCapabilities::encode_cancel)).
    fn with_stop(self, stop: &'a dyn Stop) -> Self;

    /// Set all metadata (ICC, EXIF, XMP) from a [`MetadataView`].
    ///
    /// The codec embeds whatever metadata the format supports. Metadata
    /// types not supported by the format are silently skipped — check
    /// [`capabilities()`](EncoderConfig::capabilities) to see what's supported.
    fn with_metadata(self, meta: &'a MetadataView<'a>) -> Self;

    /// Override resource limits for this operation.
    fn with_limits(self, limits: ResourceLimits) -> Self;

    /// Set the animation canvas dimensions.
    ///
    /// For compositing formats (GIF, APNG, WebP, JXL), individual frames
    /// can be smaller than the canvas. The canvas defines the logical
    /// screen / output dimensions. Call this before
    /// [`frame_encoder()`](EncodeJob::frame_encoder).
    ///
    /// For full-frame-replace formats (AVIF), this is ignored — the
    /// canvas matches the frame dimensions.
    ///
    /// Default no-op (canvas = first frame's dimensions).
    fn with_canvas_size(self, _width: u32, _height: u32) -> Self {
        self
    }

    /// Estimate the resource cost of encoding an image with these dimensions.
    ///
    /// Returns `input_bytes` and `pixel_count` (trivially computed from the
    /// arguments) plus an optional `peak_memory` estimate that accounts for
    /// this codec's quality/speed settings.
    ///
    /// Default: returns `input_bytes` and `pixel_count` with `peak_memory: None`.
    /// Override to provide codec-specific peak memory estimates.
    fn estimated_cost(
        &self,
        width: u32,
        height: u32,
        format: crate::PixelDescriptor,
    ) -> crate::EncodeCost {
        let input_bytes = width as u64 * height as u64 * format.bytes_per_pixel() as u64;
        crate::EncodeCost {
            input_bytes,
            pixel_count: width as u64 * height as u64,
            peak_memory: None,
        }
    }

    /// Create a one-shot/row-level encoder for a single image.
    fn encoder(self) -> Self::Encoder;

    /// Create a frame-by-frame encoder for animation.
    ///
    /// Returns an error if the codec does not support animation encoding.
    fn frame_encoder(self) -> Result<Self::FrameEncoder, Self::Error>;
}

/// Single-image encode: one-shot, row-level push, or pull-from-source.
///
/// Three mutually exclusive usage paths:
/// - [`encode()`](Encoder::encode) — all at once, consumes self
/// - [`push_rows()`](Encoder::push_rows) + [`finish()`](Encoder::finish) — caller pushes rows
/// - [`encode_from()`](Encoder::encode_from) — encoder pulls rows from a callback
///
/// Codecs that need full-frame data (e.g. AV1) may buffer internally
/// when rows are pushed or pulled incrementally.
pub trait Encoder: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Suggested strip height for optimal row-level encoding.
    ///
    /// For JPEG, typically the MCU height (8 or 16 rows).
    /// For PNG, typically 1 (row-at-a-time filtering).
    ///
    /// Returns 0 if the codec has no preference or doesn't support
    /// row-level encoding. This is advisory — callers may use any height.
    fn preferred_strip_height(&self) -> u32 {
        0
    }

    /// Encode a complete image at once (consumes self).
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;

    /// Push scanline rows incrementally.
    ///
    /// Codec may buffer internally if it needs full-frame data (e.g. AV1).
    ///
    /// Default: panics. Override if the codec supports row-level encoding.
    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), Self::Error> {
        unimplemented!("this codec does not support row-level encoding; use encode() instead")
    }

    /// Finalize after push_rows. Returns encoded output.
    ///
    /// Default: panics. Override if the codec supports row-level encoding.
    fn finish(self) -> Result<EncodeOutput, Self::Error> {
        unimplemented!("this codec does not support row-level encoding; use encode() instead")
    }

    /// Encode by pulling rows from a source callback.
    ///
    /// The encoder calls `source` repeatedly with the row index and a
    /// mutable buffer slice. The callback fills the buffer with pixel data
    /// for the requested rows and returns the number of rows written.
    /// Returns `0` to signal end of image.
    ///
    /// This is useful when pixel data is generated on-the-fly or comes
    /// from a source that produces rows in order (e.g., a render pipeline).
    ///
    /// Default: panics. Override if the codec supports pull encoding.
    fn encode_from(
        self,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, Self::Error> {
        unimplemented!("this codec does not support pull encoding; use encode() instead")
    }
}

/// Animation encode: push complete frames, build frames row-by-row, or
/// pull rows from a source.
///
/// Three mutually exclusive per-frame paths:
/// - [`push_frame()`](FrameEncoder::push_frame) /
///   [`push_encode_frame()`](FrameEncoder::push_encode_frame) — complete frame at once
/// - [`begin_frame()`](FrameEncoder::begin_frame) +
///   [`push_rows()`](FrameEncoder::push_rows) +
///   [`end_frame()`](FrameEncoder::end_frame) — caller pushes rows
/// - [`pull_frame()`](FrameEncoder::pull_frame) — encoder pulls rows from a callback
///
/// For sub-canvas frames with positioning and compositing, use
/// [`push_encode_frame()`](FrameEncoder::push_encode_frame) with an
/// [`EncodeFrame`] that has `frame_rect`, `blend`, and `disposal` set.
/// Set the canvas dimensions with
/// [`EncodeJob::with_canvas_size()`](EncodeJob::with_canvas_size)
/// before creating the frame encoder.
pub trait FrameEncoder: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Push a complete full-canvas frame.
    ///
    /// The pixel dimensions must match the canvas dimensions. For
    /// sub-canvas frames with positioning and compositing control,
    /// use [`push_encode_frame()`](FrameEncoder::push_encode_frame).
    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), Self::Error>;

    /// Push a frame with sub-canvas positioning and compositing.
    ///
    /// The [`EncodeFrame`] carries pixel data, duration, an optional
    /// `frame_rect` for sub-canvas positioning, and blend/disposal
    /// modes for compositing control.
    ///
    /// Default: ignores compositing fields, delegates to
    /// [`push_frame()`](FrameEncoder::push_frame). Compositing formats
    /// (GIF, APNG, WebP, JXL) should override this.
    fn push_encode_frame(&mut self, frame: EncodeFrame<'_>) -> Result<(), Self::Error> {
        self.push_frame(frame.pixels, frame.duration_ms)
    }

    /// Begin a new frame (for row-level building).
    ///
    /// Default: panics. Override if the codec supports row-level frame building.
    fn begin_frame(&mut self, _duration_ms: u32) -> Result<(), Self::Error> {
        unimplemented!(
            "this codec does not support row-level frame building; use push_frame() instead"
        )
    }

    /// Push rows into the current frame (after begin_frame).
    ///
    /// Default: panics. Override if the codec supports row-level frame building.
    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), Self::Error> {
        unimplemented!(
            "this codec does not support row-level frame building; use push_frame() instead"
        )
    }

    /// End the current frame (after pushing all rows).
    ///
    /// Default: panics. Override if the codec supports row-level frame building.
    fn end_frame(&mut self) -> Result<(), Self::Error> {
        unimplemented!(
            "this codec does not support row-level frame building; use push_frame() instead"
        )
    }

    /// Encode a frame by pulling rows from a source callback.
    ///
    /// The encoder calls `source` repeatedly with the row index and a
    /// mutable buffer slice. The callback fills the buffer and returns the
    /// number of rows written. Returns `0` to signal end of frame.
    ///
    /// Default: panics. Override if the codec supports pull frame building.
    fn pull_frame(
        &mut self,
        _duration_ms: u32,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<(), Self::Error> {
        unimplemented!(
            "this codec does not support pull frame building; use push_frame() instead"
        )
    }

    /// Set animation loop count.
    ///
    /// - `Some(0)` = loop forever
    /// - `Some(n)` = loop `n` times
    /// - `None` = format default (typically loop forever)
    ///
    /// Default no-op.
    fn with_loop_count(&mut self, _count: Option<u32>) {}

    /// Finalize animation. Returns encoded output.
    fn finish(self) -> Result<EncodeOutput, Self::Error>;
}

// ===========================================================================
// Decode traits (4)
// ===========================================================================

/// Common interface for decode configurations.
///
/// Implemented by each codec's config type (e.g. `zenjpeg::DecoderConfig`).
/// Config types are reusable (`Clone`) and have no lifetimes.
///
/// Format-specific decode settings live on the concrete config type.
/// The trait handles job creation, probing, and typed convenience methods.
pub trait DecoderConfig: Clone + Send + Sync {
    /// The image format this decoder handles (e.g. JPEG, WebP, AVIF).
    ///
    /// Static — the format is known at the type level.
    fn format() -> ImageFormat;

    /// Pixel formats this decoder can produce natively (without internal conversion).
    ///
    /// Every descriptor in this list is a guarantee: calling `decode_into()`
    /// with a `PixelSliceMut` matching one of these descriptors **must**
    /// produce correct output without any format conversion. The codec
    /// writes directly into the buffer.
    ///
    /// The decoder may also produce other formats via internal conversion
    /// in `decode()`, but `decode_into()` for a supported descriptor is the
    /// zero-overhead path.
    ///
    /// Must not be empty — every codec can natively produce at least one format.
    fn supported_descriptors() -> &'static [crate::PixelDescriptor];
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Per-operation job type, created by [`job()`](DecoderConfig::job).
    type Job<'a>: DecodeJob<'a, Error = Self::Error>
    where
        Self: 'a;

    /// Codec capabilities (metadata support, cancellation, probe cost, etc.).
    ///
    /// Returns a static reference describing what this codec supports.
    fn capabilities() -> &'static CodecCapabilities;

    /// Create a per-operation job for this config.
    fn job(&self) -> Self::Job<'_>;

    /// Probe image metadata cheaply (header parse only).
    ///
    /// This MUST be cheap — O(header), not O(pixels). Parses container
    /// headers to extract dimensions, format, and basic metadata. May not
    /// return frame counts or other data requiring a full parse.
    ///
    /// Use [`probe_full`](DecoderConfig::probe_full) when you need complete
    /// metadata including frame counts.
    fn probe_header(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;

    /// Probe image metadata with a full parse.
    ///
    /// May be expensive (e.g. parsing all GIF frames to count them, or
    /// decoding AVIF container metadata). Returns complete metadata
    /// including frame counts.
    ///
    /// Default: delegates to [`probe_header`](DecoderConfig::probe_header).
    /// Codecs that need a full parse for complete metadata should override.
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, Self::Error> {
        self.probe_header(data)
    }

    /// Convenience: decode with default job settings.
    fn decode(&self, data: &[u8]) -> Result<DecodeOutput, Self::Error> {
        self.job().decoder().decode(data)
    }

    /// Convenience: decode into a caller-provided RGB8 buffer.
    fn decode_into_rgb8(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, Rgb<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided RGBA8 buffer.
    fn decode_into_rgba8(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, Rgba<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided Gray8 buffer.
    fn decode_into_gray8(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, Gray<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided BGRA8 buffer.
    fn decode_into_bgra8(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, BGRA<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided BGRX8 buffer (alpha byte set to 255).
    fn decode_into_bgrx8(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, BGRA<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided RGB f32 buffer.
    fn decode_into_rgb_f32(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, Rgb<f32>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided RGBA f32 buffer.
    fn decode_into_rgba_f32(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, Rgba<f32>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided grayscale f32 buffer.
    fn decode_into_gray_f32(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, Gray<f32>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided 16-bit RGB buffer.
    fn decode_into_rgb16(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, Rgb<u16>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided 16-bit RGBA buffer.
    fn decode_into_rgba16(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, Rgba<u16>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }

    /// Convenience: decode into a caller-provided 16-bit grayscale buffer.
    fn decode_into_gray16(
        &self,
        data: &[u8],
        dst: ImgRefMut<'_, Gray<u16>>,
    ) -> Result<ImageInfo, Self::Error> {
        self.job()
            .decoder()
            .decode_into(data, PixelSliceMut::from(dst))
    }
}

/// Per-operation decode job.
///
/// Created by [`DecoderConfig::job()`]. Borrows temporary data (stop token,
/// resource limits) and produces either a [`Decoder`] (single image) or a
/// [`FrameDecoder`] (animation).
///
/// # Decode hints
///
/// Hints let the caller request spatial transforms (crop, scale, orientation)
/// that the decoder may apply during decode. The decoder is free to ignore
/// any hint, apply it partially (e.g., block-aligned crop), or apply it
/// fully. Call [`output_info()`](DecodeJob::output_info) after setting hints
/// to learn what the decoder will actually produce.
///
/// ```text
/// config.job()
///     .with_crop_hint(100, 100, 800, 600)   // request crop
///     .with_scale_hint(400, 300)             // request prescale
///     .with_orientation_hint(Rotate90)       // request orientation
///     .output_info(data)?                    // → what buffer to allocate
/// ```
pub trait DecodeJob<'a>: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Single-image decoder type.
    type Decoder: Decoder<Error = Self::Error>;

    /// Animation decoder type.
    type FrameDecoder: FrameDecoder<Error = Self::Error>;

    /// Set cooperative cancellation token.
    ///
    /// No-op if the codec doesn't support decode cancellation
    /// (check [`capabilities().decode_cancel()`](CodecCapabilities::decode_cancel)).
    fn with_stop(self, stop: &'a dyn Stop) -> Self;

    /// Override resource limits for this operation.
    fn with_limits(self, limits: ResourceLimits) -> Self;

    // --- Decode hints (optional, decoder may ignore) ---

    /// Hint: crop to this region in source coordinates.
    ///
    /// The decoder may adjust the crop for block alignment (JPEG MCU
    /// boundaries, etc.). Check [`OutputInfo::crop_applied`](crate::OutputInfo::crop_applied)
    /// to see what the decoder will actually do.
    ///
    /// Default: no crop (full image).
    fn with_crop_hint(self, _x: u32, _y: u32, _width: u32, _height: u32) -> Self {
        self
    }

    /// Hint: target output dimensions for prescaling.
    ///
    /// Some codecs can decode at reduced resolution cheaply (JPEG 1/2/4/8
    /// scaling, progressive JPEG XL). The decoder picks the closest
    /// resolution it can produce efficiently.
    ///
    /// Default: no scaling (native resolution).
    fn with_scale_hint(self, _width: u32, _height: u32) -> Self {
        self
    }

    /// Hint: apply this orientation during decode.
    ///
    /// If the decoder handles orientation, the output pixels are already
    /// rotated/flipped and [`OutputInfo::orientation_applied`](crate::OutputInfo::orientation_applied)
    /// reflects what was applied. If ignored, orientation remains for
    /// the caller to handle.
    ///
    /// Default: no orientation handling.
    fn with_orientation_hint(self, _orientation: crate::Orientation) -> Self {
        self
    }

    // --- Output prediction ---

    /// Predict the decode output given current config and hints.
    ///
    /// Returns [`OutputInfo`](crate::OutputInfo) describing the width, height,
    /// pixel format, and applied transforms. **This is what your destination
    /// buffer must match.**
    ///
    /// Call after setting hints, before creating a decoder. The returned
    /// info accounts for crop, scale, and orientation hints that the
    /// decoder will honor.
    fn output_info(&self, data: &[u8]) -> Result<crate::OutputInfo, Self::Error>;

    // --- Cost estimation ---

    /// Estimate the resource cost of this decode.
    ///
    /// Returns output buffer size, pixel count, and optionally peak memory.
    /// Accounts for current hints (crop, scale, etc.).
    ///
    /// Default: derives `output_bytes` and `pixel_count` from
    /// [`output_info()`](DecodeJob::output_info) with `peak_memory: None`.
    /// Override to provide codec-specific peak memory estimates.
    fn estimated_cost(&self, data: &[u8]) -> Result<crate::DecodeCost, Self::Error> {
        let info = self.output_info(data)?;
        Ok(crate::DecodeCost {
            output_bytes: info.buffer_size(),
            pixel_count: info.pixel_count(),
            peak_memory: None,
        })
    }

    // --- Executor creation ---

    /// Create a one-shot decoder.
    fn decoder(self) -> Self::Decoder;

    /// Create a frame-by-frame decoder. Parses container upfront.
    ///
    /// Returns an error if the codec does not support animation decoding
    /// or if the container parse fails.
    fn frame_decoder(self, data: &[u8]) -> Result<Self::FrameDecoder, Self::Error>;
}

/// One-shot decode: all pixels at once, into a caller buffer, or row-level callback.
pub trait Decoder: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Decode to owned pixels (codec picks native format).
    fn decode(self, data: &[u8]) -> Result<DecodeOutput, Self::Error>;

    /// Decode into caller-provided buffer.
    ///
    /// The buffer dimensions must match [`DecodeJob::output_info()`].
    /// The buffer's pixel format must be
    /// one of [`DecoderConfig::supported_descriptors()`].
    fn decode_into(self, data: &[u8], dst: PixelSliceMut<'_>) -> Result<ImageInfo, Self::Error>;

    /// Decode with row-level streaming into a caller-owned buffer.
    ///
    /// The codec calls [`DecodeRowSink::demand()`](crate::DecodeRowSink::demand)
    /// for each strip. The sink returns `(buffer, stride)` and the codec
    /// writes `width × bpp` pixel bytes per row at stride offsets — no
    /// intermediate allocation or copy.
    ///
    /// For streaming codecs (JPEG baseline, PNG), rows are produced
    /// incrementally. For full-frame codecs (WebP, AVIF, GIF), the codec
    /// decodes internally then writes strips to the sink.
    ///
    /// Pixels are written in the format from
    /// [`DecodeJob::output_info()`](crate::DecodeJob::output_info).
    /// The sink controls row stride (may be padded for SIMD alignment).
    ///
    /// # Default implementation
    ///
    /// Calls [`decode()`](Decoder::decode), then copies rows to the sink.
    /// Codecs with true streaming decode (JPEG baseline) should override
    /// for zero-copy performance.
    fn decode_rows(
        self,
        data: &[u8],
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<ImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let desc = output.descriptor();
        let bpp = desc.bytes_per_pixel();
        let w = output.width();
        let h = output.height();
        let row_bytes = w as usize * bpp;

        // Try zero-copy path (works for all formats except GrayAlpha)
        if let Some(slice) = output.pixels().as_pixel_slice() {
            for y in 0..h {
                let (buf, _stride) = sink.demand(y, 1, w, bpp);
                buf[..row_bytes].copy_from_slice(slice.row(y));
            }
        } else {
            // Fallback for GrayAlpha: to_bytes() always works
            let bytes = output.pixels().to_bytes();
            for y in 0..h {
                let (buf, _stride) = sink.demand(y, 1, w, bpp);
                let src_start = y as usize * row_bytes;
                buf[..row_bytes].copy_from_slice(&bytes[src_start..src_start + row_bytes]);
            }
        }

        Ok(output.info().clone())
    }
}

/// Streaming animation decode: pull frames or push rows via callback.
pub trait FrameDecoder: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Number of frames, if known without decoding.
    ///
    /// Some formats (GIF, APNG) require a full parse to count frames.
    /// Returns `None` if unknown or if counting requires decoding.
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

    /// Pull next complete frame. Returns `None` when done.
    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, Self::Error>;

    /// Pull next frame into caller buffer.
    ///
    /// If `prior_frame` is `Some(n)`, the buffer already contains frame `n`'s
    /// composited result, enabling efficient incremental compositing.
    /// Pass `None` when the buffer does not contain a valid prior frame.
    ///
    /// Returns `None` when done.
    fn next_frame_into(
        &mut self,
        dst: PixelSliceMut<'_>,
        prior_frame: Option<u32>,
    ) -> Result<Option<ImageInfo>, Self::Error>;
}
