//! Adversarial controls for actual suspension, output ordering and driver reuse.
//! This authored byte format tests the protocol; native codec qualification is separate.

use enough::Unstoppable;
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zencodec::decode::*;
use zencodec::operation::{Decision, DecisionCode, Decline, PolicySet, Question};
use zencodec::source::{AccessPattern, SliceReadAt};
use zencodec::storage::{StorageBudget, StorageError};
use zencodec::{At, ImageFormat, ImageInfo};
use zencodec_coroutine::{CoroutineDecoder, CoroutineError};
use zencodec_io::{CollectSink, DriveOptions, PolicyDecoder, drive_from_read_at, drive_from_slice};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

const RGBA: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;

#[test]
fn retained_control_stepper_releases_native_storage_after_invalid_reply_stop_or_unwind() {
    use zencodec_io::{ByteStepper, ControlledEvent, ControlledStepper, SliceCursor, Yield};
    struct Stop(bool);
    impl enough::Stop for Stop {
        fn check(&self) -> Result<(), enough::StopReason> {
            assert!(!self.0, "stop hook unwind");
            Err(enough::StopReason::Cancelled)
        }
    }
    for failure in 0..3 {
        let budget = StorageBudget::new(4096, 3);
        let owner = budget.clone();
        let native = CoroutineDecoder::controlled_with_storage(
            32,
            AccessPattern::RandomAccess,
            budget.clone(),
            move |mut ctx| async move {
                let retained = owner.bytes(8)?;
                ctx.set_info(ImageInfo::new(2, 2, ImageFormat::Qoi));
                ctx.set_output_descriptor(RGBA);
                ctx.interpretation(InterpretationFacts::new(
                    2,
                    2,
                    Some(RGBA),
                    InterpretationState::Final,
                ))
                .await?;
                core::hint::black_box(retained.as_slice());
                Ok::<_, CoroutineError>(OutputInfo::full_decode(2, 2, RGBA))
            },
        )
        .unwrap();
        let mut sink = Sink::default();
        let mut stage = ControlledStepper::new(native, &mut sink);
        let mut cursor = SliceCursor::new(1);
        let Yield::Output(ControlledEvent::Suspended(boundary)) =
            cursor.next(&[], &mut stage, &Unstoppable).unwrap()
        else {
            panic!("expected final boundary")
        };
        assert_eq!(budget.reserved_objects(), 3);
        if failure == 0 {
            let stale = DecodeBoundary::new(boundary.facts()).unwrap();
            assert!(matches!(
                stage.resume(
                    stale.id(),
                    BoundaryAction::AdoptInterpretation,
                    &Unstoppable
                ),
                Err(StepError::Control(ControlError::InvalidReply))
            ));
        } else if failure == 1 {
            assert!(matches!(
                stage.resume(
                    boundary.id(),
                    BoundaryAction::AdoptInterpretation,
                    &Stop(false)
                ),
                Err(StepError::Cancelled(enough::StopReason::Cancelled))
            ));
        } else {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    stage.resume(
                        boundary.id(),
                        BoundaryAction::AdoptInterpretation,
                        &Stop(true),
                    )
                }))
                .is_err()
            );
        }
        assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
        assert!(stage.info().is_none());
        assert!(stage.requested().is_empty());
        assert!(matches!(
            stage.step(Input::new(0, &[]), &Unstoppable),
            Err(StepError::Finished)
        ));
        drop(stage);
        assert_eq!((sink.began, sink.rows, sink.ended), (0, 0, 0));
    }
}

