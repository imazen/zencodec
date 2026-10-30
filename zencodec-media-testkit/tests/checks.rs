//! Every conformance check passes on the toy reference implementations, and
//! catches a wrapper that breaks the rule it enforces.

use std::sync::Arc;

use zencodec::how_far::NoPulse;
use zencodec::time::{FrameDuration, TimeBase, Timestamp};
use zencodec_media::adapt::{CaptureFrame, Gray8};
use zencodec_media::container::{
    DemuxProgress, Demuxer, Input, InputRequest, MuxFinish, MuxHint, MuxTiming, Muxer, SeekMode,
    SeekScope, SeekTarget,
};
use zencodec_media::frame::{Frame, FrameTiming};
use zencodec_media::limits::MediaLimits;
use zencodec_media::packet::{Packet, RandomAccess};
use zencodec_media::session::{
    CodecConfig, Receive, SendStatus, SessionError, VideoDecoder, VideoEncoder, VideoStreamInfo,
};
use zencodec_media::track::{CodecId, CodecTag, TrackInfo, VideoTrackInfo};
use zencodec_media_testkit::toy::{
    ToyDecoder, ToyDemuxer, ToyEncoder, ToyError, ToyMuxer, toy_payload,
};
use zencodec_media_testkit::{
    check_animation_over_video, check_demuxer_contract, check_demuxer_seek,
    check_muxer_output_honesty, check_video_decoder_contract, check_video_encoder_contract,
};
use zenpixels::{PixelBuffer, PixelDescriptor};

const W: u32 = 5;
const H: u32 = 3;

fn tb() -> TimeBase {
    TimeBase::new(1001, 30000).unwrap()
}

fn gray(v: u8) -> Vec<u8> {
    vec![v; (W * H) as usize]
}

fn config() -> CodecConfig {
    CodecConfig::new(CodecId::Other(CodecTag::new("toy")), tb())
        .with_video(VideoTrackInfo::new(W, H))
}

fn track() -> TrackInfo {
    TrackInfo::video(
        0,
        CodecId::Other(CodecTag::new("toy")),
        tb(),
        VideoTrackInfo::new(W, H),
    )
    .build()
    .unwrap()
}

fn packet(payload: Vec<u8>, pts: i64, ra: bool) -> Packet {
    Packet::from_vec(payload)
        .unwrap()
        .with_track(0)
        .with_pts(Timestamp::new(pts, tb()))
        .with_random_access(if ra {
            RandomAccess::Yes
        } else {
            RandomAccess::No
        })
}

/// Packets with 0..3 pictures and a show-existing replay.
fn packets() -> Vec<Packet> {
    vec![
        packet(toy_payload(&[&gray(10), &gray(20)], false), 0, true),
        packet(toy_payload(&[&gray(30)], false), 1, false),
        packet(toy_payload(&[], true), 2, false),
        packet(
            toy_payload(&[&gray(40), &gray(50), &gray(60)], false),
            3,
            false,
        ),
        packet(toy_payload(&[], false), 4, false),
        packet(toy_payload(&[&gray(70)], false), 5, true),
    ]
}

/// One single-picture packet per frame, pts = index.
fn simple_packets(n: u32, keyint: u32) -> Vec<Packet> {
    (0..n)
        .map(|i| {
            packet(
                toy_payload(&[&gray(i as u8 * 9)], false),
                i64::from(i),
                i % keyint == 0,
            )
        })
        .collect()
}

fn toy_file(n: u32, keyint: u32, write_count: bool) -> Vec<u8> {
    let mut m = ToyMuxer::new(write_count);
    m.begin(&[track()], MuxHint::default()).unwrap();
    let mut out = Vec::new();
    for p in simple_packets(n, keyint) {
        m.write_packet(p, Some(&NoPulse)).unwrap();
        m.take_output(&mut out);
    }
    m.finish(Some(&NoPulse))
        .unwrap()
        .apply_to(&mut out)
        .unwrap();
    out
}

