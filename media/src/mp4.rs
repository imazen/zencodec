//! Bounded ISO BMFF (MP4) demuxing over `Read + Seek`.
//!
//! Classic (non-fragmented) files: `moov` sample tables are parsed once under
//! `max_header_bytes`; packet payloads are fetched lazily from `mdat` at
//! computed offsets — no whole-file buffering, no expanded per-sample tables.
//! Fragmented MP4 (`moof`) and compact size tables (`stz2`) are explicit
//! `Unsupported` errors, not silent misreads. `stsd` multi-descriptor changes
//! surface as `config_epoch` bumps on packets.
//!
//! Timestamps: `stts` gives decode-order deltas; `ctts` (v0 unsigned / v1
//! signed) adds composition offsets; `elst` media_time becomes
//! `TrackSpec::edit_delay_ticks`. All on the track's `mdhd` timescale.

use crate::time::{TimeBase, Timestamp};
use crate::track::{
    AudioInfo, Codec, MediaError, MediaLimits, MediaPacket, TrackKind, TrackSpec, VideoInfo,
};
use std::io::{Read, Seek, SeekFrom};

fn fmt(m: &'static str) -> MediaError {
    MediaError::Format(m)
}

/// Big-endian readers over a byte slice cursor.
struct Rd<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }
    fn left(&self) -> usize {
        self.b.len() - self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], MediaError> {
        if self.left() < n {
            return Err(fmt("truncated field"));
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, MediaError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, MediaError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u24(&mut self) -> Result<u32, MediaError> {
        let b = self.take(3)?;
        Ok((b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32)
    }
    fn u32(&mut self) -> Result<u32, MediaError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, MediaError> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, MediaError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, MediaError> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn skip(&mut self, n: usize) -> Result<(), MediaError> {
        self.take(n).map(|_| ())
    }
    fn fourcc(&mut self) -> Result<[u8; 4], MediaError> {
        Ok(self.take(4)?.try_into().unwrap())
    }
}

/// One parsed box header: `payload` is the body slice range within its parent.
#[derive(Clone, Copy, Debug)]
struct BoxHdr {
    fourcc: [u8; 4],
    /// Byte offset of the payload within the parsed buffer/file region.
    payload_start: usize,
    payload_len: u64,
}

fn box_hdr(r: &mut Rd, base: usize) -> Result<Option<BoxHdr>, MediaError> {
    if r.left() == 0 {
        return Ok(None);
    }
    if r.left() < 8 {
        return Err(fmt("truncated box header"));
    }
    let size32 = r.u32()?;
    let cc = r.fourcc()?;
    let (size, hdr) = match size32 {
        1 => (r.u64()?, 16usize),
        0 => ((r.left() + 8) as u64, 8usize), // extends to end of parent
        s => (s as u64, 8usize),
    };
    if size < hdr as u64 {
        return Err(fmt("box smaller than its header"));
    }
    let payload_len = size - hdr as u64;
    if payload_len > r.left() as u64 {
        return Err(fmt("box overruns parent"));
    }
    Ok(Some(BoxHdr {
        fourcc: cc,
        payload_start: base + r.pos,
        payload_len,
    }))
}

/// `stsd` sample-description record: one entry per config epoch.
struct SampleDesc {
    codec: Codec,
    codec_private: Option<Vec<u8>>,
    video: Option<VideoInfo>,
    audio: Option<AudioInfo>,
}

struct Mp4Track {
    spec: TrackSpec,
    /// stts entries: (sample_count, delta).
    stts: Vec<(u32, u32)>,
    /// ctts entries: (sample_count, offset); signed on v1.
    ctts: Vec<(u32, i32)>,
    /// stsz: fixed size (0 → per-sample table) + optional per-sample sizes.
    stsz_const: u32,
    stsz: Vec<u32>,
    /// stsc runs: (first_chunk_1based, samples_per_chunk, desc_idx_1based).
    stsc: Vec<(u32, u32, u32)>,
    /// Chunk byte offsets (stco/co64).
    chunk_offsets: Vec<u64>,
    /// stss 1-based sample numbers, or None = all sync.
    sync: Option<Vec<u32>>,
    /// Per-sample-description codec info, index = desc_idx - 1.
    descs: Vec<SampleDesc>,
}

/// Lazy per-sample cursor over one track's tables.
struct SampleCursor {
    sample: u64,         // 0-based sample number
    chunk: usize,        // 0-based chunk number
    in_chunk: u64,       // samples already consumed in current chunk
    chunk_byte_off: u64, // running byte offset inside current chunk
    dts: u64,            // running decode timestamp in track ticks
    stts_i: usize,
    stts_left: u32,
    ctts_i: usize,
    ctts_left: u32,
    sync_i: usize,
}

/// MP4 demuxer. Requires `Seek` because `moov` may follow `mdat`; for
/// nonseekable transports, `source::SeekableSource` or `http::HttpRangeReader`
/// provides the snapshot/range behavior.
pub struct Mp4Demuxer<R> {
    r: R,
    tracks: Vec<Mp4Track>,
    /// Cursor per track; merged decode-order emission happens across tracks.
    cursors: Vec<SampleCursor>,
    /// Emission buffer: next packet per track, merged by dts.
    heads: Vec<Option<MediaPacket>>,
    limits: MediaLimits,
    ended: bool,
    failed: bool,
}

impl<R: Read + Seek> Mp4Demuxer<R> {
    /// Parse `moov` and prepare packet iteration. Reads box headers
    /// sequentially; `moov` is loaded under `max_header_bytes`.
    pub fn new(mut r: R, limits: MediaLimits) -> Result<Self, MediaError> {
        // Top-level walk: find moov (load it), note mdat/ftyp.
        let len = r.seek(SeekFrom::End(0)).map_err(MediaError::Io)?;
        r.seek(SeekFrom::Start(0)).map_err(MediaError::Io)?;
        let mut moov: Option<Vec<u8>> = None;
        let mut pos = 0u64;
        while pos < len {
            r.seek(SeekFrom::Start(pos)).map_err(MediaError::Io)?;
            let mut hbuf = [0u8; 16];
            let n = read_up_to(&mut r, &mut hbuf)?;
            if n < 8 {
                if len - pos < 8 {
                    break;
                }
                return Err(fmt("truncated top-level box header"));
            }
            let mut rd = Rd::new(&hbuf[..n]);
            let size32 = rd.u32()?;
            let cc = rd.fourcc()?;
            let (size, hdr) = match size32 {
                1 => (rd.u64().map_err(|_| fmt("truncated largesize"))?, 16u64),
                0 => (len - pos, 8u64),
                s => (s as u64, 8u64),
            };
            if size < hdr || pos + size > len {
                return Err(fmt("top-level box overruns file"));
            }
            if &cc == b"moov" {
                let body = size - hdr;
                if body > limits.max_header_bytes {
                    return Err(MediaError::Limit("moov exceeds header limit"));
                }
                // hbuf prefetch advanced the cursor past the payload start.
                r.seek(SeekFrom::Start(pos + hdr)).map_err(MediaError::Io)?;
                let mut v = vec![0u8; body as usize];
                r.read_exact(&mut v).map_err(MediaError::Io)?;
                moov = Some(v);
            }
            if &cc == b"moof" {
                return Err(MediaError::Unsupported("fragmented MP4 (moof)"));
            }
            pos += size;
        }
        let moov = moov.ok_or(fmt("no moov box"))?;
        let mut tracks = parse_moov(&moov, limits)?;
        if tracks.is_empty() {
            return Err(fmt("moov has no tracks"));
        }
        if tracks.len() > limits.max_tracks as usize {
            return Err(MediaError::Limit("too many tracks"));
        }
        let cursors = (0..tracks.len())
            .map(|_| SampleCursor {
                sample: 0,
                chunk: 0,
                in_chunk: 0,
                chunk_byte_off: 0,
                dts: 0,
                stts_i: 0,
                stts_left: 0,
                ctts_i: 0,
                ctts_left: 0,
                sync_i: 0,
            })
            .collect();
        // Fill TrackSpec index fields now that counts are known.
        for (i, t) in tracks.iter_mut().enumerate() {
            t.spec.index = i as u32;
        }
        let heads = (0..tracks.len()).map(|_| None).collect();
        Ok(Self {
            r,
            tracks,
            cursors,
            heads,
            limits,
            ended: false,
            failed: false,
        })
    }

    /// Declared tracks.
    pub fn tracks(&self) -> Vec<TrackSpec> {
        self.tracks.iter().map(|t| t.spec.clone()).collect()
    }

    /// The `TrackSpec` a packet's `config_epoch` refers to: `stsd` may carry
    /// several sample descriptions; packets record which one was active.
    /// Epoch 0 is `tracks()[track]` itself.
    pub fn track_spec_at_epoch(&self, track: usize, epoch: u64) -> Option<TrackSpec> {
        let t = self.tracks.get(track)?;
        let d = t.descs.get(epoch as usize)?;
        let mut spec = t.spec.clone();
        spec.codec = d.codec;
        spec.codec_private = d.codec_private.clone();
        spec.video = d.video;
        spec.audio = d.audio;
        spec.config_epoch = epoch;
        Some(spec)
    }

    /// Next packet in decode order (lowest dts across tracks), `None` at end.
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
        // Fill empty heads.
        for ti in 0..self.tracks.len() {
            if self.heads[ti].is_none() {
                self.heads[ti] = self.fetch_packet(ti)?;
            }
        }
        // Pick the head with the smallest dts (tracks' dts all on their own
        // timescale — compare physical time via Timestamp::compare).
        let mut best: Option<usize> = None;
        for (ti, h) in self.heads.iter().enumerate() {
            if let Some(p) = h {
                let better = match best {
                    None => true,
                    Some(b) => {
                        let bp = self.heads[b].as_ref().unwrap();
                        p.dts.unwrap_or(p.pts).compare(bp.dts.unwrap_or(bp.pts))
                            == std::cmp::Ordering::Less
                    }
                };
                if better {
                    best = Some(ti);
                }
            }
        }
        let Some(b) = best else {
            self.ended = true;
            return Ok(None);
        };
        Ok(self.heads[b].take())
    }

    /// Read the next sample of `ti`: lazily resolve chunk/split, then fetch.
    fn fetch_packet(&mut self, ti: usize) -> Result<Option<MediaPacket>, MediaError> {
        let (off, size, pts, dts, dur, key, epoch) = {
            let t = &self.tracks[ti];
            let c = &mut self.cursors[ti];
            let total = t.spec.declared_packets.unwrap_or(0);
            if c.sample >= total {
                return Ok(None);
            }
            // Advance chunk until it has unconsumed samples.
            loop {
                if c.chunk >= t.chunk_offsets.len() {
                    return Err(fmt("sample table overruns chunk table"));
                }
                let spc = stsc_spc(&t.stsc, (c.chunk + 1) as u32)
                    .ok_or(fmt("stsc run missing"))?
                    .0 as u64;
                if c.in_chunk < spc {
                    break;
                }
                c.chunk += 1;
                c.in_chunk = 0;
                c.chunk_byte_off = 0;
            }
            let idx = c.sample as usize;
            let size = if t.stsz_const != 0 {
                t.stsz_const
            } else {
                *t.stsz.get(idx).ok_or(fmt("stsz underrun"))?
            };
            if size as u64 > self.limits.max_packet_bytes as u64 {
                return Err(MediaError::Limit("sample exceeds packet limit"));
            }
            let off = t.chunk_offsets[c.chunk]
                .checked_add(c.chunk_byte_off)
                .ok_or(fmt("sample offset overflow"))?;
            // stts → dts advance.
            while c.stts_left == 0 {
                let &(cnt, _delta) = t.stts.get(c.stts_i).ok_or(fmt("stts underrun"))?;
                c.stts_left = cnt;
                c.stts_i += 1;
            }
            let dts = c.dts;
            let delta = t.stts[c.stts_i - 1].1;
            c.stts_left -= 1;
            c.dts += delta as u64;
            // ctts → pts offset (v1 signed; v0 values parse unsigned then cast).
            let mut pts = dts;
            if !t.ctts.is_empty() {
                while c.ctts_left == 0 {
                    let &(cnt, _) = t.ctts.get(c.ctts_i).ok_or(fmt("ctts underrun"))?;
                    c.ctts_left = cnt;
                    c.ctts_i += 1;
                }
                let coff = t.ctts[c.ctts_i - 1].1;
                c.ctts_left -= 1;
                pts = (dts as i64 + coff as i64) as u64;
            }
            // stss → keyframe (1-based sample numbers).
            let key = match &t.sync {
                None => true,
                Some(s) => {
                    let num = (c.sample + 1) as u32;
                    while c.sync_i < s.len() && s[c.sync_i] < num {
                        c.sync_i += 1;
                    }
                    s.get(c.sync_i).copied() == Some(num)
                }
            };
            let (_, desc) = stsc_spc(&t.stsc, (c.chunk + 1) as u32).unwrap();
            let epoch = desc.saturating_sub(1) as u64;
            c.sample += 1;
            c.in_chunk += 1;
            c.chunk_byte_off += size as u64;
            (off, size, pts, dts, delta, key, epoch)
        };

        self.r.seek(SeekFrom::Start(off)).map_err(MediaError::Io)?;
        let mut data = vec![0u8; size as usize];
        self.r.read_exact(&mut data).map_err(MediaError::Io)?;
        let spec = &self.tracks[ti].spec;
        Ok(Some(MediaPacket {
            track: ti as u32,
            ordinal: self.cursors[ti].sample - 1,
            config_epoch: epoch,
            data,
            pts: Timestamp::new(pts as i64, spec.time_base),
            dts: Some(Timestamp::new(dts as i64, spec.time_base)),
            duration_ticks: Some(dur),
            keyframe: key,
            discard_padding_ns: None,
        }))
    }
}

/// stsc lookup: samples-per-chunk + desc for 1-based chunk number.
fn stsc_spc(stsc: &[(u32, u32, u32)], chunk: u32) -> Option<(u32, u32)> {
    // Runs are sorted by first_chunk; the active run is the last with
    // first_chunk <= chunk.
    let mut cur = None;
    for &(first, spc, desc) in stsc {
        if first <= chunk {
            cur = Some((spc, desc));
        } else {
            break;
        }
    }
    cur
}

fn read_up_to<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize, MediaError> {
    r.read(buf).map_err(MediaError::Io)
}

// --- moov subtree -----------------------------------------------------------

fn parse_moov(moov: &[u8], limits: MediaLimits) -> Result<Vec<Mp4Track>, MediaError> {
    let mut tracks = Vec::new();
    let mut top = Rd::new(moov);
    while let Some(h) = box_hdr(&mut top, 0)? {
        if &h.fourcc == b"trak" {
            tracks.push(parse_trak(
                &moov[h.payload_start..h.payload_start + h.payload_len as usize],
                limits,
            )?);
        }
        top.pos = h.payload_start + h.payload_len as usize;
    }
    Ok(tracks)
}

fn parse_trak(trak: &[u8], limits: MediaLimits) -> Result<Mp4Track, MediaError> {
    let mut r = Rd::new(trak);
    let mut mdia: Option<&[u8]> = None;
    let mut edit_delay: Option<i64> = None;
    while let Some(h) = box_hdr(&mut r, 0)? {
        let body = &trak[h.payload_start..h.payload_start + h.payload_len as usize];
        match &h.fourcc {
            b"mdia" => mdia = Some(body),
            b"edts" => edit_delay = parse_edts(body)?,
            _ => {}
        }
        r.pos = h.payload_start + h.payload_len as usize;
    }
    let mdia = mdia.ok_or(fmt("trak without mdia"))?;
    parse_mdia(mdia, edit_delay, limits)
}

/// elst → media delay (media_time of the non-empty edit), in track ticks.
fn parse_edts(edts: &[u8]) -> Result<Option<i64>, MediaError> {
    let mut r = Rd::new(edts);
    while let Some(h) = box_hdr(&mut r, 0)? {
        if &h.fourcc == b"elst" {
            let body = &edts[h.payload_start..h.payload_start + h.payload_len as usize];
            let mut e = Rd::new(body);
            let version = e.u8()?;
            e.skip(3)?;
            let count = e.u32()?;
            for _ in 0..count.min(64) {
                let (_segdur, media_time) = if version == 1 {
                    (e.u64()?, e.i64()?)
                } else {
                    (e.u32()? as u64, e.i32()? as i64)
                };
                e.skip(4)?; // rate
                if media_time >= 0 {
                    return Ok(Some(media_time));
                }
            }
        }
        r.pos = h.payload_start + h.payload_len as usize;
    }
    Ok(None)
}

fn parse_mdia(
    mdia: &[u8],
    edit_delay: Option<i64>,
    limits: MediaLimits,
) -> Result<Mp4Track, MediaError> {
    let mut r = Rd::new(mdia);
    let mut timescale = 0u32;
    let mut declared_dur = None;
    let mut kind = TrackKind::Other("unknown");
    let mut stbl: Option<&[u8]> = None;
    while let Some(h) = box_hdr(&mut r, 0)? {
        let body = &mdia[h.payload_start..h.payload_start + h.payload_len as usize];
        match &h.fourcc {
            b"mdhd" => {
                let mut m = Rd::new(body);
                let version = m.u8()?;
                m.skip(3)?;
                if version == 1 {
                    m.skip(8 + 8)?;
                    timescale = m.u32()?;
                    declared_dur = Some(m.u64()?);
                } else {
                    m.skip(4 + 4)?;
                    timescale = m.u32()?;
                    declared_dur = Some(m.u32()? as u64);
                }
            }
            b"hdlr" => {
                let mut h2 = Rd::new(body);
                h2.skip(8)?; // version/flags + pre_defined
                let ht = h2.fourcc()?;
                kind = match &ht {
                    b"vide" => TrackKind::Video,
                    b"soun" => TrackKind::Audio,
                    _ => TrackKind::Other("non-av"),
                };
            }
            b"minf" => {
                let mut mr = Rd::new(body);
                while let Some(mh) = box_hdr(&mut mr, 0)? {
                    if &mh.fourcc == b"stbl" {
                        stbl = Some(
                            &body[mh.payload_start..mh.payload_start + mh.payload_len as usize],
                        );
                    }
                    mr.pos = mh.payload_start + mh.payload_len as usize;
                }
            }
            _ => {}
        }
        r.pos = h.payload_start + h.payload_len as usize;
    }
    if timescale == 0 {
        return Err(fmt("mdhd missing or zero timescale"));
    }
    let stbl = stbl.ok_or(fmt("no stbl"))?;
    let (descs, stts, ctts, stsz_const, stsz, stsc, chunk_offsets, sync, sample_count) =
        parse_stbl(stbl, limits)?;

    let declared = if stsz_const != 0 {
        Some(sample_count)
    } else {
        Some(stsz.len() as u64)
    };
    let (codec, private, video, audio) = match descs.first() {
        Some(d) => (d.codec, d.codec_private.clone(), d.video, d.audio),
        None => (Codec::Other("unknown"), None, None, None),
    };
    let spec = TrackSpec {
        index: 0,
        kind,
        codec,
        codec_private: private,
        time_base: TimeBase::new(1, timescale).map_err(|_| fmt("bad timescale"))?,
        video,
        audio,
        codec_delay_ns: 0,
        seek_preroll_ns: 0,
        config_epoch: 0,
        declared_packets: declared,
        declared_duration: declared_dur,
        edit_delay_ticks: edit_delay,
    };
    Ok(Mp4Track {
        spec,
        stts,
        ctts,
        stsz_const,
        stsz,
        stsc,
        chunk_offsets,
        sync,
        descs,
    })
}

type Stbl = (
    Vec<SampleDesc>,
    Vec<(u32, u32)>,
    Vec<(u32, i32)>,
    u32,
    Vec<u32>,
    Vec<(u32, u32, u32)>,
    Vec<u64>,
    Option<Vec<u32>>,
    u64,
);

fn parse_stbl(stbl: &[u8], limits: MediaLimits) -> Result<Stbl, MediaError> {
    let mut descs = Vec::new();
    let mut stts = Vec::new();
    let mut ctts = Vec::new();
    let mut stsz_const = 0u32;
    let mut stsz = Vec::new();
    let mut stsc = Vec::new();
    let mut offs = Vec::new();
    let mut sync: Option<Vec<u32>> = None;
    let mut sample_count = 0u64;

    let mut r = Rd::new(stbl);
    while let Some(h) = box_hdr(&mut r, 0)? {
        let body = &stbl[h.payload_start..h.payload_start + h.payload_len as usize];
        match &h.fourcc {
            b"stsd" => descs = parse_stsd(body)?,
            b"stts" => stts = parse_delta_table(body, limits, false)?,
            b"ctts" => ctts = parse_ctts(body, limits)?,
            b"stsz" => {
                let mut s = Rd::new(body);
                s.skip(4)?; // version/flags
                stsz_const = s.u32()?;
                let n = s.u32()?;
                sample_count = n as u64;
                if n as u64 > limits.max_table_entries as u64 {
                    return Err(MediaError::Limit("stsz too large"));
                }
                if stsz_const == 0 {
                    for _ in 0..n {
                        stsz.push(s.u32()?);
                    }
                }
            }
            b"stz2" => return Err(MediaError::Unsupported("compact stz2 sample sizes")),
            b"stsc" => stsc = parse_stsc(body, limits)?,
            b"stco" => {
                offs = parse_offsets(body, limits, false)?;
            }
            b"co64" => {
                offs = parse_offsets(body, limits, true)?;
            }
            b"stss" => {
                let mut s = Rd::new(body);
                s.skip(4)?;
                let n = s.u32()?;
                if n as u64 > limits.max_table_entries as u64 {
                    return Err(MediaError::Limit("stss too large"));
                }
                let mut v = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    v.push(s.u32()?);
                }
                sync = Some(v);
            }
            _ => {}
        }
        r.pos = h.payload_start + h.payload_len as usize;
    }
    if descs.is_empty() {
        return Err(fmt("stbl without stsd"));
    }
    Ok((
        descs,
        stts,
        ctts,
        stsz_const,
        stsz,
        stsc,
        offs,
        sync,
        sample_count,
    ))
}

