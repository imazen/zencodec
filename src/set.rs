//! Multi-codec registry: build a [`CodecSet`] once, share it everywhere.
//!
//! A [`CodecSet`] holds any number of decoder and encoder configs behind the
//! object-safe [`DynDecoderConfig`] / [`DynEncoderConfig`] traits and gives
//! them one entry point per operation: detect-and-decode, probe, push decode,
//! animation decode, format-keyed encode, and resource estimation.
//! Registration is one line per codec — each config announces its own formats
//! ([`DecoderConfig::formats()`] / [`EncoderConfig::format()`]), so the set
//! needs no per-codec wiring.
//!
//! ```rust,ignore
//! use zencodec::{CodecSet, ImageFormat};
//!
//! let codecs = CodecSet::new()
//!     .with_decoder(zenjpeg::JpegDecoderConfig::new())
//!     .with_decoder(zenpng::PngDecoderConfig::new())
//!     .with_encoder(zenjpeg::JpegEncoderConfig::new().with_generic_quality(85.0));
//!
//! let image = codecs.decode(&bytes)?;                          // detect → decode
//! let jpeg = codecs.encode(ImageFormat::Jpeg, image.pixels())?; // format-keyed
//! ```
//!
//! # Sharing one set app-wide
//!
//! [`CodecSet`] is `Send + Sync + 'static` and every operation takes `&self`,
//! so build it once and share it for the life of the process — behind a
//! `std::sync::LazyLock` / `OnceLock` static, an `Arc`, or (in `no_std`) a
//! `Box::leak`'d `&'static CodecSet`:
//!
//! ```rust,ignore
//! use std::sync::LazyLock;
//! use zencodec::CodecSet;
//!
//! static CODECS: LazyLock<CodecSet> = LazyLock::new(|| {
//!     CodecSet::new()
//!         .with_decoder(zenjpeg::JpegDecoderConfig::new())
//!         .with_encoder(zenjpeg::JpegEncoderConfig::new())
//! });
//!
//! let image = CODECS.decode(&bytes)?; // any thread, no locking
//! ```
//!
//! Registered encoder configs act as *templates*: codec-specific options are
//! set on the concrete config before registration, and per-call fidelity goes
//! through [`CodecSet::encode_with`], which clones the template internally.
//! [`CodecSet`] is also [`Clone`], so a shared base set can be extended per
//! tenant or per request.
//!
//! # Format detection
//!
//! [`CodecSet::detect`] consults only the formats that have a registered
//! decoder, in the priority order of
//! [`ImageFormatRegistry::common()`](crate::ImageFormatRegistry::common)
//! (which resolves ambiguous ISOBMFF containers — AVIF before HEIC — and DNG
//! before TIFF), followed by any [`ImageFormat::Custom`] formats in
//! registration order. Registration order therefore cannot break the curated
//! magic-byte priority.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;

use zenpixels::{PixelDescriptor, PixelSlice};

use crate::estimate::{ComputeEnvironment, ImageCharacteristics, ResourceEstimate};
use crate::fidelity::Fidelity;
use crate::format::{ImageFormat, ImageFormatRegistry};
use crate::traits::{
    AnimationFrameEncoder, BoxedError, DecoderConfig, DynAnimationFrameDecoder, DynDecodeJob,
    DynDecoderConfig, DynEncodeJob, DynEncoderConfig, DynStreamingDecoder, EncodeJob, Encoder,
    EncoderConfig,
};
use crate::{
    ColorEmitPolicy, DecodeOutput, DecodePolicy, DecodeRowSink, EncodeOutput, EncodePolicy,
    ImageInfo, Metadata, MetadataPolicy, OutputInfo, ResourceLimits, StopToken,
};

// ===========================================================================
// Internal entries — object-safe configs that can also clone themselves
// ===========================================================================
//
// `DynDecoderConfig` / `DynEncoderConfig` are deliberately left untouched
// (adding a required method would break any manual implementor). The set
// instead captures clone/fidelity capability at registration time, where the
// concrete type — and its `Clone` bound from `DecoderConfig`/`EncoderConfig`
// — is still known.

trait DecoderEntry: DynDecoderConfig {
    fn clone_entry(&self) -> Box<dyn DecoderEntry>;
    fn as_dyn(&self) -> &dyn DynDecoderConfig;
}

impl<C> DecoderEntry for C
where
    C: DecoderConfig + 'static,
{
    fn clone_entry(&self) -> Box<dyn DecoderEntry> {
        Box::new(self.clone())
    }
    fn as_dyn(&self) -> &dyn DynDecoderConfig {
        self
    }
}

trait EncoderEntry: DynEncoderConfig {
    fn clone_entry(&self) -> Box<dyn EncoderEntry>;
    fn with_fidelity_entry(self: Box<Self>, fidelity: Fidelity) -> Box<dyn EncoderEntry>;
    fn as_dyn(&self) -> &dyn DynEncoderConfig;
}

