//! Native checkpoints, not elapsed-time guesses, synchronize request abort.
include!("../../../docs/examples/request_cancellation.rs");
use how_far_along::{Outcome, Phase, PulseTree, Total};
use how_far_really::{
    diagnostics::{DiagnosticPulse, Kind, Options},
    profile::{Profiler, SpanKind, StdClock},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use zencodec::{
    enough::{Stop, StopReason},
    how_far,
};

struct Gate {
    checks: AtomicUsize,
    at: usize,
    cancelled: Arc<AtomicBool>,
    rendezvous: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
}
impl Stop for Gate {
    fn check(&self) -> Result<(), StopReason> {
        if self.checks.fetch_add(1, Ordering::SeqCst) == self.at {
            let (started, release) = self.rendezvous.lock().unwrap().take().unwrap();
            started.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        if self.cancelled.load(Ordering::SeqCst) {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}
impl how_far::Report for Gate {
    fn advance(&self, _: u64) {}
}
impl how_far::Pulse for Gate {
    fn split(
        &self,
        _: how_far::Execution,
        _: &[how_far::PhaseSpec<'_>],
    ) -> Result<Vec<how_far::Child<'_>>, how_far::PlanError> {
        Err(how_far::PlanError::Unsupported)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_request_cancels_native_jpeg_at_early_middle_and_last_checkpoints() {
    for progressive in [false, true] {
        let bytes = crate::fixtures::fixture(progressive, crate::fixtures::Placement::Header);
        let baseline = Gate {
            checks: AtomicUsize::new(0),
            at: usize::MAX,
            cancelled: Arc::new(AtomicBool::new(false)),
            rendezvous: Mutex::new(None),
        };
        zenjpeg::decoder::Decoder::new()
            .decode_with_pulse(&bytes, Some(&baseline))
            .unwrap();
        let checks = baseline.checks.load(Ordering::SeqCst);
        assert!(checks > 2, "need distinct native checkpoints");
        for at in [0, checks / 2, checks - 1] {
            let slots = Arc::new(tokio::sync::Semaphore::new(1));
            let permit = slots.clone().acquire_owned().await.unwrap();
            let cancelled = Arc::new(AtomicBool::new(false));
            let cancel = cancelled.clone();
            let (started, ready) = tokio::sync::oneshot::channel();
            let (release, wait) = std::sync::mpsc::channel();
            let (result_tx, result_rx) = tokio::sync::oneshot::channel();
            let gate = Gate {
                checks: AtomicUsize::new(0),
                at,
                cancelled,
                rendezvous: Mutex::new(Some((started, wait))),
            };
            let profiler = Profiler::new(StdClock::new(), 4096);
            let cancellation_profiler = profiler.clone();
            let input = bytes.clone();
            let request = tokio::spawn(cancel_on_request_drop(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let operation =
                            profiler.span(None, "native blocking JPEG worker", SpanKind::Work);
                        let pulse = DiagnosticPulse::new(
                            PulseTree::new(Phase::new("decode", Total::Unknown), gate),
                            &profiler,
                        );
                        let observer = pulse.observer();
                        let result = zenjpeg::decoder::Decoder::new()
                            .decode_with_pulse(&input, Some(&pulse));
                        // The independent receipt survives request abort. A worker
                        // panic closes the channel, which fails the control.
                        let stopped = matches!(
                            result.as_ref().map_err(|e| e.kind()),
                            Err(zenjpeg::decoder::ErrorKind::Cancelled(
                                StopReason::Cancelled
                            ))
                        );
                        // The typed native cause was checked above. Only a unit
                        // completion crosses the deliberately aborted task boundary.
                        let completion = result.map(|_| ()).map_err(|_| ());
                        drop(input);
                        let outcome = if stopped {
                            Outcome::Cancelled
                        } else {
                            Outcome::Failed
                        };
                        pulse.finish(outcome).unwrap();
                        operation.finish(outcome);
                        // Request cancellation is not worker completion. Release
                        // the admitted slot only after native and diagnostic owners.
                        drop(permit);
                        profiler.operation_returned();
                        result_tx
                            .send((
                                stopped,
                                profiler.snapshot().with_progress(observer.snapshot()),
                            ))
                            .unwrap();
                        completion
                    })
                    .await
                    .unwrap()
                },
                move || {
                    cancellation_profiler.cancellation_requested();
                    cancel.store(true, Ordering::SeqCst);
                },
            ));
            tokio::time::timeout(Duration::from_secs(10), ready)
                .await
                .unwrap()
                .unwrap();
            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
            assert!(slots.clone().try_acquire_owned().is_err());
            release.send(()).unwrap();
            let (stopped, trace) = tokio::time::timeout(Duration::from_secs(10), result_rx)
                .await
                .unwrap()
                .unwrap();
            assert!(
                stopped,
                "progressive={progressive}, checkpoint={at}/{checks}"
            );
            assert_eq!(trace.active_spans, 0);
            assert_eq!(trace.dropped_spans, 0);
            assert_eq!(trace.dropped_incidents, 0);
            let requested = trace.cancelled_at.expect("request drop recorded");
            let observed = trace.observed_at.expect("native observation recorded");
            let returned = trace.returned_at.expect("worker cleanup recorded");
            assert!(requested <= observed && observed <= returned);
            let findings = trace.diagnose(&Options::default());
            assert!(
                !findings.iter().any(|f| matches!(
                    f.kind,
                    Kind::StopOutcomeMismatch | Kind::UnobservedCancellation
                )),
                "{findings:?}"
            );
            let root = std::env::var_os("JPEG_POLICY_ARTIFACTS")
                .expect("set JPEG_POLICY_ARTIFACTS to retain diagnostic traces");
            let dir = std::path::PathBuf::from(root)
                .join(format!("request-abort-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let mut json = String::new();
            trace.write_json(&mut json).unwrap();
            std::fs::write(
                dir.join(format!("progressive-{progressive}-at-{at}.json")),
                json,
            )
            .unwrap();
            let _released = tokio::time::timeout(Duration::from_secs(10), slots.acquire())
                .await
                .unwrap()
                .unwrap();
        }
    }
}
