use super::*;
use crate::{
    ImageFormat,
    encode::{ColorCarriers, EncodePlanError, Fidelity},
    operation::{
        BoundPolicy, Decision, DecisionCode, PolicyRequirement, RequirementEvidence, StagingFacts,
        StagingKind,
    },
};
use alloc::{sync::Arc, vec::Vec};
use core::convert::Infallible;
use zenpixels::PixelDescriptor;

fn declaration() -> EncodeDeclaration<'static> {
    EncodeDeclaration::new(7, 9, PixelDescriptor::RGB8_SRGB)
}
fn job_evidence() -> EncodeEvidence {
    EncodeEvidence::new(EncodeOffer {
        format: ImageFormat::Png,
        fidelity: Some(Fidelity::Lossless),
        preference_index: 5,
        resolution: None,
    })
}
fn plan(max: Option<u32>) -> EncodePlan {
    EncodePlan::replay(
        SourcePasses::new(1, max).unwrap(),
        OutputTiming::streaming(0),
        OutputPhase::FinalPass,
    )
}
fn offer(acquisition: InputAcquisition) -> EncodeRouteOffer {
    EncodeRouteOffer::new(
        acquisition,
        OutputScheduleOffer::new(OutputSchedule::FinishFirst),
    )
}
fn resolve(
    plan: &EncodePlan,
    offers: &[EncodeRouteOffer],
    requirements: PolicySet,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    resolve_encode_route(
        &declaration(),
        plan,
        job_evidence(),
        offers,
        &mut { requirements },
        PolicySet::new(),
        Some(&crate::how_far::NoPulse),
    )
}
fn failures(error: &At<PolicyFailure>) -> Vec<(PolicyRequirement, RequirementEvidence)> {
    error.error().record().unmet_requirements.iter().collect()
}

#[test]
fn finite_pass_caps_use_external_maximum_and_staging_does_not_prove_termination() {
    for max in [Some(1), Some(2), None] {
        let plan = plan(max);
        for cap in [0, 1, 2, u32::MAX] {
            let requirements = PolicySet::new().with_max_source_passes(cap);
            for acquisition in [
                InputAcquisition::BorrowedRaster,
                InputAcquisition::DirectRows,
                InputAcquisition::MemorySnapshot,
                InputAcquisition::DiskReplay,
            ] {
                let staged = matches!(
                    acquisition,
                    InputAcquisition::MemorySnapshot | InputAcquisition::DiskReplay
                );
                let expected = if staged {
                    cap >= 1
                } else {
                    max.is_some_and(|max| max <= cap)
                };
                let result = resolve(&plan, &[offer(acquisition)], requirements);
                assert_eq!(
                    result.is_ok(),
                    expected,
                    "{acquisition:?} {max:?} cap={cap}"
                );
                if let Err(error) = result {
                    assert!(
                        failures(&error)
                            .iter()
                            .any(|(r, _)| *r == PolicyRequirement::SourcePasses)
                    );
                }
            }
        }
        // Unlimited is not implemented as a finite u32::MAX cap.
        assert!(
            resolve(
                &plan,
                &[offer(InputAcquisition::DirectRows)],
                PolicySet::new()
            )
            .is_ok()
        );
    }
}