impl<C> EncoderEntry for C
where
    C: EncoderConfig + 'static,
    <C::Job as EncodeJob>::Enc: Encoder + Send,
    <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    fn clone_entry(&self) -> Box<dyn EncoderEntry> {
        Box::new(self.clone())
    }
    fn with_fidelity_entry(self: Box<Self>, fidelity: Fidelity) -> Box<dyn EncoderEntry> {
        Box::new((*self).with_fidelity(fidelity))
    }
    fn as_dyn(&self) -> &dyn DynEncoderConfig {
        self
    }
}

// ===========================================================================
// CodecSetError
// ===========================================================================

/// Errors from [`CodecSet`] operations.
///
/// The set-level failures ([`UnrecognizedFormat`](CodecSetError::UnrecognizedFormat),
/// [`NoDecoder`](CodecSetError::NoDecoder), [`NoEncoder`](CodecSetError::NoEncoder))
/// are matchable directly; a failure inside the selected codec is passed
/// through as [`Codec`](CodecSetError::Codec), whose
/// [`source()`](core::error::Error::source) chain exposes the codec's own
/// error (inspect it with [`CodecErrorExt`](crate::CodecErrorExt) /
/// [`find_cause`](crate::find_cause)).
#[derive(Debug)]
#[non_exhaustive]
pub enum CodecSetError {
    /// No registered decoder's format matched the input's magic bytes.
    UnrecognizedFormat,
    /// The format is known, but no decoder for it is registered in this set.
    NoDecoder(ImageFormat),
    /// No encoder for the requested format is registered in this set.
    NoEncoder(ImageFormat),
    /// This convenience path cannot retain all source content or samples.
    /// Use the codec's explicit sequence, gain-map, or pixel conversion API.
    TranscodeRequiresExplicitHandling(TranscodeBarrier),
    /// Operation was cancelled by a stop or pulse token.
    Cancelled,
    /// The selected codec failed.
    Codec(BoxedError),
}

/// Source property that the still-image transcode convenience cannot preserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TranscodeBarrier {
    /// The source contains an animation or independent images.
    Sequence,
    /// The source contains a gain map or another auxiliary image.
    AuxiliaryImage,
    /// The target cannot accept the decoder's native pixel descriptor.
    PixelDescriptor,
}

impl fmt::Display for CodecSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnrecognizedFormat => {
                f.write_str("no registered decoder's format matched the input bytes")
            }
            Self::NoDecoder(fmt_) => write!(f, "no decoder registered for {fmt_}"),
            Self::NoEncoder(fmt_) => write!(f, "no encoder registered for {fmt_}"),
            Self::TranscodeRequiresExplicitHandling(barrier) => {
                write!(f, "transcode requires explicit handling of {barrier:?}")
            }
            Self::Cancelled => f.write_str("operation cancelled"),
            Self::Codec(e) => write!(f, "codec error: {e}"),
        }
    }
}

impl core::error::Error for CodecSetError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(&**e),
            _ => None,
        }
    }
}

impl From<BoxedError> for CodecSetError {
    fn from(e: BoxedError) -> Self {
        Self::Codec(e)
    }
}

// ===========================================================================
// CodecSet
// ===========================================================================

/// A runtime set of codecs with one entry point per operation.
///
/// Register configs with [`with_decoder`](Self::with_decoder) and
/// [`with_encoder`](Self::with_encoder). The set is `Send + Sync`; operations
/// borrow it, so one registration can be shared across calls.
///
/// Operation defaults ([`with_limits`](Self::with_limits),
/// [`with_stop`](Self::with_stop), [`with_decode_policy`](Self::with_decode_policy),
/// [`with_encode_policy`](Self::with_encode_policy)) are stamped onto every
/// job the set creates. Per-operation control (decode hints, metadata,
/// canvas/loop settings) is available through [`decode_job`](Self::decode_job)
/// / [`encode_job`](Self::encode_job), which return the stamped job for
/// further configuration.
pub struct CodecSet {
    decoders: Vec<Box<dyn DecoderEntry>>,
    encoders: Vec<Box<dyn EncoderEntry>>,
    limits: Option<ResourceLimits>,
    stop: Option<StopToken>,
    pulse: Option<how_far::SharedPulse>,
    decode_policy: Option<DecodePolicy>,
    encode_policy: Option<EncodePolicy>,
}

impl CodecSet {
    /// An empty set. Register codecs with [`with_decoder`](Self::with_decoder)
    /// and [`with_encoder`](Self::with_encoder).
    pub fn new() -> Self {
        Self {
            decoders: Vec::new(),
            encoders: Vec::new(),
            limits: None,
            stop: None,
            pulse: None,
            decode_policy: None,
            encode_policy: None,
        }
    }

    // --- Registration -----------------------------------------------------

    /// Register a decoder config. It serves every format in its
    /// [`DecoderConfig::formats()`] list.
    ///
    /// If several registered decoders claim the same format, the first
    /// registered wins.
    pub fn with_decoder(mut self, config: impl DecoderConfig + 'static) -> Self {
        self.decoders.push(Box::new(config));
        self
    }

    /// Register an encoder config for its [`EncoderConfig::format()`].
    ///
    /// The config is a *template*: codec-specific options are set on the
    /// concrete type before registration; per-call fidelity goes through
    /// [`encode_with`](Self::encode_with). If several registered encoders
    /// claim the same format, the first registered wins.
    pub fn with_encoder<C>(mut self, config: C) -> Self
    where
        C: EncoderConfig + 'static,
        <C::Job as EncodeJob>::Enc: Encoder + Send,
        <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
    {
        self.encoders.push(Box::new(config));
        self
    }

