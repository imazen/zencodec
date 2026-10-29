//! Native qualification of the shared controlled protocol. This adapter is
//! deliberately whole-source: its input owner is admitted, but native workspace
//! and coroutine row copies are outside that allowance.

use crate::fixtures;
use std::{
    borrow::Cow,
    convert::Infallible,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zencodec::{
    At, CodecError, OrientationHint,
    decode::*,
    operation::{Decision, Decline, PolicySet, Question},
    source::{AccessPattern, SliceReadAt},
};
use zencodec_coroutine::{CoroutineDecoder, CoroutineError};
use zencodec_io::{
    CollectSink, DriveOptions, PolicyDecoder, drive_from_read_at, drive_from_slice,
    storage::{StorageBudget, StorageError},
};
use zenjpeg::{JpegDecoderConfig, container::metadata::JpegMetadata};
use zenpixels::{PixelDescriptor, PixelSliceMut};

#[test]
fn registered_native_jpeg_stays_suspended_while_an_adapter_prepares_its_destination() {
    use zencodec::{CodecSet, NoPulse, operation::DecodeRequest};
    use zencodec_io::{ControlledEvent, ControlledStepper, ReadAtCursor, Yield};
    let codecs = CodecSet::new().with_decoder(JpegDecoderConfig::new());
    for progressive in [false, true] {
        let bytes = fixtures::fixture(progressive, fixtures::Placement::Header);
        let expected = DecodeRequest::new().decode(&codecs, &bytes).unwrap();
        let budget = StorageBudget::new(64 * 1024, 4);
        let native = DecodeRequest::new()
            .start_controlled(&codecs, budget.clone())
            .unwrap();
        let mut source = SliceReadAt::new(&bytes);
        let mut cursor = ReadAtCursor::new(DriveOptions::new(0, 43));
        let mut sink = Sink::default();
        let mut stage = ControlledStepper::new(native, &mut sink);
        let mut states = Vec::new();
        loop {
            match cursor
                .next(&mut source, &mut stage, Some(&NoPulse))
                .unwrap()
            {
                Yield::Output(ControlledEvent::Suspended(boundary)) => {
                    states.push(boundary.facts().state);
                    assert_eq!(
                        (stage.sink().begin, stage.sink().rows, stage.sink().finish),
                        (0, 0, 0)
                    );
                    let before = cursor.stats();
                    assert!(
                        matches!(cursor.next(&mut source, &mut stage, Some(&NoPulse)).unwrap(),
                        Yield::Output(ControlledEvent::Suspended(again)) if again == boundary)
                    );
                    let after = cursor.stats();
                    assert_eq!(
                        (before.reads, before.bytes_read),
                        (after.reads, after.bytes_read)
                    );
                    if boundary.facts().state == InterpretationState::Final {
                        assert_eq!(
                            stage.output_descriptor(),
                            Some(expected.pixels().descriptor())
                        );
                        assert!(stage.info().unwrap().source_color.icc_profile.is_some());
                        assert_eq!(budget.reserved_objects(), 3, "no native row staging yet");
                    }
                    let (action, _) = zencodec::operation::resolve_boundary(
                        boundary,
                        &mut PolicySet::new(),
                        PolicySet::new(),
                        Some(&NoPulse),
                    )
                    .unwrap();
                    stage.resume(boundary.id(), action, Some(&NoPulse)).unwrap();
                    assert_eq!(stage.sink().begin, 0, "resume does not do native work");
                }
                Yield::Finished(ControlledEvent::Decode(Event::Done(_))) => break,
                Yield::Output(ControlledEvent::Decode(_)) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(
            states,
            [InterpretationState::Provisional, InterpretationState::Final]
        );
        assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
        let got = stage.sink().pixels.as_pixel_slice().unwrap();
        for y in 0..got.rows() {
            assert_eq!(got.row(y), expected.pixels().row(y));
        }
    }
}

#[test]
fn registered_native_controlled_jpeg_matches_the_same_stored_request() {
    use zencodec::{CodecSet, operation::DecodeRequest};
    let codecs = CodecSet::new().with_decoder(JpegDecoderConfig::new());
    for progressive in [false, true] {
        for bytes in [
            fixtures::base(progressive),
            fixtures::fixture(progressive, fixtures::Placement::Header),
        ] {
            let direct = DecodeRequest::new().decode(&codecs, &bytes).unwrap();
            let request = DecodeRequest::new().with_output(direct.pixels().descriptor());
            assert!(!fixtures::oracle(&bytes).is_empty());
            for positioned in [false, true] {
                let budget = StorageBudget::new(64 * 1024, 4);
                let native = request.start_controlled(&codecs, budget.clone()).unwrap();
                let mut session = PolicyDecoder::new(native, PolicySet::new());
                let mut sink = Sink::default();
                if positioned {
                    drive_from_read_at(
                        &mut session,
                        &mut SliceReadAt::new(&bytes),
                        &mut sink,
                        None,
                        DriveOptions::new(0, 43),
                    )
                    .unwrap();
                } else {
                    drive_from_slice(&mut session, &bytes, &mut sink, None, 7).unwrap();
                }
                let pixels = sink.pixels.as_pixel_slice().unwrap();
                assert_eq!(pixels.descriptor(), direct.pixels().descriptor());
                assert_eq!(
                    (pixels.width(), pixels.rows()),
                    (direct.width(), direct.height())
                );
                for y in 0..pixels.rows() {
                    assert_eq!(pixels.row(y), direct.pixels().row(y));
                }
                assert_eq!((sink.begin, sink.finish), (1, 1));
                let states: Vec<_> = session
                    .report()
                    .decisions()
                    .map(|d| {
                        let zencodec::operation::DecisionContext::Boundary(facts) = d.context
                        else {
                            panic!("unexpected native decision domain")
                        };
                        assert_eq!(facts.unavailable, None);
                        facts.state
                    })
                    .collect();
                assert_eq!(
                    states,
                    [InterpretationState::Provisional, InterpretationState::Final]
                );
                assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
            }
        }
    }
}

#[test]
fn registered_native_refusal_preserves_reason_without_output_or_owned_storage() {
    use zencodec::{
        CodecSet,
        operation::{DecisionContext, DecodeRequest},
    };
    let codecs = CodecSet::new().with_decoder(JpegDecoderConfig::new());
    for progressive in [false, true] {
        for placement in [
            fixtures::Placement::AfterFirstScan,
            fixtures::Placement::BeforeEoi,
            fixtures::Placement::SplitIcc,
        ] {
            let bytes = fixtures::fixture(progressive, placement);
            let budget = StorageBudget::new(64 * 1024, 4);
            let native = DecodeRequest::new()
                .start_controlled(&codecs, budget.clone())
                .unwrap();
            let mut session = PolicyDecoder::new(native, PolicySet::new());
            let mut sink = Sink::default();
            let error = drive_from_slice(&mut session, &bytes, &mut sink, None, 13).unwrap_err();
            assert!(zencodec::find_cause::<zencodec::operation::PolicyFailure>(&error).is_some());
            let record = session.report().decisions().last().unwrap();
            let DecisionContext::Boundary(facts) = record.context else {
                panic!()
            };
            assert_eq!(
                facts.unavailable,
                Some(InterpretationUnavailable::LateColorMetadata)
            );
            assert_eq!(record.offered, 0);
            assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
            assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
        }
    }
}

#[test]
#[allow(deprecated)] // Preserve the legacy-placement regression during migration.
fn native_final_boundary_cannot_bypass_unproven_preset_requirements() {
    use zencodec::{
        CodecSet,
        operation::{
            DecisionCode, DecisionContext, DecodeRequest, MetadataTransfer, OutputSchedule,
        },
    };
    let codecs = CodecSet::new().with_decoder(JpegDecoderConfig::new());
    let requirements = [
        PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
        PolicySet::new().require_metadata(MetadataTransfer::PreserveReported),
        PolicySet::new().allow_opaque_profiles(false),
        PolicySet::new().with_workspace_limit(u64::MAX),
        PolicySet::new().require_extended_range(true),
        PolicySet::new().require_output_schedule(OutputSchedule::FinishFirst),
        PolicySet::new().with_output_wait_limit(u64::MAX),
    ];
    for progressive in [false, true] {
        let bytes = fixtures::fixture(progressive, fixtures::Placement::Header);
        for preset in requirements {
            for delegated in [false, true] {
                let budget = StorageBudget::new(64 * 1024, 4);
                let native = DecodeRequest::new()
                    .start_controlled(&codecs, budget.clone())
                    .unwrap();
                let mut direct = preset;
                let resolver = move |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                    if delegated {
                        Ok(Decision::UsePreset)
                    } else {
                        zencodec::operation::OperationPolicy::decide(&mut direct, q)
                    }
                };
                let mut session = PolicyDecoder::new(native, resolver).with_policy_set(preset);
                let mut sink = Sink::default();
                let result = drive_from_slice(&mut session, &bytes, &mut sink, None, 13);
                assert!(result.is_err(), "unproven preset accepted: {preset:?}");
                let error = result.unwrap_err();
                if preset.check_workspace(None).is_err() {
                    let unmet =
                        zencodec::find_cause::<zencodec::operation::UnmetRequirements>(&error)
                            .expect("workspace must refuse before native discovery");
                    assert_eq!(unmet, &preset.check_workspace(None).unwrap_err());
                    assert_eq!(session.report().decisions().count(), 0);
                    assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
                    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
                    continue;
                }
                assert!(
                    zencodec::find_cause::<zencodec::operation::PolicyFailure>(&error).is_some()
                );
                let records: Vec<_> = session.report().decisions().collect();
                assert_eq!(records.len(), 2, "discovery must remain permitted");
                assert_eq!(records[0].code, DecisionCode::Selected);
                assert_eq!(records[1].code, DecisionCode::Declined);
                let DecisionContext::Boundary(facts) = records[1].context else {
                    panic!()
                };
                assert_eq!(facts.state, InterpretationState::Final);
                assert!(
                    !records[1].used_preset,
                    "mandatory admission precedes callback"
                );
                assert!(!records[1].unmet_requirements.is_empty());
                assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
                assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
            }
        }
    }
}

#[derive(Debug)]
enum Error {
    Coroutine(CoroutineError),
    Storage(StorageError),
    Native(At<CodecError>),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coroutine(e) => e.fmt(f),
            Self::Storage(e) => e.fmt(f),
            Self::Native(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Coroutine(e) => e,
            Self::Storage(e) => e,
            Self::Native(e) => e,
        })
    }
}
impl From<CoroutineError> for Error {
    fn from(e: CoroutineError) -> Self {
        Self::Coroutine(e)
    }
}
impl From<StorageError> for Error {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}
impl From<At<CodecError>> for Error {
    fn from(e: At<CodecError>) -> Self {
        Self::Native(e)
    }
}

