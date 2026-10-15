//! Structural limits for containers and the queues between stages.
//!
//! [`zencodec::ResourceLimits`] keeps bounding decoded pixels, frames,
//! duration and memory. [`MediaLimits`] bounds what a container parser and a
//! session hold before any pixel exists: packet sizes, track counts, header
//! and index sizes, queue depths. Each is checked before the allocation it
//! guards. A refusal is a [`MediaLimitExceeded`] naming the
//! [`zencodec::LimitKind`].

use core::fmt;

use zencodec::LimitKind;

/// Caps on container structure and queue depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MediaLimits {
    /// Largest packet payload, in bytes.
    pub max_packet_bytes: usize,
    /// Most tracks one container may declare.
    pub max_tracks: u32,
    /// Most header bytes read before the first packet.
    pub max_header_bytes: u64,
    /// Most entries in an index or sample table.
    pub max_table_entries: u64,
    /// Most packets queued between two stages.
    pub max_queued_packets: usize,
    /// Most decoded frames a codec session holds for output.
    pub max_queued_frames: usize,
}

impl MediaLimits {
    /// Untrusted-input defaults: 64 MiB packets, 64 tracks, 256 MiB of
    /// headers, 4 Mi table entries, 64 queued packets, 8 queued frames.
    pub const DEFAULT: Self = Self {
        max_packet_bytes: 64 << 20,
        max_tracks: 64,
        max_header_bytes: 256 << 20,
        max_table_entries: 4 << 20,
        max_queued_packets: 64,
        max_queued_frames: 8,
    };

    /// The largest packet payload.
    pub const fn with_max_packet_bytes(mut self, n: usize) -> Self {
        self.max_packet_bytes = n;
        self
    }

    /// The most tracks.
    pub const fn with_max_tracks(mut self, n: u32) -> Self {
        self.max_tracks = n;
        self
    }

    /// The most header bytes.
    pub const fn with_max_header_bytes(mut self, n: u64) -> Self {
        self.max_header_bytes = n;
        self
    }

    /// The most index or table entries.
    pub const fn with_max_table_entries(mut self, n: u64) -> Self {
        self.max_table_entries = n;
        self
    }

    /// The most queued packets.
    pub const fn with_max_queued_packets(mut self, n: usize) -> Self {
        self.max_queued_packets = n;
        self
    }

    /// The most queued frames.
    pub const fn with_max_queued_frames(mut self, n: usize) -> Self {
        self.max_queued_frames = n;
        self
    }

    /// Refuse a packet of `bytes` bytes over the cap.
    ///
    /// # Errors
    ///
    /// [`LimitKind::PacketBytes`] over the cap.
    pub fn check_packet_bytes(&self, bytes: u64) -> Result<(), MediaLimitExceeded> {
        MediaLimitExceeded::check(LimitKind::PacketBytes, bytes, self.max_packet_bytes as u64)
    }

    /// Refuse a container declaring more than `max_tracks` tracks.
    ///
    /// # Errors
    ///
    /// [`LimitKind::Tracks`] over the cap.
    pub fn check_tracks(&self, tracks: u64) -> Result<(), MediaLimitExceeded> {
        MediaLimitExceeded::check(LimitKind::Tracks, tracks, u64::from(self.max_tracks))
    }

    /// Refuse a header of `bytes` bytes over the cap.
    ///
    /// # Errors
    ///
    /// [`LimitKind::HeaderBytes`] over the cap.
    pub fn check_header_bytes(&self, bytes: u64) -> Result<(), MediaLimitExceeded> {
        MediaLimitExceeded::check(LimitKind::HeaderBytes, bytes, self.max_header_bytes)
    }

    /// Refuse a table of `entries` entries over the cap.
    ///
    /// # Errors
    ///
    /// [`LimitKind::TableEntries`] over the cap.
    pub fn check_table_entries(&self, entries: u64) -> Result<(), MediaLimitExceeded> {
        MediaLimitExceeded::check(LimitKind::TableEntries, entries, self.max_table_entries)
    }
}

impl Default for MediaLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A structural limit was exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MediaLimitExceeded {
    /// Which cap.
    pub kind: LimitKind,
    /// What the input asked for.
    pub requested: u64,
    /// The cap.
    pub limit: u64,
}

impl MediaLimitExceeded {
    /// `Err` when `requested` exceeds `limit`.
    ///
    /// # Errors
    ///
    /// The exceeded limit.
    pub const fn check(kind: LimitKind, requested: u64, limit: u64) -> Result<(), Self> {
        if requested > limit {
            Err(Self {
                kind,
                requested,
                limit,
            })
        } else {
            Ok(())
        }
    }
}

impl fmt::Display for MediaLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} limit exceeded: {} > {}",
            self.kind, self.requested, self.limit
        )
    }
}

impl core::error::Error for MediaLimitExceeded {}
