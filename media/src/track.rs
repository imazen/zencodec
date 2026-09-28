//! Container-agnostic track and packet contracts for bounded media sessions.
//!
//! A demuxer opens a `TrackSet`, reports every declared track, and yields
//! `MediaPacket`s in decode order. A muxer accepts `TrackSpec`s up front and
//! interleaved `MediaPacket`s in any order; ordering is the caller's (or a
//! session layer's) responsibility. Timestamps are exact rationals; audio uses
//! exact sample counts. Unsupported container features are explicit errors —
//! never silent drops.
//!
//! Packet identity is (track_index, ordinal). A codec's bitstream configuration
//! can change mid-stream (MP4 `stsd` entries, in-band parameter sets): each
//! track carries a monotonically increasing `config_epoch`, and every packet
//! records the epoch it was decoded under. A muxer that sees an epoch change
//! without a matching `TrackSpec::config_update` must fail loudly.

use crate::time::TimeBase;
use std::fmt;

/// Container-agnostic codec identity. Container mappings live in the
/// container modules (`mp4`, `webm`); this enum is the shared vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Codec {
    Av1,
    Vp9,
    Vp8,
    H264,
    Aac,
    Opus,
    Flac,
    /// Recognized but not otherwise supported: `fourcc`/`id` preserved.
    Other(&'static str),
}

impl Codec {
    /// Stable short name for diagnostics and reports.
    pub fn name(self) -> &'static str {
        match self {
            Codec::Av1 => "av1",
            Codec::Vp9 => "vp9",
            Codec::Vp8 => "vp8",
            Codec::H264 => "h264",
            Codec::Aac => "aac",
            Codec::Opus => "opus",
            Codec::Flac => "flac",
            Codec::Other(n) => n,
        }
    }
}

/// Track class. Only video and audio exist in the first route; subtitle/data
/// tracks parse as explicit `Unsupported` tracks rather than being skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackKind {
    Video,
    Audio,
    /// Anything else: subtitle, metadata, chapters. Muxers reject them.
    Other(&'static str),
}

/// Video parameters a muxer needs before the first packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
}

/// Audio parameters a muxer needs before the first packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    /// Sampling rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
}

/// A track offered by a demuxer or requested of a muxer.
///
/// `codec_private` is the container's codec configuration record (avcC,
/// esds audio config, OpusHead, …). It is *opaque bytes* here; adapter code
/// interprets it. `None` means the container carries no record and the codec
/// is self-configuring from packet payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackSpec {
    /// Demuxer-assigned stable index into `TrackSet::tracks` / muxer slot.
    pub index: u32,
    pub kind: TrackKind,
    pub codec: Codec,
    /// Container-specific codec configuration record, if any.
    pub codec_private: Option<Vec<u8>>,
    /// Tick units for this track's timestamps (MP4 timescale; Matroska is
    /// normalized to the segment timebase at the boundary).
    pub time_base: TimeBase,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
    /// Decoder delay in nanoseconds (Opus pre-skip; AAC priming is carried via
    /// `edit_delay_ticks`/iTunSMPB instead on the MP4 side). Matroska CodecDelay
    /// is natively ns and not scaled by TimestampScale.
    pub codec_delay_ns: u64,
    /// Random-access preroll in nanoseconds (Opus SeekPreRoll; H.264 open-GOP
    /// recovery belongs to the decoder, not the container).
    pub seek_preroll_ns: u64,
    /// Codec configuration epoch: increments when `codec_private` changes.
    pub config_epoch: u64,
    /// Total declared sample/frame count when the container knows it.
    pub declared_packets: Option<u64>,
    /// Declared media duration in track ticks when present.
    pub declared_duration: Option<u64>,
    /// Edit-list media delay in track ticks (MP4 elst media_time) — the gap
    /// between decode order and presentation start, e.g. AAC encoder priming.
    /// `None` = none declared (not "zero").
    pub edit_delay_ticks: Option<i64>,
}

impl TrackSpec {
    /// Compact description for errors and reports.
    pub fn describe(&self) -> String {
        let mut s = format!("track {} {:?} {}", self.index, self.kind, self.codec.name());
        if let Some(v) = self.video {
            s += &format!(" {}x{}", v.width, v.height);
        }
        if let Some(a) = self.audio {
            s += &format!(" {}Hz {}ch", a.sample_rate, a.channels);
        }
        s
    }
}