#[derive(Default)]
struct Work {
    constructors: AtomicUsize,
    batches: AtomicUsize,
}
#[derive(Default)]
struct Sink {
    pixels: CollectSink,
    begin: usize,
    rows: usize,
    finish: usize,
}
impl DecodeRowSink for Sink {
    fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
        self.begin += 1;
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
        self.finish += 1;
        self.pixels.finish()
    }
}

fn decoder(budget: StorageBudget, work: Arc<Work>) -> Box<DynControlledDecoder> {
    configured_decoder(budget, work, false)
}

fn configured_decoder(
    budget: StorageBudget,
    work: Arc<Work>,
    admitted: bool,
) -> Box<DynControlledDecoder> {
    let storage = budget.clone();
    let init = move |mut ctx: zencodec_coroutine::CoroutineContext| async move {
        let len = usize::try_from(ctx.len().await?).map_err(|_| StorageError::SizeOverflow)?;
        let mut input = budget.bytes(len)?;
        for (i, chunk) in input.as_mut_slice().chunks_mut(4096).enumerate() {
            ctx.read_exact_at((i * 4096) as u64, chunk).await?;
        }
        let bytes = input.as_slice();
        let config = JpegDecoderConfig::new();
        let job = config.job().with_orientation(OrientationHint::Preserve);
        let header = job.probe(bytes)?;
        ctx.set_info(header.clone());
        ctx.announce(MetadataUpdate::HeaderReady).await?;
        ctx.interpretation(InterpretationFacts::new(
            header.width,
            header.height,
            None,
            InterpretationState::Provisional,
        ))
        .await?;
        let full = job.probe_full(bytes)?;
        let output = job.output_info(bytes)?;
        // This is an adapter qualification, not a JPEG validity rule. Inventory
        // scope alone does not prove that a late profile governed pixel work.
        let complete_header_profile =
            JpegMetadata::parse(bytes).is_ok_and(|m| m.icc_profile().is_ok());
        let qualified = complete_header_profile && header.source_color == full.source_color;
        ctx.set_info(full);
        ctx.set_output_descriptor(output.native_format);
        ctx.interpretation(InterpretationFacts::new(
            output.width,
            output.height,
            Some(output.native_format),
            if qualified {
                InterpretationState::Final
            } else {
                InterpretationState::Unavailable
            },
        ))
        .await?;
        // In progressive JPEG even construction can decode coefficients. Keep
        // it on the far side of the actual await, not just ahead of row delivery.
        work.constructors.fetch_add(1, Ordering::SeqCst);
        let mut native = job.streaming_decoder(Cow::Borrowed(bytes), &[])?;
        while let Some((y, rows)) = native.next_batch()? {
            work.batches.fetch_add(1, Ordering::SeqCst);
            ctx.write_rows(BatchMeta::final_rows(y), &rows).await?;
        }
        Ok::<_, Error>(output)
    };
    erase_controlled(if admitted {
        CoroutineDecoder::controlled_with_storage(4096, AccessPattern::RandomAccess, storage, init)
            .unwrap()
    } else {
        CoroutineDecoder::controlled(4096, AccessPattern::RandomAccess, init)
    })
}