#[test]
fn exact_requires_actual_borrow_or_private_snapshot_and_rejects_mispaired_claims() {
    for acquisition in [
        InputAcquisition::BorrowedRaster,
        InputAcquisition::DirectRows,
        InputAcquisition::MemorySnapshot,
        InputAcquisition::DiskReplay,
    ] {
        for replay in [
            ReplayEvidence::Unknown,
            ReplayEvidence::ImmutableBorrow,
            ReplayEvidence::PrivateSnapshot,
            ReplayEvidence::VerifiedHash,
            ReplayEvidence::VerifiedIdentity,
        ] {
            let mut candidate = offer(acquisition);
            candidate.replay = replay;
            let expected = matches!(
                (acquisition, replay),
                (
                    InputAcquisition::BorrowedRaster,
                    ReplayEvidence::ImmutableBorrow
                ) | (
                    InputAcquisition::MemorySnapshot,
                    ReplayEvidence::PrivateSnapshot
                )
            );
            assert_eq!(
                resolve(
                    &plan(Some(2)),
                    &[candidate],
                    PolicySet::new().require_exact_replay(true)
                )
                .is_ok(),
                expected
            );
        }
    }
    let mut forged = offer(InputAcquisition::DirectRows);
    forged.replay = ReplayEvidence::ImmutableBorrow;
    let error = resolve(&plan(Some(2)), &[forged], PolicySet::new()).unwrap_err();
    assert_eq!(
        failures(&error),
        [(
            PolicyRequirement::EncodeRoute,
            RequirementEvidence::Contradicted
        )]
    );
}

#[test]
fn callback_cannot_bypass_requirements_and_only_sees_original_admitted_slots() {
    let plan = plan(None);
    let mut offers = [offer(InputAcquisition::DirectRows); 64];
    offers[63] = offer(InputAcquisition::MemorySnapshot);
    offers[63].replay = ReplayEvidence::PrivateSnapshot;
    let requirements = PolicySet::new()
        .with_max_source_passes(1)
        .require_exact_replay(true);
    let mut calls = 0;
    let (selected, record) = resolve_encode_route(
        &declaration(),
        &plan,
        job_evidence(),
        &offers,
        &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            calls += 1;
            let Question::EncodeRoute { choices, .. } = q else {
                panic!()
            };
            assert_eq!(choices.len(), 1);
            let choice = choices.first().unwrap();
            assert!(core::ptr::eq(choice.value(), &offers[63]));
            Ok(Decision::choose(choice))
        },
        requirements,
        Some(&crate::how_far::NoPulse),
    )
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(selected, 63);
    assert_eq!((record.offered, record.admitted), (64, 1));
    assert!(!record.used_preset);
    for requirements in [requirements, PolicySet::new().with_max_source_passes(0)] {
        let error = resolve_encode_route(
            &declaration(),
            &plan,
            job_evidence(),
            &offers[..63],
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                panic!("mandatory refusal must precede callback")
            },
            requirements,
            Some(&crate::how_far::NoPulse),
        )
        .unwrap_err();
        assert_eq!(error.error().record().admitted, 0);
        assert_eq!(error.error().record().code, DecisionCode::Declined);
    }
}

#[test]
fn input_and_output_are_selected_together_under_bound_and_legacy_conjunction() {
    let plan = plan(Some(2));
    let mut candidates = [
        offer(InputAcquisition::DirectRows),
        offer(InputAcquisition::MemorySnapshot),
        offer(InputAcquisition::MemorySnapshot),
    ];
    candidates[1].output = OutputScheduleOffer::new(OutputSchedule::Live);
    candidates[1].replay = ReplayEvidence::PrivateSnapshot;
    candidates[2].replay = ReplayEvidence::PrivateSnapshot;
    let legacy = PolicySet::new().with_max_source_passes(1);
    let required = PolicySet::new()
        .require_exact_replay(true)
        .require_output_schedule(OutputSchedule::FinishFirst);
    for delegate in [false, true] {
        let mut bound = BoundPolicy::new(
            required,
            |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::EncodeRoute { choices, .. } = q else {
                    panic!()
                };
                assert_eq!(choices.len(), 1);
                Ok(if delegate {
                    Decision::UsePreset
                } else {
                    Decision::choose(choices.first().unwrap())
                })
            },
        );
        let (index, record) = resolve_encode_route(
            &declaration(),
            &plan,
            job_evidence(),
            &candidates,
            &mut bound,
            legacy,
            Some(&crate::how_far::NoPulse),
        )
        .unwrap();
        assert_eq!(index, 2);
        assert_eq!(record.used_preset, delegate);
    }
    // Requiring no raw RAM staging conflicts with the external one-pass cap.
    let mut bound = BoundPolicy::new(
        required.allow_raw_memory_staging(false),
        |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            panic!("no admissible joint route")
        },
    );
    let error = resolve_encode_route(
        &declaration(),
        &plan,
        job_evidence(),
        &candidates,
        &mut bound,
        legacy,
        Some(&crate::how_far::NoPulse),
    )
    .unwrap_err();
    assert!(failures(&error).contains(&(
        PolicyRequirement::RawMemoryStaging,
        RequirementEvidence::Contradicted
    )));
}

