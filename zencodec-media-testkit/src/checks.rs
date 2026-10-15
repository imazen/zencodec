//! Conformance checks. Each takes factories for the implementation under
//! test and inputs it should handle, and returns `Err` with the first rule
//! broken.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use enough::{Stop, StopReason, Unstoppable};
use zencodec::decode::{AnimationFrameDecoder, Input};
use zencodec::encode::AnimationFrameEncoder;
use zencodec::source::{ReadAt, SourceError};
use zencodec::time::{FrameDuration, TimeBase, Timestamp};
use zencodec_io::{DriveFailure, DriveOptions, ReadAtCursor, SliceCursor, Yield};
use zencodec_media::adapt::{
    AnimationVideoEncoder, CaptureFrame, RenderFrame, VideoAnimationDecoder,
};
use zencodec_media::container::{
    DemuxEvent, DemuxStage, Demuxer, MuxHint, Muxer, SeekStatus, SeekTarget, apply_finish,
};
use zencodec_media::frame::{Frame, PlaneSamples};
use zencodec_media::packet::{Packet, RandomAccess};
use zencodec_media::session::{
    CodecConfig, ProtocolViolation, Receive, SendStatus, SessionError, UnsupportedMedia,
    VideoDecoder, VideoEncoder, erase_video_decoder,
};
use zencodec_media::track::TrackInfo;
use zenpixels::PixelBuffer;

/// A broken rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The check that failed.
    pub check: &'static str,
    /// What went wrong.
    pub detail: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.check, self.detail)
    }
}

impl core::error::Error for Failure {}

/// The result of a check.
pub type Conformance = Result<(), Failure>;

fn fail(check: &'static str, detail: impl Into<String>) -> Failure {
    Failure {
        check,
        detail: detail.into(),
    }
}

/// A stop that has tripped.
struct Tripped;

impl Stop for Tripped {
    fn check(&self) -> Result<(), StopReason> {
        Err(StopReason::Cancelled)
    }
}

