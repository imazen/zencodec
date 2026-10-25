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
use zenpixels::{PixelDescriptor, PixelSlice};

mod decoding;
mod policy;
pub use policy::{
    BoundaryOffer, Choice, ChoiceId, Choices, Decision, DecisionCode, DecisionRecord, Decline,
    EncodeFacts, EncodeOffer, OperationReport, PolicyFailure, PolicyResolver, PolicySet, Question,
    Selection, resolve_boundary,
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
#[derive(Debug)]
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
    /// The decoder could not produce the required descriptor without conversion.
    OutputDescriptor {
        /// Required output interpretation and layout.
        requested: PixelDescriptor,
        /// The decoder's actual output interpretation and layout.
        actual: PixelDescriptor,
    },
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
            Self::OutputDescriptor { requested, actual } => write!(
                f,
                "required output {requested:?}; decoder produced {actual:?}"
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
    Prefer(Vec<PixelDescriptor>),
    Require(PixelDescriptor),
}

/// Reusable requirements for decoding one image into owned pixels.
///
/// Set-level controls (limits, stop/pulse, policy) are inherited. An explicitly
/// supplied policy replaces the set's policy and is checked before the decoder
/// receives input. Resource limits keep their codec-defined enforcement scope;
/// this request does not turn an estimate into a whole-operation memory bound.
#[derive(Clone, Debug, Default)]
pub struct DecodeRequest {
    format: Option<ImageFormat>,
    policy: Option<DecodePolicy>,
    output: Output,
}

impl DecodeRequest {
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
    pub const fn with_policy(mut self, policy: DecodePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Require this output descriptor. An unsupported result is an error;
    /// conversion or lower precision is never selected implicitly.
    pub fn with_output(mut self, descriptor: PixelDescriptor) -> Self {
        self.output = Output::Require(descriptor);
        self
    }

    /// Prefer these descriptors in order, permitting the decoder's native
    /// fallback. Inspect the returned pixels for the selected descriptor.
    pub fn preferring_output(mut self, descriptors: impl Into<Vec<PixelDescriptor>>) -> Self {
        self.output = Output::Prefer(descriptors.into());
        self
    }

    /// Start controlled decoding through the same registered templates and
    /// checked requirements as this request's one-shot path. Format detection
    /// consumes no prefix bytes. A required descriptor is checked at final
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
                .operation_stop()
                .check()
                .map_err(OperationError::Stopped)?;
            let format = self
                .format
                .or_else(|| codecs.detect(input))
                .ok_or(OperationError::Codec(CodecSetError::UnrecognizedFormat))?;
            let preferred = match &self.output {
                Output::Native => &[][..],
                Output::Prefer(list) => list,
                Output::Require(descriptor) => core::slice::from_ref(descriptor),
            };
            let job = codecs
                .checked_decode_job(format, self.policy)
                .map_err(OperationError::Codec)?;
            let decoder = job
                .into_decoder(Cow::Borrowed(input), preferred)
                .map_err(|e| OperationError::Codec(e.into()))?;
            let output = decoder
                .decode()
                .map_err(|e| OperationError::Codec(e.into()))?;
            codecs
                .operation_stop()
                .check()
                .map_err(OperationError::Stopped)?;
            if let Output::Require(requested) = self.output {
                let actual = output.pixels().descriptor();
                if actual != requested {
                    return Err(OperationError::OutputDescriptor { requested, actual });
                }
            }
            Ok(output)
        };
        run()
            .map(|output| Outcome {
                output,
                report: OperationReport::default(),
            })
            .map_err(|e| At::wrap(e).at_named("checked image decode"))
    }
}

/// Reusable requirements for encoding pixels with a registered template.
///
/// A fidelity target is checked at configuration time. This confirms the
/// requested mode/quality selection, not a perceptual score or that arbitrary
/// inputs satisfy a backend's content-dependent lossless restrictions.
#[derive(Clone, Debug)]
pub struct EncodeRequest {
    format: ImageFormat,
    fidelity: FidelityRequest,
    policy: Option<EncodePolicy>,
    preset: PolicySet,
}