#[test]
fn metadata_workspace_profile_and_range_require_evidence_from_the_same_job() {
    let requirements = PolicySet::new()
        .require_metadata(MetadataTransfer::DiscardDescriptive)
        .allow_opaque_profiles(false)
        .with_workspace_limit(8)
        .require_extended_range(true);
    let mut candidate = offer(InputAcquisition::BorrowedRaster);
    let unknown = resolve(&plan(Some(1)), &[candidate], requirements).unwrap_err();
    assert_eq!(
        failures(&unknown),
        [
            (PolicyRequirement::Metadata, RequirementEvidence::Unknown),
            (
                PolicyRequirement::OpaqueProfiles,
                RequirementEvidence::Unknown
            ),
            (PolicyRequirement::Workspace, RequirementEvidence::Unknown),
            (
                PolicyRequirement::ExtendedRange,
                RequirementEvidence::Unknown
            ),
        ]
    );
    let mut evidence = job_evidence();
    evidence.metadata = Some(MetadataTransfer::DiscardDescriptive);
    evidence.retains_opaque_profile = Some(false);
    candidate.workspace_upper_bound = Some(8);
    candidate.preserves_extended_range = Some(true);
    let resolve_job = |evidence, candidate| {
        resolve_encode_route(
            &declaration(),
            &plan(Some(1)),
            evidence,
            &[candidate],
            &mut { requirements },
            PolicySet::new(),
            Some(&crate::how_far::NoPulse),
        )
    };
    assert!(resolve_job(evidence, candidate).is_ok());
    for field in 0..4 {
        let mut wrong_route = candidate;
        let mut wrong_job = evidence;
        match field {
            0 => wrong_job.metadata = Some(MetadataTransfer::PreserveReported),
            1 => wrong_job.retains_opaque_profile = Some(true),
            2 => wrong_route.workspace_upper_bound = Some(9),
            _ => wrong_route.preserves_extended_range = Some(false),
        }
        assert!(resolve_job(wrong_job, wrong_route).is_err());
    }
    for fidelity in [None, Some(Fidelity::codec_quality(80.0))] {
        let mut job = evidence;
        job.encoder.fidelity = fidelity;
        assert!(resolve_job(job, candidate).is_err());
    }
}

#[test]
fn raw_ram_ban_does_not_ban_compressed_ram_staging_or_claim_a_memory_bound() {
    let mut candidate = offer(InputAcquisition::BorrowedRaster);
    candidate.output.staging = Some(StagingFacts::new(StagingKind::Memory, 0, 4096));
    assert!(
        resolve(
            &plan(Some(2)),
            &[candidate],
            PolicySet::new().allow_raw_memory_staging(false)
        )
        .is_ok()
    );
    assert!(
        resolve(
            &plan(Some(2)),
            &[candidate],
            PolicySet::new().with_output_wait_limit(4096)
        )
        .is_err()
    );
}

