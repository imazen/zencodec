//! Mandatory admission shared by built-in and application resolvers.
use super::{BoundaryAction, Fidelity, PolicySet, Question};
use core::fmt;

/// A mandatory constraint on a prepared operation route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolicyRequirement {
    /// Descriptive metadata treatment.
    Metadata,
    /// Exclusion of retained opaque source profiles.
    OpaqueProfiles,
    /// Qualified additional codec/conversion workspace limit.
    Workspace,
    /// Qualified extended-range preservation.
    ExtendedRange,
    /// Required final destination schedule.
    OutputSchedule,
    /// Qualified retained memory bound during output waits.
    OutputWait,
}
const REQUIREMENTS: [PolicyRequirement; 6] = [
    PolicyRequirement::Metadata,
    PolicyRequirement::OpaqueProfiles,
    PolicyRequirement::Workspace,
    PolicyRequirement::ExtendedRange,
    PolicyRequirement::OutputSchedule,
    PolicyRequirement::OutputWait,
];

/// Why an alternative cannot establish a requirement. No payloads are retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequirementEvidence {
    /// This question/route does not provide the necessary evidence.
    Unknown,
    /// Available evidence does not satisfy the required value or bound.
    Contradicted,
}

/// Bounded union of failures across excluded alternatives. Different alternatives
/// may fail different requirements; an entry need not apply to every alternative.
/// Contains only enum bits, with no allocations or input payloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnmetRequirements {
    unknown: u8,
    contradicted: u8,
}
impl UnmetRequirements {
    /// No alternatives were excluded for mandatory requirements.
    pub const fn is_empty(self) -> bool {
        self.unknown == 0 && self.contradicted == 0
    }
    /// Distinct requirement/evidence pairs, in stable requirement order.
    pub fn iter(self) -> impl Iterator<Item = (PolicyRequirement, RequirementEvidence)> {
        REQUIREMENTS
            .into_iter()
            .enumerate()
            .flat_map(move |(index, requirement)| {
                let bit = 1 << index;
                [
                    (self.unknown & bit != 0)
                        .then_some((requirement, RequirementEvidence::Unknown)),
                    (self.contradicted & bit != 0)
                        .then_some((requirement, RequirementEvidence::Contradicted)),
                ]
                .into_iter()
                .flatten()
            })
    }
    pub(super) fn merge(&mut self, other: Self) {
        self.unknown |= other.unknown;
        self.contradicted |= other.contradicted;
    }
    fn check(&mut self, index: usize, required: bool, evidence: Option<bool>) {
        if !required {
            return;
        }
        match evidence {
            Some(true) => {}
            Some(false) => self.contradicted |= 1 << index,
            None => self.unknown |= 1 << index,
        }
    }
}
impl fmt::Display for UnmetRequirements {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unmet requirements")?;
        for (requirement, evidence) in self.iter() {
            write!(f, " {requirement:?}({evidence:?})")?;
        }
        Ok(())
    }
}

impl PolicySet {
    pub(super) const fn has_requirements(self) -> bool {
        self.metadata.is_some()
            || !self.allow_opaque_profiles
            || self.workspace_limit.is_some()
            || self.require_extended_range
            || self.output_schedule.is_some()
            || self.output_wait_limit.is_some()
    }

    /// Evaluate one original-index choice. Discovery is the only deferred case:
    /// it cannot itself publish affected output, and adoption asks again.
    pub(super) fn unmet(self, question: Question<'_>) -> UnmetRequirements {
        let mut metadata = None;
        let mut opaque = None;
        let mut workspace = None;
        let mut range = None;
        let mut schedule = None;
        match question {
            Question::Transcode { choices, .. } => {
                let offer = choices.first().expect("one admission candidate").value();
                metadata = Some(offer.metadata);
                opaque = offer.retains_opaque_profile.map(|retained| !retained);
                workspace = offer.workspace_upper_bound;
            }
            Question::ImageConversion { choices, .. } => {
                let offer = choices.first().expect("one admission candidate").value();
                workspace = offer.workspace_upper_bound;
                range = offer.preserves_extended_range.then_some(true);
            }
            Question::FrameRoute { choices, .. } => {
                let offer = choices.first().expect("one admission candidate").value();
                workspace = offer.workspace_upper_bound;
                range = offer
                    .conversion
                    .filter(|c| c.preserves_extended_range)
                    .and_then(|_| offer.fidelity.map(|f| f == Fidelity::Lossless));
                schedule = offer.output_schedule;
            }
            Question::OutputSchedule { choices, .. } => {
                schedule = Some(*choices.first().expect("one admission candidate").value());
            }
            Question::Boundary { choices, .. } => {
                if choices
                    .first()
                    .expect("one admission candidate")
                    .value()
                    .action
                    == BoundaryAction::ContinueReading
                {
                    return UnmetRequirements::default();
                }
            }
            Question::Encode { .. } | Question::Image { .. } | Question::Frame { .. } => {}
        }
        let mut unmet = UnmetRequirements::default();
        unmet.check(
            0,
            self.metadata.is_some(),
            self.metadata.zip(metadata).map(|(a, b)| a == b),
        );
        unmet.check(1, !self.allow_opaque_profiles, opaque);
        unmet.check(
            2,
            self.workspace_limit.is_some(),
            self.workspace_limit
                .zip(workspace)
                .map(|(limit, bound)| bound <= limit),
        );
        unmet.check(3, self.require_extended_range, range);
        unmet.check(
            4,
            self.output_schedule.is_some(),
            self.output_schedule
                .zip(schedule)
                .map(|(a, b)| a == b.schedule),
        );
        unmet.check(
            5,
            self.output_wait_limit.is_some(),
            self.output_wait_limit
                .zip(schedule.and_then(|s| s.wait_retained_upper_bound))
                .map(|(limit, bound)| bound <= limit),
        );
        unmet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        At, ImageFormat,
        decode::{DecodeBoundary, InterpretationFacts, InterpretationState},
        operation::{Decision, EncodeOffer, MetadataTransfer, TranscodeOffer, resolve_transcode},
    };
    use core::convert::Infallible;
    use zenpixels::PixelDescriptor;

