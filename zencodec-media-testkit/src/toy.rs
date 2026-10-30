//! Reference implementations: a toy container (TIVF: a 20-byte header and
//! length-prefixed timed packets) and a toy codec (a packet holds 0..3 grey
//! pictures and may re-present the previous one).
//!
//! They exercise every rule of the media contracts: output later than input
//! and 0..N frames per packet, shared pictures, backpressure, drain, reset,
//! configuration epochs, seeking with an index scan, and pulled muxer output
//! with or without patches. The conformance checks are validated against
//! them, as `zencodec-testkit`'s checks are against its reference codec.
//!
//! TIVF header: `b"TIVF"`, width `u16`, height `u16`, time base numerator
//! `u32`, denominator `u32`, packet count `u32` (0 = unknown), all little
//! endian. Packet: payload length `u32`, pts `i64`, flags `u8` (bit 0:
//! random access), payload. Toy payload: picture count `u8` (0..=3),
//! show-existing `u8`, then the grey planes.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::fmt;
use zenpixels::sample::SampleEncoding;

use zencodec::encode::OutputBuffer;

use zencodec::time::{TimeBase, Timestamp};
use zencodec_media::container::{
    AccessPattern, DemuxEvent, DemuxProgress, Demuxer, Input, InputRequest, MuxFinish, MuxHint,
    MuxTiming, Muxer, Patch, SeekMode, SeekResult, SeekScope, SeekStatus, SeekTarget,
};
use zencodec_media::frame::{
    ChromaSiting, Frame, FrameGeometry, FrameInterpretation, FrameTiming, OwnedPlane, OwnedPlanes,
    PlaneData, PlaneRole, PlaneSamples, Presentation, Subsampling,
};
use zencodec_media::limits::MediaLimits;
use zencodec_media::packet::{Packet, RandomAccess};
use zencodec_media::session::{
    CodecConfig, ProtocolViolation, Receive, SendStatus, SessionError, UnsupportedMedia,
    VideoDecoder, VideoEncoder, VideoStreamInfo,
};
use zencodec_media::track::{CodecId, CodecTag, TrackInfo, TrackKind, VideoTrackInfo};

/// The TIVF file signature.
pub const MAGIC: &[u8; 4] = b"TIVF";
/// TIVF header length.
pub const HEADER_LEN: usize = 20;
/// TIVF packet header length.
pub const PACKET_HEADER_LEN: usize = 13;
const COUNT_OFFSET: u64 = 16;

/// Errors of the toy container and codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToyError {
    /// Not a TIVF stream.
    BadMagic,
    /// The stream ends inside a record.
    Truncated,
    /// A zero time base.
    BadTimeBase,
    /// A malformed toy payload.
    BadPayload,
    /// No presentation satisfies the seek.
    NoSuchFrame,
    /// No such track.
    NoSuchTrack,
    /// Seek before the headers were read.
    HeadersFirst,
    /// Frame geometry differs from the configuration.
    ShapeMismatch,
    /// TIVF holds exactly one video track.
    WrongTrackCount,
}

impl fmt::Display for ToyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadMagic => "not a TIVF stream",
            Self::Truncated => "stream ends inside a record",
            Self::BadTimeBase => "zero time base",
            Self::BadPayload => "malformed toy packet payload",
            Self::NoSuchFrame => "no presentation satisfies the seek",
            Self::NoSuchTrack => "no such track",
            Self::HeadersFirst => "headers have not been read",
            Self::ShapeMismatch => "frame geometry does not match the configuration",
            Self::WrongTrackCount => "toy container holds exactly one video track",
        })
    }
}
impl core::error::Error for ToyError {}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn i64_at(b: &[u8], i: usize) -> i64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[i..i + 8]);
    i64::from_le_bytes(a)
}

#[derive(Clone, Copy, Debug)]
struct IndexEntry {
    offset: u64,
    pts: i64,
    random_access: bool,
    ordinal: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Header,
    Packets,
    Scanning,
    Done,
}