    // --- Operation defaults -------------------------------------------------

    /// Resource limits stamped onto every job this set creates.
    pub fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Cooperative-cancellation token stamped onto every job this set creates.
    pub fn with_stop(mut self, stop: StopToken) -> Self {
        self.stop = Some(stop);
        self
    }

    /// Progress and cancellation pulse stamped onto every job this set creates.
    ///
    /// Automatically populates the cancellation token for codecs that only support `Stop`.
    pub fn with_pulse(mut self, pulse: how_far::SharedPulse) -> Self {
        self.stop = Some(StopToken::new(pulse.clone()));
        self.pulse = Some(pulse);
        self
    }

    /// Best-effort decode policy stamped onto every decode job this set creates.
    ///
    /// The codec may ignore this legacy policy setter. For a required policy,
    /// obtain a job with [`decode_job`](Self::decode_job), then call
    /// [`DynDecodeJob::try_set_policy`](crate::decode::DynDecodeJob::try_set_policy).
    pub fn with_decode_policy(mut self, policy: DecodePolicy) -> Self {
        self.decode_policy = Some(policy);
        self
    }

    /// Best-effort encode policy stamped onto every encode job this set creates.
    ///
    /// The codec may ignore this legacy policy setter. For a required policy,
    /// obtain a job with [`encode_job`](Self::encode_job), then call
    /// [`DynEncodeJob::try_set_policy`](crate::encode::DynEncodeJob::try_set_policy).
    pub fn with_encode_policy(mut self, policy: EncodePolicy) -> Self {
        self.encode_policy = Some(policy);
        self
    }

    // --- Queries ------------------------------------------------------------

    /// Whether a decoder for `format` is registered.
    pub fn can_decode(&self, format: ImageFormat) -> bool {
        self.decoder_entry(format).is_some()
    }

    /// Whether an encoder for `format` is registered.
    pub fn can_encode(&self, format: ImageFormat) -> bool {
        self.encoder_entry(format).is_some()
    }

    /// The registered decoder config for `format`, if any.
    pub fn decoder_for(&self, format: ImageFormat) -> Option<&dyn DynDecoderConfig> {
        self.decoder_entry(format).map(|e| e.as_dyn())
    }

    /// The registered encoder config for `format`, if any.
    pub fn encoder_for(&self, format: ImageFormat) -> Option<&dyn DynEncoderConfig> {
        self.encoder_entry(format).map(|e| e.as_dyn())
    }

