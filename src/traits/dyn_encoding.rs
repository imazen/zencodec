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
use enough::Stop;
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
/// Encoders may borrow job-scoped data (stop tokens, metadata) so they
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
/// [`into_animation_frame_encoder`](DynEncodeJob::into_animation_frame_encoder).
pub trait DynEncodeJob {
    /// Set cooperative cancellation token.
    fn set_stop(&mut self, stop: StopToken);

    /// Override resource limits.
    fn set_limits(&mut self, limits: ResourceLimits);

    /// Set encode security policy.
    fn set_policy(&mut self, policy: crate::EncodePolicy);

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
    fn set_stop(&mut self, stop: StopToken) {
        self.try_apply(|job| job.with_stop(stop));
    }

    fn set_limits(&mut self, limits: ResourceLimits) {
        self.try_apply(|job| job.with_limits(limits));
    }

    fn set_policy(&mut self, policy: crate::EncodePolicy) {
        self.try_apply(|job| job.with_policy(policy));
    }

    fn set_metadata_policy(&mut self, meta: Metadata, policy: crate::MetadataPolicy) {
        self.try_apply(|job| job.with_metadata_policy(meta, policy));
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

    /// Create a dyn-dispatched encode job.
    ///
    /// The job owns its config (cloned). The `'static` bound means
    /// the job can outlive the config reference — the only remaining
    /// lifetime dependency is the stop token (set via `set_stop`).
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

    fn dyn_job(&self) -> Box<dyn DynEncodeJob + 'static> {
        Box::new(EncodeJobShim(Some(self.clone().job())))
    }
}
