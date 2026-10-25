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
        session.step(Input::empty(), &mut sink, &zencodec::enough::Unstoppable),
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
