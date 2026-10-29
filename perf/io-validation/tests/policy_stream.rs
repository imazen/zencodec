//! A native ordering counterexample plus an explicit private barrier protocol.
//! Late discovery and media epochs are authored controls, not codec guarantees.
#![forbid(unsafe_code)]
#[path = "../src/admission.rs"]
mod admission;
#[path = "../src/policy/mod.rs"]
mod policy;
#[path = "../src/policy_disclosure.rs"]
mod policy_disclosure;
#[path = "../src/policy_stream.rs"]
mod policy_stream;
use core::{convert::Infallible, error::Error, fmt};
use policy::*;
use policy_disclosure::{Obligation, Prepared, Privacy, TranscodeRoute};
use policy_stream::*;
use std::sync::atomic::{AtomicBool, Ordering};
use zenbitmaps::{QoiDecoderConfig, QoiEncoderConfig};
use zencodec::decode::{
    DecodeRowSink, DecoderConfig, Event, IncrementalDecode, Input, MetadataUpdate,
};
use zencodec::encode::{EncodeJob, EncoderConfig};
use zencodec::enough::{Stop, StopReason};
use zencodec::{At, NoPulse};
use zencodec_io::EncoderSink;
use zenpixels::{PixelDescriptor, PixelSlice};

fn engine() -> Engine {
    Engine::new(PolicySet::default(), 32)
}
fn first(question: Question<'_>) -> Result<Decision, At<Infallible>> {
    Ok(Decision::choose(
        question.choices().unwrap().first().unwrap(),
    ))
}
fn gate(cap: usize) -> Gate {
    Gate::new(7, cap, false).unwrap()
}
fn final_color(gate: &mut Gate, engine: &mut Engine) -> Receipt {
    gate.discover(Knowledge::Final(Interpretation::Srgb))
        .unwrap();
    gate.resolve(engine, &mut first, Some(&NoPulse)).unwrap()
}

#[test]
fn real_incremental_step_has_started_the_encoder_before_metadata_observation() {
    let bytes = [31, 63, 127, 255];
    let encoded = QoiEncoderConfig::new()
        .encode(PixelSlice::new(&bytes, 1, 1, 4, PixelDescriptor::RGBA8_SRGB).unwrap())
        .unwrap();
    let mut decoder = QoiDecoderConfig::new()
        .job()
        .into_incremental_decoder(&[])
        .unwrap();
    let mut sink = EncoderSink::new(QoiEncoderConfig::new().job().encoder().unwrap());
    assert!(sink.pending_output().is_empty());
    let progress = decoder
        .step(
            Input::new(0, encoded.data()).with_total_len(encoded.data().len() as u64),
            &mut sink,
            Some(&NoPulse),
        )
        .unwrap();
    assert_eq!(progress.event, Event::Metadata(MetadataUpdate::HeaderReady));
    assert_eq!(sink.pending_output().len(), 14);
    assert_eq!(&sink.pending_output()[..4], b"qoif");
    // The observer only runs here. This proves ordering, not wrong QOI color.
    assert!(sink.output_timing().is_some());
}