/// Sans-I/O TIVF demuxer. Sequential by default; `seek` builds a bounded
/// index by scanning packet headers with `Range` requests.
pub struct ToyDemuxer {
    limits: MediaLimits,
    state: State,
    tracks: Vec<TrackInfo>,
    requests: Vec<InputRequest>,
    time_base: Option<TimeBase>,
    pos: u64,
    ordinal: u64,
    declared: u32,
    emitted: u64,
    index: Vec<IndexEntry>,
    index_complete: bool,
    scan_pos: u64,
    scan_ordinal: u64,
    pending_seek: Option<SeekTarget>,
}

impl ToyDemuxer {
    /// A demuxer enforcing `limits`.
    pub fn new(limits: MediaLimits) -> Self {
        Self {
            limits,
            state: State::Header,
            tracks: Vec::new(),
            requests: Vec::new(),
            time_base: None,
            pos: 0,
            ordinal: 0,
            declared: 0,
            emitted: 0,
            index: Vec::new(),
            index_complete: false,
            scan_pos: HEADER_LEN as u64,
            scan_ordinal: 0,
            pending_seek: None,
        }
    }

    fn need(&mut self, r: InputRequest) -> Result<DemuxProgress, SessionError<ToyError>> {
        self.requests.push(r);
        Ok(DemuxProgress::need(0))
    }

    fn note(&mut self, entry: IndexEntry) {
        if !self.index_complete && self.index.len() as u64 == entry.ordinal {
            self.index.push(entry);
            self.scan_pos = entry.offset;
            self.scan_ordinal = entry.ordinal;
        }
    }

    fn pick(&self, target: &SeekTarget) -> Result<IndexEntry, SessionError<ToyError>> {
        let tb = self
            .time_base
            .ok_or(SessionError::Codec(ToyError::HeadersFirst))?;
        let candidates = self
            .index
            .iter()
            .filter(|e| target.scope == SeekScope::Any || e.random_access);
        let t = target.time;
        // Distance on a common denominator, exact in i128.
        let value = |e: &IndexEntry| {
            i128::from(e.pts) * i128::from(tb.numerator()) * i128::from(t.base().denominator())
        };
        let want =
            i128::from(t.ticks()) * i128::from(t.base().numerator()) * i128::from(tb.denominator());
        let chosen = match target.mode {
            SeekMode::Nearest => candidates.min_by(|a, b| {
                let da = (value(a) - want).abs();
                let db = (value(b) - want).abs();
                da.cmp(&db).then(a.pts.cmp(&b.pts))
            }),
            SeekMode::AtOrBefore => candidates
                .filter(|e| value(e) <= want)
                .max_by_key(|e| e.pts),
            SeekMode::AtOrAfter => candidates
                .filter(|e| value(e) >= want)
                .min_by_key(|e| e.pts),
            _ => return Err(SessionError::Unsupported(UnsupportedMedia::Seek)),
        };
        chosen
            .copied()
            .ok_or(SessionError::Codec(ToyError::NoSuchFrame))
    }

    fn position(&mut self, e: IndexEntry) -> SeekResult {
        self.pos = e.offset;
        self.ordinal = e.ordinal;
        self.state = State::Packets;
        SeekResult::new(
            Timestamp::new(e.pts, self.time_base.expect("headers")),
            e.ordinal,
            if e.random_access {
                RandomAccess::Yes
            } else {
                RandomAccess::No
            },
            0,
        )
    }
}

impl Demuxer for ToyDemuxer {
    type Error = ToyError;