/// Every plane's samples in order, as bytes: what a frame shows.
fn frame_pixels(frame: &Frame) -> Result<Vec<u8>, String> {
    let mapped = frame.map().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for i in 0..mapped.plane_count() {
        let plane = mapped.plane(i).ok_or("plane disappeared")?;
        for y in 0..plane.height() {
            match plane.row(y).ok_or("row out of range")? {
                PlaneSamples::U8(r) => out.extend_from_slice(r),
                PlaneSamples::U16(r) => {
                    for v in r {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
    }
    Ok(out)
}

/// What a decoded frame is, for comparing two decodes.
#[derive(Debug, PartialEq, Eq)]
struct Shown {
    pts: Option<Timestamp>,
    keyframe: bool,
    show_existing: bool,
    pixels: Vec<u8>,
}

fn shown(frame: &Frame) -> Result<Shown, String> {
    Ok(Shown {
        pts: frame.timing().pts,
        keyframe: frame.presentation().keyframe,
        show_existing: frame.presentation().show_existing,
        pixels: frame_pixels(frame)?,
    })
}

/// Decode `packets` receiving everything as soon as it is ready.
fn decode_eager<D: VideoDecoder>(dec: &mut D, packets: &[Packet]) -> Result<Vec<Frame>, String> {
    let mut frames = Vec::new();
    for p in packets {
        let mut p = p.clone();
        zencodec_media::session::send_packet_until_accepted(dec, &mut p, &Unstoppable, |f| {
            frames.push(f);
            Ok::<(), ()>(())
        })
        .map_err(|e| format!("{e:?}"))?;
    }
    zencodec_media::session::drain_decoder(dec, &Unstoppable, |f| {
        frames.push(f);
        Ok::<(), ()>(())
    })
    .map_err(|e| format!("{e:?}"))?;
    Ok(frames)
}

/// Verify a [`VideoDecoder`] against the session contract.
///
/// `make` builds a fresh decoder; `config` configures it; `packets` is a
/// valid stream for it. Checked:
///
/// - Receiving lazily (only when the decoder says `ReceivePending`, then
///   draining) and eagerly (after every packet) yields the same frames:
///   timestamps, presentation flags and pixels.
/// - `ReceivePending` leaves the packet untaken and is followed by a
///   `Ready`.
/// - `end_input` is idempotent, `EndOfStream` is stable, and `send` after
///   `end_input` is [`ProtocolViolation::SendAfterEnd`].
/// - A packet from another configuration epoch is
///   [`ProtocolViolation::EpochMismatch`].
/// - A frame and a mapping of it stay valid while the decoder decodes more,
///   resets and is dropped; after `reset` the same packets decode to the
///   same frames.
/// - Behind [`erase_video_decoder`], a tripped stop is `Cancelled`, every
///   call after it `Finished`, until `reset`.
pub fn check_video_decoder_contract<D, F>(
    mut make: F,
    config: &CodecConfig,
    packets: &[Packet],
) -> Conformance
where
    D: VideoDecoder + Send + 'static,
    F: FnMut() -> D,
{
    const CHECK: &str = "video_decoder_contract";
    let configured = |make: &mut F| -> Result<D, Failure> {
        let mut d = make();
        d.configure(config, &Unstoppable)
            .map_err(|e| fail(CHECK, format!("configure: {e:?}")))?;
        Ok(d)
    };
    if packets.is_empty() {
        return Err(fail(CHECK, "no packets supplied"));
    }

    let mut dec = configured(&mut make)?;
    let eager = decode_eager(&mut dec, packets).map_err(|e| fail(CHECK, e))?;
    if eager.is_empty() {
        return Err(fail(CHECK, "the stream decoded to no frames"));
    }
    let want: Vec<Shown> = eager
        .iter()
        .map(shown)
        .collect::<Result<_, _>>()
        .map_err(|e| fail(CHECK, e))?;

    // Lazy: receive only when pushed back, then at the end.
    let mut dec = configured(&mut make)?;
    let mut lazy = Vec::new();
    for (i, p) in packets.iter().enumerate() {
        let mut p = p.clone();
        loop {
            match dec.send(&mut p, &Unstoppable) {
                Ok(SendStatus::Accepted) => break,
                Ok(SendStatus::ReceivePending) => {
                    if p.is_taken() {
                        return Err(fail(
                            CHECK,
                            format!("packet {i}: ReceivePending took the payload"),
                        ));
                    }
                    match dec.receive(&Unstoppable) {
                        Ok(Receive::Ready(f)) => lazy.push(f),
                        other => {
                            return Err(fail(
                                CHECK,
                                format!("packet {i}: ReceivePending, then {other:?}"),
                            ));
                        }
                    }
                }
                Ok(other) => return Err(fail(CHECK, format!("packet {i}: send {other:?}"))),
                Err(e) => return Err(fail(CHECK, format!("packet {i}: send {e:?}"))),
            }
        }
    }
    dec.end_input()
        .map_err(|e| fail(CHECK, format!("end_input: {e:?}")))?;
    dec.end_input()
        .map_err(|e| fail(CHECK, format!("second end_input: {e:?}")))?;
    loop {
        match dec.receive(&Unstoppable) {
            Ok(Receive::Ready(f)) => lazy.push(f),
            Ok(Receive::EndOfStream) => break,
            other => return Err(fail(CHECK, format!("drain: {other:?}"))),
        }
    }
    if !matches!(dec.receive(&Unstoppable), Ok(Receive::EndOfStream)) {
        return Err(fail(CHECK, "EndOfStream is not stable"));
    }
    let got: Vec<Shown> = lazy
        .iter()
        .map(shown)
        .collect::<Result<_, _>>()
        .map_err(|e| fail(CHECK, e))?;
    if got != want {
        return Err(fail(
            CHECK,
            format!(
                "lazy receive gave {} frames, eager {}, or they differ",
                got.len(),
                want.len()
            ),
        ));
    }
    let mut again = packets[0].clone();
    match dec.send(&mut again, &Unstoppable) {
        Err(SessionError::Protocol(ProtocolViolation::SendAfterEnd)) => {}
        other => return Err(fail(CHECK, format!("send after end_input: {other:?}"))),
    }

    // Epochs.
    let mut dec = configured(&mut make)?;
    let mut other_epoch = packets[0]
        .clone()
        .with_config_epoch(config.epoch.wrapping_add(1));
    match dec.send(&mut other_epoch, &Unstoppable) {
        Err(SessionError::Protocol(ProtocolViolation::EpochMismatch)) => {}
        other => {
            return Err(fail(
                CHECK,
                format!("a packet from another epoch: {other:?}"),
            ));
        }
    }

    // Ownership: a frame and its mapping outlive more decoding, reset and
    // the decoder.
    let mut dec = configured(&mut make)?;
    let first = decode_eager(&mut dec, packets).map_err(|e| fail(CHECK, e))?;
    let held = first[0].clone();
    let mapping = held.map().map_err(|e| fail(CHECK, e.to_string()))?;
    dec.reset()
        .map_err(|e| fail(CHECK, format!("reset: {e:?}")))?;
    let after_reset = decode_eager(&mut dec, packets).map_err(|e| fail(CHECK, e))?;
    drop(dec);
    drop(mapping);
    let after: Vec<Shown> = after_reset
        .iter()
        .map(shown)
        .collect::<Result<_, _>>()
        .map_err(|e| fail(CHECK, e))?;
    if after != want {
        return Err(fail(
            CHECK,
            "after reset the same packets decode to different frames",
        ));
    }
    if shown(&held).map_err(|e| fail(CHECK, e))? != want[0] {
        return Err(fail(
            CHECK,
            "a frame changed after its decoder decoded more, reset and dropped",
        ));
    }

    // Cancellation is terminal until reset, behind the erased wrapper.
    let mut dec = erase_video_decoder(configured(&mut make)?);
    match dec.receive(&Tripped) {
        Err(SessionError::Cancelled(_)) => {}
        other => return Err(fail(CHECK, format!("a tripped stop: {other:?}"))),
    }
    match dec.receive(&Unstoppable) {
        Err(SessionError::Finished) => {}
        other => return Err(fail(CHECK, format!("after cancellation: {other:?}"))),
    }
    dec.reset()
        .map_err(|e| fail(CHECK, format!("reset after cancellation: {e:?}")))?;
    if !matches!(dec.receive(&Unstoppable), Ok(Receive::NeedInput)) {
        return Err(fail(CHECK, "reset did not reopen the session"));
    }
    Ok(())
}

/// Verify a [`VideoEncoder`] against the session contract.
///
/// `make` builds a fresh encoder; `frames` are valid input for it, with
/// timestamps on its clock. Checked:
///
/// - `output_config` is available before the first frame.
/// - Receiving lazily and eagerly yields the same packets (bytes,
///   timestamps, random-access flags); every packet has a timestamp and the
///   timestamps are exactly the frames'.
/// - `ReceivePending` is followed by a `Ready`; `end_input` is idempotent,
///   `EndOfStream` stable, `send` after it refused.
/// - A frame on another clock is [`ProtocolViolation::ClockMismatch`].
pub fn check_video_encoder_contract<E, F>(mut make: F, frames: &[Frame]) -> Conformance
where
    E: VideoEncoder,
    F: FnMut() -> E,
{
    const CHECK: &str = "video_encoder_contract";
    if frames.is_empty() {
        return Err(fail(CHECK, "no frames supplied"));
    }
    type Out = (Vec<u8>, Option<Timestamp>, RandomAccess);
    let run = |enc: &mut E, eager: bool| -> Result<Vec<Out>, Failure> {
        let mut out = Vec::new();
        let keep = |p: Packet| -> Out { (p.bytes().to_vec(), p.pts(), p.random_access()) };
        for (i, f) in frames.iter().enumerate() {
            loop {
                match enc.send(f, &Unstoppable) {
                    Ok(SendStatus::Accepted) => break,
                    Ok(SendStatus::ReceivePending) => match enc.receive(&Unstoppable) {
                        Ok(Receive::Ready(p)) => out.push(keep(p)),
                        other => {
                            return Err(fail(
                                CHECK,
                                format!("frame {i}: ReceivePending, then {other:?}"),
                            ));
                        }
                    },
                    other => return Err(fail(CHECK, format!("frame {i}: send {other:?}"))),
                }
            }
            if eager {
                while let Ok(Receive::Ready(p)) = enc.receive(&Unstoppable) {
                    out.push(keep(p));
                }
            }
        }
        enc.end_input()
            .and_then(|()| enc.end_input())
            .map_err(|e| fail(CHECK, format!("end_input: {e:?}")))?;
        loop {
            match enc.receive(&Unstoppable) {
                Ok(Receive::Ready(p)) => out.push(keep(p)),
                Ok(Receive::EndOfStream) => break,
                other => return Err(fail(CHECK, format!("drain: {other:?}"))),
            }
        }
        if !matches!(enc.receive(&Unstoppable), Ok(Receive::EndOfStream)) {
            return Err(fail(CHECK, "EndOfStream is not stable"));
        }
        match enc.send(&frames[0], &Unstoppable) {
            Err(SessionError::Protocol(ProtocolViolation::SendAfterEnd)) => {}
            other => return Err(fail(CHECK, format!("send after end_input: {other:?}"))),
        }
        Ok(out)
    };
    let mut enc = make();
    let Some(config) = enc.output_config().cloned() else {
        return Err(fail(CHECK, "no output_config before the first frame"));
    };
    let eager = run(&mut enc, true)?;
    let lazy = run(&mut make(), false)?;
    if eager != lazy {
        return Err(fail(CHECK, "lazy and eager receive gave different packets"));
    }
    let mut got: Vec<Timestamp> = Vec::new();
    for (i, (_, pts, _)) in eager.iter().enumerate() {
        got.push(pts.ok_or_else(|| fail(CHECK, format!("packet {i} has no timestamp")))?);
    }
    let mut want: Vec<Timestamp> = frames.iter().filter_map(|f| f.timing().pts).collect();
    got.sort_by(|a, b| a.compare(*b));
    want.sort_by(|a, b| a.compare(*b));
    if got != want {
        return Err(fail(
            CHECK,
            "packet timestamps are not exactly the frames' timestamps",
        ));
    }
    // A frame on another clock.
    let mut enc = make();
    let other_base = if config.time_base == TimeBase::MILLIS {
        TimeBase::SECONDS
    } else {
        TimeBase::MILLIS
    };
    let mut timing = frames[0].timing();
    timing.pts = Some(Timestamp::new(0, other_base));
    let stray = frames[0].represent(timing, frames[0].presentation());
    match enc.send(&stray, &Unstoppable) {
        Err(SessionError::Protocol(ProtocolViolation::ClockMismatch)) => Ok(()),
        other => Err(fail(CHECK, format!("a frame on another clock: {other:?}"))),
    }
}

/// A [`ReadAt`] over bytes that does not admit being in memory, so the
/// windowed and range paths of the drivers run.
struct Remote<'a>(&'a [u8]);

impl ReadAt for Remote<'_> {
    type Error = SourceError;

    fn len(&mut self) -> Result<u64, SourceError> {
        Ok(self.0.len() as u64)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
        if self.read_at(offset, out)? == out.len() {
            Ok(())
        } else {
            Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(self.0.len() as u64),
            })
        }
    }

    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, SourceError> {
        let start = (offset as usize).min(self.0.len());
        let n = out.len().min(self.0.len() - start);
        out[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
}

/// A packet as data, for comparing demuxes.
#[derive(Debug, PartialEq, Eq)]
struct Demuxed {
    track: u32,
    bytes: Vec<u8>,
    pts: Option<Timestamp>,
    dts: Option<Timestamp>,
    random_access: RandomAccess,
    epoch: u32,
}

#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Headers(usize),
    Packet(Demuxed),
    Config(u32, u32),
}

fn seen(ev: DemuxEvent, tracks: usize) -> Option<Seen> {
    match ev {
        DemuxEvent::Headers => Some(Seen::Headers(tracks)),
        DemuxEvent::Packet(p) => Some(Seen::Packet(Demuxed {
            track: p.track(),
            bytes: p.bytes().to_vec(),
            pts: p.pts(),
            dts: p.dts(),
            random_access: p.random_access(),
            epoch: p.config_epoch(),
        })),
        DemuxEvent::ConfigChanged { track, epoch } => Some(Seen::Config(track, epoch)),
        _ => None,
    }
}

type DemuxErr<D> = DriveFailure<SessionError<<D as Demuxer>::Error>, SourceError>;

fn demux_all<D: Demuxer>(
    dem: &mut D,
    mut next: impl FnMut(&mut D) -> Result<Yield<DemuxEvent>, DemuxErr<D>>,
) -> Result<Vec<Seen>, DemuxErr<D>> {
    let mut out = Vec::new();
    loop {
        let y = next(dem)?;
        let finished = y.is_finished();
        if let Some(s) = seen(y.into_inner(), dem.tracks().len()) {
            out.push(s);
        }
        if finished {
            return Ok(out);
        }
    }
}

/// Verify a [`Demuxer`] against the container contract.
///
/// `make` builds a fresh demuxer; `file` is a valid container for it.
/// Checked:
///
/// - The in-memory driver at windows of 1, 7 and 64 bytes and the whole
///   file, and the random-access driver at 64 bytes and 4 KiB, give the
///   same events: headers, packets (bytes, track, timestamps, random-access
///   evidence, epoch), configuration changes.
/// - Input at an offset the demuxer did not ask for is not consumed, and
///   `requested` is never empty after `Need`.
/// - Every truncation (at 13 evenly spread cut points) either fails as the
///   demuxer's own error or succeeds with a prefix of the events; it never
///   breaks the driver protocol.
///
/// For seeking, see [`check_demuxer_seek`].
pub fn check_demuxer_contract<D, F>(mut make: F, file: &[u8]) -> Conformance
where
    D: Demuxer,
    F: FnMut() -> D,
{
    const CHECK: &str = "demuxer_contract";
    let slice = |make: &mut F, bytes: &[u8], window: usize| {
        let mut dem = make();
        let mut cursor = SliceCursor::new(window);
        demux_all(&mut dem, |d| {
            cursor
                .next(bytes, &mut DemuxStage(d), &Unstoppable)
                .map_err(DriveFailure::widen)
        })
    };
    let want =
        slice(&mut make, file, file.len().max(1)).map_err(|e| fail(CHECK, format!("{e:?}")))?;
    if !want.iter().any(|s| matches!(s, Seen::Packet(_))) {
        return Err(fail(CHECK, "the file yielded no packets"));
    }
    for window in [1usize, 7, 64] {
        let got = slice(&mut make, file, window)
            .map_err(|e| fail(CHECK, format!("window {window}: {e:?}")))?;
        if got != want {
            return Err(fail(
                CHECK,
                format!("window {window} gave different events than the whole file"),
            ));
        }
    }
    for read_size in [64usize, 4096] {
        let mut dem = make();
        let mut src = Remote(file);
        let mut cursor = ReadAtCursor::new(DriveOptions::new(0, read_size));
        let got = demux_all(&mut dem, |d| {
            cursor.next(&mut src, &mut DemuxStage(d), &Unstoppable)
        })
        .map_err(|e| fail(CHECK, format!("read_at {read_size}: {e:?}")))?;
        if got != want {
            return Err(fail(
                CHECK,
                format!("the ReadAt driver at {read_size} bytes gave different events"),
            ));
        }
    }

    // Unrequested input is not consumed.
    let mut dem = make();
    let at = file.len() / 2;
    let p = dem
        .step(Input::new(at as u64 + 1, &file[at..]), &Unstoppable)
        .map_err(|e| fail(CHECK, format!("an unrequested input: {e:?}")))?;
    if p.consumed != 0 || !matches!(p.event, DemuxEvent::Need) || dem.requested().is_empty() {
        return Err(fail(
            CHECK,
            "input at an offset nobody asked for was used, or no request was named",
        ));
    }

    // Truncations.
    for k in 1..=13usize {
        let cut = file.len() * k / 14;
        match slice(&mut make, &file[..cut], 7) {
            Ok(got) => {
                if got.len() > want.len() || got[..] != want[..got.len()] {
                    return Err(fail(
                        CHECK,
                        format!("a file cut at {cut} gave events the whole file does not"),
                    ));
                }
            }
            Err(DriveFailure::Step(_)) => {}
            Err(e) => {
                return Err(fail(
                    CHECK,
                    format!("a file cut at {cut} broke the protocol: {e:?}"),
                ));
            }
        }
    }
    Ok(())
}

/// Verify a demuxer's [`seek`](Demuxer::seek) against `targets`.
///
/// After the headers, each target is sought (stepping through
/// `SeekStatus::Scanning` until `Seeked`). Checked: the reported position
/// is the next packet's actual timestamp and random-access evidence, never
/// the request echoed back; a `RandomAccessPoints` seek lands on a packet
/// the container marks `RandomAccess::Yes`.
pub fn check_demuxer_seek<D, F>(mut make: F, file: &[u8], targets: &[SeekTarget]) -> Conformance
where
    D: Demuxer,
    F: FnMut() -> D,
{
    const CHECK: &str = "demuxer_seek";
    let mut dem = make();
    let mut cursor = SliceCursor::new(16);
    loop {
        match cursor
            .next(file, &mut DemuxStage(&mut dem), &Unstoppable)
            .map_err(|e| fail(CHECK, format!("{e:?}")))?
            .into_inner()
        {
            DemuxEvent::Headers => break,
            DemuxEvent::Done => return Err(fail(CHECK, "no headers")),
            _ => {}
        }
    }
    for (i, target) in targets.iter().enumerate() {
        let result = match dem.seek(target) {
            Ok(SeekStatus::Positioned(r)) => r,
            Ok(SeekStatus::Scanning) => loop {
                match cursor
                    .next(file, &mut DemuxStage(&mut dem), &Unstoppable)
                    .map_err(|e| fail(CHECK, format!("target {i}: {e:?}")))?
                    .into_inner()
                {
                    DemuxEvent::Seeked(r) => break r,
                    DemuxEvent::Done => {
                        return Err(fail(CHECK, format!("target {i}: scan ended unpositioned")));
                    }
                    _ => {}
                }
            },
            Ok(other) => return Err(fail(CHECK, format!("target {i}: {other:?}"))),
            Err(SessionError::Unsupported(UnsupportedMedia::Seek)) => {
                return Err(fail(CHECK, "seek is unsupported"));
            }
            Err(e) => return Err(fail(CHECK, format!("target {i}: {e:?}"))),
        };
        let next = loop {
            match cursor
                .next(file, &mut DemuxStage(&mut dem), &Unstoppable)
                .map_err(|e| fail(CHECK, format!("target {i}: {e:?}")))?
                .into_inner()
            {
                DemuxEvent::Packet(p) if p.track() == target.track => break p,
                DemuxEvent::Done => {
                    return Err(fail(CHECK, format!("target {i}: no packet after the seek")));
                }
                _ => {}
            }
        };
        if next.pts().is_none_or(|p| !p.same_instant(result.pts)) {
            return Err(fail(
                CHECK,
                format!(
                    "target {i}: reported {:?} but the next packet is at {:?}",
                    result.pts,
                    next.pts()
                ),
            ));
        }
        if next.random_access() != result.random_access {
            return Err(fail(
                CHECK,
                format!("target {i}: reported random access differs from the packet's"),
            ));
        }
        if target.scope == zencodec_media::container::SeekScope::RandomAccessPoints
            && result.random_access != RandomAccess::Yes
        {
            return Err(fail(
                CHECK,
                format!("target {i}: a random-access seek landed elsewhere"),
            ));
        }
    }
    Ok(())
}

/// How a muxer check drains output.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drain {
    Never,
    All,
    Partial,
    Take,
}

