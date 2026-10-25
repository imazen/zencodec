use std::{
    convert::Infallible,
    error::Error,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use enough::{Stop, StopReason, Unstoppable};
use zenavif::media::{Av1SessionError, Av1VideoDecoder};
use zencodec::{
    At,
    operation::{Decision, DecisionCode, Decline, FrameFacts, PolicySet, Question},
    time::{TimeBase, Timestamp},
};
use zencodec_media::{
    frame::{Frame, Picture, Subsampling},
    packet::Packet,
    policy::{PolicyDecoder, Stage},
    session::{CodecConfig, Receive, SendStatus, SessionError, VideoDecoder, VideoStreamInfo},
    track::{CodecId, VideoTrackInfo},
};
use zenpixels::Cicp;

use crate::fixtures::*;

#[derive(Default)]
struct Observed {
    pictures: Mutex<Vec<Weak<dyn Picture>>>,
    dropped: AtomicBool,
}
/// Observes the actual native result without replacing its storage or facts.
struct Watched {
    decoder: Av1VideoDecoder,
    observed: Arc<Observed>,
}
impl Drop for Watched {
    fn drop(&mut self) {
        self.observed.dropped.store(true, Ordering::SeqCst);
    }
}
impl VideoDecoder for Watched {
    type Error = Av1SessionError;
    fn configure(
        &mut self,
        config: &CodecConfig,
        stop: &dyn Stop,
    ) -> Result<(), SessionError<Self::Error>> {
        self.decoder.configure(config, stop)
    }
    fn send(
        &mut self,
        packet: &mut Packet,
        stop: &dyn Stop,
    ) -> Result<SendStatus, SessionError<Self::Error>> {
        self.decoder.send(packet, stop)
    }
    fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Frame>, SessionError<Self::Error>> {
        let output = self.decoder.receive(stop)?;
        if let Receive::Ready(frame) = &output {
            self.observed
                .pictures
                .lock()
                .unwrap()
                .push(Arc::downgrade(frame.picture()));
        }
        Ok(output)
    }
    fn end_input(&mut self) -> Result<(), SessionError<Self::Error>> {
        self.decoder.end_input()
    }
    fn reset(&mut self) -> Result<(), SessionError<Self::Error>> {
        self.decoder.reset()
    }
    fn stream_info(&self) -> Option<&VideoStreamInfo> {
        self.decoder.stream_info()
    }
}
fn watched(threads: u32) -> (Watched, Arc<Observed>) {
    let observed = Arc::new(Observed::default());
    let mut settings = rav1d_safe::Settings::default();
    settings.threads = threads;
    settings.max_frame_delay = 4;
    (
        Watched {
            decoder: Av1VideoDecoder::with_settings(settings).unwrap(),
            observed: Arc::clone(&observed),
        },
        observed,
    )
}

#[test]
fn approved_native_frames_reencode_exactly_under_independent_decoder() {
    let root = std::env::var_os("AV1_POLICY_ARTIFACTS")
        .expect("AV1_POLICY_ARTIFACTS must name a fresh persistent artifact directory");
    let root = std::path::Path::new(&root);
    std::fs::create_dir_all(root).unwrap();
    for bits in [8, 10, 12] {
        for (sub, identity) in [
            (Subsampling::Mono, false),
            (Subsampling::Yuv420, false),
            (Subsampling::Yuv422, false),
            (Subsampling::Yuv444, false),
            (Subsampling::Yuv444, true),
        ] {
            let s = settings(bits, sub, identity);
            let inputs: Vec<_> = (0..3).map(|i| frame(&s, i)).collect();
            let (config, packets) = encode(s.clone(), &inputs);
            let (native, observed) = watched(4);
            let mut seen = Vec::new();
            let mut decoder = PolicyDecoder::new(
                native,
                |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                    let Question::Frame {
                        interpretation,
                        timing,
                        choices,
                    } = question
                    else {
                        panic!("expected frame decision")
                    };
                    assert_eq!(interpretation.encoding.code_bits(), bits);
                    assert_eq!(interpretation.subsampling, sub);
                    assert_eq!(interpretation.roles, inputs[0].interpretation().roles);
                    assert_eq!(interpretation.color.bitstream, Some(s.color));
                    seen.push(FrameFacts::new(interpretation, timing));
                    Ok(Decision::choose(choices.first().unwrap()))
                },
            );
            let outputs = decode(&mut decoder, &config, &packets, &Unstoppable).unwrap();
            assert_eq!(outputs.len(), inputs.len());
            assert_eq!(decoder.report().decisions().count(), inputs.len());
            for ((output, input), owner) in outputs
                .iter()
                .zip(&inputs)
                .zip(observed.pictures.lock().unwrap().iter())
            {
                assert!(Arc::ptr_eq(output.picture(), &owner.upgrade().unwrap()));
                assert_eq!(samples(output), samples(input));
                assert_eq!(output.timing().pts, input.timing().pts);
            }
            decoder.reset().unwrap();
            drop(decoder);
            assert!(observed.dropped.load(Ordering::SeqCst));
            assert_eq!(seen.len(), inputs.len());
            // Re-encode the original native owners, after their decoder is gone.
            let (_, approved) = encode(s.clone(), &outputs);
            oracle(
                root,
                &format!("{bits}-{sub:?}-identity-{identity}"),
                &s,
                &approved,
                &inputs,
            );
            drop(outputs);
            assert!(
                observed
                    .pictures
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|p| p.upgrade().is_none())
            );
        }
    }
}