#[test]
fn a_late_fact_resumes_native_encoding_and_preserves_strided_pixels_exactly() {
    let raw = [
        10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 0xa5, 0xa5, 0xa5, 0xa5, 12, 22, 32, 42,
        52, 62, 72, 82, 92, 102, 112, 122, 0xa5, 0xa5, 0xa5, 0xa5,
    ];
    let pixels = PixelSlice::new(&raw, 3, 2, 16, PixelDescriptor::RGBA8_SRGB).unwrap();
    let oracle = QoiEncoderConfig::new().encode(pixels.clone()).unwrap();
    let mut gate = gate(24);
    let mut engine = engine();
    let memory = gate.storage();
    let mut calls = 0;
    let mut resolver = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
        calls += 1;
        match q {
            Question::Boundary {
                facts:
                    BoundaryFacts {
                        trigger: BoundaryTrigger::Interpretation { status, cicp, .. },
                        ..
                    },
                ..
            } => {
                if calls == 1 {
                    assert_eq!(status, FactStatus::Pending);
                    assert_eq!(cicp, None);
                } else {
                    assert_eq!(status, FactStatus::Final);
                    assert_eq!(cicp, Some(zencodec::Cicp::SRGB));
                }
            }
            _ => panic!("expected a typed interpretation question"),
        }
        // Existing admitted data can remain live. A callback must not run with
        // the budget mutex held; reading occupancy here exercises that boundary.
        let _ = memory.occupancy();
        first(q)
    };
    let wait = gate
        .resolve(&mut engine, &mut resolver, Some(&NoPulse))
        .unwrap();
    assert_eq!(wait.action(), BoundaryAction::Wait);
    assert_eq!(
        gate.before_effect(&wait).unwrap_err().error(),
        &GateCode::Suspended
    );
    for row in 0..2 {
        gate.stage(&wait, pixels.row(row), Some(&NoPulse)).unwrap();
    }
    assert_eq!(gate.bytes_held(), 24);
    assert!(
        gate.discover(Knowledge::Final(Interpretation::Srgb))
            .unwrap()
    );
    assert_eq!(
        gate.before_effect(&wait).unwrap_err().error(),
        &GateCode::Stale
    );
    let ready = gate
        .resolve(&mut engine, &mut resolver, Some(&NoPulse))
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(ready.action(), BoundaryAction::Adopt);
    let leases = gate.take_staged(&ready).unwrap();
    let mut sink = EncoderSink::new(QoiEncoderConfig::new().job().encoder().unwrap());
    gate.before_effect(&ready).unwrap();
    sink.begin(3, 2, PixelDescriptor::RGBA8_SRGB).unwrap();
    gate.encoder_started(&ready).unwrap();
    let mut output = Vec::new();
    for (y, lease) in leases.into_iter().enumerate() {
        gate.before_effect(&ready).unwrap();
        {
            let mut buffer = sink
                .provide_next_buffer(y as u32, 1, 3, PixelDescriptor::RGBA8_SRGB)
                .unwrap();
            buffer.row_mut(0).copy_from_slice(lease.bytes());
        }
        gate.before_effect(&ready).unwrap();
        gate.published(&ready, sink.pending_output().len()).unwrap();
        sink.take_output(&mut output);
    }
    assert_eq!(gate.bytes_held(), 0);
    let remainder = sink.finish().unwrap();
    gate.before_effect(&ready).unwrap();
    output.extend_from_slice(remainder.data());
    gate.published(&ready, remainder.data().len()).unwrap();
    assert_eq!(output, oracle.data());
    let decoded = QoiDecoderConfig::new().decode(&output).unwrap();
    for y in 0..2 {
        assert_eq!(decoded.pixels().row(y), pixels.row(y));
    }
    gate.finish(&ready).unwrap();
    assert_eq!(gate.context().published_bytes, output.len() as u64);
    assert_eq!(engine.records().count(), 2);
}

#[test]
fn wait_means_bounded_storage_or_refusal_not_unbounded_raw_retention() {
    let mut gate = gate(12);
    let mut engine = engine();
    let wait = gate
        .resolve(&mut engine, &mut first, Some(&NoPulse))
        .unwrap();
    gate.stage(&wait, &[1; 8], Some(&NoPulse)).unwrap();
    assert_eq!(gate.bytes_held(), 8);
    let err = gate.stage(&wait, &[2; 8], Some(&NoPulse)).unwrap_err();
    assert_eq!(err.error(), &GateCode::Capacity(admission::Refusal::Bytes));
    assert_eq!(gate.bytes_held(), 0);
    assert_eq!(gate.storage().occupancy().peak_bytes, 8);
    assert_eq!(gate.commitments(), Commitments::default());
    assert!(
        gate.discover(Knowledge::Final(Interpretation::Srgb))
            .is_err()
    );
    assert_eq!(engine.records().count(), 1);
}

#[test]
fn pending_provisional_and_unavailable_are_not_false_or_srgb_defaults() {
    for fact in [
        Knowledge::Pending,
        Knowledge::Provisional(Interpretation::Linear),
        Knowledge::Unavailable,
    ] {
        let mut gate = gate(8);
        gate.discover(fact).unwrap();
        let result = gate.resolve(&mut engine(), &mut first, Some(&NoPulse));
        if fact == Knowledge::Unavailable {
            assert_eq!(
                zencodec::find_cause::<Failure<Infallible>>(&result.err().unwrap())
                    .unwrap()
                    .code(),
                Code::NoAlternative
            );
        } else {
            let wait = result.unwrap();
            assert_eq!(wait.action(), BoundaryAction::Wait);
            assert!(gate.before_effect(&wait).is_err());
            assert!(
                gate.finish(&wait).is_err(),
                "EOF must not silently finalize unknown color"
            );
        }
    }
}

