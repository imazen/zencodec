//! Encoder configuration and encode jobs.

use alloc::boxed::Box;

use crate::format::ImageFormat;
use crate::{EncodeCapabilities, Metadata, ResourceLimits};
use zenpixels::PixelDescriptor;

use super::BoxedError;
use super::dyn_encoding::{DynEncoder, DynFullFrameEncoder, FullFrameEncoderShim};
use super::encoder::{Encoder, FullFrameEncoder};

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

    /// Create a per-operation job, consuming the config.
    ///
    /// The job owns the config. The lifetime `'a` on `Job<'a>` is
    /// determined by the caller — typically inferred from `with_stop`:
    ///
    /// ```ignore
    /// let job = config.job().with_stop(&stop); // 'a = lifetime of stop
    /// let job = config.job();                  // 'a = 'static (no stop)
    /// ```
    fn job(self) -> Self::Job;
}

// ===========================================================================
// Encode job
// ===========================================================================

/// Per-operation encode job.
///
/// Created by [`EncoderConfig::job()`]. Binds metadata, limits, and
/// cancellation for a single encode operation. Produces either an `Enc`
/// (single image via per-format traits) or a `FullFrameEnc` (animation
/// via full-frame encoder).
pub trait EncodeJob: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Single-image encoder type (implements [`Encoder`]).
    type Enc: Sized + 'static;

    /// Full-frame animation encoder type (implements [`FullFrameEncoder`]).
    ///
    /// Must be `'static` and `Send` — frame encoders own their configuration
    /// (clone configs, convert stop tokens to owned form). This lets
    /// callers use the encoder independently of the job's scope and across
    /// thread boundaries (e.g., in pipeline `Sink` implementations).
    type FullFrameEnc: Sized + Send + 'static;

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
    /// Default no-op. Codecs that support policy check the flags in
    /// [`EncodePolicy`](crate::encode::EncodePolicy) to decide what to embed.
    fn with_policy(self, _policy: crate::EncodePolicy) -> Self {
        self
    }

    /// Set metadata (ICC, EXIF, XMP) to embed in the output.
    ///
    /// The codec embeds what the format supports, silently skips the rest.
    fn with_metadata(self, meta: &Metadata) -> Self;

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
    /// Must be set before [`full_frame_encoder()`](EncodeJob::full_frame_encoder)
    /// because formats write the loop count before frame data.
    fn with_loop_count(self, _count: Option<u32>) -> Self {
        self
    }

    /// Access codec-specific extensions for this job.
    ///
    /// Returns a reference to a `'static` extension type stored inside
    /// the job. Callers downcast via [`Any::downcast_ref`] to the codec's
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
    fn full_frame_encoder(self) -> Result<Self::FullFrameEnc, Self::Error>;

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
    ///     .with_metadata(&meta)
    ///     .dyn_encoder()?;
    ///
    /// // No generics from here on
    /// let output = encode.encode(pixels)?;
    /// ```
    fn dyn_encoder(self) -> Result<Box<dyn DynEncoder>, BoxedError>
    where
        Self::Enc: Encoder,
    {
        let enc = self.encoder().map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(super::dyn_encoding::EncoderShim(enc)))
    }

    /// Create a type-erased full-frame animation encoder.
    ///
    /// Only available when `FullFrameEnc` implements [`FullFrameEncoder`].
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut enc = config.job()
    ///     .with_loop_count(Some(0))
    ///     .dyn_full_frame_encoder()?;
    ///
    /// enc.push_frame(frame1_pixels, 100, None)?;
    /// enc.push_frame(frame2_pixels, 100, None)?;
    /// let output = enc.finish(None)?;
    /// ```
    fn dyn_full_frame_encoder(self) -> Result<Box<dyn DynFullFrameEncoder>, BoxedError>
    where
        Self::FullFrameEnc: FullFrameEncoder + Send,
    {
        let enc = self
            .full_frame_encoder()
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(FullFrameEncoderShim(enc)))
    }
}