#[test]
fn native_jpeg_only_starts_after_final_policy_and_matches_direct_pixels() {
    for progressive in [false, true] {
        for bytes in [
            fixtures::base(progressive),
            fixtures::fixture(progressive, fixtures::Placement::Header),
        ] {
            let direct = JpegDecoderConfig::new()
                .job()
                .with_orientation(OrientationHint::Preserve)
                .decoder(Cow::Borrowed(&bytes), &[])
                .unwrap()
                .decode()
                .unwrap();
            // Independent decoder accepts the same complete profile and stream.
            assert!(!fixtures::oracle(&bytes).is_empty());
            for random_access in [false, true] {
                let budget = StorageBudget::new(bytes.len(), 1);
                let work = Arc::new(Work::default());
                let mut calls = 0;
                let mut resolver = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                    let Question::Boundary { facts, choices } = q else {
                        panic!("wrong domain")
                    };
                    assert_eq!(work.constructors.load(Ordering::SeqCst), 0);
                    assert_eq!(work.batches.load(Ordering::SeqCst), 0);
                    assert_eq!(budget.reserved_bytes(), bytes.len());
                    assert_eq!(choices.len(), 1);
                    assert_eq!(
                        facts.state,
                        if calls == 0 {
                            InterpretationState::Provisional
                        } else {
                            InterpretationState::Final
                        }
                    );
                    calls += 1;
                    Ok(Decision::UsePreset)
                };
                let mut decoder =
                    PolicyDecoder::new(decoder(budget.clone(), work.clone()), &mut resolver);
                let mut sink = Sink::default();
                if random_access {
                    drive_from_read_at(
                        &mut decoder,
                        &mut SliceReadAt::new(&bytes),
                        &mut sink,
                        None,
                        DriveOptions::new(0, 113),
                    )
                    .unwrap();
                } else {
                    drive_from_slice(&mut decoder, &bytes, &mut sink, None, 7).unwrap();
                }
                assert_eq!((sink.begin, sink.finish), (1, 1));
                assert_eq!(sink.rows, work.batches.load(Ordering::SeqCst));
                assert_eq!(work.constructors.load(Ordering::SeqCst), 1);
                assert_eq!(decoder.report().decisions().count(), 2);
                for y in 0..fixtures::HEIGHT {
                    assert_eq!(
                        sink.pixels.as_pixel_slice().unwrap().row(y),
                        direct.pixels().row(y)
                    );
                }
                assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
                drop(decoder);
                assert_eq!(calls, 2);
            }
        }
    }
}