#[test]
fn cursors_hold_real_boundaries_while_the_adapter_prepares_other_stages() {
    use zencodec_io::{ControlledEvent, ControlledStepper, ReadAtCursor, SliceCursor, Yield};
    for positioned in [false, true] {
        let work = Arc::new(AtomicUsize::new(0));
        let mut sink = Sink::default();
        let input = bytes();
        let mut source = SliceReadAt::new(&input);
        let mut slice = SliceCursor::new(1);
        let mut read = ReadAtCursor::new(DriveOptions::new(0, 3));
        let mut stage = ControlledStepper::new(decoder(work.clone(), false, false), &mut sink);
        let mut boundaries = 0;
        loop {
            let output = if positioned {
                read.next(&mut source, &mut stage, &Unstoppable).unwrap()
            } else {
                slice.next(&input, &mut stage, &Unstoppable).unwrap()
            };
            match output {
                Yield::Output(ControlledEvent::Suspended(boundary)) => {
                    assert_eq!(work.load(Ordering::SeqCst), 0);
                    assert_eq!(
                        (stage.sink().began, stage.sink().rows, stage.sink().ended),
                        (0, 0, 0)
                    );
                    assert_eq!(stage.info().unwrap().width, 2);
                    let before = if positioned {
                        read.stats()
                    } else {
                        slice.stats()
                    };
                    let repeated = if positioned {
                        read.next(&mut source, &mut stage, &Unstoppable).unwrap()
                    } else {
                        slice.next(&input, &mut stage, &Unstoppable).unwrap()
                    };
                    assert!(matches!(repeated,
                        Yield::Output(ControlledEvent::Suspended(again)) if again == boundary));
                    let after = if positioned {
                        read.stats()
                    } else {
                        slice.stats()
                    };
                    assert_eq!(
                        (before.reads, before.bytes_read),
                        (after.reads, after.bytes_read)
                    );
                    assert_eq!(work.load(Ordering::SeqCst), 0);
                    let action = if boundaries == 0 {
                        assert_eq!(boundary.facts().state, InterpretationState::Provisional);
                        BoundaryAction::ContinueReading
                    } else {
                        assert_eq!(stage.output_descriptor(), Some(RGBA));
                        assert_eq!(boundary.facts().state, InterpretationState::Final);
                        BoundaryAction::AdoptInterpretation
                    };
                    boundaries += 1;
                    stage.resume(boundary.id(), action, &Unstoppable).unwrap();
                    assert_eq!(
                        work.load(Ordering::SeqCst),
                        0,
                        "resume itself does no pixel work"
                    );
                }
                Yield::Finished(ControlledEvent::Decode(Event::Done(info))) => {
                    assert_eq!((info.width, info.height), (2, 2));
                    break;
                }
                Yield::Output(ControlledEvent::Decode(_)) => {}
                _ => panic!("invalid completion or boundary kind"),
            }
        }
        drop(stage);
        assert_eq!(boundaries, 2);
        assert_eq!(work.load(Ordering::SeqCst), 1);
        assert_eq!(sink.pixels.bytes(), &(1..=16).collect::<Vec<u8>>());
        assert_eq!((sink.began, sink.rows, sink.ended), (1, 1, 1));
    }
}

#[test]
fn storage_admission_precedes_initialization_and_row_publication() {
    let initialized = Arc::new(AtomicUsize::new(0));
    let seen = initialized.clone();
    let budget = StorageBudget::new(4096, 1); // payload fits, index object does not
    let result = CoroutineDecoder::controlled_with_storage(
        32,
        AccessPattern::RandomAccess,
        budget.clone(),
        move |_ctx| {
            seen.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, CoroutineError>(OutputInfo::full_decode(2, 2, RGBA)) }
        },
    );
    assert!(matches!(
        result,
        Err(CoroutineError::Storage(StorageError::Objects { limit: 1 }))
    ));
    assert_eq!(initialized.load(Ordering::SeqCst), 0);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));

    for objects in [2, 3] {
        let budget = StorageBudget::new(4096, objects);
        let native = CoroutineDecoder::controlled_with_storage(
            32,
            AccessPattern::RandomAccess,
            budget.clone(),
            |mut ctx| async move {
                ctx.set_info(ImageInfo::new(2, 2, ImageFormat::Qoi));
                ctx.set_output_descriptor(RGBA);
                ctx.interpretation(InterpretationFacts::new(
                    2,
                    2,
                    Some(RGBA),
                    InterpretationState::Final,
                ))
                .await?;
                let mut pixels = [0; 24];
                ctx.read_exact_at(0, &mut pixels).await?;
                ctx.write_rows(
                    BatchMeta::final_rows(0),
                    &PixelSlice::new(&pixels, 2, 2, 12, RGBA).unwrap(),
                )
                .await?;
                Ok::<_, CoroutineError>(OutputInfo::full_decode(2, 2, RGBA))
            },
        )
        .unwrap();
        assert_eq!(budget.reserved_objects(), 2);
        let mut decoder = PolicyDecoder::new(native, PolicySet::new());
        let mut sink = Sink::default();
        let input: Vec<u8> = (0..24).collect();
        let result = drive_from_slice(&mut decoder, &input, &mut sink, None, 1);
        if objects == 2 {
            let error = result.unwrap_err();
            assert!(zencodec::find_cause::<StorageError>(&error).is_some());
            assert_eq!((sink.began, sink.rows, sink.ended), (0, 0, 0));
        } else {
            result.unwrap();
            assert_eq!(sink.pixels.bytes(), &[&input[..8], &input[12..20]].concat());
            assert_eq!((sink.began, sink.rows, sink.ended), (1, 1, 1));
        }
        assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    }
}

