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
//!
//! Borrowed Pulse arguments are optional. `None` adds no per-call control;
//! retained job controls and structural limits still apply. Forwarding adapters
//! pass the original option through to the implementation.

use alloc::boxed::Box;
use alloc::vec::Vec;

pub use zencodec::decode::{ByteRange, Input, InputRequest};
pub use zencodec::source::{AccessPattern, NativeBound};

use crate::packet::{Packet, RandomAccess};
use crate::registry::MediaError;
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
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<DemuxProgress, SessionError<Self::Error>>;

    /// What the demuxer needs next, after [`DemuxEvent::Need`]. Never empty
    /// then.
    fn requested(&self) -> &[InputRequest];

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

/// Pre-admission quote for repositioning a [`SeekableDemuxer`] without mutating
/// demuxer state or issuing any I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SeekOffer {
    /// Byte access pattern required to reach `target`.
    pub required_access: AccessPattern,
    /// Conservative bound on bytes that may be scanned to position the seek.
    pub max_scan_bytes: NativeBound<u64>,
    /// Exact or estimated landing point when already known from an in-memory index.
    pub landing_estimate: Option<SeekResult>,
    /// Whether this seek requires a container index (e.g. `moov`/`Cues`).
    pub requires_index: bool,
}

impl SeekOffer {
    /// Construct a seek admission offer.
    #[inline]
    #[must_use]
    pub const fn new(required_access: AccessPattern, max_scan_bytes: NativeBound<u64>) -> Self {
        Self {
            required_access,
            max_scan_bytes,
            landing_estimate: None,
            requires_index: false,
        }
    }

    /// Record a known landing point from an already-parsed index.
    #[inline]
    #[must_use]
    pub const fn with_landing_estimate(mut self, landing: SeekResult) -> Self {
        self.landing_estimate = Some(landing);
        self
    }

    /// Declare whether an index is required to execute this seek.
    #[inline]
    #[must_use]
    pub const fn with_requires_index(mut self, requires_index: bool) -> Self {
        self.requires_index = requires_index;
        self
    }
}

/// Extension of [`Demuxer`] supporting pre-admission seek quoting and pulse-aware seeking.
pub trait SeekableDemuxer: Demuxer {
    /// Inspect what repositioning to `target` requires without mutating the demuxer or reading.
    fn seek_offer(&self, target: &SeekTarget) -> Result<SeekOffer, SessionError<Self::Error>>;

    /// Begin repositioning to `target` after source admission succeeds.
    fn begin_seek(
        &mut self,
        target: &SeekTarget,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<SeekStatus, SessionError<Self::Error>>;
}

/// Object-safe demuxer with erased error type and pre-admission seek methods.
pub trait DynDemuxer: Demuxer<Error = ErasedError> {
    /// Inspect seek requirements without mutating state.
    fn seek_offer(&self, target: &SeekTarget) -> Result<SeekOffer, MediaError> {
        let _ = target;
        Err(MediaError::Unsupported(UnsupportedMedia::Seek))
    }

    /// Begin repositioning to `target`.
    ///
    /// Defaults to refusing with [`UnsupportedMedia::Seek`] so an unquoted
    /// [`Demuxer`] cannot bypass [`seek_offer`](Self::seek_offer) via legacy
    /// [`Demuxer::seek`]. Implement [`SeekableDemuxer`] and wrap with
    /// [`SeekableDemuxerShim`] / [`erase_seekable_demuxer`] to expose seeking.
    fn begin_seek(
        &mut self,
        target: &SeekTarget,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<SeekStatus, MediaError> {
        let _ = target;
        if let Some(p) = pulse {
            p.check().map_err(MediaError::Cancelled)?;
        }
        Err(MediaError::Unsupported(UnsupportedMedia::Seek))
    }
}

/// Wrapper erasing a concrete [`Demuxer`]'s error type into [`DynDemuxer`].
pub struct DemuxerShim<D>(pub D);

impl<D: Demuxer> Demuxer for DemuxerShim<D> {
    type Error = ErasedError;

    fn step(
        &mut self,
        input: Input<'_>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<DemuxProgress, SessionError<ErasedError>> {
        self.0
            .step(input, pulse)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }

    fn requested(&self) -> &[InputRequest] {
        self.0.requested()
    }

    fn tracks(&self) -> &[TrackInfo] {
        self.0.tracks()
    }

    fn access_pattern(&self) -> AccessPattern {
        self.0.access_pattern()
    }

    fn seek(&mut self, target: &SeekTarget) -> Result<SeekStatus, SessionError<ErasedError>> {
        self.0
            .seek(target)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
}

impl<D: Demuxer> DynDemuxer for DemuxerShim<D> {}

/// Wrapper erasing a [`SeekableDemuxer`] into [`DynDemuxer`] while preserving `seek_offer`
/// and `begin_seek`.
pub struct SeekableDemuxerShim<D>(pub D);

impl<D: SeekableDemuxer> Demuxer for SeekableDemuxerShim<D> {
    type Error = ErasedError;

