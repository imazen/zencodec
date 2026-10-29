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
    change_fidelity: bool,
    accepted: bool,
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
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<Self::Error>> {
        let status = self.native.send(frame, stop)?;
        self.accepted |= status == SendStatus::Accepted;
        Ok(status)
    }
    fn receive(
        &mut self,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Packet>, SessionError<Self::Error>> {
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
    fn output_fidelity(&self) -> Option<zencodec::encode::Fidelity> {
        // This witness constructs only quantizer-zero native encoders, qualified
        // against independent decoded samples. Fault injection changes evidence
        // without changing configuration bytes.
        if self.change_fidelity && self.accepted {
            None
        } else {
            Some(zencodec::encode::Fidelity::Lossless)
        }
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
    let native =
        zenavif::media::Av1VideoEncoder::new(settings(8, Subsampling::Yuv420, false), TB).unwrap();
    assert_eq!(
        native.output_fidelity(),
        None,
        "unadapted encoder must remain unqualified"
    );
    OwnedEncoder {
        native,
        live: live.clone(),
        corrupt_epoch: false,
        change_fidelity: false,
        accepted: false,
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
            Some(&NoPulse),
        )
        .unwrap();
        let settings = settings(8, Subsampling::Yuv420, false);
        let frames: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
        for (index, input) in frames.iter().enumerate() {
            if index == 0 {
                operation.push(input.clone(), Some(&NoPulse)).unwrap();
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
                        Some(&NoPulse),
                    )
                    .unwrap();
                assert_eq!(executions.get(), 1);
            }
            assert_eq!(calls.get(), 0);
        }
        let result = operation.finish(Some(&NoPulse));
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
    for mode in [
        "policy", "drift", "unwind", "budget", "cancel", "epoch", "fidelity",
    ] {
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
        native.change_fidelity = mode == "fidelity";
        let mut operation = StagedEncode::new(
            Box::new(zencodec_media::session::VideoEncoderShim(
                zencodec_media::session::FusedEncoder::new(native),
            )),
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
            Some(&NoPulse),
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
                Some(&zencodec::how_far::StopOnly::borrowed(&control)),
            )
        }));
        if mode == "budget" || mode == "epoch" {
            // Native encoders may delay their first packet until draining.
            if result.unwrap().is_ok() {
                assert!(operation.finish(Some(&NoPulse)).is_err());
            } else {
                assert!(operation.is_failed());
            }
        } else {
            if mode == "unwind" {
                assert!(result.is_err());
            } else {
                let error = result.unwrap().unwrap_err();
                if mode == "fidelity" {
                    assert_eq!(
                        zencodec::find_cause::<zencodec_media::staged::Refusal>(&error),
                        Some(&zencodec_media::staged::Refusal::ConfigurationChanged)
                    );
                }
                if mode == "cancel" {
                    use zencodec::CodecErrorExt;
                    assert_eq!(
                        error.error_category(),
                        Some(zencodec::ErrorCategory::Stopped(
                            enough::StopReason::TimedOut
                        ))
                    );
                }
            }
            assert!(operation.is_failed());
            assert!(operation.push(input, Some(&NoPulse)).is_err());
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
#[allow(deprecated)] // Preserve the legacy-placement regression during migration.
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
            change_fidelity: false,
            accepted: false,
        };
        let mut operation = StagedEncode::new(
            Box::new(zencodec_media::session::VideoEncoderShim(
                zencodec_media::session::FusedEncoder::new(native),
            )),
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
                assert_eq!(offer.fidelity, Some(zencodec::encode::Fidelity::Lossless));
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
            Some(&NoPulse),
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
            let completed = operation.finish(Some(&NoPulse)).unwrap();
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

#[test]
fn staged_native_disk_spool_preserves_output_and_refuses_before_destination_effects() {
    use zencodec::operation::StagingKind;
    use zencodec_media::staging::{DiskLimits, Staging};
    let settings = settings(8, Subsampling::Yuv420, false);
    let frames: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
    let mut expected_bytes = None;
    for mode in [
        "memory",
        "disk",
        "decline",
        "disk_cap",
        "replay_cap",
        "readonly",
    ] {
        let live = Rc::new(Cell::new(false));
        let writes = Rc::new(Cell::new(0));
        let budget = StorageBudget::new(if mode == "replay_cap" { 0 } else { 1 << 20 }, 64);
        let mut bytes = Vec::new();
        let path = root().join(format!("spool-{mode}.scratch"));
        let prefix = b"preserved scratch prefix";
        let staging = if mode == "memory" {
            Staging::memory(budget.clone())
        } else {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(prefix).unwrap();
            if mode == "readonly" {
                drop(file);
                file = std::fs::File::open(&path).unwrap();
                file.seek(SeekFrom::End(0)).unwrap();
            }
            Staging::disk(
                file,
                DiskLimits::new(if mode == "disk_cap" { 0 } else { 1 << 20 }, 64, 16),
                budget.clone(),
            )
            .unwrap()
        };
        let mut operation = StagedEncode::with_staging(
            encoder(&live),
            ExactMp4(zenmp4::Mp4Muxer::new()),
            Forward(Writer {
                bytes: &mut bytes,
                live: live.clone(),
                calls: writes.clone(),
                fail: false,
            }),
            |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
                let Question::FrameRoute { choices, .. } = q else {
                    panic!()
                };
                let staging = choices.first().unwrap().value().staging.unwrap();
                assert_eq!(
                    staging.kind,
                    if mode == "memory" {
                        StagingKind::Memory
                    } else {
                        StagingKind::Disk
                    }
                );
                if mode == "decline" {
                    Ok(Decision::decline(Decline::NoPreferredChoice))
                } else {
                    Ok(Decision::UsePreset)
                }
            },
            staging,
            Some(&NoPulse),
        )
        .unwrap();
        let mut failure = None;
        for input in &frames {
            if let Err(error) = operation.push(input.clone(), Some(&NoPulse)) {
                failure = Some(error);
                break;
            }
            assert_eq!(writes.get(), 0);
            if mode != "memory" {
                assert_eq!(
                    budget.reserved_bytes(),
                    0,
                    "disk payloads accumulated in replay RAM"
                );
            }
        }
        let result = if let Some(error) = failure {
            drop(operation);
            Err(error)
        } else {
            operation.finish(Some(&NoPulse))
        };
        assert!(!live.get());
        assert_eq!(budget.reserved_bytes(), 0);
        match mode {
            "memory" => {
                result.unwrap();
                expected_bytes = Some(bytes);
            }
            "disk" => {
                result.unwrap();
                assert_eq!(&bytes, expected_bytes.as_ref().unwrap());
                oracle_container(
                    &root(),
                    "owned-disk-spooled",
                    "mp4",
                    &settings,
                    &bytes,
                    &frames,
                );
            }
            _ => {
                let error = result.unwrap_err();
                assert_eq!(writes.get(), 0);
                assert!(bytes.is_empty());
                match mode {
                    "decline" => assert!(
                        zencodec::find_cause::<zencodec::operation::PolicyFailure>(&error)
                            .is_some()
                    ),
                    "disk_cap" => assert_eq!(
                        zencodec::find_cause::<zencodec_media::staging::Refusal>(&error),
                        Some(&zencodec_media::staging::Refusal::Bytes)
                    ),
                    "replay_cap" => assert!(
                        zencodec::find_cause::<zencodec::storage::StorageError>(&error).is_some()
                    ),
                    "readonly" => assert!(zencodec::find_cause::<io::Error>(&error).is_some()),
                    _ => unreachable!(),
                }
            }
        }
        if mode != "memory" {
            let scratch = std::fs::read(path).unwrap();
            assert!(scratch.starts_with(prefix));
            if matches!(mode, "decline" | "disk_cap" | "readonly") {
                assert_eq!(scratch, prefix);
            }
        }
    }
}

