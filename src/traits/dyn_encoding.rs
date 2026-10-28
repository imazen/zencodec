//! Object-safe layered encode traits — zero-generics codec-agnostic dispatch.
//!
//! Mirrors the generic encode hierarchy with dyn-safe traits:
//!
//!   DynEncoderConfig → DynEncodeJob → DynEncoder / DynAnimationFrameEncoder
//!
//! Each layer is a separate trait with blanket impls via private shim structs.
//! Every method from the generic traits is exposed.
//!
//! ```rust,ignore
//! fn save(config: &dyn DynEncoderConfig, data: &[u8], w: u32, h: u32) -> Result<Vec<u8>, BoxedError> {
//!     let mut job = config.dyn_job();
//!     job.set_metadata_policy(meta, MetadataPolicy::Web);
//!     job.set_limits(limits);
//!     let encoder = job.into_encoder()?;
//!     let output = encoder.encode_srgba8(data, true, w, h, w)?;
//!     Ok(output.into_vec())
//! }
//! ```

use alloc::boxed::Box;
use core::any::Any;

use crate::StopToken;
use crate::estimate::{ComputeEnvironment, ImageCharacteristics, ResourceEstimate};
use crate::format::ImageFormat;
use crate::{EncodeCapabilities, EncodeOutput, Metadata, ResourceLimits};

use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

use super::BoxedError;
use super::encoder::{AnimationFrameEncoder, Encoder};
use super::encoding::{EncodeJob, EncoderConfig};

// ===========================================================================
// DynEncoder
// ===========================================================================

/// Object-safe single-image encoder.
///
/// Wraps [`Encoder`] for dyn dispatch. Produced by
/// [`DynEncodeJob::into_encoder`].
///
/// Encoders may borrow job-scoped data (pulse tokens, metadata) so they
/// are not guaranteed `'static`. Attach codec-specific output data via
/// [`EncodeOutput::with_extras`](crate::EncodeOutput::with_extras) instead
/// of downcasting.
pub trait DynEncoder: Send {
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

    /// Declare the image before the first `push_rows` and learn when its
    /// bytes arrive; see [`Encoder::begin`](super::Encoder::begin).
    /// The default refuses with [`crate::UnsupportedOperation::RowLevelEncode`],
    /// leaving one-shot encoding available.
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<crate::encode::OutputTiming, BoxedError> {
        let _ = (width, height, descriptor);
        Err(Box::new(crate::UnsupportedOperation::RowLevelEncode))
    }

    /// Start the complete admitted declaration; see `Encoder::begin_declared`.
    /// Direct implementations default to explicit refusal, without legacy startup.
    fn begin_declared(
        &mut self,
        _declaration: &crate::encode::EncodeDeclaration<'_>,
    ) -> Result<crate::encode::EncodePlan, BoxedError> {
        Err(Box::new(crate::UnsupportedOperation::EncodePlanning))
    }

    /// Encoder-driven row acquisition; see [`Encoder::pull_rows`].
    /// The default refuses before calling the source or changing state.
    fn pull_rows(
        &mut self,
        _rows: &mut dyn crate::encode::EncodeRows,
        _pulse: &dyn crate::Pulse,
    ) -> Result<crate::encode::PullProgress, BoxedError> {
        Err(Box::new(crate::UnsupportedOperation::PullEncode))
    }

    /// Prepare actual destination access; see [`Encoder::begin_output`].
    fn begin_output(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
        access: crate::output_patch::OutputAccess,
    ) -> Result<crate::encode::OutputTiming, BoxedError> {
        let _ = access;
        self.begin(width, height, descriptor)
    }
    /// Complete negotiated bytes and patches; see [`Encoder::finish_output`].
    fn finish_output(
        self: Box<Self>,
    ) -> Result<crate::output_patch::OutputFinish<EncodeOutput>, BoxedError> {
        Ok(crate::output_patch::OutputFinish::new(
            self.finish()?,
            alloc::vec::Vec::new(),
        ))
    }

    /// Encoded bytes ready to send; see
    /// [`Encoder::pending_output`](super::Encoder::pending_output).
    fn pending_output(&self) -> &[u8] {
        &[]
    }

    /// Mark pending bytes as sent; see
    /// [`Encoder::consume_output`](super::Encoder::consume_output).
    fn consume_output(&mut self, n: usize) {
        let _ = n;
    }

    /// Append pending bytes to `into` and consume them; see
    /// [`Encoder::take_output`](super::Encoder::take_output).
    fn take_output(&mut self, into: &mut alloc::vec::Vec<u8>) {
        let pending = self.pending_output();
        let n = pending.len();
        into.extend_from_slice(pending);
        self.consume_output(n);
    }

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

impl<E: Encoder + Send> DynEncoder for EncoderShim<E> {
    fn pull_rows(
        &mut self,
        rows: &mut dyn crate::encode::EncodeRows,
        pulse: &dyn crate::Pulse,
    ) -> Result<crate::encode::PullProgress, BoxedError> {
        self.0
            .pull_rows(rows, pulse)
            .map_err(|error| Box::new(error) as BoxedError)
    }

    fn begin_declared(
        &mut self,
        declaration: &crate::encode::EncodeDeclaration<'_>,
    ) -> Result<crate::encode::EncodePlan, BoxedError> {
        self.0
            .begin_declared(declaration)
            .map_err(|error| Box::new(error) as BoxedError)
    }