/// stts/ctts-style run tables → (count, delta) pairs, count-capped.
fn parse_delta_table(
    body: &[u8],
    limits: MediaLimits,
    _signed: bool,
) -> Result<Vec<(u32, u32)>, MediaError> {
    let mut r = Rd::new(body);
    r.skip(4)?; // version/flags
    let n = r.u32()?;
    if n as u64 > limits.max_table_entries as u64 {
        return Err(MediaError::Limit("delta table too large"));
    }
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        v.push((r.u32()?, r.u32()?));
    }
    Ok(v)
}

fn parse_ctts(body: &[u8], limits: MediaLimits) -> Result<Vec<(u32, i32)>, MediaError> {
    let mut r = Rd::new(body);
    let version = r.u8()?;
    r.skip(3)?;
    let n = r.u32()?;
    if n as u64 > limits.max_table_entries as u64 {
        return Err(MediaError::Limit("ctts too large"));
    }
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let cnt = r.u32()?;
        let off = if version == 1 {
            r.i32()?
        } else {
            r.u32()? as i32
        };
        v.push((cnt, off));
    }
    Ok(v)
}

fn parse_stsc(body: &[u8], limits: MediaLimits) -> Result<Vec<(u32, u32, u32)>, MediaError> {
    let mut r = Rd::new(body);
    r.skip(4)?;
    let n = r.u32()?;
    if n as u64 > limits.max_table_entries as u64 {
        return Err(MediaError::Limit("stsc too large"));
    }
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        v.push((r.u32()?, r.u32()?, r.u32()?));
    }
    Ok(v)
}

