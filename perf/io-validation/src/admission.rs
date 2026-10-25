//! Executable candidate contracts. Private to the experiment; no library API.
//! Storage is charged by allocation, across transfer/cache/decode ownership.
#![allow(dead_code)]
use std::sync::{Arc, Mutex};
use zencodec::decode::ByteRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Bounds,
    Descriptors,
    Bytes,
    Allocation,
    Identity,
    Incomplete,
    Duplicate,
}

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub bytes: usize,
    pub descriptors: usize,
    pub range_bytes: usize,
    pub merge_gap: u64,
}

/// All listed ranges are dependencies. This deliberately differs from the
/// existing decoder `requested()` alternatives, any ONE of which may suffice.
pub(crate) fn plan_all(
    input: &[ByteRange],
    length: u64,
    limits: Limits,
) -> Result<Vec<ByteRange>, Refusal> {
    if input.len() > limits.descriptors {
        return Err(Refusal::Descriptors);
    }
    if limits.range_bytes == 0 {
        return Err(Refusal::Bytes);
    }
    for r in input {
        if r.length == 0
            || r.offset
                .checked_add(r.length)
                .is_none_or(|end| end > length)
        {
            return Err(Refusal::Bounds);
        }
    }
    let mut sorted = Vec::new();
    sorted
        .try_reserve_exact(input.len())
        .map_err(|_| Refusal::Allocation)?;
    sorted.extend_from_slice(input);
    sorted.sort_unstable_by_key(|r| (r.offset, r.length));
    let mut merged: Vec<ByteRange> = Vec::new();
    merged
        .try_reserve_exact(input.len())
        .map_err(|_| Refusal::Allocation)?;
    for r in sorted {
        if let Some(last) = merged.last_mut()
            && r.offset <= last.end()
        {
            last.length = last.end().max(r.end()) - last.offset;
        } else {
            merged.push(r);
        }
    }
    let useful = merged.iter().try_fold(0u64, |n, r| {
        n.checked_add(r.length)
            .filter(|n| *n <= limits.bytes as u64)
            .ok_or(Refusal::Bytes)
    })?;
    let mut slack = limits.bytes as u64 - useful;
    let mut count = 0usize;
    for i in 0..merged.len() {
        let r = merged[i];
        if count > 0 {
            let last = &mut merged[count - 1];
            let gap = r.offset - last.end();
            if gap <= limits.merge_gap
                && gap <= slack
                && r.end() - last.offset <= limits.range_bytes as u64
            {
                last.length = r.end() - last.offset;
                slack -= gap;
                continue;
            }
        }
        merged[count] = r;
        count += 1;
    }
    merged.truncate(count);
    // Count split descriptors and total bytes before allocating their buffers.
    let mut count = 0usize;
    let mut bytes = 0u64;
    for r in &merged {
        let pieces = r.length.div_ceil(limits.range_bytes as u64);
        count = count
            .checked_add(usize::try_from(pieces).map_err(|_| Refusal::Descriptors)?)
            .filter(|n| *n <= limits.descriptors)
            .ok_or(Refusal::Descriptors)?;
        bytes = bytes
            .checked_add(r.length)
            .filter(|n| *n <= limits.bytes as u64)
            .ok_or(Refusal::Bytes)?;
    }
    let mut out = Vec::new();
    out.try_reserve_exact(count)
        .map_err(|_| Refusal::Allocation)?;
    for r in merged {
        let mut offset = r.offset;
        while offset < r.end() {
            let n = (r.end() - offset).min(limits.range_bytes as u64);
            out.push(ByteRange::new(offset, n));
            offset += n;
        }
    }
    Ok(out)
}

#[derive(Default, Debug, Clone, Copy)]
pub(crate) struct Occupancy {
    pub bytes: usize,
    pub objects: usize,
    pub peak_bytes: usize,
}

struct Shared {
    limits: Limits,
    used: Mutex<Occupancy>,
}

#[derive(Clone)]
pub(crate) struct Storage(Arc<Shared>);
impl Storage {
    pub fn new(limits: Limits) -> Self {
        Self(Arc::new(Shared {
            limits,
            used: Mutex::new(Occupancy::default()),
        }))
    }
    pub fn occupancy(&self) -> Occupancy {
        *self.0.used.lock().unwrap()
    }
    pub fn reserve(&self, size: usize) -> Result<Reservation, Refusal> {
        let mut used = self.0.used.lock().unwrap();
        let bytes = used
            .bytes
            .checked_add(size)
            .filter(|n| *n <= self.0.limits.bytes)
            .ok_or(Refusal::Bytes)?;
        if used.objects >= self.0.limits.descriptors {
            return Err(Refusal::Descriptors);
        }
        used.bytes = bytes;
        used.objects += 1;
        used.peak_bytes = used.peak_bytes.max(bytes);
        Ok(Reservation {
            storage: self.clone(),
            size,
        })
    }
}
pub(crate) struct Reservation {
    storage: Storage,
    size: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut used = self.storage.0.used.lock().unwrap();
        used.bytes -= self.size;
        used.objects -= 1;
    }
}