    fn begin_output(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
        access: crate::output_patch::OutputAccess,
    ) -> Result<crate::encode::OutputTiming, BoxedError> {
        self.0
            .begin_output(width, height, descriptor, access)
            .map_err(|e| Box::new(e) as BoxedError)
    }
    fn finish_output(
        self: Box<Self>,
    ) -> Result<crate::output_patch::OutputFinish<EncodeOutput>, BoxedError> {
        self.0
            .finish_output()
            .map_err(|e| Box::new(e) as BoxedError)
    }

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

    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<crate::encode::OutputTiming, BoxedError> {
        self.0
            .begin(width, height, descriptor)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn pending_output(&self) -> &[u8] {
        self.0.pending_output()
    }

    fn consume_output(&mut self, n: usize) {
        self.0.consume_output(n)
    }

    fn take_output(&mut self, into: &mut alloc::vec::Vec<u8>) {
        self.0.take_output(into)
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
// DynAnimationFrameEncoder
// ===========================================================================

/// Object-safe full-frame animation encoder.
///
/// Wraps [`AnimationFrameEncoder`] for dyn dispatch. Produced by
/// [`DynEncodeJob::into_animation_frame_encoder`].
///
/// # Downcasting
///
/// Use [`as_any()`](DynAnimationFrameEncoder::as_any) to downcast back to the
/// concrete codec type for format-specific animation controls.
pub trait DynAnimationFrameEncoder: Send {
    /// Downcast to the concrete frame encoder type.
    fn as_any(&self) -> &dyn Any;

    /// Downcast to the concrete frame encoder type (mutable).
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Consume and downcast to the concrete frame encoder type.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;

    /// Declare the animation and learn when its bytes arrive; see
    /// [`AnimationFrameEncoder::begin`].
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
        frame_count: Option<u32>,
    ) -> Result<crate::encode::AnimationOutputTiming, BoxedError> {
        let _ = (width, height, descriptor, frame_count);
        Ok(crate::encode::AnimationOutputTiming::AT_FINISH)
    }

    /// Encoded bytes ready to send; see
    /// [`AnimationFrameEncoder::pending_output`].
    fn pending_output(&self) -> &[u8] {
        &[]
    }

    /// Mark pending bytes as sent; see
    /// [`AnimationFrameEncoder::consume_output`].
    fn consume_output(&mut self, n: usize) {
        let _ = n;
    }

    /// Append pending bytes to `into` and consume them; see
    /// [`AnimationFrameEncoder::take_output`].
    fn take_output(&mut self, into: &mut alloc::vec::Vec<u8>) {
        let pending = self.pending_output();
        let n = pending.len();
        into.extend_from_slice(pending);
        self.consume_output(n);
    }

    /// Push a complete full-canvas frame.
    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), BoxedError>;

    /// Push with destination-aware rounding and return the actual duration.
    /// See [`AnimationFrameEncoder::push_frame_timed`].
    fn push_frame_timed(
        &mut self,
        pixels: PixelSlice<'_>,
        duration: crate::animation::FrameDuration,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<crate::animation::FrameDuration, BoxedError> {
        self.push_frame_exact(pixels, duration, pulse)?;
        Ok(duration)
    }

    /// Push an exact source duration, or reject without silently quantizing it.
    /// See [`AnimationFrameEncoder::push_frame_exact`].
    fn push_frame_exact(
        &mut self,
        _pixels: PixelSlice<'_>,
        _duration: crate::animation::FrameDuration,
        _pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), BoxedError> {
        Err(Box::new(crate::UnsupportedOperation::AnimationTiming))
    }

    /// Finalize animation. Returns encoded output.
    fn finish(
        self: Box<Self>,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<EncodeOutput, BoxedError>;
}

impl core::fmt::Debug for dyn DynAnimationFrameEncoder + '_ {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynAnimationFrameEncoder")
            .finish_non_exhaustive()
    }
}

pub(super) struct AnimationFrameEncoderShim<F>(pub(super) F);

impl<F: AnimationFrameEncoder + Send + 'static> DynAnimationFrameEncoder
    for AnimationFrameEncoderShim<F>
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

    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
        frame_count: Option<u32>,
    ) -> Result<crate::encode::AnimationOutputTiming, BoxedError> {
        self.0
            .begin(width, height, descriptor, frame_count)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn pending_output(&self) -> &[u8] {
        self.0.pending_output()
    }

    fn consume_output(&mut self, n: usize) {
        self.0.consume_output(n)
    }

    fn take_output(&mut self, into: &mut alloc::vec::Vec<u8>) {
        self.0.take_output(into)
    }

    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), BoxedError> {
        self.0
            .push_frame(pixels, duration_ms, pulse)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn push_frame_timed(
        &mut self,
        pixels: PixelSlice<'_>,
        duration: crate::animation::FrameDuration,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<crate::animation::FrameDuration, BoxedError> {
        self.0
            .push_frame_timed(pixels, duration, pulse)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn push_frame_exact(
        &mut self,
        pixels: PixelSlice<'_>,
        duration: crate::animation::FrameDuration,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<(), BoxedError> {
        self.0
            .push_frame_exact(pixels, duration, pulse)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn finish(
        self: Box<Self>,
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<EncodeOutput, BoxedError> {
        self.0.finish(pulse).map_err(|e| Box::new(e) as BoxedError)
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
/// [`into_animation_frame_encoder`](DynEncodeJob::into_animation_frame_encoder).
pub trait DynEncodeJob {
    /// Plan the retained effective job before native startup. Generated shims
    /// forward the concrete job's implementation, retaining original errors.
    fn plan_encode(
        &self,
        _declaration: &crate::encode::EncodeDeclaration<'_>,
    ) -> Result<crate::encode::EncodePlan, BoxedError> {
        Err(Box::new(crate::UnsupportedOperation::EncodePlanning))
    }

    /// Set cooperative cancellation token.
    #[deprecated(note = "use set_pulse; wrap cancellation-only controls in how_far::StopOnly")]
    fn set_stop(&mut self, pulse: StopToken) {
        self.set_pulse(how_far::SharedPulse::new(how_far::StopOnly::new(pulse)));
    }

    /// Retain all operation control capabilities through type erasure.
    fn set_pulse(&mut self, pulse: how_far::SharedPulse);

    /// Override resource limits.
    fn set_limits(&mut self, limits: ResourceLimits);

    /// Legacy best-effort encode policy; unsupported fields may be ignored.
    #[deprecated(
        note = "may silently ignore policy; use try_set_policy or a checked operation request"
    )]
    fn set_policy(&mut self, policy: crate::EncodePolicy);

    /// Apply a required policy or return a recoverable rejection.
    ///
    /// Manual implementations default to accepting only the empty policy;
    /// generated shims delegate to the concrete job's `try_set_policy`.
    /// A rejection must leave the job unchanged.
    fn try_set_policy(&mut self, policy: crate::EncodePolicy) -> Result<(), BoxedError> {
        if policy != crate::EncodePolicy::none() {
            return Err(Box::new(crate::UnsupportedOperation::EncodePolicy));
        }
        Ok(())
    }

    /// Require exact physical density or unknown-unit aspect. Refusal leaves
    /// the job unchanged. Generated shims call the concrete checked setter.
    fn try_set_resolution(&mut self, resolution: crate::Resolution) -> Result<(), BoxedError> {
        resolution.validate()?;
        Err(Box::new(crate::ResolutionError::Unsupported))
    }

    /// Set metadata to embed, filtered by an explicit retention policy (blessed
    /// path; mirrors [`EncodeJob::with_metadata_policy`](crate::encode::EncodeJob::with_metadata_policy)).
    ///
    /// Provided method: filters via [`Metadata::filtered`](crate::Metadata::filtered)
    /// and routes through [`set_metadata`](Self::set_metadata). Has a default so
    /// adding it is not a breaking change for downstream implementors; the blanket
    /// impl overrides it to filter before the job is consumed.
    fn set_metadata_policy(&mut self, meta: Metadata, policy: crate::MetadataPolicy) {
        #[allow(deprecated)]
        self.set_metadata(meta.filtered(&policy));
    }

    /// Set metadata (ICC, EXIF, XMP) to embed, without a retention policy. Prefer
    /// [`set_metadata_policy`](Self::set_metadata_policy) so retention is explicit.
    #[deprecated(note = "embeds metadata without an explicit retention policy; use \
                set_metadata_policy(meta, policy)")]
    fn set_metadata(&mut self, meta: Metadata);

    /// Set animation canvas dimensions.
    fn set_canvas_size(&mut self, width: u32, height: u32);

    /// Set animation loop count.
    fn set_loop_count(&mut self, count: Option<u32>);

    /// Access codec-specific extensions for this job.
    ///
    /// Returns a reference to a `'static` extension type stored inside the
    /// concrete job. Downcast to the codec's extension type to access
    /// codec-specific configuration or alternate encode paths.
    fn extensions(&self) -> Option<&dyn Any>;

    /// Mutable access to codec-specific extensions.
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>;

    /// One-shot: encode `pixels` with this job's configuration and return the
    /// output — the common tail after `set_metadata_policy` / `set_limits` /
    /// `set_policy`. Equivalent to `self.into_encoder()?.encode(pixels)`.
    ///
    /// Provided method (adding it broke no implementor). For streaming row-push
    /// or animation, take the encoder yourself with
    /// [`into_encoder`](Self::into_encoder) /
    /// [`into_animation_frame_encoder`](Self::into_animation_frame_encoder).
    fn encode(self: Box<Self>, pixels: PixelSlice<'_>) -> Result<EncodeOutput, BoxedError> {
        self.into_encoder()?.encode(pixels)
    }

    /// Create the single-image encoder (consumes this job).
    fn into_encoder(self: Box<Self>) -> Result<Box<dyn DynEncoder>, BoxedError>;

    /// Create the full-frame animation encoder (consumes this job).
    ///
    /// The returned encoder is `'static` — it owns its configuration.
    fn into_animation_frame_encoder(
        self: Box<Self>,
    ) -> Result<Box<dyn DynAnimationFrameEncoder>, BoxedError>;
}

pub(crate) fn erase_send_job<J>(job: J) -> Box<dyn DynEncodeJob + Send>
where
    J: EncodeJob + Send + 'static,
    J::Enc: Encoder + Send,
    J::AnimationFrameEnc: AnimationFrameEncoder,
{
    Box::new(EncodeJobShim(Some(job)))
}

struct EncodeJobShim<J>(Option<J>);

impl<J> EncodeJobShim<J> {
    fn take(&mut self) -> Result<J, BoxedError> {
        self.0
            .take()
            .ok_or_else(|| "EncodeJobShim: job already consumed (double take)".into())
    }

    /// Apply `f` to the inner job if it has not been consumed.
    ///
    /// `DynEncodeJob` setters return `()` for ergonomics and backwards
    /// compatibility, so a setter call after the inner job has been
    /// consumed by an `into_*` method has no return path. We
    /// `debug_assert!` here so the misuse fires in tests and dev builds;
    /// in release the call still silently no-ops, but any subsequent
    /// `into_*` call will return the "job already consumed" error so the
    /// problem surfaces at the next observable boundary.
    fn try_apply<F: FnOnce(J) -> J>(&mut self, f: F) {
        match self.0.take() {
            Some(job) => self.0 = Some(f(job)),
            None => {
                debug_assert!(
                    false,
                    "DynEncodeJob setter called after the inner job was consumed by an into_* method; the call has no effect"
                );
            }
        }
    }
}

impl<J> DynEncodeJob for EncodeJobShim<J>
where
    J: EncodeJob,
    J::Enc: Encoder + Send,
    J::AnimationFrameEnc: AnimationFrameEncoder,
{
    fn plan_encode(
        &self,
        declaration: &crate::encode::EncodeDeclaration<'_>,
    ) -> Result<crate::encode::EncodePlan, BoxedError> {
        let Some(job) = self.0.as_ref() else {
            return Err("EncodeJobShim: job already consumed (double take)".into());
        };
        job.plan_encode(declaration)
    }

    fn set_pulse(&mut self, pulse: how_far::SharedPulse) {
        self.try_apply(|job| job.with_pulse(pulse));
    }

    fn set_limits(&mut self, limits: ResourceLimits) {
        self.try_apply(|job| job.with_limits(limits));
    }

    fn set_policy(&mut self, policy: crate::EncodePolicy) {
        // Preserve the legacy setter behavior; checked callers use try_set_policy.
        #[allow(deprecated)]
        self.try_apply(|job| job.with_policy(policy));
    }

    fn try_set_policy(&mut self, policy: crate::EncodePolicy) -> Result<(), BoxedError> {
        let Some(job) = self.0.as_mut() else {
            return Err("EncodeJobShim: job already consumed (double take)".into());
        };
        job.try_set_policy(policy)
            .map_err(|error| Box::new(error) as BoxedError)
    }

    fn set_metadata_policy(&mut self, meta: Metadata, policy: crate::MetadataPolicy) {
        self.try_apply(|job| job.with_metadata_policy(meta, policy));
    }

    fn try_set_resolution(&mut self, resolution: crate::Resolution) -> Result<(), BoxedError> {
        let Some(job) = self.0.as_mut() else {
            return Err("EncodeJobShim: job already consumed (double take)".into());
        };
        job.try_set_resolution(resolution)
            .map_err(|error| Box::new(error) as BoxedError)
    }

    #[allow(deprecated)]
    fn set_metadata(&mut self, meta: Metadata) {
        self.try_apply(|job| job.with_metadata(meta));
    }

    fn set_canvas_size(&mut self, width: u32, height: u32) {
        self.try_apply(|job| job.with_canvas_size(width, height));
    }

    fn set_loop_count(&mut self, count: Option<u32>) {
        self.try_apply(|job| job.with_loop_count(count));
    }

    fn extensions(&self) -> Option<&dyn Any> {
        self.0.as_ref().and_then(|j| j.extensions())
    }

    fn extensions_mut(&mut self) -> Option<&mut dyn Any> {
        self.0.as_mut().and_then(|j| j.extensions_mut())
    }

    fn into_encoder(mut self: Box<Self>) -> Result<Box<dyn DynEncoder>, BoxedError> {
        let job = self.take()?;
        let enc = job.encoder().map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(EncoderShim(enc)))
    }

    fn into_animation_frame_encoder(
        mut self: Box<Self>,
    ) -> Result<Box<dyn DynAnimationFrameEncoder>, BoxedError> {
        let job = self.take()?;
        let enc = job
            .animation_frame_encoder()
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(AnimationFrameEncoderShim(enc)))
    }
}

// ===========================================================================
// DynEncoderConfig
// ===========================================================================

/// Object-safe encoder configuration.
///
/// Blanket-implemented for all [`EncoderConfig`] types whose encoder
/// implements [`Encoder`] and full-frame encoder implements [`AnimationFrameEncoder`].
/// Codecs without animation support should set `type AnimationFrameEnc = ()`.
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

    /// Predict peak memory / time / core-scaling for encoding `image` on
    /// `compute` (forwards to
    /// [`EncoderConfig::estimate_encode_resources`](crate::encode::EncoderConfig::estimate_encode_resources)).
    fn estimate_encode_resources(
        &self,
        image: &ImageCharacteristics,
        compute: &ComputeEnvironment,
    ) -> ResourceEstimate {
        // Default mirrors `EncoderConfig::estimate_encode_resources` —
        // `unknown()` (every field `None`). The blanket impl over
        // `EncoderConfig` overrides this with the codec's real estimate; the
        // default keeps adding this method semver-additive for any direct
        // implementor of the dyn trait.
        let _ = (image, compute);
        ResourceEstimate::unknown()
    }

    /// Confirm a target without encoding pixels. Direct dyn implementations
    /// default to unreported support; blanket implementations share the generic
    /// checked configuration path.
    fn check_fidelity_target(
        &self,
        target: crate::encode::Fidelity,
    ) -> Result<(), crate::encode::FidelityTargetError> {
        target.validate_target()?;
        Err(crate::encode::FidelityTargetError::Unreported)
    }

    /// Create a job configured for a confirmed target. This does not guarantee
    /// content-dependent losslessness or a measured perceptual score.
    fn try_fidelity_job(
        &self,
        target: crate::encode::Fidelity,
    ) -> Result<Box<dyn DynEncodeJob>, crate::encode::FidelityTargetError> {
        target.validate_target()?;
        Err(crate::encode::FidelityTargetError::Unreported)
    }

    /// Return the first confirmed preference and its job; never retries encoding.
    fn select_fidelity_job(
        &self,
        preferences: &[crate::encode::Fidelity],
    ) -> Result<(usize, Box<dyn DynEncodeJob>), crate::encode::FidelitySelectionError> {
        crate::fidelity::select_target(preferences, |target| self.try_fidelity_job(target))
    }

    /// Create a dyn-dispatched encode job.
    ///
    /// The job owns its config (cloned). The `'static` bound means
    /// the job can outlive the config reference — the only remaining
    /// lifetime dependency is the pulse control (set via `set_pulse`).
    fn dyn_job(&self) -> Box<dyn DynEncodeJob + 'static>;
}

impl<C> DynEncoderConfig for C
where
    C: EncoderConfig + 'static,
    <C::Job as EncodeJob>::Enc: Encoder + Send,
    <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
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

    fn estimate_encode_resources(
        &self,
        image: &ImageCharacteristics,
        compute: &ComputeEnvironment,
    ) -> ResourceEstimate {
        EncoderConfig::estimate_encode_resources(self, image, compute)
    }

    fn check_fidelity_target(
        &self,
        target: crate::encode::Fidelity,
    ) -> Result<(), crate::encode::FidelityTargetError> {
        EncoderConfig::check_fidelity_target(self, target)
    }

    fn try_fidelity_job(
        &self,
        target: crate::encode::Fidelity,
    ) -> Result<Box<dyn DynEncodeJob>, crate::encode::FidelityTargetError> {
        Ok(Box::new(EncodeJobShim(Some(
            self.clone().try_with_fidelity(target)?.job(),
        ))))
    }

    fn dyn_job(&self) -> Box<dyn DynEncodeJob + 'static> {
        Box::new(EncodeJobShim(Some(self.clone().job())))
    }
}

#[cfg(test)]
mod begin_dispatch_tests {
    use super::*;
    use crate::UnsupportedOperation;

