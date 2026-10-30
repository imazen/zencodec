use enough::StopReason;
use zencodec_testkit::pulse::{Case, Limits, check_checkpoints};

fn limits() -> Limits {
    Limits {
        checkpoints: 8,
        trace_capacity: 64,
    }
}

#[test]
fn visits_every_checkpoint_and_preserves_both_reasons() {
    let mut cases = Vec::new();
    check_checkpoints(
        limits(),
        |pulse| {
            for _ in 0..3 {
                pulse.check()?;
            }
            Ok::<_, StopReason>(())
        },
        |e| Some(*e),
        |case, trace| {
            cases.push(case);
            assert_eq!(trace.active_spans, 0);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(cases.len(), 7);
    assert_eq!(cases[0], Case::Uninterrupted);
    for checkpoint in 0..3 {
        for (i, reason) in [StopReason::Cancelled, StopReason::TimedOut]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                cases[1 + checkpoint * 2 + i],
                Case::StopAt { checkpoint, reason }
            );
        }
    }
}

#[test]
fn swallowed_one_shot_is_rejected_and_failure_trace_is_observable() {
    let mut observed = Vec::new();
    let result = check_checkpoints(
        limits(),
        |pulse| {
            let _ = pulse.check();
            pulse.check()
        },
        |e| Some(*e),
        |case, trace| {
            observed.push((
                case,
                trace.spans.iter().any(|s| s.stats.stop_reason.is_some()),
            ));
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(
        observed.last(),
        Some(&(
            Case::StopAt {
                checkpoint: 0,
                reason: StopReason::Cancelled
            },
            true
        ))
    );
}

#[test]
fn collapsed_timeout_is_rejected() {
    assert!(
        check_checkpoints(
            limits(),
            |pulse| pulse.check().map_err(|_| StopReason::Cancelled),
            |e| Some(*e),
            |_, _| Ok(())
        )
        .is_err()
    );
}

#[test]
fn missing_checks_and_excessive_sweep_refuse() {
    assert!(
        check_checkpoints(
            limits(),
            |_| Ok::<_, StopReason>(()),
            |e| Some(*e),
            |_, _| Ok(())
        )
        .is_err()
    );
    let mut executions = 0;
    assert!(
        check_checkpoints(
            Limits {
                checkpoints: 1,
                ..limits()
            },
            |pulse| {
                executions += 1;
                pulse.check()?;
                pulse.check()
            },
            |e| Some(*e),
            |_, _| Ok(())
        )
        .is_err()
    );
    assert_eq!(executions, 1);
}

#[test]
fn dropped_diagnostics_and_observer_failure_are_not_skipped() {
    assert!(
        check_checkpoints(
            Limits {
                trace_capacity: 0,
                ..limits()
            },
            |pulse| pulse.check(),
            |e| Some(*e),
            |_, _| Ok(())
        )
        .is_err()
    );
    let error = check_checkpoints(
        limits(),
        |pulse| pulse.check(),
        |e| Some(*e),
        |_, _| {
            Err(zencodec_testkit::Failure {
                check: "artifact",
                detail: "write failed".into(),
            })
        },
    )
    .unwrap_err();
    assert_eq!(error.check, "artifact");
}

#[test]
fn abandoned_child_is_rejected() {
    use zencodec::how_far::{Execution, PhaseSpec, Report, Total};
    assert!(
        check_checkpoints(
            limits(),
            |pulse| {
                pulse.check()?;
                let children = pulse
                    .split(
                        Execution::Sequence,
                        &[PhaseSpec::new("work", 1, Total::Unknown)],
                    )
                    .unwrap();
                children[0].advance(1);
                drop(children);
                Ok::<_, StopReason>(())
            },
            |e| Some(*e),
            |_, _| Ok(())
        )
        .is_err()
    );
}
