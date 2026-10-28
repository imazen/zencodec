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
        _: &dyn zencodec::Pulse,
    ) -> Result<SendStatus, SessionError<NativeError>> {
        self.call(0)?;
        Ok(SendStatus::Accepted)
    }
    fn receive(
        &mut self,
        _: &dyn zencodec::Pulse,
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
    decoder.configure(&config, &NoPulse).unwrap();
    let mut packet = Packet::from_vec(toy_payload(&[&[1, 2, 3, 4]], false)).unwrap();
    decoder.send(&mut packet, &NoPulse).unwrap();
    decoder.end_input().unwrap();
    let Receive::Ready(frame) = decoder.receive(&NoPulse).unwrap() else {
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
        0 => encoder.send(frame, &NoPulse).map(|_| ()),
        1 => encoder.receive(&NoPulse).map(|_| ()),
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
        drain_encoder(encoder, &NoPulse, |_| Ok(())).unwrap_err()
    } else {
        send_frame_until_accepted(encoder, frame, &NoPulse, |_| Ok(())).unwrap_err()
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
                .send(&frame, &zencodec::how_far::StopOnly::borrowed(&Cancel))
                .map(|_| ())
        } else {
            encoder
                .receive(&zencodec::how_far::StopOnly::borrowed(&Cancel))
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
    decoder.configure(&config, &NoPulse).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decoder.receive(&zencodec::how_far::StopOnly::borrowed(&PanicStop))
    }));
    assert!(result.is_err());
    assert!(matches!(
        decoder.receive(&NoPulse),
        Err(SessionError::Finished)
    ));
    decoder.reset().unwrap();
    assert!(matches!(decoder.receive(&NoPulse), Ok(Receive::NeedInput)));
}
