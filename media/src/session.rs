//! Track-session layer: explicit per-track routing between a packet source
//! (demuxer) and a packet sink (muxer), with decode/re-encode handlers in
//! between. Nothing is dropped silently — every input packet is counted, and
//! a track excluded from the output carries a recorded reason.
//!
//! The codec adapters are traits, not implementations: qualified decoders and
//! encoders (zenextras) plug in as `Box<dyn …>`. The session owns the pump,
//! drain ordering, and packet accounting — not codec policy.

use crate::color::{ChromaLocation, Subsampling, YuvView};
use crate::plane::{Plane, Samples};
use crate::time::Timestamp;
use crate::track::{Codec, MediaError, MediaPacket, TrackKind, TrackSpec};
use zenpixels::Cicp;
use zenpixels::sample::SampleEncoding;

// --- owned decoded media ----------------------------------------------------

/// Plane payload in native word storage.
pub enum PlaneData {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

impl PlaneData {
    fn as_samples(&self) -> Samples<'_> {
        match self {
            PlaneData::U8(v) => Samples::U8(v),
            PlaneData::U16(v) => Samples::U16(v),
        }
    }
}

/// One owned component plane. `stride_samples` may exceed `width` (padding).
pub struct PlaneBuf {
    pub data: PlaneData,
    pub width: usize,
    pub height: usize,
    pub stride_samples: usize,
}

impl PlaneBuf {
    fn view(&self, encoding: SampleEncoding) -> Result<Plane<'_>, MediaError> {
        let stride_bytes = self
            .stride_samples
            .checked_mul(encoding.storage().byte_size())
            .ok_or(MediaError::Limit("plane stride overflows"))?;
        Plane::new(
            self.data.as_samples(),
            self.width,
            self.height,
            stride_bytes,
            encoding,
        )
        .map_err(|_| MediaError::Contract("decoder produced inconsistent plane geometry"))
    }
}

/// Owned decoded video frame in presentation order: native planar YUV plus
/// the presentation timestamp the decoder attributed to it.
pub struct VideoFrame {
    pub y: PlaneBuf,
    /// `None` for monochrome; `Some` for subsampled or full chroma.
    pub chroma: Option<[PlaneBuf; 2]>,
    pub subsampling: Subsampling,
    pub location: ChromaLocation,
    pub color: Cicp,
    pub encoding: SampleEncoding,
    pub pts: Timestamp,
    pub duration_ticks: Option<u32>,
}

impl VideoFrame {
    /// Borrowed view for encoders (`Av1Encoder::submit` shape).
    pub fn yuv_view(&self) -> Result<YuvView<'_>, MediaError> {
        let chroma = match &self.chroma {
            Some([cb, cr]) => Some([cb.view(self.encoding)?, cr.view(self.encoding)?]),
            None => None,
        };
        YuvView::new(
            self.y.view(self.encoding)?,
            chroma,
            self.subsampling,
            self.location,
            self.color,
        )
        .map_err(|_| MediaError::Contract("decoder produced inconsistent frame"))
    }
}

/// Interleaved PCM payload.
pub enum Pcm {
    S16(Vec<i16>),
    F32(Vec<f32>),
}

/// Owned decoded audio block. `frames` counts per-channel samples, so the
/// buffer length is `frames * channels`. Exact sample accounting lives here:
/// decoder pre-skip and container end-trim must already be applied by the
/// adapter — `frames` is the *true* audible count.
pub struct AudioBlock {
    pub pts: Timestamp,
    pub frames: usize,
    pub channels: u16,
    pub sample_rate: u32,
    pub pcm: Pcm,
}

// --- codec adapter contracts -------------------------------------------------

/// Packet-fed video decoder. Frames emerge in presentation order; the adapter
/// owns any reorder buffer. `end_input` starts the drain; `next_frame` returns
/// `Ok(None)` when more input is required and finally when the drain is done.
pub trait VideoDecoder {
    fn push_packet(&mut self, packet: &MediaPacket) -> Result<(), MediaError>;
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, MediaError>;
    fn end_input(&mut self) -> Result<(), MediaError>;
    /// Discard all decoder state (seek / config epoch change). The next packet
    /// must be a keyframe carrying fresh configuration if the codec needs it.
    fn reset(&mut self) -> Result<(), MediaError>;
}

/// Packet-fed audio decoder. Same push/drain shape; each `AudioBlock` carries
/// an exact, trim-applied sample count.
pub trait AudioDecoder {
    fn push_packet(&mut self, packet: &MediaPacket) -> Result<(), MediaError>;
    fn next_block(&mut self) -> Result<Option<AudioBlock>, MediaError>;
    fn end_input(&mut self) -> Result<(), MediaError>;
    fn reset(&mut self) -> Result<(), MediaError>;
}

