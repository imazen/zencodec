//! Compressed access units.
//!
//! A [`Packet`] is what a demuxer yields and a decoder takes, and what an
//! encoder yields and a muxer takes. It is never a picture: a decoder may
//! produce zero, one or several frames per packet, and may produce them
//! later (reordering, hidden frames, show-existing).
//!
//! The payload is a [`PacketPayload`]: an owned `Vec`, or a range of a shared
//! buffer, so a demuxer can hand out slices of one read without copying and
//! a backend can take an owned `Vec` without copying either. A consumer that
//! accepts a packet takes its payload ([`Packet::take`]); one that pushes
//! back leaves it in place for the retry.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use zencodec::time::{FrameDuration, Timestamp};

/// Where a packet's bytes live.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PacketPayload {
    /// Bytes the packet owns.
    Owned(Vec<u8>),
    /// `len` bytes of `bytes` starting at `start`.
    Shared {
        /// The shared buffer.
        bytes: Arc<[u8]>,
        /// First byte of the payload.
        start: usize,
        /// Payload length.
        len: usize,
    },
}

impl PacketPayload {
    /// The payload bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Owned(v) => v,
            Self::Shared { bytes, start, len } => &bytes[*start..*start + *len],
        }
    }

    /// Payload length in bytes.
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    /// Whether the payload is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The payload as an owned vector: free for `Owned`, a copy for `Shared`.
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            Self::Owned(v) => v,
            Self::Shared { .. } => self.as_bytes().to_vec(),
        }
    }
}

/// What the container says about random access at a packet.
///
/// This is container evidence (an MP4 sync sample, a WebM `SimpleBlock`
/// keyframe flag, a cue point), not a codec's keyframe flag: an AV1 key
/// frame that needs an earlier sequence header, or one shown later through
/// show-existing, is not a random-access point by itself. `Unknown` is
/// common (IVF records nothing) and never treated as `Yes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum RandomAccess {
    /// Decoding can start here.
    Yes,
    /// Decoding cannot start here.
    No,
    /// The container does not say.
    #[default]
    Unknown,
}

/// Side data carried with one packet: a WebM `BlockAdditional`, an alpha
/// plane coded separately. How additions pair with the main payload is the
/// codec's business, not the packet's.
#[derive(Clone, Debug)]
pub struct PacketAddition {
    id: u32,
    payload: PacketPayload,
}

impl PacketAddition {
    /// Side data with the container's identifier.
    pub fn new(id: u32, payload: PacketPayload) -> Self {
        Self { id, payload }
    }

    /// The container's identifier for this kind of side data.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// The side data.
    pub fn payload(&self) -> &PacketPayload {
        &self.payload
    }
}

/// Decoded media to drop from one packet's output: padding the encoder added
/// (WebM `DiscardPadding`, the end of an Opus or AAC stream).
///
/// Carried untouched by demuxers, muxers and stream copies, and applied
/// once, by the decoder adapter configured from the track. A container that
/// can record only one side (WebM: a positive `DiscardPadding` trims the
/// end, a negative one the start) refuses a packet that needs both rather
/// than dropping one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Discard {
    /// Media to drop from the start of the packet's output.
    pub start: Option<FrameDuration>,
    /// Media to drop from the end of the packet's output.
    pub end: Option<FrameDuration>,
}

impl Discard {
    /// Drop `d` from the start of the packet's output.
    pub const fn start(d: FrameDuration) -> Self {
        Self {
            start: Some(d),
            end: None,
        }
    }

    /// Drop `d` from the end of the packet's output.
    pub const fn end(d: FrameDuration) -> Self {
        Self {
            start: None,
            end: Some(d),
        }
    }

    /// Also drop `d` from the end.
    pub const fn with_end(mut self, d: FrameDuration) -> Self {
        self.end = Some(d);
        self
    }
}

/// A compressed access unit and what the container recorded about it.
///
/// Every timing field is optional, and a missing value stays missing:
/// nothing in this crate derives a timestamp from a nominal frame rate. A
/// timestamp uses the track's [`TimeBase`](zencodec::time::TimeBase).
#[derive(Clone, Debug)]
pub struct Packet {
    payload: Option<PacketPayload>,
    track: u32,
    pts: Option<Timestamp>,
    dts: Option<Timestamp>,
    duration_ticks: Option<u64>,
    random_access: RandomAccess,
    config_epoch: u32,
    ordinal: Option<u64>,
    input_offset: Option<u64>,
    discard: Option<Discard>,
    additions: Vec<PacketAddition>,
}

/// Invalid packet construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PacketError {
    /// A packet must carry at least one byte.
    Empty,
    /// The shared range lies outside its buffer.
    OutOfRange,
}

impl fmt::Display for PacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "packet payload is empty",
            Self::OutOfRange => "shared payload range is outside its buffer",
        })
    }
}

impl core::error::Error for PacketError {}

impl Packet {
    fn with_payload(payload: PacketPayload) -> Self {
        Self {
            payload: Some(payload),
            track: 0,
            pts: None,
            dts: None,
            duration_ticks: None,
            random_access: RandomAccess::Unknown,
            config_epoch: 0,
            ordinal: None,
            input_offset: None,
            discard: None,
            additions: Vec::new(),
        }
    }

