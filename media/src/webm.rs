//! Bounded WebM/Matroska muxing and demuxing over blocking `Read`/`Write`.
//!
//! The muxer writes the streaming profile: Segment and each Cluster use a
//! declared-size payload buffered only up to `max_cluster_ticks` of media —
//! no seeking, no backpatching, no whole-file buffering. It emits SimpleBlocks
//! unless a packet needs a BlockGroup (DiscardPadding or explicit duration).
//!
//! The demuxer reads sequentially: EBML header, Segment, Info, Tracks, then
//! Clusters with SimpleBlock/BlockGroup, all three lacing modes. Unknown
//! elements are drained within declared sizes; unknown-size non-master
//! elements are rejected. Timestamp handling is exact rational arithmetic —
//! see `TickPolicy` for the admitted quantization at the container edge.

use crate::ebml as e;
use crate::time::{TimeBase, Timestamp};
use crate::track::{
    AudioInfo, Codec, MediaError, MediaLimits, MediaPacket, TrackKind, TrackSpec, VideoInfo,
};
use std::io::{Read, Write};

/// How the muxer converts exact track timestamps into TimestampScale ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickPolicy {
    /// Every conversion must be exact; otherwise `write_packet` errors.
    Exact,
    /// Round to the nearest tick (ties away from zero); the report records
    /// the packet count and worst absolute error in nanoseconds.
    Nearest,
}

/// Statistics the muxer reports — nothing is silently quantized.
#[derive(Clone, Debug, Default)]
pub struct MuxReport {
    pub packets_written: u64,
    pub clusters_written: u64,
    /// Packets whose timestamp did not land on a tick boundary (Nearest only).
    pub quantized_packets: u64,
    /// Worst absolute timestamp quantization, nanoseconds.
    pub max_tick_error_ns: i64,
    /// End-trim actually recorded via DiscardPadding, summed ns per track.
    pub discard_padding_written_ns: u64,
}

pub(crate) fn codec_to_webm(c: Codec) -> Result<&'static str, MediaError> {
    Ok(match c {
        Codec::Av1 => "V_AV1",
        Codec::Vp9 => "V_VP9",
        Codec::Vp8 => "V_VP8",
        Codec::H264 => "V_MPEG4/ISO/AVC",
        Codec::Opus => "A_OPUS",
        Codec::Aac => "A_AAC",
        Codec::Flac => "A_FLAC",
        _ => return Err(MediaError::Unsupported("codec has no WebM mapping")),
    })
}

fn webm_to_codec(s: &str) -> Codec {
    match s {
        "V_AV1" => Codec::Av1,
        "V_VP9" => Codec::Vp9,
        "V_VP8" => Codec::Vp8,
        "V_MPEG4/ISO/AVC" => Codec::H264,
        "A_OPUS" => Codec::Opus,
        "A_AAC" => Codec::Aac,
        "A_FLAC" => Codec::Flac,
        _ => Codec::Other("unknown"),
    }
}

/// Streaming WebM muxer. `W` never needs `Seek`.
pub struct WebmMuxer<W: Write> {
    w: W,
    scale_ns: u64,
    policy: TickPolicy,
    tracks: Vec<TrackSpec>,
    max_cluster_ticks: i64,
    open_cluster: Option<ClusterState>,
    report: MuxReport,
    finished: bool,
}

struct ClusterState {
    /// Cluster Timestamp element value (ticks).
    base_ticks: i64,
    /// Buffered children (SimpleBlock/BlockGroup bytes).
    buf: Vec<u8>,
}