#[test]
fn final_pass_timing_is_not_wait_retention_and_the_plan_is_borrowed() {
    let plan = plan(Some(2)).with_carriers(ColorCarriers::new(Some(Arc::from([7; 2048])), None));
    let mut candidate = offer(InputAcquisition::BorrowedRaster);
    candidate.output = OutputScheduleOffer::new(OutputSchedule::Live);
    let requirements = PolicySet::new().with_output_wait_limit(0);
    let error = resolve(&plan, &[candidate], requirements).unwrap_err();
    assert_eq!(
        failures(&error),
        [(PolicyRequirement::OutputWait, RequirementEvidence::Unknown)]
    );
    candidate.output.wait_retained_upper_bound = Some(0);
    let (_, record) = resolve_encode_route(
        &declaration(),
        &plan,
        job_evidence(),
        &[candidate],
        &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::EncodeRoute {
                plan: borrowed,
                facts,
                choices,
                ..
            } = q
            else {
                panic!()
            };
            assert!(core::ptr::eq(borrowed, &plan));
            assert_eq!(
                Arc::strong_count(plan.carriers.as_ref().unwrap().icc.as_ref().unwrap()),
                1
            );
            assert_eq!(facts.output_phase, OutputPhase::FinalPass);
            assert_eq!(facts.timing, OutputTiming::streaming(0));
            assert_eq!(facts.evidence.encoder.preference_index, 5);
            Ok(Decision::choose(choices.first().unwrap()))
        },
        requirements,
        Some(&crate::how_far::NoPulse),
    )
    .unwrap();
    assert!(
        matches!(record.context, DecisionContext::EncodeRoute(f) if f.output_phase == OutputPhase::FinalPass)
    );
    assert!(!alloc::format!("{record:?}").contains("7, 7, 7"));
    let mut observed = plan.clone();
    observed.output_phase = OutputPhase::AnyPass;
    assert!(plan.validate_started(&observed).is_err());
    observed = plan.clone();
    observed.timing = OutputTiming::streaming(1);
    assert!(plan.validate_started(&observed).is_err());
}

#[test]
fn incompatible_routes_are_filtered_before_callback_even_without_requirements() {
    let plan = EncodePlan::whole_raster();
    let mut candidates = [
        offer(InputAcquisition::Unknown),
        offer(InputAcquisition::DirectRows),
        offer(InputAcquisition::BorrowedRaster),
        offer(InputAcquisition::BorrowedRaster),
    ];
    candidates[2].output = OutputScheduleOffer::new(OutputSchedule::Live);
    let (index, record) = resolve_encode_route(
        &declaration(),
        &plan,
        job_evidence(),
        &candidates,
        &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::EncodeRoute { choices, .. } = q else {
                panic!()
            };
            assert_eq!(choices.len(), 1);
            Ok(Decision::choose(choices.first().unwrap()))
        },
        PolicySet::new(),
        Some(&crate::how_far::NoPulse),
    )
    .unwrap();
    assert_eq!(index, 3);
    assert_eq!((record.offered, record.admitted), (4, 1));
    assert_eq!(
        record.unmet_requirements.iter().collect::<Vec<_>>(),
        [(
            PolicyRequirement::EncodeRoute,
            RequirementEvidence::Contradicted
        )]
    );
    let mut invalid = plan.clone();
    invalid.timing = OutputTiming::streaming(0);
    let error = resolve_encode_route(
        &declaration(),
        &invalid,
        job_evidence(),
        &candidates,
        &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> { panic!("invalid plan") },
        PolicySet::new(),
        Some(&crate::how_far::NoPulse),
    )
    .unwrap_err();
    assert_eq!(error.error().record().admitted, 0);
}

#[test]
fn unrelated_domains_do_not_discharge_new_requirements() {
    for required in [
        PolicySet::new().with_max_source_passes(1),
        PolicySet::new().require_exact_replay(true),
        PolicySet::new().allow_raw_memory_staging(false),
    ] {
        let error = crate::operation::resolve_output_schedule(
            crate::operation::OutputScheduleFacts::new(
                OutputTiming::streaming(0),
                crate::operation::OutputSource::BorrowedRaster,
            ),
            &[OutputScheduleOffer::new(OutputSchedule::FinishFirst)],
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                panic!("wrong-domain evidence")
            },
            required,
            Some(&crate::how_far::NoPulse),
        )
        .unwrap_err();
        assert_eq!(failures(&error).len(), 1);
        assert_eq!(failures(&error)[0].1, RequirementEvidence::Unknown);
    }
}

