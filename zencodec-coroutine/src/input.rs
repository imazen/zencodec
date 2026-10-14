//! Bounded store of fed bytes, keyed by absolute offset.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Fed-but-unread bytes. Segments are keyed by their start offset and
/// never overlap; adjacent feeds merge; reads release what they consume.
pub(crate) struct InputStore {
    segments: BTreeMap<u64, Vec<u8>>,
    retained: usize,
    cap: usize,
    total: Option<u64>,
}

impl InputStore {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            segments: BTreeMap::new(),
            retained: 0,
            cap: cap.max(1),
            total: None,
        }
    }

    pub(crate) fn total(&self) -> Option<u64> {
        self.total
    }

    pub(crate) fn retained(&self) -> usize {
        self.retained
    }

    /// Store as much of `bytes` as fits. Returns bytes accepted (bytes
    /// already held are accepted without being stored twice).
    pub(crate) fn feed(&mut self, offset: u64, bytes: &[u8], eof: bool) -> usize {
        if eof {
            self.total = Some(offset.saturating_add(bytes.len() as u64));
        }
        if bytes.is_empty() {
            return 0;
        }
        // Skip any prefix we already hold.
        let mut skip = 0usize;
        while skip < bytes.len() && self.contains(offset + skip as u64) {
            skip += 1;
        }
        if skip == bytes.len() {
            return bytes.len();
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
            return skip;
        }
        let bytes = &bytes[..take];
        // Merge onto a segment that ends exactly here.
        let merged = match self.segments.range_mut(..offset).next_back() {
            Some((start, seg)) if *start + seg.len() as u64 == offset => {
                seg.extend_from_slice(bytes);
                true
            }
            _ => false,
        };
        if !merged {
            self.segments.insert(offset, bytes.to_vec());
        }
        self.retained += take;
        skip + take
    }

    fn contains(&self, pos: u64) -> bool {
        self.segments
            .range(..=pos)
            .next_back()
            .is_some_and(|(start, seg)| pos < *start + seg.len() as u64)
    }

    /// Copy `out.len()` bytes from `offset` if every byte is held.
    pub(crate) fn try_read(&self, offset: u64, out: &mut [u8]) -> bool {
        let mut filled = 0usize;
        while filled < out.len() {
            let pos = offset + filled as u64;
            let Some((start, seg)) = self.segments.range(..=pos).next_back() else {
                return false;
            };
            let seg_end = *start + seg.len() as u64;
            if pos >= seg_end {
                return false;
            }
            let from = (pos - start) as usize;
            let n = (seg.len() - from).min(out.len() - filled);
            out[filled..filled + n].copy_from_slice(&seg[from..from + n]);
            filled += n;
        }
        true
    }

    /// The spans of `offset .. offset + len` that are not held, in order.
    pub(crate) fn missing(&self, offset: u64, len: usize) -> Vec<(u64, u64)> {
        let end = offset + len as u64;
        let mut gaps = Vec::new();
        let mut pos = offset;
        while pos < end {
            match self.segments.range(..=pos).next_back() {
                Some((start, seg)) if pos < *start + seg.len() as u64 => {
                    pos = *start + seg.len() as u64;
                }
                _ => {
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
        gaps
    }

    /// Whether a read of `len` at `offset` can never be satisfied.
    pub(crate) fn beyond_end(&self, offset: u64, len: usize) -> bool {
        self.total
            .is_some_and(|t| offset.saturating_add(len as u64) > t)
    }

    /// Drop every held byte in `offset .. offset + len`.
    pub(crate) fn release(&mut self, offset: u64, len: usize) {
        if len == 0 {
            return;
        }
        let end = offset + len as u64;
        let keys: Vec<u64> = self
            .segments
            .range(..end)
            .filter(|(start, seg)| *start + seg.len() as u64 > offset)
            .map(|(k, _)| *k)
            .collect();
        for start in keys {
            let seg = self.segments.remove(&start).expect("key listed");
            let seg_end = start + seg.len() as u64;
            self.retained -= seg.len();
            if start < offset {
                let head = seg[..(offset - start) as usize].to_vec();
                self.retained += head.len();
                self.segments.insert(start, head);
            }
            if seg_end > end {
                let tail = seg[(end - start) as usize..].to_vec();
                self.retained += tail.len();
                self.segments.insert(end, tail);
            }
        }
    }

    /// Drop every held byte before `offset`.
    pub(crate) fn release_before(&mut self, offset: u64) {
        let first = self.segments.keys().next().copied();
        if let Some(first) = first
            && first < offset
        {
            self.release(first, (offset - first) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_merge_read_release() {
        let mut s = InputStore::new(1024);
        assert_eq!(s.feed(0, &[1, 2, 3], false), 3);
        assert_eq!(s.feed(3, &[4, 5], false), 2);
        assert_eq!(s.segments.len(), 1, "adjacent feeds merge");
        assert_eq!(s.feed(10, &[9, 9], true), 2);
        assert_eq!(s.total(), Some(12));
        let mut out = [0u8; 5];
        assert!(s.try_read(0, &mut out));
        assert_eq!(out, [1, 2, 3, 4, 5]);
        assert!(!s.try_read(4, &mut [0u8; 3]), "gap at 5..10");
        assert_eq!(s.missing(3, 9), [(5, 5)]);
        assert!(s.missing(0, 2).is_empty());
        assert_eq!(s.missing(5, 2), [(5, 2)]);
        assert_eq!(s.missing(9, 5), [(9, 1), (12, 2)]);
        assert!(s.beyond_end(11, 2));
        s.release(1, 2);
        assert_eq!(s.retained(), 5);
        assert!(s.try_read(0, &mut [0u8; 1]));
        assert!(!s.try_read(1, &mut [0u8; 1]));
        assert!(s.try_read(3, &mut [0u8; 2]));
        s.release_before(11);
        assert_eq!(s.retained(), 1);
        assert!(s.try_read(11, &mut [0u8; 1]));
    }

    #[test]
    fn cap_limits_and_duplicates_are_free() {
        let mut s = InputStore::new(4);
        assert_eq!(s.feed(0, &[1, 2, 3, 4, 5, 6], false), 4);
        assert_eq!(s.feed(0, &[1, 2, 3, 4], false), 4, "already held");
        assert_eq!(s.feed(4, &[5, 6], false), 0, "full");
        s.release(0, 2);
        assert_eq!(s.feed(4, &[5, 6], false), 2);
    }
}