    /// Formats with a registered decoder, in registration order.
    ///
    /// May repeat a format if several decoders claim it.
    pub fn decodable_formats(&self) -> impl Iterator<Item = ImageFormat> + '_ {
        self.decoders
            .iter()
            .flat_map(|d| d.formats().iter().copied())
    }

    /// Formats with a registered encoder, in registration order.
    pub fn encodable_formats(&self) -> impl Iterator<Item = ImageFormat> + '_ {
        self.encoders.iter().map(|e| e.format())
    }

    /// Detect the input's format from magic bytes, consulting only formats
    /// with a registered decoder.
    ///
    /// Built-in formats are checked in the curated priority order of
    /// [`ImageFormatRegistry::common()`](ImageFormatRegistry::common), then
    /// [`ImageFormat::Custom`] formats from registered decoders in
    /// registration order. Fetch at least
    /// [`ImageFormat::RECOMMENDED_PROBE_BYTES`] for reliable detection.
    pub fn detect(&self, data: &[u8]) -> Option<ImageFormat> {
        for def in ImageFormatRegistry::common().formats() {
            let format = def.to_image_format();
            if self.can_decode(format) && (def.detect)(data) {
                return Some(format);
            }
        }
        for decoder in &self.decoders {
            for &format in decoder.formats() {
                if let ImageFormat::Custom(def) = format
                    && (def.detect)(data)
                {
                    return Some(format);
                }
            }
        }
        None
    }

    // --- Decode -------------------------------------------------------------

    /// Probe image metadata (header parse only) after detecting the format.
    pub fn probe<'a>(&'a self, data: &'a [u8]) -> Result<ImageInfo, CodecSetError> {
        let format = self.detect(data).ok_or(CodecSetError::UnrecognizedFormat)?;
        Ok(self.decode_job(format)?.probe(data)?)
    }

    /// Detect the format and decode to owned pixels in the decoder's native
    /// pixel format.
    pub fn decode<'a>(&'a self, data: &'a [u8]) -> Result<DecodeOutput, CodecSetError> {
        self.decode_preferring(data, &[])
    }

    /// Detect the format and decode, requesting output in one of the
    /// `preferred` pixel formats (ranked; the decoder picks the first it can
    /// produce without lossy conversion, else its native format).
    pub fn decode_preferring<'a>(
        &'a self,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<DecodeOutput, CodecSetError> {
        let format = self.detect(data).ok_or(CodecSetError::UnrecognizedFormat)?;
        self.decode_as(format, data, preferred)
    }

    /// Decode `data` as `format`, skipping detection.
    pub fn decode_as<'a>(
        &'a self,
        format: ImageFormat,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<DecodeOutput, CodecSetError> {
        Ok(self
            .decode_job(format)?
            .into_decoder(Cow::Borrowed(data), preferred)?
            .decode()?)
    }

    /// Detect the format and decode into a caller-owned sink (push model, the
    /// most memory-efficient path).
    pub fn push_decode<'a>(
        &'a self,
        data: &'a [u8],
        sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, CodecSetError> {
        let format = self.detect(data).ok_or(CodecSetError::UnrecognizedFormat)?;
        Ok(self
            .decode_job(format)?
            .push_decode(Cow::Borrowed(data), sink, preferred)?)
    }

    /// Detect the format and create a full-frame animation decoder.
    ///
    /// The returned decoder owns its data (`'static`) and outlives both the
    /// set borrow and `data`.
    pub fn animation_decoder<'a>(
        &'a self,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynAnimationFrameDecoder>, CodecSetError> {
        let format = self.detect(data).ok_or(CodecSetError::UnrecognizedFormat)?;
        Ok(self
            .decode_job(format)?
            .into_animation_frame_decoder(Cow::Borrowed(data), preferred)?)
    }

    /// Detect the format and create a pull-streaming decoder that yields
    /// scanline batches.
    ///
    /// The returned decoder borrows this set (and `data`); a set stored in a
    /// `static` therefore yields a `'static` decoder from `'static` data. Not
    /// every codec supports pull streaming — those return an error (use
    /// [`push_decode`](Self::push_decode) instead).
    pub fn streaming_decoder<'a>(
        &'a self,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<Box<dyn DynStreamingDecoder + 'a>, CodecSetError> {
        let format = self.detect(data).ok_or(CodecSetError::UnrecognizedFormat)?;
        Ok(self
            .decode_job(format)?
            .into_streaming_decoder(Cow::Borrowed(data), preferred)?)
    }

    /// Create an incremental (sans-I/O) decoder that detects the format from
    /// the first bytes offered and delegates to the registered decoder.
    ///
    /// While probing it asks for the first
    /// [`ImageFormat::RECOMMENDED_PROBE_BYTES`] (or the whole source if
    /// shorter), takes none of them, and hands the same input to the
    /// detected decoder. Before detection it reports the most demanding
    /// access pattern among the registered decoders.
    pub fn incremental_decoder(
        &self,
        preferred: &[PixelDescriptor],
    ) -> Box<crate::traits::DynIncrementalDecoder> {
        crate::traits::erase_incremental(CodecSetIncrementalDecoder::new(self.clone(), preferred))
    }

    /// Decode from a random-access source into `sink`, detecting the format
    /// from the source's first bytes.
    ///
    /// Probes offset 0 with a short read (at most
    /// [`ImageFormat::RECOMMENDED_PROBE_BYTES`] plus headroom), then dispatches to
    /// the detected codec's
    /// [`push_decoder_from_source`](DynDecodeJob::push_decoder_from_source).
    /// A codec without the `seekable` capability reports
    /// [`UnsupportedOperation::SeekableDecode`](crate::UnsupportedOperation).
    pub fn push_decode_from_source(
        &self,
        source: &mut dyn crate::source::DynReadAt,
        sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, BoxedError> {
        // A short read (not `len()`) bounds the probe, so a stream-backed
        // source is not consumed to its end before decoding starts.
        let mut probe = alloc::vec![0u8; ImageFormat::RECOMMENDED_PROBE_BYTES + 1024];
        let n = source.read_at(0, &mut probe)?;
        probe.truncate(n);
        let format = self
            .detect(&probe)
            .ok_or_else(|| Box::new(CodecSetError::UnrecognizedFormat) as BoxedError)?;
        self.decode_job(format)
            .map_err(|e| Box::new(e) as BoxedError)?
            .push_decoder_from_source(source, sink, preferred)
    }

    /// A decode job for `format` with this set's defaults stamped on.
    ///
    /// The escape hatch behind every decode convenience: set decode hints
    /// (crop, orientation, gain map, start frame) on the job, then call one of
    /// its `into_*` executors or [`probe`](DynDecodeJob::probe).
    pub fn decode_job<'a>(
        &'a self,
        format: ImageFormat,
    ) -> Result<Box<dyn DynDecodeJob<'a> + 'a>, CodecSetError> {
        let entry = self
            .decoder_entry(format)
            .ok_or(CodecSetError::NoDecoder(format))?;
        let mut job = entry.dyn_job();
        if let Some(limits) = self.limits {
            job.set_limits(limits);
        }
        if let Some(pulse) = &self.pulse {
            job.set_pulse(pulse.clone());
        } else if let Some(stop) = &self.stop {
            job.set_stop(stop.clone());
        }
        if let Some(policy) = self.decode_policy {
            job.set_policy(policy);
        }
        Ok(job)
    }

    pub(crate) fn checked_decode_job<'a>(
        &'a self,
        format: ImageFormat,
        policy: Option<DecodePolicy>,
    ) -> Result<Box<dyn DynDecodeJob<'a> + 'a>, CodecSetError> {
        let entry = self
            .decoder_entry(format)
            .ok_or(CodecSetError::NoDecoder(format))?;
        let mut job = entry.dyn_job();
        if let Some(limits) = self.limits {
            job.set_limits(limits);
        }
        if let Some(pulse) = &self.pulse {
            job.set_pulse(pulse.clone());
        } else if let Some(stop) = &self.stop {
            job.set_stop(stop.clone());
        }
        if let Some(policy) = policy.or(self.decode_policy) {
            job.try_set_policy(policy)?;
        }
        Ok(job)
    }

    pub(crate) fn checked_encode_job(
        &self,
        format: ImageFormat,
        fidelity: Option<Fidelity>,
        policy: Option<EncodePolicy>,
    ) -> Result<Box<dyn DynEncodeJob>, CodecSetError> {
        let config = self
            .encoder_entry(format)
            .ok_or(CodecSetError::NoEncoder(format))?
            .as_dyn();
        let mut job = match fidelity {
            Some(target) => config
                .try_fidelity_job(target)
                .map_err(|e| CodecSetError::Codec(Box::new(e)))?,
            None => config.dyn_job(),
        };
        if let Some(limits) = self.limits {
            job.set_limits(limits);
        }
        if let Some(pulse) = &self.pulse {
            job.set_pulse(pulse.clone());
        } else if let Some(stop) = &self.stop {
            job.set_stop(stop.clone());
        }
        if let Some(policy) = policy.or(self.encode_policy) {
            job.try_set_policy(policy)?;
        }
        Ok(job)
    }

    // --- Encode -------------------------------------------------------------

    /// Encode `pixels` to `format` using the registered encoder template.
    pub fn encode(
        &self,
        format: ImageFormat,
        pixels: PixelSlice<'_>,
    ) -> Result<EncodeOutput, CodecSetError> {
        Ok(self.encode_job(format)?.encode(pixels)?)
    }

    /// Encode `pixels` to `format` at a per-call [`Fidelity`], overriding the
    /// template's quality/lossless setting.
    ///
    /// Clones the registered template internally; codec-specific options set
    /// at registration are preserved.
    pub fn encode_with(
        &self,
        format: ImageFormat,
        fidelity: Fidelity,
        pixels: PixelSlice<'_>,
    ) -> Result<EncodeOutput, CodecSetError> {
        Ok(self.encode_job_with(format, fidelity)?.encode(pixels)?)
    }

    /// An encode job for `format` with this set's defaults stamped on.
    ///
    /// The escape hatch behind the encode conveniences: set metadata, canvas
    /// size, or loop count on the job, then call
    /// [`into_encoder`](DynEncodeJob::into_encoder) (one-shot or
    /// `push_rows`/`finish` streaming) or
    /// [`into_animation_frame_encoder`](DynEncodeJob::into_animation_frame_encoder).
    pub fn encode_job(&self, format: ImageFormat) -> Result<Box<dyn DynEncodeJob>, CodecSetError> {
        let entry = self
            .encoder_entry(format)
            .ok_or(CodecSetError::NoEncoder(format))?;
        Ok(self.stamped_encode_job(entry.as_dyn()))
    }

    /// Like [`encode_job`](Self::encode_job), with a per-call [`Fidelity`]
    /// applied to a clone of the template first.
    pub fn encode_job_with(
        &self,
        format: ImageFormat,
        fidelity: Fidelity,
    ) -> Result<Box<dyn DynEncodeJob>, CodecSetError> {
        let entry = self
            .encoder_entry(format)
            .ok_or(CodecSetError::NoEncoder(format))?;
        let tuned = entry.clone_entry().with_fidelity_entry(fidelity);
        Ok(self.stamped_encode_job(tuned.as_dyn()))
    }

    /// Decode `input` and re-encode it to `target` at `fidelity` — the one-call
    /// recompress / reformat, carrying the source's metadata under an explicit
    /// retention policy and applying a color-emission policy.
    ///
    /// The headline proxy operation: `decode → carry source metadata → encode`.
    /// The source's ICC / EXIF / XMP / CICP / HDR / orientation are read off the
    /// decode result and offered to the encoder, filtered by `metadata`
    /// ([`Web`](MetadataPolicy::Web) strips GPS / camera / timestamps / XMP and
    /// keeps orientation + rights + color; [`PreserveExact`](MetadataPolicy::PreserveExact)
    /// requests all carriers). An encoder may lack a requested carrier; this
    /// method does not certify byte-exact metadata preservation. `color`
    /// ([`ColorEmitPolicy`]) governs how
    /// color *signaling* (ICC vs CICP) is emitted — it is **not** a pixel-level
    /// color-space conversion. The color policy merges onto the set-level
    /// [`EncodePolicy`] (overriding only its color field).
    ///
    /// # Scope and failure modes
    ///
    /// - **Single image** — rejects known animation and multi-image sources
    ///   rather than silently selecting their first image. For animation use
    ///   [`animation_decoder`](Self::animation_decoder) + [`encode_job`](Self::encode_job).
    /// - **No pixel processing** — recompresses at the decoded dimensions and
    ///   pixel format; no resize, crop, or CMS conversion.
    /// - **No implicit descriptor downgrade** — if the target cannot accept
    ///   the decoded descriptor, use an explicit pixel conversion or a
    ///   codec-specific encode path. The registered decoder is not re-run at a
    ///   lower precision behind the caller's back.
    /// - **Fidelity and carrier support are best-effort** — the existing
    ///   `with_fidelity`, `with_metadata_policy` and `with_policy` contracts do
    ///   not report whether every request was honored. Inspect the selected
    ///   codec's capabilities or use its checked API when a property is required.
    ///
    /// Errors with [`NoDecoder`](CodecSetError::NoDecoder) when `input`'s
    /// detected format has no registered decoder, [`NoEncoder`](CodecSetError::NoEncoder)
    /// when `target` has no registered encoder, and forwards any decode or
    /// encode error.
    pub fn transcode(
        &self,
        input: &[u8],
        target: ImageFormat,
        fidelity: Fidelity,
        metadata: MetadataPolicy,
        color: ColorEmitPolicy,
    ) -> Result<EncodeOutput, CodecSetError> {
        let target_inputs = self
            .encoder_for(target)
            .ok_or(CodecSetError::NoEncoder(target))?
            .supported_descriptors();
        // This is a still-image convenience. Reject known source content that
        // cannot travel through a single PixelSlice before decoding samples.
        let format = self
            .detect(input)
            .ok_or(CodecSetError::UnrecognizedFormat)?;
        let source = self.decode_job(format)?.probe_full(input)?;
        if !matches!(source.sequence, crate::ImageSequence::Single) {
            return Err(CodecSetError::TranscodeRequiresExplicitHandling(
                TranscodeBarrier::Sequence,
            ));
        }
        if source.gain_map.is_present()
            || source.supplements.gain_map
            || source.supplements.depth_map
            || source.supplements.segmentation_mattes
            || source.supplements.auxiliary
        {
            return Err(CodecSetError::TranscodeRequiresExplicitHandling(
                TranscodeBarrier::AuxiliaryImage,
            ));
        }
        let decoded = self.decode(input)?;
        if !target_inputs.contains(&decoded.pixels().descriptor()) {
            return Err(CodecSetError::TranscodeRequiresExplicitHandling(
                TranscodeBarrier::PixelDescriptor,
            ));
        }
        let source_metadata = Metadata::from(decoded.info());
        let mut job = self.encode_job_with(target, fidelity)?;
        job.set_metadata_policy(source_metadata, metadata);
        job.set_policy(
            self.encode_policy
                .unwrap_or_else(EncodePolicy::none)
                .with_color(color),
        );
        Ok(job.encode(decoded.pixels())?)
    }

    // --- Resource estimation ------------------------------------------------

    /// Predict the peak memory and wall-time of **encoding** an image with the
    /// given [`ImageCharacteristics`] as `format` on `compute`, without
    /// encoding anything.
    ///
    /// Forwards to the registered encoder's
    /// [`estimate_encode_resources`](DynEncoderConfig::estimate_encode_resources).
    /// A codec without a calibrated cost model returns
    /// [`ResourceEstimate::unknown`] (every field `None`) rather than failing;
    /// the call errors only with [`NoEncoder`](CodecSetError::NoEncoder) when no
    /// encoder for `format` is registered.
    ///
    /// Unlike [`probe`](Self::probe), this takes explicit characteristics
    /// instead of input bytes: an encode has no input image yet, so the caller
    /// describes the pixels it is about to hand the encoder.
    pub fn estimate_encode(
        &self,
        format: ImageFormat,
        image: &ImageCharacteristics,
        compute: &ComputeEnvironment,
    ) -> Result<ResourceEstimate, CodecSetError> {
        Ok(self
            .encoder_for(format)
            .ok_or(CodecSetError::NoEncoder(format))?
            .estimate_encode_resources(image, compute))
    }

    /// Predict encode resources for the `pixels` you are about to hand the
    /// encoder, reading their dimensions and format straight off the slice —
    /// the pixels-based counterpart to [`estimate_encode`](Self::estimate_encode)
    /// (as [`estimate_decode_of`](Self::estimate_decode_of) is to
    /// [`estimate_decode`](Self::estimate_decode)).
    ///
    /// Saves building an [`ImageCharacteristics`] by hand when you already hold
    /// the pixel slice. Errors only with [`NoEncoder`](CodecSetError::NoEncoder)
    /// when no encoder for `format` is registered.
    pub fn estimate_encode_of(
        &self,
        format: ImageFormat,
        pixels: PixelSlice<'_>,
        compute: &ComputeEnvironment,
    ) -> Result<ResourceEstimate, CodecSetError> {
        let image = ImageCharacteristics::new(pixels.width(), pixels.rows(), pixels.descriptor());
        self.estimate_encode(format, &image, compute)
    }

    /// Predict the peak memory and wall-time of **decoding** an image with the
    /// given [`ImageCharacteristics`] as `format` on `compute`, without
    /// decoding anything.
    ///
    /// Forwards to the registered decoder's
    /// [`estimate_decode_resources`](DynDecoderConfig::estimate_decode_resources).
    /// A codec without a calibrated cost model returns
    /// [`ResourceEstimate::unknown`]; the call errors only with
    /// [`NoDecoder`](CodecSetError::NoDecoder) when no decoder for `format` is
    /// registered.
    ///
    /// To estimate straight from encoded bytes, pair with [`probe`](Self::probe):
    /// probe for the real dimensions and pixel format, build
    /// [`ImageCharacteristics`] from them, then call this.
    pub fn estimate_decode(
        &self,
        format: ImageFormat,
        image: &ImageCharacteristics,
        compute: &ComputeEnvironment,
    ) -> Result<ResourceEstimate, CodecSetError> {
        Ok(self
            .decoder_for(format)
            .ok_or(CodecSetError::NoDecoder(format))?
            .estimate_decode_resources(image, compute))
    }

    /// Predict the peak memory and wall-time of **decoding** `data`, probing it
    /// for the format and dimensions first — the bytes-based counterpart to
    /// [`estimate_decode`](Self::estimate_decode), as convenient as
    /// [`probe`](Self::probe).
    ///
    /// This detects and header-parses the input like [`probe`](Self::probe)
    /// (same `UnrecognizedFormat` / codec errors), then estimates a still frame
    /// at the probed canvas dimensions in the decoder's native output format —
    /// its first
    /// [`supported_descriptors`](DynDecoderConfig::supported_descriptors), which
    /// carries the real channel count and bit depth (falling back to RGBA8 /
    /// RGB8 from the probed alpha only if the decoder lists none). Animations
    /// are costed as one canvas frame; for per-frame or full-sequence cost, or
    /// a specific output pixel format, build [`ImageCharacteristics`] yourself
    /// and call [`estimate_decode`](Self::estimate_decode).
    pub fn estimate_decode_of(
        &self,
        data: &[u8],
        compute: &ComputeEnvironment,
    ) -> Result<ResourceEstimate, CodecSetError> {
        // Key off the *detected* (registration) format, which is what
        // `decoder_for` / `estimate_decode` index on — not `info.format`, the
        // format the decoder *reports*, which can differ (e.g. a decoder
        // registered under a custom format).
        let format = self.detect(data).ok_or(CodecSetError::UnrecognizedFormat)?;
        let info = self.decode_job(format)?.probe(data)?;
        let descriptor = self
            .decoder_for(format)
            .and_then(|d| d.supported_descriptors().first().copied())
            .unwrap_or(if info.has_alpha {
                PixelDescriptor::RGBA8_SRGB
            } else {
                PixelDescriptor::RGB8_SRGB
            });
        let image = ImageCharacteristics::new(info.width, info.height, descriptor);
        self.estimate_decode(format, &image, compute)
    }

    // --- Internals ----------------------------------------------------------

    fn decoder_entry(&self, format: ImageFormat) -> Option<&dyn DecoderEntry> {
        self.decoders
            .iter()
            .find(|d| d.formats().contains(&format))
            .map(|b| &**b)
    }

    fn encoder_entry(&self, format: ImageFormat) -> Option<&dyn EncoderEntry> {
        self.encoders
            .iter()
            .find(|e| e.format() == format)
            .map(|b| &**b)
    }

    fn stamped_encode_job(&self, config: &dyn DynEncoderConfig) -> Box<dyn DynEncodeJob> {
        let mut job = config.dyn_job();
        if let Some(limits) = self.limits {
            job.set_limits(limits);
        }
        if let Some(pulse) = &self.pulse {
            job.set_pulse(pulse.clone());
        } else if let Some(stop) = &self.stop {
            job.set_stop(stop.clone());
        }
        if let Some(policy) = self.encode_policy {
            job.set_policy(policy);
        }
        job
    }
}