    // Exercise both defaults independently: ordinary erased encoders delegate
    // through EncoderShim, but callers may also implement DynEncoder directly.
    struct OneShot;

    impl Encoder for OneShot {
        type Error = UnsupportedOperation;

        fn reject(op: UnsupportedOperation) -> Self::Error {
            op
        }

        fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
            // Only a dispatch fixture; this test does not encode an image.
            Ok(EncodeOutput::new(alloc::vec::Vec::new(), ImageFormat::Png))
        }
    }

    impl DynEncoder for OneShot {
        fn preferred_strip_height(&self) -> u32 {
            Encoder::preferred_strip_height(self)
        }

        fn encode(self: Box<Self>, pixels: PixelSlice<'_>) -> Result<EncodeOutput, BoxedError> {
            Encoder::encode(*self, pixels).map_err(Into::into)
        }

        fn encode_srgba8(
            self: Box<Self>,
            data: &mut [u8],
            make_opaque: bool,
            width: u32,
            height: u32,
            stride_pixels: u32,
        ) -> Result<EncodeOutput, BoxedError> {
            Encoder::encode_srgba8(*self, data, make_opaque, width, height, stride_pixels)
                .map_err(Into::into)
        }

        fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), BoxedError> {
            Encoder::push_rows(self, rows).map_err(Into::into)
        }

