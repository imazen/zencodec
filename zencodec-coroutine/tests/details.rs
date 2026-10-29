//! Terminal ownership, not a native metadata-completeness certificate.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec::decode::*;
use zencodec::{ImageFormat, ImageInfo};
use zencodec_coroutine::{CoroutineDecoder, CoroutineError};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};
const DESC: PixelDescriptor = PixelDescriptor::GRAY8_SRGB;
struct Owner(Arc<AtomicUsize>);
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn details(drops: Arc<AtomicUsize>) -> DecodeDetails {
    let mut details =
        DecodeDetails::new(ImageInfo::new(1, 1, ImageFormat::Png).with_xmp(vec![7, 8]));
    details.extensions.insert(Owner(drops));
    details
}
#[derive(Default)]
struct Sink {
    bytes: [u8; 1],
    finish_error: bool,
    finish_panic: bool,
    finished: bool,
}
impl DecodeRowSink for Sink {
    fn provide_next_buffer(
        &mut self,
        _: u32,
        rows: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        Ok(PixelSliceMut::new(
            &mut self.bytes,
            width,
            rows,
            1,
            descriptor,
        )?)
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        assert!(!self.finish_panic, "sink finish panic");
        if self.finish_error {
            return Err("sink finish failure".into());
        }
        self.finished = true;
        Ok(())
    }
}
fn producer(
    drops: Arc<AtomicUsize>,
    wrong_geometry: bool,
) -> impl IncrementalDecode<Error = CoroutineError> {
    CoroutineDecoder::new(move |mut ctx| async move {
        ctx.set_info(ImageInfo::new(1, 1, ImageFormat::Png));
        ctx.set_output_descriptor(DESC);
        let mut result = details(drops);
        if wrong_geometry {
            result.info.width = 2;
        }
        ctx.set_details(result);
        let rows = PixelSlice::new(&[17], 1, 1, 1, DESC).unwrap();
        ctx.write_rows(BatchMeta::final_rows(0), &rows).await?;
        Ok::<_, CoroutineError>(OutputInfo::full_decode(1, 1, DESC))
    })
}
fn finish<D: IncrementalDecode<Error = CoroutineError>>(
    d: &mut D,
    s: &mut Sink,
) -> Result<(), StepError<CoroutineError>> {
    for _ in 0..8 {
        if matches!(d.step(Input::empty(), s, None)?.event, Event::Done(_)) {
            return Ok(());
        }
    }
    panic!("producer did not terminate");
}
#[test]
fn successful_completion_moves_final_details_once() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut decoder = producer(drops.clone(), false);
    let mut sink = Sink::default();
    assert!(decoder.take_details().is_none());
    finish(&mut decoder, &mut sink).unwrap();
    assert!(sink.finished);
    assert_eq!(sink.bytes, [17]);
    assert_eq!(
        decoder.info().unwrap().metadata().xmp.as_deref(),
        Some(&[7, 8][..])
    );
    let result = decoder.take_details().unwrap();
    assert!(decoder.take_details().is_none());
    assert!(decoder.info().is_none());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(result);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
#[test]
fn sink_error_panic_and_inconsistent_geometry_release_pending_details() {
    for mode in 0..3 {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut decoder = producer(drops.clone(), mode == 2);
        let mut sink = Sink {
            finish_error: mode == 0,
            finish_panic: mode == 1,
            ..Sink::default()
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            finish(&mut decoder, &mut sink)
        }));
        if mode == 1 {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_err());
        }
        assert!(decoder.take_details().is_none());
        assert!(!sink.finished);
        assert_eq!(drops.load(Ordering::SeqCst), 1, "decoder still alive");
        assert!(matches!(
            decoder.step(Input::empty(), &mut sink, None),
            Err(StepError::Finished)
        ));
    }
}
struct Cancel;
impl enough::Stop for Cancel {
    fn check(&self) -> Result<(), enough::StopReason> {
        Err(enough::StopReason::Cancelled)
    }
}
#[test]
fn cancellation_while_waiting_releases_pending_details() {
    let drops = Arc::new(AtomicUsize::new(0));
    let owned = drops.clone();
    let mut decoder = CoroutineDecoder::new(move |mut ctx| async move {
        ctx.set_details(details(owned));
        let mut byte = [0];
        ctx.read_exact_at(0, &mut byte).await?;
        Ok::<_, CoroutineError>(OutputInfo::full_decode(1, 1, DESC))
    });
    let mut sink = Sink::default();
    assert!(matches!(
        decoder.step(Input::empty(), &mut sink, None).unwrap().event,
        Event::Need
    ));
    let pulse = zencodec::how_far::StopOnly::borrowed(&Cancel);
    assert!(matches!(
        decoder.step(Input::empty(), &mut sink, Some(&pulse)),
        Err(StepError::Cancelled(_))
    ));
    assert!(decoder.take_details().is_none());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn future_error_and_unwind_release_pending_details() {
    for panic in [false, true] {
        let drops = Arc::new(AtomicUsize::new(0));
        let owned = drops.clone();
        let mut decoder = CoroutineDecoder::new(move |mut ctx| async move {
            ctx.set_details(details(owned));
            assert!(!panic, "producer panic");
            Err::<OutputInfo, _>(CoroutineError::Stalled)
        });
        let mut sink = Sink::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            decoder.step(Input::empty(), &mut sink, None)
        }));
        if panic {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_err());
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(decoder.take_details().is_none());
        assert!(matches!(
            decoder.step(Input::empty(), &mut sink, None),
            Err(StepError::Finished)
        ));
    }
}

