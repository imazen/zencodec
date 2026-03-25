//! Object-safe layered decode traits — zero-generics codec-agnostic dispatch.
//!
//! Mirrors the generic decode hierarchy with dyn-safe traits:
//!
//!   DynDecoderConfig → DynDecodeJob → DynDecoder / DynAnimationFrameDecoder / DynStreamingDecoder
//!
//! Each layer is a separate trait with blanket impls via private shim structs.
//! Every method from the generic traits is exposed.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use core::any::Any;

use crate::format::ImageFormat;
use crate::orientation::OrientationHint;
use crate::output::OwnedAnimationFrame;
use crate::{DecodeCapabilities, DecodeOutput, ImageInfo, OutputInfo, ResourceLimits, StopToken};
use enough::Stop;
use zenpixels::{PixelDescriptor, PixelSlice};

use super::BoxedError;
use super::decoder::{AnimationFrameDecoder, Decode, StreamingDecode};
use super::decoding::{DecodeJob, DecoderConfig};

// ===========================================================================
// DynDecoder
// ===========================================================================

/// Object-safe one-shot decoder.
///
/// Wraps [`Decode`] for dyn dispatch. Produced by
/// [`DynDecodeJob::into_decoder`].
pub trait DynDecoder {
    /// Decode to owned pixels (consumes self).
    fn decode(self: Box<Self>) -> Result<DecodeOutput, BoxedError>;
}

impl core::fmt::Debug for dyn DynDecoder + '_ {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynDecoder").finish_non_exhaustive()
    }
}

pub(super) struct DecoderShim<D>(pub(super) D);

impl<D: Decode> DynDecoder for DecoderShim<D> {
    fn decode(self: Box<Self>) -> Result<DecodeOutput, BoxedError> {
        self.0.decode().map_err(|e| Box::new(e) as BoxedError)
    }
}

// ===========================================================================
// DynAnimationFrameDecoder
// ===========================================================================

/// Object-safe full-frame animation decoder.
///
/// Wraps [`AnimationFrameDecoder`] for dyn dispatch. Produced by
/// [`DynDecodeJob::into_animation_frame_decoder`].
///
/// # Downcasting
///
/// Use [`as_any()`](DynAnimationFrameDecoder::as_any) to downcast back to the
/// concrete codec type for format-specific animation controls.
pub trait DynAnimationFrameDecoder: Send {
    /// Downcast to the concrete frame decoder type.
    fn as_any(&self) -> &dyn Any;

    /// Downcast to the concrete frame decoder type (mutable).
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Consume and downcast to the concrete frame decoder type.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;

    /// Image metadata, available after construction.
    fn info(&self) -> &ImageInfo;

    /// Number of frames, if known without decoding.
    fn frame_count(&self) -> Option<u32>;

    /// Animation loop count from the container.
    fn loop_count(&self) -> Option<u32>;

    /// Render the next frame as an owned copy.
    fn render_next_frame_owned(
        &mut self,
        stop: Option<&dyn Stop>,
    ) -> Result<Option<OwnedAnimationFrame>, BoxedError>;

    /// Render the next frame directly into a caller-owned sink.
    fn render_next_frame_to_sink(
        &mut self,
        stop: Option<&dyn Stop>,
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, BoxedError>;
}

impl core::fmt::Debug for dyn DynAnimationFrameDecoder + '_ {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynAnimationFrameDecoder")
            .finish_non_exhaustive()
    }
}

pub(super) struct AnimationFrameDecoderShim<F>(pub(super) F);

