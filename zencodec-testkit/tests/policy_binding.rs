//! Production operation binding: requirements stay outside callback wrappers.
//!
//! Requirements belong to the operation binding, outside callback wrappers.
//! The binding delegates admission to the real shared resolver. It does not
//! solve deferred obligations across decode/processing/encode boundaries.
use std::{cell::Cell, convert::Infallible};
use zencodec::{
    At, NoPulse,
    decode::{DecodeBoundary, InterpretationFacts, InterpretationState},
    encode::OutputTiming,
    operation::{
        BoundPolicy, Decision, DecisionCode, DecisionRecord, OutputSchedule, OutputScheduleFacts,
        OutputScheduleOffer, OutputSource, PolicyFailure, PolicyRequirement, PolicyResolver,
        PolicySet, Question, RequirementEvidence, resolve_boundary, resolve_output_schedule,
    },
};
use zenpixels::PixelDescriptor;

trait ExerciseBinding {
    fn schedule(
        &mut self,
        offers: &[OutputScheduleOffer],
    ) -> Result<(usize, DecisionRecord), At<PolicyFailure>>;
    fn boundary(
        &mut self,
        state: InterpretationState,
    ) -> Result<(zencodec::decode::BoundaryAction, DecisionRecord), At<PolicyFailure>>;
}
impl<R: PolicyResolver> ExerciseBinding for BoundPolicy<R> {
    fn schedule(
        &mut self,
        offers: &[OutputScheduleOffer],
    ) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
        resolve_output_schedule(
            OutputScheduleFacts::new(OutputTiming::streaming(0), OutputSource::PullRows),
            offers,
            self,
            PolicySet::new(),
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
            self,
            PolicySet::new(),
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
    assert_eq!(policy.resolver_mut().calls.get(), 0);
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

#[test]
fn direct_preset_names_unknown_wait_and_contradictory_schedule() {
    for (mut policy, requirement, evidence) in [
        (
            PolicySet::new().with_output_wait_limit(8),
            PolicyRequirement::OutputWait,
            RequirementEvidence::Unknown,
        ),
        (
            PolicySet::new().require_output_schedule(OutputSchedule::Live),
            PolicyRequirement::OutputSchedule,
            RequirementEvidence::Contradicted,
        ),
    ] {
        let error = resolve_output_schedule(
            OutputScheduleFacts::new(OutputTiming::AT_FINISH, OutputSource::PullRows),
            &[OutputScheduleOffer::new(OutputSchedule::FinishFirst)],
            &mut policy,
            PolicySet::new(),
            &NoPulse,
        )
        .unwrap_err();
        let record = error.error().record();
        assert_eq!((record.offered, record.admitted), (1, 0));
        assert_eq!(
            record.unmet_requirements.iter().collect::<Vec<_>>(),
            [(requirement, evidence)]
        );
        assert!(!record.used_preset, "admission precedes built-in selection");
    }
}

#[test]
fn legacy_and_bound_requirements_intersect_without_replacing_each_other() {
    for (legacy, bound) in [
        (
            PolicySet::new().require_output_schedule(OutputSchedule::Live),
            PolicySet::new().with_output_wait_limit(8),
        ),
        (
            PolicySet::new().with_output_wait_limit(8),
            PolicySet::new().require_output_schedule(OutputSchedule::Live),
        ),
    ] {
        let mut calls = 0;
        let mut policy = BoundPolicy::new(
            bound,
            |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                calls += 1;
                let Question::OutputSchedule { choices, .. } = q else {
                    panic!("wrong domain")
                };
                assert_eq!(choices.len(), 1);
                Ok(Decision::choose(choices.first().unwrap()))
            },
        );
        let mut offers = [
            OutputScheduleOffer::new(OutputSchedule::FinishFirst),
            OutputScheduleOffer::new(OutputSchedule::Live),
            OutputScheduleOffer::new(OutputSchedule::Live),
        ];
        offers[0].wait_retained_upper_bound = Some(8);
        offers[1].wait_retained_upper_bound = Some(9);
        offers[2].wait_retained_upper_bound = Some(8);
        let (index, record) = resolve_output_schedule(
            OutputScheduleFacts::new(OutputTiming::streaming(0), OutputSource::PullRows),
            &offers,
            &mut policy,
            legacy,
            &NoPulse,
        )
        .unwrap();
        assert_eq!(index, 2);
        assert_eq!((record.offered, record.admitted), (3, 1));
        assert_eq!(calls, 1);
    }
}

#[test]
fn erased_callback_and_reborrowed_operation_keep_the_same_admission() {
    use zencodec::operation::OperationPolicy;
    let mut binding = BoundPolicy::new(PolicySet::new().with_output_wait_limit(8), preset);
    let erased: &mut BoundPolicy<dyn PolicyResolver<Error = Infallible>> = &mut binding;
    let mut borrowed = &mut *erased;
    let operation: &mut dyn OperationPolicy<Error = Infallible> = &mut borrowed;
    let error = resolve_output_schedule(
        OutputScheduleFacts::new(OutputTiming::streaming(0), OutputSource::PullRows),
        &[OutputScheduleOffer::new(OutputSchedule::Live)],
        operation,
        PolicySet::new(),
        &NoPulse,
    )
    .unwrap_err();
    assert_eq!(
        error
            .error()
            .record()
            .unmet_requirements
            .iter()
            .collect::<Vec<_>>(),
        [(PolicyRequirement::OutputWait, RequirementEvidence::Unknown)]
    );
}