#[derive(Clone, Debug)]
enum FidelityRequest {
    Template,
    Required(Fidelity),
    Alternatives(Vec<Fidelity>),
}

impl EncodeRequest {
    /// Use the registered encoder template, inheriting the set's controls.
    pub const fn new(format: ImageFormat) -> Self {
        Self {
            format,
            fidelity: FidelityRequest::Template,
            policy: None,
            preset: PolicySet::new(),
        }
    }

    /// Require this fidelity target; unsupported, substituted and unreported
    /// targets fail before encoding pixels.
    pub fn with_fidelity(mut self, fidelity: Fidelity) -> Self {
        self.fidelity = FidelityRequest::Required(fidelity);
        self
    }

    /// Permit only these fidelity targets, in preference order. Invalid values
    /// refuse the whole request; unsupported values may be omitted from offers.
    /// There is no encode retry after the resolver selects a prepared job.
    pub fn preferring_fidelity(mut self, targets: impl Into<Vec<Fidelity>>) -> Self {
        self.fidelity = FidelityRequest::Alternatives(targets.into());
        self
    }

    /// Configure the built-in policy used by ordinary execution and delegation.
    pub const fn with_policy_set(mut self, preset: PolicySet) -> Self {
        self.preset = preset;
        self
    }

    /// Record a required policy, replacing the inherited set policy.
    pub const fn with_policy(mut self, policy: EncodePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Validate configuration and encode, preserving strided pixel input and
    /// native errors. The stored request and registered template are reusable.
    #[track_caller]
    pub fn encode(
        &self,
        codecs: &CodecSet,
        pixels: PixelSlice<'_>,
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
    pub fn encode_with<R: PolicyResolver + ?Sized>(
        &self,
        codecs: &CodecSet,
        pixels: PixelSlice<'_>,
        resolver: &mut R,
    ) -> Result<Outcome<EncodeOutput>, At<OperationError>> {
        let targets = match &self.fidelity {
            FidelityRequest::Template => None,
            FidelityRequest::Required(target) => Some(core::slice::from_ref(target)),
            FidelityRequest::Alternatives(targets) => Some(targets.as_slice()),
        };
        let count = targets.map_or(1, <[_]>::len);
        let failure = |error| At::wrap(error).at_named("prepare encode routes");
        codecs
            .operation_stop()
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
        let mut offers = Vec::new();
        let mut jobs = Vec::new();
        let mut rejections = Vec::new();
        offers
            .try_reserve_exact(count)
            .map_err(|_| failure(OperationError::Allocation))?;
        jobs.try_reserve_exact(count)
            .map_err(|_| failure(OperationError::Allocation))?;
        rejections
            .try_reserve_exact(count)
            .map_err(|_| failure(OperationError::Allocation))?;
        for index in 0..count {
            codecs
                .operation_stop()
                .check()
                .map_err(|reason| failure(OperationError::Stopped(reason)))?;
            let fidelity = targets.map(|targets| targets[index]);
            match codecs.checked_encode_job(self.format, fidelity, self.policy) {
                Ok(job) => {
                    offers.push(EncodeOffer {
                        format: self.format,
                        fidelity,
                        preference_index: index,
                    });
                    jobs.push(job);
                }
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
        let facts = EncodeFacts {
            width: pixels.width(),
            height: pixels.rows(),
            descriptor: pixels.descriptor(),
        };
        let (selected, record) = policy::choose_encode(
            facts,
            &offers,
            resolver,
            self.preset,
            codecs.operation_stop(),
        )
        .map_err(|error| {
            error
                .map_error(OperationError::Policy)
                .at_named("resolve encode route")
        })?;
        let mut report = OperationReport::default();
        report.record(record);
        let job = jobs.swap_remove(selected);
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