        fn finish(self: Box<Self>) -> Result<EncodeOutput, BoxedError> {
            Encoder::finish(*self).map_err(Into::into)
        }

        fn encode_from(
            self: Box<Self>,
            source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
        ) -> Result<EncodeOutput, BoxedError> {
            Encoder::encode_from(*self, source).map_err(Into::into)
        }
    }

    struct NoFill;
    impl crate::encode::EncodeRows for NoFill {
        fn fill(
            &mut self,
            _: crate::encode::RowDemand<'_>,
        ) -> Result<(), crate::encode::RowFillError> {
            panic!("unsupported pull must never call the source")
        }
    }

    #[test]
    fn default_pull_refuses_before_source_for_typed_shim_and_direct_dyn() {
        let mut typed = OneShot;
        assert_eq!(
            Encoder::pull_rows(&mut typed, &mut NoFill, &crate::how_far::NoPulse),
            Err(UnsupportedOperation::PullEncode)
        );
        for mut erased in [
            Box::new(EncoderShim(OneShot)) as Box<dyn DynEncoder>,
            Box::new(OneShot) as Box<dyn DynEncoder>,
        ] {
            let error = erased
                .pull_rows(&mut NoFill, &crate::how_far::NoPulse)
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<UnsupportedOperation>(),
                Some(&UnsupportedOperation::PullEncode)
            );
            let pixels = zenpixels::PixelBuffer::new(1, 1, PixelDescriptor::RGB8_SRGB);
            assert!(erased.encode(pixels.as_slice()).is_ok());
        }
    }

    #[test]
    fn direct_dyn_declared_start_refuses_without_losing_one_shot() {
        let mut encoder: Box<dyn DynEncoder> = Box::new(OneShot);
        let declaration = crate::encode::EncodeDeclaration::new(1, 1, PixelDescriptor::RGB8_SRGB);
        let error = encoder.begin_declared(&declaration).unwrap_err();
        assert_eq!(
            error.downcast_ref::<UnsupportedOperation>(),
            Some(&UnsupportedOperation::EncodePlanning)
        );
        assert!(encoder.pending_output().is_empty());
        let pixels = zenpixels::PixelBuffer::new(1, 1, PixelDescriptor::RGB8_SRGB);
        assert!(encoder.encode(pixels.as_slice()).is_ok());
    }

    #[test]
    fn default_begin_refuses_rows_for_typed_shim_and_direct_dyn_encoders() {
        let pixels = zenpixels::PixelBuffer::new(1, 1, PixelDescriptor::RGB8_SRGB);
        let mut typed = OneShot;
        assert_eq!(
            Encoder::begin(&mut typed, 1, 1, PixelDescriptor::RGB8_SRGB),
            Err(UnsupportedOperation::RowLevelEncode)
        );
        assert!(Encoder::encode(typed, pixels.as_slice()).is_ok());

        let encoders: [Box<dyn DynEncoder>; 2] =
            [Box::new(EncoderShim(OneShot)), Box::new(OneShot)];
        for mut encoder in encoders {
            let error = encoder
                .begin(1, 1, PixelDescriptor::RGB8_SRGB)
                .expect_err("one-shot default must refuse before accepting any rows");
            assert_eq!(
                error.downcast_ref::<UnsupportedOperation>(),
                Some(&UnsupportedOperation::RowLevelEncode)
            );
            assert!(encoder.pending_output().is_empty());
            assert!(encoder.encode(pixels.as_slice()).is_ok());
        }
    }
}