#[test]
fn duplicate_facts_rows_and_stalls_do_not_trigger_more_policy_calls() {
    let mut gate = gate(64);
    let mut engine = engine();
    let wait = gate
        .resolve(&mut engine, &mut first, Some(&NoPulse))
        .unwrap();
    for _ in 0..8 {
        assert!(!gate.discover(Knowledge::Pending).unwrap());
        gate.stage(&wait, &[1, 2, 3, 4], Some(&NoPulse)).unwrap();
    }
    assert!(
        gate.resolve(&mut engine, &mut first, Some(&NoPulse))
            .is_err()
    );
    assert_eq!(engine.records().count(), 1);
    let ready = final_color(&mut gate, &mut engine);
    for _ in 0..8 {
        assert!(
            !gate
                .discover(Knowledge::Final(Interpretation::Srgb))
                .unwrap()
        );
        gate.before_effect(&ready).unwrap();
    }
    assert_eq!(engine.records().count(), 2);
    drop(gate.take_staged(&ready).unwrap());
    gate.finish(&ready).unwrap();
}

#[test]
fn all_four_commitments_independently_prevent_reinterpreting_existing_work() {
    for mask in 0..16 {
        let commitments = Commitments {
            samples_changed: mask & 1 != 0,
            encoder_started: mask & 2 != 0,
            headers_sent: mask & 4 != 0,
            body_bytes: if mask & 8 != 0 { 1 } else { 0 },
        };
        for value in [Interpretation::Srgb, Interpretation::Linear] {
            let mut gate = gate(8).with_prior_effects(Interpretation::Srgb, commitments);
            gate.discover(Knowledge::Final(value)).unwrap();
            let mut engine = engine();
            let result = gate.resolve(&mut engine, &mut first, Some(&NoPulse));
            assert_eq!(
                result.is_ok(),
                mask == 0 || value == Interpretation::Srgb,
                "mask={mask}, value={value:?}"
            );
            if let Err(error) = result {
                let failure = zencodec::find_cause::<Failure<Infallible>>(&error).unwrap();
                assert_eq!(failure.code(), Code::NoAlternative);
                assert!(
                    matches!(failure.details().unwrap().facts, Facts::Boundary(BoundaryFacts { commitments: c, .. }) if c == commitments)
                );
            }
            assert_eq!(gate.commitments(), commitments);
        }
    }
}

#[test]
fn finality_is_a_codec_promise_and_cannot_be_overridden_by_policy() {
    let mut gate = gate(8);
    let mut engine = engine();
    let ready = final_color(&mut gate, &mut engine);
    gate.published(&ready, 5).unwrap();
    assert_eq!(
        gate.discover(Knowledge::Final(Interpretation::Linear))
            .unwrap_err()
            .error(),
        &GateCode::ContradictoryFinality
    );
    assert_eq!(gate.context().published_bytes, 5);
    assert_eq!(engine.records().count(), 1);
    assert!(gate.before_effect(&ready).is_err());
}

#[test]
fn receipts_cannot_cross_source_operation_fact_revision_or_media_epoch() {
    let mut engine = engine();
    let mut a = gate(8);
    let mut b = Gate::new(99, 8, false).unwrap();
    let ready = final_color(&mut a, &mut engine);
    let _other = final_color(&mut b, &mut engine);
    assert_eq!(
        b.before_effect(&ready).unwrap_err().error(),
        &GateCode::Stale
    );
    assert_eq!(
        a.next_epoch(&ready).unwrap_err().error(),
        &GateCode::EpochNotSupported
    );
    let mut media = Gate::new(7, 8, true).unwrap();
    let old = final_color(&mut media, &mut engine);
    media.published(&old, 20).unwrap();
    media.next_epoch(&old).unwrap();
    assert_eq!(
        media.before_effect(&old).unwrap_err().error(),
        &GateCode::Stale
    );
    media
        .discover(Knowledge::Final(Interpretation::Linear))
        .unwrap();
    let new = media
        .resolve(&mut engine, &mut first, Some(&NoPulse))
        .unwrap();
    media.before_effect(&new).unwrap();
    assert_eq!(media.context().epoch, 1);
    assert_eq!(media.context().published_bytes, 20);
}

#[derive(Debug)]
struct AccountError;
impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("private account payload")
    }
}
impl Error for AccountError {}