#[test]
fn native_jpeg_runs_with_shared_input_and_row_admission() {
    for progressive in [false, true] {
        let bytes = fixtures::fixture(progressive, fixtures::Placement::Header);
        let direct = JpegDecoderConfig::new()
            .job()
            .with_orientation(OrientationHint::Preserve)
            .decoder(Cow::Borrowed(&bytes), &[])
            .unwrap()
            .decode()
            .unwrap();
        for decline in [false, true] {
            let budget = StorageBudget::new(64 * 1024, 4);
            let work = Arc::new(Work::default());
            let native = configured_decoder(budget.clone(), work.clone(), true);
            let initial = budget.reserved_bytes();
            assert_eq!(
                budget.reserved_objects(),
                2,
                "input payload and index are separate admitted allocations"
            );
            let mut resolver = |_q: Question<'_>| -> Result<Decision, At<Infallible>> {
                assert_eq!(budget.reserved_bytes(), initial + bytes.len());
                assert_eq!(budget.reserved_objects(), 3);
                assert_eq!(work.constructors.load(Ordering::SeqCst), 0);
                Ok(if decline {
                    Decision::decline(Decline::ApplicationRule)
                } else {
                    Decision::UsePreset
                })
            };
            let mut session = PolicyDecoder::new(native, &mut resolver);
            let mut sink = Sink::default();
            let result = drive_from_read_at(
                &mut session,
                &mut SliceReadAt::new(&bytes),
                &mut sink,
                None,
                DriveOptions::new(0, 37),
            );
            if decline {
                assert!(result.is_err());
                assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
            } else {
                result.unwrap();
                for y in 0..fixtures::HEIGHT {
                    assert_eq!(
                        sink.pixels.as_pixel_slice().unwrap().row(y),
                        direct.pixels().row(y)
                    );
                }
            }
            assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
        }
    }
}

