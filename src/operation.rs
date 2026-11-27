//! Stored operation requirements, checked automatically when executed.
//!
//! These requests are reusable configuration. Building one performs no codec
//! work. Execution uses the registered template and checked job primitives;
//! required policies and fidelity targets cannot fall back to legacy setters.
//! Pixel conversion is an explicit pipeline operation, not an implicit retry.

use crate::{
    At, CodecSet, CodecSetError, DecodeOutput, DecodePolicy, EncodeOutput, EncodePolicy,
    ImageFormat, encode::Fidelity,
};
use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;
use zenpixels::{PixelFormat, PixelView};

mod binding;
mod collection;
mod conversion;
mod decoding;
mod encoding_route;
mod frame;
mod image;
mod policy;
mod scheduling;
mod transcode;
mod transition;
pub use binding::{BoundPolicy, OperationPolicy};
pub use collection::{CheckedCollection, CollectionOperationError, CollectionStage};
pub use conversion::{ImageConversionOffer, resolve_image_conversion};
pub use encoding_route::{
    EncodeEvidence, EncodeRouteFacts, EncodeRouteOffer, InputAcquisition, ReplayEvidence,
    resolve_encode_route,
};
pub use frame::{FrameSummary, PublicationOffer, resolve_frame};
pub use image::{ImageSummary, resolve_image};
pub use policy::{
    BoundaryOffer, Choice, ChoiceId, Choices, Decision, DecisionCode, DecisionContext,
    DecisionRecord, Decline, EncodeFacts, EncodeOffer, OperationReport, PolicyFailure,
    PolicyRequirement, PolicyResolver, PolicySet, Question, RequirementEvidence, Selection,
    UnmetRequirements, resolve_boundary,
};
pub use scheduling::{
    OutputSchedule, OutputScheduleFacts, OutputScheduleOffer, OutputSource, resolve_output_schedule,
};
pub use transcode::{MetadataTransfer, TranscodeOffer, resolve_transcode};
pub use transition::{
    DestinationFacts, FrameConversionOffer, FrameRoute, FrameRouteFacts, FrameRouteOffer,
    FrameRouteState, StagingFacts, StagingKind, resolve_frame_route,
};

/// Successful output and the decisions that selected its permitted contract.
/// Dereferences to the ordinary codec output; reports never replace errors for
/// a violated requirement.
#[derive(Debug)]
pub struct Outcome<T> {
    output: T,
    report: OperationReport,
}
impl<T> Outcome<T> {
    /// Assemble an adapter's completed output and bounded decision record.
    /// This container does not independently validate the adapter's postconditions.
    pub const fn from_parts(output: T, report: OperationReport) -> Self {
        Self { output, report }
    }
    /// Inspect the ordinary codec output.
    pub const fn output(&self) -> &T {
        &self.output
    }
    /// Take the ordinary codec output, discarding its decision report.
    pub fn into_output(self) -> T {
        self.output
    }
    /// Take both the ordinary output and its decision report.
    pub fn into_parts(self) -> (T, OperationReport) {
        (self.output, self.report)
    }
    /// Decisions taken before producing this output.
    pub const fn report(&self) -> &OperationReport {
        &self.report
    }
}
impl<T> core::ops::Deref for Outcome<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.output
    }
}

/// Why one permitted fidelity route could not be prepared. No encode was run.
#[derive(Debug)]
pub struct RouteRejection {
    /// Index in the original preference list.
    pub preference_index: usize,
    /// Requested fidelity mode or target.
    pub fidelity: Option<Fidelity>,
    /// The native configuration refusal.
    pub cause: CodecSetError,
}