fn parse_offsets(body: &[u8], limits: MediaLimits, wide: bool) -> Result<Vec<u64>, MediaError> {
    let mut r = Rd::new(body);
    r.skip(4)?;
    let n = r.u32()?;
    if n as u64 > limits.max_table_entries as u64 {
        return Err(MediaError::Limit("chunk offset table too large"));
    }
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        v.push(if wide { r.u64()? } else { r.u32()? as u64 });
    }
    Ok(v)
}

/// stsd → SampleDesc per entry. Entry bodies are parsed per codec family.
fn parse_stsd(body: &[u8]) -> Result<Vec<SampleDesc>, MediaError> {
    let mut r = Rd::new(body);
    r.skip(4)?; // version/flags
    let count = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..count.min(64) {
        if r.left() < 8 {
            return Err(fmt("stsd truncated"));
        }
        let esize = r.u32()? as usize;
        let cc = r.fourcc()?;
        if esize < 8 || esize - 8 > r.left() {
            return Err(fmt("stsd entry overruns"));
        }
        let entry = r.take(esize - 8)?;
        out.push(parse_sample_entry(&cc, entry)?);
    }
    Ok(out)
}

fn parse_sample_entry(cc: &[u8; 4], entry: &[u8]) -> Result<SampleDesc, MediaError> {
    match cc {
        b"avc1" | b"avc3" => {
            // VisualSampleEntry body: width@24, height@26; child boxes at 78.
            if entry.len() < 78 {
                return Err(fmt("short avc1 entry"));
            }
            let w = u16::from_be_bytes(entry[24..26].try_into().unwrap()) as u32;
            let h = u16::from_be_bytes(entry[26..28].try_into().unwrap()) as u32;
            let private = find_child(&entry[78..], b"avcC").map(|c| c.to_vec());
            Ok(SampleDesc {
                codec: Codec::H264,
                codec_private: private,
                video: Some(VideoInfo {
                    width: w,
                    height: h,
                }),
                audio: None,
            })
        }
        b"hvc1" | b"hev1" => Err(MediaError::Unsupported("hvc1/HEVC input")),
        b"mp4a" => {
            // AudioSampleEntry v0 body: channels@16, samplerate(16.16)@24,
            // child boxes (esds) at 28.
            if entry.len() < 28 {
                return Err(fmt("short mp4a entry"));
            }
            let ch = u16::from_be_bytes(entry[16..18].try_into().unwrap());
            let rate = u32::from_be_bytes(entry[24..28].try_into().unwrap()) >> 16;
            // esds child box at entry[28..] → DecSpecificInfo = ASC.
            let private =
                find_child(&entry[28..], b"esds").and_then(|e| esds_audio_specific_config(e).ok());
            // mp4a in MP4 is assumed AAC (object type in ASC is authoritative).
            Ok(SampleDesc {
                codec: Codec::Aac,
                codec_private: private,
                video: None,
                audio: Some(AudioInfo {
                    sample_rate: rate,
                    channels: ch,
                }),
            })
        }
        b"Opus" => {
            if entry.len() < 28 {
                return Err(fmt("short Opus entry"));
            }
            let ch = u16::from_be_bytes(entry[16..18].try_into().unwrap());
            let rate = u32::from_be_bytes(entry[24..28].try_into().unwrap()) >> 16;
            let private = find_child(&entry[28..], b"dOps").map(|c| c.to_vec());
            Ok(SampleDesc {
                codec: Codec::Opus,
                codec_private: private,
                video: None,
                audio: Some(AudioInfo {
                    sample_rate: rate,
                    channels: ch,
                }),
            })
        }
        _ => Ok(SampleDesc {
            codec: Codec::Other("unmapped"),
            codec_private: None,
            video: None,
            audio: None,
        }),
    }
}