#[test]
fn cancelled_resume_discards_pending_details_without_adoption() {
    let drops = Arc::new(AtomicUsize::new(0));
    let owned = drops.clone();
    let mut decoder = CoroutineDecoder::controlled(
        16,
        zencodec::source::AccessPattern::Sequential,
        move |mut ctx| async move {
            ctx.set_info(ImageInfo::new(1, 1, ImageFormat::Png));
            ctx.set_output_descriptor(DESC);
            ctx.set_details(details(owned));
            ctx.interpretation(InterpretationFacts::new(
                1,
                1,
                Some(DESC),
                InterpretationState::Final,
            ))
            .await?;
            Ok::<_, CoroutineError>(OutputInfo::full_decode(1, 1, DESC))
        },
    );
    let mut sink = Sink::default();
    let ControlledProgress::Suspended { boundary, .. } = decoder
        .step_controlled(Input::empty(), &mut sink, None)
        .unwrap()
    else {
        panic!("expected boundary")
    };
    assert!(decoder.take_details().is_none());
    let pulse = zencodec::how_far::StopOnly::borrowed(&Cancel);
    assert!(matches!(
        decoder.resume(
            boundary.id(),
            BoundaryAction::AdoptInterpretation,
            Some(&pulse)
        ),
        Err(StepError::Cancelled(_))
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(decoder.take_details().is_none());
    assert!(!sink.finished);
}

#[test]
fn terminal_details_cannot_change_adopted_interpretation() {
    let drops = Arc::new(AtomicUsize::new(0));
    let owned = drops.clone();
    let mut decoder = CoroutineDecoder::controlled(
        16,
        zencodec::source::AccessPattern::Sequential,
        move |mut ctx| async move {
            ctx.set_info(ImageInfo::new(1, 1, ImageFormat::Png));
            ctx.set_output_descriptor(DESC);
            ctx.interpretation(InterpretationFacts::new(
                1,
                1,
                Some(DESC),
                InterpretationState::Final,
            ))
            .await?;
            let mut result = details(owned);
            result.info.width = 2;
            ctx.set_details(result);
            let rows = PixelSlice::new(&[17], 1, 1, 1, DESC).unwrap();
            ctx.write_rows(BatchMeta::final_rows(0), &rows).await?;
            Ok::<_, CoroutineError>(OutputInfo::full_decode(1, 1, DESC))
        },
    );
    let mut sink = Sink::default();
    let ControlledProgress::Suspended { boundary, .. } = decoder
        .step_controlled(Input::empty(), &mut sink, None)
        .unwrap()
    else {
        panic!("expected boundary")
    };
    decoder
        .resume(boundary.id(), BoundaryAction::AdoptInterpretation, None)
        .unwrap();
    assert!(
        decoder
            .step_controlled(Input::empty(), &mut sink, None)
            .is_err()
    );
    assert_eq!(sink.bytes, [0]);
    assert!(!sink.finished);
    assert!(decoder.take_details().is_none());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn sink_stop_prevents_completion_and_releases_details() {
    use std::sync::atomic::AtomicBool;
    use zencodec::enough::{Stop, StopReason};
    struct Once(Arc<AtomicBool>, StopReason);
    impl Stop for Once {
        fn check(&self) -> Result<(), StopReason> {
            if self.0.swap(false, Ordering::SeqCst) {
                Err(self.1)
            } else {
                Ok(())
            }
        }
    }
    struct StoppingSink {
        signal: Arc<AtomicBool>,
        at_begin: bool,
        fail: bool,
        finishes: usize,
    }
    impl DecodeRowSink for StoppingSink {
        fn provide_next_buffer(
            &mut self,
            _: u32,
            _: u32,
            _: u32,
            _: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            panic!("completion-only producer has no row callback");
        }
        fn begin(&mut self, _: u32, _: u32, _: PixelDescriptor) -> Result<(), SinkError> {
            if self.at_begin {
                self.signal.store(true, Ordering::SeqCst);
                if self.fail {
                    return Err("begin failure".into());
                }
            }
            Ok(())
        }
        fn finish(&mut self) -> Result<(), SinkError> {
            self.finishes += 1;
            self.signal.store(true, Ordering::SeqCst);
            if self.fail {
                return Err("finish failure".into());
            }
            Ok(())
        }
    }
    for reason in [StopReason::Cancelled, StopReason::TimedOut] {
        for at_begin in [true, false] {
            for fail in [false, true] {
                let drops = Arc::new(AtomicUsize::new(0));
                let owner = drops.clone();
                let mut decoder = CoroutineDecoder::new(move |mut ctx| async move {
                    ctx.set_info(ImageInfo::new(1, 1, ImageFormat::Png));
                    ctx.set_output_descriptor(DESC);
                    ctx.set_details(details(owner));
                    Ok::<_, CoroutineError>(OutputInfo::full_decode(1, 1, DESC))
                });
                let signal = Arc::new(AtomicBool::new(false));
                let pulse = zencodec::how_far::StopOnly::new(Once(signal.clone(), reason));
                let mut sink = StoppingSink {
                    signal,
                    at_begin,
                    fail,
                    finishes: 0,
                };
                let result = decoder.step(Input::empty(), &mut sink, Some(&pulse));
                if fail {
                    assert!(matches!(result, Err(StepError::Sink(_))), "{result:?}");
                } else {
                    assert!(
                        matches!(result, Err(StepError::Cancelled(r)) if r == reason),
                        "{result:?}"
                    );
                }
                assert_eq!(sink.finishes, usize::from(!at_begin));
                assert!(decoder.take_details().is_none());
                assert_eq!(drops.load(Ordering::SeqCst), 1);
                assert!(matches!(
                    decoder.step(Input::empty(), &mut sink, None),
                    Err(StepError::Finished)
                ));
                assert_eq!(sink.finishes, usize::from(!at_begin));
            }
        }
    }
}
