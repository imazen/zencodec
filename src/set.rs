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
//! tenant or per request. Cloning shares the immutable registration lists and
//! templates; extending a clone copies the lists on write. It does not clone
//! every native configuration. Creating
//! an operation still clones its selected config into an independently owned job.
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
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use zenpixels::{PixelFormat, PixelView};

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
    ImageInfo, Metadata, MetadataPolicy, OutputInfo, ResourceLimits,
};

mod collection;
pub mod role;

pub use role::{
    EncodedTransformFactory, EncodedTransformRole, FactoryRole, ImageDecodeRole,
    ImageEncodeFactory, ImageEncodeRole, NamedFactory, RegistrationError, RegistrationId,
};

// ===========================================================================
// Internal entries — shared templates with per-operation config cloning
// ===========================================================================
//
// `DynDecoderConfig` / `DynEncoderConfig` are deliberately left untouched
// (adding a required method would break any manual implementor). The set
// instead captures clone/fidelity/Send capability at registration time, where
// the concrete type — and its `Clone` bound from `DecoderConfig`/`EncoderConfig`
// — is still known.

struct LocalImageEncodeFactory<C>(Arc<C>);

impl<C> ImageEncodeFactory for LocalImageEncodeFactory<C>
where
    C: EncoderConfig + 'static,
    <C::Job as EncodeJob>::Enc: Encoder + Send,
    <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    fn config(&self) -> &dyn DynEncoderConfig {
        &*self.0
    }
}

// Registration captures Send while the concrete job type is still known.
// Local entries remain available for thread-affine jobs.
struct SendImageEncodeFactory<C>(Arc<C>);