    fn step(
        &mut self,
        input: Input<'_>,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<DemuxProgress, SessionError<ToyError>> {
        let stop = stop.unwrap_or(&zencodec::NoPulse);
        stop.check()?;
        self.requests.clear();
        match self.state {
            State::Header => {
                if input.offset != 0 {
                    return self.need(InputRequest::at_least(0, HEADER_LEN));
                }
                let b = input.bytes;
                if b.len() < HEADER_LEN {
                    if input.reaches_end() {
                        return Err(SessionError::Codec(ToyError::Truncated));
                    }
                    return self.need(InputRequest::at_least(0, HEADER_LEN));
                }
                if &b[0..4] != MAGIC {
                    return Err(SessionError::Codec(ToyError::BadMagic));
                }
                let (w, h) = (u32::from(u16_at(b, 4)), u32::from(u16_at(b, 6)));
                let tb = TimeBase::new(u32_at(b, 8), u32_at(b, 12))
                    .map_err(|_| SessionError::Codec(ToyError::BadTimeBase))?;
                self.declared = u32_at(b, 16);
                let mut track = TrackInfo::video(
                    0,
                    CodecId::Other(CodecTag::new("toy")),
                    tb,
                    VideoTrackInfo::new(w, h),
                );
                if self.declared != 0 {
                    track = track.with_declared_packets(u64::from(self.declared));
                }
                let track = track
                    .build()
                    .map_err(|_| SessionError::Codec(ToyError::ShapeMismatch))?;
                self.tracks = vec![track];
                self.time_base = Some(tb);
                self.state = State::Packets;
                self.pos = HEADER_LEN as u64;
                Ok(DemuxProgress::new(HEADER_LEN, DemuxEvent::Headers))
            }
            State::Packets => {
                if input.offset != self.pos {
                    return self.need(InputRequest::at_least(self.pos, PACKET_HEADER_LEN));
                }
                let b = input.bytes;
                if b.is_empty() && input.reaches_end() {
                    self.state = State::Done;
                    self.index_complete = true;
                    if self.declared != 0 && self.emitted != u64::from(self.declared) {
                        return Err(SessionError::Codec(ToyError::Truncated));
                    }
                    return Ok(DemuxProgress::new(0, DemuxEvent::Done));
                }
                if b.len() < PACKET_HEADER_LEN {
                    if input.reaches_end() {
                        return Err(SessionError::Codec(ToyError::Truncated));
                    }
                    return self.need(InputRequest::at_least(self.pos, PACKET_HEADER_LEN));
                }
                let len = u32_at(b, 0) as usize;
                self.limits
                    .check_packet_bytes(len as u64)
                    .map_err(SessionError::Limit)?;
                let total = PACKET_HEADER_LEN + len;
                if b.len() < total {
                    if input.reaches_end() {
                        return Err(SessionError::Codec(ToyError::Truncated));
                    }
                    return self.need(InputRequest::at_least(self.pos, total));
                }
                let pts = i64_at(b, 4);
                let flags = b[12];
                let tb = self.time_base.expect("headers");
                let packet = Packet::from_vec(b[PACKET_HEADER_LEN..total].to_vec())
                    .map_err(|_| SessionError::Codec(ToyError::BadPayload))?
                    .with_pts(Timestamp::new(pts, tb))
                    .with_random_access(if flags & 1 != 0 {
                        RandomAccess::Yes
                    } else {
                        RandomAccess::No
                    })
                    .with_track(0)
                    .with_ordinal(self.ordinal)
                    .with_input_offset(self.pos);
                self.note(IndexEntry {
                    offset: self.pos,
                    pts,
                    random_access: flags & 1 != 0,
                    ordinal: self.ordinal,
                });
                self.pos += total as u64;
                self.scan_pos = self.scan_pos.max(self.pos);
                self.scan_ordinal = self.scan_ordinal.max(self.ordinal + 1);
                self.ordinal += 1;
                self.emitted += 1;
                Ok(DemuxProgress::new(total, DemuxEvent::Packet(packet)))
            }
            State::Scanning => {
                let Some(total) = input.total_len else {
                    return self.need(InputRequest::Len);
                };
                if self.scan_pos >= total {
                    self.index_complete = true;
                    let target = self.pending_seek.take().expect("scanning implies a target");
                    let entry = self.pick(&target)?;
                    let result = self.position(entry);
                    return Ok(DemuxProgress::new(0, DemuxEvent::Seeked(result)));
                }
                if input.offset != self.scan_pos || input.bytes.len() < PACKET_HEADER_LEN {
                    if input.offset == self.scan_pos && input.reaches_end() {
                        return Err(SessionError::Codec(ToyError::Truncated));
                    }
                    return self.need(InputRequest::range(self.scan_pos, PACKET_HEADER_LEN as u64));
                }
                let b = input.bytes;
                let len = u32_at(b, 0) as u64;
                let entry = IndexEntry {
                    offset: self.scan_pos,
                    pts: i64_at(b, 4),
                    random_access: b[12] & 1 != 0,
                    ordinal: self.scan_ordinal,
                };
                if self.index.len() as u64 == entry.ordinal {
                    self.index.push(entry);
                }
                self.scan_pos += PACKET_HEADER_LEN as u64 + len;
                self.scan_ordinal += 1;
                if self.scan_pos >= total {
                    // The scan reached the end: every packet is indexed.
                    self.index_complete = true;
                    let target = self.pending_seek.take().expect("scanning implies a target");
                    let entry = self.pick(&target)?;
                    let result = self.position(entry);
                    return Ok(DemuxProgress::new(
                        PACKET_HEADER_LEN,
                        DemuxEvent::Seeked(result),
                    ));
                }
                self.requests
                    .push(InputRequest::range(self.scan_pos, PACKET_HEADER_LEN as u64));
                Ok(DemuxProgress::need(PACKET_HEADER_LEN))
            }
            State::Done => Ok(DemuxProgress::new(0, DemuxEvent::Done)),
        }
    }