/// A checked operation failure. Native causes remain in the source chain.
#[non_exhaustive]
pub enum OperationError {
    /// Codec registration, configuration or execution failed.
    Codec(CodecSetError),
    /// The stored preference list is empty or exceeds the bounded planner.
    AlternativeCount {
        /// Supplied number of alternatives.
        count: usize,
        /// Maximum alternatives accepted by this implementation.
        limit: usize,
    },
    /// A malformed target is an invalid request, not an excuse to choose another.
    InvalidFidelity {
        /// Index of the invalid target.
        preference_index: usize,
        /// Target validation failure.
        cause: crate::encode::FidelityTargetError,
    },
    /// No permitted encoder route passed configuration validation.
    NoEncodeRoute(Vec<RouteRejection>),
    /// Bounded planner storage could not be reserved.
    Allocation,
    /// Cancellation observed outside an application policy question.
    Stopped(enough::StopReason),
    /// Policy callback, decline, invalid choice or cancellation failure.
    Policy(PolicyFailure),
    /// The selected implementation failed; its decision report survives.
    Execution {
        /// Native codec failure.
        cause: Box<CodecSetError>,
        /// Decisions that preceded this failure.
        report: Box<OperationReport>,
    },
    /// The decoder could not produce the required pixel format without conversion.
    OutputPixelFormat {
        /// Required output interpretation and layout.
        requested: Box<PixelFormat>,
        /// The decoder's actual output interpretation and layout.
        actual: Box<PixelFormat>,
    },
}

