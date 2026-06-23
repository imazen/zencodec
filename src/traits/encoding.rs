//! Encoder configuration and encode jobs.

use alloc::boxed::Box;

use crate::estimate::{ComputeEnvironment, ImageCharacteristics, ResourceEstimate};
use crate::fidelity::{Fidelity, LossyTarget};
use crate::format::ImageFormat;
use crate::{EncodeCapabilities, Metadata, ResourceLimits};
use zenpixels::PixelDescriptor;

use super::BoxedError;
use super::dyn_encoding::{AnimationFrameEncoderShim, DynAnimationFrameEncoder, DynEncoder};
use super::encoder::{AnimationFrameEncoder, Encoder};

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
/// The `job()` method consumes the config and creates a per-operation
/// [`EncodeJob`] that owns it. Clone the config first if you need to
/// reuse it: `config.clone().job()`.
pub trait EncoderConfig: Clone + Send + Sync {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Per-operation job type. Owns all configuration including stop token.
    type Job: EncodeJob<Error = Self::Error>;

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

    /// Set encoding quality on a calibrated 0.0--100.0 scale.
    ///
    /// "Generic" because this is the codec-agnostic quality knob. Individual
    /// codecs may also have format-specific quality methods on their config types.
    ///
    /// # Note
    ///
    /// The default implementation is a no-op. Not all codecs support quality
    /// tuning. Use [`generic_quality()`](EncoderConfig::generic_quality) after
    /// calling this to verify the codec accepted the value -- it returns `None`
    /// if the codec does not support quality settings.
    fn with_generic_quality(self, _quality: f32) -> Self {
        self
    }

    /// Set encoding effort (higher = slower, better compression).
    ///
    /// "Generic" because this is the codec-agnostic effort knob. Individual
    /// codecs may also have format-specific effort/speed methods.
    ///
    /// Each codec maps this to its internal effort/speed scale.
    ///
    /// # Note
    ///
    /// The default implementation is a no-op. Not all codecs support effort
    /// tuning. Use [`generic_effort()`](EncoderConfig::generic_effort) after
    /// calling this to verify the codec accepted the value -- it returns `None`
    /// if the codec does not support effort settings.
    fn with_generic_effort(self, _effort: i32) -> Self {
        self
    }

    /// Enable or disable lossless encoding.
    ///
    /// When lossless is enabled, quality is ignored.
    ///
    /// # Note
    ///
    /// The default implementation is a no-op. Not all codecs support lossless
    /// mode. Use [`is_lossless()`](EncoderConfig::is_lossless) after calling
    /// this to verify the codec accepted the value -- it returns `None` if the
    /// codec does not support lossless encoding.
    fn with_lossless(self, _lossless: bool) -> Self {
        self
    }

    /// Set independent alpha channel quality on a calibrated 0.0--100.0 scale.
    ///
    /// # Note
    ///
    /// The default implementation is a no-op. Not all codecs support separate
    /// alpha quality. Use [`alpha_quality()`](EncoderConfig::alpha_quality) after
    /// calling this to verify the codec accepted the value -- it returns `None`
    /// if the codec does not support alpha quality settings.
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

    /// Set the encode [`Fidelity`] — a lossy [`LossyTarget`], a near-lossless
    /// [`NearLosslessBudget`](crate::encode::NearLosslessBudget), or lossless.
    ///
    /// Infallible and **best-effort**: the codec does what it can and silently
    /// substitutes the rest. Read what it resolved to with
    /// [`resolved_target_fidelity`](Self::resolved_target_fidelity).
    ///
    /// The default bridges to the legacy [`with_generic_quality`](Self::with_generic_quality)
    /// / [`with_lossless`](Self::with_lossless) setters, so a codec that has not
    /// implemented native fidelity still behaves sensibly — a near-lossless budget
    /// promotes to exact lossless. Codecs override to honor budgets natively (e.g.
    /// PNG's L∞ bit-rounding, WebP's near-lossless dial).
    fn with_fidelity(self, fidelity: Fidelity) -> Self {
        match fidelity {
            // Default fallback. Only the 0–100-scaled targets map to the generic
            // quality dial — a butteraugli *distance* has no honest
            // codec-agnostic → 0–100 conversion, so the default just selects
            // lossy and leaves quality at the codec default (visible through
            // `resolved_target_fidelity`). Codecs override to honor butteraugli
            // / SSIM2 / their native scale precisely.
            Fidelity::Lossy(LossyTarget::ApproxSsim2(q) | LossyTarget::CodecSpecificQuality(q)) => {
                self.with_lossless(false).with_generic_quality(q)
            }
            Fidelity::Lossy(LossyTarget::ApproxButteraugli(_)) => self.with_lossless(false),
            Fidelity::NearLossless(_) | Fidelity::Lossless => self.with_lossless(true),
        }
    }

