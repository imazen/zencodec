use super::*;
use std::{cell::Cell, rc::Rc};
use zencodec::frame::Frame;
use zencodec::{
    At,
    operation::{Decision, Decline, FrameConversionOffer, FrameFacts, PolicySet, Question},
    storage::StorageBudget,
};
use zencodec_media::session::{Receive, SendStatus};
use zencodec_media::{
    session::VideoEncoder,
    staged::{Stage as EncodeStage, StagedEncode},
};

struct OwnedEncoder {
    native: zenavif::media::Av1VideoEncoder,
    live: Rc<Cell<bool>>,
    corrupt_epoch: bool,
}
impl Drop for OwnedEncoder {
    fn drop(&mut self) {
        self.live.set(false);
    }
}
impl VideoEncoder for OwnedEncoder {
    type Error = <zenavif::media::Av1VideoEncoder as VideoEncoder>::Error;
    fn send(
        &mut self,
        frame: &Frame,
        stop: &dyn Stop,
    ) -> Result<SendStatus, SessionError<Self::Error>> {
        self.native.send(frame, stop)
    }
    fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Packet>, SessionError<Self::Error>> {
        Ok(match self.native.receive(stop)? {
            Receive::Ready(packet) if self.corrupt_epoch => {
                Receive::Ready(packet.with_config_epoch(99))
            }
            other => other,
        })
    }
    fn end_input(&mut self) -> Result<(), SessionError<Self::Error>> {
        self.native.end_input()
    }
    fn output_config(&self) -> Option<&CodecConfig> {
        self.native.output_config()
    }
}
struct Writer<'a> {
    bytes: &'a mut Vec<u8>,
    live: Rc<Cell<bool>>,
    calls: Rc<Cell<usize>>,
    fail: bool,
}
impl Write for Writer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        assert!(!self.live.get(), "writer waited with encoder still alive");
        self.calls.set(self.calls.get() + 1);
        if self.fail && self.bytes.len() >= 13 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let n = bytes
            .len()
            .min(if self.fail { 13 - self.bytes.len() } else { 3 });
        self.bytes.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        assert!(!self.live.get());
        Ok(())
    }
}
fn encoder(live: &Rc<Cell<bool>>) -> OwnedEncoder {
    live.set(true);
    OwnedEncoder {
        native: zenavif::media::Av1VideoEncoder::new(settings(8, Subsampling::Yuv420, false), TB)
            .unwrap(),
        live: live.clone(),
        corrupt_epoch: false,
    }
}

#[test]
fn staged_native_owned_pipeline_drops_encoder_before_writer_and_preserves_pixels() {
    for fail in [false, true] {
        let live = Rc::new(Cell::new(false));
        let calls = Rc::new(Cell::new(0));
        let budget = StorageBudget::new(1 << 20, 64);
        let mut bytes = Vec::new();
        let mut operation = StagedEncode::new(
            encoder(&live),
            ExactMp4(zenmp4::Mp4Muxer::new()),
            Forward(Writer {
                bytes: &mut bytes,
                live: live.clone(),
                calls: calls.clone(),
                fail,
            }),
            PolicySet::new(),
            budget.clone(),
            &Unstoppable,
        )
        .unwrap();
        let settings = settings(8, Subsampling::Yuv420, false);
        let frames: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
        for (index, input) in frames.iter().enumerate() {
            if index == 0 {
                operation.push(input.clone(), &Unstoppable).unwrap();
            } else {
                let target = input.interpretation().clone();
                let conversion =
                    FrameConversionOffer::new(FrameFacts::new(&target, input.timing()));
                let executions = Cell::new(0);
                operation
                    .push_converted(
                        input.clone(),
                        &target,
                        conversion,
                        |held| {
                            executions.set(executions.get() + 1);
                            Ok(held)
                        },
                        &Unstoppable,
                    )
                    .unwrap();
                assert_eq!(executions.get(), 1);
            }
            assert_eq!(calls.get(), 0);
        }
        let result = operation.finish(&Unstoppable);
        assert!(!live.get());
        assert_eq!(budget.reserved_bytes(), 0);
        if fail {
            let error = result.unwrap_err();
            assert_eq!(error.error().stage, EncodeStage::Publish);
            assert_eq!(error.error().report.decisions().count(), 3);
            assert_eq!(
                zencodec::find_cause::<io::Error>(&error).unwrap().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                zencodec::find_cause::<zencodec_media::output::Failure>(&error)
                    .unwrap()
                    .emitted,
                13
            );
            assert_eq!(bytes.len(), 13);
        } else {
            let completed = result.unwrap();
            assert_eq!(completed.report.decisions().count(), 3);
            assert_eq!(completed.stats.emitted as usize, bytes.len());
            oracle_container(&root(), "owned-staged", "mp4", &settings, &bytes, &frames);
        }
    }
}

