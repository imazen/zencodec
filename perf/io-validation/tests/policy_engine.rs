//! Shared resolver contracts. Costs/effects in these controls are authored;
//! native qualification is a separate suite, not implied by these declarations.
#![forbid(unsafe_code)]
#[path = "../src/policy/mod.rs"]
mod policy;
use core::{convert::Infallible, error::Error, fmt};
use policy::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use zencodec::encode::Fidelity;
use zencodec::{
    At, Unstoppable,
    enough::{Stop, StopReason},
};

fn engine() -> Engine {
    Engine::new(PolicySet::default(), 100)
}
fn requirements() -> Requirements {
    Requirements {
        ram: Some(64),
        disk: Some(0),
        accounting: Accounting::Operation,
        effects: Effects(0),
    }
}
fn context() -> Context {
    Context {
        source_revision: 7,
        plan_revision: 2,
        epoch: 0,
        published_bytes: 0,
        replayable: true,
    }
}
fn facts() -> Facts {
    Facts::Execution(ExecutionFacts {
        resident_input: 0,
        source_length: None,
        network: true,
    })
}
fn alternative(action: Action) -> Alternative {
    Alternative {
        action,
        ram: Evidence::Upper(8),
        disk: Evidence::Upper(0),
        accounting: Accounting::Operation,
        effects: if action == Action::Fidelity(Fidelity::Lossless) {
            Effects::LOSSLESS
        } else {
            Effects(0)
        },
        needs_replay: false,
        preserves_published: true,
    }
}
fn first(question: Question<'_>) -> Result<Decision, At<Infallible>> {
    Ok(Decision::choose(
        question.choices().unwrap().first().unwrap(),
    ))
}

#[derive(Debug)]
struct ApplicationError(&'static str);
impl fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for ApplicationError {}
fn callback_error(_: Question<'_>) -> Result<Decision, At<ApplicationError>> {
    Err(At::wrap(ApplicationError("private fixture payload")).at_named("application rule origin"))
}

#[test]
fn original_error_type_and_trace_survive_typed_and_erased_resolution() {
    let offers = [alternative(Action::Execution(Execution::Buffered))];
    let typed = engine()
        .resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            &mut callback_error,
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(typed.error().code(), Code::Callback);
    assert_eq!(typed.frame_count(), 2);
    assert_eq!(
        zencodec::find_cause::<ApplicationError>(&typed).unwrap().0,
        "private fixture payload"
    );
    assert!(format!("{}", typed.full_trace()).contains("application rule origin"));
    assert!(!format!("{typed}").contains("private fixture payload"));
    assert!(!format!("{:?}", typed.error()).contains("private fixture payload"));

    let mut erased: Box<dyn PolicyResolver<Error = ErasedError> + Send> =
        Box::new(Erased(callback_error));
    let error = engine()
        .resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            erased.as_mut(),
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(error.frame_count(), 3);
    assert_eq!(
        zencodec::find_cause::<ApplicationError>(&error).unwrap().0,
        "private fixture payload"
    );
    let trace = format!("{}", error.full_trace());
    assert!(trace.contains("application rule origin"));
    assert!(trace.contains("policy callback erasure"));
}

#[test]
fn deliberate_decline_has_its_own_code_native_cause_and_callback_location() {
    let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        Ok(Decision::reject(Decline::ApplicationRule))
    };
    let offers = [alternative(Action::Execution(Execution::Buffered))];
    let error = engine()
        .resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            &mut resolver,
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(error.error().code(), Code::Declined);
    assert_eq!(
        zencodec::find_cause::<Decline>(&error),
        Some(&Decline::ApplicationRule)
    );
    let trace = format!("{}", error.full_trace());
    assert!(
        trace.contains("policy_engine.rs"),
        "decline lost the application's location: {trace}"
    );
    assert_eq!(error.frame_count(), 2);
}

