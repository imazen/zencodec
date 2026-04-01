//! Decoder configuration and decode jobs.

use alloc::borrow::Cow;
use alloc::boxed::Box;

use crate::format::ImageFormat;
use crate::orientation::OrientationHint;
use crate::{DecodeCapabilities, ImageInfo, OutputInfo, ResourceLimits, StopToken};
use zenpixels::PixelDescriptor;

use super::BoxedError;
use super::decoder::{AnimationFrameDecoder, Decode, StreamingDecode};
use super::dyn_decoding::{
    AnimationFrameDecoderShim, DecoderShim, DynAnimationFrameDecoder, DynDecoder,
    DynStreamingDecoder, StreamingDecoderShim,
};

// ===========================================================================
// Decoder configuration
// ===========================================================================

/// Reusable decoder configuration.
///
/// Implemented by each codec's config type. Config types are `Clone + Send +
/// Sync` with no lifetimes.
///
/// Probing lives on [`DecodeJob`], not here, because probing needs limits
/// and cancellation context.
pub trait DecoderConfig: Clone + Send + Sync {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Per-operation job type.
    ///
    /// The GAT lifetime `'a` is the data lifetime — it becomes concrete
    /// when the caller passes data to [`DecodeJob::decoder`].
    type Job<'a>: DecodeJob<'a, Error = Self::Error> + 'static;

    /// The image formats this decoder handles.
    ///
    /// A single-format decoder returns one element. Multi-format decoders
    /// (e.g., a bitmap crate handling BMP, ICO, CUR, TGA, PNM) return all
    /// formats they can decode. The dispatch layer registers this config
    /// for each format in the list.
    ///
    /// Must not be empty.
    fn formats() -> &'static [ImageFormat];

    /// Pixel formats this decoder can produce natively.
    ///
    /// Every descriptor is a guarantee: the decoder can produce this format
    /// without lossy conversion. Must not be empty.
    fn supported_descriptors() -> &'static [PixelDescriptor];

    /// Decoder capabilities (metadata support, cancellation, etc.).
    ///
    /// Returns a static reference describing what this decoder supports.
    fn capabilities() -> &'static DecodeCapabilities {
        &DecodeCapabilities::EMPTY
    }

    /// Create a per-operation job, consuming the config.
    ///
    /// The job owns the config and all configuration set on it.
    /// The lifetime parameter is the data lifetime, inferred from usage.
    fn job<'a>(self) -> Self::Job<'a>;
}

// ===========================================================================
// Decode job
// ===========================================================================

