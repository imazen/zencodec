//! Native success cannot hide a stop raised before returning to the wrapper.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use zencodec::{
    ImageInfo, Pulse,
    decode::*,
    enough::{Stop, StopReason},
    how_far::StopOnly,
};
use zencodec_io::CollectSink;
use zenpixels::PixelDescriptor;

struct Once {
    pending: Arc<AtomicBool>,
    reason: StopReason,
}
impl Stop for Once {
    fn check(&self) -> Result<(), StopReason> {
        if self.pending.swap(false, Ordering::SeqCst) {
            Err(self.reason)
        } else {
            Ok(())
        }
    }
}
struct Native {
    pending: Arc<AtomicBool>,
    boundary: DecodeBoundary,
    adopted: bool,
    stop_in_resume: bool,
    fail: bool,
    calls: usize,
}
impl Native {
    fn complete(&mut self) -> Result<Progress, StepError<core::fmt::Error>> {
        self.calls += 1;
        self.pending.store(true, Ordering::SeqCst);
        if self.fail {
            return Err(StepError::Codec(core::fmt::Error));
        }
        Ok(Progress::new(
            0,
            Event::Done(OutputInfo::full_decode(1, 1, PixelDescriptor::RGB8_SRGB)),
        ))
    }
}
impl IncrementalDecode for Native {
    type Error = core::fmt::Error;
    fn step(
        &mut self,
        _: Input<'_>,
        _: &mut dyn DecodeRowSink,
        _: Option<&dyn Pulse>,
    ) -> Result<Progress, StepError<Self::Error>> {
        self.complete()
    }
    fn requested(&self) -> &[InputRequest] {
        &[]
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
    fn take_details(&mut self) -> Option<DecodeDetails> {
        panic!("failed fused decoder must not delegate completion details")
    }
}
impl ControlledIncrementalDecode for Native {
    fn step_controlled(
        &mut self,
        _: Input<'_>,
        _: &mut dyn DecodeRowSink,
        _: Option<&dyn Pulse>,
    ) -> Result<ControlledProgress, StepError<Self::Error>> {
        if self.adopted {
            self.complete().map(Into::into)
        } else {
            Ok(ControlledProgress::Suspended {
                consumed: 0,
                boundary: self.boundary,
            })
        }
    }
    fn resume(
        &mut self,
        _: BoundaryId,
        _: BoundaryAction,
        _: Option<&dyn Pulse>,
    ) -> Result<(), StepError<Self::Error>> {
        self.calls += 1;
        if self.stop_in_resume {
            self.pending.store(true, Ordering::SeqCst);
            if self.fail {
                return Err(StepError::Codec(core::fmt::Error));
            }
        }
        self.adopted = true;
        Ok(())
    }
}
fn setup(
    reason: StopReason,
    stop_in_resume: bool,
    fail: bool,
) -> (Fused<Native>, StopOnly<'static>) {
    let pending = Arc::new(AtomicBool::new(false));
    let boundary = DecodeBoundary::new(InterpretationFacts::new(
        1,
        1,
        Some(PixelDescriptor::RGB8_SRGB),
        InterpretationState::Final,
    ))
    .unwrap();
    (
        Fused::new(Native {
            pending: pending.clone(),
            boundary,
            adopted: false,
            stop_in_resume,
            fail,
            calls: 0,
        }),
        StopOnly::new(Once { pending, reason }),
    )
}
fn expect_failure<T: core::fmt::Debug>(
    result: Result<T, StepError<core::fmt::Error>>,
    reason: StopReason,
    native_error: bool,
) {
    if native_error {
        assert!(matches!(result, Err(StepError::Codec(_))), "{result:?}");
    } else {
        assert!(
            matches!(result, Err(StepError::Cancelled(actual)) if actual == reason),
            "{result:?}"
        );
    }
}
fn terminal(decoder: &mut Fused<Native>, controlled: bool) {
    let calls = decoder.get_ref().calls;
    let mut sink = CollectSink::new();
    if controlled {
        assert!(matches!(
            decoder.step_controlled(Input::empty(), &mut sink, None),
            Err(StepError::Finished)
        ));
    } else {
        assert!(matches!(
            decoder.step(Input::empty(), &mut sink, None),
            Err(StepError::Finished)
        ));
    }
    assert_eq!(decoder.get_ref().calls, calls);
    assert!(decoder.take_details().is_none());
}
#[test]
fn raw_completion_observes_post_call_stop_and_preserves_native_errors() {
    for reason in [StopReason::Cancelled, StopReason::TimedOut] {
        for fail in [false, true] {
            let (mut decoder, pulse) = setup(reason, false, fail);
            expect_failure(
                decoder.step(Input::empty(), &mut CollectSink::new(), Some(&pulse)),
                reason,
                fail,
            );
            terminal(&mut decoder, false);
        }
    }
}
#[test]
fn controlled_completion_observes_post_call_stop_and_preserves_native_errors() {
    for reason in [StopReason::Cancelled, StopReason::TimedOut] {
        for fail in [false, true] {
            let (mut decoder, pulse) = setup(reason, false, fail);
            let mut sink = CollectSink::new();
            let ControlledProgress::Suspended { boundary, .. } = decoder
                .step_controlled(Input::empty(), &mut sink, None)
                .unwrap()
            else {
                panic!("boundary");
            };
            decoder
                .resume(
                    boundary.id(),
                    BoundaryAction::AdoptInterpretation,
                    Some(&pulse),
                )
                .unwrap();
            expect_failure(
                decoder.step_controlled(Input::empty(), &mut sink, Some(&pulse)),
                reason,
                fail,
            );
            terminal(&mut decoder, true);
        }
    }
}
#[test]
fn controlled_resume_observes_post_call_stop_and_preserves_native_errors() {
    for reason in [StopReason::Cancelled, StopReason::TimedOut] {
        for fail in [false, true] {
            let (mut decoder, pulse) = setup(reason, true, fail);
            let ControlledProgress::Suspended { boundary, .. } = decoder
                .step_controlled(Input::empty(), &mut CollectSink::new(), None)
                .unwrap()
            else {
                panic!("boundary");
            };
            expect_failure(
                decoder.resume(
                    boundary.id(),
                    BoundaryAction::AdoptInterpretation,
                    Some(&pulse),
                ),
                reason,
                fail,
            );
            terminal(&mut decoder, true);
        }
    }
}
