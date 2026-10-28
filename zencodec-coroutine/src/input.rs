//! Bounded store of fed bytes, keyed by absolute offset.

use crate::{CoroutineError, admitted_input::AdmittedInput};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use zencodec::storage::StorageBudget;

/// One run of held bytes. `data[head..]` is live and starts at absolute
/// offset `start`; releasing from the front moves `head` instead of
/// copying, and the buffer is compacted once the dead prefix dominates.
struct Segment {
    start: u64,
    data: Vec<u8>,
    head: usize,
}

impl Segment {
    fn live(&self) -> &[u8] {
        &self.data[self.head..]
    }
    fn len(&self) -> usize {
        self.data.len() - self.head
    }
    fn end(&self) -> u64 {
        self.start + self.len() as u64
    }
}

/// Fed-but-unread bytes. Segments are keyed by their start offset and
/// never overlap; adjacent feeds merge; reads release what they consume.
pub(crate) struct InputStore {
    admitted: Option<AdmittedInput>,
    segments: BTreeMap<u64, Segment>,
    retained: usize,
    cap: usize,
    total: Option<u64>,
    /// Buffer of the last fully consumed segment, reused by the next
    /// feed so a strip-per-chunk stream allocates no new buffer.
    spare: Option<Vec<u8>>,
}