    fn boundary() -> DecodeBoundary {
        DecodeBoundary::new(InterpretationFacts::new(
            3,
            2,
            Some(PixelDescriptor::RGB8_SRGB),
            InterpretationState::Final,
        ))
        .unwrap()
    }
    fn offer() -> TranscodeOffer {
        let mut offer = TranscodeOffer::new(
            EncodeOffer {
                format: ImageFormat::Png,
                fidelity: Some(Fidelity::Lossless),
                preference_index: 0,
                resolution: None,
            },
            PixelDescriptor::RGB8_SRGB,
            MetadataTransfer::DiscardDescriptive,
        );
        offer.retains_opaque_profile = Some(false);
        offer.workspace_upper_bound = Some(8);
        offer
    }
    fn requirements() -> PolicySet {
        PolicySet::new()
            .require_metadata(MetadataTransfer::DiscardDescriptive)
            .allow_opaque_profiles(false)
            .with_workspace_limit(8)
    }
    #[test]
    fn callback_only_sees_admitted_original_indices_including_bit_63() {
        let mut offers = [offer(); 64];
        for offer in &mut offers[..63] {
            offer.retains_opaque_profile = None;
        }
        let (index, record) = resolve_transcode(
            boundary(),
            &offers,
            &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Transcode { choices, .. } = q else {
                    panic!()
                };
                assert_eq!(choices.len(), 1);
                let mut iter = choices.iter();
                assert_eq!(iter.len(), 1);
                let choice = iter.next().unwrap();
                assert_eq!(iter.len(), 0);
                assert!(iter.next().is_none());
                assert!(core::ptr::eq(choice.value(), &offers[63]));
                Ok(Decision::choose(choice))
            },
            requirements(),
            &crate::how_far::NoPulse,
        )
        .unwrap();
        assert_eq!(index, 63);
        assert_eq!((record.offered, record.admitted), (64, 1));
        assert!(!record.used_preset);
        assert_eq!(
            record
                .unmet_requirements
                .iter()
                .collect::<alloc::vec::Vec<_>>(),
            [(
                PolicyRequirement::OpaqueProfiles,
                RequirementEvidence::Unknown
            )]
        );
    }
    #[test]
    fn all_excluded_refuses_before_callback_and_names_competing_failures() {
        let mut offers = [offer(); 2];
        offers[0].metadata = MetadataTransfer::PreserveReported;
        offers[0].workspace_upper_bound = None;
        offers[1].retains_opaque_profile = None;
        offers[1].workspace_upper_bound = Some(9);
        let error = resolve_transcode(
            boundary(),
            &offers,
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                panic!("requirements must precede callback")
            },
            requirements(),
            &crate::how_far::NoPulse,
        )
        .unwrap_err();
        let record = error.error().record();
        assert_eq!((record.offered, record.admitted), (2, 0));
        assert_eq!(
            record
                .unmet_requirements
                .iter()
                .collect::<alloc::vec::Vec<_>>(),
            [
                (
                    PolicyRequirement::Metadata,
                    RequirementEvidence::Contradicted
                ),
                (
                    PolicyRequirement::OpaqueProfiles,
                    RequirementEvidence::Unknown
                ),
                (PolicyRequirement::Workspace, RequirementEvidence::Unknown),
                (
                    PolicyRequirement::Workspace,
                    RequirementEvidence::Contradicted
                ),
            ]
        );
        let text = alloc::format!("{}", error.error());
        assert!(text.contains("Metadata(Contradicted)"));
        assert!(text.contains("Workspace(Unknown)"));
        assert!(!record.used_preset);
    }
    #[test]
    fn lossless_preference_does_not_constrain_direct_selection() {
        let mut offers = [offer(); 2];
        offers[0].encoder.fidelity = Some(Fidelity::codec_quality(80.0));
        let preset = requirements().prefer_lossless(true);
        let (index, _) = resolve_transcode(
            boundary(),
            &offers,
            &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Transcode { choices, .. } = q else {
                    panic!()
                };
                Ok(Decision::choose(choices.first().unwrap()))
            },
            preset,
            &crate::how_far::NoPulse,
        )
        .unwrap();
        assert_eq!(index, 0);
        let (index, _) = resolve_transcode(
            boundary(),
            &offers,
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> { Ok(Decision::UsePreset) },
            preset,
            &crate::how_far::NoPulse,
        )
        .unwrap();
        assert_eq!(index, 1);
    }
}