#[test]
fn one_closure_handles_metadata_execution_and_fidelity_with_preset_fallback() {
    let mut calls = Vec::new();
    let mut resolver = |question: Question<'_>| -> Result<Decision, At<Infallible>> {
        match question {
            Question::Metadata { facts, choices } if facts.has_location == Some(true) => {
                calls.push(Kind::Metadata);
                Ok(Decision::choose(
                    choices
                        .iter()
                        .find(|c| c.value.action == Action::Metadata(MetadataAction::FilterPublic))
                        .unwrap(),
                ))
            }
            Question::Execution { choices, .. } => {
                calls.push(Kind::Execution);
                Ok(Decision::choose(
                    choices
                        .iter()
                        .find(|c| c.value.action == Action::Execution(Execution::Sequential))
                        .unwrap(),
                ))
            }
            Question::Fidelity { .. } => {
                calls.push(Kind::Fidelity);
                Ok(Decision::UsePreset)
            }
            _ => Ok(Decision::reject(Decline::UnknownQuestion)),
        }
    };
    let mut engine = engine();
    let metadata = [
        alternative(Action::Metadata(MetadataAction::KeepExact)),
        alternative(Action::Metadata(MetadataAction::FilterPublic)),
    ];
    let q = Facts::Metadata(MetadataFacts {
        packet_bytes: 40,
        parse_complete: true,
        has_location: Some(true),
        has_rights: Some(true),
    });
    assert_eq!(
        engine
            .resolve(
                q,
                &metadata,
                requirements(),
                context(),
                &mut resolver,
                &Unstoppable
            )
            .unwrap()
            .index,
        1
    );
    let execution = [
        alternative(Action::Execution(Execution::Buffered)),
        alternative(Action::Execution(Execution::Sequential)),
    ];
    assert_eq!(
        engine
            .resolve(
                facts(),
                &execution,
                requirements(),
                context(),
                &mut resolver,
                &Unstoppable
            )
            .unwrap()
            .index,
        1
    );
    let fidelity = [
        alternative(Action::Fidelity(Fidelity::codec_quality(60.0))),
        alternative(Action::Fidelity(Fidelity::Lossless)),
    ];
    let q = Facts::Fidelity(FidelityFacts {
        alpha: true,
        bits_per_sample: 8,
    });
    assert_eq!(
        engine
            .resolve(
                q,
                &fidelity,
                requirements(),
                context(),
                &mut resolver,
                &Unstoppable
            )
            .unwrap()
            .index,
        1
    );
    assert_eq!(calls, [Kind::Metadata, Kind::Execution, Kind::Fidelity]);
    assert_eq!(engine.records().last().unwrap().route, Route::Preset);
}

#[test]
fn a_configured_policy_works_directly_or_as_the_single_fallback() {
    for buffered_cost in [16, 40] {
        let mut buffered = alternative(Action::Execution(Execution::Buffered));
        buffered.ram = Evidence::Upper(buffered_cost);
        let offers = [
            buffered,
            alternative(Action::Execution(Execution::Sequential)),
        ];
        let configured = PolicySet {
            execution: ExecutionPreference::BufferedUpTo(32),
            ..PolicySet::default()
        };
        let direct = engine()
            .resolve(
                facts(),
                &offers,
                requirements(),
                context(),
                &mut { configured },
                &Unstoppable,
            )
            .unwrap();
        let mut calls = 0;
        let mut defer = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            calls += 1;
            Ok(Decision::UsePreset)
        };
        let mut host = Engine::new(configured, 10);
        let fallback = host
            .resolve(
                facts(),
                &offers,
                requirements(),
                context(),
                &mut defer,
                &Unstoppable,
            )
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(direct.index, if buffered_cost == 16 { 0 } else { 1 });
        assert_eq!(fallback.index, direct.index);
    }
}

#[test]
fn empty_or_unsupported_choices_refuse_before_calling_application_code() {
    let mut called = false;
    let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        called = true;
        Ok(Decision::UsePreset)
    };
    let missing = engine()
        .resolve(
            facts(),
            &[],
            requirements(),
            context(),
            &mut resolver,
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(missing.error().code(), Code::NoAlternative);
    let mut offer = alternative(Action::Metadata(MetadataAction::DropDescriptive));
    offer.effects = Effects::NO_GPS;
    let mut r = requirements();
    r.effects = Effects::NO_GPS.union(Effects::RIGHTS);
    let q = Facts::Metadata(MetadataFacts {
        packet_bytes: 40,
        parse_complete: false,
        has_location: None,
        has_rights: Some(true),
    });
    let unsupported = engine()
        .resolve(q, &[offer], r, context(), &mut resolver, &Unstoppable)
        .unwrap_err();
    assert_eq!(unsupported.error().code(), Code::NoAlternative);
    assert!(!called);
}