#[cfg(test)]
mod timing_dispatch_tests {
    use super::*;
    use crate::{
        UnsupportedOperation,
        animation::{FrameDuration, TimingQuantizer},
    };

    struct Timed {
        quantizer: TimingQuantizer,
        accepted: alloc::vec::Vec<u8>,
    }
    impl Timed {
        fn new() -> Self {
            Self {
                quantizer: TimingQuantizer::new(100, 255).unwrap(),
                accepted: alloc::vec::Vec::new(),
            }
        }
    }
    impl AnimationFrameEncoder for Timed {
        type Error = UnsupportedOperation;
        fn reject(op: UnsupportedOperation) -> Self::Error {
            op
        }
        fn push_frame(
            &mut self,
            _: PixelSlice<'_>,
            _: u32,
            _: Option<&dyn crate::Pulse>,
        ) -> Result<(), Self::Error> {
            panic!("rational dispatch must not call the legacy path")
        }
        fn push_frame_exact(
            &mut self,
            _: PixelSlice<'_>,
            duration: FrameDuration,
            _: Option<&dyn crate::Pulse>,
        ) -> Result<(), Self::Error> {
            let ticks = duration
                .ticks_at(100)
                .ok()
                .and_then(|ticks| u8::try_from(ticks).ok())
                .ok_or(UnsupportedOperation::AnimationTiming)?;
            self.accepted.push(ticks);
            Ok(())
        }
        fn push_frame_timed(
            &mut self,
            pixels: PixelSlice<'_>,
            duration: FrameDuration,
            pulse: Option<&dyn crate::Pulse>,
        ) -> Result<FrameDuration, Self::Error> {
            let mut next = self.quantizer;
            let encoded = next
                .quantize(duration)
                .map_err(|_| UnsupportedOperation::AnimationTiming)?;
            self.push_frame_exact(pixels, encoded, pulse)?;
            self.quantizer = next;
            Ok(encoded)
        }
        fn finish(self, _: Option<&dyn crate::Pulse>) -> Result<EncodeOutput, Self::Error> {
            Ok(EncodeOutput::new(self.accepted, ImageFormat::Gif))
        }
    }

