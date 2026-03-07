//! Object-safe layered encode traits — zero-generics codec-agnostic dispatch.
//!
//! Mirrors the generic encode hierarchy with dyn-safe traits:
//!
//!   DynEncoderConfig → DynEncodeJob → DynEncoder / DynFrameEncoder
//!
//! Each layer is a separate trait with blanket impls via private shim structs.
//! Every method from the generic traits is exposed.
//!
//! ```rust,ignore
//! fn save(config: &dyn DynEncoderConfig, data: &[u8], w: u32, h: u32) -> Result<Vec<u8>, BoxedError> {
//!     let mut job = config.dyn_job();
//!     job.set_metadata(&meta);
//!     job.set_limits(limits);
//!     let encoder = job.into_encoder()?;
//!     let output = encoder.encode_srgba8(data, true, w, h, w)?;
//!     Ok(output.into_vec())
//! }
//! ```

use alloc::boxed::Box;

use crate::format::ImageFormat;
use crate::{EncodeCapabilities, EncodeFrame, EncodeOutput, MetadataView, ResourceLimits, Stop};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

use super::encoder::{Encoder, FrameEncoder};
use super::encoding::{EncodeJob, EncoderConfig};
use super::BoxedError;

// ===========================================================================
// DynEncoder
// ===========================================================================

/// Object-safe single-image encoder.
///
/// Wraps [`Encoder`] for dyn dispatch. Produced by
/// [`DynEncodeJob::into_encoder`].
pub trait DynEncoder {
    /// Suggested strip height for optimal row-level encoding.
    fn preferred_strip_height(&self) -> u32;

    /// Encode a complete image from type-erased pixels (consumes self).
    fn encode(self: Box<Self>, pixels: PixelSlice<'_>) -> Result<EncodeOutput, BoxedError>;

    /// Encode from sRGB RGBA8 raw bytes (consumes self).
    ///
    /// The buffer is mutable — the encoder may modify it in-place for
    /// format adaptation. See [`Encoder::encode_srgba8`] for details.
    fn encode_srgba8(
        self: Box<Self>,
        data: &mut [u8],
        make_opaque: bool,
        width: u32,
        height: u32,
        stride_pixels: u32,
    ) -> Result<EncodeOutput, BoxedError>;

    /// Push scanline rows incrementally.
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), BoxedError>;

    /// Finalize after push_rows. Returns encoded output.
    fn finish(self: Box<Self>) -> Result<EncodeOutput, BoxedError>;

    /// Encode by pulling rows from a source callback.
    fn encode_from(
        self: Box<Self>,
        source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, BoxedError>;
}

pub(super) struct EncoderShim<E>(pub(super) E);

impl<E: Encoder> DynEncoder for EncoderShim<E> {
    fn preferred_strip_height(&self) -> u32 {
        self.0.preferred_strip_height()
    }

    fn encode(self: Box<Self>, pixels: PixelSlice<'_>) -> Result<EncodeOutput, BoxedError> {
        self.0.encode(pixels).map_err(|e| Box::new(e) as BoxedError)
    }