/// Frame-fed video encoder. Frames are owned so the adapter can queue them
/// behind encoder backpressure. Output packets carry presentation timestamps
/// — the session never re-times them. `end_input` starts the drain.
pub trait VideoEncoder {
    fn push_frame(&mut self, frame: VideoFrame) -> Result<(), MediaError>;
    fn next_packet(&mut self) -> Result<Option<MediaPacket>, MediaError>;
    fn end_input(&mut self) -> Result<(), MediaError>;
}

/// Block-fed audio encoder. Blocks are owned for the same queueing reason.
pub trait AudioEncoder {
    fn push_block(&mut self, block: AudioBlock) -> Result<(), MediaError>;
    fn next_packet(&mut self) -> Result<Option<MediaPacket>, MediaError>;
    fn end_input(&mut self) -> Result<(), MediaError>;
}

// --- container endpoints -----------------------------------------------------

/// Anything that yields `MediaPacket`s — demuxers implement this.
pub trait PacketSource {
    fn tracks(&self) -> &[TrackSpec];
    fn next_packet(&mut self) -> Result<Option<MediaPacket>, MediaError>;
}

/// Anything that accepts `MediaPacket`s — muxers implement this. Finalization
/// stays with the concrete muxer (`finish` returns its writer/report).
pub trait PacketSink {
    fn write_packet(&mut self, packet: &MediaPacket) -> Result<(), MediaError>;
}

// --- routing -----------------------------------------------------------------

/// Why a track was excluded from the output. Recorded, never implicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    /// No qualified decoder/encoder chain exists for the codec.
    UnsupportedCodec,
    /// The target container has no legal mapping for the codec.
    UnsupportedContainer,
    /// The caller asked to drop it.
    CallerChoice,
}

/// Per-input-track handler — the caller's routing decision made concrete.
pub enum TrackHandler {
    /// Copy packets through unchanged (muxer still enforces codec mapping and
    /// single-config-epoch rules).
    Copy,
    /// Exclude the track; every packet is counted as dropped.
    Drop { reason: DropReason },
    /// Decode then re-encode.
    Video {
        decoder: Box<dyn VideoDecoder>,
        encoder: Box<dyn VideoEncoder>,
    },
    Audio {
        decoder: Box<dyn AudioDecoder>,
        encoder: Box<dyn AudioEncoder>,
    },
}

/// Per-track accounting.
#[derive(Clone, Debug)]
pub struct TrackReport {
    /// Input track index.
    pub track: u32,
    /// Packets read from the source on this track.
    pub packets_in: u64,
    /// Packets written to the sink (0 for dropped tracks).
    pub packets_out: u64,
    /// Packets excluded by the plan (`Drop`) — or lost to a decode that
    /// produced no frames (transcode of an un-decodable tail).
    pub packets_dropped: u64,
    /// Frames/blocks that made it through decode.
    pub frames_decoded: u64,
    /// The recorded reason, when the track was dropped.
    pub drop_reason: Option<DropReason>,
}

/// Whole-session accounting. `packets_in == packets_out + packets_dropped`
/// over every track is the invariant callers can assert.
#[derive(Clone, Debug)]
pub struct SessionReport {
    pub tracks: Vec<TrackReport>,
}