impl<F: AnimationFrameDecoder + Send + 'static> DynAnimationFrameDecoder
    for AnimationFrameDecoderShim<F>
{
    fn as_any(&self) -> &dyn Any {
        &self.0
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut self.0
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        Box::new(self.0)
    }

    fn info(&self) -> &ImageInfo {
        self.0.info()
    }

    fn frame_count(&self) -> Option<u32> {
        self.0.frame_count()
    }

    fn loop_count(&self) -> Option<u32> {
        self.0.loop_count()
    }

    fn render_next_frame_owned(
        &mut self,
        stop: Option<&dyn Stop>,
    ) -> Result<Option<OwnedAnimationFrame>, BoxedError> {
        self.0
            .render_next_frame_owned(stop)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn render_next_frame_to_sink(
        &mut self,
        stop: Option<&dyn Stop>,
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, BoxedError> {
        self.0
            .render_next_frame_to_sink(stop, sink)
            .map_err(|e| Box::new(e) as BoxedError)
    }
}

// ===========================================================================
// DynStreamingDecoder
// ===========================================================================

/// Object-safe streaming scanline-batch decoder.
///
/// Wraps [`StreamingDecode`] for dyn dispatch. Produced by
/// [`DynDecodeJob::into_streaming_decoder`].
pub trait DynStreamingDecoder: Send {
    /// Pull the next batch of scanlines.
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, BoxedError>;

    /// Image metadata, available after construction.
    fn info(&self) -> &ImageInfo;
}

impl core::fmt::Debug for dyn DynStreamingDecoder + '_ {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynStreamingDecoder")
            .finish_non_exhaustive()
    }
}

pub(super) struct StreamingDecoderShim<S>(pub(super) S);

impl<S: StreamingDecode + Send> DynStreamingDecoder for StreamingDecoderShim<S> {
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, BoxedError> {
        self.0.next_batch().map_err(|e| Box::new(e) as BoxedError)
    }

    fn info(&self) -> &ImageInfo {
        self.0.info()
    }
}

// ===========================================================================
// DynDecodeJob
// ===========================================================================

