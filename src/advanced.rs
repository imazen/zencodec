//! Codec-authoring adapters and advanced execution helpers.
//!
//! Ordinary consumers use the crate root ([`CodecSet`](crate::CodecSet),
//! [`operation::DecodeOptions`](crate::operation::DecodeOptions),
//! [`operation::EncodeOptions`](crate::operation::EncodeOptions)) and the
//! [`decode`](crate::decode) / [`encode`](crate::encode) modules.
//!
//! Codec authors implementing a simple one-shot still-image codec can
//! implement [`StillDecoder`] or [`StillEncoder`] and wrap it in
//! [`StillDecoderConfig`] or [`StillEncoderConfig`] to obtain a complete
//! [`DecoderConfig`] / [`EncoderConfig`] implementation without writing
//! boilerplate job, push-sink, or animation-stub glue.

use alloc::borrow::Cow;
use alloc::vec::Vec;
use zenpixels::{PixelFormat, PixelView};

use crate::decode::{
    Decode, DecodeCapabilities, DecodeJob, DecodeOutput, DecodeRowSink, DecoderConfig, OutputInfo,
    SinkError,
};
use crate::encode::{EncodeCapabilities, EncodeJob, EncodeOutput, Encoder, EncoderConfig};
use crate::{
    ImageFormat, ImageInfo, Metadata, ResourceLimits, SharedPulse, Unsupported,
    UnsupportedOperation,
};

// Re-export advanced codec-authoring and driver primitives in one discoverable place.
pub use crate::decode::{
    BoundaryAction, BoundaryId, ControlError, ControlledIncrementalDecode, ControlledProgress,
    DecodeBoundary, DynControlledDecoder, ErasedIncrementalDecoder, FusedIncrementalDecoder,
    IncrementalDecode, Input, InputRequest, InterpretationFacts, InterpretationState,
    InterpretationUnavailable, Progress, RowStrip, StepError, StripDimensions, erase_controlled,
    erase_incremental,
};
pub use crate::encode::{
    ColorCarriers, EncodeDeclaration, EncodePlan, EncodePlanError, EncodeRows, InputRequirement,
    OrientationPlan, OutputPhase, OutputTiming, PullProgress, RowDemand, RowFillError,
    SourcePasses,
};
pub use crate::helpers;

// =========================================================================
// StillEncoder / StillEncoderConfig
// =========================================================================

/// Simplified authoring trait for a one-shot still-image encoder.
///
/// Wrap an implementation in [`StillEncoderConfig`] to satisfy the full
/// [`EncoderConfig`] → [`EncodeJob`] → [`Encoder`] hierarchy without writing
/// boilerplate job or animation stubs.
///
/// Codecs that support streaming row push, animation, checked metadata
/// policies, or custom fidelity negotiation should implement [`EncoderConfig`]
/// directly.
pub trait StillEncoder: Clone + Send + Sync + 'static {
    /// Codec error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Image format produced by this encoder.
    fn format() -> ImageFormat;

    /// Pixel formats accepted natively by [`encode_still`](Self::encode_still).
    fn supported_pixel_formats() -> &'static [PixelFormat];

    /// Static encoder capabilities.
    fn capabilities() -> &'static EncodeCapabilities;

    /// Reject an unsupported operation using this codec's error type.
    fn reject(op: UnsupportedOperation) -> Self::Error;

    /// Encode one complete still image.
    fn encode_still(
        self,
        pixels: PixelView<'_>,
        metadata: Option<Metadata>,
        limits: ResourceLimits,
        pulse: SharedPulse,
    ) -> Result<EncodeOutput, Self::Error>;
}

/// Adapter that turns any [`StillEncoder`] into a complete [`EncoderConfig`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StillEncoderConfig<T> {
    inner: T,
}

impl<T> StillEncoderConfig<T> {
    /// Wrap a [`StillEncoder`] implementation.
    pub const fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Borrow the underlying still encoder configuration.
    pub const fn inner(&self) -> &T {
        &self.inner
    }

    /// Mutably borrow the underlying still encoder configuration.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwrap the underlying still encoder configuration.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: StillEncoder> EncoderConfig for StillEncoderConfig<T> {
    type Error = T::Error;
    type Job = StillEncodeJob<T>;