/// Identity is an application-issued snapshot token, not a URL or first-seen
/// per-part ETag. A real HTTP adapter must establish the token's semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity(pub u64);

pub(crate) struct Transfer {
    // Cancellation must free the payload before making its reservation reusable.
    bytes: Vec<u8>,
    reservation: Reservation,
    identity: Identity,
    range: ByteRange,
    filled: usize,
    failed: bool,
}
impl Transfer {
    pub fn new(storage: &Storage, identity: Identity, range: ByteRange) -> Result<Self, Refusal> {
        if range.length == 0 {
            return Err(Refusal::Bounds);
        }
        range
            .offset
            .checked_add(range.length)
            .ok_or(Refusal::Bounds)?;
        let n = usize::try_from(range.length).map_err(|_| Refusal::Bytes)?;
        let reservation = storage.reserve(n)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(n)
            .map_err(|_| Refusal::Allocation)?;
        if bytes.capacity() > reservation.size {
            return Err(Refusal::Allocation);
        }
        bytes.resize(n, 0);
        Ok(Self {
            reservation,
            identity,
            range,
            bytes,
            filled: 0,
            failed: false,
        })
    }
    pub fn append(&mut self, chunk: &[u8]) -> Result<(), Refusal> {
        if self.failed {
            return Err(Refusal::Bytes);
        }
        let Some(end) = self
            .filled
            .checked_add(chunk.len())
            .filter(|n| *n <= self.bytes.len())
        else {
            self.failed = true;
            return Err(Refusal::Bytes);
        };
        self.bytes[self.filled..end].copy_from_slice(chunk);
        self.filled = end;
        Ok(())
    }
    pub fn publish(self, verified_identity: Identity) -> Result<Lease, Refusal> {
        if self.failed {
            return Err(Refusal::Bytes);
        }
        if self.identity != verified_identity {
            return Err(Refusal::Identity);
        }
        if self.filled != self.bytes.len() {
            return Err(Refusal::Incomplete);
        }
        Ok(Lease(Arc::new(Owned {
            // Drop bytes before the reservation releases their storage charge.
            bytes: self.bytes,
            _reservation: self.reservation,
            identity: self.identity,
            range: self.range,
        })))
    }
}
struct Owned {
    bytes: Vec<u8>,
    _reservation: Reservation,
    identity: Identity,
    range: ByteRange,
}
#[derive(Clone)]
pub(crate) struct Lease(Arc<Owned>);
impl Lease {
    pub fn bytes(&self) -> &[u8] {
        &self.0.bytes
    }
    pub fn identity(&self) -> Identity {
        self.0.identity
    }
    pub fn range(&self) -> ByteRange {
        self.0.range
    }
}

/// Completed packets may arrive out of order, while decode order stays strict.
/// Each waiting value keeps its storage lease. Only the next `limit` ordinals
/// can complete into the queue, preserving a slot for the missing head. Callers
/// must also admit the head's transport storage before launching later work.
pub(crate) struct Ordered {
    next: u64,
    generation: u64,
    limit: usize,
    ready: std::collections::BTreeMap<u64, Lease>,
}
impl Ordered {
    pub fn new(limit: usize) -> Self {
        Self {
            next: 0,
            generation: 0,
            limit,
            ready: Default::default(),
        }
    }
    pub fn insert(&mut self, generation: u64, ordinal: u64, lease: Lease) -> Result<(), Refusal> {
        if generation != self.generation {
            return Err(Refusal::Identity);
        }
        if ordinal < self.next || self.ready.contains_key(&ordinal) {
            return Err(Refusal::Duplicate);
        }
        if !usize::try_from(ordinal - self.next).is_ok_and(|delta| delta < self.limit) {
            return Err(Refusal::Descriptors);
        }
        self.ready.insert(ordinal, lease);
        Ok(())
    }
    pub fn pop(&mut self) -> Option<Lease> {
        let lease = self.ready.remove(&self.next)?;
        self.next += 1;
        Some(lease)
    }
    pub fn reset(&mut self, generation: u64) {
        self.generation = generation;
        self.next = 0;
        self.ready.clear();
    }
}