#[test]
fn late_callback_error_preserves_its_cause_and_trace_and_releases_staging() {
    let mut gate = gate(8);
    let mut engine = engine();
    let wait = gate
        .resolve(&mut engine, &mut first, Some(&NoPulse))
        .unwrap();
    gate.stage(&wait, &[42; 8], Some(&NoPulse)).unwrap();
    gate.discover(Knowledge::Final(Interpretation::Srgb))
        .unwrap();
    let mut fail = |_: Question<'_>| -> Result<Decision, At<AccountError>> {
        Err(At::wrap(AccountError).at_named("late application rule"))
    };
    let error = gate
        .resolve(&mut engine, &mut fail, Some(&NoPulse))
        .unwrap_err();
    assert!(zencodec::find_cause::<AccountError>(&error).is_some());
    assert!(format!("{}", error.full_trace()).contains("late application rule"));
    assert!(!format!("{error:?}").contains("private account payload"));
    assert_eq!(gate.bytes_held(), 0);
    assert_eq!(gate.commitments(), Commitments::default());
    assert_eq!(engine.records().count(), 2);
    assert_eq!(
        gate.discover(Knowledge::Pending).unwrap_err().error(),
        &GateCode::Terminal
    );
}

struct Cancel(AtomicBool);
impl Stop for Cancel {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.load(Ordering::Relaxed) {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[test]
fn cancellation_and_panic_never_implicitly_resume_the_suspended_stream() {
    let mut gate = gate(8);
    let mut engine = engine();
    let wait = gate
        .resolve(&mut engine, &mut first, Some(&NoPulse))
        .unwrap();
    gate.stage(&wait, &[1; 8], Some(&NoPulse)).unwrap();
    gate.discover(Knowledge::Final(Interpretation::Srgb))
        .unwrap();
    let stop = Cancel(AtomicBool::new(false));
    let mut cancel = |q: Question<'_>| {
        stop.0.store(true, Ordering::Relaxed);
        first(q)
    };
    assert!(
        gate.resolve(
            &mut engine,
            &mut cancel,
            Some(&zencodec::how_far::StopOnly::borrowed(&stop)),
        )
        .is_err()
    );
    assert_eq!(gate.bytes_held(), 0);
    assert_eq!(gate.commitments(), Commitments::default());
    let mut gate = Gate::new(7, 8, false).unwrap();
    let mut engine = Engine::new(PolicySet::default(), 4);
    let wait = gate
        .resolve(&mut engine, &mut first, Some(&NoPulse))
        .unwrap();
    gate.stage(&wait, &[1; 8], Some(&NoPulse)).unwrap();
    let memory = gate.storage();
    gate.discover(Knowledge::Final(Interpretation::Srgb))
        .unwrap();
    let mut panic =
        |_: Question<'_>| -> Result<Decision, At<Infallible>> { panic!("callback panic control") };
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gate.resolve(
            &mut engine,
            &mut panic,
            Some(&NoPulse)
        )))
        .is_err()
    );
    assert_eq!(
        gate.discover(Knowledge::Pending).unwrap_err().error(),
        &GateCode::Terminal
    );
    assert_eq!(
        memory.occupancy().bytes,
        8,
        "panic cannot uncharge live storage"
    );
    drop(gate);
    assert_eq!(memory.occupancy().bytes, 0);
}

#[test]
fn trailing_opaque_metadata_is_dropped_before_disclosure_without_replanning_color() {
    let mut gate = gate(0);
    let mut engine = engine();
    let ready = final_color(&mut gate, &mut engine);
    let mut writer = b"allowed encoded prefix".to_vec();
    gate.published(&ready, writer.len()).unwrap();
    let approved = Prepared::opaque(
        b"private trailing packet",
        Carrier::Xmp,
        Obligation::OptionalRemovable,
        Privacy::OrientationOnlyV1,
        TranscodeRoute::Media,
    )
    .resolve(&mut engine, gate.context(), &mut first, Some(&NoPulse))
    .unwrap();
    assert_eq!(approved.action, BoundaryAction::Drop);
    gate.before_effect(&ready).unwrap();
    writer.extend_from_slice(approved.bytes());
    assert_eq!(writer, b"allowed encoded prefix");
    assert_eq!(engine.records().count(), 2);
    let required = Prepared::opaque(
        b"private trailing packet",
        Carrier::Xmp,
        Obligation::ExactBytes,
        Privacy::OrientationOnlyV1,
        TranscodeRoute::Media,
    )
    .resolve(&mut engine, gate.context(), &mut first, Some(&NoPulse));
    assert!(required.is_err());
    gate.abort();
    assert_eq!(
        writer, b"allowed encoded prefix",
        "an error must not excuse a prior disclosure"
    );
}