    fn format() -> ImageFormat {
        T::format()
    }

    fn supported_pixel_formats() -> &'static [PixelFormat] {
        T::supported_pixel_formats()
    }

    fn capabilities() -> &'static EncodeCapabilities {
        T::capabilities()
    }

    fn job(self) -> Self::Job {
        StillEncodeJob {
            config: self.inner,
            metadata: None,
            limits: ResourceLimits::none(),
            pulse: SharedPulse::default(),
        }
    }
}

/// Per-image encode job produced by [`StillEncoderConfig`].
#[derive(Clone, Debug)]
pub struct StillEncodeJob<T> {
    config: T,
    metadata: Option<Metadata>,
    limits: ResourceLimits,
    pulse: SharedPulse,
}

impl<T: StillEncoder> EncodeJob for StillEncodeJob<T> {
    type Error = T::Error;
    type Enc = StillEncodeExec<T>;
    type AnimationFrameEnc = ();

    fn with_pulse(mut self, pulse: SharedPulse) -> Self {
        self.pulse = pulse;
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn with_unfiltered_metadata(mut self, meta: Metadata) -> Self {
        self.metadata = Some(meta);
        self
    }

    fn encoder(self) -> Result<Self::Enc, Self::Error> {
        Ok(StillEncodeExec {
            config: self.config,
            metadata: self.metadata,
            limits: self.limits,
            pulse: self.pulse,
        })
    }

    fn animation_frame_encoder(self) -> Result<Self::AnimationFrameEnc, Self::Error> {
        Err(T::reject(UnsupportedOperation::AnimationEncode))
    }
}

/// One-shot encoder executor produced by [`StillEncodeJob`].
#[derive(Clone, Debug)]
pub struct StillEncodeExec<T> {
    config: T,
    metadata: Option<Metadata>,
    limits: ResourceLimits,
    pulse: SharedPulse,
}

impl<T: StillEncoder> Encoder for StillEncodeExec<T> {
    type Error = T::Error;

    fn reject(op: UnsupportedOperation) -> Self::Error {
        T::reject(op)
    }

    fn encode(self, pixels: PixelView<'_>) -> Result<EncodeOutput, Self::Error> {
        self.config
            .encode_still(pixels, self.metadata, self.limits, self.pulse)
    }
}

// =========================================================================
// StillDecoder / StillDecoderConfig
// =========================================================================

/// Simplified authoring trait for a one-shot still-image decoder.
///
/// Wrap an implementation in [`StillDecoderConfig`] to satisfy the full
/// [`DecoderConfig`] → [`DecodeJob`] → [`Decode`] hierarchy (including
/// [`DecodeJob::push_decoder`] via [`helpers::copy_decode_to_sink`]) without
/// writing streaming or animation stubs.
///
/// Codecs that support native row streaming, incremental byte-driven decoding,
/// animation, or checked decode policies should implement [`DecoderConfig`]
/// directly.
pub trait StillDecoder: Clone + Send + Sync + 'static {
    /// Codec error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Image formats recognized by this decoder.
    fn formats() -> &'static [ImageFormat];

    /// Pixel formats this decoder can produce natively.
    fn supported_pixel_formats() -> &'static [PixelFormat];

    /// Static decoder capabilities.
    fn capabilities() -> &'static DecodeCapabilities;

    /// Reject an unsupported operation using this codec's error type.
    fn reject(op: UnsupportedOperation) -> Self::Error;

    /// Convert a [`SinkError`] from the default `push_decoder` bridge into
    /// this codec's error type.
    fn wrap_sink_error(err: SinkError) -> Self::Error;

    /// Probe image metadata from header bytes without decoding pixels.
    fn probe_still(&self, data: &[u8], limits: &ResourceLimits) -> Result<ImageInfo, Self::Error>;

    /// Predict decoded dimensions and pixel format without decoding pixels.
    fn output_info_still(
        &self,
        data: &[u8],
        limits: &ResourceLimits,
    ) -> Result<OutputInfo, Self::Error>;