#[test]
fn callback_errors_keep_their_original_cause_and_tokens_cannot_cross_questions() {
    #[derive(Debug)]
    struct ApplicationFailure;
    impl core::fmt::Display for ApplicationFailure {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("private callback payload")
        }
    }
    impl core::error::Error for ApplicationFailure {}
    let candidates = [offer(InputAcquisition::BorrowedRaster)];
    let error = resolve_encode_route(
        &declaration(),
        &plan(Some(2)),
        job_evidence(),
        &candidates,
        &mut |_: Question<'_>| -> Result<Decision, At<ApplicationFailure>> {
            Err(At::wrap(ApplicationFailure))
        },
        PolicySet::new(),
        Some(&crate::how_far::NoPulse),
    )
    .unwrap_err();
    assert!(crate::find_cause::<ApplicationFailure>(error.error()).is_some());
    assert!(
        !alloc::format!("{} {:?}", error.error(), error.error())
            .contains("private callback payload")
    );
    assert_eq!(error.error().record().code, DecisionCode::CallbackFailed);

    let mut token = None;
    resolve_encode_route(
        &declaration(),
        &plan(Some(2)),
        job_evidence(),
        &candidates,
        &mut |question: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::EncodeRoute { choices, .. } = question else {
                panic!()
            };
            token = Some(choices.first().unwrap().id());
            Ok(Decision::UsePreset)
        },
        PolicySet::new(),
        Some(&crate::how_far::NoPulse),
    )
    .unwrap();
    let error = resolve_encode_route(
        &declaration(),
        &plan(Some(2)),
        job_evidence(),
        &candidates,
        &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            Ok(Decision::Choose(token.unwrap()))
        },
        PolicySet::new(),
        Some(&crate::how_far::NoPulse),
    )
    .unwrap_err();
    assert_eq!(error.error().record().code, DecisionCode::InvalidChoice);
}

#[test]
fn discovery_may_continue_but_final_adoption_cannot_bypass_replay_obligations() {
    use crate::decode::{BoundaryAction, DecodeBoundary, InterpretationFacts, InterpretationState};
    for requirements in [
        PolicySet::new().with_max_source_passes(1),
        PolicySet::new().require_exact_replay(true),
        PolicySet::new().allow_raw_memory_staging(false),
    ] {
        for state in [InterpretationState::Provisional, InterpretationState::Final] {
            let boundary = DecodeBoundary::new(InterpretationFacts::new(
                7,
                9,
                Some(PixelDescriptor::RGB8_SRGB),
                state,
            ))
            .unwrap();
            let mut calls = 0;
            let result = crate::operation::resolve_boundary(
                boundary,
                &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                    calls += 1;
                    let Question::Boundary { choices, .. } = q else {
                        panic!()
                    };
                    Ok(Decision::choose(choices.first().unwrap()))
                },
                requirements,
                Some(&crate::how_far::NoPulse),
            );
            if state == InterpretationState::Final {
                assert_eq!(calls, 0);
                assert_eq!(
                    failures(&result.unwrap_err())[0].1,
                    RequirementEvidence::Unknown
                );
            } else {
                assert_eq!(calls, 1);
                assert_eq!(result.unwrap().0, BoundaryAction::ContinueReading);
            }
        }
    }
}