impl fmt::Debug for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OperationError({self})")
    }
}

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "operation: {e}"),
            Self::AlternativeCount { count, limit } => {
                write!(f, "request has {count} alternatives; expected 1..={limit}")
            }
            Self::InvalidFidelity {
                preference_index,
                cause,
            } => write!(f, "fidelity alternative {preference_index}: {cause}"),
            Self::NoEncodeRoute(rejections) => write!(
                f,
                "none of {} permitted encoder routes could be prepared",
                rejections.len()
            ),
            Self::Allocation => f.write_str("operation planning allocation failed"),
            Self::Stopped(reason) => write!(f, "operation stopped: {reason:?}"),
            Self::Policy(e) => e.fmt(f),
            Self::Execution { .. } => {
                f.write_str("selected encoder failed; inspect the decision report and native cause")
            }
            Self::OutputPixelFormat { requested, actual } => write!(
                f,
                "required output {:?}; decoder produced {:?}",
                crate::helpers::redacted_spec(requested),
                crate::helpers::redacted_spec(actual)
            ),
        }
    }
}
impl core::error::Error for OperationError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            Self::InvalidFidelity { cause, .. } => Some(cause),
            Self::NoEncodeRoute(rejections) => rejections
                .first()
                .map(|r| &r.cause as &dyn core::error::Error),
            Self::Policy(e) => Some(e),
            Self::Execution { cause, .. } => Some(cause.as_ref()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
enum Output {
    #[default]
    Native,
    Prefer(Vec<PixelFormat>),
    Require(PixelFormat),
}

/// Reusable requirements and preferences for decoding one image into owned pixels.
///
/// Set-level controls (limits, stop/pulse, policy) are inherited. An explicitly
/// supplied policy replaces the set's policy and is checked before the decoder
/// receives input. Resource limits keep their codec-defined enforcement scope;
/// these options do not turn an estimate into a whole-operation memory bound.
#[derive(Clone, Debug, Default)]
pub struct DecodeOptions {
    format: Option<ImageFormat>,
    policy: Option<DecodePolicy>,
    output: Output,
}

impl DecodeOptions {
    /// Detect the registered input format and request its native output.
    pub const fn new() -> Self {
        Self {
            format: None,
            policy: None,
            output: Output::Native,
        }
    }

    /// Decode as this format instead of detecting it.
    pub const fn with_format(mut self, format: ImageFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Record a required policy, replacing the inherited set policy.
    pub const fn require_policy(mut self, policy: DecodePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Require this output pixel format. An unsupported result is an error;
    /// conversion or lower precision is never selected implicitly.
    ///
    /// The one-shot path first asks the retained decoder to prepare its output
    /// format, when supported, then always verifies the actual result. When
    /// early facts are unavailable, refusal can follow a full raster allocation.
    /// For controlled execution,
    /// [`Self::start_controlled`] checks the final interpretation before sink
    /// effects; this is not a bound on native decoding workspace.
    pub fn require_pixel_format(mut self, format: PixelFormat) -> Self {
        self.output = Output::Require(format);
        self
    }

    /// Prefer these pixel formats in order, permitting the decoder's native
    /// fallback. Inspect the returned pixels for the selected format.
    pub fn prefer_pixel_formats(mut self, formats: impl Into<Vec<PixelFormat>>) -> Self {
        self.output = Output::Prefer(formats.into());
        self
    }

    /// Start controlled decoding through the same registered templates and
    /// checked requirements as these options' one-shot path. Format detection
    /// consumes no prefix bytes. A required pixel format is checked at final
    /// interpretation, before policy adoption or sink effects.
    ///
    /// Serve the returned session with `zencodec_io::PolicyDecoder` and any
    /// existing transport driver. Unsupported controlled execution refuses;
    /// there is no automatic fallback to ordinary incremental decoding. A known
    /// format is checked immediately, before the driver consumes any input.
    #[track_caller]
    pub fn start_controlled(
        &self,
        codecs: &CodecSet,
        storage: crate::storage::StorageBudget,
    ) -> Result<Box<crate::decode::DynControlledDecoder>, At<OperationError>> {
        let session = decoding::CheckedDecoder::new(self.clone(), codecs.clone(), storage)
            .map_err(|e| At::wrap(e).at_named("start controlled decode"))?;
        Ok(crate::decode::erase_controlled(session))
    }

    /// Validate configuration and decode. No capability-query choreography is
    /// required. Failures retain a location trace and the native codec cause.
    #[track_caller]
    pub fn decode(
        &self,
        codecs: &CodecSet,
        input: &[u8],
    ) -> Result<Outcome<DecodeOutput>, At<OperationError>> {
        let run = || -> Result<DecodeOutput, OperationError> {
            codecs
                .operation_pulse()
                .check()
                .map_err(OperationError::Stopped)?;
            codecs
                .check_input_limit(input.len())
                .map_err(OperationError::Codec)?;
            let format = self
                .format
                .or_else(|| codecs.detect(input))
                .ok_or(OperationError::Codec(CodecSetError::UnrecognizedFormat))?;
            let preferred = match &self.output {
                Output::Native => &[][..],
                Output::Prefer(list) => list,
                Output::Require(format) => core::slice::from_ref(format),
            };
            let job = codecs
                .checked_decode_job(format, self.policy)
                .map_err(OperationError::Codec)?;
            let decoder = job
                .into_decoder(Cow::Borrowed(input), preferred)
                .map_err(|e| OperationError::Codec(e.into()))?;
            let required = match &self.output {
                Output::Require(format) => Some(format.clone()),
                _ => None,
            };
            decoding::decode_owned(decoder, required, codecs.operation_pulse())
        };
        run()
            .map(|output| Outcome {
                output,
                report: OperationReport::default(),
            })
            .map_err(|e| At::wrap(e).at_named("checked image decode"))
    }
}

/// A checked encoder job retained for a pipeline's joint decision.
///
/// Preparing it checks required fidelity and encode policy and stamps the
/// registry's limits/cancellation. It does not construct an encoder, read
/// pixels, validate output metadata, or establish a native memory bound.
/// Ordinary callers use `EncodeOptions::encode`; adapters can retain this
/// same object alongside a conversion plan until their resolver chooses it.
pub struct PreparedEncode {
    offer: EncodeOffer,
    job: Box<dyn crate::encode::DynEncodeJob>,
}
impl fmt::Debug for PreparedEncode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedEncode")
            .field("offer", &self.offer)
            .finish_non_exhaustive()
    }
}
impl PreparedEncode {
    /// Configuration confirmed during preparation.
    pub const fn offer(&self) -> EncodeOffer {
        self.offer
    }

    /// Transfer the exact prepared job to an adapter. Metadata/carrier
    /// validation and any further configuration are that adapter's obligation.
    /// Replacing its checked policy or fidelity invalidates the original offer.
    pub fn into_job(self) -> Box<dyn crate::encode::DynEncodeJob> {
        self.job
    }
}

/// A prepared encoder job with compile-time thread-transfer capability.
///
/// The same checked requirements and offer as `PreparedEncode`, retaining Send
/// until native encoder construction. Created by `EncodeOptions::prepare_send`.
pub struct PreparedSendEncode {
    offer: EncodeOffer,
    job: Box<dyn crate::encode::DynEncodeJob + Send>,
}
impl fmt::Debug for PreparedSendEncode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedSendEncode")
            .field("offer", &self.offer)
            .finish_non_exhaustive()
    }
}
impl PreparedSendEncode {
    /// Confirmed configuration, with the original preference index.
    pub const fn offer(&self) -> EncodeOffer {
        self.offer
    }
    /// Transfer the same job, retaining its Send guarantee.
    pub fn into_job(self) -> Box<dyn crate::encode::DynEncodeJob + Send> {
        self.job
    }
}

