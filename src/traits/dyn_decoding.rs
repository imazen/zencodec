//! Object-safe layered decode traits — zero-generics codec-agnostic dispatch.
//!
//! Mirrors the generic decode hierarchy with dyn-safe traits:
//!
//!   DynDecoderConfig → DynDecodeJob → DynDecoder / DynFrameDecoder / DynStreamingDecoder
//!
//! Each layer is a separate trait with blanket impls via private shim structs.
//! Every method from the generic traits is exposed.

use alloc::boxed::Box;

use crate::format::ImageFormat;
use crate::orientation::OrientationHint;
use crate::{
    DecodeCapabilities, DecodeFrame, DecodeOutput, ImageInfo, OutputInfo, ResourceLimits, Stop,
};
use zenpixels::{PixelDescriptor, PixelSlice};

use super::decoder::{Decode, FrameDecode, StreamingDecode};
use super::decoding::{DecodeJob, DecoderConfig};
use super::BoxedError;

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

pub(super) struct DecoderShim<D>(pub(super) D);

impl<D: Decode> DynDecoder for DecoderShim<D> {
    fn decode(self: Box<Self>) -> Result<DecodeOutput, BoxedError> {
        self.0.decode().map_err(|e| Box::new(e) as BoxedError)
    }
}

// ===========================================================================
// DynFrameDecoder
// ===========================================================================

/// Object-safe animation decoder.
///
/// Wraps [`FrameDecode`] for dyn dispatch. Produced by
/// [`DynDecodeJob::into_frame_decoder`].
pub trait DynFrameDecoder {
    /// Number of frames, if known without decoding.
    fn frame_count(&self) -> Option<u32>;

    /// Animation loop count from the container.
    fn loop_count(&self) -> Option<u32>;

    /// Pull next frame. Returns `None` when all frames consumed.
    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, BoxedError>;

    /// Decode next frame directly into a caller-owned sink (push model).
    fn next_frame_to_sink(
        &mut self,
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, BoxedError>;
}

pub(super) struct FrameDecoderShim<F>(pub(super) F);

impl<F: FrameDecode> DynFrameDecoder for FrameDecoderShim<F> {
    fn frame_count(&self) -> Option<u32> {
        self.0.frame_count()
    }

    fn loop_count(&self) -> Option<u32> {
        self.0.loop_count()
    }

    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, BoxedError> {
        self.0.next_frame().map_err(|e| Box::new(e) as BoxedError)
    }

    fn next_frame_to_sink(
        &mut self,
        sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, BoxedError> {
        self.0
            .next_frame_to_sink(sink)
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
pub trait DynStreamingDecoder {
    /// Pull the next batch of scanlines.
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, BoxedError>;

    /// Image metadata, available after construction.
    fn info(&self) -> &ImageInfo;
}

pub(super) struct StreamingDecoderShim<S>(pub(super) S);

impl<S: StreamingDecode> DynStreamingDecoder for StreamingDecoderShim<S> {
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
    fn set_stop(&mut self, stop: &'a dyn Stop);

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

    /// Hint: target output dimensions for prescaling.
    fn set_scale_hint(&mut self, max_width: u32, max_height: u32);

    /// Set orientation handling strategy.
    fn set_orientation(&mut self, hint: OrientationHint);

    /// Predict what the decoder will produce given current hints.
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, BoxedError>;

    /// Create a one-shot decoder bound to `data` (consumes this job).
    fn into_decoder(
        self: Box<Self>,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynDecoder + 'a>, BoxedError>;

    /// Decode into a caller-owned sink (consumes this job).
    fn push_decode(
        self: Box<Self>,
        data: &'a [u8],
        sink: &mut dyn crate::DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, BoxedError>;

    /// Create a streaming decoder (consumes this job).
    fn into_streaming_decoder(
        self: Box<Self>,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError>;

    /// Create a frame-by-frame animation decoder (consumes this job).
    fn into_frame_decoder(
        self: Box<Self>,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynFrameDecoder + 'a>, BoxedError>;
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
{
    fn set_stop(&mut self, stop: &'a dyn Stop) {
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

    fn set_scale_hint(&mut self, max_width: u32, max_height: u32) {
        let job = self.take();
        self.put(job.with_scale_hint(max_width, max_height));
    }

    fn set_orientation(&mut self, hint: OrientationHint) {
        let job = self.take();
        self.put(job.with_orientation(hint));
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, BoxedError> {
        self.as_ref()
            .output_info(data)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn into_decoder(
        mut self: Box<Self>,
        data: &'a [u8],
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
        data: &'a [u8],
        sink: &mut dyn crate::DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, BoxedError> {
        let job = self.take();
        job.push_decoder(data, sink, preferred)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn into_streaming_decoder(
        mut self: Box<Self>,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError> {
        let job = self.take();
        let dec = job
            .streaming_decoder(data, preferred)
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(StreamingDecoderShim(dec)))
    }

    fn into_frame_decoder(
        mut self: Box<Self>,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynFrameDecoder + 'a>, BoxedError> {
        let job = self.take();
        let dec = job
            .frame_decoder(data, preferred)
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(FrameDecoderShim(dec)))
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
///     config.dyn_job().into_decoder(data, &[])?.decode()
/// }
///
/// let jpeg = JpegDecoderConfig::new();
/// let webp = WebpDecoderConfig::new();
/// let img = load(&jpeg, &jpeg_bytes)?;
/// let img = load(&webp, &webp_bytes)?;
/// ```
pub trait DynDecoderConfig: Send + Sync {
    /// The image format this decoder handles.
    fn format(&self) -> ImageFormat;

    /// Pixel formats this decoder can produce natively.
    fn supported_descriptors(&self) -> &'static [PixelDescriptor];

    /// Decoder capabilities (metadata support, cancellation, etc.).
    fn capabilities(&self) -> &'static DecodeCapabilities;

    /// Create a dyn-dispatched decode job.
    fn dyn_job(&self) -> Box<dyn DynDecodeJob<'_> + '_>;
}

impl<C> DynDecoderConfig for C
where
    C: DecoderConfig,
{
    fn format(&self) -> ImageFormat {
        C::format()
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