#[test]
fn queued_native_epochs_present_original_facts_to_policy() {
    let s = settings(10, Subsampling::Yuv420, false).with_color(Cicp::new(2, 2, 2, true));
    let inputs: Vec<_> = (0..7).map(|i| frame(&s, i)).collect();
    let (base, packets) = encode(s.clone(), &inputs);
    assert_eq!(packets.len(), inputs.len());
    let clocks = [TB, TimeBase::hz(90_000).unwrap()];
    let colors = [Cicp::new(1, 13, 1, true), Cicp::new(9, 16, 9, true)];
    for threads in [1, 4] {
        let (native, _) = watched(threads);
        let mut seen = Vec::new();
        let mut decoder = PolicyDecoder::new(
            native,
            |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Frame {
                    interpretation,
                    timing,
                    ..
                } = question
                else {
                    panic!("expected frame decision")
                };
                seen.push(FrameFacts::new(interpretation, timing));
                Ok(Decision::UsePreset)
            },
        );
        let mut outputs = Vec::new();
        for (i, packet) in packets.iter().enumerate() {
            let mut config = base.clone().with_epoch(i as u32);
            config.time_base = clocks[i % 2];
            config.video = Some(VideoTrackInfo::new(s.width, s.height).with_color(colors[i % 2]));
            decoder.configure(&config, &Unstoppable).unwrap();
            let mut packet = packet
                .clone()
                .with_pts(Timestamp::new(i as i64 * 40, config.time_base))
                .with_config_epoch(config.epoch)
                .with_ordinal(7)
                .with_input_offset(1000 + i as u64);
            while decoder.send(&mut packet, &Unstoppable).unwrap() != SendStatus::Accepted {
                let Receive::Ready(frame) = decoder.receive(&Unstoppable).unwrap() else {
                    panic!("decoder stalled")
                };
                outputs.push(frame);
            }
        }
        decoder
            .configure(&base.clone().with_epoch(99), &Unstoppable)
            .unwrap();
        decoder.end_input().unwrap();
        loop {
            match decoder.receive(&Unstoppable).unwrap() {
                Receive::Ready(frame) => outputs.push(frame),
                Receive::EndOfStream => break,
                other => panic!("drain stalled: {other:?}"),
            }
        }
        drop(decoder);
        assert_eq!(seen.len(), inputs.len());
        for (i, ((facts, output), input)) in seen.iter().zip(&outputs).zip(&inputs).enumerate() {
            assert_eq!(facts.config_epoch, i as u32);
            assert_eq!(facts.color.bitstream, Some(s.color));
            assert_eq!(facts.color.container, Some(colors[i % 2]));
            assert_eq!(facts.color.resolved, Some(colors[i % 2]));
            assert_eq!(
                facts.timing.pts,
                Some(Timestamp::new(i as i64 * 40, clocks[i % 2]))
            );
            assert_eq!(facts.timing.packet_ordinal, Some(7));
            assert_eq!(facts.timing.input_offset, Some(1000 + i as u64));
            assert_eq!(samples(output), samples(input));
        }
    }
}

#[test]
fn one_native_packet_with_multiple_presentations_asks_for_each() {
    let s = settings(8, Subsampling::Yuv420, false);
    let inputs: Vec<_> = (0..3).map(|i| frame(&s, i)).collect();
    let (config, packets) = encode(s, &inputs);
    let combined = Packet::from_vec(packets.iter().flat_map(|p| p.bytes()).copied().collect())
        .unwrap()
        .with_ordinal(7)
        .with_input_offset(1234);
    let mut seen = 0;
    let (native, _) = watched(4);
    let mut decoder = PolicyDecoder::new(
        native,
        |question: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Frame { timing, .. } = question else {
                panic!("expected frame decision")
            };
            assert_eq!(timing.packet_ordinal, Some(7));
            assert_eq!(timing.input_offset, Some(1234));
            seen += 1;
            Ok(Decision::UsePreset)
        },
    );
    let outputs = decode(&mut decoder, &config, &[combined], &Unstoppable).unwrap();
    drop(decoder);
    assert_eq!(seen, 3);
    assert_eq!(
        outputs.iter().map(samples).collect::<Vec<_>>(),
        inputs.iter().map(samples).collect::<Vec<_>>()
    );
}