    fn step(
        &mut self,
        input: Input<'_>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<DemuxProgress, SessionError<ErasedError>> {
        self.0
            .step(input, pulse)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }

    fn requested(&self) -> &[InputRequest] {
        self.0.requested()
    }

    fn tracks(&self) -> &[TrackInfo] {
        self.0.tracks()
    }

    fn access_pattern(&self) -> AccessPattern {
        self.0.access_pattern()
    }

    fn seek(&mut self, target: &SeekTarget) -> Result<SeekStatus, SessionError<ErasedError>> {
        self.0
            .seek(target)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
}

impl<D: SeekableDemuxer> DynDemuxer for SeekableDemuxerShim<D> {
    fn seek_offer(&self, target: &SeekTarget) -> Result<SeekOffer, MediaError> {
        self.0.seek_offer(target).map_err(MediaError::from)
    }

    fn begin_seek(
        &mut self,
        target: &SeekTarget,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<SeekStatus, MediaError> {
        self.0.begin_seek(target, pulse).map_err(MediaError::from)
    }
}

/// Erase a concrete [`Demuxer`] into a boxed [`DynDemuxer`].
pub fn erase_demuxer<D: Demuxer + Send + 'static>(d: D) -> Box<dyn DynDemuxer + Send> {
    Box::new(DemuxerShim(d))
}

/// Erase a [`SeekableDemuxer`] into a boxed [`DynDemuxer`], preserving seek admission.
pub fn erase_seekable_demuxer<D: SeekableDemuxer + Send + 'static>(
    d: D,
) -> Box<dyn DynDemuxer + Send> {
    Box::new(SeekableDemuxerShim(d))
}

/// Admission quote for a bounded muxer's output schedule, pending buffer, workspace,
/// and finish trailer size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MuxOffer {
    /// Streaming vs `AT_FINISH` vs patching behavior.
    pub timing: MuxTiming,
    /// Conservative upper bound on unconsumed pending bytes buffered at once.
    pub maximum_pending: NativeBound<u64>,
    /// Conservative upper bound on muxer working memory (sample tables, indexes).
    pub workspace: NativeBound<u64>,
    /// Conservative upper bound on trailing bytes emitted by `finish`.
    pub finish_bytes: NativeBound<u64>,
}

impl MuxOffer {
    /// Construct a muxer resource/schedule offer.
    #[inline]
    #[must_use]
    pub const fn new(
        timing: MuxTiming,
        maximum_pending: NativeBound<u64>,
        workspace: NativeBound<u64>,
        finish_bytes: NativeBound<u64>,
    ) -> Self {
        Self {
            timing,
            maximum_pending,
            workspace,
            finish_bytes,
        }
    }
}

/// Outcome of [`BoundedMuxer::try_write_packet`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PacketWrite {
    /// The packet was taken (`packet` is now `None`) and admitted into pending output.
    Accepted,
    /// Pending output must be drained (`pending_output` / `consume_output`) before
    /// retrying; `packet` was left untouched (`Some(...)`).
    DrainOutput,
}

/// Extension of [`DynMuxer`] with bounded output credit backpressure.
pub trait BoundedMuxer: DynMuxer {
    /// The admitted output schedule and resource bounds for this live muxer.
    fn admitted_offer(&self) -> &MuxOffer;