#[test]
fn proven_single_native_pass_needs_no_replay_evidence_or_raw_snapshot() {
    let plans = [
        EncodePlan::rows(OutputTiming::streaming(0)),
        EncodePlan::whole_raster(),
        EncodePlan::replay(
            SourcePasses::ONCE,
            OutputTiming::streaming(0),
            OutputPhase::FinalPass,
        ),
    ];
    let required = PolicySet::new()
        .require_exact_replay(true)
        .allow_raw_memory_staging(false)
        .with_max_source_passes(1);
    for plan in plans {
        let acquisition = if plan.input == InputRequirement::WholeRaster {
            InputAcquisition::BorrowedRaster
        } else {
            InputAcquisition::DirectRows
        };
        for replay in [
            ReplayEvidence::Unknown,
            ReplayEvidence::VerifiedHash,
            ReplayEvidence::VerifiedIdentity,
        ] {
            let mut candidate = offer(acquisition);
            candidate.replay = replay;
            for delegate in [false, true] {
                let mut bound = BoundPolicy::new(
                    required,
                    |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                        let Question::EncodeRoute { choices, .. } = q else {
                            panic!()
                        };
                        Ok(if delegate {
                            Decision::UsePreset
                        } else {
                            Decision::choose(choices.first().unwrap())
                        })
                    },
                );
                let (_, record) = resolve_encode_route(
                    &declaration(),
                    &plan,
                    job_evidence(),
                    &[candidate],
                    &mut bound,
                    required,
                    Some(&crate::how_far::NoPulse),
                )
                .unwrap();
                // No fabricated exact-borrow or snapshot evidence in the record.
                assert!(
                    matches!(record.selected, Some(Selection::EncodeRoute(o)) if o.replay == replay)
                );
            }
            let error = resolve_encode_route(
                &declaration(),
                &plan,
                job_evidence(),
                &[candidate],
                &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                    panic!("zero cap still forbids source reads")
                },
                required.with_max_source_passes(0),
                Some(&crate::how_far::NoPulse),
            )
            .unwrap_err();
            assert_eq!(
                failures(&error),
                [(
                    PolicyRequirement::SourcePasses,
                    RequirementEvidence::Contradicted
                )]
            );
        }
    }
}

#[test]
fn one_external_staging_pass_does_not_exempt_unknown_or_multiple_native_passes() {
    for max in [None, Some(2)] {
        for acquisition in [
            InputAcquisition::DirectRows,
            InputAcquisition::DiskReplay,
            InputAcquisition::MemorySnapshot,
            InputAcquisition::BorrowedRaster,
        ] {
            for replay in [
                ReplayEvidence::Unknown,
                ReplayEvidence::VerifiedHash,
                ReplayEvidence::VerifiedIdentity,
            ] {
                let mut candidate = offer(acquisition);
                candidate.replay = replay;
                let error = resolve_encode_route(
                    &declaration(),
                    &plan(max),
                    job_evidence(),
                    &[candidate],
                    &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                        panic!("native replay remains unqualified")
                    },
                    PolicySet::new().require_exact_replay(true),
                    Some(&crate::how_far::NoPulse),
                )
                .unwrap_err();
                assert_eq!(
                    failures(&error),
                    [(
                        PolicyRequirement::ExactReplay,
                        if replay == ReplayEvidence::Unknown {
                            RequirementEvidence::Unknown
                        } else {
                            RequirementEvidence::Contradicted
                        }
                    )]
                );
            }
        }
        for (acquisition, replay) in [
            (
                InputAcquisition::BorrowedRaster,
                ReplayEvidence::ImmutableBorrow,
            ),
            (
                InputAcquisition::MemorySnapshot,
                ReplayEvidence::PrivateSnapshot,
            ),
        ] {
            let mut candidate = offer(acquisition);
            candidate.replay = replay;
            assert!(
                resolve(
                    &plan(max),
                    &[candidate],
                    PolicySet::new().require_exact_replay(true)
                )
                .is_ok()
            );
        }
    }
}

