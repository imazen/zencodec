//! Drive a resumable decoder from a slice or a random-access source.

use alloc::vec::Vec;

use enough::{Stop, Unstoppable};
use how_far::{Pulse, Total};
use zencodec::decode::{
    ByteRange, DecodeRowSink, Event, IncrementalDecode, Input, OutputInfo, StepError,
};
use zencodec::source::ReadAt;

use crate::stepper::{DriveFailure, ReadAtCursor};
use crate::window::check_need;

/// Tuning for the drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DriveOptions {
    /// Requested ranges closer than this are merged into one read (HTTP:
    /// 16–64 KiB).
    pub max_gap: u64,
    /// Bytes read per round for a sequential request, on top of the
    /// decoder's minimum.
    pub read_size: usize,
}

impl Default for DriveOptions {
    fn default() -> Self {
        Self {
            max_gap: 0,
            read_size: 64 * 1024,
        }
    }
}

impl DriveOptions {
    /// Options with the given coalescing gap and sequential read size.
    pub const fn new(max_gap: u64, read_size: usize) -> Self {
        Self { max_gap, read_size }
    }
}

/// Counters from one drive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DriveStats {
    /// Steps taken.
    pub steps: u64,
    /// Source read calls issued.
    pub reads: u64,
    /// Bytes read from the source.
    pub bytes_read: u64,
    /// Rows the decoder reported writing.
    pub rows: u64,
}

/// Failure of an image drive: a [`DriveFailure`] whose stage error is the
/// decoder's [`StepError`].
pub type DriveError<CE, SE> = DriveFailure<StepError<CE>, SE>;

/// Merge overlapping or near-adjacent ranges (gap `<= max_gap`) into one.
pub fn coalesce_byte_ranges(ranges: &[ByteRange], max_gap: u64) -> Vec<ByteRange> {
    let mut out = Vec::new();
    coalesce_into(ranges, max_gap, &mut out);
    out
}

/// [`coalesce_byte_ranges`] writing into `out` (cleared first), so a loop
/// reuses one allocation.
pub fn coalesce_into(ranges: &[ByteRange], max_gap: u64, out: &mut Vec<ByteRange>) {
    out.clear();
    out.extend(ranges.iter().copied().filter(|r| !r.is_empty()));
    if out.is_empty() {
        return;
    }
    out.sort_by_key(|r| (r.offset, r.length));
    let mut write = 0usize;
    for i in 1..out.len() {
        let r = out[i];
        let cur = out[write];
        if r.offset <= cur.end().saturating_add(max_gap) {
            out[write].length = cur.end().max(r.end()) - cur.offset;
        } else {
            write += 1;
            out[write] = r;
        }
    }
    out.truncate(write + 1);
}

/// Report the row total once the header is known, and rows as they land.
pub(crate) struct Progressor<'a> {
    pulse: Option<&'a dyn Pulse>,
    told_total: bool,
}

impl<'a> Progressor<'a> {
    pub(crate) fn new(pulse: Option<&'a dyn Pulse>) -> Self {
        Self {
            pulse,
            told_total: false,
        }
    }

    pub(crate) fn stop(&self) -> &'a dyn Stop {
        match self.pulse {
            Some(p) => p,
            None => &Unstoppable,
        }
    }

    #[inline]
    pub(crate) fn observe<D: IncrementalDecode + ?Sized>(&mut self, dec: &D, event: &Event) {
        let Some(p) = self.pulse else { return };
        if !self.told_total
            && let Some(info) = dec.info()
        {
            // A pulse that cannot take a total records the issue itself.
            let _ = p.set_total(Total::Exact(info.height as u64));
            self.told_total = true;
        }
        if let Event::Rows { rows, .. } = event {
            p.advance(*rows as u64);
        }
    }
}