fn frames(n: u8) -> Vec<Frame> {
    let mut cap = Gray8::default();
    (0..n)
        .map(|i| {
            let mut b = PixelBuffer::new(W, H, PixelDescriptor::GRAY8_SRGB);
            for y in 0..H {
                for (x, px) in b.as_slice_mut().row_mut(y).iter_mut().enumerate() {
                    *px = i * 31 + y as u8 * 5 + x as u8;
                }
            }
            cap.capture(
                &b.as_slice(),
                &config(),
                FrameTiming::at(Timestamp::new(i64::from(i), tb())),
            )
            .unwrap()
        })
        .collect()
}

// ---------------------------------------------------- reference passes ---

#[test]
fn the_toy_decoder_passes() {
    check_video_decoder_contract(|| ToyDecoder::new(2), &config(), &packets()).unwrap();
    check_video_decoder_contract(|| ToyDecoder::new(8), &config(), &packets()).unwrap();
}

#[test]
fn the_toy_encoder_passes() {
    check_video_encoder_contract(|| ToyEncoder::new(W, H, tb(), 2, 3), &frames(7)).unwrap();
    check_video_encoder_contract(|| ToyEncoder::new(W, H, tb(), 0, 1), &frames(3)).unwrap();
}

#[test]
fn the_toy_demuxer_passes() {
    for write_count in [false, true] {
        let file = toy_file(9, 3, write_count);
        check_demuxer_contract(|| ToyDemuxer::new(MediaLimits::default()), &file).unwrap();
    }
}

#[test]
fn the_toy_demuxer_seeks() {
    let file = toy_file(12, 4, false);
    let at = |ticks: i64| {
        Timestamp::new(ticks, tb())
            .rescale(TimeBase::MILLIS, zencodec::time::Rounding::Nearest)
            .unwrap()
    };
    let targets = [
        SeekTarget::new(0, at(6), SeekMode::Nearest, SeekScope::Any),
        SeekTarget::new(
            0,
            at(6),
            SeekMode::AtOrBefore,
            SeekScope::RandomAccessPoints,
        ),
        SeekTarget::new(0, at(5), SeekMode::AtOrAfter, SeekScope::RandomAccessPoints),
        SeekTarget::new(0, at(0), SeekMode::Nearest, SeekScope::Any),
    ];
    check_demuxer_seek(|| ToyDemuxer::new(MediaLimits::default()), &file, &targets).unwrap();
}

#[test]
fn the_toy_muxer_passes() {
    let p = simple_packets(9, 4);
    check_muxer_output_honesty(|| ToyMuxer::new(true), &[track()], &p).unwrap();
    check_muxer_output_honesty(|| ToyMuxer::new(false), &[track()], &p).unwrap();
}

#[test]
fn animation_over_the_toy_passes() {
    let frames: Vec<PixelBuffer> = (0..6u8)
        .map(|i| {
            let mut b = PixelBuffer::new(W, H, PixelDescriptor::GRAY8_SRGB);
            for y in 0..H {
                for (x, px) in b.as_slice_mut().row_mut(y).iter_mut().enumerate() {
                    *px = i * 40 + y as u8 * 4 + x as u8;
                }
            }
            b
        })
        .collect();
    let durations: Vec<FrameDuration> = (1..=6u64)
        .map(|n| FrameDuration::new(n * 1001, 30000).unwrap())
        .collect();
    for lookahead in [0usize, 1, 2] {
        check_animation_over_video(
            || ToyEncoder::new(W, H, tb(), lookahead, 3),
            || ToyMuxer::new(false),
            lookahead as u32,
            Gray8::default,
            || ToyDemuxer::new(MediaLimits::default()),
            || ToyDecoder::new(4),
            Gray8::default(),
            &frames,
            &durations,
        )
        .unwrap();
    }
}

// ---------------------------------------------------------- liars caught ---

/// Takes the payload even when pushing back.
struct GreedyDecoder(ToyDecoder);

