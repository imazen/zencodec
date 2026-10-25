//! Admission shared by synchronous and asynchronous input drivers.

use alloc::collections::TryReserveError;
use alloc::vec::Vec;
use core::fmt;
use zencodec::decode::{ByteRange, Request};

/// Limits on storage and requests owned by one input driver.
///
/// This bounds retained payload capacities, stored range descriptors and the
/// number of ranges handed to a source at once. It excludes caller-owned
/// slices, allocator bookkeeping/transient reallocation, source caches, codec
/// allocations and output. It is not a process-memory limit.
///
/// Zero is valid: no owned payload, range descriptors or reads, respectively.
/// Borrowed resident input remains usable with all three limits set to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoLimits {
    input_bytes: usize,
    ranges: usize,
    in_flight: usize,
}

impl IoLimits {
    /// Construct finite limits. Requests are checked before allocation or I/O.
    pub const fn new(input_bytes: usize, ranges: usize, in_flight: usize) -> Self {
        Self {
            input_bytes,
            ranges,
            in_flight,
        }
    }

    /// Preserve the historical unbounded driver configuration.
    pub const fn unlimited() -> Self {
        Self::new(usize::MAX, usize::MAX, usize::MAX)
    }

    /// Maximum retained driver-owned payload capacity.
    pub const fn max_input_bytes(self) -> usize {
        self.input_bytes
    }
    /// Maximum range descriptors in a fetch plan (excluding codec-owned requests).
    pub const fn max_ranges(self) -> usize {
        self.ranges
    }
    /// Maximum ranges passed to a source in one batch.
    pub const fn max_in_flight(self) -> usize {
        self.in_flight
    }

    pub(crate) fn bytes(self, needed: u64) -> Result<usize, InputError> {
        if needed > self.input_bytes as u64 {
            return Err(InputError::Limit {
                needed,
                limit: self.input_bytes,
            });
        }
        usize::try_from(needed).map_err(|_| InputError::Limit {
            needed,
            limit: self.input_bytes,
        })
    }

    pub(crate) fn reading(self) -> Result<(), InputError> {
        if self.in_flight == 0 {
            Err(InputError::ReadsDisabled)
        } else {
            Ok(())
        }
    }
}

impl Default for IoLimits {
    fn default() -> Self {
        Self::unlimited()
    }
}

/// Driver admission or allocation failure, separate from codec/source errors.
#[derive(Debug)]
#[non_exhaustive]
pub enum InputError {
    /// A contiguous request or retained capacity exceeds the input allowance.
    Limit {
        /// Required payload capacity.
        needed: u64,
        /// Configured maximum capacity.
        limit: usize,
    },
    /// The configuration allows no stored range descriptors.
    RangesDisabled,
    /// The configuration allows no source reads.
    ReadsDisabled,
    /// Fallible storage reservation failed. The original allocator error remains available.
    Allocation {
        /// Requested allocation size (saturating for descriptor overflow).
        bytes: usize,
        /// Original allocation failure.
        source: TryReserveError,
    },
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit { needed, limit } => {
                write!(f, "input needs {needed} bytes; limit is {limit}")
            }
            Self::RangesDisabled => f.write_str("input range descriptors are disabled"),
            Self::ReadsDisabled => f.write_str("input reads are disabled"),
            Self::Allocation { bytes, .. } => {
                write!(f, "could not reserve {bytes} input-storage bytes")
            }
        }
    }
}

impl core::error::Error for InputError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Allocation { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) fn reserve<T>(v: &mut Vec<T>, len: usize) -> Result<(), InputError> {
    v.try_reserve_exact(len.saturating_sub(v.len()))
        .map_err(|source| InputError::Allocation {
            bytes: len.saturating_mul(core::mem::size_of::<T>()),
            source,
        })
}