/// Drive a session to completion: read packets in decode order, route each
/// through its handler, drain decoders at end-of-input, drain encoders after
/// their decoders, then return the accounting. The muxer's `finish` is the
/// caller's step after this returns.
///
/// Bounded: the pump itself holds at most one source packet plus whatever the
/// codec adapters queue internally — adapters with reorder/lookahead must
/// enforce their own depth limits.
pub fn pump<S: PacketSource, K: PacketSink>(
    src: &mut S,
    sink: &mut K,
    handlers: &mut [TrackHandler],
) -> Result<SessionReport, MediaError> {
    if handlers.len() != src.tracks().len() {
        return Err(MediaError::Contract(
            "every input track needs an explicit handler",
        ));
    }
    for (h, t) in handlers.iter().zip(src.tracks().iter()) {
        let ok = matches!(
            (h, t.kind),
            (TrackHandler::Video { .. }, TrackKind::Video)
                | (TrackHandler::Audio { .. }, TrackKind::Audio)
                | (TrackHandler::Copy, _)
                | (TrackHandler::Drop { .. }, _)
        );
        if !ok {
            return Err(MediaError::Contract(
                "handler kind does not match track kind",
            ));
        }
    }

    let mut reports: Vec<TrackReport> = src
        .tracks()
        .iter()
        .enumerate()
        .map(|(i, _)| TrackReport {
            track: i as u32,
            packets_in: 0,
            packets_out: 0,
            packets_dropped: 0,
            frames_decoded: 0,
            drop_reason: match &handlers[i] {
                TrackHandler::Drop { reason } => Some(*reason),
                _ => None,
            },
        })
        .collect();

    while let Some(pkt) = src.next_packet()? {
        let ti = pkt.track as usize;
        if ti >= handlers.len() {
            return Err(MediaError::Format("packet for undeclared track"));
        }
        reports[ti].packets_in += 1;
        match &mut handlers[ti] {
            TrackHandler::Copy => {
                sink.write_packet(&pkt)?;
                reports[ti].packets_out += 1;
            }
            TrackHandler::Drop { .. } => {
                reports[ti].packets_dropped += 1;
            }
            TrackHandler::Video { decoder, encoder } => {
                decoder.push_packet(&pkt)?;
                while let Some(f) = decoder.next_frame()? {
                    reports[ti].frames_decoded += 1;
                    encoder.push_frame(f)?;
                    while let Some(out) = encoder.next_packet()? {
                        sink.write_packet(&out)?;
                        reports[ti].packets_out += 1;
                    }
                }
            }
            TrackHandler::Audio { decoder, encoder } => {
                decoder.push_packet(&pkt)?;
                while let Some(b) = decoder.next_block()? {
                    reports[ti].frames_decoded += 1;
                    encoder.push_block(b)?;
                    while let Some(out) = encoder.next_packet()? {
                        sink.write_packet(&out)?;
                        reports[ti].packets_out += 1;
                    }
                }
            }
        }
    }

    // End-of-input: drain each decoder into its encoder, then each encoder
    // into the sink. Any packet that decoded to nothing stays counted in
    // packets_in — the out+dropped ≤ in inequality tells the caller.
    for (ti, h) in handlers.iter_mut().enumerate() {
        match h {
            TrackHandler::Video { decoder, encoder } => {
                decoder.end_input()?;
                while let Some(f) = decoder.next_frame()? {
                    reports[ti].frames_decoded += 1;
                    encoder.push_frame(f)?;
                    while let Some(out) = encoder.next_packet()? {
                        sink.write_packet(&out)?;
                        reports[ti].packets_out += 1;
                    }
                }
                encoder.end_input()?;
                while let Some(out) = encoder.next_packet()? {
                    sink.write_packet(&out)?;
                    reports[ti].packets_out += 1;
                }
            }
            TrackHandler::Audio { decoder, encoder } => {
                decoder.end_input()?;
                while let Some(b) = decoder.next_block()? {
                    reports[ti].frames_decoded += 1;
                    encoder.push_block(b)?;
                    while let Some(out) = encoder.next_packet()? {
                        sink.write_packet(&out)?;
                        reports[ti].packets_out += 1;
                    }
                }
                encoder.end_input()?;
                while let Some(out) = encoder.next_packet()? {
                    sink.write_packet(&out)?;
                    reports[ti].packets_out += 1;
                }
            }
            _ => {}
        }
    }

    Ok(SessionReport { tracks: reports })
}

// --- concrete endpoint impls -------------------------------------------------

impl<R: std::io::Read + std::io::Seek> PacketSource for crate::mp4::Mp4Demuxer<R> {
    fn tracks(&self) -> &[TrackSpec] {
        self.tracks()
    }
    fn next_packet(&mut self) -> Result<Option<MediaPacket>, MediaError> {
        self.next_packet()
    }
}

impl<R: std::io::Read> PacketSource for crate::webm::WebmDemuxer<R> {
    fn tracks(&self) -> &[TrackSpec] {
        self.tracks()
    }
    fn next_packet(&mut self) -> Result<Option<MediaPacket>, MediaError> {
        self.next_packet()
    }
}

impl<W: std::io::Write> PacketSink for crate::webm::WebmMuxer<W> {
    fn write_packet(&mut self, packet: &MediaPacket) -> Result<(), MediaError> {
        self.write_packet(packet)
    }
}

/// Codec legality check for the WebM container — used by callers building
/// plans. Copy is only meaningful when the codec maps into the target.
pub fn codec_allowed_in_webm(codec: &Codec) -> bool {
    crate::webm::codec_to_webm(*codec).is_ok()
}