    /// Validate input before constructing the one-shot decoder.
    ///
    /// Defaults to calling [`probe_still`](Self::probe_still).
    fn validate_input(&self, data: &[u8], limits: &ResourceLimits) -> Result<(), Self::Error> {
        self.probe_still(data, limits).map(|_| ())
    }

    /// Decode one complete still image.
    fn decode_still(
        self,
        data: Cow<'_, [u8]>,
        preferred: &[PixelFormat],
        limits: ResourceLimits,
        pulse: SharedPulse,
    ) -> Result<DecodeOutput, Self::Error>;
}

/// Adapter that turns any [`StillDecoder`] into a complete [`DecoderConfig`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StillDecoderConfig<T> {
    inner: T,
}

impl<T> StillDecoderConfig<T> {
    /// Wrap a [`StillDecoder`] implementation.
    pub const fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Borrow the underlying still decoder configuration.
    pub const fn inner(&self) -> &T {
        &self.inner
    }

    /// Mutably borrow the underlying still decoder configuration.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwrap the underlying still decoder configuration.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: StillDecoder> DecoderConfig for StillDecoderConfig<T> {
    type Error = T::Error;
    type Job<'a> = StillDecodeJob<T>;

    fn formats() -> &'static [ImageFormat] {
        T::formats()
    }

    fn supported_pixel_formats() -> &'static [PixelFormat] {
        T::supported_pixel_formats()
    }

    fn capabilities() -> &'static DecodeCapabilities {
        T::capabilities()
    }

    fn job<'a>(self) -> Self::Job<'a> {
        StillDecodeJob {
            config: self.inner,
            limits: ResourceLimits::none(),
            pulse: SharedPulse::default(),
        }
    }
}

/// Per-image decode job produced by [`StillDecoderConfig`].
#[derive(Clone, Debug)]
pub struct StillDecodeJob<T> {
    config: T,
    limits: ResourceLimits,
    pulse: SharedPulse,
}

impl<'a, T: StillDecoder> DecodeJob<'a> for StillDecodeJob<T> {
    type Error = T::Error;
    type Dec = StillDecodeExec<'a, T>;
    type StreamDec = Unsupported<T::Error>;
    type AnimationFrameDec = Unsupported<T::Error>;

    fn with_pulse(mut self, pulse: SharedPulse) -> Self {
        self.pulse = pulse;
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error> {
        self.config.probe_still(data, &self.limits)
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, Self::Error> {
        self.config.output_info_still(data, &self.limits)
    }

    fn decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelFormat],
    ) -> Result<Self::Dec, Self::Error> {
        self.config.validate_input(&data, &self.limits)?;
        Ok(StillDecodeExec {
            config: self.config,
            data,
            preferred: preferred.to_vec(),
            limits: self.limits,
            pulse: self.pulse,
        })
    }

    fn push_decoder(
        self,
        data: Cow<'a, [u8]>,
        sink: &mut dyn DecodeRowSink,
        preferred: &[PixelFormat],
    ) -> Result<OutputInfo, Self::Error> {
        crate::helpers::copy_decode_to_sink(self, data, sink, preferred, T::wrap_sink_error)
    }

    fn streaming_decoder(
        self,
        _data: Cow<'a, [u8]>,
        _preferred: &[PixelFormat],
    ) -> Result<Self::StreamDec, Self::Error> {
        Err(T::reject(UnsupportedOperation::RowLevelDecode))
    }

    fn animation_frame_decoder(
        self,
        _data: Cow<'a, [u8]>,
        _preferred: &[PixelFormat],
    ) -> Result<Self::AnimationFrameDec, Self::Error> {
        Err(T::reject(UnsupportedOperation::AnimationDecode))
    }
}

/// One-shot decoder executor produced by [`StillDecodeJob`].
#[derive(Debug)]
pub struct StillDecodeExec<'a, T> {
    config: T,
    data: Cow<'a, [u8]>,
    preferred: Vec<PixelFormat>,
    limits: ResourceLimits,
    pulse: SharedPulse,
}

impl<T: StillDecoder> Decode for StillDecodeExec<'_, T> {
    type Error = T::Error;

    fn decode(self) -> Result<DecodeOutput, Self::Error> {
        self.config
            .decode_still(self.data, &self.preferred, self.limits, self.pulse)
    }
}
