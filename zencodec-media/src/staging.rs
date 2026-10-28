//! Explicit compressed-packet staging for finite encoding.
//!
//! Disk staging writes while the encoder is alive and may block. It is not
//! permission to backpressure a live encoder on the final output transport.
//! Packet metadata stays in memory; payloads and additions are preserved without
//! container rewriting. Disk files must be private scratch files exclusively
//! controlled by this operation: aliases/external mutation are not supported.
//! The caller owns the file's path, permissions, retention and deletion policy.
use crate::{
    packet::{Packet, PacketPayload},
    staged::Cause,
};
use alloc::{collections::VecDeque, vec::Vec};
use core::fmt;

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
};
use zencodec::{
    At,
    operation::{StagingFacts, StagingKind},
    storage::StorageBudget,
};

/// Independent caps for disk payload and its in-memory packet index.
#[derive(Clone, Copy, Debug)]
pub struct DiskLimits {
    /// Total packet and addition bytes appended after the scratch origin.
    pub max_bytes: u64,
    /// Maximum number of staged packets.
    pub max_packets: usize,
    /// Maximum side-data entries on each packet, including empty entries.
    pub max_additions_per_packet: usize,
}
impl DiskLimits {
    /// Zero limits are real limits. No unbounded or unlimited sentinel is used.
    pub const fn new(max_bytes: u64, max_packets: usize, max_additions_per_packet: usize) -> Self {
        Self {
            max_bytes,
            max_packets,
            max_additions_per_packet,
        }
    }
}
/// Staging refused before exceeding its configured scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// Appending this packet would exceed the disk byte allowance.
    Bytes,
    /// Appending this packet would exceed the packet-index allowance.
    Packets,
    /// Packet side-data count exceeds its index allowance.
    Additions,
    /// Scratch file must start at its existing end, preserving all prior bytes.
    NotAtEnd,
    /// A size/offset calculation cannot be represented.
    Overflow,
}
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "packet staging refused: {self:?}")
    }
}
impl core::error::Error for Refusal {}
fn refuse(reason: Refusal) -> Cause {
    alloc::boxed::Box::new(reason)
}
fn check(pulse: &dyn zencodec::Pulse) -> Result<(), Cause> {
    pulse
        .check()
        .map_err(|r| At::wrap(zencodec::CodecError::new(None, r.into())).into())
}
struct Entry {
    metadata: Packet,
    offset: u64,
    lengths: Vec<usize>,
    bytes: usize,
}
struct Disk {
    file: File,
    limits: DiskLimits,
    replay: StorageBudget,
    origin: u64,
    entries: VecDeque<Entry>,
}
enum Store {
    Memory {
        budget: StorageBudget,
        packets: VecDeque<Packet>,
    },
    Disk(Disk),
}
/// Retained store chosen before accepting the first frame.
///
/// Byte limits do not bound native codec/muxer workspace, packet metadata or
/// caller-owned pictures. Disk replay allocates admitted packet owners; a muxer
/// that retains packets keeps their charges, so replay admission can later fail.
/// No automatic disk fallback, path creation, deletion or retry occurs.
pub struct Staging {
    store: Store,
    bytes: u64,
}
impl Staging {
    /// Retain all compressed packet/addition payloads in admitted memory.
    pub fn memory(budget: StorageBudget) -> Self {
        Self {
            store: Store::Memory {
                budget,
                packets: VecDeque::new(),
            },
            bytes: 0,
        }
    }
    /// Append to an owned private scratch file at its current end. Existing
    /// prefixes are preserved. This may seek, but writes nothing at construction.
    /// Replay uses `replay` to admit owned packet bytes after the encoder drops.
    /// A failed write may leave a partial spool; the final destination is separate.
    pub fn disk(mut file: File, limits: DiskLimits, replay: StorageBudget) -> Result<Self, Cause> {
        let origin = file.stream_position()?;
        let end = file.seek(SeekFrom::End(0))?;
        if origin != end {
            return Err(refuse(Refusal::NotAtEnd));
        }
        Ok(Self {
            store: Store::Disk(Disk {
                file,
                limits,
                replay,
                origin,
                entries: VecDeque::new(),
            }),
            bytes: 0,
        })
    }
    /// Bounded policy evidence for the actual store. Disk permits blocking spool
    /// I/O during encoding; neither kind qualifies native workspace bounds.
    pub fn facts(&self) -> StagingFacts {
        let (kind, limit) = match &self.store {
            Store::Memory { budget, .. } => (StagingKind::Memory, budget.max_bytes() as u64),
            Store::Disk(disk) => (StagingKind::Disk, disk.limits.max_bytes),
        };
        StagingFacts::new(kind, self.bytes, limit)
    }
    pub(crate) fn admitted_memory(&self) -> usize {
        match &self.store {
            Store::Memory { budget, .. } => budget.reserved_bytes(),
            Store::Disk(disk) => disk.replay.reserved_bytes(),
        }
    }
    pub(crate) fn push(
        &mut self,
        packet: Packet,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<(), Cause> {
        check(pulse)?;
        let bytes = packet
            .additions()
            .iter()
            .try_fold(packet.len(), |sum, a| sum.checked_add(a.payload().len()))
            .ok_or_else(|| refuse(Refusal::Overflow))?;
        let total = self
            .bytes
            .checked_add(bytes as u64)
            .ok_or_else(|| refuse(Refusal::Overflow))?;
        match &mut self.store {
            Store::Memory { budget, packets } => {
                packets.try_reserve(1)?;
                packets.push_back(packet.accounted(budget)?);
            }
            Store::Disk(disk) => {
                if total > disk.limits.max_bytes {
                    return Err(refuse(Refusal::Bytes));
                }
                if disk.entries.len() >= disk.limits.max_packets {
                    return Err(refuse(Refusal::Packets));
                }
                if packet.additions().len() > disk.limits.max_additions_per_packet {
                    return Err(refuse(Refusal::Additions));
                }
                let offset = disk
                    .origin
                    .checked_add(self.bytes)
                    .ok_or_else(|| refuse(Refusal::Overflow))?;
                disk.origin
                    .checked_add(total)
                    .ok_or_else(|| refuse(Refusal::Overflow))?;
                disk.entries.try_reserve(1)?;
                let mut lengths = Vec::new();
                lengths.try_reserve(packet.additions().len() + 1)?;
                disk.file.seek(SeekFrom::Start(offset))?;
                let metadata = packet.map_payloads(|payload| {
                    lengths.push(payload.len());
                    for chunk in payload.as_bytes().chunks(64 * 1024) {
                        check(pulse)?;
                        disk.file.write_all(chunk)?;
                    }
                    Ok(PacketPayload::Owned(Vec::new()))
                })?;
                disk.entries.push_back(Entry {
                    metadata,
                    offset,
                    lengths,
                    bytes,
                });
            }
        }
        self.bytes = total;
        check(pulse)
    }
    pub(crate) fn begin_replay(&mut self, pulse: &dyn zencodec::Pulse) -> Result<(), Cause> {
        check(pulse)?;
        if let Store::Disk(disk) = &mut self.store {
            disk.file.flush()?;
        }
        check(pulse)
    }
    pub(crate) fn next(&mut self, pulse: &dyn zencodec::Pulse) -> Result<Option<Packet>, Cause> {
        check(pulse)?;
        match &mut self.store {
            Store::Memory { packets, .. } => Ok(packets.pop_front()),
            Store::Disk(disk) => {
                let Some(entry) = disk.entries.pop_front() else {
                    return Ok(None);
                };
                let mut payload = disk.replay.bytes(entry.bytes)?;
                disk.file.seek(SeekFrom::Start(entry.offset))?;
                for chunk in payload.as_mut_slice().chunks_mut(64 * 1024) {
                    check(pulse)?;
                    disk.file.read_exact(chunk)?;
                }
                check(pulse)?;
                let payload = payload.freeze();
                let mut lengths = entry.lengths.into_iter();
                let mut offset = 0;
                let packet = entry.metadata.map_payloads(|_| {
                    let len = lengths
                        .next()
                        .expect("private metadata/payload count agrees");
                    let next = offset + len;
                    let view = payload.slice(offset..next)?;
                    offset = next;
                    Ok(PacketPayload::Accounted(view))
                })?;
                Ok(Some(packet))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{PacketAddition, RandomAccess};
    use alloc::vec;
    use std::{
        fs::OpenOptions,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use zencodec::time::{TimeBase, Timestamp};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn file() -> (File, PathBuf) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/staging-tests");
        std::fs::create_dir_all(&root).unwrap();
        for _ in 0..1000 {
            let path = root.join(alloc::format!(
                "{}-{}.scratch",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    file.write_all(b"prefix").unwrap();
                    return (file, path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("scratch creation failed: {e}"),
            }
        }
        panic!("scratch namespace exhausted")
    }
    fn packet() -> Packet {
        Packet::from_vec(vec![1, 2, 3])
            .unwrap()
            .with_track(7)
            .with_pts(Timestamp::new(21, TimeBase::MILLIS))
            .with_dts(Timestamp::new(19, TimeBase::MILLIS))
            .with_duration_ticks(2)
            .with_config_epoch(3)
            .with_ordinal(11)
            .with_input_offset(101)
            .with_random_access(RandomAccess::Yes)
            .with_addition(PacketAddition::new(9, PacketPayload::Owned(vec![4, 5])))
    }
    #[test]
    fn disk_preserves_packet_metadata_additions_prefix_and_replay_owners() {
        let (file, path) = file();
        let budget = StorageBudget::new(5, 1);
        let mut staging = Staging::disk(file, DiskLimits::new(5, 1, 1), budget.clone()).unwrap();
        let original = packet();
        staging
            .push(original.clone(), &zencodec::how_far::NoPulse)
            .unwrap();
        assert_eq!(budget.reserved_bytes(), 0);
        assert_eq!(staging.facts().payload_bytes, 5);
        staging.begin_replay(&zencodec::how_far::NoPulse).unwrap();
        let output = staging.next(&zencodec::how_far::NoPulse).unwrap().unwrap();
        assert_eq!(output.bytes(), original.bytes());
        assert_eq!(output.track(), original.track());
        assert_eq!(output.pts(), original.pts());
        assert_eq!(output.dts(), original.dts());
        assert_eq!(output.duration_ticks(), original.duration_ticks());
        assert_eq!(output.config_epoch(), original.config_epoch());
        assert_eq!(output.ordinal(), original.ordinal());
        assert_eq!(output.input_offset(), original.input_offset());
        assert_eq!(output.random_access(), original.random_access());
        assert_eq!(output.additions()[0].id(), 9);
        assert_eq!(output.additions()[0].payload().as_bytes(), &[4, 5]);
        assert_eq!(budget.reserved_bytes(), 5);
        assert_eq!(budget.reserved_objects(), 1);
        assert!(staging.next(&zencodec::how_far::NoPulse).unwrap().is_none());
        drop(staging);
        assert_eq!(output.bytes(), &[1, 2, 3]);
        drop(output);
        assert_eq!(budget.reserved_bytes(), 0);
        assert_eq!(std::fs::read(path).unwrap(), b"prefix\x01\x02\x03\x04\x05");
    }
    #[test]
    fn disk_limits_reject_whole_packet_before_touching_scratch() {
        for (limits, expected) in [
            (DiskLimits::new(4, 1, 1), Refusal::Bytes),
            (DiskLimits::new(5, 0, 1), Refusal::Packets),
            (DiskLimits::new(5, 1, 0), Refusal::Additions),
        ] {
            let (file, path) = file();
            let mut staging = Staging::disk(file, limits, StorageBudget::new(5, 1)).unwrap();
            let error = staging
                .push(packet(), &zencodec::how_far::NoPulse)
                .unwrap_err();
            assert_eq!(error.downcast_ref::<Refusal>(), Some(&expected));
            assert_eq!(std::fs::read(path).unwrap(), b"prefix");
        }
    }
    #[test]
    fn replay_truncation_keeps_io_cause_and_releases_admission() {
        let (file, path) = file();
        let budget = StorageBudget::new(5, 1);
        let mut staging = Staging::disk(file, DiskLimits::new(5, 1, 1), budget.clone()).unwrap();
        staging.push(packet(), &zencodec::how_far::NoPulse).unwrap();
        // Deliberate external interference is fault injection, not a supported
        // scratch-sharing mode or proof of same-length mutation detection.
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(7)
            .unwrap();
        let error = staging.next(&zencodec::how_far::NoPulse).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        assert_eq!(budget.reserved_bytes(), 0);
    }
    #[test]
    fn disk_requires_end_position_without_overwriting_existing_content() {
        let (mut file, path) = file();
        file.seek(SeekFrom::Start(0)).unwrap();
        let error = Staging::disk(file, DiskLimits::new(5, 1, 1), StorageBudget::new(5, 1))
            .err()
            .unwrap();
        assert_eq!(error.downcast_ref::<Refusal>(), Some(&Refusal::NotAtEnd));
        assert_eq!(std::fs::read(path).unwrap(), b"prefix");
    }
}