impl<W: Write> WebmMuxer<W> {
    /// Write the EBML header, Segment (unknown size), Info and Tracks.
    /// `timestamp_scale_ns` is the Segment tick length (Matroska requires a
    /// positive integer; 1_000_000 = 1 ms is the WebM default).
    /// `max_cluster_ticks` bounds buffered media per cluster.
    pub fn new(
        mut w: W,
        timestamp_scale_ns: u64,
        tracks: &[TrackSpec],
        max_cluster_ticks: i64,
        policy: TickPolicy,
    ) -> Result<Self, MediaError> {
        if timestamp_scale_ns == 0 || timestamp_scale_ns > u32::MAX as u64 {
            return Err(MediaError::Unsupported("timestamp scale must fit u32 ns"));
        }
        if max_cluster_ticks <= 0 {
            return Err(MediaError::Contract("cluster tick bound must be positive"));
        }
        if tracks.is_empty() {
            return Err(MediaError::Contract("at least one track required"));
        }
        if tracks.len() > 126 {
            return Err(MediaError::Limit("WebM track number must fit 1-byte vint"));
        }

        // EBML header.
        let mut h = Vec::with_capacity(64);
        e::write_uint(&mut h, e::id::EBML_VERSION, 1)?;
        e::write_uint(&mut h, e::id::EBML_READ_VERSION, 1)?;
        e::write_uint(&mut h, e::id::EBML_MAX_ID_LENGTH, 4)?;
        e::write_uint(&mut h, e::id::EBML_MAX_SIZE_LENGTH, 8)?;
        e::write_utf8(&mut h, e::id::DOC_TYPE, "webm")?;
        e::write_uint(&mut h, e::id::DOC_TYPE_VERSION, 4)?;
        e::write_uint(&mut h, e::id::DOC_TYPE_READ_VERSION, 2)?;
        e::write_element(&mut w, e::id::EBML, &h)?;

        // Segment, unknown size — live-streaming profile.
        e::write_id(&mut w, e::id::SEGMENT)?;
        e::write_size(&mut w, None)?;

        // Info.
        let mut info = Vec::with_capacity(64);
        e::write_uint(&mut info, e::id::TIMESTAMP_SCALE, timestamp_scale_ns)?;
        e::write_utf8(&mut info, e::id::MUXING_APP, "zencodec-media")?;
        e::write_utf8(&mut info, e::id::WRITING_APP, "zencodec-media")?;
        e::write_element(&mut w, e::id::INFO, &info)?;

        // Tracks.
        let mut tk = Vec::with_capacity(256 * tracks.len());
        for (i, t) in tracks.iter().enumerate() {
            let mut te = Vec::with_capacity(128);
            let track_no = (i + 1) as u64;
            e::write_uint(&mut te, e::id::TRACK_NUMBER, track_no)?;
            e::write_uint(&mut te, e::id::TRACK_UID, track_no)?;
            let ttype = match t.kind {
                TrackKind::Video => 1,
                TrackKind::Audio => 2,
                TrackKind::Other(_) => {
                    return Err(MediaError::Unsupported("WebM carries only audio/video"));
                }
            };
            e::write_uint(&mut te, e::id::TRACK_TYPE, ttype)?;
            e::write_uint(&mut te, e::id::FLAG_ENABLED, 1)?;
            e::write_uint(&mut te, e::id::FLAG_DEFAULT, 1)?;
            // Lacing stays on: zero overhead for unlaced blocks, Opus benefits.
            e::write_uint(&mut te, e::id::FLAG_LACING, 1)?;
            e::write_utf8(&mut te, e::id::CODEC_ID, codec_to_webm(t.codec)?)?;
            if let Some(cp) = &t.codec_private {
                e::write_element(&mut te, e::id::CODEC_PRIVATE, cp)?;
            }
            if t.codec_delay_ns > 0 {
                e::write_uint(&mut te, e::id::CODEC_DELAY, t.codec_delay_ns)?;
            }
            if t.seek_preroll_ns > 0 {
                e::write_uint(&mut te, e::id::SEEK_PRE_ROLL, t.seek_preroll_ns)?;
            }
            if let Some(v) = t.video {
                let mut ve = Vec::with_capacity(16);
                e::write_uint(&mut ve, e::id::PIXEL_WIDTH, v.width as u64)?;
                e::write_uint(&mut ve, e::id::PIXEL_HEIGHT, v.height as u64)?;
                e::write_element(&mut te, e::id::VIDEO, &ve)?;
            }
            if let Some(a) = t.audio {
                let mut ae = Vec::with_capacity(24);
                // SamplingFrequency is an EBML float element — f64 IEEE.
                e::write_id(&mut ae, e::id::SAMPLING_FREQUENCY)?;
                e::write_size(&mut ae, Some(8))?;
                ae.write_all(&(a.sample_rate as f64).to_be_bytes())?;
                e::write_uint(&mut ae, e::id::CHANNELS, a.channels as u64)?;
                e::write_element(&mut te, e::id::AUDIO, &ae)?;
            }
            e::write_element(&mut tk, e::id::TRACK_ENTRY, &te)?;
        }
        e::write_element(&mut w, e::id::TRACKS, &tk)?;

        Ok(Self {
            w,
            scale_ns: timestamp_scale_ns,
            policy,
            tracks: tracks.to_vec(),
            max_cluster_ticks,
            open_cluster: None,
            report: MuxReport::default(),
            finished: false,
        })
    }

