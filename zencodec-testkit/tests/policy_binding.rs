//! Executable API experiment, not a new production API.
//!
//! Requirements belong to the operation binding, outside callback wrappers.
//! The candidate delegates admission to the real shared resolver. It does not
//! solve deferred obligations across decode/processing/encode boundaries.
use std::{cell::Cell, convert::Infallible};
use zencodec::{
    At, NoPulse,
    decode::{DecodeBoundary, InterpretationFacts, InterpretationState},
    encode::OutputTiming,
    operation::{
        Decision, DecisionCode, DecisionRecord, OutputSchedule, OutputScheduleFacts,
        OutputScheduleOffer, OutputSource, PolicyFailure, PolicyRequirement, PolicyResolver,
        PolicySet, Question, RequirementEvidence, resolve_boundary, resolve_output_schedule,
    },
};
use zenpixels::PixelDescriptor;

// Deliberately does NOT implement PolicyResolver. An operation takes this
// binding; instrumentation wraps only its callback and cannot hide the set.
struct BoundPolicy<R> {
    requirements: PolicySet,
    resolver: R,
}
impl<R: PolicyResolver> BoundPolicy<R> {
    fn new(requirements: PolicySet, resolver: R) -> Self {
        Self {
            requirements,
            resolver,
        }
    }
    fn schedule(
        &mut self,
        offers: &[OutputScheduleOffer],
    ) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
        resolve_output_schedule(
            OutputScheduleFacts::new(OutputTiming::streaming(0), OutputSource::PullRows),
            offers,
            &mut self.resolver,
            self.requirements,
            &NoPulse,
        )
    }
    fn boundary(
        &mut self,
        state: InterpretationState,
    ) -> Result<(zencodec::decode::BoundaryAction, DecisionRecord), At<PolicyFailure>> {
        resolve_boundary(
            DecodeBoundary::new(InterpretationFacts::new(
                3,
                2,
                (state == InterpretationState::Final).then_some(PixelDescriptor::RGB8_SRGB),
                state,
            ))
            .unwrap(),
            &mut self.resolver,
            self.requirements,
            &NoPulse,
        )
    }
}

fn preset(_: Question<'_>) -> Result<Decision, At<Infallible>> {
    Ok(Decision::UsePreset)
}

struct Logged<R> {
    inner: R,
    calls: Cell<usize>,
}
impl<R: PolicyResolver> PolicyResolver for Logged<R> {
    type Error = R::Error;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<Self::Error>> {
        self.calls.set(self.calls.get() + 1);
        self.inner.resolve(question)
    }
}

#[test]
fn bound_requirements_survive_a_callback_only_logging_wrapper() {
    let mut policy = BoundPolicy::new(
        PolicySet::new().with_output_wait_limit(8),
        Logged {
            inner: preset,
            calls: Cell::new(0),
        },
    );
    let error = policy
        .schedule(&[OutputScheduleOffer::new(OutputSchedule::FinishFirst)])
        .unwrap_err();
    assert_eq!(policy.resolver.calls.get(), 0);
    let record = error.error().record();
    assert_eq!(record.code, DecisionCode::Declined);
    assert_eq!((record.offered, record.admitted), (1, 0));
    assert_eq!(
        record.unmet_requirements.iter().collect::<Vec<_>>(),
        [(PolicyRequirement::OutputWait, RequirementEvidence::Unknown)]
    );
}

#[test]
fn bound_direct_choice_sees_only_compliant_original_indices() {
    let mut offers = [OutputScheduleOffer::new(OutputSchedule::Live); 3];
    offers[1].wait_retained_upper_bound = Some(9);
    offers[2].wait_retained_upper_bound = Some(8);
    let mut policy = BoundPolicy::new(
        PolicySet::new().with_output_wait_limit(8),
        |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::OutputSchedule { choices, .. } = q else {
                panic!("wrong domain")
            };
            assert_eq!(choices.len(), 1);
            assert_eq!(
                choices.first().unwrap().value().wait_retained_upper_bound,
                Some(8)
            );
            Ok(Decision::choose(choices.first().unwrap()))
        },
    );
    let (index, record) = policy.schedule(&offers).unwrap();
    assert_eq!(index, 2);
    assert_eq!((record.offered, record.admitted), (3, 1));
    assert_eq!(
        record.unmet_requirements.iter().collect::<Vec<_>>(),
        [
            (PolicyRequirement::OutputWait, RequirementEvidence::Unknown),
            (
                PolicyRequirement::OutputWait,
                RequirementEvidence::Contradicted
            ),
        ]
    );
}

#[test]
fn bound_schedule_refusal_names_the_contradiction() {
    let mut policy = BoundPolicy::new(
        PolicySet::new().require_output_schedule(OutputSchedule::Live),
        preset,
    );
    let error = policy
        .schedule(&[OutputScheduleOffer::new(OutputSchedule::FinishFirst)])
        .unwrap_err();
    assert_eq!(
        error
            .error()
            .record()
            .unmet_requirements
            .iter()
            .collect::<Vec<_>>(),
        [(
            PolicyRequirement::OutputSchedule,
            RequirementEvidence::Contradicted
        )]
    );
}

#[test]
fn binding_does_not_turn_discovery_into_discharge_of_output_obligations() {
    let mut policy = BoundPolicy::new(
        PolicySet::new().require_output_schedule(OutputSchedule::FinishFirst),
        preset,
    );
    policy.boundary(InterpretationState::Pending).unwrap();
    let error = policy.boundary(InterpretationState::Final).unwrap_err();
    assert_eq!(
        error
            .error()
            .record()
            .unmet_requirements
            .iter()
            .collect::<Vec<_>>(),
        [(
            PolicyRequirement::OutputSchedule,
            RequirementEvidence::Unknown
        )]
    );
    // A decoder adoption boundary may publish rows. Sharing this binding with
    // an encoder is not permission to postpone an unproven output obligation.
    // A future pipeline contract must prove intermediate ownership first.
}

#[test]
fn callback_failure_preserves_its_typed_cause_through_the_binding() {
    #[derive(Debug)]
    struct TenantFailure;
    impl std::fmt::Display for TenantFailure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("private tenant detail")
        }
    }
    impl std::error::Error for TenantFailure {}
    let mut policy = BoundPolicy::new(
        PolicySet::new(),
        |_: Question<'_>| -> Result<Decision, At<TenantFailure>> {
            Err(At::wrap(TenantFailure).at())
        },
    );
    let error = policy
        .schedule(&[OutputScheduleOffer::new(OutputSchedule::Live)])
        .unwrap_err();
    assert_eq!(error.error().record().code, DecisionCode::CallbackFailed);
    assert!(!error.to_string().contains("private tenant detail"));
    assert!(zencodec::find_cause::<TenantFailure>(&error).is_some());
}