    /// Attempt to write `packet` without exceeding [`MuxOffer::maximum_pending`].
    ///
    /// On [`PacketWrite::Accepted`], takes `packet` (leaving `None`). On
    /// [`PacketWrite::DrainOutput`], leaves `packet` untouched and guarantees
    /// `pending_output()` is non-empty so the caller can drain and retry.
    fn try_write_packet(
        &mut self,
        packet: &mut Option<Packet>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<PacketWrite, SessionError<ErasedError>>;
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

/// Shared completion and patch types; retained paths for media implementations.
pub use zencodec::output_patch::{OutputFinish as MuxFinish, Patch, PatchError};

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

    /// Prepare a layout that preserves supplied PTS, DTS and explicit sample
    /// durations exactly, or refuse without making output available.
    ///
    /// Unlike a capability query, success binds this muxer instance. Later
    /// packets that cannot be represented must fail before their payload or
    /// timing affects pending output. Missing timestamps/durations may be
    /// refused; values must not be guessed, rounded or rebased to make a packet
    /// fit. A missing DTS may use its PTS as decode time when the format requires
    /// one. This does not certify metadata, packet additions or workspace bounds.
    ///
    /// Preparation cannot prove facts about future packets. A driver reports
    /// already committed output on a subsequent failure and must not retry a
    /// different layout. The default refuses: calling legacy `begin` alone is
    /// not evidence of this stronger contract.
    fn begin_exact_timing(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<Self::Error>> {
        let _ = (tracks, hint);
        Err(SessionError::Unsupported(UnsupportedMedia::ExactTiming))
    }

    /// Write one packet. Takes the packet so its payload can be reused
    /// without a copy.
    fn write_packet(
        &mut self,
        packet: Packet,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<Self::Error>>;

    /// Bytes ready to send, not yet consumed.
    fn pending_output(&self) -> &[u8];

    /// Mark the first `n` pending bytes as sent (clamped).
    fn consume_output(&mut self, n: usize);

    /// Append the pending bytes to `into` and mark them sent.
    fn take_output(&mut self, into: &mut Vec<u8>);

    /// Finish: the bytes not yet consumed, and any patches.
    fn finish(
        self,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<MuxFinish, SessionError<Self::Error>>
    where
        Self: Sized;
}

/// Object-safe muxer (`finish` by box), the `DynAnimationFrameEncoder` pattern.
pub trait DynMuxer {
    /// See [`Muxer::begin_exact_timing`]. Default refuses without calling `begin`.
    fn begin_exact_timing(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<ErasedError>> {
        let _ = (tracks, hint);
        Err(SessionError::Unsupported(UnsupportedMedia::ExactTiming))
    }
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
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<ErasedError>>;
    /// See [`Muxer::pending_output`].
    fn pending_output(&self) -> &[u8];
    /// See [`Muxer::consume_output`].
    fn consume_output(&mut self, n: usize);
    /// See [`Muxer::take_output`].
    fn take_output(&mut self, into: &mut Vec<u8>);
    /// See [`Muxer::finish`].
    fn finish(
        self: Box<Self>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<MuxFinish, SessionError<ErasedError>>;
    /// Borrow the bounded credit-backpressure view when this muxer supports
    /// [`BoundedMuxer`] writes (`MuxPlan::supports_bounded_writes == true`).
    fn as_bounded(&self) -> Option<&dyn BoundedMuxer> {
        None
    }
    /// Mutably borrow the bounded credit-backpressure view when this muxer supports
    /// [`BoundedMuxer`] writes (`MuxPlan::supports_bounded_writes == true`).
    fn as_bounded_mut(&mut self) -> Option<&mut dyn BoundedMuxer> {
        None
    }
}

/// Shim erasing a muxer's error type (the `DynAnimationFrameEncoder` pattern:
/// a wrapper, not a blanket impl, so method names never collide).
pub struct MuxerShim<M>(pub M);

impl<M: Muxer> DynMuxer for MuxerShim<M> {
    fn begin_exact_timing(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<ErasedError>> {
        self.0
            .begin_exact_timing(tracks, hint)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
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
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<ErasedError>> {
        self.0
            .write_packet(packet, pulse)
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
    fn finish(
        self: Box<Self>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<MuxFinish, SessionError<ErasedError>> {
        self.0
            .finish(pulse)
            .map_err(|e| e.map_codec(|c| ErasedError::new(Box::new(c))))
    }
}

/// Erase a muxer's error type.
pub fn erase_muxer<M: Muxer + Send + 'static>(m: M) -> Box<dyn DynMuxer + Send> {
    Box::new(MuxerShim(m))
}

/// Wrapper preserving [`BoundedMuxer`] access across [`DynMuxer`] erasure so routed
/// [`MuxFactory::open`](crate::registry::MuxFactory::open) sessions expose
/// [`DynMuxer::as_bounded`] / [`DynMuxer::as_bounded_mut`].
pub struct BoundedMuxerShim<M>(pub M);

impl<M: BoundedMuxer> DynMuxer for BoundedMuxerShim<M> {
    fn begin_exact_timing(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<ErasedError>> {
        self.0.begin_exact_timing(tracks, hint)
    }
    fn begin(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<ErasedError>> {
        self.0.begin(tracks, hint)
    }
    fn write_packet(
        &mut self,
        packet: Packet,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<ErasedError>> {
        self.0.write_packet(packet, pulse)
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
    fn finish(
        self: Box<Self>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<MuxFinish, SessionError<ErasedError>> {
        Box::new(self.0).finish(pulse)
    }
    fn as_bounded(&self) -> Option<&dyn BoundedMuxer> {
        Some(&self.0)
    }
    fn as_bounded_mut(&mut self) -> Option<&mut dyn BoundedMuxer> {
        Some(&mut self.0)
    }
}

/// Erase a [`BoundedMuxer`] into a boxed [`DynMuxer`], preserving bounded write access
/// through [`DynMuxer::as_bounded`] and [`DynMuxer::as_bounded_mut`].
pub fn erase_bounded_muxer<M: BoundedMuxer + Send + 'static>(m: M) -> Box<dyn DynMuxer + Send> {
    Box::new(BoundedMuxerShim(m))
}

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
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Stepped<DemuxEvent>, Self::Error> {
        let p = self.0.step(input, pulse)?;
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

    fn requested(&self) -> &[InputRequest] {
        self.0.requested()
    }
}