#[test]
fn stale_choices_fail_across_calls_engine_instances_sources_and_epochs() {
    let offers = [alternative(Action::Execution(Execution::Buffered))];
    let mut old = None;
    let mut capture = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
        let choice = q.choices().unwrap().first().unwrap();
        old = Some(choice.id);
        Ok(Decision::choose(choice))
    };
    let mut original = engine();
    original
        .resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            &mut capture,
            &Unstoppable,
        )
        .unwrap();
    let mut stale = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        Ok(Decision::Choose(old.unwrap()))
    };
    for changed in [
        context(),
        Context {
            source_revision: 8,
            ..context()
        },
        Context {
            epoch: 1,
            ..context()
        },
        Context {
            plan_revision: 3,
            ..context()
        },
    ] {
        for host in [&mut original, &mut engine()] {
            let error = host
                .resolve(
                    facts(),
                    &offers,
                    requirements(),
                    changed,
                    &mut stale,
                    &Unstoppable,
                )
                .unwrap_err();
            assert_eq!(error.error().code(), Code::InvalidChoice);
        }
    }
}

#[test]
fn unknown_questions_delegate_once_then_refuse() {
    let mut calls = 0;
    let mut resolver = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
        assert!(matches!(q, Question::Unknown { code: 500 }));
        calls += 1;
        Ok(Decision::UsePreset)
    };
    let error = engine()
        .resolve(
            Facts::Unknown(500),
            &[],
            requirements(),
            context(),
            &mut resolver,
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(
        zencodec::find_cause::<Decline>(&error),
        Some(&Decline::UnknownQuestion)
    );
    assert_eq!(error.error().record().route, Route::Preset);
}

#[test]
fn a_hard_cap_accepts_upper_or_enforced_bounds_but_not_estimates_or_wrong_scope() {
    for cost in [
        Evidence::Upper(64),
        Evidence::Upper(65),
        Evidence::Enforced(64),
        Evidence::Enforced(65),
        Evidence::Estimate(1),
        Evidence::Unknown,
    ] {
        let mut offer = alternative(Action::Execution(Execution::Buffered));
        offer.ram = cost;
        let result = engine().resolve(
            facts(),
            &[offer],
            requirements(),
            context(),
            &mut first,
            &Unstoppable,
        );
        assert_eq!(
            result.is_ok(),
            matches!(cost, Evidence::Upper(64) | Evidence::Enforced(64))
        );
    }
    let mut offer = alternative(Action::Execution(Execution::Buffered));
    offer.accounting = Accounting::Driver;
    let error = engine()
        .resolve(
            facts(),
            &[offer],
            requirements(),
            context(),
            &mut first,
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(error.error().code(), Code::NoAlternative);
    let q = Facts::Execution(ExecutionFacts {
        resident_input: 100,
        source_length: None,
        network: false,
    });
    offer.accounting = Accounting::Operation;
    let error = engine()
        .resolve(
            q,
            &[offer],
            requirements(),
            context(),
            &mut first,
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(error.error().code(), Code::InvalidOffer);
}

#[test]
fn malformed_choices_are_not_hidden_by_an_earlier_valid_choice() {
    let q = Facts::Fidelity(FidelityFacts {
        alpha: false,
        bits_per_sample: 8,
    });
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let offers = [
            alternative(Action::Fidelity(Fidelity::Lossless)),
            alternative(Action::Fidelity(Fidelity::codec_quality(value))),
        ];
        let error = engine()
            .resolve(
                q,
                &offers,
                requirements(),
                context(),
                &mut first,
                &Unstoppable,
            )
            .unwrap_err();
        assert_eq!(error.error().code(), Code::InvalidOffer);
    }
    let wrong_family = [
        alternative(Action::Execution(Execution::Buffered)),
        alternative(Action::Metadata(MetadataAction::KeepExact)),
    ];
    assert_eq!(
        engine()
            .resolve(
                facts(),
                &wrong_family,
                requirements(),
                context(),
                &mut first,
                &Unstoppable
            )
            .unwrap_err()
            .error()
            .code(),
        Code::InvalidOffer
    );
    let mut invalid = requirements();
    invalid.effects = Effects(128);
    assert_eq!(
        engine()
            .resolve(
                facts(),
                &wrong_family[..1],
                invalid,
                context(),
                &mut first,
                &Unstoppable
            )
            .unwrap_err()
            .error()
            .code(),
        Code::InvalidRequirements
    );
}

#[test]
fn rewind_and_prefix_changes_are_removed_when_input_or_output_cannot_be_replayed() {
    let mut rewind = alternative(Action::Execution(Execution::Buffered));
    rewind.needs_replay = true;
    let mut changes_prefix = alternative(Action::Execution(Execution::Sequential));
    changes_prefix.preserves_published = false;
    let continuation = alternative(Action::Execution(Execution::DiskStaged));
    let offers = [rewind, changes_prefix, continuation];
    let initial = engine()
        .resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            &mut first,
            &Unstoppable,
        )
        .unwrap();
    assert_eq!(initial.index, 0);
    let forward = Context {
        replayable: false,
        ..context()
    };
    assert_eq!(
        engine()
            .resolve(
                facts(),
                &offers,
                requirements(),
                forward,
                &mut first,
                &Unstoppable
            )
            .unwrap()
            .index,
        1
    );
    let published = Context {
        published_bytes: 1,
        ..context()
    };
    assert_eq!(
        engine()
            .resolve(
                facts(),
                &offers,
                requirements(),
                published,
                &mut first,
                &Unstoppable
            )
            .unwrap()
            .index,
        2
    );
}