/// Reusable requirements and preferences for encoding pixels with a registered template.
///
/// A fidelity target is checked at configuration time. This confirms the
/// requested mode/quality selection, not a perceptual score or that arbitrary
/// inputs satisfy a backend's content-dependent lossless restrictions.
#[derive(Clone, Debug)]
pub struct EncodeOptions {
    format: ImageFormat,
    fidelity: FidelityRequest,
    policy: Option<EncodePolicy>,
    policy_defaults: EncodePolicy,
    resolution: Option<crate::Resolution>,
    orientation_mode: Option<crate::encode::EncodeOrientation>,
    metadata: Option<(crate::Metadata, crate::MetadataPolicy)>,
}

#[derive(Clone, Debug)]
enum FidelityRequest {
    Template,
    Required(Fidelity),
    Alternatives(Vec<Fidelity>),
}

impl EncodeOptions {
    /// Use the registered encoder template, inheriting the set's controls.
    pub const fn new(format: ImageFormat) -> Self {
        Self {
            format,
            fidelity: FidelityRequest::Template,
            policy: None,
            policy_defaults: EncodePolicy::none(),
            resolution: None,
            orientation_mode: None,
            metadata: None,
        }
    }

    /// Require retention of policy-filtered metadata instead of best-effort embedding.
    /// Preparation calls the checked setter on each retained candidate job before
    /// selection or pixel input. Unsupported candidates refuse; no capability
    /// query or callback can waive this requirement. Omission inherits the
    /// registered template's metadata configuration.
    pub fn require_metadata_policy(
        mut self,
        metadata: crate::Metadata,
        policy: crate::MetadataPolicy,
    ) -> Self {
        self.metadata = Some((metadata, policy));
        self
    }

    /// Require this fidelity target; unsupported, substituted and unreported
    /// targets fail before encoding pixels.
    pub fn require_fidelity(mut self, fidelity: Fidelity) -> Self {
        self.fidelity = FidelityRequest::Required(fidelity);
        self
    }

    /// Permit only these fidelity targets, in preference order. Invalid values
    /// refuse the whole request; unsupported values may be omitted from offers.
    /// There is no encode retry after the resolver selects a prepared job.
    pub fn prefer_fidelities(mut self, targets: impl Into<Vec<Fidelity>>) -> Self {
        self.fidelity = FidelityRequest::Alternatives(targets.into());
        self
    }

    /// Require exact physical density, or an equivalent unknown-unit aspect.
    /// No implicit rounding, unit loss, resize or orientation transform is allowed.
    /// Validation occurs during preparation, before the callback or pixel encoding.
    pub const fn require_resolution(mut self, resolution: crate::Resolution) -> Self {
        self.resolution = Some(resolution);
        self
    }

    /// Require stored or baked orientation through the configured job.
    /// Preparation refuses unsupported codecs before invoking the resolver or
    /// encoding pixels. Metadata still supplies the intrinsic orientation.
    /// This is not a memory bound or an implicit conversion between formats.
    pub const fn require_orientation_mode(
        mut self,
        mode: crate::encode::EncodeOrientation,
    ) -> Self {
        self.orientation_mode = Some(mode);
        self
    }