/// Run `decoder` over an in-memory image without copying it.
///
/// Every input is a sub-slice of `bytes` carrying the total length. A
/// sequential request gets at least its minimum and up to `window` bytes
/// (a small `window` exercises boundaries); a range gets exactly its span.
///
/// `pulse` is checked for cancellation each step (a plain `Stop` passes as
/// `how_far::StopOnly::borrowed(&stop)`), told the row total once the
/// header is known, and advanced by every strip written.
pub fn drive_from_slice<D>(
    decoder: &mut D,
    bytes: &[u8],
    sink: &mut dyn DecodeRowSink,
    pulse: Option<&dyn Pulse>,
    window: usize,
) -> Result<(OutputInfo, DriveStats), DriveError<D::Error, core::convert::Infallible>>
where
    D: IncrementalDecode + ?Sized,
{
    let mut progress = Progressor::new(pulse);
    let stop = progress.stop();
    let total = bytes.len() as u64;
    let window = window.max(1);
    let mut stats = DriveStats::default();
    // The current input: `bytes[start..end]`. `start <= end <= len` holds:
    // consumption never exceeds the offer and `check_need` keeps every served
    // request inside `total`.
    let (mut start, mut end) = (0usize, 0usize);
    loop {
        let input = Input::new(start as u64, &bytes[start..end]).with_total_len(total);
        let p = decoder
            .step(input, sink, stop)
            .map_err(DriveFailure::Step)?;
        stats.steps += 1;
        start += p.consumed.min(end - start);
        progress.observe(decoder, &p.event);
        match p.event {
            Event::Done(info) => return Ok((info, stats)),
            Event::Need => {}
            Event::Rows { rows, .. } => {
                stats.rows += rows as u64;
                continue;
            }
            _ => continue,
        }
        let requests = decoder.requested();
        check_need(requests, &input, p.consumed).map_err(DriveFailure::Protocol)?;
        (start, end) = crate::stepper::slice_span(start, end, window, bytes.len(), requests);
        stats.reads += 1;
        stats.bytes_read += (end - start) as u64;
    }
}

/// Run `decoder` over a random-access source.
///
/// Sequential requests read forward into a driver-owned window (short
/// reads mark the end of the source; [`ReadAt::len`] is called only when the
/// decoder asks for the length). Range requests are coalesced (`max_gap`)
/// and fetched with one [`read_many_at`](ReadAt::read_many_at) per round, so
/// a concurrent source overlaps their round trips; later ranges of the same
/// round are served from the fetched buffers. A source that is already in
/// memory ([`ReadAt::as_slice`]) is driven without copying. The same driver
/// serves any [`ByteStepper`](crate::ByteStepper) through
/// [`ReadAtCursor`].
pub fn drive_from_read_at<D, S>(
    decoder: &mut D,
    source: &mut S,
    sink: &mut dyn DecodeRowSink,
    pulse: Option<&dyn Pulse>,
    opts: DriveOptions,
) -> Result<(OutputInfo, DriveStats), DriveError<D::Error, S::Error>>
where
    D: IncrementalDecode + ?Sized,
    S: ReadAt + ?Sized,
{
    // A source already in memory is driven without copying, by the slice
    // driver.
    if let Some(bytes) = source.as_slice() {
        return drive_from_slice(decoder, bytes, sink, pulse, opts.read_size)
            .map_err(DriveFailure::widen);
    }
    let mut progress = Progressor::new(pulse);
    let stop = progress.stop();
    let mut cursor = ReadAtCursor::new(opts);
    let mut rows = 0u64;
    loop {
        let input = cursor.offer(source);
        let p = decoder
            .step(input, sink, stop)
            .map_err(DriveFailure::Step)?;
        if p.event == Event::Need {
            check_need(decoder.requested(), &input, p.consumed).map_err(DriveFailure::Protocol)?;
        }
        cursor.take(p.consumed);
        progress.observe(decoder, &p.event);
        match p.event {
            Event::Done(info) => {
                let mut stats = cursor.stats();
                stats.rows = rows;
                return Ok((info, stats));
            }
            Event::Need => cursor.serve(decoder.requested(), source)?,
            Event::Rows { rows: n, .. } => rows += n as u64,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesce_merges_within_gap() {
        let ranges = [
            ByteRange::new(500, 100),
            ByteRange::new(100, 50),
            ByteRange::new(180, 20),
        ];
        assert_eq!(
            coalesce_byte_ranges(&ranges, 64),
            [ByteRange::new(100, 100), ByteRange::new(500, 100)]
        );
        assert_eq!(coalesce_byte_ranges(&ranges, 0).len(), 3);
    }
}