/// One compressed access unit on one track.
///
/// `pts`/`dts` are exact `Timestamp`s on the track's own `TimeBase`. `dts`
/// being `None` means decode order == presentation order (audio, all-intra).
/// `duration` is optional because many containers store only start times; a
/// packet without one inherits the next packet's start minus its own (the
/// session layer computes, the muxer may require it).
#[derive(Clone, Debug)]
pub struct MediaPacket {
    /// Index into the session's track list.
    pub track: u32,
    /// Monotonic per-track ordinal — packets are uniquely (track, ordinal).
    pub ordinal: u64,
    /// Config epoch this packet was demuxed under (`TrackSpec::config_epoch`).
    pub config_epoch: u64,
    /// Compressed payload. Allocation was already bounded by the demuxer's
    /// `max_packet_bytes`; muxers may impose their own cap.
    pub data: Vec<u8>,
    /// Presentation timestamp.
    pub pts: crate::time::Timestamp,
    /// Decode timestamp when the container stores one (MP4 stts); `None` when
    /// presentation order is the storage order.
    pub dts: Option<crate::time::Timestamp>,
    /// Sample duration in track ticks when the container records it.
    pub duration_ticks: Option<u32>,
    /// Sync sample / keyframe flag.
    pub keyframe: bool,
    /// Matroska DiscardPadding in nanoseconds: positive trims the end of this
    /// block, negative trims its start. This is how WebM expresses exact Opus
    /// end-trim — surfaced so the session layer can keep sample counts exact.
    pub discard_padding_ns: Option<i64>,
}

/// Hard bounds every demuxer/muxer accepts. Zero limits are "no limit" only
/// where explicitly documented per field — otherwise zero means "reject all".
#[derive(Clone, Copy, Debug)]
pub struct MediaLimits {
    /// Per-packet compressed payload cap, checked *before* allocation.
    pub max_packet_bytes: usize,
    /// Total tracks admitted from one container.
    pub max_tracks: u32,
    /// Header/metadata region cap (moov, Segment headers, codec_private).
    pub max_header_bytes: u64,
    /// Per-track sample-table entries (MP4 stts/stsz rows) admitted.
    pub max_table_entries: u32,
    /// Maximum buffered packets for stream-order muxing (bounded reordering
    /// window the muxer is allowed to hold).
    pub max_mux_queue: usize,
}

impl Default for MediaLimits {
    fn default() -> Self {
        Self {
            max_packet_bytes: 64 * 1024 * 1024,
            max_tracks: 64,
            max_header_bytes: 256 * 1024 * 1024,
            max_table_entries: 4 * 1024 * 1024,
            max_mux_queue: 1024,
        }
    }
}

/// What the caller wants done with a demuxed track — made explicit so a track
/// can never be silently dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackDecision {
    /// Pass compressed packets through; requires the output container to
    /// carry the codec (checked at muxer open, not at packet time).
    Copy,
    /// Decode and re-encode on this session's pipeline.
    Reencode,
    /// Intentionally exclude. Recorded in the session report.
    Drop,
}

/// Per-track outcome record: the "no silent audio drop" audit trail.
#[derive(Clone, Debug)]
pub struct TrackOutcome {
    pub index: u32,
    pub kind: TrackKind,
    pub codec: Codec,
    pub decision: TrackDecision,
    /// Output track index when copied/reencoded into a muxer.
    pub output_track: Option<u32>,
    /// Packets admitted on the input side.
    pub packets_in: u64,
    /// Packets written to the output. For reencode, input ≠ output is normal
    /// (decoder delay, frame reordering); equality is *not* asserted.
    pub packets_out: u64,
    /// Exact first/last presentation timestamps seen, when any.
    pub first_pts: Option<crate::time::Timestamp>,
    pub last_pts: Option<crate::time::Timestamp>,
    /// Audio only: exact sample counts decoded / encoded.
    pub samples_in: Option<u64>,
    pub samples_out: Option<u64>,
}

/// Errors produced by container/session layers. `std::io::Error` carriers are
/// for I/O; these are contract/format violations.
#[derive(Debug)]
#[non_exhaustive]
pub enum MediaError {
    /// Input violates container structure or exceeds a declared limit.
    Format(&'static str),
    /// Codec/track combination the container cannot express.
    Unsupported(&'static str),
    /// A limit in `MediaLimits` was exceeded.
    Limit(&'static str),
    /// Caller violated session ordering (e.g. packet for unknown track).
    Contract(&'static str),
    /// Underlying I/O.
    Io(std::io::Error),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::Format(m) => write!(f, "format: {m}"),
            MediaError::Unsupported(m) => write!(f, "unsupported: {m}"),
            MediaError::Limit(m) => write!(f, "limit: {m}"),
            MediaError::Contract(m) => write!(f, "contract: {m}"),
            MediaError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for MediaError {}

impl From<std::io::Error> for MediaError {
    fn from(e: std::io::Error) -> Self {
        MediaError::Io(e)
    }
}
