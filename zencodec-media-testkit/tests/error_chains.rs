//! Concrete causes survive the public pump, mapping and erasure boundaries.
use enough::Unstoppable;
use std::{error::Error, sync::Arc};
use zencodec::{At, find_cause, time::TimeBase};
use zencodec_media::{
    adapt::AdaptError,
    frame::{MapError, MappedPlanes, Picture},
    packet::Packet,
    session::{
        CodecConfig, ErasedError, FusedDecoder, PumpError, SessionError, VideoDecoder,
        drain_decoder, erase_video_decoder, send_packet_until_accepted,
    },
    track::{CodecId, CodecTag, VideoTrackInfo},
};
use zencodec_media_testkit::toy::{ToyDecoder, ToyError, toy_payload};

#[derive(Debug)]
struct NativeFailure {
    code: u16,
    identity: Arc<()>,
}

impl std::fmt::Display for NativeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native refusal {}", self.code)
    }
}
impl Error for NativeFailure {}

fn traced(code: u16, identity: &Arc<()>) -> At<NativeFailure> {
    At::wrap(NativeFailure {
        code,
        identity: identity.clone(),
    })
    .at()
}

fn assert_preserved(error: &(dyn Error + 'static), code: u16, identity: &Arc<()>) {
    let trace = find_cause::<At<NativeFailure>>(error).expect("located native cause");
    let native = find_cause::<NativeFailure>(error).expect("concrete native cause");
    assert_eq!(native.code, code);
    assert!(Arc::ptr_eq(&native.identity, identity));
    assert!(core::ptr::eq(trace.error(), native));
    assert!(format!("{trace:?}").contains("error_chains.rs"));
}

fn configured() -> ToyDecoder {
    let mut decoder = ToyDecoder::new(1);
    let config = CodecConfig::new(
        CodecId::Other(CodecTag::new("toy")),
        TimeBase::new(1, 1).unwrap(),
    )
    .with_video(VideoTrackInfo::new(2, 2));
    decoder.configure(&config, &Unstoppable).unwrap();
    decoder
}

fn upstream<D: VideoDecoder + ?Sized>(decoder: &mut D) {
    let mut malformed = Packet::from_vec(vec![4, 0]).unwrap();
    let failure = send_packet_until_accepted(decoder, &mut malformed, &Unstoppable, |_| {
        Ok::<(), core::convert::Infallible>(())
    })
    .unwrap_err();
    assert!(matches!(failure, PumpError::Upstream(_)));
    assert!(matches!(
        find_cause::<ToyError>(&failure),
        Some(ToyError::BadPayload)
    ));
    assert!(matches!(
        decoder.receive(&Unstoppable),
        Err(SessionError::Finished)
    ));
}

#[test]
fn pump_keeps_decoder_causes_after_erasure() {
    upstream(&mut FusedDecoder::new(configured()));
    upstream(&mut *erase_video_decoder(configured()));
}

fn downstream<D: VideoDecoder + ?Sized>(decoder: &mut D) {
    let identity = Arc::new(());
    let mut packet = Packet::from_vec(toy_payload(&[&[1, 2, 3, 4]], false)).unwrap();
    decoder.send(&mut packet, &Unstoppable).unwrap();
    let failure = drain_decoder(decoder, &Unstoppable, |_| Err(traced(71, &identity))).unwrap_err();
    assert!(matches!(failure, PumpError::Downstream(_)));
    assert_preserved(&failure, 71, &identity);
    let erased = ErasedError::new(Box::new(failure));
    assert_preserved(&erased, 71, &identity);
}

#[test]
fn pump_keeps_downstream_trace_and_identity() {
    downstream(&mut FusedDecoder::new(configured()));
    downstream(&mut *erase_video_decoder(configured()));
}

struct RefusedPicture(Arc<()>);
impl Picture for RefusedPicture {
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        Err(MapError::Backend(Box::new(traced(93, &self.0))))
    }
}

#[test]
fn mapping_cause_survives_adapter_and_erasure() {
    let identity = Arc::new(());
    let picture = RefusedPicture(identity.clone());
    let failure = match picture.map() {
        Err(failure) => failure,
        Ok(_) => panic!("authored mapping refusal"),
    };
    assert_preserved(&failure, 93, &identity);
    let adapted: AdaptError<ToyError, ToyError> = AdaptError::Map(failure);
    assert_preserved(&adapted, 93, &identity);
    let erased = ErasedError::new(Box::new(adapted));
    assert_preserved(&erased, 93, &identity);
    assert!(MapError::Unavailable.source().is_none());
}