/// Choose an affordable alternative, preserving the codec's order of preference.
/// The borrowed request list need not be copied to make this choice.
pub(crate) fn select(requests: &[Request], limits: IoLimits) -> Result<Request, InputError> {
    let mut first_error = None;
    for &request in requests {
        let result = match request {
            Request::AtLeast { min_len, .. } => limits.bytes(min_len.max(1) as u64).map(|_| ()),
            Request::Range(r) => limits.bytes(r.length).and({
                if limits.ranges == 0 {
                    Err(InputError::RangesDisabled)
                } else {
                    Ok(())
                }
            }),
            _ => Ok(()),
        };
        match result {
            Ok(()) => return Ok(request),
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    Err(first_error.unwrap_or(InputError::RangesDisabled))
}

/// A bounded prefetch plan for alternatives, not an all-dependency plan.
/// Always includes `first`. Optional ranges and gap bytes may be omitted to
/// fit the budget. Overlaps are merged without charging their bytes twice.
pub(crate) fn plan(
    first: ByteRange,
    requests: &[Request],
    gap: u64,
    limits: IoLimits,
    out: &mut Vec<ByteRange>,
) -> Result<(), InputError> {
    limits.reading()?;
    limits.bytes(first.length)?;
    if limits.ranges == 0 {
        return Err(InputError::RangesDisabled);
    }
    out.clear();
    reserve(out, requests.len().min(limits.ranges))?;
    out.push(first);
    let mut bytes = first.length;
    for request in requests {
        let Request::Range(r) = *request else {
            continue;
        };
        if r.is_empty() || r == first || out.len() == limits.ranges {
            continue;
        }
        // Conservative admission before sorting. Overlaps may be fetched in a
        // later round if counting them separately would exceed this allowance.
        if let Some(sum) = bytes.checked_add(r.length)
            && sum <= limits.input_bytes as u64
        {
            out.push(r);
            bytes = sum;
        }
    }
    out.sort_unstable_by_key(|r| (r.offset, r.length));
    let mut write = 0;
    for i in 1..out.len() {
        let r = out[i];
        let current = out[write];
        let extra = r.offset.saturating_sub(current.end());
        if r.offset <= current.end().saturating_add(gap)
            && extra <= (limits.input_bytes as u64).saturating_sub(bytes)
        {
            let merged = current.end().max(r.end()) - current.offset;
            bytes = bytes - current.length - r.length + merged;
            out[write].length = merged;
        } else {
            write += 1;
            out[write] = r;
        }
    }
    out.truncate(write + 1);
    Ok(())
}

pub(crate) fn capacity(fetched: &[(u64, Vec<u8>)]) -> usize {
    fetched
        .iter()
        .fold(0usize, |sum, (_, b)| sum.saturating_add(b.capacity()))
}

/// Async image drivers cannot borrow a non-Sync decoder across an await. Copy
/// only the selected request and a bounded number of affordable alternatives.
#[cfg(feature = "tokio")]
pub(crate) fn copy_requests(
    requests: &[Request],
    limits: IoLimits,
    out: &mut Vec<Request>,
) -> Result<(), InputError> {
    let first = select(requests, limits)?;
    out.clear();
    let count = if matches!(first, Request::Range(_)) {
        requests.len().min(limits.ranges)
    } else {
        1
    };
    reserve(out, count)?;
    out.push(first);
    if matches!(first, Request::Range(_)) {
        for &r in requests {
            if out.len() == count {
                break;
            }
            if r != first && matches!(r, Request::Range(b) if b.length <= limits.input_bytes as u64)
            {
                out.push(r);
            }
        }
    }
    Ok(())
}

pub(crate) fn make_room(
    window: &crate::window::Window,
    fetched: &mut Vec<(u64, Vec<u8>)>,
    target: usize,
    limits: IoLimits,
) {
    if capacity(fetched).saturating_add(window.capacity().max(target)) > limits.input_bytes {
        fetched.clear();
    }
}

pub(crate) fn allocate_ranges(
    ranges: &[ByteRange],
    limits: IoLimits,
    fetched: &mut Vec<(u64, Vec<u8>)>,
) -> Result<(), InputError> {
    // Reuse allocations when their capacities fit. Discard them before growth
    // otherwise; old and new rounds must not silently double retained payload.
    fetched.truncate(ranges.len());
    let reusable = ranges.iter().enumerate().fold(0usize, |sum, (i, r)| {
        sum.saturating_add(
            fetched
                .get(i)
                .map_or(0, |(_, b)| b.capacity())
                .max(r.length as usize),
        )
    });
    if reusable > limits.input_bytes {
        fetched.clear();
    }
    reserve(fetched, ranges.len())?;
    fetched.resize_with(ranges.len(), Default::default);
    let mut used = 0usize;
    for (range, (offset, buf)) in ranges.iter().zip(fetched.iter_mut()) {
        let len = limits.bytes(range.length)?;
        *offset = range.offset;
        reserve(buf, len)?;
        used = used.saturating_add(buf.capacity());
        limits.bytes(used as u64)?;
        buf.resize(len, 0);
    }
    Ok(())
}

#[cfg(all(test, feature = "tokio"))]
mod tests {
    use super::*;

    #[test]
    fn async_request_copy_does_not_copy_an_unbounded_alternative_list() {
        let requests: Vec<_> = (0..10000)
            .map(|offset| Request::range(offset * 8, 8))
            .collect();
        let mut copied = Vec::new();
        copy_requests(&requests, IoLimits::new(16, 2, 1), &mut copied).unwrap();
        assert_eq!(copied, requests[..2]);
        assert!(copied.capacity() <= 2);
        copy_requests(
            &[Request::range(0, 17), Request::at_least(32, 8)],
            IoLimits::new(16, 0, 1),
            &mut copied,
        )
        .unwrap();
        assert_eq!(copied, [Request::at_least(32, 8)]);
    }
}