    /// Record a required policy, replacing the inherited set policy.
    pub const fn require_policy(mut self, policy: EncodePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Supply adapter defaults for unspecified policy fields.
    ///
    /// Explicit fields in the options' policy (or the inherited set policy)
    /// win, including `Some(false)`. Filled fields go through the same checked
    /// setter as requirements; an unsupported default refuses preparation.
    /// An all-`None` default leaves the registered template unchanged.
    pub const fn with_policy_defaults(mut self, defaults: EncodePolicy) -> Self {
        self.policy_defaults = defaults;
        self
    }

    fn effective_policy(&self, codecs: &CodecSet) -> Option<EncodePolicy> {
        let required = self.policy.or(codecs.operation_encode_policy());
        if self.policy_defaults == EncodePolicy::none() {
            return required;
        }
        let mut policy = required.unwrap_or_else(EncodePolicy::none);
        policy.color = policy.color.or(self.policy_defaults.color);
        policy.embed_icc = policy.embed_icc.or(self.policy_defaults.embed_icc);
        policy.embed_exif = policy.embed_exif.or(self.policy_defaults.embed_exif);
        policy.embed_xmp = policy.embed_xmp.or(self.policy_defaults.embed_xmp);
        policy.strict = policy.strict.or(self.policy_defaults.strict);
        Some(policy)
    }

    /// Prepare permitted configurations without constructing an encoder or
    /// touching pixels. Adapter API for coupling conversion, metadata and
    /// encode into one question. All malformed targets reject the request;
    /// unsupported alternatives are omitted. An empty result is an error.
    /// The returned jobs retain original preference indices and must be the
    /// objects executed after selection. This method never calls a resolver.
    #[track_caller]
    pub fn prepare(&self, codecs: &CodecSet) -> Result<Vec<PreparedEncode>, At<OperationError>> {
        self.prepare_with(codecs, |offer| {
            let mut job = codecs.checked_encode_job(
                self.format,
                offer.fidelity,
                self.effective_policy(codecs),
            )?;
            if let Some((metadata, policy)) = &self.metadata {
                job.try_set_metadata_policy(metadata.clone(), *policy)?;
            }
            if let Some(mode) = self.orientation_mode {
                job.try_set_orientation_mode(mode)?;
            }
            if let Some(resolution) = offer.resolution {
                job.try_set_resolution(resolution)?;
            }
            Ok(PreparedEncode { offer, job })
        })
    }

    /// Prepare transferable jobs for an adapter that holds them across awaits.
    /// The selected registry entry must use `CodecSet::with_send_encoder`.
    /// Refusal occurs during preparation, before the adapter consumes input.
    #[track_caller]
    pub fn prepare_send(
        &self,
        codecs: &CodecSet,
    ) -> Result<Vec<PreparedSendEncode>, At<OperationError>> {
        self.prepare_with(codecs, |offer| {
            let mut job = codecs.checked_send_encode_job(
                self.format,
                offer.fidelity,
                self.effective_policy(codecs),
            )?;
            if let Some((metadata, policy)) = &self.metadata {
                job.try_set_metadata_policy(metadata.clone(), *policy)?;
            }
            if let Some(mode) = self.orientation_mode {
                job.try_set_orientation_mode(mode)?;
            }
            if let Some(resolution) = offer.resolution {
                job.try_set_resolution(resolution)?;
            }
            Ok(PreparedSendEncode { offer, job })
        })
    }

    fn prepare_with<T>(
        &self,
        codecs: &CodecSet,
        mut create: impl FnMut(EncodeOffer) -> Result<T, CodecSetError>,
    ) -> Result<Vec<T>, At<OperationError>> {
        let failure = |error| At::wrap(error).at_named("prepare encode routes");
        if let Some(resolution) = self.resolution {
            resolution.validate().map_err(|error| {
                failure(OperationError::Codec(CodecSetError::Codec(Box::new(error))))
            })?;
        }
        let targets = match &self.fidelity {
            FidelityRequest::Template => None,
            FidelityRequest::Required(target) => Some(core::slice::from_ref(target)),
            FidelityRequest::Alternatives(targets) => Some(targets.as_slice()),
        };
        let count = targets.map_or(1, <[_]>::len);
        codecs
            .operation_pulse()
            .check()
            .map_err(|reason| failure(OperationError::Stopped(reason)))?;
        if count == 0 || count > policy::MAX_OFFERS {
            return Err(failure(OperationError::AlternativeCount {
                count,
                limit: policy::MAX_OFFERS,
            }));
        }
        if let Some(targets) = targets {
            for (index, target) in targets.iter().enumerate() {
                target.validate_target().map_err(|cause| {
                    failure(OperationError::InvalidFidelity {
                        preference_index: index,
                        cause,
                    })
                })?;
            }
        }
        let mut jobs = Vec::new();
        let mut rejections = Vec::new();
        jobs.try_reserve_exact(count)
            .map_err(|_| failure(OperationError::Allocation))?;
        rejections
            .try_reserve_exact(count)
            .map_err(|_| failure(OperationError::Allocation))?;
        for index in 0..count {
            codecs
                .operation_pulse()
                .check()
                .map_err(|reason| failure(OperationError::Stopped(reason)))?;
            let fidelity = targets.map(|targets| targets[index]);
            match create(EncodeOffer {
                format: self.format,
                fidelity,
                preference_index: index,
                resolution: self.resolution,
            }) {
                Ok(job) => jobs.push(job),
                Err(cause) => rejections.push(RouteRejection {
                    preference_index: index,
                    fidelity,
                    cause,
                }),
            }
        }
        if jobs.is_empty() {
            return Err(failure(OperationError::NoEncodeRoute(rejections)));
        }
        Ok(jobs)
    }

    /// Validate configuration and encode, preserving strided pixel input and
    /// native errors. The stored request and registered template are reusable.
    #[track_caller]
    pub fn encode(
        &self,
        codecs: &CodecSet,
        pixels: PixelView<'_>,
    ) -> Result<Outcome<EncodeOutput>, At<OperationError>> {
        let mut delegate = |_: Question<'_>| -> Result<Decision, At<core::convert::Infallible>> {
            Ok(Decision::UsePreset)
        };
        self.encode_with(codecs, pixels, &mut delegate)
    }