/// Per-operation decode job.
///
/// Created by [`DecoderConfig::job()`]. Holds limits, cancellation, and
/// decode hints. Probing lives here because it needs the limits/stop context.
///
/// # Decode hints
///
/// Hints let the caller request spatial transforms (crop, scale, orientation)
/// that the decoder may apply during decode. The decoder is free to ignore
/// any hint. Call [`output_info()`](DecodeJob::output_info) after setting
/// hints to learn what the decoder will actually produce.
pub trait DecodeJob<'a>: Sized {
    /// The codec-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Single-image decoder type.
    type Dec: Decode<Error = Self::Error>;

    /// Streaming decoder type.
    ///
    /// Implements [`StreamingDecode`] for batch/scanline-level decode.
    /// Set to `()` if the codec does not support streaming decode.
    type StreamDec: StreamingDecode<Error = Self::Error> + Send;

    /// Full-frame animation decoder type.
    ///
    /// Must be `'static` and `Send` — frame decoders own their data (typically by
    /// copying the input slice at construction time). This lets callers
    /// drop the input buffer while still iterating frames, and use decoders
    /// across thread boundaries (e.g., in pipeline `Source` implementations).
    type AnimationFrameDec: AnimationFrameDecoder<Error = Self::Error> + Send + 'static;

    /// Set cooperative cancellation token.
    ///
    /// [`StopToken`](crate::StopToken) is `Clone + Send + Sync + 'static` —
    /// an owned, type-erased stop. Convert any `Stop + 'static` with
    /// `StopToken::new(stop)`.
    fn with_stop(self, stop: StopToken) -> Self;

    /// Override resource limits for this operation.
    fn with_limits(self, limits: ResourceLimits) -> Self;

    /// Set decode security policy (controls metadata extraction, parsing strictness, etc.).
    ///
    /// # Note
    ///
    /// The default implementation is a no-op that returns `self` unchanged.
    /// Not all codecs implement policy support. Check the codec's documentation
    /// or [`DecodeCapabilities`](crate::DecodeCapabilities) to determine whether
    /// the codec honors policy settings. Calling this on a codec that does not
    /// implement it will silently have no effect.
    fn with_policy(self, _policy: crate::DecodePolicy) -> Self {
        self
    }

    // --- Probing (needs limits + stop context) ---

    /// Probe image metadata cheaply (header parse only).
    ///
    /// O(header), not O(pixels). Parses container headers to extract
    /// dimensions, format, and basic metadata. May not return frame
    /// counts or data requiring a full parse.
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;

    /// Probe image metadata with a full parse.
    ///
    /// May be expensive (e.g., parsing all GIF frames to count them).
    /// Returns complete metadata including frame counts.
    ///
    /// Default: delegates to [`probe()`](DecodeJob::probe).
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, Self::Error> {
        self.probe(data)
    }

    // --- Decode hints (optional, decoder may ignore) ---

    /// Hint: crop to this region in source coordinates.
    ///
    /// The decoder may adjust for block alignment (JPEG MCU boundaries).
    fn with_crop_hint(self, _x: u32, _y: u32, _width: u32, _height: u32) -> Self {
        self
    }

    /// Set orientation handling strategy.
    ///
    /// See [`OrientationHint`] for the available strategies.
    /// Default: [`OrientationHint::Preserve`].
    fn with_orientation(self, _hint: OrientationHint) -> Self {
        self
    }

    /// Hint: start decoding from a specific frame (0-based).
    ///
    /// For animation formats, the decoder seeks to the nearest keyframe
    /// at or before `index` and composites forward to produce the
    /// requested frame as the first yielded result.
    ///
    /// Only meaningful before [`animation_frame_decoder()`](DecodeJob::animation_frame_decoder).
    fn with_start_frame_index(self, _index: u32) -> Self {
        self
    }

    /// Opt in to supplementary gain map extraction.
    ///
    /// When true, codecs that support gain maps (AVIF tmap, JXL jhgm,
    /// JPEG UltraHDR) extract and attach gain map data to decode output
    /// extras. Typically set after probing reveals a gain map is present.
    fn with_extract_gain_map(self, _extract: bool) -> Self {
        self
    }

    /// Access codec-specific extensions for this job.
    ///
    /// Returns a reference to a `'static` extension type stored inside
    /// the job. Callers downcast via `Any::downcast_ref` to the codec's
    /// extension type. Returns `None` if the codec has no extensions.
    fn extensions(&self) -> Option<&dyn core::any::Any> {
        None
    }

    /// Mutable access to codec-specific extensions.
    fn extensions_mut(&mut self) -> Option<&mut dyn core::any::Any> {
        None
    }

    // --- Output prediction ---

    /// Predict what the decoder will produce given current hints.
    ///
    /// Returns dimensions, pixel format, and which hints were honored.
    /// Call after setting hints, before creating a decoder.
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, Self::Error>;

    // --- Executor creation ---
    //
    // All executors bind `data` here so the DecodeJob is the single
    // place where input is provided. This keeps Decode/StreamingDecode/
    // AnimationFrameDecoder free of data parameters, and prepares for future
    // IO-read sources (the job can bind a reader instead of a slice).
    //
    // Consistent parameter order: data, [sink], preferred.

    /// Create a one-shot decoder bound to `data`.
    ///
    /// The decoder stores the [`Cow`] and borrows from it via [`core::ops::Deref`].
    /// Pass `Cow::Borrowed(&slice)` for zero-copy slice access, or
    /// `Cow::Owned(vec)` to donate a buffer (avoids a copy in codecs
    /// that need owned data internally).
    ///
    /// `preferred` is a ranked list of desired output formats. The decoder
    /// picks the first it can produce without lossy conversion. Pass `&[]`
    /// for the decoder's native format.
    fn decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Self::Dec, Self::Error>;

    /// Decode directly into a caller-owned sink (push model).
    ///
    /// Calls [`begin()`](crate::DecodeRowSink::begin) once, then pushes
    /// strips via [`provide_next_buffer()`](crate::DecodeRowSink::provide_next_buffer),
    /// then calls [`finish()`](crate::DecodeRowSink::finish).
    /// Returns [`OutputInfo`] describing what was produced.
    ///
    /// `preferred` is a ranked list of desired output formats.
    ///
    /// Codecs with native row/strip streaming should write decoded rows
    /// directly into the sink. Codecs that can only do one-shot decode
    /// should call [`zencodec::helpers::copy_decode_to_sink()`](crate::helpers::copy_decode_to_sink) as a fallback.
    fn push_decoder(
        self,
        data: Cow<'a, [u8]>,
        sink: &mut dyn crate::DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, Self::Error>;

    /// Create a streaming decoder that yields scanline batches.
    ///
    /// Returns an error if the codec does not support streaming decode.
    ///
    /// `preferred` is a ranked list of desired output formats.
    ///
    /// See [`StreamingDecode`] for the batch pull API.
    fn streaming_decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Self::StreamDec, Self::Error>;

    /// Create a full-frame animation decoder.
    ///
    /// The decoder composites internally and yields full-canvas frames.
    /// The decoder calls [`Cow::into_owned()`] to take ownership of the
    /// data (required because `AnimationFrameDec: 'static`). When the caller
    /// passes `Cow::Owned(vec)`, this is a free move with no copy.
    ///
    /// `preferred` is a ranked list of desired output formats.
    fn animation_frame_decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Self::AnimationFrameDec, Self::Error>;

    // --- Type-erased convenience methods ---

    /// Create a type-erased one-shot decoder.
    ///
    /// Returns a boxed closure that decodes to owned pixels. All hints
    /// and preferences are bound before this call.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let decode = config.job()
    ///     .with_crop_hint(0, 0, 800, 600)
    ///     .dyn_decoder(data, &[PixelDescriptor::rgb8()])?;
    ///
    /// let output: DecodeOutput = decode.decode()?;
    /// ```
    fn dyn_decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynDecoder + 'a>, BoxedError>
    where
        Self: 'a,
    {
        let dec = self
            .decoder(data, preferred)
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(DecoderShim(dec)))
    }

    /// Create a type-erased full-frame animation decoder.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut dec = config.job()
    ///     .dyn_animation_frame_decoder(data, &[])?;
    ///
    /// while let Some(frame) = dec.render_next_frame_owned(None)? {
    ///     // process frame
    /// }
    /// ```
    fn dyn_animation_frame_decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynAnimationFrameDecoder>, BoxedError>
    where
        Self: 'a,
        Self::AnimationFrameDec: Send,
    {
        let dec = self
            .animation_frame_decoder(data, preferred)
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(AnimationFrameDecoderShim(dec)))
    }

    /// Create a type-erased streaming decoder.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut dec = config.job()
    ///     .dyn_streaming_decoder(data, &[])?;
    ///
    /// while let Some((y, strip)) = dec.next_batch()? {
    ///     // process strip
    /// }
    /// ```
    fn dyn_streaming_decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError>
    where
        Self: 'a,
        Self::StreamDec: Send,
    {
        let dec = self
            .streaming_decoder(data, preferred)
            .map_err(|e| Box::new(e) as BoxedError)?;
        Ok(Box::new(StreamingDecoderShim(dec)))
    }
}