#[test]
fn single_pass_exemption_does_not_admit_incompatible_routes_or_metadata() {
    let mut false_borrow = offer(InputAcquisition::DirectRows);
    false_borrow.replay = ReplayEvidence::ImmutableBorrow;
    let mut live = offer(InputAcquisition::BorrowedRaster);
    live.output = OutputScheduleOffer::new(OutputSchedule::Live);
    for (plan, candidate) in [
        (plan(Some(1)), offer(InputAcquisition::Unknown)),
        (plan(Some(1)), false_borrow),
        (
            EncodePlan::whole_raster(),
            offer(InputAcquisition::DirectRows),
        ),
        (
            EncodePlan::whole_raster(),
            offer(InputAcquisition::DiskReplay),
        ),
        (EncodePlan::whole_raster(), live),
    ] {
        let error = resolve_encode_route(
            &declaration(),
            &plan,
            job_evidence(),
            &[candidate],
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                panic!("incompatible route")
            },
            PolicySet::new().require_exact_replay(true),
            Some(&crate::how_far::NoPulse),
        )
        .unwrap_err();
        assert_eq!(error.error().record().code, DecisionCode::Declined);
        assert_eq!(
            failures(&error),
            [(
                PolicyRequirement::EncodeRoute,
                RequirementEvidence::Contradicted
            )]
        );
        assert!(crate::find_cause::<EncodePlanError>(error.error()).is_none());
    }
    let error = resolve(
        &plan(Some(1)),
        &[offer(InputAcquisition::DirectRows)],
        PolicySet::new()
            .require_exact_replay(true)
            .require_metadata(MetadataTransfer::DiscardDescriptive),
    )
    .unwrap_err();
    assert_eq!(
        failures(&error),
        [(PolicyRequirement::Metadata, RequirementEvidence::Unknown)]
    );
}

#[test]
fn invalid_shared_plans_keep_typed_causes_including_with_no_offers() {
    let mut invalid = EncodePlan::whole_raster();
    invalid.timing = OutputTiming::streaming(0);
    let cases = [
        (
            EncodeDeclaration::new(0, 9, PixelDescriptor::RGB8_SRGB),
            plan(Some(1)),
            EncodePlanError::Dimensions,
        ),
        (
            declaration(),
            EncodePlan::rows(OutputTiming::streaming(9)),
            EncodePlanError::OutputLag,
        ),
        (
            declaration(),
            EncodePlan::rows(OutputTiming::AT_FINISH.with_patches()),
            EncodePlanError::DestinationAccess,
        ),
        (declaration(), invalid, EncodePlanError::InputProtocol),
    ];
    for (declaration, plan, cause) in cases {
        for offers in [&[][..], &[offer(InputAcquisition::BorrowedRaster)][..]] {
            let error = resolve_encode_route(
                &declaration,
                &plan,
                job_evidence(),
                offers,
                &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                    panic!("invalid plan")
                },
                PolicySet::new().require_exact_replay(true),
                Some(&crate::how_far::NoPulse),
            )
            .unwrap_err();
            assert_eq!(error.error().record().code, DecisionCode::InvalidOffers);
            assert_eq!(error.error().record().offered, offers.len() as u8);
            assert_eq!(error.error().record().admitted, 0);
            assert!(failures(&error).is_empty());
            assert_eq!(
                crate::find_cause::<EncodePlanError>(error.error()),
                Some(&cause)
            );
            assert_eq!(
                core::error::Error::source(error.error())
                    .unwrap()
                    .downcast_ref::<EncodePlanError>(),
                Some(&cause)
            );
        }
    }
}