    #[test]
    fn shim_preserves_actual_timing_errors_and_accepted_frame_sequence() {
        let buffer = zenpixels::PixelBuffer::new(1, 1, PixelDescriptor::RGB8_SRGB);
        let mut generic = Timed::new();
        let mut erased: Box<dyn DynAnimationFrameEncoder> =
            Box::new(AnimationFrameEncoderShim(Timed::new()));
        for _ in 0..9 {
            let duration = FrameDuration::new(1, 30).unwrap();
            assert_eq!(
                generic.push_frame_exact(buffer.as_slice(), duration, None),
                Err(UnsupportedOperation::AnimationTiming)
            );
            let error = erased
                .push_frame_exact(buffer.as_slice(), duration, None)
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<UnsupportedOperation>(),
                Some(&UnsupportedOperation::AnimationTiming)
            );
            assert_eq!(
                generic
                    .push_frame_timed(buffer.as_slice(), duration, None)
                    .unwrap(),
                erased
                    .push_frame_timed(buffer.as_slice(), duration, None)
                    .unwrap()
            );
        }
        let exact = FrameDuration::new(1, 100).unwrap();
        generic
            .push_frame_exact(buffer.as_slice(), exact, None)
            .unwrap();
        erased
            .push_frame_exact(buffer.as_slice(), exact, None)
            .unwrap();
        assert_eq!(
            generic.finish(None).unwrap().data(),
            erased.finish(None).unwrap().data()
        );
    }
}