#[test]
fn staged_native_refusal_conversion_drift_and_unwind_never_publish_or_resume() {
    for mode in ["policy", "drift", "unwind", "budget", "cancel", "epoch"] {
        let live = Rc::new(Cell::new(false));
        let calls = Rc::new(Cell::new(0));
        let budget = StorageBudget::new(if mode == "budget" { 0 } else { 1 << 20 }, 64);
        let mut bytes = Vec::new();
        let executions = Cell::new(0);
        let control = StopAfterWrite {
            armed: std::sync::atomic::AtomicBool::new(false),
            stopped: std::sync::atomic::AtomicBool::new(false),
            reason: enough::StopReason::TimedOut,
        };
        let mut native = encoder(&live);
        native.corrupt_epoch = mode == "epoch";
        let mut operation = StagedEncode::new(
            native,
            ExactMp4(zenmp4::Mp4Muxer::new()),
            Forward(Writer {
                bytes: &mut bytes,
                live: live.clone(),
                calls: calls.clone(),
                fail: false,
            }),
            |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
                let Question::FrameRoute { choices, .. } = q else {
                    panic!()
                };
                assert!(
                    choices
                        .first()
                        .unwrap()
                        .value()
                        .destination
                        .unwrap()
                        .exact_timing
                );
                if mode == "policy" {
                    Ok(Decision::decline(Decline::NoPreferredChoice))
                } else {
                    Ok(Decision::UsePreset)
                }
            },
            budget.clone(),
            &Unstoppable,
        )
        .unwrap();
        let input = frame(&settings(8, Subsampling::Yuv420, false), 0);
        let target = input.interpretation().clone();
        let conversion = FrameConversionOffer::new(FrameFacts::new(&target, input.timing()));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            operation.push_converted(
                input.clone(),
                &target,
                conversion,
                |held| {
                    executions.set(executions.get() + 1);
                    assert!(live.get());
                    if mode == "unwind" {
                        panic!("controlled converter unwind");
                    }
                    if mode == "cancel" {
                        control
                            .stopped
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    if mode == "drift" {
                        Ok(held.represent(
                            zencodec::frame::FrameTiming::default(),
                            held.presentation(),
                        ))
                    } else {
                        Ok(held)
                    }
                },
                &control,
            )
        }));
        if mode == "budget" || mode == "epoch" {
            // Native encoders may delay their first packet until draining.
            if result.unwrap().is_ok() {
                assert!(operation.finish(&Unstoppable).is_err());
            } else {
                assert!(operation.is_failed());
            }
        } else {
            if mode == "unwind" {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_err());
            }
            assert!(operation.is_failed());
            assert!(operation.push(input, &Unstoppable).is_err());
        }
        assert!(!live.get());
        assert_eq!(calls.get(), 0);
        assert_eq!(budget.reserved_bytes(), 0);
        if mode == "policy" {
            assert_eq!(executions.get(), 0);
        }
    }
}

