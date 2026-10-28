//! Fixed storage for input payload and its bounded range index. Sequential
//! append/read/release needs no reallocation. Out-of-order insertion and holes
//! compact the same admitted payload allocation instead of growing a tree.

use crate::CoroutineError;
use alloc::vec::Vec;
use zencodec::storage::{OwnedStorage, StorageBudget};

pub(crate) const MAX_SPANS: usize = 64;

#[derive(Clone, Copy, Default)]
struct Span {
    start: u64,
    at: usize,
    len: usize,
}
impl Span {
    fn end(self) -> u64 {
        self.start + self.len as u64
    }
}

pub(crate) struct AdmittedInput {
    data: OwnedStorage<u8>,
    spans: OwnedStorage<Span>,
    count: usize,
    retained: usize,
}

impl AdmittedInput {
    pub(crate) fn new(cap: usize, budget: &StorageBudget) -> Result<Self, CoroutineError> {
        Ok(Self {
            data: budget.bytes(cap)?,
            spans: budget.filled(MAX_SPANS, Span::default())?,
            count: 0,
            retained: 0,
        })
    }
    pub(crate) fn retained(&self) -> usize {
        self.retained
    }
    fn live(&self) -> &[Span] {
        &self.spans.as_slice()[..self.count]
    }
    fn covering(&self, pos: u64) -> Option<Span> {
        self.live()
            .iter()
            .copied()
            .find(|s| s.start <= pos && pos < s.end())
    }
    pub(crate) fn feed(&mut self, offset: u64, bytes: &[u8]) -> Result<usize, CoroutineError> {
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(CoroutineError::InputRangeOverflow)?;
        let mut start = offset;
        while let Some(s) = self.covering(start) {
            start = s.end().min(end);
            if start == end {
                break;
            }
        }
        let skip = (start - offset) as usize;
        if skip == bytes.len() {
            return Ok(skip);
        }
        let index = self.live().partition_point(|s| s.start < start);
        let gap = self.live().get(index).map_or(bytes.len() - skip, |s| {
            (s.start - start).min((bytes.len() - skip) as u64) as usize
        });
        let take = gap.min(self.data.len() - self.retained);
        if take == 0 {
            return Ok(skip);
        }
        let previous = index.checked_sub(1).map(|i| self.spans.as_slice()[i]);
        let next = self.live().get(index).copied();
        let merge_previous = previous.is_some_and(|s| s.end() == start);
        let merge_next = next.is_some_and(|s| start + take as u64 == s.start);
        if !merge_previous && !merge_next && self.count == MAX_SPANS {
            return Err(CoroutineError::InputSpanLimit { limit: MAX_SPANS });
        }
        let at = next.map_or(self.retained, |s| s.at);
        self.data
            .as_mut_slice()
            .copy_within(at..self.retained, at + take);
        self.data.as_mut_slice()[at..at + take].copy_from_slice(&bytes[skip..skip + take]);
        let spans = self.spans.as_mut_slice();
        for s in &mut spans[index..self.count] {
            s.at += take;
        }
        if merge_previous {
            spans[index - 1].len += take;
            if merge_next {
                spans[index - 1].len += spans[index].len;
                spans.copy_within(index + 1..self.count, index);
                self.count -= 1;
            }
        } else if merge_next {
            spans[index].start = start;
            spans[index].at = at;
            spans[index].len += take;
        } else {
            spans.copy_within(index..self.count, index + 1);
            spans[index] = Span {
                start,
                at,
                len: take,
            };
            self.count += 1;
        }
        self.retained += take;
        Ok(skip + take)
    }
    pub(crate) fn try_read(&self, offset: u64, out: &mut [u8]) -> bool {
        let mut filled = 0;
        while filled < out.len() {
            let Some(pos) = offset.checked_add(filled as u64) else {
                return false;
            };
            let Some(span) = self.covering(pos) else {
                return false;
            };
            let from = (pos - span.start) as usize;
            let take = (out.len() - filled).min(span.len - from);
            out[filled..filled + take]
                .copy_from_slice(&self.data.as_slice()[span.at + from..span.at + from + take]);
            filled += take;
        }
        true
    }
    pub(crate) fn missing_into(&self, offset: u64, len: usize, gaps: &mut Vec<(u64, u64)>) {
        // The read future validates this sum before requesting gaps.
        let end = offset + len as u64;
        let mut pos = offset;
        while pos < end {
            if let Some(span) = self.covering(pos) {
                pos = span.end().min(end);
            } else {
                let next = self
                    .live()
                    .iter()
                    .find(|s| s.start > pos)
                    .map_or(end, |s| s.start.min(end));
                gaps.push((pos, next - pos));
                pos = next;
            }
        }
    }
    pub(crate) fn release(&mut self, lo: u64, hi: u64) -> Result<(), CoroutineError> {
        if lo >= hi {
            return Ok(());
        }
        let split = self.live().iter().any(|s| s.start < lo && hi < s.end());
        if split && self.count == MAX_SPANS {
            return Err(CoroutineError::InputSpanLimit { limit: MAX_SPANS });
        }
        let spans = self.spans.as_mut_slice();
        for i in (0..self.count).rev() {
            let s = spans[i];
            let cut_lo = s.start.max(lo);
            let cut_hi = s.end().min(hi);
            if cut_lo >= cut_hi {
                continue;
            }
            if cut_lo == s.start && cut_hi == s.end() {
                spans.copy_within(i + 1..self.count, i);
                self.count -= 1;
            } else if cut_lo == s.start {
                let removed = (cut_hi - s.start) as usize;
                spans[i] = Span {
                    start: cut_hi,
                    at: s.at + removed,
                    len: s.len - removed,
                };
            } else if cut_hi == s.end() {
                spans[i].len = (cut_lo - s.start) as usize;
            } else {
                spans.copy_within(i + 1..self.count, i + 2);
                spans[i].len = (cut_lo - s.start) as usize;
                spans[i + 1] = Span {
                    start: cut_hi,
                    at: s.at + (cut_hi - s.start) as usize,
                    len: (s.end() - cut_hi) as usize,
                };
                self.count += 1;
            }
        }
        let mut at = 0;
        for s in &mut spans[..self.count] {
            self.data.as_mut_slice().copy_within(s.at..s.at + s.len, at);
            s.at = at;
            at += s.len;
        }
        self.retained = at;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_ranges_match_an_independent_byte_map_through_holes_and_refills() {
        let budget = StorageBudget::new(8192, 2);
        let mut store = AdmittedInput::new(128, &budget).unwrap();
        let mut expected = [None; 256];
        let mut rng = 0x456739aau32;
        for iteration in 0..10_000 {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let start = rng as usize % 240;
            let len = (rng >> 12) as usize % 16;
            if rng & 1 != 0 {
                store.release(start as u64, (start + len) as u64).unwrap();
                expected[start..start + len].fill(None);
            } else {
                let bytes = [iteration as u8; 16];
                let accepted = store.feed(start as u64, &bytes[..len]).unwrap();
                for slot in &mut expected[start..start + accepted] {
                    if slot.is_none() {
                        *slot = Some(iteration as u8);
                    }
                }
            }
            assert_eq!(
                store.retained(),
                expected.iter().filter(|v| v.is_some()).count()
            );
            for (pos, value) in expected.iter().enumerate() {
                let mut byte = [0];
                assert_eq!(store.try_read(pos as u64, &mut byte), value.is_some());
                if let Some(value) = value {
                    assert_eq!(byte[0], *value);
                }
            }
            let charged = 128 + MAX_SPANS * core::mem::size_of::<Span>();
            assert_eq!(
                (budget.reserved_bytes(), budget.reserved_objects()),
                (charged, 2)
            );
        }
        drop(store);
        assert_eq!(budget.reserved_bytes(), 0);
    }
    #[test]
    fn span_admission_and_overflow_refuse_without_mutation() {
        let budget = StorageBudget::new(8192, 2);
        let mut store = AdmittedInput::new(256, &budget).unwrap();
        for i in 0..MAX_SPANS {
            assert_eq!(store.feed((i * 4) as u64, &[1, 2, 3]).unwrap(), 3);
        }
        assert!(matches!(
            store.feed(999, &[7]),
            Err(CoroutineError::InputSpanLimit { .. })
        ));
        assert!(matches!(
            store.release(1, 2),
            Err(CoroutineError::InputSpanLimit { .. })
        ));
        assert!(matches!(
            store.feed(u64::MAX, &[1]),
            Err(CoroutineError::InputRangeOverflow)
        ));
        assert_eq!(store.retained(), MAX_SPANS * 3);
        let mut bytes = [0; 3];
        assert!(store.try_read(0, &mut bytes));
        assert_eq!(bytes, [1, 2, 3]);
        // Filling a gap merges ranges and needs no extra index slot.
        assert_eq!(store.feed(3, &[4]).unwrap(), 1);
        store.release(1, 2).unwrap();
    }
}