/// Verify a [`Muxer`]'s pulled output.
///
/// `make` builds a fresh muxer; `tracks` and `packets` are valid input for
/// it. Checked:
///
/// - The finished file is identical whether output is never drained,
///   drained fully after every packet, drained in uneven prefixes
///   (sometimes asking for more than is pending, which clamps), or taken
///   with `take_output`; patches land inside the file.
/// - A second `begin` is [`ProtocolViolation::BeginTwice`].
/// - With [`MuxHint::forward_only`], `begin` either refuses with
///   [`UnsupportedMedia::ForwardOnlyLayout`] or produces no patches, and
///   the timing it reports says so.
pub fn check_muxer_output_honesty<M, F>(
    mut make: F,
    tracks: &[TrackInfo],
    packets: &[Packet],
) -> Conformance
where
    M: Muxer,
    F: FnMut() -> M,
{
    const CHECK: &str = "muxer_output_honesty";
    let run = |make: &mut F, drain: Drain, hint: MuxHint| -> Result<Vec<u8>, Failure> {
        let mut m = make();
        let timing = m
            .begin(tracks, hint)
            .map_err(|e| fail(CHECK, format!("begin: {e:?}")))?;
        let mut out = Vec::new();
        let mut rng = 0x2545_f491u32;
        for (i, p) in packets.iter().enumerate() {
            m.write_packet(p.clone(), &Unstoppable)
                .map_err(|e| fail(CHECK, format!("packet {i}: {e:?}")))?;
            let pending = m.pending_output().len();
            match drain {
                Drain::Never => {}
                Drain::All => {
                    out.extend_from_slice(m.pending_output());
                    m.consume_output(pending);
                }
                Drain::Partial => {
                    rng ^= rng << 13;
                    rng ^= rng >> 17;
                    rng ^= rng << 5;
                    let n = rng as usize % (pending + 2);
                    out.extend_from_slice(&m.pending_output()[..n.min(pending)]);
                    m.consume_output(n);
                    if m.pending_output().len() != pending - n.min(pending) {
                        return Err(fail(CHECK, "consume_output did not clamp"));
                    }
                }
                Drain::Take => m.take_output(&mut out),
            }
            if !timing.streams() && !m.pending_output().is_empty() {
                return Err(fail(
                    CHECK,
                    "pending output before finish from a muxer that reported AT_FINISH",
                ));
            }
        }
        let fin = m
            .finish(&Unstoppable)
            .map_err(|e| fail(CHECK, format!("finish: {e:?}")))?;
        if !timing.patches_at_finish() && !fin.patches.is_empty() {
            return Err(fail(
                CHECK,
                "patches from a muxer whose timing said it would not patch",
            ));
        }
        apply_finish(&mut out, fin).map_err(|e| fail(CHECK, e.to_string()))?;
        Ok(out)
    };
    let hint = MuxHint::default();
    let want = run(&mut make, Drain::Never, hint)?;
    for (name, drain) in [
        ("draining all", Drain::All),
        ("uneven prefixes", Drain::Partial),
        ("take_output", Drain::Take),
    ] {
        if run(&mut make, drain, hint)? != want {
            return Err(fail(CHECK, format!("{name} changed the file")));
        }
    }
    let mut m = make();
    m.begin(tracks, hint)
        .map_err(|e| fail(CHECK, format!("begin: {e:?}")))?;
    match m.begin(tracks, hint) {
        Err(SessionError::Protocol(ProtocolViolation::BeginTwice)) => {}
        other => return Err(fail(CHECK, format!("a second begin: {other:?}"))),
    }
    let mut m = make();
    match m.begin(tracks, MuxHint::forward_only()) {
        Err(SessionError::Unsupported(UnsupportedMedia::ForwardOnlyLayout)) => {}
        Ok(t) if t.patches_at_finish() => {
            return Err(fail(
                CHECK,
                "a forward-only destination got a layout that patches",
            ));
        }
        Ok(_) => {
            run(&mut make, Drain::Take, MuxHint::forward_only())?;
        }
        Err(e) => return Err(fail(CHECK, format!("forward-only begin: {e:?}"))),
    }
    Ok(())
}

