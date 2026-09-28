//! A bounded, decoded presentation index and exact timestamp extraction for IVF.
//!
//! IVF has no seek table. Building this index reads and decodes the complete
//! stream once, retaining metadata and sequence headers, never decoded pictures.
//! The owned source must remain unchanged for the lifetime of the index.

use crate::{
    av1::{Av1Frame, Av1IvfDecoder, DecodeError},
    ivf::{IvfInfo, IvfReader},
    time::{TimeBase, Timestamp},
};
use std::{
    cmp::Ordering,
    io::{self, Read, Seek, SeekFrom},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameSelection {
    Nearest,
    AtOrBefore,
    AtOrAfter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameScope {
    All,
    Keyframes,
}

struct PacketEntry {
    offset: u64,
    len: usize,
    ticks: i64,
    sequence: Option<Arc<[u8]>>,
    used: bool,
}
struct Presentation {
    ticks: i64,
    packet: usize,
    keyframe: bool,
}

/// An owned result. The returned frame reports its actual PTS; requested time is
/// never assigned to its pixels. Equal-distance ties choose the earlier PTS,
/// then the earliest presentation ordinal among duplicate timestamps.
pub struct ExtractedFrame {
    frame: Av1Frame,
    presentation_index: usize,
    packets_read: usize,
}
impl ExtractedFrame {
    pub fn frame(&self) -> &Av1Frame {
        &self.frame
    }
    pub fn into_frame(self) -> Av1Frame {
        self.frame
    }
    pub fn presentation_index(&self) -> usize {
        self.presentation_index
    }
    /// Compressed packets read during this extraction, excluding index creation.
    /// Includes a failed anchor attempt if decoding had to restart at the start.
    pub fn packets_read(&self) -> usize {
        self.packets_read
    }
}

pub struct IndexedAv1<R> {
    reader: R,
    info: IvfInfo,
    packets: Vec<PacketEntry>,
    frames: Vec<Presentation>,
    time_order: Vec<usize>,
    key_time_order: Vec<usize>,
    max_packet_bytes: usize,
    settings: rav1d_safe::Settings,
    stop: Option<Arc<dyn enough::Stop>>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn reserve<T>(items: &mut Vec<T>, count: usize) -> io::Result<()> {
    items
        .try_reserve(count)
        .map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "index allocation failed"))
}

impl<R: Read + Seek> IndexedAv1<R> {
    /// Index from byte zero. `max_entries` bounds packets and presentations
    /// independently; `max_sequence_bytes` bounds retained unique header bytes.
    /// Both metadata vectors and sorting indices are O(max_entries). Codec
    /// references and bounded compressed-packet storage are additional memory.
    /// A stop token cancels index construction and subsequent extraction.
    ///
    /// This IVF adapter requires at most one visible presentation per packet.
    /// Hidden coded pictures inside a temporal unit are supported. Multilayer
    /// output and invisible-frame output are rejected rather than misindexed.
    pub fn build(
        mut reader: R,
        max_packet_bytes: usize,
        max_entries: usize,
        max_sequence_bytes: usize,
        settings: rav1d_safe::Settings,
        stop: Option<Arc<dyn enough::Stop>>,
    ) -> Result<Self, DecodeError> {
        if max_entries == 0
            || max_sequence_bytes == 0
            || settings.output_invisible_frames
            || settings.all_layers
        {
            return Err(
                invalid("index needs positive limits and visible single-layer output").into(),
            );
        }
        reader.seek(SeekFrom::Start(0))?;
        let mut decoder = Av1IvfDecoder::new(reader, max_packet_bytes, settings.clone())?;
        decoder.set_stop(stop.clone());
        let info = decoder.info();
        let mut packets: Vec<PacketEntry> = Vec::new();
        let mut frames: Vec<Presentation> = Vec::new();
        let mut sequence: Option<Arc<[u8]>> = None;
        let mut sequence_bytes = 0_usize;
        loop {
            let frame = decoder.next_frame_observed(&mut |packet| {
                if packets.len() == max_entries {
                    return Err(invalid("packet index limit exceeded"));
                }
                if let Some(header) = sequence_header(&packet.data)?
                    && sequence.as_deref() != Some(header)
                {
                    sequence_bytes = sequence_bytes
                        .checked_add(header.len())
                        .filter(|&n| n <= max_sequence_bytes)
                        .ok_or_else(|| invalid("sequence-header index limit exceeded"))?;
                    let mut bytes = Vec::new();
                    reserve(&mut bytes, header.len())?;
                    bytes.extend_from_slice(header);
                    sequence = Some(bytes.into());
                }
                reserve(&mut packets, 1)?;
                packets.push(PacketEntry {
                    offset: packet.byte_offset,
                    len: packet.data.len(),
                    ticks: packet.timestamp.ticks(),
                    sequence: sequence.clone(),
                    used: false,
                });
                Ok(())
            })?;
            let Some(frame) = frame else { break };
            if frames.len() == max_entries {
                return Err(invalid("presentation index limit exceeded").into());
            }
            let offset = frame
                .input_offset()
                .ok_or_else(|| invalid("missing presentation packet offset"))?;
            let packet = packets
                .binary_search_by_key(&offset, |p| p.offset)
                .map_err(|_| invalid("presentation does not identify an indexed packet"))?;
            if std::mem::replace(&mut packets[packet].used, true) {
                return Err(invalid(
                    "multiple visible presentations in one IVF packet are unsupported",
                )
                .into());
            }
            let ticks = frame
                .timestamp()
                .ok_or_else(|| invalid("missing presentation timestamp"))?
                .ticks();
            if ticks != packets[packet].ticks {
                return Err(invalid("presentation timestamp differs from its packet").into());
            }
            reserve(&mut frames, 1)?;
            frames.push(Presentation {
                ticks,
                packet,
                keyframe: frame.is_keyframe(),
            });
        }
        let mut time_order = Vec::new();
        reserve(&mut time_order, frames.len())?;
        time_order.extend(0..frames.len());
        time_order.sort_unstable_by_key(|&i| (frames[i].ticks, i));
        let mut key_time_order = Vec::new();
        reserve(
            &mut key_time_order,
            frames.iter().filter(|f| f.keyframe).count(),
        )?;
        key_time_order.extend(time_order.iter().copied().filter(|&i| frames[i].keyframe));
        Ok(Self {
            reader: decoder.into_inner(),
            info,
            packets,
            frames,
            time_order,
            key_time_order,
            max_packet_bytes,
            settings,
            stop,
        })
    }

    pub fn info(&self) -> IvfInfo {
        self.info
    }
    pub fn presentation_count(&self) -> usize {
        self.frames.len()
    }
    pub fn keyframe_count(&self) -> usize {
        self.key_time_order.len()
    }
    pub fn set_stop(&mut self, stop: Option<Arc<dyn enough::Stop>>) {
        self.stop = stop;
    }
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Find and decode a presentation. Nearest selects an endpoint outside the
    /// stream; directional queries return None if no candidate satisfies them.
    /// Seeking recreates codec state, supplies the retained sequence header and
    /// decodes forward from the preceding visible key picture. If that IVF
    /// packet also contains earlier dependent coding data, restart from byte
    /// zero rather than claiming that the picture flag guarantees packet access.
    pub fn extract(
        &mut self,
        requested: Timestamp,
        selection: FrameSelection,
        scope: FrameScope,
    ) -> Result<Option<ExtractedFrame>, DecodeError> {
        let order = match scope {
            FrameScope::All => &self.time_order,
            FrameScope::Keyframes => &self.key_time_order,
        };
        let Some(target) = select(
            &self.frames,
            order,
            self.info.time_base(),
            requested,
            selection,
        ) else {
            return Ok(None);
        };
        let anchor = self.frames[..=target]
            .iter()
            .rposition(|f| f.keyframe)
            .map_or(0, |i| self.frames[i].packet);
        let mut packets_read = 0;
        let result = self.decode_from(anchor, target, &mut packets_read);
        let frame = match result {
            Err(DecodeError::Codec(ref error))
                if anchor != 0 && error.error() == &rav1d_safe::Error::InvalidData =>
            {
                self.decode_from(0, target, &mut packets_read)?
            }
            result => result?,
        };
        Ok(Some(ExtractedFrame {
            frame,
            presentation_index: target,
            packets_read,
        }))
    }

    fn decode_from(
        &mut self,
        start: usize,
        target: usize,
        packets_read: &mut usize,
    ) -> Result<Av1Frame, DecodeError> {
        let input = IvfReader::resume(
            &mut self.reader,
            self.info,
            self.packets[start].offset,
            start as u64,
            self.max_packet_bytes,
        )?;
        let mut decoder = Av1IvfDecoder::from_input(
            input,
            self.settings.clone(),
            self.packets[start].sequence.as_deref(),
        )?;
        decoder.set_stop(self.stop.clone());
        let expected = &self.frames[target];
        loop {
            let frame = decoder
                .next_frame_observed(&mut |packet| {
                    *packets_read += 1;
                    let indexed = usize::try_from(packet.ordinal)
                        .ok()
                        .and_then(|i| self.packets.get(i))
                        .ok_or_else(|| invalid("source gained packets after indexing"))?;
                    if indexed.offset != packet.byte_offset
                        || indexed.len != packet.data.len()
                        || indexed.ticks != packet.timestamp.ticks()
                    {
                        return Err(invalid("source packet metadata changed after indexing"));
                    }
                    Ok(())
                })?
                .ok_or_else(|| invalid("indexed presentation disappeared"))?;
            if frame.input_offset() == Some(self.packets[expected.packet].offset) {
                if frame.timestamp() != Some(Timestamp::new(expected.ticks, self.info.time_base()))
                    || frame.is_keyframe() != expected.keyframe
                {
                    return Err(invalid("indexed presentation metadata changed").into());
                }
                return Ok(frame);
            }
        }
    }
}

fn select(
    frames: &[Presentation],
    order: &[usize],
    clock: TimeBase,
    target: Timestamp,
    mode: FrameSelection,
) -> Option<usize> {
    if order.is_empty() {
        return None;
    }
    let compare = |&i: &usize| Timestamp::new(frames[i].ticks, clock).compare(target);
    let after = order.partition_point(|i| compare(i) == Ordering::Less);
    let upper = order.partition_point(|i| compare(i) != Ordering::Greater);
    let before = upper.checked_sub(1).map(|i| {
        let ticks = frames[order[i]].ticks;
        order[order.partition_point(|&j| frames[j].ticks < ticks)]
    });
    let after = order.get(after).copied();
    match mode {
        FrameSelection::AtOrBefore => before,
        FrameSelection::AtOrAfter => after,
        FrameSelection::Nearest => match (before, after) {
            (Some(a), Some(b)) => {
                // Each signed cross product fits i128, but their difference may
                // not. abs_diff returns u128 and preserves the complete domain.
                let target_value = i128::from(target.ticks())
                    * i128::from(target.time_base().numerator())
                    * i128::from(clock.denominator());
                let distance = |i: usize| {
                    (i128::from(frames[i].ticks)
                        * i128::from(clock.numerator())
                        * i128::from(target.time_base().denominator()))
                    .abs_diff(target_value)
                };
                Some(if distance(a) <= distance(b) { a } else { b })
            }
            (a, b) => a.or(b),
        },
    }
}

/// Parse only bounded OBU framing, leaving AV1 semantics to the native decoder.
/// Distinct sequence headers inside one temporal unit need a finer-grained
/// index and are rejected. Unknown OBU kinds are skipped by their declared size.
fn sequence_header(data: &[u8]) -> io::Result<Option<&[u8]>> {
    let mut cursor = 0;
    let mut sequence = None;
    while cursor < data.len() {
        let start = cursor;
        let header = data[cursor];
        cursor += 1;
        if header & 0x81 != 0 {
            return Err(invalid("invalid OBU header bits"));
        }
        if header & 4 != 0 {
            let extension = *data
                .get(cursor)
                .ok_or_else(|| invalid("truncated OBU extension"))?;
            if extension & 7 != 0 {
                return Err(invalid("invalid OBU extension bits"));
            }
            cursor += 1;
        }
        let len = if header & 2 != 0 {
            let mut value = 0_u64;
            let mut complete = false;
            for shift in (0..56).step_by(7) {
                let byte = *data
                    .get(cursor)
                    .ok_or_else(|| invalid("truncated OBU size"))?;
                cursor += 1;
                value |= u64::from(byte & 127) << shift;
                if byte & 128 == 0 {
                    complete = true;
                    break;
                }
            }
            if !complete || value > u64::from(u32::MAX) {
                return Err(invalid("invalid OBU size"));
            }
            usize::try_from(value).map_err(|_| invalid("OBU size exceeds addressable memory"))?
        } else {
            data.len() - cursor
        };
        cursor = cursor
            .checked_add(len)
            .filter(|&n| n <= data.len())
            .ok_or_else(|| invalid("truncated OBU payload"))?;
        if (header >> 3) & 15 == 1 {
            let bytes = &data[start..cursor];
            if sequence.is_some_and(|prior| prior != bytes) {
                return Err(invalid("multiple sequence headers in a temporal unit"));
            }
            sequence = Some(bytes);
        }
    }
    Ok(sequence)
}