    fn requested(&self) -> &[InputRequest] {
        &self.requests
    }

    fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }

    fn access_pattern(&self) -> AccessPattern {
        if self.state == State::Scanning {
            AccessPattern::RandomAccess
        } else {
            AccessPattern::Sequential
        }
    }

    fn seek(&mut self, target: &SeekTarget) -> Result<SeekStatus, SessionError<ToyError>> {
        if self.state == State::Header {
            return Err(SessionError::Codec(ToyError::HeadersFirst));
        }
        if target.track != 0 {
            return Err(SessionError::Codec(ToyError::NoSuchTrack));
        }
        if self.index_complete {
            let e = self.pick(target)?;
            return Ok(SeekStatus::Positioned(self.position(e)));
        }
        self.pending_seek = Some(*target);
        self.state = State::Scanning;
        Ok(SeekStatus::Scanning)
    }
}

/// TIVF muxer. `write_count` means the header's packet count is patched at
/// finish, which a forward-only destination cannot accept.
pub struct ToyMuxer {
    out: OutputBuffer,
    begun: bool,
    write_count: bool,
    time_base: Option<TimeBase>,
    count: u32,
}

impl ToyMuxer {
    /// A muxer; with `write_count`, the header's packet count is patched at
    /// `finish`.
    pub fn new(write_count: bool) -> Self {
        Self {
            out: OutputBuffer::new(),
            begun: false,
            write_count,
            time_base: None,
            count: 0,
        }
    }
}

impl Muxer for ToyMuxer {
    type Error = ToyError;

