//! Reusable synchronous Pulse checkpoint conformance with retained diagnostics.
use crate::{Conformance, fail};
use enough::{Stop, StopReason};
use how_far_along::{Outcome, Phase, PulseTree, Snapshot, Status, Total};
use how_far_really::{
    diagnostics::{DiagnosticPulse, Kind, Options},
    profile::{Profiler, SpanKind, StdClock, Trace},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec::Pulse;

/// Which independently constructed operation produced an observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Case {
    /// Successful discovery run; its observed check count defines the sweep.
    Uninterrupted,
    /// Inject one stop at this zero-based check, then return success on later polls.
    StopAt {
        checkpoint: usize,
        reason: StopReason,
    },
}

/// Explicit bounds on repeated executions and retained diagnostic evidence.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum checkpoints in the discovery run; excess refuses, never samples.
    pub checkpoints: usize,
    /// Profiler capacity per execution; dropped evidence fails conformance.
    pub trace_capacity: usize,
}

fn finished_tree(tree: &Snapshot) -> bool {
    matches!(tree.status, Status::Finished(outcome) if outcome != Outcome::Abandoned)
        && tree.children.iter().all(finished_tree)
}

struct StopAt {
    calls: Arc<AtomicUsize>,
    case: Case,
}
impl Stop for StopAt {
    fn check(&self) -> Result<(), StopReason> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        if let Case::StopAt { checkpoint, reason } = self.case
            && index == checkpoint
        {
            return Err(reason);
        }
        Ok(())
    }
}

/// Exercise every observed cancellation checkpoint with both stop reasons.
///
/// `run` must construct a fresh synchronous operation on the same successful
/// fixture every time, and join its workers before returning. It can verify
/// pixels itself before returning success. `classify` must extract the original
/// typed stop reason, not infer it from display text. `inspect` receives every
/// completed trace, including failed conformance cases, before built-in checks;
/// use it to retain artifacts and assert codec-specific work units and phases.
///
/// This checks one-shot stop preservation, bounded diagnostic retention and
/// reporting-protocol integrity. It does not require a particular phase shape,
/// prove useful progress, measure latency, cover report-triggered stops, or
/// bound a native call's runtime. Put the test command under an external timeout.
/// A nondeterministic path that does not reach an injected checkpoint fails;
/// choose deterministic fixtures or qualify its different schedules separately.
/// No failed checkpoint is silently skipped.
pub fn check_checkpoints<E: core::fmt::Debug>(
    limits: Limits,
    mut run: impl FnMut(&dyn Pulse) -> Result<(), E>,
    classify: impl Fn(&E) -> Option<StopReason>,
    mut inspect: impl FnMut(Case, &Trace) -> Conformance,
) -> Conformance {
    const CHECK: &str = "pulse_checkpoints";
    let mut execute = |case| -> Result<usize, crate::Failure> {
        let calls = Arc::new(AtomicUsize::new(0));
        let profiler = Profiler::new(StdClock::new(), limits.trace_capacity);
        let operation = profiler.span(None, "codec operation", SpanKind::Work);
        let pulse = DiagnosticPulse::new(
            PulseTree::new(
                Phase::new("codec", Total::Unknown),
                StopAt {
                    calls: calls.clone(),
                    case,
                },
            ),
            &profiler,
        );
        let observer = pulse.observer();
        let result = run(&pulse);
        profiler.operation_returned();
        // Report the ACTUAL returned outcome; reporting the expected one would
        // hide swallowed stops from the diagnostic oracle.
        let outcome = match &result {
            Ok(()) => Outcome::Succeeded,
            Err(error) if classify(error).is_some() => Outcome::Cancelled,
            Err(_) => Outcome::Failed,
        };
        let finish = pulse.finish(outcome);
        operation.finish(outcome);
        let trace = profiler.snapshot().with_progress(observer.snapshot());
        inspect(case, &trace)?;
        finish.map_err(|e| fail(CHECK, format!("{case:?}: finish: {e:?}")))?;
        let count = calls.load(Ordering::SeqCst);
        match case {
            Case::Uninterrupted => {
                if result.is_err() || count == 0 || count > limits.checkpoints {
                    return Err(fail(
                        CHECK,
                        format!(
                            "discovery: result={result:?}, checks={count}, limit={}",
                            limits.checkpoints
                        ),
                    ));
                }
            }
            Case::StopAt { checkpoint, reason } => {
                if count <= checkpoint || result.as_ref().err().and_then(&classify) != Some(reason)
                {
                    return Err(fail(
                        CHECK,
                        format!("{case:?}: checks={count}, result={result:?}"),
                    ));
                }
                if !trace
                    .spans
                    .iter()
                    .any(|span| span.stats.stop_reason == Some(reason))
                {
                    return Err(fail(CHECK, format!("{case:?}: missing stop evidence")));
                }
            }
        }
        if !trace.progress.as_ref().is_some_and(finished_tree) {
            return Err(fail(
                CHECK,
                format!("{case:?}: unfinished or abandoned progress phase"),
            ));
        }
        if trace.active_spans != 0
            || trace.dropped_spans != 0
            || trace.dropped_incidents != 0
            || !trace.incidents.is_empty()
        {
            return Err(fail(
                CHECK,
                format!("{case:?}: incomplete or invalid diagnostic trace: {trace:?}"),
            ));
        }
        if trace
            .diagnose(&Options::default())
            .iter()
            .any(|finding| finding.kind == Kind::StopOutcomeMismatch)
        {
            return Err(fail(CHECK, format!("{case:?}: stop/outcome mismatch")));
        }
        Ok(count)
    };
    let checks = execute(Case::Uninterrupted)?;
    for checkpoint in 0..checks {
        for reason in [StopReason::Cancelled, StopReason::TimedOut] {
            execute(Case::StopAt { checkpoint, reason })?;
        }
    }
    Ok(())
}