#[derive(Default)]
struct Sink {
    pixels: CollectSink,
    began: usize,
    rows: usize,
    ended: usize,
}
impl DecodeRowSink for Sink {
    fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
        self.began += 1;
        self.pixels.begin(w, h, d)
    }
    fn provide_next_buffer(
        &mut self,
        y: u32,
        h: u32,
        w: u32,
        d: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.rows += 1;
        self.pixels.provide_next_buffer(y, h, w, d)
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        self.ended += 1;
        self.pixels.finish()
    }
}
fn bytes() -> Vec<u8> {
    vec![
        0, 1, 2, 3, 4, 5, 6, 7, 8, 0xa5, 0xa5, 0xa5, 0xa5, 9, 10, 11, 12, 13, 14, 15, 16, 0xa5,
        0xa5, 0xa5, 0xa5, 7,
    ]
}

fn decoder(work: Arc<AtomicUsize>, early: bool, change_final: bool) -> Box<DynControlledDecoder> {
    erase_controlled(CoroutineDecoder::controlled(
        32,
        AccessPattern::RandomAccess,
        move |mut ctx| async move {
            let mut magic = [0];
            ctx.read_exact_at(0, &mut magic).await?;
            ctx.set_info(ImageInfo::new(2, 2, ImageFormat::Qoi));
            ctx.announce(MetadataUpdate::HeaderReady).await?;
            let action = ctx
                .interpretation(InterpretationFacts::new(
                    2,
                    2,
                    None,
                    InterpretationState::Provisional,
                ))
                .await?;
            assert_eq!(action, BoundaryAction::ContinueReading);
            if early {
                ctx.write_rows(
                    BatchMeta::final_rows(0),
                    &PixelSlice::new(&[0; 16], 2, 2, 8, RGBA).unwrap(),
                )
                .await?;
            }
            let mut late = [0];
            ctx.read_exact_at(25, &mut late).await?;
            assert_eq!(late, [7]);
            ctx.set_output_descriptor(RGBA);
            ctx.interpretation(InterpretationFacts::new(
                2,
                2,
                Some(RGBA),
                InterpretationState::Final,
            ))
            .await?;
            work.fetch_add(1, Ordering::SeqCst);
            if change_final {
                ctx.set_info(
                    ImageInfo::new(2, 2, ImageFormat::Qoi)
                        .with_cicp(zencodec::Cicp::new(1, 8, 0, true)),
                );
            }
            let mut rows = [0; 24];
            ctx.read_exact_at(1, &mut rows).await?;
            ctx.write_rows(
                BatchMeta::final_rows(0),
                &PixelSlice::new(&rows, 2, 2, 12, RGBA).unwrap(),
            )
            .await?;
            Ok::<_, CoroutineError>(OutputInfo::full_decode(2, 2, RGBA))
        },
    ))
}

#[test]
fn same_policy_drives_late_facts_over_slices_and_random_access() {
    for window in [1, 3, 64] {
        let work = Arc::new(AtomicUsize::new(0));
        let mut decisions = 0;
        let mut resolver = |question: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Boundary { facts, choices } = question else {
                panic!("wrong domain")
            };
            assert_eq!(
                work.load(Ordering::SeqCst),
                0,
                "no affected work before final adoption"
            );
            assert_eq!(choices.len(), 1);
            assert_eq!(
                facts.state,
                if decisions == 0 {
                    InterpretationState::Provisional
                } else {
                    InterpretationState::Final
                }
            );
            decisions += 1;
            Ok(Decision::UsePreset)
        };
        let mut dec = PolicyDecoder::new(decoder(work.clone(), false, false), &mut resolver);
        let mut sink = Sink::default();
        drive_from_slice(&mut dec, &bytes(), &mut sink, None, window).unwrap();
        assert_eq!(sink.pixels.bytes(), &(1..=16).collect::<Vec<u8>>());
        assert_eq!((sink.began, sink.rows, sink.ended), (1, 1, 1));
        assert_eq!(dec.report().decisions().count(), 2);
        assert!(
            dec.report()
                .decisions()
                .all(|r| r.used_preset && r.code == DecisionCode::Selected)
        );
        assert_eq!(work.load(Ordering::SeqCst), 1);
        drop(dec);
        assert_eq!(decisions, 2);
    }
    let work = Arc::new(AtomicUsize::new(0));
    let mut dec = PolicyDecoder::new(decoder(work, false, false), PolicySet::new());
    let input = bytes();
    let mut source = SliceReadAt::new(&input);
    let mut sink = Sink::default();
    drive_from_read_at(
        &mut dec,
        &mut source,
        &mut sink,
        None,
        DriveOptions::new(0, 3),
    )
    .unwrap();
    assert_eq!(sink.pixels.bytes(), &(1..=16).collect::<Vec<u8>>());
}