impl Default for CodecSet {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for CodecSet {
    fn clone(&self) -> Self {
        Self {
            decoders: self.decoders.iter().map(|d| d.clone_entry()).collect(),
            encoders: self.encoders.iter().map(|e| e.clone_entry()).collect(),
            limits: self.limits,
            stop: self.stop.clone(),
            pulse: self.pulse.clone(),
            decode_policy: self.decode_policy,
            encode_policy: self.encode_policy,
        }
    }
}

impl fmt::Debug for CodecSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let decodable: Vec<ImageFormat> = self.decodable_formats().collect();
        let encodable: Vec<ImageFormat> = self.encodable_formats().collect();
        f.debug_struct("CodecSet")
            .field("decodable", &decodable)
            .field("encodable", &encodable)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

// ===========================================================================
// CodecSetIncrementalDecoder
// ===========================================================================

/// Format-detecting incremental decoder: asks for the stream prefix until a
/// registered format is recognised, then delegates every step to that
/// codec's incremental decoder. It never copies input.
pub struct CodecSetIncrementalDecoder {
    set: CodecSet,
    preferred: Vec<PixelDescriptor>,
    inner: Option<Box<crate::traits::DynIncrementalDecoder>>,
    probe: [crate::traits::Request; 1],
}

impl CodecSetIncrementalDecoder {
    /// Create a new incremental decoder tied to `set`.
    pub fn new(set: CodecSet, preferred: &[PixelDescriptor]) -> Self {
        Self {
            set,
            preferred: preferred.to_vec(),
            inner: None,
            probe: [crate::traits::Request::at_least(
                0,
                ImageFormat::RECOMMENDED_PROBE_BYTES,
            )],
        }
    }
}

impl crate::traits::IncrementalDecode for CodecSetIncrementalDecoder {
    type Error = CodecSetError;