    /// Convert a track-timebase `Timestamp` into segment ticks per policy.
    /// Returns (ticks, quantization error in ns).
    fn ticks_for(&mut self, ts: Timestamp) -> Result<i64, MediaError> {
        let tb = ts.time_base();
        // pts_ns = ticks * num * 1e9 / den — exact i128 arithmetic.
        let ns =
            ts.ticks() as i128 * tb.numerator() as i128 * 1_000_000_000 / tb.denominator() as i128;
        let scaled = ns / self.scale_ns as i128;
        let exact = scaled * self.scale_ns as i128 == ns;
        if exact {
            return i64::try_from(scaled).map_err(|_| MediaError::Format("timestamp overflow"));
        }
        match self.policy {
            TickPolicy::Exact => Err(MediaError::Format(
                "timestamp not representable in segment tick scale",
            )),
            TickPolicy::Nearest => {
                let rem = ns - scaled * self.scale_ns as i128;
                let adj = if ns >= 0 {
                    scaled + i128::from(rem * 2 >= self.scale_ns as i128)
                } else {
                    scaled - i128::from(rem.abs() * 2 > self.scale_ns as i128)
                };
                let err = (ns - adj * self.scale_ns as i128).abs() as i64;
                self.report.quantized_packets += 1;
                if err > self.report.max_tick_error_ns {
                    self.report.max_tick_error_ns = err;
                }
                i64::try_from(adj).map_err(|_| MediaError::Format("timestamp overflow"))
            }
        }
    }

    /// Append a packet. Packets may arrive interleaved across tracks; each is
    /// buffered into the current cluster until it rolls. `pts` ordering across
    /// cluster boundaries is the caller's job (monotonic within a track).
    pub fn write_packet(&mut self, pkt: &MediaPacket) -> Result<(), MediaError> {
        if self.finished {
            return Err(MediaError::Contract("write after finish"));
        }
        let ti = pkt.track as usize;
        let spec = self
            .tracks
            .get(ti)
            .ok_or(MediaError::Contract("packet for unknown track"))?;
        if spec.config_epoch != pkt.config_epoch {
            return Err(MediaError::Contract(
                "codec config epoch changed without a track update",
            ));
        }
        let ticks = self.ticks_for(pkt.pts)?;

        // Roll the cluster when this timestamp can't be expressed relative to
        // the cluster base (block timestamp is s16) or the cap is exceeded.
        let need_new = match &self.open_cluster {
            None => true,
            Some(c) => {
                let rel = ticks - c.base_ticks;
                rel < i16::MIN as i64 || rel > i16::MAX as i64 || rel > self.max_cluster_ticks
            }
        };
        if need_new {
            self.close_cluster()?;
            // Cluster Timestamp is unsigned; a negative-pts packet still
            // expresses fine because block timestamps are signed i16 relative.
            self.open_cluster = Some(ClusterState {
                base_ticks: ticks.max(0),
                buf: Vec::new(),
            });
        }
        let c = self.open_cluster.as_mut().unwrap();
        let rel = (ticks - c.base_ticks) as i16;

        let mut block = Vec::with_capacity(pkt.data.len() + 8);
        // Track number as vint (tracks ≤ 126 → 1 byte).
        block.push(0x80 | (ti as u8 + 1));
        block.extend_from_slice(&rel.to_be_bytes());
        let flags = if pkt.keyframe { 0x80 } else { 0x00 }; // no lacing
        block.push(flags);
        block.extend_from_slice(&pkt.data);

        if pkt.discard_padding_ns.is_some() || pkt.duration_ticks.is_some() {
            // BlockGroup: Block (+BlockDuration) (+DiscardPadding).
            let mut bg = Vec::with_capacity(block.len() + 16);
            e::write_element(&mut bg, e::id::BLOCK, &block)?;
            if let Some(d) = pkt.duration_ticks {
                e::write_uint(&mut bg, e::id::BLOCK_DURATION, d as u64)?;
            }
            if let Some(dp) = pkt.discard_padding_ns {
                e::write_int(&mut bg, e::id::DISCARD_PADDING, dp)?;
                self.report.discard_padding_written_ns = self
                    .report
                    .discard_padding_written_ns
                    .saturating_add(dp.unsigned_abs());
            }
            e::write_element(&mut c.buf, e::id::BLOCK_GROUP, &bg)?;
        } else {
            e::write_element(&mut c.buf, e::id::SIMPLE_BLOCK, &block)?;
        }
        self.report.packets_written += 1;
        Ok(())
    }

