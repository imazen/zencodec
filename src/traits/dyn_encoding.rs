//! Object-safe layered encode traits — zero-generics codec-agnostic dispatch.
//!
//! Mirrors the generic encode hierarchy with dyn-safe traits:
//!
//!   DynEncoderConfig → DynEncodeJob → DynEncoder / DynFullFrameEncoder
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
use core::any::Any;

use crate::format::ImageFormat;
use crate::{EncodeCapabilities, EncodeOutput, MetadataView, ResourceLimits};
use enough::Stop;
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

use super::BoxedError;
use super::encoder::{Encoder, FullFrameEncoder};
use super::encoding::{EncodeJob, EncoderConfig};

// ===========================================================================
// DynEncoder
// ===========================================================================

/// Object-safe single-image encoder.
///
/// Wraps [`Encoder`] for dyn dispatch. Produced by
/// [`DynEncodeJob::into_encoder`].
///
/// Encoders may borrow job-scoped data (stop tokens, metadata) so they
/// are not guaranteed `'static`. Attach codec-specific output data via
/// [`EncodeOutput::with_extras`](crate::EncodeOutput::with_extras) instead
/// of downcasting.
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

impl core::fmt::Debug for dyn DynEncoder + '_ {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynEncoder").finish_non_exhaustive()
    }
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
// DynFullFrameEncoder
// ===========================================================================

/// Object-safe full-frame animation encoder.
///
/// Wraps [`FullFrameEncoder`] for dyn dispatch. Produced by
/// [`DynEncodeJob::into_full_frame_encoder`].
///
/// # Downcasting
///
/// Use [`as_any()`](DynFullFrameEncoder::as_any) to downcast back to the
/// concrete codec type for format-specific animation controls.
pub trait DynFullFrameEncoder {
    /// Downcast to the concrete frame encoder type.
    fn as_any(&self) -> &dyn Any;

    /// Downcast to the concrete frame encoder type (mutable).
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Consume and downcast to the concrete frame encoder type.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;

    /// Push a complete full-canvas frame.
    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        stop: Option<&dyn Stop>,
    ) -> Result<(), BoxedError>;

    /// Finalize animation. Returns encoded output.
    fn finish(self: Box<Self>, stop: Option<&dyn Stop>) -> Result<EncodeOutput, BoxedError>;
}

impl core::fmt::Debug for dyn DynFullFrameEncoder + '_ {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynFullFrameEncoder")
            .finish_non_exhaustive()
    }
}

pub(super) struct FullFrameEncoderShim<F>(pub(super) F);

impl<F: FullFrameEncoder + 'static> DynFullFrameEncoder for FullFrameEncoderShim<F> {
    fn as_any(&self) -> &dyn Any {
        &self.0
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut self.0
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        Box::new(self.0)
    }

    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        stop: Option<&dyn Stop>,
    ) -> Result<(), BoxedError> {
        self.0
            .push_frame(pixels, duration_ms, stop)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn finish(self: Box<Self>, stop: Option<&dyn Stop>) -> Result<EncodeOutput, BoxedError> {
        self.0.finish(stop).map_err(|e| Box::new(e) as BoxedError)
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
/// [`into_full_frame_encoder`](DynEncodeJob::into_full_frame_encoder).
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

    /// Create the full-frame animation encoder (consumes this job).
    ///
    /// The returned encoder is `'static` — it owns its configuration.
    fn into_full_frame_encoder(self: Box<Self>)
    -> Result<Box<dyn DynFullFrameEncoder>, BoxedError>;
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
    J::FullFrameEnc: FullFrameEncoder,
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

    fn into_full_frame_encoder(
        mut self: Box<Self>,
    ) -> Result<Box<dyn DynFullFrameEncoder>, BoxedError> {
        let job = self.take();
        let enc = job
            .full_frame_encoder()
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(FullFrameEncoderShim(enc)))
    }
}

// ===========================================================================
// DynEncoderConfig
// ===========================================================================

/// Object-safe encoder configuration.
///
/// Blanket-implemented for all [`EncoderConfig`] types whose encoder
/// implements [`Encoder`] and full-frame encoder implements [`FullFrameEncoder`].
/// Codecs without animation support should set `type FullFrameEnc = ()`.
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
    /// Downcast to the concrete config type.
    ///
    /// ```rust,ignore
    /// let config: &dyn DynEncoderConfig = &JpegConfig::new();
    /// let jpeg = config.as_any().downcast_ref::<JpegConfig>().unwrap();
    /// ```
    fn as_any(&self) -> &dyn Any;

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
    C: EncoderConfig + 'static,
    for<'a> <C::Job<'a> as EncodeJob<'a>>::Enc: Encoder,
    for<'a> <C::Job<'a> as EncodeJob<'a>>::FullFrameEnc: FullFrameEncoder,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

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
