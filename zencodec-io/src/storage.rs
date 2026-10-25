//! Storage admission tied to the allocation's last owner.
//!
//! The allowance covers retained vector capacity managed by this module. It is
//! not an RSS or whole-codec bound: allocator bookkeeping, reference-count
//! headers, transient allocation behavior, native codec workspace and explicit
//! copies made outside this module are excluded. Each allocation also consumes
//! one object slot. Clones and subviews share that charge.

use alloc::{collections::TryReserveError, sync::Arc, vec::Vec};
use core::{
    fmt,
    mem::size_of,
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

/// Storage could not be admitted or a requested view was invalid.
#[derive(Debug)]
#[non_exhaustive]
pub enum StorageError {
    /// The requested capacity exceeds the remaining managed byte allowance.
    Bytes {
        /// Bytes requested by this allocation.
        requested: usize,
        /// Total configured allowance.
        limit: usize,
    },
    /// All managed allocation slots are occupied, including empty allocations.
    Objects {
        /// Total configured allocation slots.
        limit: usize,
    },
    /// Element-count arithmetic overflowed or exceeded addressable storage.
    SizeOverflow,
    /// The allocator refused the admitted allocation.
    Allocation(TryReserveError),
    /// The allocator exposed more capacity than admitted. That temporary
    /// allocation is freed before returning; it is never published to a caller.
    Capacity {
        /// Admitted byte count.
        admitted: usize,
        /// Actual vector capacity in bytes.
        actual: usize,
    },
    /// A subview lies outside its parent view.
    Range,
}
impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes { requested, limit } => write!(
                f,
                "cannot admit {requested} bytes under managed storage limit {limit}"
            ),
            Self::Objects { limit } => {
                write!(f, "managed storage allocation limit {limit} reached")
            }
            Self::SizeOverflow => f.write_str("storage size is not addressable"),
            Self::Allocation(_) => f.write_str("admitted storage allocation failed"),
            Self::Capacity { admitted, actual } => write!(
                f,
                "allocator exposed {actual} bytes for an admission of {admitted}"
            ),
            Self::Range => f.write_str("storage view is outside its parent"),
        }
    }
}
impl core::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            _ => None,
        }
    }
}

struct State {
    max_bytes: usize,
    max_objects: usize,
    bytes: AtomicUsize,
    objects: AtomicUsize,
}

/// Shared allowance for buffers created through this allocator.
///
/// Admission is thread-safe and occurs before vector allocation. Failed
/// allocation releases its reservation. A view of one byte retains the charge
/// for its whole backing allocation, including row padding and unused capacity.
#[derive(Clone)]
pub struct StorageBudget(Arc<State>);
impl StorageBudget {
    /// Set independent byte and allocation-count limits. Zero is a real limit.
    pub fn new(max_bytes: usize, max_objects: usize) -> Self {
        Self(Arc::new(State {
            max_bytes,
            max_objects,
            bytes: AtomicUsize::new(0),
            objects: AtomicUsize::new(0),
        }))
    }
    /// Bytes currently reserved, including admitted allocations still being built.
    pub fn reserved_bytes(&self) -> usize {
        self.0.bytes.load(Ordering::Acquire)
    }
    /// Allocation slots currently reserved. Reading this and `reserved_bytes`
    /// separately is diagnostic, not an atomic two-counter snapshot.
    pub fn reserved_objects(&self) -> usize {
        self.0.objects.load(Ordering::Acquire)
    }
    /// The byte allowance.
    pub fn max_bytes(&self) -> usize {
        self.0.max_bytes
    }
    /// The allocation-count allowance.
    pub fn max_objects(&self) -> usize {
        self.0.max_objects
    }

    /// Allocate zeroed bytes after admission. Mutation cannot resize the buffer.
    pub fn bytes(&self, len: usize) -> Result<OwnedStorage<u8>, StorageError> {
        self.allocate(len)
    }
    /// Allocate zeroed 16-bit words, charging two bytes per word.
    pub fn words(&self, len: usize) -> Result<OwnedStorage<u16>, StorageError> {
        self.allocate(len)
    }
    /// Allocate zeroed 32-bit floating-point samples, charging four bytes each.
    pub fn floats(&self, len: usize) -> Result<OwnedStorage<f32>, StorageError> {
        self.allocate(len)
    }