    fn step(
        &mut self,
        input: crate::traits::Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn enough::Stop,
    ) -> Result<crate::traits::Progress, crate::traits::StepError<CodecSetError>> {
        use crate::traits::{Progress, StepError};
        for set_stop in [
            self.set.pulse.as_ref().map(|p| p as &dyn enough::Stop),
            self.set.stop.as_ref().map(|s| s as &dyn enough::Stop),
        ]
        .into_iter()
        .flatten()
        {
            set_stop.check()?;
        }
        if self.inner.is_none() {
            if input.offset != 0 {
                return Ok(Progress::need(0));
            }
            let detected = self.set.detect(input.bytes);
            let complete = self.probe[0].is_satisfied_by(&input);
            let conclusive = match detected {
                // TIFF magic is short and shared with TIFF-based raws: look
                // at more of the file before committing.
                Some(ImageFormat::Tiff) => complete,
                Some(_) => true,
                None => false,
            };
            if !conclusive {
                if complete {
                    return Err(StepError::Codec(CodecSetError::UnrecognizedFormat));
                }
                return Ok(Progress::need(0));
            }
            let format = detected.expect("conclusive implies detected");
            let job = self.set.decode_job(format).map_err(StepError::Codec)?;
            let inner = job
                .into_incremental_decoder(&self.preferred)
                .map_err(|e| StepError::Codec(CodecSetError::Codec(e)))?;
            self.inner = Some(inner);
        }
        let inner = self.inner.as_mut().expect("set above");
        inner
            .step(input, sink, stop)
            .map_err(|e| e.map_codec(|c| CodecSetError::Codec(c.into_inner())))
    }