    /// Execute with one application resolver. Only prepared jobs are offered;
    /// callback failures retain their concrete `At<E>` in the error chain.
    /// Callback success cannot authorize a stale or fabricated choice.
    #[track_caller]
    pub fn encode_with<R: OperationPolicy + ?Sized>(
        &self,
        codecs: &CodecSet,
        pixels: PixelView<'_>,
        resolver: &mut R,
    ) -> Result<Outcome<EncodeOutput>, At<OperationError>> {
        let mut jobs = self.prepare(codecs)?;
        let mut offers = Vec::new();
        offers
            .try_reserve_exact(jobs.len())
            .map_err(|_| At::wrap(OperationError::Allocation).at())?;
        offers.extend(jobs.iter().map(PreparedEncode::offer));
        let facts = EncodeFacts {
            width: pixels.width(),
            height: pixels.height(),
            pixel_format: pixels.format().clone(),
        };
        let (selected, record) = policy::choose_encode(
            facts,
            &offers,
            resolver,
            PolicySet::new(),
            codecs.operation_pulse(),
        )
        .map_err(|error| {
            error
                .map_error(OperationError::Policy)
                .at_named("resolve encode route")
        })?;
        let mut report = OperationReport::default();
        report.record(record);
        let job = jobs.swap_remove(selected).into_job();
        // Release unselected jobs before starting the chosen codec. The chosen
        // configuration is exactly the prepared object described by its offer.
        drop(jobs);
        let output = job.encode(pixels).map_err(|cause| {
            At::wrap(OperationError::Execution {
                cause: Box::new(cause.into()),
                report: Box::new(report.clone()),
            })
            .at_named("execute selected encoder")
        })?;
        Ok(Outcome { output, report })
    }
}

#[cfg(test)]
mod descriptor_diagnostics {
    use super::*;
    use zenpixels::{AlphaMode, ColorSpace, RenderingIntent, SampleLayout};

    #[test]
    fn descriptor_refusal_preserves_live_value_and_redacts_opaque_bytes() {
        let marker: alloc::sync::Arc<[u8]> = alloc::vec![239u8; 231].into();
        let request = PixelFormat::new(
            SampleLayout::RGB8,
            ColorSpace::Icc {
                profile: marker.clone(),
                color_channels: 3,
                alpha: AlphaMode::None,
                intent: RenderingIntent::RelativeColorimetric,
            },
        );
        let failure = OperationError::OutputPixelFormat {
            requested: Box::new(request),
            actual: Box::new(PixelFormat::RGB8_SRGB),
        };
        for diagnostic in [alloc::format!("{failure}"), alloc::format!("{failure:?}")] {
            assert!(!diagnostic.contains("239, 239"));
            assert!(diagnostic.len() < 600);
        }
        let OperationError::OutputPixelFormat { requested, .. } = failure else {
            unreachable!()
        };
        let ColorSpace::Icc { profile, .. } = &requested.color else {
            unreachable!()
        };
        assert!(alloc::sync::Arc::ptr_eq(profile, &marker));
    }
}