#[test]
fn staged_native_late_spool_failure_retains_confirmed_output_progress() {
    use zencodec_media::staging::{DiskLimits, Staging};
    struct TruncateSpool<'a> {
        writer: Writer<'a>,
        scratch: std::fs::File,
    }
    impl Write for TruncateSpool<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            // Deliberate fault injection: ordinary callers must exclusively
            // own scratch. First replay packet is already held before publication.
            self.scratch.set_len(0)?;
            self.writer.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.writer.flush()
        }
    }
    let live = Rc::new(Cell::new(false));
    let calls = Rc::new(Cell::new(0));
    let mut bytes = Vec::new();
    let scratch = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(root().join("late-spool-fault.scratch"))
        .unwrap();
    let fault = scratch.try_clone().unwrap();
    let budget = StorageBudget::new(1 << 20, 64);
    let staging = Staging::disk(scratch, DiskLimits::new(1 << 20, 64, 16), budget.clone()).unwrap();
    let mut operation = StagedEncode::with_staging(
        encoder(&live),
        ExactMp4(zenmp4::Mp4Muxer::new()),
        Forward(TruncateSpool {
            writer: Writer {
                bytes: &mut bytes,
                live: live.clone(),
                calls: calls.clone(),
                fail: false,
            },
            scratch: fault,
        }),
        PolicySet::new(),
        staging,
        Some(&NoPulse),
    )
    .unwrap();
    let settings = settings(8, Subsampling::Yuv420, false);
    for i in 0..3 {
        operation.push(frame(&settings, i), Some(&NoPulse)).unwrap();
    }
    let error = operation.finish(Some(&NoPulse)).unwrap_err();
    let failure = zencodec::find_cause::<zencodec_media::staged::Failure>(&error).unwrap();
    assert_eq!(failure.stage, EncodeStage::Publish);
    assert!(!bytes.is_empty(), "fault must follow actual publication");
    assert_eq!(failure.emitted, bytes.len() as u64);
    assert_eq!(
        zencodec::find_cause::<io::Error>(&error).unwrap().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert!(!live.get());
    assert_eq!(budget.reserved_bytes(), 0);
}