    fn requested(&self) -> &[crate::traits::Request] {
        match &self.inner {
            Some(inner) => inner.requested(),
            None => &self.probe,
        }
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.inner.as_ref().and_then(|d| d.info())
    }

    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.inner.as_ref().and_then(|d| d.output_descriptor())
    }

    fn access_pattern(&self) -> crate::source::AccessPattern {
        match &self.inner {
            Some(inner) => inner.access_pattern(),
            None => self
                .set
                .decoders
                .iter()
                .map(|d| d.capabilities().access_pattern())
                .fold(crate::source::AccessPattern::Sequential, |a, b| {
                    a.most_demanding(b)
                }),
        }
    }
}

// ===========================================================================
// Tests (codec-free; end-to-end behavior tests live in zencodec-testkit)
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync_static<T: Send + Sync + 'static>() {}

    #[test]
    fn codec_set_is_send_sync_static() {
        assert_send_sync_static::<CodecSet>();
        assert_send_sync_static::<CodecSetError>();
    }

    #[test]
    fn empty_set_detects_nothing() {
        let set = CodecSet::new();
        // Valid JPEG magic, but no decoder registered for it.
        assert_eq!(set.detect(&[0xFF, 0xD8, 0xFF, 0xE0]), None);
        assert!(!set.can_decode(ImageFormat::Jpeg));
        assert!(!set.can_encode(ImageFormat::Jpeg));
        assert_eq!(set.decodable_formats().count(), 0);
        assert_eq!(set.encodable_formats().count(), 0);
    }

    #[test]
    fn empty_set_decode_is_unrecognized() {
        let set = CodecSet::new();
        match set.decode(&[0xFF, 0xD8, 0xFF, 0xE0]) {
            Err(CodecSetError::UnrecognizedFormat) => {}
            other => panic!("expected UnrecognizedFormat, got {other:?}"),
        }
    }

    #[test]
    fn decode_as_without_decoder_is_no_decoder() {
        let set = CodecSet::new();
        match set.decode_as(ImageFormat::Jpeg, &[0xFF, 0xD8], &[]) {
            Err(CodecSetError::NoDecoder(ImageFormat::Jpeg)) => {}
            other => panic!("expected NoDecoder(Jpeg), got {other:?}"),
        }
    }

    #[test]
    fn encode_without_encoder_is_no_encoder() {
        let set = CodecSet::new();
        let bytes = [0u8; 3];
        let pixels =
            PixelSlice::new(&bytes, 1, 1, 3, PixelDescriptor::RGB8_SRGB).expect("pixel slice");
        match set.encode(ImageFormat::Png, pixels) {
            Err(CodecSetError::NoEncoder(ImageFormat::Png)) => {}
            other => panic!("expected NoEncoder(Png), got {other:?}"),
        }
    }

    #[test]
    fn empty_set_clone_and_debug() {
        let set = CodecSet::new()
            .with_limits(ResourceLimits::default())
            .clone();
        let dbg = alloc::format!("{set:?}");
        assert!(dbg.contains("CodecSet"));
    }

    #[test]
    fn error_display_names_format() {
        let msg = alloc::format!("{}", CodecSetError::NoDecoder(ImageFormat::Jpeg));
        assert!(msg.contains("JPEG"), "{msg}");
        let msg = alloc::format!("{}", CodecSetError::NoEncoder(ImageFormat::WebP));
        assert!(msg.contains("WebP"), "{msg}");
    }
}
