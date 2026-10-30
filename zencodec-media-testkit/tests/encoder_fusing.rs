//! Terminal-state enforcement is independent of a backend's own error handling.
use enough::{Stop, StopReason};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec::how_far::NoPulse;
use zencodec::time::TimeBase;
use zencodec_media::{
    frame::Frame,
    packet::Packet,
    session::{
        CodecConfig, FusedEncoder, PumpError, Receive, SendStatus, SessionError, VideoDecoder,
        VideoEncoder, drain_encoder, erase_video_encoder, send_frame_until_accepted,
    },
    track::{CodecId, CodecTag, VideoTrackInfo},
};
use zencodec_media_testkit::toy::{ToyDecoder, toy_payload};

// A backend with an independently owned job control. It also verifies that
// wrappers and pumps preserve the caller's absence/presence of per-call control.
struct ControlledEncoder {
    inner: zencodec_media_testkit::toy::ToyEncoder,
    owned: zencodec::SharedPulse,
    expect_none: bool,
    retained: Option<Arc<std::sync::Mutex<Option<zencodec::SharedPulse>>>>,
}
impl ControlledEncoder {
    fn check(
        &self,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<zencodec_media_testkit::toy::ToyError>> {
        assert_eq!(pulse.is_none(), self.expect_none);
        self.owned.check()?;
        pulse.unwrap_or(&NoPulse).check()?;
        Ok(())
    }
}
impl VideoEncoder for ControlledEncoder {
    type Error = zencodec_media_testkit::toy::ToyError;
    fn send(
        &mut self,
        frame: &Frame,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<Self::Error>> {
        self.check(pulse)?;
        if let Some(retained) = &self.retained {
            use zencodec::how_far::{Execution, PhaseSpec, Total};
            let report = pulse.expect("reporting control must reach the backend");
            report.set_total(Total::Exact(5)).unwrap();
            let children = report
                .split(
                    Execution::Sequence,
                    &[PhaseSpec::new("encode", 1, Total::Unknown)],
                )
                .unwrap();
            assert_eq!(children.len(), 1);
            children[0].check()?;
            report.record_issue(zencodec::how_far::PlanError::Unsupported);
            report.advance(2);
            *retained.lock().unwrap() = Some(report.share().unwrap());
        }
        self.inner.send(frame, pulse)
    }
    fn receive(
        &mut self,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Packet>, SessionError<Self::Error>> {
        self.check(pulse)?;
        self.inner.receive(pulse)
    }
    fn end_input(&mut self) -> Result<(), SessionError<Self::Error>> {
        self.owned.check()?;
        self.inner.end_input()
    }
    fn output_config(&self) -> Option<&CodecConfig> {
        self.inner.output_config()
    }
}

fn controlled_encoder(owned: zencodec::SharedPulse, expect_none: bool) -> ControlledEncoder {
    ControlledEncoder {
        inner: zencodec_media_testkit::toy::ToyEncoder::new(
            2,
            2,
            TimeBase::new(1, 1).unwrap(),
            0,
            1,
        ),
        owned,
        expect_none,
        retained: None,
    }
}

fn pump_one<E: VideoEncoder + ?Sized>(
    encoder: &mut E,
    pulse: Option<&dyn zencodec::Pulse>,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    let source = frame();
    let mut timing = source.timing();
    timing.pts = Some(zencodec::time::Timestamp::new(
        0,
        TimeBase::new(1, 1).unwrap(),
    ));
    let timed = source.represent(timing, source.presentation());
    send_frame_until_accepted(encoder, &timed, pulse, |packet| {
        bytes.extend_from_slice(packet.bytes());
        Ok::<(), core::convert::Infallible>(())
    })
    .unwrap();
    drain_encoder(encoder, pulse, |packet| {
        bytes.extend_from_slice(packet.bytes());
        Ok::<(), core::convert::Infallible>(())
    })
    .unwrap();
    bytes
}

#[test]
fn none_and_explicit_noop_keep_pixels_and_original_option_through_typed_erased_pumps() {
    let expected = toy_payload(&[&[1, 2, 3, 4]], false);
    for pulse in [None, Some(&NoPulse as &dyn zencodec::Pulse)] {
        let mut typed = FusedEncoder::new(controlled_encoder(
            zencodec::SharedPulse::default(),
            pulse.is_none(),
        ));
        assert_eq!(pump_one(&mut typed, pulse), expected);
        let mut erased = erase_video_encoder(controlled_encoder(
            zencodec::SharedPulse::default(),
            pulse.is_none(),
        ));
        assert_eq!(pump_one(&mut erased, pulse), expected);
    }
}

struct JobStop(Arc<std::sync::atomic::AtomicBool>);
impl Stop for JobStop {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.load(Ordering::SeqCst) {
            Err(StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}

fn check_owned_stop<E: VideoEncoder + ?Sized>(
    encoder: &mut E,
    stopped: &std::sync::atomic::AtomicBool,
    pulse: Option<&dyn zencodec::Pulse>,
) {
    stopped.store(true, Ordering::SeqCst);
    let failure = send_frame_until_accepted(encoder, &frame(), pulse, |_| {
        Ok::<(), core::convert::Infallible>(())
    })
    .unwrap_err();
    assert!(matches!(
        failure,
        PumpError::Upstream(SessionError::Cancelled(StopReason::TimedOut))
    ));
    stopped.store(false, Ordering::SeqCst);
    assert!(matches!(encoder.receive(None), Err(SessionError::Finished)));
}

#[test]
fn none_or_explicit_noop_cannot_disable_retained_job_stop_or_reopen_failed_sessions() {
    for pulse in [None, Some(&NoPulse as &dyn zencodec::Pulse)] {
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let make = || {
            controlled_encoder(
                zencodec::SharedPulse::new(zencodec::how_far::StopOnly::new(JobStop(
                    stopped.clone(),
                ))),
                pulse.is_none(),
            )
        };
        let mut typed = FusedEncoder::new(make());
        check_owned_stop(&mut typed, &stopped, pulse);
        let mut erased = erase_video_encoder(make());
        check_owned_stop(&mut erased, &stopped, pulse);
    }
}

#[derive(Default)]
struct Observed {
    units: AtomicUsize,
    totals: AtomicUsize,
    phases: AtomicUsize,
    issues: AtomicUsize,
    stopped: std::sync::atomic::AtomicBool,
}
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
impl zencodec::how_far::Report for Meter {
    fn advance(&self, n: u64) {
        self.0.units.fetch_add(n as usize, Ordering::SeqCst);
    }
}
impl zencodec::Pulse for Meter {
    fn set_total(
        &self,
        total: zencodec::how_far::Total,
    ) -> Result<(), zencodec::how_far::PlanError> {
        assert_eq!(total, zencodec::how_far::Total::Exact(5));
        self.0.totals.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn record_issue(&self, _: zencodec::how_far::PlanError) {
        self.0.issues.fetch_add(1, Ordering::SeqCst);
    }
    fn split(
        &self,
        execution: zencodec::how_far::Execution,
        parts: &[zencodec::how_far::PhaseSpec<'_>],
    ) -> Result<Vec<zencodec::how_far::Child<'_>>, zencodec::how_far::PlanError> {
        self.0.phases.fetch_add(1, Ordering::SeqCst);
        zencodec::Pulse::split(&NoPulse, execution, parts)
    }
}

#[test]
fn some_retains_phase_progress_issues_and_share_after_erased_pump_and_caller_drop() {
    use zencodec::{Pulse, how_far::Report};
    let observed = Arc::new(Observed::default());
    let retained = Arc::new(std::sync::Mutex::new(None));
    let pulse = zencodec::SharedPulse::new(Meter(observed.clone()));
    let mut native = controlled_encoder(zencodec::SharedPulse::default(), false);
    native.retained = Some(retained.clone());
    let mut encoder = erase_video_encoder(native);
    assert_eq!(
        pump_one(&mut encoder, Some(&pulse)),
        toy_payload(&[&[1, 2, 3, 4]], false)
    );
    drop(encoder);
    drop(pulse);
    let shared = retained
        .lock()
        .unwrap()
        .take()
        .expect("backend retained the caller's reporting control");
    shared.advance(3);
    assert_eq!(observed.units.load(Ordering::SeqCst), 5);
    assert_eq!(observed.totals.load(Ordering::SeqCst), 1);
    assert_eq!(observed.phases.load(Ordering::SeqCst), 1);
    assert_eq!(observed.issues.load(Ordering::SeqCst), 1);
    observed.stopped.store(true, Ordering::SeqCst);
    assert_eq!(shared.check(), Err(StopReason::TimedOut));
    assert_eq!(shared.share().unwrap().check(), Err(StopReason::TimedOut));
}

#[derive(Debug)]
struct NativeError(u8);
impl core::fmt::Display for NativeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "native refusal {}", self.0)
    }
}
impl core::error::Error for NativeError {}

struct Loose {
    fail_at: u8,
    calls: Arc<AtomicUsize>,
    panic: bool,
}
impl Loose {
    fn call(&self, method: u8) -> Result<(), SessionError<NativeError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_at == method {
            assert!(!self.panic, "authored backend panic");
            Err(SessionError::Codec(NativeError(method)))
        } else {
            Ok(())
        }
    }
}
impl VideoEncoder for Loose {
    type Error = NativeError;
    fn send(
        &mut self,
        _: &Frame,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<NativeError>> {
        self.call(0)?;
        Ok(SendStatus::Accepted)
    }
    fn receive(
        &mut self,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Packet>, SessionError<NativeError>> {
        self.call(1)?;
        Ok(Receive::NeedInput)
    }
    fn end_input(&mut self) -> Result<(), SessionError<NativeError>> {
        self.call(2)
    }
    fn output_config(&self) -> Option<&CodecConfig> {
        None
    }
}

fn frame() -> Frame {
    let mut decoder = ToyDecoder::new(1);
    let config = CodecConfig::new(
        CodecId::Other(CodecTag::new("toy")),
        TimeBase::new(1, 1).unwrap(),
    )
    .with_video(VideoTrackInfo::new(2, 2));
    decoder.configure(&config, Some(&NoPulse)).unwrap();
    let mut packet = Packet::from_vec(toy_payload(&[&[1, 2, 3, 4]], false)).unwrap();
    decoder.send(&mut packet, Some(&NoPulse)).unwrap();
    decoder.end_input().unwrap();
    let Receive::Ready(frame) = decoder.receive(Some(&NoPulse)).unwrap() else {
        panic!("frame fixture")
    };
    frame
}

fn invoke<E: VideoEncoder + ?Sized>(
    encoder: &mut E,
    frame: &Frame,
    method: u8,
) -> Result<(), SessionError<E::Error>> {
    match method {
        0 => encoder.send(frame, Some(&NoPulse)).map(|_| ()),
        1 => encoder.receive(Some(&NoPulse)).map(|_| ()),
        _ => encoder.end_input(),
    }
}

fn assert_terminal<E: VideoEncoder + ?Sized>(encoder: &mut E, frame: &Frame, calls: &AtomicUsize) {
    let before = calls.load(Ordering::SeqCst);
    for method in 0..3 {
        assert!(matches!(
            invoke(encoder, frame, method),
            Err(SessionError::Finished)
        ));
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "failed session re-entered the backend"
    );
}

#[test]
fn every_typed_and_erased_work_method_is_terminal_after_native_failure() {
    let frame = frame();
    for method in 0..3 {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut typed = FusedEncoder::new(Loose {
            fail_at: method,
            calls: calls.clone(),
            panic: false,
        });
        let failure = invoke(&mut typed, &frame, method).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<NativeError>(&failure).unwrap().0,
            method
        );
        assert_terminal(&mut typed, &frame, &calls);
        let mut erased = erase_video_encoder(Loose {
            fail_at: method,
            calls: calls.clone(),
            panic: false,
        });
        let failure = invoke(&mut erased, &frame, method).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<NativeError>(&failure).unwrap().0,
            method
        );
        assert_terminal(&mut erased, &frame, &calls);
    }
}

fn pump_failure<E: VideoEncoder + ?Sized>(
    encoder: &mut E,
    frame: &Frame,
    method: u8,
) -> PumpError<SessionError<E::Error>, core::convert::Infallible> {
    if method == 2 {
        drain_encoder(encoder, Some(&NoPulse), |_| Ok(())).unwrap_err()
    } else {
        send_frame_until_accepted(encoder, frame, Some(&NoPulse), |_| Ok(())).unwrap_err()
    }
}

#[test]
fn pump_keeps_native_causes_through_typed_and_erased_encoders() {
    let frame = frame();
    for method in 0..3 {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut typed = FusedEncoder::new(Loose {
            fail_at: method,
            calls: calls.clone(),
            panic: false,
        });
        let failure = pump_failure(&mut typed, &frame, method);
        assert!(matches!(failure, PumpError::Upstream(_)));
        assert_eq!(
            zencodec::find_cause::<NativeError>(&failure).unwrap().0,
            method
        );
        assert_terminal(&mut typed, &frame, &calls);

        let mut erased = erase_video_encoder(Loose {
            fail_at: method,
            calls: calls.clone(),
            panic: false,
        });
        let failure = pump_failure(&mut erased, &frame, method);
        assert!(matches!(failure, PumpError::Upstream(_)));
        assert_eq!(
            zencodec::find_cause::<NativeError>(&failure).unwrap().0,
            method
        );
        assert_terminal(&mut erased, &frame, &calls);
    }
}

struct Cancel;
impl Stop for Cancel {
    fn check(&self) -> Result<(), StopReason> {
        Err(StopReason::Cancelled)
    }
}

#[test]
fn cancellation_is_checked_before_entering_an_uncooperative_encoder() {
    let frame = frame();
    for send in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut encoder = erase_video_encoder(Loose {
            fail_at: 9,
            calls: calls.clone(),
            panic: false,
        });
        let result = if send {
            encoder
                .send(
                    &frame,
                    Some(&zencodec::how_far::StopOnly::borrowed(&Cancel)),
                )
                .map(|_| ())
        } else {
            encoder
                .receive(Some(&zencodec::how_far::StopOnly::borrowed(&Cancel)))
                .map(|_| ())
        };
        assert!(matches!(result, Err(SessionError::Cancelled(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_terminal(&mut encoder, &frame, &calls);
    }
}

#[test]
fn catching_backend_unwind_does_not_reopen_the_encoder() {
    let frame = frame();
    for method in 0..3 {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut encoder = FusedEncoder::new(Loose {
            fail_at: method,
            calls: calls.clone(),
            panic: true,
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            invoke(&mut encoder, &frame, method)
        }));
        assert!(result.is_err());
        assert_terminal(&mut encoder, &frame, &calls);
    }
}

#[test]
fn a_panicking_stop_poisoned_decoder_reopens_only_after_reset() {
    struct PanicStop;
    impl Stop for PanicStop {
        fn check(&self) -> Result<(), StopReason> {
            panic!("authored stop panic")
        }
    }
    let mut decoder = zencodec_media::session::erase_video_decoder(ToyDecoder::new(1));
    let config = CodecConfig::new(
        CodecId::Other(CodecTag::new("toy")),
        TimeBase::new(1, 1).unwrap(),
    )
    .with_video(VideoTrackInfo::new(2, 2));
    decoder.configure(&config, Some(&NoPulse)).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decoder.receive(Some(&zencodec::how_far::StopOnly::borrowed(&PanicStop)))
    }));
    assert!(result.is_err());
    assert!(matches!(
        decoder.receive(Some(&NoPulse)),
        Err(SessionError::Finished)
    ));
    decoder.reset().unwrap();
    assert!(matches!(
        decoder.receive(Some(&NoPulse)),
        Ok(Receive::NeedInput)
    ));
}

// Deliberately ignores per-call Pulse, including a stop raised during receive.
struct BoundaryCodec {
    stopped: Arc<std::sync::atomic::AtomicBool>,
    calls: usize,
    cancel_receive: bool,
    fail_receive: bool,
}
impl BoundaryCodec {
    fn receive_boundary(&mut self) -> Result<(), SessionError<std::io::Error>> {
        self.calls += 1;
        if self.cancel_receive {
            self.stopped.store(true, Ordering::SeqCst);
        }
        if self.fail_receive {
            return Err(SessionError::Codec(std::io::Error::other("native receive")));
        }
        Ok(())
    }
}
impl VideoEncoder for BoundaryCodec {
    type Error = std::io::Error;
    fn send(
        &mut self,
        _: &Frame,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<Self::Error>> {
        self.calls += 1;
        Ok(SendStatus::Accepted)
    }
    fn receive(
        &mut self,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Packet>, SessionError<Self::Error>> {
        self.receive_boundary()?;
        Ok(Receive::Ready(Packet::from_vec(vec![1]).unwrap()))
    }
    fn end_input(&mut self) -> Result<(), SessionError<Self::Error>> {
        self.calls += 1;
        Ok(())
    }
    fn output_config(&self) -> Option<&CodecConfig> {
        None
    }
}
impl VideoDecoder for BoundaryCodec {
    type Error = std::io::Error;
    fn configure(
        &mut self,
        _: &CodecConfig,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<Self::Error>> {
        Ok(())
    }
    fn send(
        &mut self,
        _: &mut Packet,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<Self::Error>> {
        self.calls += 1;
        Ok(SendStatus::Accepted)
    }
    fn receive(
        &mut self,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Frame>, SessionError<Self::Error>> {
        self.receive_boundary()?;
        Ok(Receive::Ready(frame()))
    }
    fn end_input(&mut self) -> Result<(), SessionError<Self::Error>> {
        self.calls += 1;
        Ok(())
    }
    fn reset(&mut self) -> Result<(), SessionError<Self::Error>> {
        Ok(())
    }
    fn info(&self) -> Option<&zencodec_media::session::VideoStreamInfo> {
        None
    }
}

#[test]
fn pumps_guard_native_and_publication_boundaries_independently() {
    use zencodec::how_far::StopOnly;
    use zencodec_media::session::{drain_decoder, send_packet_until_accepted};
    for method in 0..4 {
        // Pre-cancel, stop inside receive, stop in successful downstream,
        // and a native failure simultaneous with cancellation.
        for scenario in 0..4 {
            let stopped = Arc::new(std::sync::atomic::AtomicBool::new(scenario == 0));
            let pulse = StopOnly::new(JobStop(stopped.clone()));
            let mut codec = BoundaryCodec {
                stopped: stopped.clone(),
                calls: 0,
                cancel_receive: scenario == 1 || scenario == 3,
                fail_receive: scenario == 3,
            };
            let mut published = 0;
            let mut deliver = || {
                published += 1;
                stopped.store(true, Ordering::SeqCst);
                // Bound a broken pump without spinning forever.
                if published > 1 {
                    Err("published twice")
                } else {
                    Ok(())
                }
            };
            let result = match method {
                0 => send_frame_until_accepted(&mut codec, &frame(), Some(&pulse), |_| deliver()),
                1 => drain_encoder(&mut codec, Some(&pulse), |_| deliver()),
                2 => send_packet_until_accepted(
                    &mut codec,
                    &mut Packet::from_vec(vec![1]).unwrap(),
                    Some(&pulse),
                    |_| deliver(),
                ),
                _ => drain_decoder(&mut codec, Some(&pulse), |_| deliver()),
            };
            if scenario == 3 {
                assert!(matches!(
                    result,
                    Err(PumpError::Upstream(SessionError::Codec(_)))
                ));
            } else {
                assert!(
                    matches!(
                        result,
                        Err(PumpError::Upstream(SessionError::Cancelled(
                            StopReason::TimedOut
                        )))
                    ),
                    "method={method} scenario={scenario}: {result:?}"
                );
            }
            assert_eq!(published, usize::from(scenario == 2));
            assert_eq!(codec.calls, if scenario == 0 { 0 } else { 2 });
        }
    }
}
