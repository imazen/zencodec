//! Joint admission for one retained configured job and its executable routes.
use super::{
    DecisionContext, DecisionRecord, EncodeFacts, EncodeOffer, MetadataTransfer, OperationPolicy,
    OutputSchedule, OutputScheduleOffer, PolicyFailure, PolicySet, Question, Selection, policy,
};
use crate::{
    At,
    encode::{
        EncodeDeclaration, EncodePlan, InputRequirement, OutputPhase, OutputTiming, SourcePasses,
    },
};

/// Actual input mechanism retained by the driver, not a source capability flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputAcquisition {
    /// No executable acquisition mechanism has been established.
    #[default]
    Unknown,
    /// An actual immutable complete raster borrow is held through execution.
    BorrowedRaster,
    /// Rows are read directly from the external source for each native pass.
    DirectRows,
    /// One external traversal fills private raw RAM, then native reads use it.
    MemorySnapshot,
    /// One external traversal fills a raw disk store; aliases may still exist.
    DiskReplay,
}

/// Evidence for identical samples on repeated reads of the retained route.
/// These are adapter claims, not transferable proofs or source-owner handles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReplayEvidence {
    /// Repeated sample identity is unqualified.
    #[default]
    Unknown,
    /// The driver holds the actual immutable raster borrow for every read.
    ImmutableBorrow,
    /// Native reads use the same private immutable snapshot after acquisition.
    /// This does not certify an atomic historical snapshot of a mutable source.
    PrivateSnapshot,
    /// External identity/version is checked; this is weaker than exact samples.
    VerifiedIdentity,
    /// Replayed samples are hash-checked; collisions remain possible.
    VerifiedHash,
}
impl ReplayEvidence {
    pub(super) const fn exact(self) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::ImmutableBorrow | Self::PrivateSnapshot => Some(true),
            Self::VerifiedIdentity | Self::VerifiedHash => Some(false),
        }
    }
}

/// Evidence of the one retained configured job, shared by every acquisition
/// and output alternative. Route selection cannot change these job facts.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct EncodeEvidence {
    /// Checked effective encoder configuration, including its fidelity target.
    pub encoder: EncodeOffer,
    /// Qualified descriptive treatment of this job; None means unknown.
    pub metadata: Option<MetadataTransfer>,
    /// Whether this job retains an opaque source profile. None is unknown,
    /// not evidence of absence; strict exclusion requires Some(false).
    pub retains_opaque_profile: Option<bool>,
}
impl EncodeEvidence {
    /// No qualified format, fidelity, metadata, profile or resolution facts.
    ///
    /// Use for a retained generic job whose trait does not expose these facts.
    /// Unknown evidence cannot satisfy a requirement for a known property;
    /// callers must not infer configuration from successful setters or timing.
    pub const fn unknown() -> Self {
        Self::new(EncodeOffer {
            format: crate::ImageFormat::Unknown,
            fidelity: None,
            preference_index: 0,
            resolution: None,
        })
    }

    /// Retain checked configuration with unknown metadata/profile evidence.
    pub const fn new(encoder: EncodeOffer) -> Self {
        Self {
            encoder,
            metadata: None,
            retains_opaque_profile: None,
        }
    }
}

/// Bounded summary of the immutable declaration and configured job plan.
/// ICC/color payloads and input owners remain borrowed only in the question.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct EncodeRouteFacts {
    /// Actual declared sample geometry and layout.
    pub pixels: EncodeFacts,
    /// Effective configuration and metadata facts shared by every route.
    pub evidence: EncodeEvidence,
    /// Native acquisition contract; staged external passes are counted separately.
    pub input: InputRequirement,
    /// Byte availability within an emitting pass, independent of retention.
    pub timing: OutputTiming,
    /// Which native pass can emit output; never a retained-memory bound.
    pub output_phase: OutputPhase,
}
impl EncodeRouteFacts {
    /// Copy only bounded facts; do not clone materialized color carriers.
    pub const fn new(
        declaration: &EncodeDeclaration<'_>,
        plan: &EncodePlan,
        evidence: EncodeEvidence,
    ) -> Self {
        Self {
            pixels: EncodeFacts {
                width: declaration.width,
                height: declaration.height,
                descriptor: declaration.descriptor,
            },
            evidence,
            input: plan.input,
            timing: plan.timing,
            output_phase: plan.output_phase,
        }
    }
}