    /// The fidelity the codec actually resolved to, or `None` if it has no
    /// fidelity control.
    ///
    /// The default derives from [`is_lossless`](Self::is_lossless) and
    /// [`generic_quality`](Self::generic_quality), so codecs that only implement
    /// the legacy getters still report a `Fidelity`. Codecs that honor a
    /// near-lossless budget natively override this to report the budget they met.
    fn resolved_target_fidelity(&self) -> Option<Fidelity> {
        if self.is_lossless() == Some(true) {
            return Some(Fidelity::Lossless);
        }
        // The default codec stored a generic quality; report it on the
        // codec-specific scale. Codecs that honor a metric/budget natively
        // override this to report precisely.
        self.generic_quality().map(Fidelity::codec_quality)
    }

    /// Current alpha quality value, or `None` if unsupported.
    fn alpha_quality(&self) -> Option<f32> {
        None
    }

    /// Predict peak memory, wall time, and CPU-core scaling for encoding
    /// `image` on the `compute` environment.
    ///
    /// The returned [`ResourceEstimate`] is already adjusted for
    /// `compute.cores()` (its `time_ms` and peak terms fold in the codec's
    /// measured [`ThreadingInformation`](crate::estimate::ThreadingInformation)). The
    /// three inputs are expandable: this config carries the encode knobs
    /// (effort / quality / lossless / thread intent),
    /// [`ImageCharacteristics`] the image, and [`ComputeEnvironment`] the
    /// hardware.
    ///
    /// The default is [`ResourceEstimate::unknown`] — every field `None`,
    /// i.e. "this codec does not model its resource use." Codecs with a
    /// calibrated `heuristics` model override this (a quick option is to return
    /// [`ResourceEstimate::conservative`]`(image).at_cores(compute.cores())`).
    fn estimate_encode_resources(
        &self,
        image: &ImageCharacteristics,
        compute: &ComputeEnvironment,
    ) -> ResourceEstimate {
        let _ = (image, compute);
        ResourceEstimate::unknown()
    }

    /// Create a per-operation job, consuming the config.
    ///
    /// The job owns the config and all configuration set on it
    /// (stop token, limits, metadata).
    fn job(self) -> Self::Job;
}

// ===========================================================================
// Encode job
// ===========================================================================