#[derive(Debug)]
struct TenantError;
impl std::fmt::Display for TenantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private tenant rule")
    }
}
impl Error for TenantError {}
struct Cancel(AtomicBool);
impl Stop for Cancel {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.load(Ordering::SeqCst) {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}
fn cause<'a, E: Error + 'static>(mut error: &'a (dyn Error + 'static)) -> &'a E {
    loop {
        if let Some(found) = error.downcast_ref::<E>() {
            return found;
        }
        error = error
            .source()
            .expect("original cause must remain inspectable");
    }
}

#[test]
fn native_refusal_stale_choice_error_cancel_and_unwind_release_held_picture() {
    let s = settings(10, Subsampling::Yuv420, false);
    let inputs: Vec<_> = (0..3).map(|i| frame(&s, i)).collect();
    let (config, packets) = encode(s, &inputs);
    for mode in 0..5 {
        let (native, observed) = watched(4);
        let stop = Cancel(AtomicBool::new(false));
        let mut token = None;
        let mut decoder = PolicyDecoder::new(
            native,
            |question: Question<'_>| -> Result<Decision, At<TenantError>> {
                let Question::Frame { choices, .. } = question else {
                    panic!("expected frame decision")
                };
                let Some(first) = token else {
                    token = Some(choices.first().unwrap().id());
                    return Ok(Decision::UsePreset);
                };
                match mode {
                    0 => Ok(Decision::decline(Decline::ApplicationRule)),
                    1 => Ok(Decision::Choose(first)),
                    2 => Err(At::wrap(TenantError).at_named("private-policy-location")),
                    3 => {
                        stop.0.store(true, Ordering::SeqCst);
                        Ok(Decision::UsePreset)
                    }
                    _ => panic!("caught application callback panic"),
                }
            },
        );
        decoder.configure(&config, &stop).unwrap();
        for packet in &packets {
            assert_eq!(
                decoder.send(&mut packet.clone(), &stop).unwrap(),
                SendStatus::Accepted
            );
        }
        decoder.end_input().unwrap();
        let Receive::Ready(old) = decoder.receive(&stop).unwrap() else {
            panic!("first presentation missing")
        };
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decoder.receive(&stop)));
        if mode == 4 {
            assert!(result.is_err());
        } else {
            let SessionError::Codec(error) = result.unwrap().unwrap_err() else {
                panic!("missing policy error")
            };
            assert_eq!(error.stage(), Stage::Policy);
            assert_eq!(
                error.report().decisions().last().unwrap().code,
                [
                    DecisionCode::Declined,
                    DecisionCode::InvalidChoice,
                    DecisionCode::CallbackFailed,
                    DecisionCode::Stopped
                ][mode]
            );
            assert!(!format!("{error:?} {error}").contains("private"));
            if mode == 2 {
                assert!(cause::<At<TenantError>>(&error).frame_count() > 0);
            }
        }
        assert!(decoder.is_failed());
        assert!(observed.dropped.load(Ordering::SeqCst));
        assert!(matches!(
            decoder.receive(&Unstoppable),
            Err(SessionError::Finished)
        ));
        assert!(matches!(decoder.reset(), Err(SessionError::Finished)));
        let pictures = observed.pictures.lock().unwrap();
        assert_eq!(pictures.len(), 2);
        assert!(Arc::ptr_eq(old.picture(), &pictures[0].upgrade().unwrap()));
        assert!(
            pictures[1].upgrade().is_none(),
            "unpublished native frame leaked"
        );
        assert_eq!(samples(&old), samples(&inputs[0]));
        drop(old);
        assert!(pictures[0].upgrade().is_none());
    }
}

#[test]
fn native_error_keeps_rav1d_whereat_trace_through_policy_and_erasure() {
    fn check<D: VideoDecoder + ?Sized>(decoder: &mut D, observed: &Observed) {
        decoder
            .configure(&CodecConfig::new(CodecId::Av1, TB), &Unstoppable)
            .unwrap();
        let error = decoder
            .send(&mut Packet::from_vec(vec![0xff]).unwrap(), &Unstoppable)
            .unwrap_err();
        let trace = cause::<At<rav1d_safe::Error>>(&error);
        assert_eq!(trace.error(), &rav1d_safe::Error::InvalidData);
        assert!(trace.frame_count() > 0);
        assert!(observed.dropped.load(Ordering::SeqCst));
        assert!(matches!(
            decoder.receive(&Unstoppable),
            Err(SessionError::Finished)
        ));
    }
    let (native, observed) = watched(1);
    check(&mut PolicyDecoder::new(native, PolicySet::new()), &observed);
    let (native, observed) = watched(1);
    let mut erased =
        zencodec_media::session::erase_video_decoder(PolicyDecoder::new(native, PolicySet::new()));
    check(&mut *erased, &observed);
}
