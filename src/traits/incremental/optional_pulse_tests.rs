//! Absence must survive erasure and must not cancel retained operation controls.
use super::*;
use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use how_far::{Child, Execution, PhaseSpec, PlanError, Pulse, Report, SharedPulse, Stop, Total};

#[derive(Default)]
struct Observed {
    stopped: AtomicBool,
    units: AtomicUsize,
    totals: AtomicUsize,
    splits: AtomicUsize,
    shares: AtomicUsize,
    native_calls: AtomicUsize,
    saw_none: AtomicBool,
}
#[derive(Clone)]
struct Meter(Arc<Observed>);
impl Stop for Meter {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.stopped.load(Ordering::SeqCst) {
            Err(StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}
impl Report for Meter {
    fn advance(&self, n: u64) {
        self.0.units.fetch_add(n as usize, Ordering::SeqCst);
    }
}
impl Pulse for Meter {
    fn set_total(&self, total: Total) -> Result<(), PlanError> {
        assert_eq!(total, Total::Exact(2));
        self.0.totals.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn split(
        &self,
        execution: Execution,
        parts: &[PhaseSpec<'_>],
    ) -> Result<Vec<Child<'_>>, PlanError> {
        self.0.splits.fetch_add(1, Ordering::SeqCst);
        crate::NoPulse.split(execution, parts)
    }
    fn share(&self) -> Result<SharedPulse, PlanError> {
        self.0.shares.fetch_add(1, Ordering::SeqCst);
        Ok(SharedPulse::new(self.clone()))
    }
}
struct Probe {
    retained: SharedPulse,
    observed: Arc<Observed>,
}
impl IncrementalDecode for Probe {
    type Error = core::fmt::Error;
    fn step(
        &mut self,
        _: Input<'_>,
        _: &mut dyn DecodeRowSink,
        pulse: Option<&dyn Pulse>,
    ) -> Result<Progress, StepError<Self::Error>> {
        self.retained.check()?;
        let control = pulse.unwrap_or(&self.retained);
        control.check()?;
        self.observed.native_calls.fetch_add(1, Ordering::SeqCst);
        self.observed
            .saw_none
            .store(pulse.is_none(), Ordering::SeqCst);
        control.set_total(Total::Exact(2)).unwrap();
        control.advance(1);
        control.share().unwrap().advance(1);
        // Deliberately invalid planning request: forwarding must preserve the
        // planner call, including its refusal, rather than erase it to Stop.
        assert!(control.split(Execution::Sequence, &[]).is_err());
        Ok(Progress::need(0))
    }
    fn requested(&self) -> &[InputRequest] {
        &[InputRequest::Len]
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
}
struct NoRows;
impl DecodeRowSink for NoRows {
    fn provide_next_buffer(
        &mut self,
        _: u32,
        _: u32,
        _: u32,
        _: zenpixels::PixelDescriptor,
    ) -> Result<zenpixels::PixelSliceMut<'_>, crate::SinkError> {
        panic!("this protocol-only probe never publishes rows")
    }
}
fn probe(observed: &Arc<Observed>) -> Probe {
    Probe {
        retained: SharedPulse::new(Meter(observed.clone())),
        observed: observed.clone(),
    }
}

#[test]
fn none_survives_fused_erasure_and_uses_retained_reporting() {
    let observed = Arc::new(Observed::default());
    let mut decoder = erase_incremental(Fused::new(probe(&observed)));
    decoder.step(Input::empty(), &mut NoRows, None).unwrap();
    assert!(observed.saw_none.load(Ordering::SeqCst));
    assert_eq!(observed.native_calls.load(Ordering::SeqCst), 1);
    assert_eq!(observed.units.load(Ordering::SeqCst), 2);
    assert_eq!(observed.totals.load(Ordering::SeqCst), 1);
    assert_eq!(observed.splits.load(Ordering::SeqCst), 1);
    assert_eq!(observed.shares.load(Ordering::SeqCst), 1);
}

#[test]
fn some_keeps_progress_planning_sharing_and_original_stop_reason() {
    let retained = Arc::new(Observed::default());
    let observed = Arc::new(Observed::default());
    let pulse = Meter(observed.clone());
    let mut decoder = erase_incremental(Fused::new(probe(&retained)));
    decoder
        .step(Input::empty(), &mut NoRows, Some(&pulse))
        .unwrap();
    assert!(!retained.saw_none.load(Ordering::SeqCst));
    assert_eq!(observed.units.load(Ordering::SeqCst), 2);
    assert_eq!(observed.totals.load(Ordering::SeqCst), 1);
    assert_eq!(observed.splits.load(Ordering::SeqCst), 1);
    assert_eq!(observed.shares.load(Ordering::SeqCst), 1);
    assert_eq!(retained.units.load(Ordering::SeqCst), 0);
    observed.stopped.store(true, Ordering::SeqCst);
    assert!(matches!(
        decoder.step(Input::empty(), &mut NoRows, Some(&pulse)),
        Err(StepError::Cancelled(StopReason::TimedOut))
    ));
    assert_eq!(retained.native_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        decoder.step(Input::empty(), &mut NoRows, None),
        Err(StepError::Finished)
    ));
}

#[test]
fn none_and_some_noop_cannot_disable_a_retained_stop() {
    for extra in [None, Some(&crate::NoPulse as &dyn Pulse)] {
        let observed = Arc::new(Observed::default());
        observed.stopped.store(true, Ordering::SeqCst);
        let mut decoder = erase_incremental(Fused::new(probe(&observed)));
        assert!(matches!(
            decoder.step(Input::empty(), &mut NoRows, extra),
            Err(StepError::Cancelled(StopReason::TimedOut))
        ));
        assert_eq!(observed.native_calls.load(Ordering::SeqCst), 0);
        assert_eq!(observed.units.load(Ordering::SeqCst), 0);
    }
}