/// Object-safe decode job.
///
/// Wraps [`DecodeJob`] for dyn dispatch. Produced by
/// [`DynDecoderConfig::dyn_job`]. Use the `set_*` methods to configure,
/// then call one of the `into_*` methods to create a decoder.
pub trait DynDecodeJob<'a> {
    /// Set cooperative cancellation token.
    fn set_stop(&mut self, stop: StopToken);

    /// Override resource limits.
    fn set_limits(&mut self, limits: ResourceLimits);

    /// Set decode security policy.
    fn set_policy(&mut self, policy: crate::DecodePolicy);

    /// Probe image metadata without decoding pixels (header parse).
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, BoxedError>;

    /// Probe image metadata with a full parse.
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, BoxedError>;

    /// Hint: crop to this region in source coordinates.
    fn set_crop_hint(&mut self, x: u32, y: u32, width: u32, height: u32);

    /// Set orientation handling strategy.
    fn set_orientation(&mut self, hint: OrientationHint);

    /// Hint: start decoding from a specific frame (0-based).
    fn set_start_frame_index(&mut self, index: u32);

    /// Access codec-specific extensions for this job.
    ///
    /// Returns a reference to a `'static` extension type stored inside the
    /// concrete job. Downcast to the codec's extension type to access
    /// codec-specific configuration or alternate decode paths.
    fn extensions(&self) -> Option<&dyn Any>;

    /// Mutable access to codec-specific extensions.
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>;

    /// Predict what the decoder will produce given current hints.
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, BoxedError>;

    /// Create a one-shot decoder bound to `data` (consumes this job).
    fn into_decoder(
        self: Box<Self>,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynDecoder + 'a>, BoxedError>;

    /// Decode into a caller-owned sink (consumes this job).
    fn push_decode(
        self: Box<Self>,
        data: Cow<'a, [u8]>,
        sink: &mut dyn crate::DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, BoxedError>;

    /// Create a streaming decoder (consumes this job).
    fn into_streaming_decoder(
        self: Box<Self>,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError>;

    /// Create a full-frame animation decoder (consumes this job).
    ///
    /// The returned decoder is `'static` — it owns all its data.
    /// Pass `Cow::Owned(vec)` to avoid a copy.
    fn into_animation_frame_decoder(
        self: Box<Self>,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynAnimationFrameDecoder>, BoxedError>;
}

struct DecodeJobShim<J>(Option<J>);

impl<J> DecodeJobShim<J> {
    fn take(&mut self) -> J {
        self.0.take().expect("job already consumed")
    }

    fn put(&mut self, job: J) {
        self.0 = Some(job);
    }

    fn as_ref(&self) -> &J {
        self.0.as_ref().expect("job already consumed")
    }
}

impl<'a, J> DynDecodeJob<'a> for DecodeJobShim<J>
where
    J: DecodeJob<'a> + 'a,
    J::StreamDec: Send,
    J::AnimationFrameDec: Send,
{
    fn set_stop(&mut self, stop: StopToken) {
        let job = self.take();
        self.put(job.with_stop(stop));
    }

    fn set_limits(&mut self, limits: ResourceLimits) {
        let job = self.take();
        self.put(job.with_limits(limits));
    }

    fn set_policy(&mut self, policy: crate::DecodePolicy) {
        let job = self.take();
        self.put(job.with_policy(policy));
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, BoxedError> {
        self.as_ref()
            .probe(data)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, BoxedError> {
        self.as_ref()
            .probe_full(data)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn set_crop_hint(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let job = self.take();
        self.put(job.with_crop_hint(x, y, width, height));
    }

    fn set_orientation(&mut self, hint: OrientationHint) {
        let job = self.take();
        self.put(job.with_orientation(hint));
    }

    fn set_start_frame_index(&mut self, index: u32) {
        let job = self.take();
        self.put(job.with_start_frame_index(index));
    }

    fn extensions(&self) -> Option<&dyn Any> {
        self.0.as_ref().and_then(|j| j.extensions())
    }

    fn extensions_mut(&mut self) -> Option<&mut dyn Any> {
        self.0.as_mut().and_then(|j| j.extensions_mut())
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, BoxedError> {
        self.as_ref()
            .output_info(data)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn into_decoder(
        mut self: Box<Self>,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynDecoder + 'a>, BoxedError> {
        let job = self.take();
        let dec = job
            .decoder(data, preferred)
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(DecoderShim(dec)))
    }

    fn push_decode(
        mut self: Box<Self>,
        data: Cow<'a, [u8]>,
        sink: &mut dyn crate::DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, BoxedError> {
        let job = self.take();
        job.push_decoder(data, sink, preferred)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn into_streaming_decoder(
        mut self: Box<Self>,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError> {
        let job = self.take();
        let dec = job
            .streaming_decoder(data, preferred)
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(StreamingDecoderShim(dec)))
    }

    fn into_animation_frame_decoder(
        mut self: Box<Self>,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynAnimationFrameDecoder>, BoxedError> {
        let job = self.take();
        let dec = job
            .animation_frame_decoder(data, preferred)
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(AnimationFrameDecoderShim(dec)))
    }
}

// ===========================================================================
// DynDecoderConfig
// ===========================================================================

/// Object-safe decoder configuration.
///
/// Blanket-implemented for all [`DecoderConfig`] types. Enables fully
/// codec-agnostic decode with no generic parameters.
///
/// ```rust,ignore
/// fn load(config: &dyn DynDecoderConfig, data: &[u8]) -> Result<DecodeOutput, BoxedError> {
///     config.dyn_job().into_decoder(Cow::Borrowed(data), &[])?.decode()
/// }
///
/// let jpeg = JpegDecoderConfig::new();
/// let webp = WebpDecoderConfig::new();
/// let img = load(&jpeg, &jpeg_bytes)?;
/// let img = load(&webp, &webp_bytes)?;
/// ```
pub trait DynDecoderConfig: Send + Sync {
    /// Downcast to the concrete config type.
    ///
    /// ```rust,ignore
    /// let config: &dyn DynDecoderConfig = &JpegDecoderConfig::new();
    /// let jpeg = config.as_any().downcast_ref::<JpegDecoderConfig>().unwrap();
    /// ```
    fn as_any(&self) -> &dyn Any;

    /// The image formats this decoder handles.
    fn formats(&self) -> &'static [ImageFormat];

    /// Pixel formats this decoder can produce natively.
    fn supported_descriptors(&self) -> &'static [PixelDescriptor];

    /// Decoder capabilities (metadata support, cancellation, etc.).
    fn capabilities(&self) -> &'static DecodeCapabilities;

    /// Create a dyn-dispatched decode job.
    fn dyn_job(&self) -> Box<dyn DynDecodeJob<'_> + '_>;
}

impl<C> DynDecoderConfig for C
where
    C: DecoderConfig + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn formats(&self) -> &'static [ImageFormat] {
        C::formats()
    }

    fn supported_descriptors(&self) -> &'static [PixelDescriptor] {
        C::supported_descriptors()
    }

    fn capabilities(&self) -> &'static DecodeCapabilities {
        C::capabilities()
    }

    fn dyn_job(&self) -> Box<dyn DynDecodeJob<'_> + '_> {
        Box::new(DecodeJobShim(Some(DecoderConfig::job(self))))
    }
}