#[test]
fn native_late_or_incomplete_profiles_refuse_before_decoder_and_sink_effects() {
    for progressive in [false, true] {
        for placement in [
            fixtures::Placement::AfterFirstScan,
            fixtures::Placement::BeforeEoi,
            fixtures::Placement::SplitIcc,
        ] {
            let bytes = fixtures::fixture(progressive, placement);
            let budget = StorageBudget::new(bytes.len(), 1);
            let work = Arc::new(Work::default());
            let mut unavailable = false;
            let mut resolver = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Boundary { facts, choices } = q else {
                    panic!()
                };
                if facts.state == InterpretationState::Unavailable {
                    unavailable = true;
                    assert!(choices.is_empty());
                }
                Ok(Decision::UsePreset)
            };
            let mut decoder =
                PolicyDecoder::new(decoder(budget.clone(), work.clone()), &mut resolver);
            let mut sink = Sink::default();
            assert!(drive_from_slice(&mut decoder, &bytes, &mut sink, None, 17).is_err());
            assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
            assert_eq!(work.constructors.load(Ordering::SeqCst), 0);
            assert_eq!(decoder.report().decisions().count(), 2);
            // A caller may retain the failed session to inspect its report.
            assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
            drop(decoder);
            assert!(unavailable, "{progressive}/{placement:?}");
        }
    }
}

#[test]
#[allow(deprecated)] // Preserve the legacy-placement regression during migration.
fn native_input_admission_and_policy_decline_precede_affected_work() {
    let bytes = fixtures::base(true);
    for too_small in [true, false] {
        let budget = StorageBudget::new(bytes.len() - usize::from(too_small), 1);
        let work = Arc::new(Work::default());
        let mut resolver = |_q: Question<'_>| -> Result<Decision, At<Infallible>> {
            Ok(Decision::decline(Decline::ApplicationRule))
        };
        let mut decoder = PolicyDecoder::new(decoder(budget.clone(), work.clone()), &mut resolver)
            .with_policy_set(PolicySet::new());
        let mut sink = Sink::default();
        let error = drive_from_slice(&mut decoder, &bytes, &mut sink, None, 23).unwrap_err();
        if too_small {
            assert!(zencodec::find_cause::<StorageError>(&error).is_some());
        }
        assert_eq!(work.constructors.load(Ordering::SeqCst), 0);
        assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
        assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    }
}

#[test]
fn native_waiting_input_is_released_after_callback_unwind_and_native_failure() {
    let bytes = fixtures::base(true);
    let budget = StorageBudget::new(bytes.len(), 1);
    let work = Arc::new(Work::default());
    let mut session = PolicyDecoder::new(
        decoder(budget.clone(), work.clone()),
        |_q: Question<'_>| -> Result<Decision, At<Infallible>> { panic!("application callback") },
    );
    let mut sink = Sink::default();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drive_from_slice(&mut session, &bytes, &mut sink, None, 31)
        }))
        .is_err()
    );
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    assert!(matches!(
        session.step(Input::empty(), &mut sink, Some(&zencodec::how_far::NoPulse)),
        Err(StepError::Finished)
    ));
    assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
    assert_eq!(work.constructors.load(Ordering::SeqCst), 0);
    drop(session);

    let truncated = &bytes[..bytes.len() - 2];
    let mut native_error =
        PolicyDecoder::new(decoder(budget.clone(), work.clone()), PolicySet::new());
    let error = drive_from_slice(&mut native_error, truncated, &mut sink, None, 31).unwrap_err();
    assert!(zencodec::find_cause::<CodecError>(&error).is_some());
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
    assert_eq!(work.constructors.load(Ordering::SeqCst), 0);
}