#[cfg(test)]
mod negotiated_output_tests {
    use super::*;
    use crate::output_patch::{OutputAccess, OutputFinish, Patch};
    struct Patching(bool);
    impl Encoder for Patching {
        type Error = crate::UnsupportedOperation;
        fn reject(op: Self::Error) -> Self::Error {
            op
        }
        fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
            Err(crate::UnsupportedOperation::RowLevelEncode)
        }
        fn begin_output(
            &mut self,
            _: u32,
            _: u32,
            _: PixelDescriptor,
            access: OutputAccess,
        ) -> Result<crate::encode::OutputTiming, Self::Error> {
            if access != OutputAccess::SeekBack {
                return Err(crate::UnsupportedOperation::RowLevelEncode);
            }
            self.0 = true;
            Ok(crate::encode::OutputTiming::AT_FINISH.with_patches())
        }
        fn finish_output(self) -> Result<OutputFinish<EncodeOutput>, Self::Error> {
            assert!(self.0);
            Ok(OutputFinish::new(
                EncodeOutput::new(alloc::vec![0], ImageFormat::Jpeg)
                    .with_extras(7u32)
                    .with_extension("qualified"),
                alloc::vec![Patch {
                    offset: 0,
                    bytes: alloc::vec![9]
                }],
            ))
        }
    }
    #[test]
    fn erased_encoder_forwards_negotiation_patches_and_output_metadata() {
        let mut encoder: Box<dyn DynEncoder> = Box::new(EncoderShim(Patching(false)));
        assert!(
            encoder
                .begin_output(1, 1, PixelDescriptor::RGB8_SRGB, OutputAccess::Forward)
                .is_err()
        );
        assert!(
            encoder
                .begin_output(1, 1, PixelDescriptor::RGB8_SRGB, OutputAccess::SeekBack)
                .unwrap()
                .patches_at_finish()
        );
        let finish = encoder.finish_output().unwrap();
        assert_eq!(finish.validate(0).unwrap(), 1);
        assert_eq!(finish.tail.extras::<u32>(), Some(&7));
        assert_eq!(finish.tail.extension(), "qualified");
        assert_eq!(finish.patches[0].bytes, [9]);
    }
}

#[cfg(test)]
mod pull_dispatch_tests {
    use super::*;
    use crate::encode::{EncodeRows, PullProgress, RowDemand, RowFillError};
    use crate::{At, UnsupportedOperation};

    struct Pulling {
        pixels: [u8; 18],
        done: bool,
    }
    impl Encoder for Pulling {
        type Error = At<UnsupportedOperation>;
        fn reject(op: UnsupportedOperation) -> Self::Error {
            At::wrap(op)
        }
        fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
            Err(Self::reject(UnsupportedOperation::PullEncode))
        }
        fn pull_rows(
            &mut self,
            rows: &mut dyn EncodeRows,
            _: &dyn crate::Pulse,
        ) -> Result<PullProgress, Self::Error> {
            if self.done {
                return Ok(PullProgress::Done);
            }
            let view =
                PixelSliceMut::new(&mut self.pixels, 2, 2, 9, PixelDescriptor::RGB8_SRGB).unwrap();
            rows.fill(
                RowDemand::new(2, 3, view)
                    .with_final_pass(true)
                    .with_label(7),
            )
            .map_err(|_| Self::reject(UnsupportedOperation::PullEncode))?;
            assert_eq!(
                self.pixels,
                [
                    1, 2, 3, 4, 5, 6, 165, 165, 165, 7, 8, 9, 10, 11, 12, 165, 165, 165
                ]
            );
            self.done = true;
            Ok(PullProgress::Filled)
        }
    }
    struct Fill {
        calls: usize,
        fail: bool,
    }
    impl EncodeRows for Fill {
        fn fill(&mut self, mut demand: RowDemand<'_>) -> Result<(), RowFillError> {
            self.calls += 1;
            assert_eq!(
                (demand.pass, demand.first, demand.label, demand.final_pass),
                (2, 3, 7, true)
            );
            assert_eq!((demand.rows.width(), demand.rows.rows()), (2, 2));
            if self.fail {
                return Err(RowFillError);
            }
            demand.rows.row_mut(0).copy_from_slice(&[1, 2, 3, 4, 5, 6]);
            demand
                .rows
                .row_mut(1)
                .copy_from_slice(&[7, 8, 9, 10, 11, 12]);
            Ok(())
        }
    }
    fn encoder() -> Pulling {
        Pulling {
            pixels: [165; 18],
            done: false,
        }
    }

    #[test]
    fn erased_pull_preserves_native_row_loan_and_progress() {
        let mut typed = encoder();
        let mut source = Fill {
            calls: 0,
            fail: false,
        };
        assert_eq!(
            typed
                .pull_rows(&mut source, &crate::how_far::NoPulse)
                .unwrap(),
            PullProgress::Filled
        );
        assert_eq!(
            typed
                .pull_rows(&mut source, &crate::how_far::NoPulse)
                .unwrap(),
            PullProgress::Done
        );
        assert_eq!(source.calls, 1);
        let mut erased: Box<dyn DynEncoder> = Box::new(EncoderShim(encoder()));
        assert_eq!(
            erased
                .pull_rows(&mut source, &crate::how_far::NoPulse)
                .unwrap(),
            PullProgress::Filled
        );
        assert_eq!(
            erased
                .pull_rows(&mut source, &crate::how_far::NoPulse)
                .unwrap(),
            PullProgress::Done
        );
        assert_eq!(source.calls, 2);
    }

    #[test]
    fn erased_pull_preserves_the_located_native_failure() {
        let mut erased: Box<dyn DynEncoder> = Box::new(EncoderShim(encoder()));
        let mut source = Fill {
            calls: 0,
            fail: true,
        };
        let error = erased
            .pull_rows(&mut source, &crate::how_far::NoPulse)
            .unwrap_err();
        assert_eq!(source.calls, 1);
        let native = error
            .downcast_ref::<At<UnsupportedOperation>>()
            .expect("original located native error");
        assert_eq!(native.error(), &UnsupportedOperation::PullEncode);
    }
}

#[cfg(test)]
mod declared_encode_tests {
    use super::*;
    use crate::encode::{ColorCarriers, EncodeDeclaration, EncodePlan, OutputTiming};
    use crate::{Cicp, UnsupportedOperation};
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use zenpixels::ColorContext;