impl InputStore {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            admitted: None,
            segments: BTreeMap::new(),
            retained: 0,
            cap: cap.max(1),
            total: None,
            spare: None,
        }
    }

    pub(crate) fn with_storage(cap: usize, budget: &StorageBudget) -> Result<Self, CoroutineError> {
        Ok(Self {
            admitted: Some(AdmittedInput::new(cap.max(1), budget)?),
            ..Self::new(cap)
        })
    }

    pub(crate) fn clear(&mut self) {
        self.admitted = None;
        self.segments.clear();
        self.spare = None;
        self.retained = 0;
    }

    pub(crate) fn total(&self) -> Option<u64> {
        self.total
    }

    pub(crate) fn set_total(&mut self, total: u64) {
        self.total = Some(total);
    }

    pub(crate) fn retained(&self) -> usize {
        self.admitted
            .as_ref()
            .map_or(self.retained, AdmittedInput::retained)
    }

    pub(crate) fn cap(&self) -> usize {
        self.cap
    }

    /// The segment holding `pos`, if any.
    fn segment_at(&self, pos: u64) -> Option<(&u64, &Segment)> {
        self.segments
            .range(..=pos)
            .next_back()
            .filter(|(_, seg)| pos < seg.end())
    }

    /// Store as much of `bytes` as fits. Returns bytes accepted (bytes
    /// already held are accepted without being stored twice).
    pub(crate) fn feed(
        &mut self,
        offset: u64,
        bytes: &[u8],
        eof: bool,
    ) -> Result<usize, CoroutineError> {
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(CoroutineError::InputRangeOverflow)?;
        if eof {
            self.total = Some(end);
        }
        if let Some(admitted) = &mut self.admitted {
            return admitted.feed(offset, bytes);
        }
        if bytes.is_empty() {
            return Ok(0);
        }
        // Skip any prefix we already hold.
        let mut skip = 0usize;
        while skip < bytes.len() {
            match self.segment_at(offset + skip as u64) {
                Some((_, seg)) => skip = (seg.end() - offset) as usize,
                None => break,
            }
        }
        if skip >= bytes.len() {
            return Ok(bytes.len());
        }
        let offset = offset + skip as u64;
        let bytes = &bytes[skip..];
        // Do not run into a segment that starts inside this chunk.
        let limit = self
            .segments
            .range(offset..)
            .next()
            .map(|(start, _)| (*start - offset) as usize)
            .unwrap_or(bytes.len())
            .min(bytes.len());
        let room = self.cap.saturating_sub(self.retained);
        let take = limit.min(room);
        if take == 0 {
            return Ok(skip);
        }
        let bytes = &bytes[..take];
        // Merge onto a segment that ends exactly here.
        let merged = match self.segments.range_mut(..offset).next_back() {
            Some((_, seg)) if seg.end() == offset => {
                seg.data.extend_from_slice(bytes);
                true
            }
            _ => false,
        };
        if !merged {
            let mut data = self.spare.take().unwrap_or_default();
            data.clear();
            data.extend_from_slice(bytes);
            self.segments.insert(
                offset,
                Segment {
                    start: offset,
                    data,
                    head: 0,
                },
            );
        }
        self.retained += take;
        Ok(skip + take)
    }

    /// Copy `out.len()` bytes from `offset` if every byte is held.
    pub(crate) fn try_read(&self, offset: u64, out: &mut [u8]) -> bool {
        if let Some(admitted) = &self.admitted {
            return admitted.try_read(offset, out);
        }
        let mut filled = 0usize;
        while filled < out.len() {
            let pos = offset + filled as u64;
            let Some((_, seg)) = self.segment_at(pos) else {
                return false;
            };
            let from = (pos - seg.start) as usize;
            let live = seg.live();
            let n = (live.len() - from).min(out.len() - filled);
            out[filled..filled + n].copy_from_slice(&live[from..from + n]);
            filled += n;
        }
        true
    }

    /// Append the spans of `offset .. offset + len` that are not held.
    pub(crate) fn missing_into(&self, offset: u64, len: usize, gaps: &mut Vec<(u64, u64)>) {
        if let Some(admitted) = &self.admitted {
            return admitted.missing_into(offset, len, gaps);
        }
        let end = offset + len as u64;
        let mut pos = offset;
        while pos < end {
            match self.segment_at(pos) {
                Some((_, seg)) => pos = seg.end(),
                None => {
                    let next_start = self
                        .segments
                        .range(pos..)
                        .next()
                        .map(|(s, _)| *s)
                        .unwrap_or(end)
                        .min(end);
                    gaps.push((pos, next_start - pos));
                    pos = next_start;
                }
            }
        }
    }

    /// Whether a read of `len` at `offset` can never be satisfied.
    pub(crate) fn beyond_end(&self, offset: u64, len: usize) -> bool {
        self.total
            .is_some_and(|t| offset.saturating_add(len as u64) > t)
    }

    /// Drop every held byte in `offset .. offset + len`.
    ///
    /// Releasing a segment's front moves its head (no copy, amortised
    /// compaction); releasing its back truncates; a hole in the middle
    /// splits it, which is the only case that copies.
    pub(crate) fn release(&mut self, offset: u64, len: usize) -> Result<(), CoroutineError> {
        let end = offset
            .checked_add(len as u64)
            .ok_or(CoroutineError::InputRangeOverflow)?;
        if let Some(admitted) = &mut self.admitted {
            return admitted.release(offset, end);
        }
        if len == 0 {
            return Ok(());
        }
        // One overlapping segment at a time, so a release allocates
        // nothing: find the last segment that starts before `end` and
        // overlaps, handle it, repeat.
        let mut scan_end = end;
        while let Some(key) = self
            .segments
            .range(..scan_end)
            .next_back()
            .filter(|(_, seg)| seg.end() > offset)
            .map(|(k, _)| *k)
        {
            scan_end = key;
            let mut seg = self.segments.remove(&key).expect("key listed");
            let seg_start = seg.start;
            let seg_end = seg.end();
            let cut_lo = offset.max(seg_start);
            let cut_hi = end.min(seg_end);
            self.retained -= (cut_hi - cut_lo) as usize;
            if cut_lo == seg_start && cut_hi == seg_end {
                // Whole segment gone: keep its buffer for the next feed.
                if self
                    .spare
                    .as_ref()
                    .is_none_or(|s| s.capacity() < seg.data.capacity())
                {
                    self.spare = Some(seg.data);
                }
                continue;
            }
            if cut_lo == seg_start {
                // Front release: advance the head, compact occasionally.
                seg.head += (cut_hi - seg_start) as usize;
                seg.start = cut_hi;
                if seg.head > seg.data.len() / 2 && seg.head >= 4096 {
                    seg.data.drain(..seg.head);
                    seg.head = 0;
                }
                self.segments.insert(seg.start, seg);
            } else if cut_hi == seg_end {
                // Back release: truncate.
                seg.data.truncate(seg.head + (cut_lo - seg_start) as usize);
                self.segments.insert(seg.start, seg);
            } else {
                // Hole: keep both sides.
                let tail_from = seg.head + (cut_hi - seg_start) as usize;
                let tail = Segment {
                    start: cut_hi,
                    data: seg.data[tail_from..].to_vec(),
                    head: 0,
                };
                seg.data.truncate(seg.head + (cut_lo - seg_start) as usize);
                self.segments.insert(seg.start, seg);
                self.segments.insert(tail.start, tail);
            }
        }
        Ok(())
    }

    /// Drop every held byte before `offset`.
    pub(crate) fn release_before(&mut self, offset: u64) {
        if let Some(admitted) = &mut self.admitted {
            // A prefix cannot split a span or grow the index.
            admitted
                .release(0, offset)
                .expect("prefix release cannot grow the range index");
            return;
        }
        let first = self.segments.keys().next().copied();
        if let Some(first) = first
            && first < offset
        {
            self.release(first, (offset - first) as usize)
                .expect("prefix range is representable");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(s: &InputStore, offset: u64, len: usize) -> Vec<(u64, u64)> {
        let mut v = Vec::new();
        s.missing_into(offset, len, &mut v);
        v
    }

    #[test]
    fn feed_merge_read_release() {
        let mut s = InputStore::new(1024);
        assert_eq!(s.feed(0, &[1, 2, 3], false).unwrap(), 3);
        assert_eq!(s.feed(3, &[4, 5], false).unwrap(), 2);
        assert_eq!(s.segments.len(), 1, "adjacent feeds merge");
        assert_eq!(s.feed(10, &[9, 9], true).unwrap(), 2);
        assert_eq!(s.total(), Some(12));
        let mut out = [0u8; 5];
        assert!(s.try_read(0, &mut out));
        assert_eq!(out, [1, 2, 3, 4, 5]);
        assert!(!s.try_read(4, &mut [0u8; 3]), "gap at 5..10");
        assert_eq!(missing(&s, 3, 9), [(5, 5)]);
        assert!(missing(&s, 0, 2).is_empty());
        assert_eq!(missing(&s, 5, 2), [(5, 2)]);
        assert_eq!(missing(&s, 9, 5), [(9, 1), (12, 2)]);
        assert!(s.beyond_end(11, 2));
        s.release(1, 2).unwrap();
        assert_eq!(s.retained(), 5);
        assert!(s.try_read(0, &mut [0u8; 1]));
        assert!(!s.try_read(1, &mut [0u8; 1]));
        let mut two = [0u8; 2];
        assert!(s.try_read(3, &mut two));
        assert_eq!(two, [4, 5]);
        s.release_before(11);
        assert_eq!(s.retained(), 1);
        let mut one = [0u8; 1];
        assert!(s.try_read(11, &mut one));
        assert_eq!(one, [9]);
    }

    #[test]
    fn cap_limits_and_duplicates_are_free() {
        let mut s = InputStore::new(4);
        assert_eq!(s.feed(0, &[1, 2, 3, 4, 5, 6], false).unwrap(), 4);
        assert_eq!(s.feed(0, &[1, 2, 3, 4], false).unwrap(), 4, "already held");
        assert_eq!(s.feed(4, &[5, 6], false).unwrap(), 0, "full");
        s.release(0, 2).unwrap();
        assert_eq!(s.feed(4, &[5, 6], false).unwrap(), 2);
        let mut out = [0u8; 4];
        assert!(s.try_read(2, &mut out));
        assert_eq!(out, [3, 4, 5, 6]);
    }

    #[test]
    fn front_release_is_a_head_move_and_compacts() {
        let mut s = InputStore::new(1 << 20);
        let data: Vec<u8> = (0..20_000u32).map(|i| i as u8).collect();
        assert_eq!(s.feed(0, &data, true).unwrap(), 20_000);
        let cap_before = s.segments[&0].data.capacity();
        let mut buf = [0u8; 100];
        for i in 0..100u64 {
            assert!(s.try_read(i * 100, &mut buf));
            assert_eq!(buf[0], (i * 100) as u8);
            s.release(i * 100, 100).unwrap();
        }
        assert_eq!(s.retained(), 10_000);
        let seg = s.segments.values().next().unwrap();
        assert_eq!(seg.start, 10_000);
        assert!(
            seg.data.capacity() <= cap_before,
            "no reallocation on front release"
        );
        assert!(s.try_read(10_000, &mut buf));
        assert_eq!(buf[0], 10_000u32 as u8);
    }
}
