//! Joint admission of metadata and publication evidence from one retained route.
use super::*;
use crate::{
    ImageFormat,
    decode::{InterpretationFacts, InterpretationState},
    encode::Fidelity,
    how_far::NoPulse,
    operation::{
        BoundPolicy, Decision, OutputSchedule, OutputScheduleOffer, Question, resolve_boundary,
    },
};
use core::convert::Infallible;

fn boundary(state: InterpretationState) -> DecodeBoundary {
    DecodeBoundary::new(InterpretationFacts::new(
        1,
        1,
        Some(PixelDescriptor::RGB8_SRGB),
        state,
    ))
    .unwrap()
}
fn offer() -> TranscodeOffer {
    let mut o = TranscodeOffer::new(
        EncodeOffer {
            format: ImageFormat::Png,
            fidelity: Some(Fidelity::Lossless),
            preference_index: 0,
            resolution: None,
        },
        PixelDescriptor::RGB8_SRGB,
        MetadataTransfer::DiscardDescriptive,
    );
    o.retains_opaque_profile = Some(false);
    let mut schedule = OutputScheduleOffer::new(OutputSchedule::FinishFirst);
    schedule.wait_retained_upper_bound = Some(4096); // Synthetic policy-test bound, not a codec measurement.
    o.output_schedule = Some(schedule);
    o
}
fn requirements() -> PolicySet {
    PolicySet::new()
        .require_metadata(MetadataTransfer::DiscardDescriptive)
        .allow_opaque_profiles(false)
        .require_output_schedule(OutputSchedule::FinishFirst)
        .with_output_wait_limit(4096)
}

#[test]
fn combined_evidence_is_filtered_before_callback_and_builtin_selection() {
    let good = offer();
    let mut offers = [good; 6];
    offers[0].metadata = MetadataTransfer::PreserveReported;
    offers[1].output_schedule = None;
    offers[2].output_schedule.as_mut().unwrap().schedule = OutputSchedule::Live;
    offers[3]
        .output_schedule
        .as_mut()
        .unwrap()
        .wait_retained_upper_bound = None;
    offers[4]
        .output_schedule
        .as_mut()
        .unwrap()
        .wait_retained_upper_bound = Some(4097);
    let mut callback = BoundPolicy::new(
        requirements(),
        |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Transcode { choices, .. } = q else {
                panic!("wrong question")
            };
            assert_eq!(choices.len(), 1);
            assert!(core::ptr::eq(choices.first().unwrap().value(), &offers[5]));
            Ok(Decision::choose(choices.first().unwrap()))
        },
    );
    let (index, record) = resolve_transcode(
        boundary(InterpretationState::Final),
        &offers,
        &mut callback,
        PolicySet::new(),
        Some(&NoPulse),
    )
    .unwrap();
    assert_eq!(index, 5);
    assert_eq!((record.offered, record.admitted), (6, 1));
    let mut builtin = requirements();
    assert_eq!(
        resolve_transcode(
            boundary(InterpretationState::Final),
            &offers,
            &mut builtin,
            PolicySet::new(),
            Some(&NoPulse)
        )
        .unwrap()
        .0,
        5
    );
    let mut delegated = BoundPolicy::new(
        requirements(),
        |_: Question<'_>| -> Result<Decision, At<Infallible>> { Ok(Decision::UsePreset) },
    );
    assert_eq!(
        resolve_transcode(
            boundary(InterpretationState::Final),
            &offers,
            &mut delegated,
            PolicySet::new(),
            Some(&NoPulse)
        )
        .unwrap()
        .0,
        5
    );
}

#[test]
fn complete_route_does_not_authorize_bare_decode_or_unqualified_workspace() {
    let mut policy = requirements();
    resolve_boundary(
        boundary(InterpretationState::Pending),
        &mut policy,
        PolicySet::new(),
        Some(&NoPulse),
    )
    .unwrap();
    let final_boundary = boundary(InterpretationState::Final);
    resolve_transcode(
        final_boundary,
        &[offer()],
        &mut policy,
        PolicySet::new(),
        Some(&NoPulse),
    )
    .unwrap();
    // Only the executing adapter directly resumes the held boundary, with its
    // private sink installed. A separate native-sink question remains unproven.
    assert!(
        resolve_boundary(
            final_boundary,
            &mut policy,
            PolicySet::new(),
            Some(&NoPulse)
        )
        .is_err()
    );
    let mut workspace = requirements().with_workspace_limit(u64::MAX);
    assert!(
        resolve_transcode(
            final_boundary,
            &[offer()],
            &mut workspace,
            PolicySet::new(),
            Some(&NoPulse)
        )
        .is_err()
    );
}

#[test]
fn finish_first_without_wait_evidence_stays_unknown() {
    let mut o = offer();
    o.output_schedule
        .as_mut()
        .unwrap()
        .wait_retained_upper_bound = None;
    assert!(
        resolve_transcode(
            boundary(InterpretationState::Final),
            &[o],
            &mut requirements(),
            PolicySet::new(),
            Some(&NoPulse)
        )
        .is_err()
    );
    let mut policy = PolicySet::new()
        .require_metadata(MetadataTransfer::DiscardDescriptive)
        .require_output_schedule(OutputSchedule::FinishFirst);
    resolve_transcode(
        boundary(InterpretationState::Final),
        &[o],
        &mut policy,
        PolicySet::new(),
        Some(&NoPulse),
    )
    .unwrap();
}

#[test]
fn missing_or_contradictory_publication_evidence_names_both_requirements() {
    use crate::operation::{DecisionCode, PolicyRequirement, RequirementEvidence};
    use core::cell::Cell;
    for contradiction in [false, true] {
        let mut candidate = offer();
        let evidence = if contradiction {
            let schedule = candidate.output_schedule.as_mut().unwrap();
            schedule.schedule = OutputSchedule::Live;
            schedule.wait_retained_upper_bound = Some(4097);
            RequirementEvidence::Contradicted
        } else {
            candidate.output_schedule = None;
            RequirementEvidence::Unknown
        };
        let called = Cell::new(false);
        let mut policy = BoundPolicy::new(
            requirements(),
            |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                called.set(true);
                let Question::Transcode { choices, .. } = q else {
                    panic!("wrong question")
                };
                Ok(Decision::choose(choices.first().unwrap()))
            },
        );
        let error = resolve_transcode(
            boundary(InterpretationState::Final),
            &[candidate],
            &mut policy,
            PolicySet::new(),
            Some(&NoPulse),
        )
        .unwrap_err();
        assert!(
            !called.get(),
            "unqualified routes reached application selection"
        );
        let record = error.error().record();
        assert_eq!(record.code, DecisionCode::Declined);
        assert_eq!((record.offered, record.admitted), (1, 0));
        assert_eq!(
            record
                .unmet_requirements
                .iter()
                .collect::<alloc::vec::Vec<_>>(),
            [
                (PolicyRequirement::OutputSchedule, evidence),
                (PolicyRequirement::OutputWait, evidence),
            ]
        );
    }
}