fn suspend(dec: &mut DynControlledDecoder, sink: &mut Sink) -> DecodeBoundary {
    let p = dec
        .step_controlled(Input::new(0, &[0]), sink, &Unstoppable)
        .unwrap();
    assert!(matches!(
        p,
        ControlledProgress::Decode(Progress {
            event: Event::Metadata(_),
            ..
        })
    ));
    let ControlledProgress::Suspended { boundary, consumed } = dec
        .step_controlled(Input::empty(), sink, &Unstoppable)
        .unwrap()
    else {
        panic!("not suspended")
    };
    assert_eq!(consumed, 0);
    boundary
}

#[test]
fn suspension_requires_its_exact_reply_and_does_not_consume_more_input() {
    let work = Arc::new(AtomicUsize::new(0));
    let mut dec = decoder(work.clone(), false, false);
    let mut sink = Sink::default();
    let first = suspend(&mut *dec, &mut sink);
    for _ in 0..4 {
        let p = dec
            .step_controlled(Input::new(100, &[255; 16]), &mut sink, &Unstoppable)
            .unwrap();
        assert!(
            matches!(p, ControlledProgress::Suspended { consumed: 0, boundary } if boundary == first)
        );
    }
    assert_eq!(
        (sink.began, sink.rows, work.load(Ordering::SeqCst)),
        (0, 0, 0)
    );
    let other = DecodeBoundary::new(first.facts()).unwrap();
    assert!(matches!(
        dec.resume(other.id(), BoundaryAction::ContinueReading, &Unstoppable),
        Err(StepError::Control(ControlError::InvalidReply))
    ));
    assert!(matches!(
        dec.step_controlled(Input::empty(), &mut sink, &Unstoppable),
        Err(StepError::Finished)
    ));
}

struct Stopped;
impl enough::Stop for Stopped {
    fn check(&self) -> Result<(), enough::StopReason> {
        Err(enough::StopReason::Cancelled)
    }
}

#[test]
fn cancellation_and_replayed_replies_cannot_resume_a_later_boundary() {
    for resume in [false, true] {
        let mut dec = decoder(Arc::new(AtomicUsize::new(0)), false, false);
        let mut sink = Sink::default();
        let first = suspend(&mut *dec, &mut sink);
        let cancelled = if resume {
            dec.resume(first.id(), BoundaryAction::ContinueReading, &Stopped)
                .map(|_| ())
        } else {
            dec.step_controlled(Input::empty(), &mut sink, &Stopped)
                .map(|_| ())
        };
        assert!(matches!(
            cancelled,
            Err(StepError::Cancelled(enough::StopReason::Cancelled))
        ));
        assert!(matches!(
            dec.step_controlled(Input::empty(), &mut sink, &Unstoppable),
            Err(StepError::Finished)
        ));
        assert_eq!(sink.began, 0);
    }
    let mut dec = decoder(Arc::new(AtomicUsize::new(0)), false, false);
    let mut sink = Sink::default();
    let first = suspend(&mut *dec, &mut sink);
    dec.resume(first.id(), BoundaryAction::ContinueReading, &Unstoppable)
        .unwrap();
    let next = dec
        .step_controlled(Input::new(25, &[7]), &mut sink, &Unstoppable)
        .unwrap();
    let ControlledProgress::Suspended {
        boundary,
        consumed: 1,
    } = next
    else {
        panic!("missing final boundary")
    };
    assert_ne!(first.id(), boundary.id());
    assert!(matches!(
        dec.resume(
            first.id(),
            BoundaryAction::AdoptInterpretation,
            &Unstoppable
        ),
        Err(StepError::Control(ControlError::InvalidReply))
    ));
    assert_eq!(sink.began, 0);
}