/// One complete input/replay/output alternative for the exact retained job.
/// The driver keeps owners and reservations in parallel slots, indexed by the
/// resolver's returned original index. These copyable facts authorize no I/O.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct EncodeRouteOffer {
    /// Actual retained input acquisition mechanism.
    pub acquisition: InputAcquisition,
    /// Evidence from the retained input, never inferred from a public source flag.
    pub replay: ReplayEvidence,
    /// Executable final-destination schedule and scoped wait-retention evidence.
    pub output: OutputScheduleOffer,
    /// Additional codec/conversion workspace including preparation/verification.
    /// Excludes managed raw/output storage and source storage; None is unknown.
    pub workspace_upper_bound: Option<u64>,
    /// Qualified sample-range preservation by this complete route. Lossy mode
    /// or an exact replay claim alone does not establish this evidence. Required
    /// preservation also needs confirmed lossless fidelity on the shared job.
    pub preserves_extended_range: Option<bool>,
}
impl EncodeRouteOffer {
    /// Describe the executable mechanisms with all other evidence unknown.
    /// Unknown acquisition refuses even with no configured requirements.
    pub const fn new(acquisition: InputAcquisition, output: OutputScheduleOffer) -> Self {
        Self {
            acquisition,
            replay: ReplayEvidence::Unknown,
            output,
            workspace_upper_bound: None,
            preserves_extended_range: None,
        }
    }

    /// External traversals required by this route. Staging reads the source
    /// once even if the native replay maximum is unknown. This is no claim of
    /// termination, bounded internal computation, or an atomic source snapshot.
    pub const fn external_source_passes(&self, plan: &EncodePlan) -> Option<SourcePasses> {
        match self.acquisition {
            InputAcquisition::Unknown => None,
            InputAcquisition::MemorySnapshot | InputAcquisition::DiskReplay => {
                Some(SourcePasses::ONCE)
            }
            InputAcquisition::BorrowedRaster | InputAcquisition::DirectRows => {
                Some(match plan.input {
                    InputRequirement::Replay(passes) => passes,
                    InputRequirement::Rows | InputRequirement::WholeRaster => SourcePasses::ONCE,
                })
            }
        }
    }

    pub(super) fn exact_replay(&self, plan: &EncodePlan) -> Option<bool> {
        // Only the retained native plan can establish that no repeat is needed.
        // One external staging pass says nothing about native replay count.
        let once = match plan.input {
            InputRequirement::Rows | InputRequirement::WholeRaster => true,
            InputRequirement::Replay(passes) => passes.max() == Some(1),
        };
        if once {
            Some(true)
        } else {
            self.replay.exact()
        }
    }

    pub(super) const fn no_raw_memory_staging(&self) -> Option<bool> {
        match self.acquisition {
            InputAcquisition::Unknown => None,
            InputAcquisition::MemorySnapshot => Some(false),
            _ => Some(true),
        }
    }

    pub(super) fn compatible(&self, plan: &EncodePlan) -> bool {
        self.acquisition != InputAcquisition::Unknown
            && !(matches!(
                self.acquisition,
                InputAcquisition::DirectRows | InputAcquisition::DiskReplay
            ) && plan.input == InputRequirement::WholeRaster)
            && !(self.output.schedule == OutputSchedule::Live && !plan.timing.streams())
            && match self.replay {
                ReplayEvidence::ImmutableBorrow => {
                    self.acquisition == InputAcquisition::BorrowedRaster
                }
                ReplayEvidence::PrivateSnapshot => {
                    self.acquisition == InputAcquisition::MemorySnapshot
                }
                _ => true,
            }
    }
}

/// Select one complete route before source acquisition, native startup or final
/// destination effects. Both bound and legacy requirements apply unchanged.
///
/// The caller retains the same configured job, declaration, plan, input owners,
/// destination and admitted storage. Check workspace before covered preparation;
/// this later decision cannot retroactively admit that work. Revalidate source
/// and destination after the callback, then compare `begin_declared`'s complete
/// plan with this plan before any source read/publication. Never reconstruct the
/// job or retry another route after a failure. Preserve the record on failure.
///
/// Invalid shared plans refuse the whole question with the original typed
/// `crate::encode::EncodePlanError`, before callback selection. Individual
/// incompatible routes are filtered with named requirement diagnostics.
/// Only compatible, mandatory-admitted original indices reach the callback.
/// No owner or ICC payload is cloned into offers or the returned bounded record.
pub fn resolve_encode_route<R: OperationPolicy + ?Sized>(
    declaration: &EncodeDeclaration<'_>,
    plan: &EncodePlan,
    evidence: EncodeEvidence,
    offers: &[EncodeRouteOffer],
    resolver: &mut R,
    preset: PolicySet,
    pulse: Option<&dyn crate::Pulse>,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    let pulse = pulse.unwrap_or(&crate::NoPulse);
    let facts = EncodeRouteFacts::new(declaration, plan, evidence);
    policy::choose(
        offers,
        DecisionContext::EncodeRoute(facts),
        |choices| Question::EncodeRoute {
            declaration,
            plan,
            facts,
            choices,
        },
        Selection::EncodeRoute,
        resolver,
        preset,
        pulse,
    )
}

#[cfg(test)]
mod tests;