#[test]
fn staged_native_schedule_requirements_use_the_owned_operation_guarantee() {
    use zencodec::operation::OutputSchedule;
    for (schedule, limit, succeeds) in [
        (OutputSchedule::FinishFirst, None, true),
        (OutputSchedule::Live, None, false),
        (OutputSchedule::FinishFirst, Some(0), false),
    ] {
        let live = Rc::new(Cell::new(false));
        let calls = Rc::new(Cell::new(0));
        let mut bytes = Vec::new();
        let mut policy = PolicySet::new().require_output_schedule(schedule);
        if let Some(limit) = limit {
            policy = policy.with_output_wait_limit(limit);
        }
        let mut operation = StagedEncode::new(
            encoder(&live),
            ExactMp4(zenmp4::Mp4Muxer::new()),
            Forward(Writer {
                bytes: &mut bytes,
                live: live.clone(),
                calls: calls.clone(),
                fail: false,
            }),
            policy,
            StorageBudget::new(1 << 20, 64),
            Some(&NoPulse),
        )
        .unwrap();
        let settings = settings(8, Subsampling::Yuv420, false);
        let input = frame(&settings, 0);
        let result = operation.push(input.clone(), Some(&NoPulse));
        assert_eq!(calls.get(), 0);
        if succeeds {
            result.unwrap();
            operation.finish(Some(&NoPulse)).unwrap();
            assert!(!live.get());
            oracle_container(
                &root(),
                "owned-schedule-required",
                "mp4",
                &settings,
                &bytes,
                &[input],
            );
        } else {
            let error = result.unwrap_err();
            assert!(zencodec::find_cause::<zencodec::operation::PolicyFailure>(&error).is_some());
            assert!(operation.is_failed());
            assert!(!live.get());
            drop(operation);
            assert!(bytes.is_empty());
        }
    }
}