/// Verify the animation adapters over a video codec and a container.
///
/// Encodes `frames` with `durations` through [`AnimationVideoEncoder`]
/// (the encoder from `make_encoder`, the muxer from `make_muxer`, pixels
/// captured by `capture`), drained and undrained, then decodes the file
/// through [`VideoAnimationDecoder`] (`make_demuxer`, `make_decoder`,
/// `render`). Checked: draining never changes the bytes; with a streaming
/// muxer, bytes become pending across every `lag + 1` frames; the decoded
/// animation has the same frames, pixels and exact durations; a declared
/// frame count is binding.
#[allow(clippy::too_many_arguments)]
pub fn check_animation_over_video<E, M, D, C, P, R>(
    mut make_encoder: impl FnMut() -> E,
    mut make_muxer: impl FnMut() -> M,
    lag_frames: u32,
    mut capture: impl FnMut() -> P,
    make_demuxer: impl FnOnce() -> D,
    make_decoder: impl FnOnce() -> C,
    render: R,
    frames: &[PixelBuffer],
    durations: &[FrameDuration],
) -> Conformance
where
    E: VideoEncoder + Send,
    M: Muxer + Send,
    D: Demuxer + Send,
    C: VideoDecoder + Send,
    P: CaptureFrame + Send,
    R: RenderFrame + Send,
{
    const CHECK: &str = "animation_over_video";
    if frames.is_empty() || frames.len() != durations.len() {
        return Err(fail(
            CHECK,
            "supply as many durations as frames, at least one",
        ));
    }
    let (w, h, desc) = (
        frames[0].width(),
        frames[0].height(),
        frames[0].descriptor(),
    );
    let count = frames.len() as u32;
    // The file, bytes available after each push, and the reported lag.
    type Encoded = (Vec<u8>, Vec<usize>, Option<u32>);
    let mut encode = |drain: bool| -> Result<Encoded, Failure> {
        let mut a = AnimationVideoEncoder::new(make_encoder(), make_muxer(), capture(), lag_frames)
            .map_err(|e| fail(CHECK, format!("new: {e:?}")))?;
        let timing = a
            .begin(w, h, desc, Some(count))
            .map_err(|e| fail(CHECK, format!("begin: {e:?}")))?;
        let mut out = Vec::new();
        let mut points = Vec::new();
        if drain {
            a.take_output(&mut out);
        }
        for (f, d) in frames.iter().zip(durations) {
            a.push_frame_exact(f.as_slice(), *d, None)
                .map_err(|e| fail(CHECK, format!("push: {e:?}")))?;
            points.push(out.len() + a.pending_output().len());
            if drain {
                a.take_output(&mut out);
            }
        }
        out.extend_from_slice(
            a.finish(None)
                .map_err(|e| fail(CHECK, format!("finish: {e:?}")))?
                .data(),
        );
        Ok((out, points, timing.lag_frames()))
    };
    let (drained, points, lag) = encode(true)?;
    let (undrained, _, _) = encode(false)?;
    if drained != undrained {
        return Err(fail(CHECK, "draining changed the animation's bytes"));
    }
    if let Some(lag) = lag {
        let lag = lag as usize;
        let bounded = &points[..points.len().saturating_sub(1)];
        for i in 0..bounded.len() {
            if let Some(&later) = bounded.get(i + lag + 1)
                && later <= bounded[i]
            {
                return Err(fail(
                    CHECK,
                    format!(
                        "a lag of {lag} frames, but no bytes from frame {i} to {}",
                        i + lag + 1
                    ),
                ));
            }
        }
    }

    let mut anim = VideoAnimationDecoder::new(
        Arc::<[u8]>::from(drained),
        make_demuxer(),
        make_decoder(),
        render,
        durations[durations.len() - 1],
        7,
    );
    let mut index = 0usize;
    while let Some(f) = anim
        .render_next_frame(None)
        .map_err(|e| fail(CHECK, format!("decode: {e:?}")))?
    {
        let Some(want) = frames.get(index) else {
            return Err(fail(CHECK, "more frames decoded than encoded"));
        };
        if f.duration() != durations[index] {
            return Err(fail(
                CHECK,
                format!(
                    "frame {index} lasts {:?}, encoded {:?}",
                    f.duration(),
                    durations[index]
                ),
            ));
        }
        for y in 0..h {
            if f.pixels().row(y) != want.as_slice().row(y) {
                return Err(fail(CHECK, format!("frame {index} row {y} differs")));
            }
        }
        if f.pts().is_none() {
            return Err(fail(CHECK, format!("frame {index} has no timestamp")));
        }
        index += 1;
    }
    if index != frames.len() {
        return Err(fail(
            CHECK,
            format!("{index} frames decoded, {} encoded", frames.len()),
        ));
    }

    // A declared count binds.
    let mut a = AnimationVideoEncoder::new(make_encoder(), make_muxer(), capture(), lag_frames)
        .map_err(|e| fail(CHECK, format!("new: {e:?}")))?;
    a.begin(w, h, desc, Some(count + 1))
        .map_err(|e| fail(CHECK, format!("begin: {e:?}")))?;
    for (f, d) in frames.iter().zip(durations) {
        a.push_frame_exact(f.as_slice(), *d, None)
            .map_err(|e| fail(CHECK, format!("push: {e:?}")))?;
    }
    if a.finish(None).is_ok() {
        return Err(fail(
            CHECK,
            "finish short of the declared frame count was accepted",
        ));
    }
    Ok(())
}
