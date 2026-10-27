//! Containers.
//!
//! A [`Demuxer`] is sans-I/O and speaks the image decoders' dialogue: the
//! driver offers an [`Input`], the demuxer takes a prefix and either yields a
//! [`DemuxEvent`] or names what it [`requested`](Demuxer::requested) next.
//! [`DemuxStage`] makes any demuxer a [`zencodec_io::ByteStepper`], so every
//! `zencodec-io` cursor (slice, `ReadAt` with coalesced ranges, `Read`,
//! async) drives it; a container that needs its index first (an MP4 with
//! `moov` at the end) asks for that range and declares
//! [`AccessPattern::RandomAccess`].
//!
//! A [`Muxer`] takes packets and lets the caller pull its bytes through
//! `pending_output` / `consume_output` / `take_output`, as a streaming image
//! encoder does. [`Muxer::begin`] says, through [`MuxTiming`], whether bytes
//! stream and whether `finish` patches earlier bytes (sizes, offsets); a
//! caller whose destination cannot seek back passes
//! [`MuxHint::forward_only`] and gets a layout without patches, or a refusal
//! at `begin`.

use alloc::boxed::Box;
use alloc::vec::Vec;

use enough::Stop;
pub use zencodec::decode::{ByteRange, Input, Request};
pub use zencodec::source::AccessPattern;

use crate::packet::{Packet, RandomAccess};
use crate::session::{ErasedError, SessionError, UnsupportedMedia};
use crate::track::TrackInfo;
use zencodec::time::Timestamp;
use zencodec_io::{ByteStepper, Step, Stepped};

#[derive(Debug)]
#[non_exhaustive]
/// What one demuxer step produced.
pub enum DemuxEvent {
    /// Satisfy one of `requested()` and step again.
    Need,
    /// [`Demuxer::tracks`] is populated.
    Headers,
    /// One packet; [`Packet::track`] says whose.
    Packet(Packet),
    /// A track's codec configuration changed; packets that follow carry the
    /// new epoch. Decoders get `configure` before the next packet.
    ConfigChanged {
        /// The track.
        track: u32,
        /// The new epoch.
        epoch: u32,
    },
    /// A pending `seek` is positioned; packets resume there.
    Seeked(SeekResult),
    /// No more packets.
    Done,
}

/// The result of one [`Demuxer::step`].
#[derive(Debug)]
pub struct DemuxProgress {
    /// Bytes taken from the front of the input.
    pub consumed: usize,
    /// What the step produced.
    pub event: DemuxEvent,
}

impl DemuxProgress {
    /// A step that took `consumed` bytes and produced `event`.
    pub const fn new(consumed: usize, event: DemuxEvent) -> Self {
        Self { consumed, event }
    }