    fn close_cluster(&mut self) -> Result<(), MediaError> {
        let Some(c) = self.open_cluster.take() else {
            return Ok(());
        };
        let mut payload = Vec::with_capacity(c.buf.len() + 12);
        e::write_uint(&mut payload, e::id::TIMESTAMP, c.base_ticks.max(0) as u64)?;
        payload.extend_from_slice(&c.buf);
        e::write_element(&mut self.w, e::id::CLUSTER, &payload)?;
        self.report.clusters_written += 1;
        Ok(())
    }

    /// Flush the open cluster and hand back the writer plus the report.
    /// Segment stays unknown-size; no trailing Cues (streaming profile).
    pub fn finish(mut self) -> Result<(W, MuxReport), MediaError> {
        self.close_cluster()?;
        self.finished = true;
        Ok((self.w, self.report.clone()))
    }

    /// Access the running counters (e.g. for a mid-stream progress read).
    pub fn report(&self) -> &MuxReport {
        &self.report
    }

    /// Consumes without finishing — for callers that manage teardown. The
    /// open cluster, if any, is flushed first.
    pub fn flush(&mut self) -> Result<(), MediaError> {
        self.close_cluster()
    }
}

// ---------------------------------------------------------------------------
// Demux side
// ---------------------------------------------------------------------------

/// Streaming WebM/Matroska demuxer over blocking `Read`. Packet timestamps are
/// `Timestamp`s on `TimeBase { scale_ns / 1e9 }` — exact rational, no floats.
pub struct WebmDemuxer<R: Read> {
    r: Counted<R>,
    limits: MediaLimits,
    tracks: Vec<TrackSpec>,
    /// Segment tick length in ns.
    scale_ns: u64,
    /// Current cluster base timestamp in ticks.
    cluster_ticks: i64,
    /// BlockGroup packets decoded ahead of time (a BlockGroup's Block laces
    /// multiple frames; SimpleBlock does too).
    pending: std::collections::VecDeque<MediaPacket>,
    /// Per-track packet ordinals.
    ordinals: Vec<u64>,
    /// Segment payload exhausted.
    ended: bool,
    failed: bool,
    /// Current cluster's remaining payload size (None while between clusters).
    cluster_left: Option<u64>,
    /// Segment payload remaining, when known.
    segment_left: Option<u64>,
}

/// Reader that counts bytes (for byte offsets + bounded header scans).
struct Counted<R> {
    inner: R,
    pos: u64,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Read> Counted<R> {
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), MediaError> {
        self.inner.read_exact(buf)?;
        self.pos += buf.len() as u64;
        Ok(())
    }
    fn drain(&mut self, mut n: u64) -> Result<(), MediaError> {
        let mut buf = [0u8; 8192];
        while n > 0 {
            let take = n.min(buf.len() as u64) as usize;
            self.read_exact(&mut buf[..take])?;
            n -= take as u64;
        }
        Ok(())
    }
}

fn read_payload<C: Read>(r: &mut Counted<C>, size: u64, max: u64) -> Result<Vec<u8>, MediaError> {
    if size > max {
        return Err(MediaError::Limit("element payload exceeds limit"));
    }
    let mut v = Vec::new();
    v.try_reserve_exact(size as usize)
        .map_err(|_| MediaError::Limit("allocation refused"))?;
    v.resize(size as usize, 0);
    r.read_exact(&mut v)?;
    Ok(v)
}

impl<R: Read> WebmDemuxer<R> {
    /// Parse the EBML header, Segment header, and scan to the first Cluster —
    /// recording Info/Tracks on the way. `tracks()` is valid after `new`.
    pub fn new(reader: R, limits: MediaLimits) -> Result<Self, MediaError> {
        let mut r = Counted {
            inner: reader,
            pos: 0,
        };
        // EBML header.
        let h = e::read_element_header(&mut r)?;
        if h.id != e::id::EBML {
            return Err(MediaError::Format("missing EBML header"));
        }
        let hb = read_payload(
            &mut r,
            h.size.ok_or(MediaError::Format("EBML unknown size"))?,
            limits.max_header_bytes,
        )?;
        let doc = parse_ebml_doc_type(&hb)?;
        if doc != "webm" && doc != "matroska" {
            return Err(MediaError::Unsupported("EBML DocType is not webm/matroska"));
        }
        // Segment — may be unknown size.
        let s = e::read_element_header(&mut r)?;
        if s.id != e::id::SEGMENT {
            return Err(MediaError::Format("missing Segment"));
        }
        let mut dm = Self {
            r,
            limits,
            tracks: Vec::new(),
            scale_ns: 1_000_000,
            cluster_ticks: 0,
            pending: std::collections::VecDeque::new(),
            ordinals: Vec::new(),
            ended: false,
            failed: false,
            cluster_left: None,
            segment_left: s.size,
        };
        dm.scan_to_first_cluster()?;
        Ok(dm)
    }