    fn encode_srgba8(
        self: Box<Self>,
        data: &mut [u8],
        make_opaque: bool,
        width: u32,
        height: u32,
        stride_pixels: u32,
    ) -> Result<EncodeOutput, BoxedError> {
        self.0
            .encode_srgba8(data, make_opaque, width, height, stride_pixels)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), BoxedError> {
        self.0
            .push_rows(rows)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn finish(self: Box<Self>) -> Result<EncodeOutput, BoxedError> {
        self.0.finish().map_err(|e| Box::new(e) as BoxedError)
    }

    fn encode_from(
        self: Box<Self>,
        source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, BoxedError> {
        self.0
            .encode_from(source)
            .map_err(|e| Box::new(e) as BoxedError)
    }
}

// ===========================================================================
// DynFrameEncoder
// ===========================================================================

/// Object-safe animation encoder.
///
/// Wraps [`FrameEncoder`] for dyn dispatch. Produced by
/// [`DynEncodeJob::into_frame_encoder`].
pub trait DynFrameEncoder {
    /// Push a complete full-canvas frame.
    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), BoxedError>;

    /// Push a frame with sub-canvas positioning and compositing.
    fn push_encode_frame(&mut self, frame: EncodeFrame<'_>) -> Result<(), BoxedError>;

    /// Begin a new frame (for row-level building).
    fn begin_frame(&mut self, duration_ms: u32) -> Result<(), BoxedError>;

    /// Push rows into the current frame.
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), BoxedError>;

    /// End the current frame.
    fn end_frame(&mut self) -> Result<(), BoxedError>;

    /// Encode a frame by pulling rows from a source callback.
    fn pull_frame(
        &mut self,
        duration_ms: u32,
        source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<(), BoxedError>;

    /// Set animation loop count.
    fn set_loop_count(&mut self, count: Option<u32>);

    /// Finalize animation. Returns encoded output.
    fn finish(self: Box<Self>) -> Result<EncodeOutput, BoxedError>;
}

pub(super) struct FrameEncoderShim<F>(pub(super) F);

impl<F: FrameEncoder> DynFrameEncoder for FrameEncoderShim<F> {
    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), BoxedError> {
        self.0
            .push_frame(pixels, duration_ms)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn push_encode_frame(&mut self, frame: EncodeFrame<'_>) -> Result<(), BoxedError> {
        self.0
            .push_encode_frame(frame)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn begin_frame(&mut self, duration_ms: u32) -> Result<(), BoxedError> {
        self.0
            .begin_frame(duration_ms)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), BoxedError> {
        self.0
            .push_rows(rows)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn end_frame(&mut self) -> Result<(), BoxedError> {
        self.0.end_frame().map_err(|e| Box::new(e) as BoxedError)
    }

    fn pull_frame(
        &mut self,
        duration_ms: u32,
        source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<(), BoxedError> {
        self.0
            .pull_frame(duration_ms, source)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn set_loop_count(&mut self, count: Option<u32>) {
        self.0.with_loop_count(count);
    }

    fn finish(self: Box<Self>) -> Result<EncodeOutput, BoxedError> {
        self.0.finish().map_err(|e| Box::new(e) as BoxedError)
    }
}

// ===========================================================================
// DynEncodeJob
// ===========================================================================

/// Object-safe encode job.
///
/// Wraps [`EncodeJob`] for dyn dispatch. Produced by
/// [`DynEncoderConfig::dyn_job`]. Use the `set_*` methods to configure,
/// then call [`into_encoder`](DynEncodeJob::into_encoder) or
/// [`into_frame_encoder`](DynEncodeJob::into_frame_encoder).
pub trait DynEncodeJob<'a> {
    /// Set cooperative cancellation token.
    fn set_stop(&mut self, stop: &'a dyn Stop);

    /// Override resource limits.
    fn set_limits(&mut self, limits: ResourceLimits);

    /// Set encode security policy.
    fn set_policy(&mut self, policy: crate::EncodePolicy);

    /// Set metadata (ICC, EXIF, XMP) to embed.
    fn set_metadata(&mut self, meta: &'a MetadataView<'a>);

    /// Set animation canvas dimensions.
    fn set_canvas_size(&mut self, width: u32, height: u32);

    /// Set animation loop count.
    fn set_loop_count(&mut self, count: Option<u32>);

    /// Create the single-image encoder (consumes this job).
    fn into_encoder(self: Box<Self>) -> Result<Box<dyn DynEncoder + 'a>, BoxedError>;

    /// Create the animation encoder (consumes this job).
    fn into_frame_encoder(self: Box<Self>) -> Result<Box<dyn DynFrameEncoder + 'a>, BoxedError>;
}

struct EncodeJobShim<J>(Option<J>);

impl<J> EncodeJobShim<J> {
    fn take(&mut self) -> J {
        self.0.take().expect("job already consumed")
    }

    fn put(&mut self, job: J) {
        self.0 = Some(job);
    }
}

impl<'a, J> DynEncodeJob<'a> for EncodeJobShim<J>
where
    J: EncodeJob<'a> + 'a,
    J::Enc: Encoder,
    J::FrameEnc: FrameEncoder,
{
    fn set_stop(&mut self, stop: &'a dyn Stop) {
        let job = self.take();
        self.put(job.with_stop(stop));
    }

    fn set_limits(&mut self, limits: ResourceLimits) {
        let job = self.take();
        self.put(job.with_limits(limits));
    }

    fn set_policy(&mut self, policy: crate::EncodePolicy) {
        let job = self.take();
        self.put(job.with_policy(policy));
    }

    fn set_metadata(&mut self, meta: &'a MetadataView<'a>) {
        let job = self.take();
        self.put(job.with_metadata(meta));
    }

    fn set_canvas_size(&mut self, width: u32, height: u32) {
        let job = self.take();
        self.put(job.with_canvas_size(width, height));
    }

    fn set_loop_count(&mut self, count: Option<u32>) {
        let job = self.take();
        self.put(job.with_loop_count(count));
    }

    fn into_encoder(mut self: Box<Self>) -> Result<Box<dyn DynEncoder + 'a>, BoxedError> {
        let job = self.take();
        let enc = job.encoder().map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(EncoderShim(enc)))
    }

    fn into_frame_encoder(
        mut self: Box<Self>,
    ) -> Result<Box<dyn DynFrameEncoder + 'a>, BoxedError> {
        let job = self.take();
        let enc = job.frame_encoder().map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(FrameEncoderShim(enc)))
    }
}

// ===========================================================================
// DynEncoderConfig
// ===========================================================================

/// Object-safe encoder configuration.
///
/// Blanket-implemented for all [`EncoderConfig`] types whose encoder
/// implements [`Encoder`] and frame encoder implements [`FrameEncoder`].
/// Codecs without animation support should set `type FrameEnc = ()`.
///
/// ```rust,ignore
/// fn save(config: &dyn DynEncoderConfig, pixels: &[u8], w: u32, h: u32) -> Result<Vec<u8>, BoxedError> {
///     let encoder = config.dyn_job().into_encoder()?;
///     encoder.encode_srgba8(pixels, true, w, h, w)
///         .map(|o| o.into_vec())
/// }
///
/// let jpeg = JpegEncoderConfig::new().with_generic_quality(85.0);
/// let webp = WebpEncoderConfig::lossy();
/// save(&jpeg, &pixels, 100, 100)?;
/// save(&webp, &pixels, 100, 100)?;
/// ```
pub trait DynEncoderConfig: Send + Sync {
    /// The image format this encoder produces.
    fn format(&self) -> ImageFormat;

    /// Pixel formats this encoder accepts natively.
    fn supported_descriptors(&self) -> &'static [PixelDescriptor];

    /// Encoder capabilities (metadata support, cancellation, etc.).
    fn capabilities(&self) -> &'static EncodeCapabilities;

    /// Create a dyn-dispatched encode job.
    fn dyn_job(&self) -> Box<dyn DynEncodeJob<'_> + '_>;
}

impl<C> DynEncoderConfig for C
where
    C: EncoderConfig,
    for<'a> <C::Job<'a> as EncodeJob<'a>>::Enc: Encoder,
    for<'a> <C::Job<'a> as EncodeJob<'a>>::FrameEnc: FrameEncoder,
{
    fn format(&self) -> ImageFormat {
        C::format()
    }

    fn supported_descriptors(&self) -> &'static [PixelDescriptor] {
        C::supported_descriptors()
    }

    fn capabilities(&self) -> &'static EncodeCapabilities {
        C::capabilities()
    }

    fn dyn_job(&self) -> Box<dyn DynEncodeJob<'_> + '_> {
        Box::new(EncodeJobShim(Some(EncoderConfig::job(self))))
    }
}