impl VideoDecoder for GreedyDecoder {
    type Error = ToyError;
    fn configure(
        &mut self,
        c: &CodecConfig,
        s: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<ToyError>> {
        self.0.configure(c, s)
    }
    fn send(
        &mut self,
        p: &mut Packet,
        s: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<ToyError>> {
        let st = self.0.send(p, s)?;
        if st == SendStatus::ReceivePending {
            p.take();
        }
        Ok(st)
    }
    fn receive(
        &mut self,
        s: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Frame>, SessionError<ToyError>> {
        self.0.receive(s)
    }
    fn end_input(&mut self) -> Result<(), SessionError<ToyError>> {
        self.0.end_input()
    }
    fn reset(&mut self) -> Result<(), SessionError<ToyError>> {
        self.0.reset()
    }
    fn stream_info(&self) -> Option<&VideoStreamInfo> {
        self.0.stream_info()
    }
}

#[test]
fn a_decoder_that_takes_a_pushed_back_packet_is_caught() {
    let e =
        check_video_decoder_contract(|| GreedyDecoder(ToyDecoder::new(2)), &config(), &packets())
            .unwrap_err();
    assert!(e.detail.contains("took the payload"), "{e}");
}

/// Forgets the timestamp of every packet it emits.
struct AmnesiacEncoder(ToyEncoder);

impl VideoEncoder for AmnesiacEncoder {
    type Error = ToyError;
    fn send(
        &mut self,
        f: &Frame,
        s: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<ToyError>> {
        self.0.send(f, s)
    }
    fn receive(
        &mut self,
        s: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Packet>, SessionError<ToyError>> {
        Ok(match self.0.receive(s)? {
            Receive::Ready(p) => {
                let bytes = p.bytes().to_vec();
                Receive::Ready(Packet::from_vec(bytes).unwrap().with_track(p.track()))
            }
            other => other,
        })
    }
    fn end_input(&mut self) -> Result<(), SessionError<ToyError>> {
        self.0.end_input()
    }
    fn output_config(&self) -> Option<&CodecConfig> {
        self.0.output_config()
    }
}

#[test]
fn an_encoder_that_drops_timestamps_is_caught() {
    let e = check_video_encoder_contract(
        || AmnesiacEncoder(ToyEncoder::new(W, H, tb(), 1, 3)),
        &frames(4),
    )
    .unwrap_err();
    assert!(e.detail.contains("no timestamp"), "{e}");
}

/// Uses whatever bytes it is offered, wherever they start.
struct GullibleDemuxer(ToyDemuxer);

impl Demuxer for GullibleDemuxer {
    type Error = ToyError;
    fn step(
        &mut self,
        input: Input<'_>,
        s: Option<&dyn zencodec::Pulse>,
    ) -> Result<DemuxProgress, SessionError<ToyError>> {
        let p = self.0.step(input, s)?;
        if matches!(p.event, zencodec_media::container::DemuxEvent::Need) && !input.bytes.is_empty()
        {
            return Ok(DemuxProgress::need(1));
        }
        Ok(p)
    }
    fn requested(&self) -> &[InputRequest] {
        self.0.requested()
    }
    fn tracks(&self) -> &[TrackInfo] {
        self.0.tracks()
    }
}

#[test]
fn a_demuxer_that_uses_unrequested_input_is_caught() {
    let file = toy_file(6, 3, false);
    let e = check_demuxer_contract(
        || GullibleDemuxer(ToyDemuxer::new(MediaLimits::default())),
        &file,
    )
    .unwrap_err();
    assert!(
        e.detail.contains("protocol") || e.detail.contains("nobody asked"),
        "{e}"
    );
}

/// Drops a byte whenever a partial drain asks for fewer than are pending.
struct LeakyMuxer(ToyMuxer);

impl Muxer for LeakyMuxer {
    type Error = ToyError;
    fn begin(&mut self, t: &[TrackInfo], h: MuxHint) -> Result<MuxTiming, SessionError<ToyError>> {
        self.0.begin(t, h)
    }
    fn write_packet(
        &mut self,
        p: Packet,
        s: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<ToyError>> {
        self.0.write_packet(p, s)
    }
    fn pending_output(&self) -> &[u8] {
        self.0.pending_output()
    }
    fn consume_output(&mut self, n: usize) {
        let pending = self.0.pending_output().len();
        self.0
            .consume_output(if n > 0 && n < pending { n + 1 } else { n })
    }
    fn take_output(&mut self, into: &mut Vec<u8>) {
        self.0.take_output(into)
    }
    fn finish(self, s: Option<&dyn zencodec::Pulse>) -> Result<MuxFinish, SessionError<ToyError>> {
        self.0.finish(s)
    }
}

#[test]
fn a_muxer_whose_bytes_depend_on_draining_is_caught() {
    let p = simple_packets(9, 4);
    let e = check_muxer_output_honesty(|| LeakyMuxer(ToyMuxer::new(false)), &[track()], &p)
        .unwrap_err();
    assert!(
        e.detail.contains("clamp") || e.detail.contains("changed the file"),
        "{e}"
    );
}

#[test]
fn frames_are_send_and_sync_handles() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Frame>();
    let f = frames(1).remove(0);
    let g = f.clone();
    assert!(f.shares_picture(&g));
    assert_eq!(Arc::strong_count(f.picture()), 2);
}

/// Changes only advertised track facts when the driver offers the whole file.
/// Packet bytes/timing and the number of tracks remain identical.
struct WindowDependentTracks {
    inner: ToyDemuxer,
    tracks: Vec<TrackInfo>,
    first_len: Option<usize>,
    threshold: usize,
    field: u8,
    late: bool,
    changed: bool,
    pending: Option<zencodec_media::container::DemuxEvent>,
}
impl WindowDependentTracks {
    fn new(threshold: usize, field: u8, late: bool) -> Self {
        Self {
            inner: ToyDemuxer::new(MediaLimits::default()),
            tracks: Vec::new(),
            first_len: None,
            threshold,
            field,
            late,
            changed: false,
            pending: None,
        }
    }
    fn publish_track(&mut self, mutate: bool) {
        let field = if mutate { self.field } else { 255 };
        let codec = if field == 0 {
            CodecId::Av1
        } else {
            CodecId::Other(CodecTag::new("toy"))
        };
        let clock = if field == 1 { TimeBase::MILLIS } else { tb() };
        let video = VideoTrackInfo::new(if field == 2 { W + 1 } else { W }, H);
        let mut builder = TrackInfo::video(0, codec, clock, video);
        #[allow(deprecated)] // Keep the legacy-offset regression until native migration completes.
        {
            builder = match field {
                3 => builder.with_codec_private(Arc::from([1u8, 2, 3])),
                4 => builder.with_config_epoch(7),
                5 => builder.with_codec_delay(FrameDuration::new(1, 48000).unwrap()),
                6 => builder.with_seek_preroll(FrameDuration::new(80, 1000).unwrap()),
                7 => builder.with_edit_delay_ticks(-17),
                8 => builder.with_declared_packets(42),
                9 => builder.with_declared_duration_ticks(1234),
                10 => builder.with_timeline(zencodec_media::timeline::TrackTimeline::Unedited),
                _ => builder,
            };
        }
        self.tracks = vec![builder.build().unwrap()];
    }
}
impl Demuxer for WindowDependentTracks {
    type Error = ToyError;
    fn step(
        &mut self,
        input: Input<'_>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<DemuxProgress, SessionError<ToyError>> {
        use zencodec_media::container::DemuxEvent;
        if let Some(event) = self.pending.take() {
            return Ok(DemuxProgress::new(0, event));
        }
        if self.first_len.is_none() && !input.bytes.is_empty() {
            self.first_len = Some(input.bytes.len());
        }
        let p = self.inner.step(input, pulse)?;
        let mutate = self.first_len.is_some_and(|n| n > self.threshold);
        match &p.event {
            DemuxEvent::Headers => self.publish_track(mutate && !self.late),
            DemuxEvent::Packet(_) if self.late && !self.changed => {
                self.changed = true;
                self.publish_track(mutate);
                self.pending = Some(p.event);
                return Ok(DemuxProgress::new(
                    p.consumed,
                    DemuxEvent::ConfigChanged { track: 0, epoch: 0 },
                ));
            }
            _ => {}
        }
        Ok(p)
    }
    fn requested(&self) -> &[InputRequest] {
        self.inner.requested()
    }
    fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }
}

#[test]
fn demuxer_conformance_detects_track_facts_that_depend_on_input_window() {
    let file = toy_file(9, 3, false);
    for late in [false, true] {
        for field in 0..11 {
            let error = check_demuxer_contract(
                || WindowDependentTracks::new(file.len() / 2, field, late),
                &file,
            )
            .expect_err("changing track facts must not pass merely because track counts match");
            assert!(
                error.detail.contains("different events"),
                "field {field}, late {late}: {error}"
            );
        }
    }
}