#[test]
#[allow(deprecated)] // Both entry requirement locations must precede native polling.
fn workspace_requirement_refuses_before_native_discovery() {
    use zencodec::operation::{
        BoundPolicy, PolicyRequirement, RequirementEvidence, UnmetRequirements,
    };
    for legacy in [false, true] {
        for limit in [0, u64::MAX] {
            let budget = StorageBudget::new(64 * 1024, 4);
            let work = Arc::new(Work::default());
            let native = decoder(budget.clone(), work.clone());
            let required = PolicySet::new().with_workspace_limit(limit);
            let binding = BoundPolicy::new(
                if legacy { PolicySet::new() } else { required },
                |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                    panic!("unqualified workspace must refuse before callback")
                },
            );
            let mut session = PolicyDecoder::new(native, binding).with_policy_set(if legacy {
                required
            } else {
                PolicySet::new()
            });
            let mut sink = Sink::default();
            let error = session.step(Input::empty(), &mut sink, None).unwrap_err();
            let unmet = zencodec::find_cause::<UnmetRequirements>(&error).unwrap();
            assert_eq!(
                unmet.iter().collect::<Vec<_>>(),
                [(PolicyRequirement::Workspace, RequirementEvidence::Unknown)]
            );
            assert_eq!(session.report().decisions().count(), 0);
            assert_eq!(work.constructors.load(Ordering::SeqCst), 0);
            assert_eq!(work.batches.load(Ordering::SeqCst), 0);
            assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
            assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
            assert!(matches!(
                session.step(Input::empty(), &mut sink, None),
                Err(StepError::Finished)
            ));
        }
    }
}

#[test]
fn factory_admission_precedes_native_construction_and_retains_errors() {
    use zencodec::operation::{
        BoundPolicy, PolicyRequirement, RequirementEvidence, UnmetRequirements,
    };
    for limit in [0, u64::MAX] {
        let budget = StorageBudget::new(64 * 1024, 4);
        let work = Arc::new(Work::default());
        let calls = std::cell::Cell::new(0);
        let policy = BoundPolicy::new(
            PolicySet::new().with_workspace_limit(limit),
            |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                panic!("admission must precede callback")
            },
        );
        let result = PolicyDecoder::try_from_factory(
            || -> Result<_, Infallible> {
                calls.set(calls.get() + 1);
                Ok(decoder(budget.clone(), work.clone()))
            },
            policy,
        );
        let error = result
            .err()
            .expect("unknown bound must refuse before construction");
        assert_eq!(calls.get(), 0);
        assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
        assert_eq!(
            zencodec::find_cause::<UnmetRequirements>(&error)
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [(PolicyRequirement::Workspace, RequirementEvidence::Unknown)]
        );
    }
    let calls = std::cell::Cell::new(0);
    let result = PolicyDecoder::<Box<DynControlledDecoder>, _>::try_from_factory(
        || {
            calls.set(calls.get() + 1);
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "native factory timeout",
            ))
        },
        PolicySet::new(),
    );
    let error = result.err().expect("original factory failure");
    assert_eq!(calls.get(), 1);
    let cause = zencodec::find_cause::<std::io::Error>(&error).unwrap();
    assert_eq!(cause.kind(), std::io::ErrorKind::TimedOut);
    assert_eq!(cause.to_string(), "native factory timeout");
}

#[test]
fn factory_keeps_metadata_requirements_pending_for_native_boundaries() {
    use zencodec::operation::{BoundPolicy, MetadataTransfer, PolicyFailure};
    let bytes = fixtures::fixture(false, fixtures::Placement::Header);
    let budget = StorageBudget::new(64 * 1024, 4);
    let work = Arc::new(Work::default());
    let mut factory_calls = 0;
    let mut decisions = 0;
    let mut session = PolicyDecoder::try_from_factory(
        || -> Result<_, Infallible> {
            factory_calls += 1;
            Ok(decoder(budget.clone(), work.clone()))
        },
        BoundPolicy::new(
            PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
            |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                decisions += 1;
                Ok(Decision::UsePreset)
            },
        ),
    )
    .unwrap();
    assert_eq!(factory_calls, 1);
    let mut sink = Sink::default();
    let error = drive_from_slice(&mut session, &bytes, &mut sink, None, 13).unwrap_err();
    assert!(zencodec::find_cause::<PolicyFailure>(&error).is_some());
    assert_eq!(session.report().decisions().count(), 2);
    assert_eq!((sink.begin, sink.rows, sink.finish), (0, 0, 0));
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    drop(session);
    assert_eq!(decisions, 1, "only discovery reaches the callback");
}