    /// A step that took `consumed` bytes and needs more.
    pub const fn need(consumed: usize) -> Self {
        Self::new(consumed, DemuxEvent::Need)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// Which presentation a seek selects relative to the target time.
pub enum SeekMode {
    /// The nearest; on a tie, the earlier.
    Nearest,
    /// The last at or before the target.
    AtOrBefore,
    /// The first at or after the target.
    AtOrAfter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// Which packets a seek may land on.
pub enum SeekScope {
    /// Any packet; the caller decodes from an earlier random-access point
    /// and discards up to it.
    Any,
    /// Only packets the container marks as random-access points.
    RandomAccessPoints,
}

/// Where to seek.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct SeekTarget {
    /// The track whose timeline is meant.
    pub track: u32,
    /// The target instant.
    pub time: Timestamp,
    /// How to pick around it.
    pub mode: SeekMode,
    /// Which packets qualify.
    pub scope: SeekScope,
}

impl SeekTarget {
    /// A seek on `track` to `time`.
    pub const fn new(track: u32, time: Timestamp, mode: SeekMode, scope: SeekScope) -> Self {
        Self {
            track,
            time,
            mode,
            scope,
        }
    }
}

/// What a seek actually landed on: the real timestamp and ordinal, never the
/// request echoed back, plus the container's random-access evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SeekResult {
    /// The presentation timestamp of the packet the demuxer resumes at.
    pub pts: Timestamp,
    /// Its ordinal.
    pub ordinal: u64,
    /// The container's random-access evidence there.
    pub random_access: RandomAccess,
    /// The configuration epoch in force there.
    pub config_epoch: u32,
}

impl SeekResult {
    /// A landing position.
    pub const fn new(
        pts: Timestamp,
        ordinal: u64,
        random_access: RandomAccess,
        config_epoch: u32,
    ) -> Self {
        Self {
            pts,
            ordinal,
            random_access,
            config_epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// What `seek` did.
pub enum SeekStatus {
    /// Positioned at once.
    Positioned(SeekResult),
    /// The container needs bytes (an index scan, a cue read): keep stepping;
    /// `DemuxEvent::Seeked` reports the position.
    Scanning,
}

/// A sans-I/O container reader. See the [module docs](self).
pub trait Demuxer {
    /// The implementation's error (malformed or truncated input).
    type Error: core::error::Error + Send + Sync + 'static;

    /// Take a prefix of `input` and report what happened.
    fn step(
        &mut self,
        input: Input<'_>,
        stop: &dyn Stop,
    ) -> Result<DemuxProgress, SessionError<Self::Error>>;

    /// What the demuxer needs next, after [`DemuxEvent::Need`]. Never empty
    /// then.
    fn requested(&self) -> &[Request];

    /// The tracks; empty before [`DemuxEvent::Headers`].
    fn tracks(&self) -> &[TrackInfo];

    /// How the demuxer reads its input. Default: front to back.
    fn access_pattern(&self) -> AccessPattern {
        AccessPattern::Sequential
    }
    /// Reposition. Default: unsupported.
    fn seek(&mut self, target: &SeekTarget) -> Result<SeekStatus, SessionError<Self::Error>> {
        let _ = target;
        Err(SessionError::Unsupported(UnsupportedMedia::Seek))
    }
}

/// When a muxer's bytes arrive and whether it must patch earlier bytes at
/// `finish` (the writer-capability split: seekable fix-ups versus
/// forward-only emission).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MuxTiming {
    streams: bool,
    patches_at_finish: bool,
}

impl MuxTiming {
    /// Every byte comes from `finish`.
    pub const AT_FINISH: Self = Self {
        streams: false,
        patches_at_finish: false,
    };

    /// Bytes stream through `pending_output`; nothing is patched.
    pub const fn streaming() -> Self {
        Self {
            streams: true,
            patches_at_finish: false,
        }
    }
    /// Bytes stream, and `finish` patches some earlier ones.
    pub const fn streaming_with_patches() -> Self {
        Self {
            streams: true,
            patches_at_finish: true,
        }
    }
    /// Whether bytes arrive before `finish`.
    pub const fn streams(&self) -> bool {
        self.streams
    }

    /// Whether `finish` overwrites earlier bytes.
    pub const fn patches_at_finish(&self) -> bool {
        self.patches_at_finish
    }
}

#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
/// What the caller tells a muxer about its destination.
pub struct MuxHint {
    /// The destination cannot seek back: a muxer that would patch must
    /// choose a forward-only layout or refuse at `begin`.
    pub forward_only: bool,
}

impl MuxHint {
    /// A destination that cannot seek back (a socket, a pipe).
    pub const fn forward_only() -> Self {
        Self { forward_only: true }
    }
}

/// Bytes to write over earlier output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    /// File offset of the first byte.
    pub offset: u64,
    /// The new bytes.
    pub bytes: Vec<u8>,
}

#[derive(Debug, Default)]
#[non_exhaustive]
/// What a muxer's `finish` returns.
pub struct MuxFinish {
    /// Bytes not yet consumed through `pending_output`.
    pub tail: Vec<u8>,
    /// Earlier bytes to overwrite; empty for a forward-only layout.
    pub patches: Vec<Patch>,
}

/// A container writer whose bytes the caller pulls. See the
/// [module docs](self).
pub trait Muxer {
    /// The implementation's error.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Declare the tracks and learn how bytes will arrive. A second `begin`
    /// is a [`ProtocolViolation::BeginTwice`](crate::session::ProtocolViolation::BeginTwice).
    fn begin(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<Self::Error>>;

    /// Write one packet. Takes the packet so its payload can be reused
    /// without a copy.
    fn write_packet(
        &mut self,
        packet: Packet,
        stop: &dyn Stop,
    ) -> Result<(), SessionError<Self::Error>>;

    /// Bytes ready to send, not yet consumed.
    fn pending_output(&self) -> &[u8];

    /// Mark the first `n` pending bytes as sent (clamped).
    fn consume_output(&mut self, n: usize);

    /// Append the pending bytes to `into` and mark them sent.
    fn take_output(&mut self, into: &mut Vec<u8>);

    /// Finish: the bytes not yet consumed, and any patches.
    fn finish(self, stop: &dyn Stop) -> Result<MuxFinish, SessionError<Self::Error>>
    where
        Self: Sized;
}

/// Object-safe muxer (`finish` by box), the `DynAnimationFrameEncoder` pattern.
pub trait DynMuxer {
    /// See [`Muxer::begin`].
    fn begin(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<ErasedError>>;
    /// See [`Muxer::write_packet`].
    fn write_packet(
        &mut self,
        packet: Packet,
        stop: &dyn Stop,
    ) -> Result<(), SessionError<ErasedError>>;
    /// See [`Muxer::pending_output`].
    fn pending_output(&self) -> &[u8];
    /// See [`Muxer::consume_output`].
    fn consume_output(&mut self, n: usize);
    /// See [`Muxer::take_output`].
    fn take_output(&mut self, into: &mut Vec<u8>);
    /// See [`Muxer::finish`].
    fn finish(self: Box<Self>, stop: &dyn Stop) -> Result<MuxFinish, SessionError<ErasedError>>;
}

/// Shim erasing a muxer's error type (the `DynAnimationFrameEncoder` pattern:
/// a wrapper, not a blanket impl, so method names never collide).
pub struct MuxerShim<M>(pub M);

impl<M: Muxer> DynMuxer for MuxerShim<M> {
    fn begin(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<ErasedError>> {
        self.0
            .begin(tracks, hint)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
    fn write_packet(
        &mut self,
        packet: Packet,
        stop: &dyn Stop,
    ) -> Result<(), SessionError<ErasedError>> {
        self.0
            .write_packet(packet, stop)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
    fn pending_output(&self) -> &[u8] {
        self.0.pending_output()
    }
    fn consume_output(&mut self, n: usize) {
        self.0.consume_output(n)
    }
    fn take_output(&mut self, into: &mut Vec<u8>) {
        self.0.take_output(into)
    }
    fn finish(self: Box<Self>, stop: &dyn Stop) -> Result<MuxFinish, SessionError<ErasedError>> {
        self.0
            .finish(stop)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
}

/// Erase a muxer's error type.
pub fn erase_muxer<M: Muxer + Send + 'static>(m: M) -> Box<dyn DynMuxer + Send> {
    Box::new(MuxerShim(m))
}

impl MuxFinish {
    /// The remaining bytes and the patches to apply after them.
    pub fn new(tail: Vec<u8>, patches: Vec<Patch>) -> Self {
        Self { tail, patches }
    }

    /// Validate the complete finish before appending its tail or applying any
    /// patch. Offsets are relative to this muxed stream, not a surrounding file.
    /// Returns the final stream length, including the tail.
    ///
    /// This checks ranges and arithmetic only. A writer must separately enforce
    /// its output allowance and whether its chosen layout permits patches.
    /// Overlapping patches apply in their listed order. Successful validation
    /// cannot make an external writer's later I/O failures atomic.
    pub fn validate(&self, emitted_bytes: u64) -> Result<u64, PatchError> {
        let tail = u64::try_from(self.tail.len()).map_err(|_| PatchError::OutOfRange)?;
        let total = emitted_bytes
            .checked_add(tail)
            .ok_or(PatchError::OutOfRange)?;
        for patch in &self.patches {
            let length = u64::try_from(patch.bytes.len()).map_err(|_| PatchError::OutOfRange)?;
            let end = patch
                .offset
                .checked_add(length)
                .ok_or(PatchError::OutOfRange)?;
            if end > total {
                return Err(PatchError::OutOfRange);
            }
        }
        Ok(total)
    }

    /// Append the tail and apply validated patches to an in-memory stream.
    /// Validation or allocation failure leaves its length and contents unchanged.
    /// Capacity may grow on success; this does not impose a memory allowance.
    pub fn apply_to(self, file: &mut Vec<u8>) -> Result<(), PatchError> {
        self.validate(u64::try_from(file.len()).map_err(|_| PatchError::OutOfRange)?)?;
        file.try_reserve(self.tail.len())
            .map_err(|_| PatchError::AllocationFailed)?;
        file.extend_from_slice(&self.tail);
        for patch in self.patches {
            // Successful reservation proves the complete extent fits usize.
            // validate checked every range against that same extent.
            let start = patch.offset as usize;
            file[start..start + patch.bytes.len()].copy_from_slice(&patch.bytes);
        }
        Ok(())
    }
}

/// Apply a finish to an in-memory file: append the tail, then patch.
///
/// # Errors
///
/// See [`MuxFinish::apply_to`].
#[deprecated(
    note = "use finish.apply_to(file); use finish.validate(emitted_bytes) before external writes"
)]
pub fn apply_finish(file: &mut Vec<u8>, finish: MuxFinish) -> Result<(), PatchError> {
    finish.apply_to(file)
}

/// A patch that does not fit the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PatchError {
    /// The patch lies outside the written bytes.
    OutOfRange,
    /// The in-memory destination could not reserve the remaining output.
    AllocationFailed,
}

impl core::fmt::Display for PatchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::OutOfRange => "patch or final length lies outside the written stream",
            Self::AllocationFailed => "cannot reserve remaining muxed output",
        })
    }
}
impl core::error::Error for PatchError {}

/// A [`Demuxer`] as a [`ByteStepper`], so every `zencodec-io` cursor drives
/// it: `Need` maps to [`Step::Need`], `Done` to [`Step::Finished`], and
/// every other event is an output.
#[derive(Debug)]
pub struct DemuxStage<'a, D: ?Sized>(pub &'a mut D);

impl<D: Demuxer + ?Sized> ByteStepper for DemuxStage<'_, D> {
    type Output = DemuxEvent;
    type Error = SessionError<D::Error>;

    fn step(
        &mut self,
        input: Input<'_>,
        stop: &dyn Stop,
    ) -> Result<Stepped<DemuxEvent>, Self::Error> {
        let p = self.0.step(input, stop)?;
        let step = match p.event {
            DemuxEvent::Need => Step::Need,
            DemuxEvent::Done => Step::Finished(DemuxEvent::Done),
            other => Step::Output(other),
        };
        Ok(Stepped {
            consumed: p.consumed,
            step,
        })
    }

    fn requested(&self) -> &[Request] {
        self.0.requested()
    }
}