    /// A packet owning `bytes`, on track 0 with nothing else recorded.
    ///
    /// # Errors
    ///
    /// [`PacketError::Empty`] for an empty payload.
    pub fn from_vec(bytes: Vec<u8>) -> Result<Self, PacketError> {
        if bytes.is_empty() {
            return Err(PacketError::Empty);
        }
        Ok(Self::with_payload(PacketPayload::Owned(bytes)))
    }

    /// A packet over `len` bytes of `bytes` from `start`, without copying.
    ///
    /// # Errors
    ///
    /// [`PacketError::Empty`] for `len == 0`; [`PacketError::OutOfRange`]
    /// when the range leaves the buffer.
    pub fn from_shared(bytes: Arc<[u8]>, start: usize, len: usize) -> Result<Self, PacketError> {
        if len == 0 {
            return Err(PacketError::Empty);
        }
        if start.checked_add(len).is_none_or(|end| end > bytes.len()) {
            return Err(PacketError::OutOfRange);
        }
        Ok(Self::with_payload(PacketPayload::Shared {
            bytes,
            start,
            len,
        }))
    }

    /// The track this packet belongs to: an index into the demuxer's
    /// [`tracks`](crate::container::Demuxer::tracks), or into what a muxer
    /// got at `begin`.
    pub fn with_track(mut self, track: u32) -> Self {
        self.track = track;
        self
    }

    /// The presentation timestamp.
    pub fn with_pts(mut self, pts: Timestamp) -> Self {
        self.pts = Some(pts);
        self
    }

    /// The decode timestamp, when the container stores one apart from the
    /// presentation timestamp.
    pub fn with_dts(mut self, dts: Timestamp) -> Self {
        self.dts = Some(dts);
        self
    }

    /// The duration, in ticks of the track's time base, when the container
    /// records one.
    pub fn with_duration_ticks(mut self, ticks: u64) -> Self {
        self.duration_ticks = Some(ticks);
        self
    }

    /// The container's random-access evidence.
    pub fn with_random_access(mut self, ra: RandomAccess) -> Self {
        self.random_access = ra;
        self
    }

    /// The codec configuration epoch this packet was coded under.
    pub fn with_config_epoch(mut self, epoch: u32) -> Self {
        self.config_epoch = epoch;
        self
    }

    /// The packet's position in the track's packet order.
    pub fn with_ordinal(mut self, ordinal: u64) -> Self {
        self.ordinal = Some(ordinal);
        self
    }

    /// Where the payload starts in the container file.
    pub fn with_input_offset(mut self, offset: u64) -> Self {
        self.input_offset = Some(offset);
        self
    }

    /// Media the decoder adapter drops from this packet's output.
    pub fn with_discard(mut self, discard: Discard) -> Self {
        self.discard = Some(discard);
        self
    }

    /// Attach side data.
    pub fn with_addition(mut self, addition: PacketAddition) -> Self {
        self.additions.push(addition);
        self
    }

    /// Payload bytes; empty once an accepting consumer took them.
    pub fn bytes(&self) -> &[u8] {
        self.payload.as_ref().map_or(&[], PacketPayload::as_bytes)
    }

    /// Payload length; zero once taken.
    pub fn len(&self) -> usize {
        self.bytes().len()
    }

    /// Whether no payload bytes are left (empty only once taken).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether an accepting consumer took the payload.
    pub fn is_taken(&self) -> bool {
        self.payload.is_none()
    }

    /// Take the payload: what a consumer does when it accepts the packet.
    /// The metadata stays readable.
    pub fn take(&mut self) -> Option<PacketPayload> {
        self.payload.take()
    }

    /// The track this packet belongs to.
    pub fn track(&self) -> u32 {
        self.track
    }

    /// The presentation timestamp, if recorded.
    pub fn pts(&self) -> Option<Timestamp> {
        self.pts
    }

    /// The decode timestamp, if recorded apart from the presentation one.
    pub fn dts(&self) -> Option<Timestamp> {
        self.dts
    }

    /// Decode-order time: `dts` when the container stores one, else `pts`.
    pub fn decode_time(&self) -> Option<Timestamp> {
        self.dts.or(self.pts)
    }

    /// The recorded duration in ticks of the track's time base.
    pub fn duration_ticks(&self) -> Option<u64> {
        self.duration_ticks
    }

    /// The container's random-access evidence.
    pub fn random_access(&self) -> RandomAccess {
        self.random_access
    }

    /// The configuration epoch this packet was coded under.
    pub fn config_epoch(&self) -> u32 {
        self.config_epoch
    }

    /// The packet's position in its track's packet order.
    pub fn ordinal(&self) -> Option<u64> {
        self.ordinal
    }

    /// Where the payload starts in the container file.
    pub fn input_offset(&self) -> Option<u64> {
        self.input_offset
    }

    /// Media to drop from this packet's output, as the container recorded it.
    pub fn discard(&self) -> Option<Discard> {
        self.discard
    }

    /// Side data carried with the packet.
    pub fn additions(&self) -> &[PacketAddition] {
        &self.additions
    }
}
