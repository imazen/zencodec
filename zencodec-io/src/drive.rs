//! Drive a resumable decoder from a slice or a random-access source.

use alloc::vec::Vec;
use core::fmt;

use enough::{Stop, Unstoppable};
use how_far::{Pulse, Total};
use zencodec::decode::{
    ByteRange, DecodeRowSink, Event, IncrementalDecode, Input, OutputInfo, Request, StepError,
};
use zencodec::source::ReadAt;

use crate::window::{ProtocolViolation, Window, check_need};

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

/// Failure of a drive.
#[derive(Debug)]
#[non_exhaustive]
pub enum DriveError<CE, SE> {
    /// The decoder failed, the sink failed, or the stop tripped.
    Step(StepError<CE>),
    /// The source failed.
    Source(SE),
    /// The decoder broke the protocol.
    Protocol(ProtocolViolation),
}

impl<CE: fmt::Display, SE: fmt::Display> fmt::Display for DriveError<CE, SE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Step(e) => write!(f, "{e}"),
            Self::Source(e) => write!(f, "source: {e}"),
            Self::Protocol(e) => write!(f, "protocol: {e}"),
        }
    }
}

impl<CE: core::error::Error + 'static, SE: core::error::Error + 'static> core::error::Error
    for DriveError<CE, SE>
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Step(e) => Some(e),
            Self::Source(e) => Some(e),
            Self::Protocol(e) => Some(e),
        }
    }
}

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
    // The current input: `bytes[start..end]`.
    let (mut start, mut end) = (0usize, 0usize);
    loop {
        let input = Input::new(start as u64, &bytes[start..end]).with_total_len(total);
        let p = decoder.step(input, sink, stop).map_err(DriveError::Step)?;
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
        check_need(requests, &input, p.consumed).map_err(DriveError::Protocol)?;
        let request = requests[0];
        let (s, e) = match request {
            Request::AtLeast { offset, min_len } => {
                let s = offset as usize;
                (s, s.saturating_add(min_len.max(window)).min(bytes.len()))
            }
            Request::Range(r) => (r.offset as usize, r.end() as usize),
            // Every input carries the total; `check_need` reports a decoder
            // that keeps asking.
            _ => (start, end),
        };
        start = s;
        end = e;
        stats.reads += 1;
        stats.bytes_read += (e - s) as u64;
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
/// memory ([`ReadAt::as_slice`]) is driven without copying.
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
    if let Some(bytes) = source.as_slice() {
        return drive_from_slice(decoder, bytes, sink, pulse, opts.read_size).map_err(
            |e| match e {
                DriveError::Step(s) => DriveError::Step(s),
                DriveError::Protocol(p) => DriveError::Protocol(p),
                DriveError::Source(never) => match never {},
            },
        );
    }
    let mut progress = Progressor::new(pulse);
    let stop = progress.stop();
    let mut stats = DriveStats::default();
    let mut window = Window::new();
    window.total = source.len_hint();
    // Buffers fetched by the last range round, by absolute offset.
    let mut fetched: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut ranges: Vec<ByteRange> = Vec::new();
    let mut coalesced: Vec<ByteRange> = Vec::new();
    // Which input to offer next: the window, or a fetched buffer from `at`.
    let mut from_fetched: Option<(usize, usize)> = None;
    loop {
        let input = match from_fetched {
            Some((i, at)) => {
                let (off, buf) = &fetched[i];
                Input::new(off + at as u64, &buf[at..]).with_total_len_opt(window.total)
            }
            None => window.input(),
        };
        let p = decoder.step(input, sink, stop).map_err(DriveError::Step)?;
        stats.steps += 1;
        progress.observe(decoder, &p.event);
        if p.event == Event::Need {
            check_need(decoder.requested(), &input, p.consumed).map_err(DriveError::Protocol)?;
        }
        match &mut from_fetched {
            Some((_, at)) => *at += p.consumed,
            None => window.consume(p.consumed),
        }
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
        match requests[0] {
            Request::AtLeast { offset, min_len } => {
                from_fetched = None;
                if offset < window.base() || offset > window.end() {
                    window.reset_to(offset);
                } else {
                    window.consume((offset - window.base()) as usize);
                }
                // Read until the request is met or the source ends, each read
                // `read_size` bytes or the room the window has left (a short
                // read marks the end of the source).
                while window.len() < min_len.max(1) && window.total != Some(window.end()) {
                    let end = window.end();
                    let need = min_len.max(1) - window.len();
                    let spare = window.spare_up_to(need, opts.read_size.max(need));
                    let n_want = spare.len();
                    let n = source.read_at(end, spare).map_err(DriveError::Source)?;
                    window.commit(n);
                    stats.reads += 1;
                    stats.bytes_read += n as u64;
                    if n < n_want {
                        window.total = Some(end + n as u64);
                    }
                }
            }
            Request::Range(_) => {
                // Serve from the last round's buffers if one holds it.
                let first = requests[0];
                let hit = fetched.iter().position(|(off, buf)| {
                    let r = match first {
                        Request::Range(r) => r,
                        _ => unreachable!(),
                    };
                    r.offset >= *off && r.end() <= off + buf.len() as u64
                });
                if let Some(i) = hit {
                    let off = fetched[i].0;
                    let r = match first {
                        Request::Range(r) => r,
                        _ => unreachable!(),
                    };
                    from_fetched = Some((i, (r.offset - off) as usize));
                    continue;
                }
                // New round: every range request, coalesced, one batch.
                ranges.clear();
                ranges.extend(requests.iter().filter_map(|r| match r {
                    Request::Range(r) => Some(*r),
                    _ => None,
                }));
                coalesce_into(&ranges, opts.max_gap, &mut coalesced);
                fetched.resize_with(coalesced.len(), Default::default);
                for (slot, r) in fetched.iter_mut().zip(&coalesced) {
                    slot.0 = r.offset;
                    slot.1.clear();
                    slot.1.resize(r.length as usize, 0);
                }
                {
                    let mut reqs: Vec<(u64, &mut [u8])> = fetched
                        .iter_mut()
                        .map(|(off, buf)| (*off, &mut buf[..]))
                        .collect();
                    source.read_many_at(&mut reqs).map_err(DriveError::Source)?;
                }
                stats.reads += 1;
                stats.bytes_read += coalesced.iter().map(|r| r.length).sum::<u64>();
                let r = match first {
                    Request::Range(r) => r,
                    _ => unreachable!(),
                };
                let i = fetched
                    .iter()
                    .position(|(off, buf)| r.offset >= *off && r.end() <= off + buf.len() as u64)
                    .expect("coalesced ranges cover every request");
                from_fetched = Some((i, (r.offset - fetched[i].0) as usize));
            }
            Request::Len => {
                from_fetched = None;
                if window.total.is_none() {
                    window.total = Some(source.len().map_err(DriveError::Source)?);
                }
            }
            _ => {
                return Err(DriveError::Protocol(ProtocolViolation::EmptyRequest));
            }
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