    fn allocate<T: Copy + Default>(&self, len: usize) -> Result<OwnedStorage<T>, StorageError> {
        let bytes = len
            .checked_mul(size_of::<T>())
            .filter(|n| *n <= isize::MAX as usize)
            .ok_or(StorageError::SizeOverflow)?;
        let reservation = self.reserve(bytes)?;
        let mut data = Vec::new();
        data.try_reserve_exact(len)
            .map_err(StorageError::Allocation)?;
        let actual = data
            .capacity()
            .checked_mul(size_of::<T>())
            .ok_or(StorageError::SizeOverflow)?;
        if actual > bytes {
            return Err(StorageError::Capacity {
                admitted: bytes,
                actual,
            });
        }
        data.resize(len, T::default());
        Ok(OwnedStorage {
            allocation: Allocation { data, reservation },
        })
    }

    fn reserve(&self, bytes: usize) -> Result<Reservation, StorageError> {
        add_bounded(&self.0.objects, 1, self.0.max_objects).ok_or(StorageError::Objects {
            limit: self.0.max_objects,
        })?;
        if add_bounded(&self.0.bytes, bytes, self.0.max_bytes).is_none() {
            self.0.objects.fetch_sub(1, Ordering::Release);
            return Err(StorageError::Bytes {
                requested: bytes,
                limit: self.0.max_bytes,
            });
        }
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}
impl fmt::Debug for StorageBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageBudget")
            .field("max_bytes", &self.max_bytes())
            .field("max_objects", &self.max_objects())
            .finish_non_exhaustive()
    }
}

fn add_bounded(counter: &AtomicUsize, amount: usize, limit: usize) -> Option<()> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        let next = current.checked_add(amount).filter(|n| *n <= limit)?;
        match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Some(()),
            Err(actual) => current = actual,
        }
    }
}
struct Reservation {
    budget: StorageBudget,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.0.bytes.fetch_sub(self.bytes, Ordering::Release);
        self.budget.0.objects.fetch_sub(1, Ordering::Release);
    }
}
struct Allocation<T> {
    // Field order matters: free the payload before releasing its charge.
    data: Vec<T>,
    reservation: Reservation,
}

/// Uniquely owned, fixed-size admitted storage. Created by `StorageBudget`.
pub struct OwnedStorage<T> {
    allocation: Allocation<T>,
}
impl<T> OwnedStorage<T> {
    /// The initialized elements.
    pub fn as_slice(&self) -> &[T] {
        &self.allocation.data
    }
    /// Mutate elements without changing capacity or releasing their charge.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.allocation.data
    }
    /// Number of initialized elements, not bytes.
    pub fn len(&self) -> usize {
        self.allocation.data.len()
    }
    /// Whether the allocation has no elements; it still occupies an object slot.
    pub fn is_empty(&self) -> bool {
        self.allocation.data.is_empty()
    }
    /// Make storage shareable without copying its payload. The small `Arc`
    /// header allocation is outside the managed payload allowance.
    pub fn freeze(self) -> SharedStorage<T> {
        let len = self.len();
        SharedStorage {
            allocation: Arc::new(self.allocation),
            range: 0..len,
        }
    }
}
impl<T> fmt::Debug for OwnedStorage<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedStorage")
            .field("elements", &self.len())
            .finish_non_exhaustive()
    }
}

/// Immutable shared view retaining its entire backing allocation's charge.
/// Dropping a queue entry or old configuration does not release storage held by
/// another view. Borrowed slices cannot outlive the owner that keeps it charged.
pub struct SharedStorage<T> {
    allocation: Arc<Allocation<T>>,
    range: Range<usize>,
}
impl<T> Clone for SharedStorage<T> {
    fn clone(&self) -> Self {
        Self {
            allocation: self.allocation.clone(),
            range: self.range.clone(),
        }
    }
}
impl<T> SharedStorage<T> {
    /// The elements of this view.
    pub fn as_slice(&self) -> &[T] {
        &self.allocation.data[self.range.clone()]
    }
    /// Number of visible elements, not bytes.
    pub fn len(&self) -> usize {
        self.range.len()
    }
    /// Whether this view has no visible elements.
    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
    /// A subview in this view's coordinates. Empty views keep the backing charge.
    pub fn slice(&self, range: Range<usize>) -> Result<Self, StorageError> {
        if range.start > range.end || range.end > self.len() {
            return Err(StorageError::Range);
        }
        Ok(Self {
            allocation: self.allocation.clone(),
            range: self.range.start + range.start..self.range.start + range.end,
        })
    }
    /// The full backing allocation's admitted byte charge, not this view's size.
    pub fn charged_bytes(&self) -> usize {
        self.allocation.reservation.bytes
    }
}
impl<T> fmt::Debug for SharedStorage<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedStorage")
            .field("elements", &self.len())
            .field("charged_bytes", &self.charged_bytes())
            .finish_non_exhaustive()
    }
}