/// Find a child box inside a sample-entry tail.
fn find_child<'a>(mut tail: &'a [u8], want: &[u8; 4]) -> Option<&'a [u8]> {
    while tail.len() >= 8 {
        let size = u32::from_be_bytes(tail[..4].try_into().unwrap()) as usize;
        let cc = &tail[4..8];
        if size < 8 || size > tail.len() {
            return None;
        }
        if cc == want {
            return Some(&tail[8..size]);
        }
        tail = &tail[size..];
    }
    None
}

/// esds fullbox → DecoderConfigDescriptor → DecSpecificInfo (AudioSpecificConfig).
/// Minimal MPEG-4 descriptor TLV walk: tags + 1–4 byte lengths (bit-7 continue).
fn esds_audio_specific_config(esds: &[u8]) -> Result<Vec<u8>, MediaError> {
    let mut r = Rd::new(esds);
    r.skip(4)?; // version/flags
    // ES_Descriptor (0x03)
    expect_tag(&mut r, 0x03)?;
    r.u16()?; // ES_ID
    r.u8()?; // flags
    // DecoderConfigDescriptor (0x04)
    expect_tag(&mut r, 0x04)?;
    r.u8()?; // objectTypeIndication
    r.u8()?; // streamType etc
    r.u24()?; // bufferSizeDB
    r.u32()?; // maxBitrate
    r.u32()?; // avgBitrate
    // DecSpecificInfo (0x05)
    let mut len = expect_tag(&mut r, 0x05)?;
    len = len.min(r.left());
    Ok(r.take(len)?.to_vec())
}

/// Read a descriptor tag + variable length; returns (payload_len) after tag check.
fn expect_tag(r: &mut Rd, want: u8) -> Result<usize, MediaError> {
    let tag = r.u8()?;
    if tag != want {
        return Err(fmt("esds descriptor tag mismatch"));
    }
    let mut len = 0usize;
    loop {
        let b = r.u8()?;
        len = (len << 7) | (b & 0x7F) as usize;
        if b & 0x80 == 0 {
            break;
        }
    }
    Ok(len)
}