#[test]
fn shared_job_metadata_applies_to_every_route_and_survives_in_context() {
    let mut evidence = job_evidence();
    evidence.metadata = Some(MetadataTransfer::DiscardDescriptive);
    evidence.retains_opaque_profile = Some(false);
    let mut routes = [
        offer(InputAcquisition::BorrowedRaster),
        offer(InputAcquisition::DirectRows),
    ];
    routes[0].workspace_upper_bound = Some(9);
    routes[1].workspace_upper_bound = Some(8);
    routes[0].preserves_extended_range = Some(false);
    routes[1].preserves_extended_range = Some(true);
    let required = PolicySet::new()
        .require_metadata(MetadataTransfer::DiscardDescriptive)
        .allow_opaque_profiles(false)
        .require_exact_replay(true)
        .with_workspace_limit(8)
        .require_extended_range(true);
    let (index, record) = resolve_encode_route(
        &declaration(),
        &plan(Some(1)),
        evidence,
        &routes,
        &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::EncodeRoute { facts, choices, .. } = q else {
                panic!()
            };
            assert_eq!(facts.evidence.metadata, evidence.metadata);
            assert_eq!(
                facts.evidence.retains_opaque_profile,
                evidence.retains_opaque_profile
            );
            assert_eq!(choices.len(), 1);
            assert!(core::ptr::eq(choices.first().unwrap().value(), &routes[1]));
            Ok(Decision::choose(choices.first().unwrap()))
        },
        required,
        Some(&crate::how_far::NoPulse),
    )
    .unwrap();
    assert_eq!(index, 1);
    let DecisionContext::EncodeRoute(facts) = record.context else {
        panic!()
    };
    assert_eq!(facts.evidence.metadata, evidence.metadata);
    assert_eq!(
        facts.evidence.retains_opaque_profile,
        evidence.retains_opaque_profile
    );
    for field in 0..4 {
        let mut job = evidence;
        let (requirement, reason) = match field {
            0 => {
                job.metadata = Some(MetadataTransfer::PreserveReported);
                (
                    PolicyRequirement::Metadata,
                    RequirementEvidence::Contradicted,
                )
            }
            1 => {
                job.metadata = None;
                (PolicyRequirement::Metadata, RequirementEvidence::Unknown)
            }
            2 => {
                job.retains_opaque_profile = Some(true);
                (
                    PolicyRequirement::OpaqueProfiles,
                    RequirementEvidence::Contradicted,
                )
            }
            _ => {
                job.retains_opaque_profile = None;
                (
                    PolicyRequirement::OpaqueProfiles,
                    RequirementEvidence::Unknown,
                )
            }
        };
        let error = resolve_encode_route(
            &declaration(),
            &plan(Some(1)),
            job,
            &routes,
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                panic!("shared job evidence applies to every route")
            },
            required,
            Some(&crate::how_far::NoPulse),
        )
        .unwrap_err();
        assert_eq!(error.error().record().code, DecisionCode::Declined);
        assert_eq!(
            (
                error.error().record().offered,
                error.error().record().admitted
            ),
            (2, 0)
        );
        assert!(failures(&error).contains(&(requirement, reason)));
    }
}

#[test]
fn no_pulse_keeps_default_success_and_mandatory_admission_distinct() {
    let offers = [offer(InputAcquisition::BorrowedRaster)];
    let result = resolve_encode_route(
        &declaration(),
        &plan(Some(1)),
        job_evidence(),
        &offers,
        &mut PolicySet::new(),
        PolicySet::new(),
        None,
    )
    .unwrap();
    assert_eq!(result.0, 0);
    assert_eq!(result.1.admitted, 1);

    let error = resolve_encode_route(
        &declaration(),
        &plan(Some(1)),
        job_evidence(),
        &offers,
        &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            panic!("a callback cannot bypass mandatory metadata evidence")
        },
        PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
        None,
    )
    .unwrap_err();
    assert_eq!(error.error().record().admitted, 0);
    assert!(
        failures(&error).contains(&(PolicyRequirement::Metadata, RequirementEvidence::Unknown))
    );
}

#[test]
fn contradictory_orientation_refuses_before_callback() {
    let mut orientation = crate::encode::OrientationPlan::preserved(crate::Orientation::Rotate90);
    orientation.output = crate::Orientation::Identity;
    let plan = EncodePlan::whole_raster().with_orientation(orientation);
    let mut called = false;
    let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        called = true;
        Ok(Decision::UsePreset)
    };
    let error = resolve_encode_route(
        &declaration(),
        &plan,
        job_evidence(),
        &[offer(InputAcquisition::BorrowedRaster)],
        &mut resolver,
        PolicySet::new(),
        None,
    )
    .unwrap_err();
    assert!(!called);
    assert_eq!(
        crate::find_cause::<EncodePlanError>(&error),
        Some(&EncodePlanError::Orientation)
    );
}