#[test]
fn staged_native_changing_ycbcr_converts_to_fixed_rgb_before_owned_output() {
    use crate::coupled_route;
    use zencodec::operation::{PolicyFailure, Selection};
    for require_range in [false, true] {
        let inputs = coupled_route::sources();
        let live = Rc::new(Cell::new(true));
        let writes = Rc::new(Cell::new(0));
        let calls = Cell::new(0);
        let executions = Cell::new(0);
        let converted = StorageBudget::new(1 << 20, 64);
        let encoded = StorageBudget::new(1 << 20, 64);
        let retained = Cell::new(0);
        let mut bytes = Vec::new();
        let settings = settings(8, Subsampling::Yuv444, true)
            .with_siting(zencodec::frame::ChromaSiting::Unknown);
        let native = OwnedEncoder {
            native: zenavif::media::Av1VideoEncoder::new(settings.clone(), TB).unwrap(),
            live: live.clone(),
            corrupt_epoch: false,
        };
        let mut operation = StagedEncode::new(
            native,
            ExactMp4(zenmp4::Mp4Muxer::new()),
            Forward(Writer {
                bytes: &mut bytes,
                live: live.clone(),
                calls: writes.clone(),
                fail: false,
            }),
            |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
                let Question::FrameRoute { facts, choices, .. } = q else {
                    panic!()
                };
                let i = calls.get();
                assert_eq!(executions.get(), i);
                assert_eq!(
                    facts.input,
                    FrameFacts::new(inputs[i].interpretation(), inputs[i].timing())
                );
                assert!(!facts.output.output_committed);
                let choice = choices.first().unwrap();
                let offer = choice.value();
                assert!(offer.destination.unwrap().exact_timing);
                // The generic VideoEncoder trait does not yet qualify fidelity.
                assert_eq!(offer.fidelity, None);
                let conversion = offer.conversion.unwrap();
                assert!(!conversion.preserves_samples);
                assert!(!conversion.preserves_extended_range);
                assert_eq!(
                    conversion.encoder_input.color.resolved,
                    Some(zenpixels::Cicp::new(1, 13, 0, true))
                );
                assert_eq!(
                    conversion.admitted_bytes,
                    converted.reserved_bytes() - retained.get()
                );
                calls.set(i + 1);
                Ok(Decision::UsePreset)
            },
            encoded.clone(),
            &Unstoppable,
        )
        .unwrap()
        .with_policy_set(
            PolicySet::new()
                .allow_assumed_chroma_siting(true)
                .require_extended_range(require_range),
        );
        let mut expected = Vec::new();
        for input in &inputs {
            let result = coupled_route::push(
                &mut operation,
                input,
                &converted,
                &executions,
                &mut expected,
            );
            if require_range {
                let error = result.unwrap_err();
                assert!(zencodec::find_cause::<PolicyFailure>(&*error).is_some());
                assert!(operation.is_failed());
                assert_eq!(executions.get(), 0);
                assert_eq!(writes.get(), 0);
                break;
            }
            result.unwrap();
            assert_eq!(writes.get(), 0);
            retained.set(converted.reserved_bytes());
        }
        if require_range {
            drop(operation);
            assert!(bytes.is_empty());
        } else {
            let completed = operation.finish(&Unstoppable).unwrap();
            assert_eq!(calls.get(), inputs.len());
            assert_eq!(executions.get(), inputs.len());
            assert_eq!(completed.stats.emitted as usize, bytes.len());
            for record in completed.report.decisions() {
                let Some(Selection::FrameRoute(offer)) = record.selected else {
                    panic!()
                };
                assert!(!offer.conversion.unwrap().preserves_samples);
                assert!(offer.destination.unwrap().exact_timing);
            }
            oracle_container(
                &root(),
                "owned-converted-rgb",
                "mp4",
                &settings,
                &bytes,
                &expected,
            );
        }
        assert!(!live.get());
        drop(expected);
        assert_eq!(converted.reserved_bytes(), 0);
        assert_eq!(encoded.reserved_bytes(), 0);
    }
}