#[test]
fn final_callback_cancellation_does_not_start_native_work() {
    use std::sync::atomic::AtomicBool;
    struct Cancel(Arc<AtomicBool>);
    impl enough::Stop for Cancel {
        fn check(&self) -> Result<(), enough::StopReason> {
            if self.0.load(Ordering::SeqCst) {
                Err(enough::StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop = Cancel(cancelled.clone());
    let work = Arc::new(AtomicUsize::new(0));
    let mut resolver = move |question: Question<'_>| -> Result<Decision, At<Infallible>> {
        if let Question::Boundary { facts, .. } = question
            && facts.state == InterpretationState::Final
        {
            cancelled.store(true, Ordering::SeqCst);
        }
        Ok(Decision::UsePreset)
    };
    let mut dec = PolicyDecoder::new(decoder(work.clone(), false, false), &mut resolver);
    let mut sink = Sink::default();
    let pulse = zencodec::how_far::StopOnly::borrowed(&stop);
    let error = drive_from_slice(&mut dec, &bytes(), &mut sink, Some(&pulse), 64).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<zencodec::operation::PolicyFailure>(&error)
            .unwrap()
            .stop_reason(),
        Some(enough::StopReason::Cancelled)
    );
    assert_eq!(work.load(Ordering::SeqCst), 0);
    assert_eq!((sink.began, sink.rows, sink.ended), (0, 0, 0));
}

#[test]
fn legacy_drivers_and_early_output_refuse_before_sink_effects() {
    let work = Arc::new(AtomicUsize::new(0));
    let mut dec = decoder(work.clone(), false, false);
    let mut sink = Sink::default();
    assert!(drive_from_slice(&mut dec, &bytes(), &mut sink, None, 64).is_err());
    assert_eq!((sink.began, sink.rows, sink.ended), (0, 0, 0));
    for (early, changed) in [(true, false), (false, true)] {
        let mut dec = PolicyDecoder::new(decoder(work.clone(), early, changed), PolicySet::new());
        let error = drive_from_slice(&mut dec, &bytes(), &mut sink, None, 64).unwrap_err();
        assert!(zencodec::find_cause::<ControlError>(&error).is_some());
        assert_eq!((sink.began, sink.rows, sink.ended), (0, 0, 0));
        assert!(matches!(
            dec.step(Input::empty(), &mut sink, &Unstoppable),
            Err(StepError::Finished)
        ));
    }
}

#[derive(Debug)]
struct AppError;
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private-policy-payload")
    }
}
impl std::error::Error for AppError {}

#[test]
fn traced_refusal_failure_and_unwind_leave_native_work_unstarted() {
    let work = Arc::new(AtomicUsize::new(0));
    let mut sink = Sink::default();
    let mut dec = PolicyDecoder::new(
        decoder(work.clone(), false, false),
        |_: Question<'_>| -> Result<Decision, At<AppError>> {
            Err(At::wrap(AppError).at_named("tenant decision"))
        },
    );
    let error = drive_from_slice(&mut dec, &bytes(), &mut sink, None, 64).unwrap_err();
    let original = zencodec::find_cause::<At<AppError>>(&error).unwrap();
    assert!(
        original
            .full_trace()
            .to_string()
            .contains("tenant decision")
    );
    assert!(!format!("{error} {error:?}").contains("private-policy-payload"));
    assert_eq!(
        dec.report().decisions().next().unwrap().code,
        DecisionCode::CallbackFailed
    );
    assert!(matches!(
        dec.step(Input::empty(), &mut sink, &Unstoppable),
        Err(StepError::Finished)
    ));
    let mut dec = PolicyDecoder::new(
        decoder(work.clone(), false, false),
        |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            Ok(Decision::decline(Decline::ApplicationRule))
        },
    );
    assert!(drive_from_slice(&mut dec, &bytes(), &mut sink, None, 64).is_err());
    assert_eq!(
        dec.report().decisions().next().unwrap().code,
        DecisionCode::Declined
    );
    let mut dec = PolicyDecoder::new(
        decoder(work.clone(), false, false),
        |_: Question<'_>| -> Result<Decision, At<Infallible>> { panic!("application panic") },
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drive_from_slice(
            &mut dec,
            &bytes(),
            &mut sink,
            None,
            64
        )))
        .is_err()
    );
    assert!(matches!(
        dec.step(Input::empty(), &mut sink, &Unstoppable),
        Err(StepError::Finished)
    ));
    assert_eq!(work.load(Ordering::SeqCst), 0);
    assert_eq!((sink.began, sink.rows, sink.ended), (0, 0, 0));
}