    fn begin(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<ToyError>> {
        if self.begun {
            return Err(SessionError::Protocol(ProtocolViolation::BeginTwice));
        }
        let [track] = tracks else {
            return Err(SessionError::Codec(ToyError::WrongTrackCount));
        };
        let Some(video) = track
            .video_info()
            .filter(|_| track.kind() == TrackKind::Video)
        else {
            return Err(SessionError::Codec(ToyError::WrongTrackCount));
        };
        if self.write_count && hint.forward_only {
            return Err(SessionError::Unsupported(
                UnsupportedMedia::ForwardOnlyLayout,
            ));
        }
        if video.coded_width > u32::from(u16::MAX) || video.coded_height > u32::from(u16::MAX) {
            return Err(SessionError::Codec(ToyError::ShapeMismatch));
        }
        let tb = track.time_base();
        self.out.push(MAGIC);
        self.out.push(&(video.coded_width as u16).to_le_bytes());
        self.out.push(&(video.coded_height as u16).to_le_bytes());
        self.out.push(&tb.numerator().to_le_bytes());
        self.out.push(&tb.denominator().to_le_bytes());
        self.out.push(&0u32.to_le_bytes());
        self.time_base = Some(tb);
        self.begun = true;
        Ok(if self.write_count {
            MuxTiming::streaming_with_patches()
        } else {
            MuxTiming::streaming()
        })
    }

    fn write_packet(
        &mut self,
        mut packet: Packet,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<ToyError>> {
        let stop = stop.unwrap_or(&zencodec::NoPulse);
        stop.check()?;
        if !self.begun {
            return Err(SessionError::Protocol(ProtocolViolation::NoBegin));
        }
        let pts = packet
            .pts()
            .ok_or(SessionError::Protocol(ProtocolViolation::MissingTimestamp))?;
        if Some(pts.base()) != self.time_base {
            return Err(SessionError::Protocol(ProtocolViolation::ClockMismatch));
        }
        let payload = packet
            .take()
            .ok_or(SessionError::Protocol(ProtocolViolation::EmptyPacket))?;
        let bytes = payload.as_bytes();
        self.out.push(&(bytes.len() as u32).to_le_bytes());
        self.out.push(&pts.ticks().to_le_bytes());
        self.out
            .push(&[u8::from(packet.random_access() == RandomAccess::Yes)]);
        self.out.push(bytes);
        self.count += 1;
        Ok(())
    }

    fn pending_output(&self) -> &[u8] {
        self.out.pending()
    }
    fn consume_output(&mut self, n: usize) {
        self.out.consume(n)
    }
    fn take_output(&mut self, into: &mut Vec<u8>) {
        self.out.take_into(into)
    }

    fn finish(
        self,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<MuxFinish, SessionError<ToyError>> {
        let stop = stop.unwrap_or(&zencodec::NoPulse);
        stop.check()?;
        if !self.begun {
            return Err(SessionError::Protocol(ProtocolViolation::NoBegin));
        }
        let patches = if self.write_count {
            vec![Patch {
                offset: COUNT_OFFSET,
                bytes: self.count.to_le_bytes().to_vec(),
            }]
        } else {
            Vec::new()
        };
        Ok(MuxFinish::new(self.out.into_remaining(), patches))
    }
}

/// Toy decoder. Pictures come out one packet late (a one-packet reorder),
/// so output trails input; a show-existing flag re-presents the previous
/// picture. Holds at most `max_queued_frames` before pushing back.
pub struct ToyDecoder {
    config: Option<CodecConfig>,
    interpretation: Option<Arc<FrameInterpretation>>,
    stream: Option<VideoStreamInfo>,
    hold: VecDeque<Frame>,
    ready: VecDeque<Frame>,
    last: Option<Frame>,
    ended: bool,
    max_queued: usize,
}

impl ToyDecoder {
    /// A decoder holding at most `max_queued_frames` (at least 1) frames.
    pub fn new(max_queued_frames: usize) -> Self {
        Self {
            config: None,
            interpretation: None,
            stream: None,
            hold: VecDeque::new(),
            ready: VecDeque::new(),
            last: None,
            ended: false,
            max_queued: max_queued_frames.max(1),
        }
    }
}

impl VideoDecoder for ToyDecoder {
    type Error = ToyError;

    fn configure(
        &mut self,
        config: &CodecConfig,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<ToyError>> {
        let stop = stop.unwrap_or(&zencodec::NoPulse);
        stop.check()?;
        let video = config
            .video
            .ok_or(SessionError::Unsupported(UnsupportedMedia::PixelFormat))?;
        let interp = Arc::new(
            FrameInterpretation::new(
                FrameGeometry::full(video.coded_width, video.coded_height),
                Subsampling::Mono,
                ChromaSiting::Unknown,
                SampleEncoding::new(zenpixels::ChannelType::U8, 8, 0).unwrap(),
                vec![PlaneRole::Luma],
            )
            .with_config_epoch(config.epoch),
        );
        let mut stream = VideoStreamInfo::new(Arc::clone(&interp));
        stream.reorder_depth = Some(1);
        self.stream = Some(stream);
        self.interpretation = Some(interp);
        self.config = Some(config.clone());
        Ok(())
    }

    fn send(
        &mut self,
        packet: &mut Packet,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<ToyError>> {
        let stop = stop.unwrap_or(&zencodec::NoPulse);
        stop.check()?;
        if self.ended {
            return Err(SessionError::Protocol(ProtocolViolation::SendAfterEnd));
        }
        let (Some(config), Some(interp)) = (&self.config, &self.interpretation) else {
            return Err(SessionError::Protocol(ProtocolViolation::NotConfigured));
        };
        if packet.is_taken() {
            return Err(SessionError::Protocol(ProtocolViolation::EmptyPacket));
        }
        if packet.config_epoch() != config.epoch {
            return Err(SessionError::Protocol(ProtocolViolation::EpochMismatch));
        }
        if !self.ready.is_empty() && self.ready.len() + self.hold.len() >= self.max_queued {
            return Ok(SendStatus::ReceivePending);
        }
        let video = config.video.expect("configured with video");
        let (w, h) = (video.coded_width as usize, video.coded_height as usize);
        let b = packet.bytes();
        if b.len() < 2 {
            return Err(SessionError::Codec(ToyError::BadPayload));
        }
        let pictures = b[0] as usize;
        let show_existing = b[1] != 0;
        if pictures > 3 || b.len() != 2 + pictures * w * h {
            return Err(SessionError::Codec(ToyError::BadPayload));
        }
        let mut timing =
            FrameTiming::default().with_packet(packet.ordinal(), packet.input_offset());
        timing.pts = packet.pts();
        timing.duration_ticks = packet.duration_ticks();
        // Pictures of the previous packet are released now: a one-packet reorder.
        let released: Vec<Frame> = self.hold.drain(..).collect();
        self.ready.extend(released);
        for i in 0..pictures {
            let start = 2 + i * w * h;
            let data = b[start..start + w * h].to_vec();
            let planes = OwnedPlanes::new(
                vec![OwnedPlane {
                    data: PlaneData::U8(data),
                    width: w,
                    height: h,
                    stride_bytes: w,
                }],
                SampleEncoding::new(zenpixels::ChannelType::U8, 8, 0).unwrap(),
            )
            .map_err(|_| SessionError::Codec(ToyError::ShapeMismatch))?;
            let mut presentation = Presentation::default();
            presentation.keyframe = i == 0 && packet.random_access() == RandomAccess::Yes;
            let frame = Frame::new(Arc::new(planes), Arc::clone(interp), timing, presentation);
            self.last = Some(frame.clone());
            self.hold.push_back(frame);
        }
        if show_existing {
            let prev = self
                .last
                .as_ref()
                .ok_or(SessionError::Codec(ToyError::BadPayload))?;
            let again = prev.represent(timing, Presentation::SHOW_EXISTING);
            self.hold.push_back(again);
        }
        packet.take();
        Ok(SendStatus::Accepted)
    }

    fn receive(
        &mut self,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Frame>, SessionError<ToyError>> {
        let stop = stop.unwrap_or(&zencodec::NoPulse);
        stop.check()?;
        if let Some(f) = self.ready.pop_front() {
            return Ok(Receive::Ready(f));
        }
        if self.ended {
            return Ok(match self.hold.pop_front() {
                Some(f) => Receive::Ready(f),
                None => Receive::EndOfStream,
            });
        }
        Ok(Receive::NeedInput)
    }

    fn end_input(&mut self) -> Result<(), SessionError<ToyError>> {
        self.ended = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<(), SessionError<ToyError>> {
        self.hold.clear();
        self.ready.clear();
        self.last = None;
        self.ended = false;
        Ok(())
    }

    fn stream_info(&self) -> Option<&VideoStreamInfo> {
        self.stream.as_ref()
    }
}

/// Toy encoder: one gray picture per packet, `lookahead` frames held back,
/// a random-access point every `keyint` packets.
pub struct ToyEncoder {
    config: CodecConfig,
    lookahead: usize,
    keyint: u64,
    queue: VecDeque<Frame>,
    out: VecDeque<Packet>,
    max_out: usize,
    ended: bool,
    ordinal: u64,
}

impl ToyEncoder {
    /// An encoder of `width` x `height` grey pictures on `time_base`,
    /// holding `lookahead` frames and marking every `keyint`-th packet as a
    /// random-access point.
    pub fn new(
        width: u32,
        height: u32,
        time_base: TimeBase,
        lookahead: usize,
        keyint: u64,
    ) -> Self {
        Self {
            config: CodecConfig::new(CodecId::Other(CodecTag::new("toy")), time_base)
                .with_video(VideoTrackInfo::new(width, height)),
            lookahead,
            keyint: keyint.max(1),
            queue: VecDeque::new(),
            out: VecDeque::new(),
            max_out: 2,
            ended: false,
            ordinal: 0,
        }
    }

    /// Frames held before their packet comes out.
    pub fn lookahead(&self) -> usize {
        self.lookahead
    }

    fn encode(&mut self, frame: &Frame) -> Result<Packet, SessionError<ToyError>> {
        let video = self.config.video.expect("video config");
        let (w, h) = (video.coded_width as usize, video.coded_height as usize);
        let mapped = frame
            .map()
            .map_err(|_| SessionError::Codec(ToyError::ShapeMismatch))?;
        let plane = mapped
            .plane(0)
            .ok_or(SessionError::Codec(ToyError::ShapeMismatch))?;
        if plane.width() != w || plane.height() != h {
            return Err(SessionError::Codec(ToyError::ShapeMismatch));
        }
        let mut payload = Vec::with_capacity(2 + w * h);
        payload.push(1);
        payload.push(0);
        for y in 0..h {
            match plane.row(y).expect("row in range") {
                PlaneSamples::U8(r) => payload.extend_from_slice(r),
                PlaneSamples::U16(_) => return Err(SessionError::Codec(ToyError::ShapeMismatch)),
            }
        }
        let pts = frame
            .timing()
            .pts
            .ok_or(SessionError::Protocol(ProtocolViolation::MissingTimestamp))?;
        let mut p = Packet::from_vec(payload)
            .map_err(|_| SessionError::Codec(ToyError::BadPayload))?
            .with_track(0)
            .with_pts(pts)
            .with_ordinal(self.ordinal)
            .with_random_access(if self.ordinal.is_multiple_of(self.keyint) {
                RandomAccess::Yes
            } else {
                RandomAccess::No
            });
        if let Some(d) = frame.timing().duration_ticks {
            p = p.with_duration_ticks(d);
        }
        self.ordinal += 1;
        Ok(p)
    }
}

impl VideoEncoder for ToyEncoder {
    type Error = ToyError;

    fn send(
        &mut self,
        frame: &Frame,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<ToyError>> {
        let stop = stop.unwrap_or(&zencodec::NoPulse);
        stop.check()?;
        if self.ended {
            return Err(SessionError::Protocol(ProtocolViolation::SendAfterEnd));
        }
        let video = self.config.video.expect("video config");
        let g = &frame.interpretation().geometry;
        if g.coded_width != video.coded_width
            || g.coded_height != video.coded_height
            || frame.interpretation().subsampling != Subsampling::Mono
        {
            return Err(SessionError::Codec(ToyError::ShapeMismatch));
        }
        let pts = frame
            .timing()
            .pts
            .ok_or(SessionError::Protocol(ProtocolViolation::MissingTimestamp))?;
        if pts.base() != self.config.time_base {
            return Err(SessionError::Protocol(ProtocolViolation::ClockMismatch));
        }
        if self.out.len() >= self.max_out {
            return Ok(SendStatus::ReceivePending);
        }
        self.queue.push_back(frame.clone());
        while self.queue.len() > self.lookahead {
            let f = self.queue.pop_front().expect("nonempty");
            let p = self.encode(&f)?;
            self.out.push_back(p);
        }
        Ok(SendStatus::Accepted)
    }

    fn receive(
        &mut self,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Packet>, SessionError<ToyError>> {
        let stop = stop.unwrap_or(&zencodec::NoPulse);
        stop.check()?;
        if let Some(p) = self.out.pop_front() {
            return Ok(Receive::Ready(p));
        }
        if self.ended {
            return Ok(match self.queue.pop_front() {
                Some(f) => Receive::Ready(self.encode(&f)?),
                None => Receive::EndOfStream,
            });
        }
        Ok(Receive::NeedInput)
    }

    fn end_input(&mut self) -> Result<(), SessionError<ToyError>> {
        self.ended = true;
        Ok(())
    }

    fn output_config(&self) -> Option<&CodecConfig> {
        Some(&self.config)
    }
}

/// Build a toy packet payload.
pub fn toy_payload(pictures: &[&[u8]], show_existing: bool) -> Vec<u8> {
    let mut v = Vec::new();
    v.push(pictures.len() as u8);
    v.push(u8::from(show_existing));
    for p in pictures {
        v.extend_from_slice(p);
    }
    v
}

impl PartialOrd for IndexEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.pts.cmp(&other.pts))
    }
}
impl PartialEq for IndexEntry {
    fn eq(&self, other: &Self) -> bool {
        self.pts == other.pts
    }
}