impl<C> ImageEncodeFactory for SendImageEncodeFactory<C>
where
    C: EncoderConfig + 'static,
    C::Job: Send,
    <C::Job as EncodeJob>::Enc: Encoder + Send,
    <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    fn config(&self) -> &dyn DynEncoderConfig {
        &*self.0
    }

    fn supports_send_job(&self) -> bool {
        true
    }

    fn send_job(
        &self,
        fidelity: Option<Fidelity>,
    ) -> Result<Box<dyn DynEncodeJob + Send>, CodecSetError> {
        let config = match fidelity {
            Some(target) => (*self.0)
                .clone()
                .try_with_fidelity(target)
                .map_err(|error| CodecSetError::Codec(Box::new(error)))?,
            None => (*self.0).clone(),
        };
        Ok(crate::traits::erase_send_job(config.job()))
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
    /// A transferable job was required, but this entry used local registration.
    /// Register it with `CodecSet::with_send_encoder` when its job is Send.
    EncoderJobNotSend(ImageFormat),
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
    PixelFormat,
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
            Self::EncoderJobNotSend(format) => write!(
                f,
                "encoder job for {format} was not registered with with_send_encoder"
            ),
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

#[derive(Clone)]
struct RegisteredDecoder {
    config: Arc<dyn DynDecoderConfig>,
    admitted_formats: Vec<ImageFormat>,
}

#[derive(Clone, Default)]
struct Registrations {
    decoders: Vec<RegisteredDecoder>,
    encoders: Vec<Arc<dyn ImageEncodeFactory>>,
    collections: Vec<Arc<dyn crate::collection::CollectionFactory>>,
    roles: role::RoleStore,
}

/// A runtime set of codecs with one entry point per operation.
///
/// Register configs with [`with_decoder`](Self::with_decoder) and
/// [`with_encoder`](Self::with_encoder). The set is `Send + Sync`; operations
/// borrow it, so one registration can be shared across calls.
///
/// Operation defaults ([`with_limits`](Self::with_limits),
/// [`with_pulse`](Self::with_pulse), [`with_decode_policy`](Self::with_decode_policy),
/// [`with_encode_policy`](Self::with_encode_policy)) are stamped onto every
/// job the set creates. Per-operation control (decode hints, metadata,
/// canvas/loop settings) is available through [`decode_job`](Self::decode_job)
/// / [`encode_job`](Self::encode_job), which return the stamped job for
/// further configuration.
pub struct CodecSet {
    registrations: Arc<Registrations>,
    limits: Option<ResourceLimits>,
    pulse: Option<how_far::SharedPulse>,
    decode_policy: Option<DecodePolicy>,
    encode_policy: Option<EncodePolicy>,
}

impl CodecSet {
    /// An empty set. Register codecs with [`with_decoder`](Self::with_decoder)
    /// and [`with_encoder`](Self::with_encoder).
    pub fn new() -> Self {
        Self {
            registrations: Arc::new(Registrations::default()),
            limits: None,
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
        let admitted_formats = config.formats().to_vec();
        Arc::make_mut(&mut self.registrations)
            .decoders
            .push(RegisteredDecoder {
                config: Arc::new(config),
                admitted_formats,
            });
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
        Arc::make_mut(&mut self.registrations)
            .encoders
            .push(Arc::new(LocalImageEncodeFactory(Arc::new(config))));
        self
    }

    /// Register an encoder whose prepared jobs may move between threads.
    ///
    /// Use this for async operations that retain a job across an I/O wait.
    /// Existing `with_encoder` keeps supporting thread-affine jobs. Both have
    /// the same first-registration-wins rule and work with synchronous calls.
    pub fn with_send_encoder<C>(mut self, config: C) -> Self
    where
        C: EncoderConfig + 'static,
        C::Job: Send,
        <C::Job as EncodeJob>::Enc: Encoder + Send,
        <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
    {
        Arc::make_mut(&mut self.registrations)
            .encoders
            .push(Arc::new(SendImageEncodeFactory(Arc::new(config))));
        self
    }

    /// Register a typed provider factory under role `R` serving `keys`.
    ///
    /// Rejects empty `keys` ([`RegistrationError::EmptyKeys`]) and duplicate
    /// [`RegistrationId`]s within the same role ([`RegistrationError::DuplicateId`]).
    /// Repeated keys in a single registration are deduplicated in first-seen order.
    /// Downstream crates (such as `zencodec-media`) define roles without modifying
    /// `zencodec`.
    ///
    /// When `R` is [`ImageDecodeRole`] or [`ImageEncodeRole`], this method also
    /// appends the provider to the legacy first-wins decoder/encoder list so
    /// [`detect`](Self::detect), [`can_decode`](Self::can_decode), and
    /// [`can_encode`](Self::can_encode) stay synchronized with named registrations.
    /// Prefer [`with_named_decoder`](Self::with_named_decoder),
    /// [`with_named_encoder`](Self::with_named_encoder), and
    /// [`with_named_send_encoder`](Self::with_named_send_encoder) for concrete image
    /// codec configs so keys always match the config's declared formats.
    pub fn with_role<R: FactoryRole>(
        mut self,
        keys: impl IntoIterator<Item = R::Key>,
        entry: NamedFactory<R>,
    ) -> Result<Self, RegistrationError> {
        let mut collected: Vec<R::Key> = Vec::new();
        for key in keys {
            if !collected.contains(&key) {
                collected.push(key);
            }
        }
        if collected.is_empty() {
            return Err(RegistrationError::EmptyKeys);
        }

        // Validate image role keys against each provider's declared formats so ranked
        // and legacy tables share the exact same admitted format authority.
        let mirror_decoder = if let Some(dec) =
            (&entry as &dyn core::any::Any).downcast_ref::<NamedFactory<ImageDecodeRole>>()
        {
            let supported = dec.factory.formats();
            let mut admitted = Vec::with_capacity(collected.len());
            for key in &collected {
                let fmt = *(key as &dyn core::any::Any)
                    .downcast_ref::<ImageFormat>()
                    .expect("ImageDecodeRole::Key is ImageFormat");
                if !supported.contains(&fmt) {
                    return Err(RegistrationError::UnsupportedKey);
                }
                admitted.push(fmt);
            }
            Some(RegisteredDecoder {
                config: Arc::clone(&dec.factory),
                admitted_formats: admitted,
            })
        } else {
            None
        };
        let mirror_encoder = if let Some(enc) =
            (&entry as &dyn core::any::Any).downcast_ref::<NamedFactory<ImageEncodeRole>>()
        {
            let supported = enc.factory.config().format();
            for key in &collected {
                let fmt = *(key as &dyn core::any::Any)
                    .downcast_ref::<ImageFormat>()
                    .expect("ImageEncodeRole::Key is ImageFormat");
                if fmt != supported {
                    return Err(RegistrationError::UnsupportedKey);
                }
            }
            Some(Arc::clone(&enc.factory))
        } else {
            None
        };

        let regs = Arc::make_mut(&mut self.registrations);
        regs.roles.insert::<R>(collected, entry)?;
        if let Some(dec) = mirror_decoder {
            regs.decoders.push(dec);
        }
        if let Some(enc) = mirror_encoder {
            regs.encoders.push(enc);
        }
        Ok(self)
    }

    /// Iterate over all named factories registered for `key` under role `R`, in
    /// registration order.
    ///
    /// Clones `key` into the iterator so the returned iterator borrows only `self`.
    pub fn factories<R: FactoryRole>(
        &self,
        key: &R::Key,
    ) -> impl Iterator<Item = &NamedFactory<R>> + '_ {
        let target = key.clone();
        self.registrations
            .roles
            .table::<R>()
            .into_iter()
            .flat_map(|t| t.entries.iter())
            .filter(move |(k, _)| k == &target)
            .map(|(_, entry)| entry)
    }

    /// Iterate over every `(key, factory)` pair registered under role `R`, in
    /// registration order.
    pub fn role_entries<R: FactoryRole>(
        &self,
    ) -> impl Iterator<Item = (&R::Key, &NamedFactory<R>)> + '_ {
        self.registrations
            .roles
            .table::<R>()
            .into_iter()
            .flat_map(|t| t.entries.iter())
            .map(|(k, entry)| (k, entry))
    }

    /// Register a named decoder config both in [`ImageDecodeRole`] for multi-provider
    /// routing and in the legacy first-wins decoder list.
    pub fn with_named_decoder<C>(
        self,
        id: RegistrationId,
        revision: &'static str,
        config: C,
    ) -> Result<Self, RegistrationError>
    where
        C: DecoderConfig + 'static,
    {
        let keys: Vec<ImageFormat> = config.formats().to_vec();
        let entry = NamedFactory::<ImageDecodeRole>::new(
            id,
            revision,
            Arc::new(config) as Arc<dyn DynDecoderConfig>,
        );
        self.with_role::<ImageDecodeRole>(keys, entry)
    }

    /// Register a named thread-local encoder config both in [`ImageEncodeRole`] for
    /// multi-provider routing and in the legacy first-wins encoder list.
    ///
    /// The registered [`ImageEncodeFactory`] reports `supports_send_job() == false` and
    /// refuses [`ImageEncodeFactory::send_job`] with [`CodecSetError::EncoderJobNotSend`].
    pub fn with_named_encoder<C>(
        self,
        id: RegistrationId,
        revision: &'static str,
        config: C,
    ) -> Result<Self, RegistrationError>
    where
        C: EncoderConfig + 'static,
        <C::Job as EncodeJob>::Enc: Encoder + Send,
        <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
    {
        let fmt = config.format();
        let factory: Arc<dyn ImageEncodeFactory> =
            Arc::new(LocalImageEncodeFactory(Arc::new(config)));
        let entry = NamedFactory::<ImageEncodeRole>::new(id, revision, factory);
        self.with_role::<ImageEncodeRole>([fmt], entry)
    }

    /// Register a named `Send`-job encoder config both in [`ImageEncodeRole`] for
    /// multi-provider routing and in the legacy first-wins encoder list.
    ///
    /// The registered [`ImageEncodeFactory`] preserves `Send` job construction via
    /// [`ImageEncodeFactory::send_job`] for this specific provider even when it is
    /// not the first encoder registered for its format.
    pub fn with_named_send_encoder<C>(
        self,
        id: RegistrationId,
        revision: &'static str,
        config: C,
    ) -> Result<Self, RegistrationError>
    where
        C: EncoderConfig + 'static,
        C::Job: Send,
        <C::Job as EncodeJob>::Enc: Encoder + Send,
        <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
    {
        let fmt = config.format();
        let factory: Arc<dyn ImageEncodeFactory> =
            Arc::new(SendImageEncodeFactory(Arc::new(config)));
        let entry = NamedFactory::<ImageEncodeRole>::new(id, revision, factory);
        self.with_role::<ImageEncodeRole>([fmt], entry)
    }

    /// Legacy enumeration of all registered decoder configs (anonymous and named)
    /// serving `format`, in registration order.
    ///
    /// For ranked multi-provider routing with stable provider IDs and revisions, use
    /// [`factories::<ImageDecodeRole>`](Self::factories).
    pub fn decoders_for(
        &self,
        format: ImageFormat,
    ) -> impl Iterator<Item = &dyn DynDecoderConfig> + '_ {
        self.registrations
            .decoders
            .iter()
            .filter(move |d| d.admitted_formats.contains(&format))
            .map(|d| &*d.config)
    }

    /// Legacy enumeration of all registered encoder configs (anonymous and named)
    /// serving `format`, in registration order.
    ///
    /// Returns bare [`DynEncoderConfig`] templates without `Send`-job construction or
    /// provider IDs. For ranked multi-provider routing preserving per-provider `Send`
    /// capability, use [`factories::<ImageEncodeRole>`](Self::factories).
    pub fn encoders_for(
        &self,
        format: ImageFormat,
    ) -> impl Iterator<Item = &dyn DynEncoderConfig> + '_ {
        self.registrations
            .encoders
            .iter()
            .filter(move |e| e.config().format() == format)
            .map(|e| e.config())
    }

    /// Returns `true` when any operation-level defaults (`limits`, `pulse`,
    /// `decode_policy`, or `encode_policy`) are stamped on this set.
    ///
    /// Engines that require a registration-only [`CodecSet`] (managing per-run
    /// limits, pulses, and policies externally) check this method to reject
    /// pre-stamped sets.
    #[inline]
    #[must_use]
    pub fn has_operation_defaults(&self) -> bool {
        self.limits.is_some()
            || self.pulse.is_some()
            || self.decode_policy.is_some()
            || self.encode_policy.is_some()
    }

    // --- Operation defaults -------------------------------------------------

    /// Resource limits stamped onto every job this set creates.
    pub fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Progress and cancellation pulse stamped onto every job this set creates.
    pub fn with_pulse(mut self, pulse: how_far::SharedPulse) -> Self {
        self.pulse = Some(pulse);
        self
    }

    /// Checked decode policy stamped onto every decode job this set creates.
    ///
    /// Enforced via [`DynDecodeJob::try_set_policy`](crate::decode::DynDecodeJob::try_set_policy)
    /// when a job is created; a decoder that cannot honor the policy refuses
    /// with an error instead of silently ignoring it.
    pub fn with_decode_policy(mut self, policy: DecodePolicy) -> Self {
        self.decode_policy = Some(policy);
        self
    }

    /// Checked encode policy stamped onto every encode job this set creates.
    ///
    /// Enforced via [`DynEncodeJob::try_set_policy`](crate::encode::DynEncodeJob::try_set_policy)
    /// when a job is created; an encoder that cannot honor the policy refuses
    /// with an error instead of silently ignoring it.
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
        self.decoder_entry(format)
    }

    /// The registered encoder config for `format`, if any.
    pub fn encoder_for(&self, format: ImageFormat) -> Option<&dyn DynEncoderConfig> {
        self.encoder_entry(format).map(|e| e.config())
    }

    /// Formats with a registered decoder, in registration order.
    ///
    /// May repeat a format if several decoders claim it.
    pub fn decodable_formats(&self) -> impl Iterator<Item = ImageFormat> + '_ {
        self.registrations
            .decoders
            .iter()
            .flat_map(|d| d.admitted_formats.iter().copied())
    }

    /// Formats with a registered encoder, in registration order.
    pub fn encodable_formats(&self) -> impl Iterator<Item = ImageFormat> + '_ {
        self.registrations
            .encoders
            .iter()
            .map(|e| e.config().format())
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
        for decoder in &self.registrations.decoders {
            for &format in &decoder.admitted_formats {
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

    pub(crate) fn check_input_limit(&self, input_len: usize) -> Result<(), CodecSetError> {
        if let Some(limits) = self.limits {
            limits
                .check_input_size(input_len as u64)
                .map_err(|e| CodecSetError::Codec(Box::new(e)))?;
        }
        Ok(())
    }

    /// Probe image metadata (header parse only) after detecting the format.
    pub fn probe<'a>(&'a self, data: &'a [u8]) -> Result<ImageInfo, CodecSetError> {
        self.check_input_limit(data.len())?;
        let format = self.detect(data).ok_or(CodecSetError::UnrecognizedFormat)?;
        Ok(self.decode_job(format)?.probe(data)?)
    }

    /// Detect the format and decode to owned pixels in the decoder's native
    /// pixel format.
    pub fn decode<'a>(&'a self, data: &'a [u8]) -> Result<DecodeOutput, CodecSetError> {
        self.decode_preferring_formats(data, &[])
    }

    /// Detect the format and decode, requesting output in one of the
    /// `preferred` pixel formats (ranked; the decoder picks the first it can
    /// produce without lossy conversion, else its native format).
    pub fn decode_preferring_formats<'a>(
        &'a self,
        data: &'a [u8],
        preferred: &[PixelFormat],
    ) -> Result<DecodeOutput, CodecSetError> {
        self.check_input_limit(data.len())?;
        let format = self.detect(data).ok_or(CodecSetError::UnrecognizedFormat)?;
        self.decode_with_format(format, data, preferred)
    }

    /// Decode `data` as `format`, skipping magic-byte detection.
    pub fn decode_with_format<'a>(
        &'a self,
        format: ImageFormat,
        data: &'a [u8],
        preferred: &[PixelFormat],
    ) -> Result<DecodeOutput, CodecSetError> {
        self.check_input_limit(data.len())?;
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
        preferred: &[PixelFormat],
    ) -> Result<OutputInfo, CodecSetError> {
        self.check_input_limit(data.len())?;
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
        preferred: &[PixelFormat],
    ) -> Result<Box<dyn DynAnimationFrameDecoder>, CodecSetError> {
        self.check_input_limit(data.len())?;
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
        preferred: &[PixelFormat],
    ) -> Result<Box<dyn DynStreamingDecoder + 'a>, CodecSetError> {
        self.check_input_limit(data.len())?;
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
        preferred: &[PixelFormat],
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
        preferred: &[PixelFormat],
    ) -> Result<OutputInfo, CodecSetError> {
        // A short read (not `len()`) bounds the probe, so a stream-backed
        // source is not consumed to its end before decoding starts.
        let mut probe = alloc::vec![0u8; ImageFormat::RECOMMENDED_PROBE_BYTES + 1024];
        let n = source.read_at(0, &mut probe)?;
        probe.truncate(n);
        let format = self
            .detect(&probe)
            .ok_or(CodecSetError::UnrecognizedFormat)?;
        Ok(self
            .decode_job(format)?
            .push_decoder_from_source(source, sink, preferred)?)
    }

    /// A decode job for `format` with this set's defaults stamped on and
    /// checked (`limits`, `pulse`, and `decode_policy` via `try_set_policy`).
    ///
    /// The escape hatch behind every decode convenience: set decode hints
    /// (crop, orientation, gain map, start frame) on the job, then call one of
    /// its `into_*` executors or [`probe`](DynDecodeJob::probe).
    pub fn decode_job<'a>(
        &'a self,
        format: ImageFormat,
    ) -> Result<Box<dyn DynDecodeJob<'a> + 'a>, CodecSetError> {
        self.checked_decode_job(format, None)
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
        }
        if let Some(policy) = policy.or(self.decode_policy) {
            job.try_set_policy(policy)?;
        }
        Ok(job)
    }

    /// The inherited operation pulse, or `NoPulse` when none was configured.
    /// Pass this same control through policy, I/O, and native work.
    pub fn pulse(&self) -> &dyn crate::Pulse {
        self.pulse.as_ref().map_or(&crate::NoPulse, |pulse| pulse)
    }

    pub(crate) fn operation_pulse(&self) -> &dyn crate::Pulse {
        self.pulse()
    }

    pub(crate) fn check_operation_stop(&self) -> Result<(), enough::StopReason> {
        self.pulse().check()
    }

    pub(crate) fn operation_access_pattern(&self) -> crate::source::AccessPattern {
        self.registrations
            .decoders
            .iter()
            .map(|d| d.config.capabilities().access_pattern())
            .fold(crate::source::AccessPattern::Sequential, |a, b| {
                a.most_demanding(b)
            })
    }

    pub(crate) fn detect_incremental(
        &self,
        input: &crate::decode::Input<'_>,
    ) -> Result<Option<ImageFormat>, CodecSetError> {
        if input.offset != 0 {
            return Ok(None);
        }
        let detected = self.detect(input.bytes);
        let complete =
            crate::decode::InputRequest::at_least(0, ImageFormat::RECOMMENDED_PROBE_BYTES)
                .is_satisfied_by(input);
        // TIFF magic is shared with TIFF-based raws. Wait for the same complete
        // prefix in both the legacy and controlled request entry points.
        let conclusive = match detected {
            Some(ImageFormat::Tiff) => complete,
            Some(_) => true,
            None => false,
        };
        if conclusive {
            Ok(detected)
        } else if complete {
            Err(CodecSetError::UnrecognizedFormat)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn operation_encode_policy(&self) -> Option<EncodePolicy> {
        self.encode_policy
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
            .config();
        let mut job = match fidelity {
            Some(target) => config
                .try_fidelity_job(target)
                .map_err(|e| CodecSetError::Codec(Box::new(e)))?,
            None => config.dyn_job(),
        };
        self.stamp_checked_encode_job(&mut *job, policy)?;
        Ok(job)
    }

    pub(crate) fn checked_send_encode_job(
        &self,
        format: ImageFormat,
        fidelity: Option<Fidelity>,
        policy: Option<EncodePolicy>,
    ) -> Result<Box<dyn DynEncodeJob + Send>, CodecSetError> {
        let entry = self
            .encoder_entry(format)
            .ok_or(CodecSetError::NoEncoder(format))?;
        let mut job = entry.send_job(fidelity)?;
        self.stamp_checked_encode_job(&mut *job, policy)?;
        Ok(job)
    }
    fn stamp_checked_encode_job(
        &self,
        job: &mut dyn DynEncodeJob,
        policy: Option<EncodePolicy>,
    ) -> Result<(), CodecSetError> {
        if let Some(limits) = self.limits {
            job.set_limits(limits);
        }
        if let Some(pulse) = &self.pulse {
            job.set_pulse(pulse.clone());
        }
        if let Some(policy) = policy.or(self.encode_policy) {
            job.try_set_policy(policy)?;
        }
        Ok(())
    }

    // --- Encode -------------------------------------------------------------

    /// Encode `pixels` to `format` using the registered encoder template.
    pub fn encode(
        &self,
        format: ImageFormat,
        pixels: PixelView<'_>,
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
        pixels: PixelView<'_>,
    ) -> Result<EncodeOutput, CodecSetError> {
        Ok(self.encode_job_with(format, fidelity)?.encode(pixels)?)
    }

    /// An encode job for `format` with this set's defaults stamped on and
    /// checked (`limits`, `pulse`, and `encode_policy` via `try_set_policy`).
    ///
    /// The escape hatch behind the encode conveniences: set metadata, canvas
    /// size, or loop count on the job, then call
    /// [`into_encoder`](DynEncodeJob::into_encoder) (one-shot or
    /// `push_rows`/`finish` streaming) or
    /// [`into_animation_frame_encoder`](DynEncodeJob::into_animation_frame_encoder).
    pub fn encode_job(&self, format: ImageFormat) -> Result<Box<dyn DynEncodeJob>, CodecSetError> {
        self.checked_encode_job(format, None, None)
    }

    /// Like [`encode_job`](Self::encode_job), with a checked per-call
    /// [`Fidelity`] applied to a clone of the template first.
    ///
    /// Unsupported or substituted fidelity targets refuse instead of falling
    /// back silently.
    pub fn encode_job_with(
        &self,
        format: ImageFormat,
        fidelity: Fidelity,
    ) -> Result<Box<dyn DynEncodeJob>, CodecSetError> {
        self.checked_encode_job(format, Some(fidelity), None)
    }

    /// Decode `input` and re-encode it to `target` at `fidelity` — the one-call
    /// recompress / reformat, carrying the source's metadata under an explicit
    /// checked retention policy and applying a checked color-emission policy.
    ///
    /// # Scope and failure modes
    ///
    /// - **Single image** — rejects known animation and multi-image sources
    ///   rather than silently selecting their first image. For animation use
    ///   [`animation_decoder`](Self::animation_decoder) + [`encode_job`](Self::encode_job).
    /// - **No pixel processing** — recompresses at the decoded dimensions and
    ///   pixel format; no resize, crop, or CMS conversion.
    /// - **No implicit pixel-format downgrade** — if the target cannot accept
    ///   the decoded pixel format, use an explicit pixel conversion or a
    ///   codec-specific encode path. The registered decoder is not re-run at a
    ///   lower precision behind the caller's back.
    /// - **Checked fidelity, metadata, and color policy** — routes through
    ///   `try_fidelity_job`, `try_set_metadata_policy`, and `try_set_policy`;
    ///   an encoder that cannot honor the requested fidelity, retention policy,
    ///   or color policy refuses with an error.
    pub fn transcode(
        &self,
        input: &[u8],
        target: ImageFormat,
        fidelity: Fidelity,
        metadata: MetadataPolicy,
        color: ColorEmitPolicy,
    ) -> Result<EncodeOutput, CodecSetError> {
        self.check_input_limit(input.len())?;
        let target_inputs = self
            .encoder_for(target)
            .ok_or(CodecSetError::NoEncoder(target))?
            .supported_pixel_formats();
        // This is a still-image convenience. Reject known source content that
        // cannot travel through a single PixelView before decoding samples.
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
        if !target_inputs.contains(decoded.pixels().format()) {
            return Err(CodecSetError::TranscodeRequiresExplicitHandling(
                TranscodeBarrier::PixelFormat,
            ));
        }
        let source_metadata = Metadata::from(decoded.info());
        let policy = self
            .encode_policy
            .unwrap_or_else(EncodePolicy::none)
            .with_color(color);
        let mut job = self.checked_encode_job(target, Some(fidelity), Some(policy))?;
        job.try_set_metadata_policy(source_metadata, metadata)?;
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
        pixels: PixelView<'_>,
        compute: &ComputeEnvironment,
    ) -> Result<ResourceEstimate, CodecSetError> {
        let image =
            ImageCharacteristics::new(pixels.width(), pixels.height(), pixels.format().clone());
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
    /// [`supported_pixel_formats`](DynDecoderConfig::supported_pixel_formats), which
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
        self.check_input_limit(data.len())?;
        // Key off the *detected* (registration) format, which is what
        // `decoder_for` / `estimate_decode` index on — not `info.format`, the
        // format the decoder *reports*, which can differ (e.g. a decoder
        // registered under a custom format).
        let format = self.detect(data).ok_or(CodecSetError::UnrecognizedFormat)?;
        let info = self.decode_job(format)?.probe(data)?;
        let descriptor = self
            .decoder_for(format)
            .and_then(|d| d.supported_pixel_formats().first().cloned())
            .unwrap_or(if info.has_alpha {
                PixelFormat::RGBA8_SRGB
            } else {
                PixelFormat::RGB8_SRGB
            });
        let image = ImageCharacteristics::new(info.width, info.height, descriptor);
        self.estimate_decode(format, &image, compute)
    }

    // --- Internals ----------------------------------------------------------

    fn decoder_entry(&self, format: ImageFormat) -> Option<&dyn DynDecoderConfig> {
        self.registrations
            .decoders
            .iter()
            .find(|d| d.admitted_formats.contains(&format))
            .map(|b| &*b.config)
    }

    fn encoder_entry(&self, format: ImageFormat) -> Option<&dyn ImageEncodeFactory> {
        self.registrations
            .encoders
            .iter()
            .find(|e| e.config().format() == format)
            .map(|b| &**b)
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
            registrations: self.registrations.clone(),
            limits: self.limits,
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
    preferred: Vec<PixelFormat>,
    inner: Option<Box<crate::traits::DynIncrementalDecoder>>,
    probe: [crate::traits::InputRequest; 1],
}

impl CodecSetIncrementalDecoder {
    /// Create a new incremental decoder tied to `set`.
    pub fn new(set: CodecSet, preferred: &[PixelFormat]) -> Self {
        Self {
            set,
            preferred: preferred.to_vec(),
            inner: None,
            probe: [crate::traits::InputRequest::at_least(
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
        pulse: Option<&dyn crate::Pulse>,
    ) -> Result<crate::traits::Progress, crate::traits::StepError<CodecSetError>> {
        use crate::traits::{Progress, StepError};
        self.set.check_operation_stop()?;
        if self.inner.is_none() {
            let Some(format) = self
                .set
                .detect_incremental(&input)
                .map_err(StepError::Codec)?
            else {
                return Ok(Progress::need(0));
            };
            let job = self.set.decode_job(format).map_err(StepError::Codec)?;
            let inner = job
                .into_incremental_decoder(&self.preferred)
                .map_err(|e| StepError::Codec(CodecSetError::Codec(e)))?;
            self.inner = Some(inner);
        }
        let inner = self.inner.as_mut().expect("set above");
        inner
            .step(input, sink, pulse)
            .map_err(|e| e.map_codec(|c| CodecSetError::Codec(c.into_inner())))
    }

    fn requested(&self) -> &[crate::traits::InputRequest] {
        match &self.inner {
            Some(inner) => inner.requested(),
            None => &self.probe,
        }
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.inner.as_ref().and_then(|d| d.info())
    }

    fn take_details(&mut self) -> Option<crate::decode::DecodeDetails> {
        self.inner.as_mut().and_then(|d| d.take_details())
    }

    fn output_pixel_format(&self) -> Option<PixelFormat> {
        self.inner.as_ref().and_then(|d| d.output_pixel_format())
    }

    fn access_pattern(&self) -> crate::source::AccessPattern {
        match &self.inner {
            Some(inner) => inner.access_pattern(),
            None => self.set.operation_access_pattern(),
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
    fn decode_with_format_without_decoder_is_no_decoder() {
        let set = CodecSet::new();
        match set.decode_with_format(ImageFormat::Jpeg, &[0xFF, 0xD8], &[]) {
            Err(CodecSetError::NoDecoder(ImageFormat::Jpeg)) => {}
            other => panic!("expected NoDecoder(Jpeg), got {other:?}"),
        }
    }

    #[test]
    fn encode_without_encoder_is_no_encoder() {
        let set = CodecSet::new();
        let bytes = [0u8; 3];
        let pixels =
            PixelView::new(&bytes, 1, 1, 3, &zenpixels::formats::RGB8_SRGB).expect("pixel slice");
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