/// Per-operation encode job.
///
/// Created by [`EncoderConfig::job()`]. Binds metadata, limits, and
/// cancellation for a single encode operation. Produces either an `Enc`
/// (single image via per-format traits) or a `AnimationFrameEnc` (animation
/// via full-frame encoder).
pub trait EncodeJob: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Single-image encoder type (implements [`Encoder`]).
    type Enc: Sized + 'static;

    /// Full-frame animation encoder type (implements [`AnimationFrameEncoder`]).
    ///
    /// Must be `'static` and `Send` — frame encoders own their configuration
    /// (clone configs, convert stop tokens to owned form). This lets
    /// callers use the encoder independently of the job's scope and across
    /// thread boundaries (e.g., in pipeline `Sink` implementations).
    type AnimationFrameEnc: Sized + Send + 'static;

    /// Set cooperative cancellation token.
    ///
    /// [`StopToken`](almost_enough::StopToken) is `Clone + Send + Sync + 'static` —
    /// an owned, type-erased stop. Convert any `Stop + Clone + 'static` with
    /// `stop.into_token()` or `StopToken::new(stop)`.
    fn with_stop(self, stop: crate::StopToken) -> Self;

    /// Override resource limits for this operation.
    fn with_limits(self, limits: ResourceLimits) -> Self;

    /// Set encode security policy (controls metadata embedding, etc.).
    ///
    /// # Note
    ///
    /// The default implementation is a no-op that returns `self` unchanged.
    /// Not all codecs implement policy support. Check the codec's documentation
    /// or [`EncodeCapabilities`](crate::EncodeCapabilities) to determine whether
    /// the codec honors policy settings. Calling this on a codec that does not
    /// implement it will silently have no effect.
    fn with_policy(self, _policy: crate::EncodePolicy) -> Self {
        self
    }

    /// Set metadata to embed, **filtered by an explicit retention policy** — the
    /// blessed path. Metadata retention is a privacy decision, so the policy is
    /// required: [`MetadataPolicy::Web`](crate::MetadataPolicy::Web) is the
    /// privacy-safe choice (strips GPS/camera/timestamps/thumbnail/XMP, keeps
    /// orientation + rights + color signaling);
    /// [`PreserveExact`](crate::MetadataPolicy::PreserveExact) embeds verbatim.
    ///
    /// A *provided* method (no codec change): it filters via
    /// [`Metadata::filtered`](crate::Metadata::filtered) — which also reconciles
    /// the embedded EXIF orientation tag — then hands the result to the codec's
    /// [`with_metadata`](Self::with_metadata). The codec embeds what its format
    /// supports and skips the rest.
    #[must_use]
    fn with_metadata_policy(self, meta: Metadata, policy: crate::MetadataPolicy) -> Self {
        #[allow(deprecated)]
        self.with_metadata(meta.filtered(&policy))
    }

    /// Set metadata (ICC, EXIF, XMP) to embed in the output, **without choosing a
    /// retention policy**. Codecs implement this (store the bytes); callers should
    /// prefer [`with_metadata_policy`](Self::with_metadata_policy) so the
    /// privacy/retention decision is explicit.
    ///
    /// Takes ownership — callers that need to reuse the metadata should `.clone()`
    /// first (the `Arc<[u8]>` fields make cloning a cheap ref-count bump). The
    /// codec embeds what the format supports and silently skips the rest.
    #[deprecated(note = "embeds metadata without an explicit retention policy; use \
                with_metadata_policy(meta, policy) — e.g. MetadataPolicy::Web \
                (privacy-safe) or MetadataPolicy::PreserveExact")]
    fn with_metadata(self, meta: Metadata) -> Self;

    /// Set animation canvas dimensions.
    ///
    /// For full-frame animation, this sets the expected frame dimensions.
    /// All pushed frames must match these dimensions. Default: canvas =
    /// first frame's dimensions.
    fn with_canvas_size(self, _width: u32, _height: u32) -> Self {
        self
    }

    /// Set animation loop count.
    ///
    /// - `Some(0)` = loop forever
    /// - `Some(n)` = loop `n` times
    /// - `None` = format default
    ///
    /// Must be set before [`animation_frame_encoder()`](EncodeJob::animation_frame_encoder)
    /// because formats write the loop count before frame data.
    fn with_loop_count(self, _count: Option<u32>) -> Self {
        self
    }

    /// Access codec-specific extensions for this job.
    ///
    /// Returns a reference to a `'static` extension type stored inside
    /// the job. Callers downcast via `Any::downcast_ref` to the codec's
    /// extension type. Returns `None` if the codec has no extensions.
    ///
    /// The extension type must be `'static` (no borrowed data), but the
    /// *reference* borrows from the job, which is fine.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Some(ext) = job.extensions() {
    ///     if let Some(jpeg) = ext.downcast_ref::<JpegEncodeExtensions>() {
    ///         // read codec-specific state
    ///     }
    /// }
    /// ```
    fn extensions(&self) -> Option<&dyn core::any::Any> {
        None
    }

    /// Mutable access to codec-specific extensions.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Some(ext) = job.extensions_mut() {
    ///     if let Some(jpeg) = ext.downcast_mut::<JpegEncodeExtensions>() {
    ///         jpeg.optimize_huffman = true;
    ///     }
    /// }
    /// ```
    fn extensions_mut(&mut self) -> Option<&mut dyn core::any::Any> {
        None
    }

    /// Create a one-shot encoder for a single image.
    fn encoder(self) -> Result<Self::Enc, Self::Error>;

    /// Create a full-frame animation encoder.
    ///
    /// Set loop count and canvas size before calling this.
    fn animation_frame_encoder(self) -> Result<Self::AnimationFrameEnc, Self::Error>;

    // --- Type-erased convenience methods ---

    /// Create a type-erased one-shot encoder.
    ///
    /// Returns a boxed [`DynEncoder`] that accepts any [`PixelSlice`](zenpixels::PixelSlice)
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
    ///     .with_metadata_policy(meta, MetadataPolicy::Web)
    ///     .dyn_encoder()?;
    ///
    /// // No generics from here on
    /// let output = encode.encode(pixels)?;
    /// ```
    fn dyn_encoder(self) -> Result<Box<dyn DynEncoder>, BoxedError>
    where
        Self::Enc: Encoder + Send,
    {
        let enc = self.encoder().map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(super::dyn_encoding::EncoderShim(enc)))
    }

    /// Create a type-erased full-frame animation encoder.
    ///
    /// Only available when `AnimationFrameEnc` implements [`AnimationFrameEncoder`].
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut enc = config.job()
    ///     .with_loop_count(Some(0))
    ///     .dyn_animation_frame_encoder()?;
    ///
    /// enc.push_frame(frame1_pixels, 100, None)?;
    /// enc.push_frame(frame2_pixels, 100, None)?;
    /// let output = enc.finish(None)?;
    /// ```
    fn dyn_animation_frame_encoder(self) -> Result<Box<dyn DynAnimationFrameEncoder>, BoxedError>
    where
        Self::AnimationFrameEnc: AnimationFrameEncoder + Send,
    {
        let enc = self
            .animation_frame_encoder()
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(AnimationFrameEncoderShim(enc)))
    }
}