struct FlagStop(Arc<AtomicBool>);
impl Stop for FlagStop {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.load(Ordering::SeqCst) {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[test]
fn cancellation_before_and_after_callback_prevents_selection() {
    let offers = [alternative(Action::Execution(Execution::Buffered))];
    for initially_stopped in [true, false] {
        let flag = Arc::new(AtomicBool::new(initially_stopped));
        let stop = FlagStop(flag.clone());
        let mut calls = 0;
        let mut resolver = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            calls += 1;
            flag.store(true, Ordering::SeqCst);
            first(q)
        };
        let error = engine()
            .resolve(
                facts(),
                &offers,
                requirements(),
                context(),
                &mut resolver,
                &stop,
            )
            .unwrap_err();
        assert_eq!(calls, usize::from(!initially_stopped));
        assert_eq!(error.error().code(), Code::Stopped);
        assert!(matches!(
            error.error().cause(),
            Cause::Stopped(StopReason::Cancelled)
        ));
    }
}

#[test]
fn panic_is_not_relabelled_as_a_policy_refusal_and_the_engine_cannot_be_reused() {
    let offers = [alternative(Action::Execution(Execution::Buffered))];
    let mut host = engine();
    let mut panic =
        |_: Question<'_>| -> Result<Decision, At<Infallible>> { panic!("authored panic control") };
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        host.resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            &mut panic,
            &Unstoppable,
        )
    }));
    assert!(unwind.is_err());
    assert_eq!(
        host.resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            &mut first,
            &Unstoppable
        )
        .unwrap_err()
        .error()
        .code(),
        Code::InterruptedCallback
    );
}

#[test]
fn planning_candidate_and_ticket_limits_refuse_without_wrapping() {
    let offer = alternative(Action::Execution(Execution::Buffered));
    let offers = vec![offer; MAX_CHOICES];
    assert!(
        engine()
            .resolve(
                facts(),
                &offers,
                requirements(),
                context(),
                &mut first,
                &Unstoppable
            )
            .is_ok()
    );
    let too_many = vec![offer; MAX_CHOICES + 1];
    assert_eq!(
        engine()
            .resolve(
                facts(),
                &too_many,
                requirements(),
                context(),
                &mut first,
                &Unstoppable
            )
            .unwrap_err()
            .error()
            .code(),
        Code::TooManyChoices
    );
    let mut host = Engine::new(PolicySet::default(), 1);
    host.resolve(
        facts(),
        &offers,
        requirements(),
        context(),
        &mut first,
        &Unstoppable,
    )
    .unwrap();
    assert_eq!(
        host.resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            &mut first,
            &Unstoppable
        )
        .unwrap_err()
        .error()
        .code(),
        Code::PlanningLimit
    );
    let counter = AtomicUsize::new(usize::MAX - 1);
    assert_eq!(issue_ticket(&counter), Some((usize::MAX - 1) as u64));
    assert_eq!(issue_ticket(&counter), None);
    assert_eq!(issue_ticket(&counter), None);
}