    struct Job(Arc<AtomicUsize>);
    impl EncodeJob for Job {
        type Error = UnsupportedOperation;
        type Enc = Declared;
        type AnimationFrameEnc = ();
        fn with_pulse(self, _: how_far::SharedPulse) -> Self {
            self
        }
        fn with_limits(self, _: ResourceLimits) -> Self {
            self
        }
        fn with_metadata(self, _: Metadata) -> Self {
            self
        }
        fn plan_encode(
            &self,
            declaration: &EncodeDeclaration<'_>,
        ) -> Result<EncodePlan, BoxedError> {
            // The effective job is retained; planning never constructs its encoder.
            assert_eq!(self.0.load(Ordering::Relaxed), 0);
            native_plan(declaration).map_err(|error| Box::new(error) as BoxedError)
        }
        fn encoder(self) -> Result<Declared, Self::Error> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(Declared)
        }
        fn animation_frame_encoder(self) -> Result<Self::AnimationFrameEnc, Self::Error> {
            Err(UnsupportedOperation::AnimationEncode)
        }
    }

    fn native_plan(
        declaration: &EncodeDeclaration<'_>,
    ) -> Result<EncodePlan, UnsupportedOperation> {
        let p3 = ColorContext::from_cicp(Cicp::DISPLAY_P3);
        if declaration.color != Some(&p3) {
            return Err(UnsupportedOperation::PixelFormat);
        }
        assert_eq!(
            declaration.access,
            crate::output_patch::OutputAccess::SeekBack
        );
        Ok(EncodePlan::rows(OutputTiming::AT_FINISH.with_patches())
            .with_carriers(ColorCarriers::new(None, Some(Cicp::DISPLAY_P3))))
    }

    struct Declared;
    impl Encoder for Declared {
        type Error = UnsupportedOperation;
        fn reject(error: Self::Error) -> Self::Error {
            error
        }
        fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
            Err(UnsupportedOperation::RowLevelEncode)
        }
        fn begin_declared(
            &mut self,
            declaration: &EncodeDeclaration<'_>,
        ) -> Result<EncodePlan, Self::Error> {
            native_plan(declaration)
        }
        fn begin_output(
            &mut self,
            _: u32,
            _: u32,
            _: PixelDescriptor,
            _: crate::output_patch::OutputAccess,
        ) -> Result<OutputTiming, Self::Error> {
            panic!("complete declaration must not be lowered to legacy startup")
        }
    }

    #[test]
    fn erased_preflight_retains_job_color_access_and_native_error() {
        let starts = Arc::new(AtomicUsize::new(0));
        let job: Box<dyn DynEncodeJob> = Box::new(EncodeJobShim(Some(Job(starts.clone()))));
        let declaration = EncodeDeclaration::new(16, 32, PixelDescriptor::RGB8_SRGB)
            .with_access(crate::output_patch::OutputAccess::SeekBack);
        let error = job.plan_encode(&declaration).unwrap_err();
        assert_eq!(
            error.downcast_ref::<UnsupportedOperation>(),
            Some(&UnsupportedOperation::PixelFormat)
        );
        let context = ColorContext::from_cicp(Cicp::DISPLAY_P3);
        let declaration = declaration.with_color(&context);
        let planned = job.plan_encode(&declaration).unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 0);
        let mut encoder = job.into_encoder().unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 1);
        let observed = encoder.begin_declared(&declaration).unwrap();
        planned.validate(&declaration).unwrap();
        planned.validate_started(&observed).unwrap();
    }

    struct Legacy;
    impl Encoder for Legacy {
        type Error = UnsupportedOperation;
        fn reject(error: Self::Error) -> Self::Error {
            error
        }
        fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
            Err(UnsupportedOperation::RowLevelEncode)
        }
        fn begin_output(
            &mut self,
            _: u32,
            _: u32,
            _: PixelDescriptor,
            _: crate::output_patch::OutputAccess,
        ) -> Result<OutputTiming, Self::Error> {
            panic!("unsupported planning must refuse without startup")
        }
    }
    struct LegacyJob;
    impl EncodeJob for LegacyJob {
        type Error = UnsupportedOperation;
        type Enc = (); // Default planning must also support animation-only jobs.
        type AnimationFrameEnc = ();
        fn with_pulse(self, _: how_far::SharedPulse) -> Self {
            self
        }
        fn with_limits(self, _: ResourceLimits) -> Self {
            self
        }
        fn with_metadata(self, _: Metadata) -> Self {
            self
        }
        fn encoder(self) -> Result<(), Self::Error> {
            panic!("must not construct")
        }
        fn animation_frame_encoder(self) -> Result<(), Self::Error> {
            panic!("must not construct")
        }
    }

    #[test]
    fn defaults_refuse_without_legacy_startup_even_without_color() {
        let declaration = EncodeDeclaration::new(1, 1, PixelDescriptor::RGB8_SRGB);
        let error = LegacyJob.plan_encode(&declaration).unwrap_err();
        assert_eq!(
            error.downcast_ref::<UnsupportedOperation>(),
            Some(&UnsupportedOperation::EncodePlanning)
        );
        assert_eq!(
            Legacy.begin_declared(&declaration),
            Err(UnsupportedOperation::EncodePlanning)
        );
        let mut erased: Box<dyn DynEncoder> = Box::new(EncoderShim(Legacy));
        let error = erased.begin_declared(&declaration).unwrap_err();
        assert_eq!(
            error.downcast_ref::<UnsupportedOperation>(),
            Some(&UnsupportedOperation::EncodePlanning)
        );
    }
}

#[cfg(test)]
#[path = "dyn_encoding/precision_tests.rs"]
mod precision_tests;

#[cfg(test)]
#[path = "dyn_encoding/job_erasure_tests.rs"]
mod job_erasure_tests;