    /// Walk segment children until the first Cluster, capturing Info/Tracks.
    fn scan_to_first_cluster(&mut self) -> Result<(), MediaError> {
        let mut scanned = 0u64;
        loop {
            if let Some(0) = self.segment_left {
                self.ended = true;
                return Ok(());
            }
            let h = match e::read_element_header(&mut self.r) {
                Ok(h) => h,
                Err(MediaError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.ended = true;
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            if let Some(left) = &mut self.segment_left {
                *left = left
                    .checked_sub(h.header_len + h.size.unwrap_or(0))
                    .ok_or(MediaError::Format("element overruns segment"))?;
            }
            scanned += h.header_len;
            if scanned > self.limits.max_header_bytes {
                return Err(MediaError::Limit("header scan exceeded limit"));
            }
            match h.id {
                e::id::INFO => {
                    let body = read_payload(
                        &mut self.r,
                        size_or_limit(&h, self.limits.max_header_bytes)?,
                        self.limits.max_header_bytes,
                    )?;
                    if let Some(s) = parse_timestamp_scale(&body)? {
                        self.scale_ns = s;
                    }
                }
                e::id::TRACKS => {
                    let body = read_payload(
                        &mut self.r,
                        size_or_limit(&h, self.limits.max_header_bytes)?,
                        self.limits.max_header_bytes,
                    )?;
                    self.tracks = parse_tracks(&body, self.limits)?;
                    self.ordinals = vec![0; self.tracks.len()];
                }
                e::id::CLUSTER => {
                    // Cluster body is read incrementally by next_packet.
                    self.cluster_left = h.size; // None = unknown → till EOF/next.
                    return Ok(());
                }
                _ => {
                    // SeekHead, Cues, Tags, Chapters, Attachments, Void, CRC.
                    match h.size {
                        Some(sz) => self.r.drain(sz)?,
                        None => {
                            return Err(MediaError::Format(
                                "unknown-size element outside Segment/Cluster",
                            ));
                        }
                    }
                }
            }
        }
    }

    /// All declared tracks, in TrackEntry order.
    pub fn tracks(&self) -> &[TrackSpec] {
        &self.tracks
    }

    /// Segment tick length in nanoseconds (TimestampScale).
    pub fn timestamp_scale_ns(&self) -> u64 {
        self.scale_ns
    }

    /// Next packet in storage order, or `None` at end of segment.
    /// Errors poison the demuxer (subsequent calls fail).
    pub fn next_packet(&mut self) -> Result<Option<MediaPacket>, MediaError> {
        if self.failed {
            return Err(MediaError::Contract("demuxer poisoned by earlier error"));
        }
        match self.next_inner() {
            Ok(v) => Ok(v),
            Err(e) => {
                self.failed = true;
                Err(e)
            }
        }
    }

    fn next_inner(&mut self) -> Result<Option<MediaPacket>, MediaError> {
        loop {
            if let Some(p) = self.pending.pop_front() {
                return Ok(Some(p));
            }
            if self.ended {
                return Ok(None);
            }
            // Inside a cluster?
            if self.cluster_left == Some(0) {
                self.cluster_left = None;
                continue;
            }
            let h = match e::read_element_header(&mut self.r) {
                Ok(h) => h,
                Err(MediaError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.ended = true;
                    return Ok(None);
                }
                Err(e) => return Err(e),
            };
            if let Some(left) = &mut self.cluster_left {
                *left = left
                    .checked_sub(h.header_len + h.size.unwrap_or(0))
                    .ok_or(MediaError::Format("element overruns cluster"))?;
            }
            match h.id {
                e::id::TIMESTAMP => {
                    let body = read_payload(&mut self.r, h.size.unwrap_or(0), 8)?;
                    self.cluster_ticks = parse_uint(&body) as i64;
                }
                e::id::SIMPLE_BLOCK => {
                    let body = read_payload(
                        &mut self.r,
                        size_or_limit(&h, self.limits.max_packet_bytes as u64)?,
                        self.limits.max_packet_bytes as u64,
                    )?;
                    self.decode_block(&body, true, None, None)?;
                }
                e::id::BLOCK_GROUP => {
                    let size = h
                        .size
                        .ok_or(MediaError::Format("block group unknown size"))?;
                    let body =
                        read_payload(&mut self.r, size, self.limits.max_packet_bytes as u64)?;
                    self.decode_block_group(&body)?;
                }
                e::id::CLUSTER => {
                    // A new Cluster closes the previous one.
                    self.cluster_left = h.size;
                }
                _ => match h.size {
                    Some(sz) => self.r.drain(sz)?,
                    None => return Err(MediaError::Format("unknown-size element inside cluster")),
                },
            }
        }
    }

    /// Parse a BlockGroup body: Block (+Duration, DiscardPadding, ReferenceBlock).
    fn decode_block_group(&mut self, body: &[u8]) -> Result<(), MediaError> {
        let mut cur = std::io::Cursor::new(body);
        let mut block: Option<Vec<u8>> = None;
        let mut dur: Option<u64> = None;
        let mut dp: Option<i64> = None;
        while (cur.position() as usize) < body.len() {
            let h = e::read_element_header(&mut cur)?;
            let size = h
                .size
                .ok_or(MediaError::Format("unknown size in block group"))?;
            let start = cur.position() as usize;
            let end = start
                .checked_add(size as usize)
                .filter(|&e| e <= body.len())
                .ok_or(MediaError::Format("block group child overruns"))?;
            match h.id {
                e::id::BLOCK => block = Some(body[start..end].to_vec()),
                e::id::BLOCK_DURATION => dur = Some(parse_uint(&body[start..end])),
                e::id::DISCARD_PADDING => dp = Some(parse_int(&body[start..end])),
                _ => {} // ReferenceBlock et al: parsed but not needed
            }
            cur.set_position(end as u64);
        }
        let block = block.ok_or(MediaError::Format("block group without Block"))?;
        self.decode_block(&block, false, dur, dp)
    }

    /// Decode a SimpleBlock/Block payload: vint track, s16 rel ticks, flags,
    /// then laced frames. Emits MediaPackets into `pending`.
    fn decode_block(
        &mut self,
        body: &[u8],
        simple: bool,
        duration: Option<u64>,
        discard_padding: Option<i64>,
    ) -> Result<(), MediaError> {
        let mut cur = std::io::Cursor::new(body);
        let (track_no, _) = e::read_vint(&mut cur, false)?;
        let mut tb = [0u8; 2];
        cur.read_exact(&mut tb).map_err(MediaError::Io)?;
        let rel = i16::from_be_bytes(tb);
        let mut fb = [0u8; 1];
        cur.read_exact(&mut fb).map_err(MediaError::Io)?;
        let flags = fb[0];
        let keyframe = flags & 0x80 != 0;
        let invisible = flags & 0x08 != 0;
        let lacing = (flags >> 1) & 0x03;
        let _ = invisible;
        let _ = simple;

        let rest = &body[cur.position() as usize..];
        let frames: Vec<&[u8]> = match lacing {
            0 => vec![rest],
            1 => unlace_xiph(rest)?,
            2 => unlace_fixed(rest)?,
            3 => unlace_ebml(rest)?,
            _ => unreachable!(),
        };

        let ti = (track_no as usize)
            .checked_sub(1)
            .ok_or(MediaError::Format("track number 0"))?;
        if ti >= self.tracks.len() {
            return Err(MediaError::Format("block for undeclared track"));
        }
        let tb_scale = TimeBase::new(self.scale_ns as u32, 1_000_000_000)
            .map_err(|_| MediaError::Format("bad timestamp scale"))?;
        let n = frames.len();
        for (i, f) in frames.into_iter().enumerate() {
            let ord = self.ordinals[ti];
            self.ordinals[ti] += 1;
            let pts = Timestamp::new(self.cluster_ticks + rel as i64, tb_scale);
            self.pending.push_back(MediaPacket {
                track: ti as u32,
                ordinal: ord,
                config_epoch: 0,
                data: f.to_vec(),
                pts,
                dts: None,
                // Laced frames share the block's duration; only the last gets
                // DiscardPadding (trim applies to the block's tail).
                duration_ticks: duration.map(|d| (d / n as u64) as u32).filter(|_| true),
                keyframe,
                discard_padding_ns: if i == n - 1 { discard_padding } else { None },
            });
        }
        Ok(())
    }
}

fn size_or_limit(h: &e::ElementHeader, _limit: u64) -> Result<u64, MediaError> {
    // Bounded allocation happens inside read_payload; here we only reject
    // unknown-size leaf elements.
    h.size
        .ok_or(MediaError::Format("unknown-size leaf element"))
}

// --- EBML payload parsers ---------------------------------------------------

/// Walk a master element's body calling `f(child_id, payload_bytes)`.
fn walk_children(body: &[u8], mut f: impl FnMut(u64, &[u8])) -> Result<(), MediaError> {
    let mut cur = std::io::Cursor::new(body);
    while (cur.position() as usize) < body.len() {
        let h = e::read_element_header(&mut cur)?;
        let size = h.size.ok_or(MediaError::Format("unknown-size child"))? as usize;
        let start = cur.position() as usize;
        let end = start
            .checked_add(size)
            .filter(|&e| e <= body.len())
            .ok_or(MediaError::Format("child overruns master"))?;
        f(h.id, &body[start..end]);
        cur.set_position(end as u64);
    }
    Ok(())
}

fn parse_uint(b: &[u8]) -> u64 {
    let mut v = 0u64;
    for &x in b.iter().take(8) {
        v = (v << 8) | x as u64;
    }
    v
}

fn parse_int(b: &[u8]) -> i64 {
    let mut v = if b.first().is_some_and(|&x| x & 0x80 != 0) {
        -1i64
    } else {
        0
    };
    for &x in b.iter().take(8) {
        v = (v << 8) | i64::from(x);
    }
    v
}

fn parse_f64(b: &[u8]) -> Option<f64> {
    match b.len() {
        4 => Some(f32::from_be_bytes(b.try_into().ok()?) as f64),
        8 => Some(f64::from_be_bytes(b.try_into().ok()?)),
        _ => None,
    }
}

fn parse_ebml_doc_type(body: &[u8]) -> Result<String, MediaError> {
    let mut doc = String::new();
    walk_children(body, |id, p| {
        if id == e::id::DOC_TYPE {
            doc = String::from_utf8_lossy(p).into_owned();
        }
    })?;
    Ok(doc)
}

fn parse_timestamp_scale(body: &[u8]) -> Result<Option<u64>, MediaError> {
    let mut v = None;
    walk_children(body, |id, p| {
        if id == e::id::TIMESTAMP_SCALE {
            v = Some(parse_uint(p));
        }
    })?;
    Ok(v)
}

fn parse_tracks(body: &[u8], limits: MediaLimits) -> Result<Vec<TrackSpec>, MediaError> {
    let mut out = Vec::new();
    let mut err = None;
    walk_children(body, |id, p| {
        if id == e::id::TRACK_ENTRY && err.is_none() {
            match parse_track_entry(p) {
                Ok(t) => out.push(t),
                Err(e) => err = Some(e),
            }
        }
    })?;
    if let Some(e) = err {
        return Err(e);
    }
    if out.is_empty() {
        return Err(MediaError::Format("no tracks"));
    }
    if out.len() > limits.max_tracks as usize {
        return Err(MediaError::Limit("too many tracks"));
    }
    // Renumber to 0-based demuxer indices (track number is the muxer-facing id).
    for (i, t) in out.iter_mut().enumerate() {
        t.index = i as u32;
    }
    Ok(out)
}

fn parse_track_entry(body: &[u8]) -> Result<TrackSpec, MediaError> {
    let mut codec = Codec::Other("unknown");
    let mut kind = TrackKind::Other("unknown");
    let mut private = None;
    let mut video = None;
    let mut audio = None;
    let mut codec_delay = 0u64;
    let mut preroll = 0u64;
    let mut declared_dur = None;
    let mut err = None;
    walk_children(body, |id, p| match id {
        e::id::TRACK_TYPE => {
            kind = match parse_uint(p) {
                1 => TrackKind::Video,
                2 => TrackKind::Audio,
                _ => TrackKind::Other("non-av"),
            }
        }
        e::id::CODEC_ID => codec = webm_to_codec(std::str::from_utf8(p).unwrap_or("")),
        e::id::CODEC_PRIVATE => private = Some(p.to_vec()),
        e::id::CODEC_DELAY => codec_delay = parse_uint(p),
        e::id::SEEK_PRE_ROLL => preroll = parse_uint(p),
        e::id::DEFAULT_DURATION => declared_dur = Some(parse_uint(p)),
        e::id::VIDEO => {
            let (mut w, mut h) = (0u64, 0u64);
            if let Err(e) = walk_children(p, |id, pp| {
                if id == e::id::PIXEL_WIDTH {
                    w = parse_uint(pp);
                }
                if id == e::id::PIXEL_HEIGHT {
                    h = parse_uint(pp);
                }
            }) {
                err.get_or_insert(e);
            }
            video = Some(VideoInfo {
                width: w.min(u32::MAX as u64) as u32,
                height: h.min(u32::MAX as u64) as u32,
            });
        }
        e::id::AUDIO => {
            let (mut sr, mut ch) = (0f64, 0u64);
            if let Err(e) = walk_children(p, |id, pp| {
                if id == e::id::SAMPLING_FREQUENCY {
                    sr = parse_f64(pp).unwrap_or(0.0);
                }
                if id == e::id::CHANNELS {
                    ch = parse_uint(pp);
                }
            }) {
                err.get_or_insert(e);
            }
            audio = Some(AudioInfo {
                sample_rate: sr.round().max(0.0).min(u32::MAX as f64) as u32,
                channels: ch.min(u16::MAX as u64) as u16,
            });
        }
        _ => {}
    })?;
    if let Some(e) = err {
        return Err(e);
    }
    Ok(TrackSpec {
        index: 0,
        kind,
        codec,
        codec_private: private,
        // Placeholder; the demuxer normalizes to the segment scale after open.
        time_base: TimeBase::new(1_000_000, 1_000_000_000).unwrap(),
        video,
        audio,
        codec_delay_ns: codec_delay,
        seek_preroll_ns: preroll,
        config_epoch: 0,
        declared_packets: None,
        declared_duration: declared_dur,
        edit_delay_ticks: None,
    })
}

// --- lacing -----------------------------------------------------------------

fn unlace_xiph(b: &[u8]) -> Result<Vec<&[u8]>, MediaError> {
    let (&count, mut rest) = b.split_first().ok_or(MediaError::Format("empty lace"))?;
    let n = count as usize + 1;
    let mut sizes = Vec::with_capacity(n);
    let mut total = 0usize;
    for _ in 0..n - 1 {
        let mut sz = 0usize;
        loop {
            let (&x, r2) = rest
                .split_first()
                .ok_or(MediaError::Format("xiph lace oob"))?;
            rest = r2;
            sz += x as usize;
            if x != 255 {
                break;
            }
        }
        sizes.push(sz);
        total += sz;
    }
    if total > rest.len() {
        return Err(MediaError::Format("xiph lace exceeds block"));
    }
    let mut out = Vec::with_capacity(n);
    for &sz in &sizes {
        out.push(&rest[..sz]);
        rest = &rest[sz..];
    }
    out.push(rest); // last frame's size is implicit: the remainder
    Ok(out)
}

fn unlace_fixed(b: &[u8]) -> Result<Vec<&[u8]>, MediaError> {
    let (&count, rest) = b.split_first().ok_or(MediaError::Format("empty lace"))?;
    let n = count as usize + 1;
    if n == 0 || rest.len() % n != 0 {
        return Err(MediaError::Format("fixed lace indivisible"));
    }
    let sz = rest.len() / n;
    Ok((0..n).map(|i| &rest[i * sz..(i + 1) * sz]).collect())
}

fn unlace_ebml(b: &[u8]) -> Result<Vec<&[u8]>, MediaError> {
    let (&count, mut rest) = b.split_first().ok_or(MediaError::Format("empty lace"))?;
    let n = count as usize + 1;
    // First size is a plain vint; subsequent are signed-delta vints.
    let (first, l1) = read_vint_slice(rest, false)?;
    rest = &rest[l1..];
    let mut sizes = vec![first as usize];
    let mut acc = first as i64;
    for _ in 1..n - 1 {
        let (sv, l) = read_vint_slice(rest, false)?;
        // Signed vint: bias = (1 << (7*l - 1)) - 1.
        let bias = (1i64 << (7 * l - 1)) - 1;
        acc += sv as i64 - bias;
        if acc < 0 {
            return Err(MediaError::Format("ebml lace negative size"));
        }
        sizes.push(acc as usize);
        rest = &rest[l..];
    }
    let used: usize = sizes.iter().sum();
    if used > rest.len() {
        return Err(MediaError::Format("ebml lace exceeds block"));
    }
    let mut out = Vec::with_capacity(n);
    for &sz in &sizes {
        out.push(&rest[..sz]);
        rest = &rest[sz..];
    }
    out.push(rest); // last frame's size is implicit: the remainder
    Ok(out)
}

/// VINT read over a byte slice (same length-prefix rules as `read_vint`).
fn read_vint_slice(b: &[u8], keep_marker: bool) -> Result<(u64, usize), MediaError> {
    let (&first, _) = b.split_first().ok_or(MediaError::Format("empty vint"))?;
    if first == 0 {
        return Err(MediaError::Format("vint zero first byte"));
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || b.len() < len {
        return Err(MediaError::Format("vint truncated"));
    }
    let mut v = if keep_marker {
        first as u64
    } else {
        (first & (0xFF >> len)) as u64
    };
    for &x in &b[1..len] {
        v = (v << 8) | x as u64;
    }
    Ok((v, len))
}
