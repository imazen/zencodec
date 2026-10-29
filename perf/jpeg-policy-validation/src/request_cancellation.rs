//! Native checkpoints, not elapsed-time guesses, synchronize request abort.
include!("../../../docs/examples/request_cancellation.rs");
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
            let input = bytes.clone();
            let request = tokio::spawn(cancel_on_request_drop(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let _permit = permit;
                        let result =
                            zenjpeg::decoder::Decoder::new().decode_with_pulse(&input, Some(&gate));
                        // The independent receipt survives request abort. A worker
                        // panic closes the channel, which fails the control.
                        let stopped = matches!(
                            result.as_ref().map_err(|e| e.kind()),
                            Err(zenjpeg::decoder::ErrorKind::Cancelled(
                                StopReason::Cancelled
                            ))
                        );
                        result_tx.send(stopped).unwrap();
                        result.map(|_| ())
                    })
                    .await
                    .unwrap()
                },
                move || {
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
            assert!(
                tokio::time::timeout(Duration::from_secs(10), result_rx)
                    .await
                    .unwrap()
                    .unwrap(),
                "progressive={progressive}, checkpoint={at}/{checks}"
            );
            let _released = tokio::time::timeout(Duration::from_secs(10), slots.acquire())
                .await
                .unwrap()
                .unwrap();
        }
    }
}