#[test]
fn diagnostic_ring_is_bounded_and_chronological() {
    let offers = [alternative(Action::Execution(Execution::Buffered))];
    let mut host = engine();
    for _ in 0..40 {
        host.resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            &mut first,
            &Unstoppable,
        )
        .unwrap();
    }
    let records: Vec<_> = host.records().collect();
    assert_eq!(records.len(), JOURNAL_CAPACITY);
    assert_eq!(host.dropped_records(), (40 - JOURNAL_CAPACITY) as u64);
    assert_eq!(
        records.iter().map(|r| r.ordinal).collect::<Vec<_>>(),
        (24..40).collect::<Vec<_>>()
    );
    assert!(
        records
            .iter()
            .all(|r| r.code == Code::Selected && r.chosen == Some(0))
    );
}

#[test]
fn independent_thread_local_policies_do_not_share_mutable_state_or_tickets() {
    let profile = Arc::new(PolicySet {
        execution: ExecutionPreference::Sequential,
        ..PolicySet::default()
    });
    let tickets = std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..8)
            .map(|_| {
                let profile = profile.clone();
                scope.spawn(move || {
                    let mut host = Engine::new(*profile, 32);
                    let offers = [
                        alternative(Action::Execution(Execution::Buffered)),
                        alternative(Action::Execution(Execution::Sequential)),
                    ];
                    let mut calls = 0;
                    let mut callback = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                        calls += 1;
                        Ok(Decision::UsePreset)
                    };
                    let mut tickets = Vec::new();
                    for _ in 0..32 {
                        let selected = host
                            .resolve(
                                facts(),
                                &offers,
                                requirements(),
                                context(),
                                &mut callback,
                                &Unstoppable,
                            )
                            .unwrap();
                        assert_eq!(selected.index, 1);
                        tickets.push((selected.operation, selected.question));
                    }
                    assert_eq!(calls, 32);
                    tickets
                })
            })
            .collect();
        jobs.into_iter()
            .flat_map(|j| j.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(tickets.len(), 256);
    let questions: std::collections::BTreeSet<_> = tickets.iter().map(|(_, q)| q).collect();
    let operations: std::collections::BTreeSet<_> = tickets.iter().map(|(op, _)| op).collect();
    assert_eq!(questions.len(), tickets.len());
    assert_eq!(operations.len(), 8);
}

#[test]
fn callback_errors_without_frames_or_at_the_trace_limit_keep_their_cause() {
    let offers = [alternative(Action::Execution(Execution::Buffered))];
    for frames in [0, zencodec::whereat::AT_MAX_FRAMES + 10] {
        let mut callback = |_: Question<'_>| -> Result<Decision, At<ApplicationError>> {
            let mut error = At::wrap(ApplicationError("retained original"));
            for _ in 0..frames {
                error = error.at();
            }
            Err(error)
        };
        let error = engine()
            .resolve(
                facts(),
                &offers,
                requirements(),
                context(),
                &mut callback,
                &Unstoppable,
            )
            .unwrap_err();
        assert_eq!(
            zencodec::find_cause::<ApplicationError>(&error).unwrap().0,
            "retained original"
        );
        assert_eq!(
            error.frame_count(),
            if frames == 0 {
                1
            } else {
                zencodec::whereat::AT_MAX_FRAMES
            }
        );
    }
}

#[test]
fn callback_failure_remains_primary_when_cancellation_also_arrives() {
    let flag = Arc::new(AtomicBool::new(false));
    let stop = FlagStop(flag.clone());
    let mut callback = |_: Question<'_>| -> Result<Decision, At<ApplicationError>> {
        flag.store(true, Ordering::SeqCst);
        Err(At::wrap(ApplicationError("native resolver failure")).at())
    };
    let offers = [alternative(Action::Execution(Execution::Buffered))];
    let error = engine()
        .resolve(
            facts(),
            &offers,
            requirements(),
            context(),
            &mut callback,
            &stop,
        )
        .unwrap_err();
    assert_eq!(error.error().code(), Code::Callback);
    assert_eq!(
        zencodec::find_cause::<ApplicationError>(&error).unwrap().0,
        "native resolver failure"
    );
}
