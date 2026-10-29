//! Native checkpoint observations; no wall-clock performance assertions.
use how_far_along::{Outcome, Phase, PulseTree, Total};
use how_far_really::{
    diagnostics::{DiagnosticPulse, Kind, Options},
    profile::{Profiler, SpanKind, StdClock},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec::enough::{Stop, StopReason};

struct StopAt {
    calls: Arc<AtomicUsize>,
    at: usize,
    reason: StopReason,
}
impl Stop for StopAt {
    fn check(&self) -> Result<(), StopReason> {
        // A one-shot stop detects codecs which poll again and accidentally
        // replace the first interruption with a later successful check.
        if self.calls.fetch_add(1, Ordering::SeqCst) == self.at {
            Err(self.reason)
        } else {
            Ok(())
        }
    }
}

fn observed_decode(bytes: &[u8], progressive: bool, at: usize, reason: StopReason) -> usize {
    let calls = Arc::new(AtomicUsize::new(0));
    let profiler = Profiler::new(StdClock::new(), 4096);
    let operation = profiler.span(None, "native JPEG decode", SpanKind::Work);
    let pulse = DiagnosticPulse::new(
        PulseTree::new(
            Phase::new("decode", Total::Unknown),
            StopAt {
                calls: calls.clone(),
                at,
                reason,
            },
        ),
        &profiler,
    );
    let observer = pulse.observer();
    let result = zenjpeg::decoder::Decoder::new().decode_with_pulse(bytes, Some(&pulse));
    let outcome = if at == usize::MAX {
        assert!(result.is_ok(), "{result:?}");
        Outcome::Succeeded
    } else {
        assert!(
            matches!(result.as_ref().map_err(|e| e.kind()),
            Err(zenjpeg::decoder::ErrorKind::Cancelled(actual)) if *actual == reason),
            "checkpoint {at}, reason {reason:?}: {result:?}"
        );
        Outcome::Cancelled
    };
    // Synchronous decoder call has returned; no spawned task is inferred done.
    profiler.operation_returned();
    pulse.finish(outcome).unwrap();
    operation.finish(outcome);
    let trace = profiler.snapshot().with_progress(observer.snapshot());
    assert_eq!(trace.active_spans, 0);
    assert_eq!(trace.dropped_spans, 0);
    assert_eq!(trace.dropped_incidents, 0);
    if at != usize::MAX {
        assert!(
            trace
                .spans
                .iter()
                .any(|s| s.stats.stop_reason == Some(reason))
        );
    }
    let root = std::env::var_os("JPEG_POLICY_ARTIFACTS")
        .expect("set JPEG_POLICY_ARTIFACTS to retain diagnostic traces");
    let dir = std::path::PathBuf::from(root).join(format!("pulse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut json = String::new();
    trace.write_json(&mut json).unwrap();
    std::fs::write(
        dir.join(format!("progressive-{progressive}-at-{at}-{reason:?}.json")),
        json,
    )
    .unwrap();
    if at == usize::MAX {
        assert!(
            trace.spans.iter().any(|s| s.stats.reports > 0),
            "successful native decode must report progress through DiagnosticPulse"
        );
    }
    let findings = trace.diagnose(&Options::default());
    assert!(
        !findings.iter().any(|f| f.kind == Kind::StopOutcomeMismatch),
        "{findings:?}"
    );
    calls.load(Ordering::SeqCst)
}

#[test]
fn native_jpeg_preserves_one_shot_stops_with_diagnostics() {
    for progressive in [false, true] {
        let bytes = crate::fixtures::base(progressive);
        let checks = observed_decode(&bytes, progressive, usize::MAX, StopReason::Cancelled);
        assert!(
            checks > 2,
            "need distinct early/middle/last native checkpoints"
        );
        for at in [0, checks / 2, checks - 1] {
            for reason in [StopReason::Cancelled, StopReason::TimedOut] {
                observed_decode(&bytes, progressive, at, reason);
            }
        }
    }
}

#[test]
fn native_jpeg_meets_shared_incremental_contract() {
    for progressive in [false, true] {
        let bytes = crate::fixtures::base(progressive);
        zencodec_testkit::check_incremental_contract(zenjpeg::JpegDecoderConfig::new(), &bytes)
            .unwrap();
    }
}
