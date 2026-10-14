//! Drive a resumable decoder from a random-access source or a slice.

use alloc::vec::Vec;
use core::fmt;

use enough::Stop;
use how_far::{Pulse, Total};
use zencodec::decode::{ByteRange, DecodeRowSink, OutputInfo, Step};

use crate::drivable::Drivable;
use zencodec::source::ReadAt;

use crate::feeder::Feeder;

/// Tuning for [`drive_from_read_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DriveOptions {
    /// Ranges closer than this are merged into one read (HTTP: 16–64 KiB).
    pub max_gap: u64,
    /// Bytes fetched per round for a sequential request (`length == 0`).
    pub sequential_chunk: usize,
}

impl Default for DriveOptions {
    fn default() -> Self {
        Self {
            max_gap: 0,
            sequential_chunk: 64 * 1024,
        }
    }
}

impl DriveOptions {
    /// Options with the given coalescing gap and sequential chunk size.
    pub const fn new(max_gap: u64, sequential_chunk: usize) -> Self {
        Self {
            max_gap,
            sequential_chunk,
        }
    }
}

/// Counters from one drive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DriveStats {
    /// Steps taken.
    pub steps: u64,
    /// Fetch rounds (one batched source read per round).
    pub rounds: u64,
    /// Source read requests issued (after coalescing).
    pub requests: u64,
    /// Bytes fetched from the source.
    pub bytes_fetched: u64,
    /// Rows the decoder reported writing.
    pub rows: u64,
}

/// Failure of a drive.
#[derive(Debug)]
#[non_exhaustive]
pub enum DriveError<CE, SE> {
    /// The decoder returned an error.
    Codec(CE),
    /// The source returned an error.
    Source(SE),
    /// The decoder asked for bytes but named no range.
    EmptyRequest,
    /// The decoder refused bytes it keeps asking for.
    Stalled,
}

impl<CE: fmt::Display, SE: fmt::Display> fmt::Display for DriveError<CE, SE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "codec error during drive: {e}"),
            Self::Source(e) => write!(f, "source error during drive: {e}"),
            Self::EmptyRequest => write!(f, "decoder requested bytes but named no range"),
            Self::Stalled => write!(f, "decoder refused bytes it keeps requesting"),
        }
    }
}

impl<CE: core::error::Error + 'static, SE: core::error::Error + 'static> core::error::Error
    for DriveError<CE, SE>
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            Self::Source(e) => Some(e),
            _ => None,
        }
    }
}

/// Merge overlapping or near-adjacent ranges (gap `<= max_gap`) into one.
/// Zero-length (sequential) ranges are passed through unchanged.
pub fn coalesce_byte_ranges(ranges: &[ByteRange], max_gap: u64) -> Vec<ByteRange> {
    let mut out = Vec::new();
    coalesce_into(ranges, max_gap, &mut out);
    out
}

/// [`coalesce_byte_ranges`] writing into `out` (cleared first), so a driver
/// loop reuses one allocation across rounds.
pub fn coalesce_into(ranges: &[ByteRange], max_gap: u64, out: &mut Vec<ByteRange>) {
    out.clear();
    out.extend(ranges.iter().copied().filter(|r| r.is_empty()));
    let sequential = out.len();
    out.extend(ranges.iter().copied().filter(|r| !r.is_empty()));
    let sized = &mut out[sequential..];
    if sized.is_empty() {
        return;
    }
    sized.sort_by_key(|r| (r.offset, r.length));
    let mut write = 0usize;
    for i in 1..sized.len() {
        let r = sized[i];
        let cur = sized[write];
        if r.offset <= cur.end().saturating_add(max_gap) {
            let end = cur.end().max(r.end());
            sized[write].length = end - cur.offset;
        } else {
            write += 1;
            sized[write] = r;
        }
    }
    out.truncate(sequential + write + 1);
}

/// Per-drive scratch so steady state allocates nothing per round.
#[derive(Default)]
struct Scratch {
    need: Vec<ByteRange>,
    resolved: Vec<ByteRange>,
    coalesced: Vec<ByteRange>,
    buffers: Vec<(u64, Vec<u8>)>,
    requests: Vec<(u64, u64)>,
}

/// Run `decoder` to completion over `source`, writing rows into `sink`.
///
/// Each [`Step::Need`] round is coalesced and fetched with one batched
/// [`read_many_at`](ReadAt::read_many_at), so a source that serves ranges
/// concurrently overlaps their round trips. Sequential requests fetch
/// `sequential_chunk` bytes from the cursor. The final chunk of the source
/// is fed with `eof`.
///
/// `pulse` is checked for cancellation at every step (a plain `Stop` is
/// passed as `how_far::StopOnly::borrowed(&stop)`), told the row total once
/// the header is known, and advanced by every strip written. After the
/// first round, steady state allocates nothing per round or per strip.
pub fn drive_from_read_at<D, S>(
    decoder: &mut D,
    source: &mut S,
    sink: &mut dyn DecodeRowSink,
    pulse: Option<&dyn Pulse>,
    opts: DriveOptions,
) -> Result<(OutputInfo, DriveStats), DriveError<D::Error, S::Error>>
where
    D: Drivable + ?Sized,
    S: ReadAt + ?Sized,
{
    let stop = pulse.map(|p| p as &dyn Stop);
    let mut stats = DriveStats::default();
    let mut feeder = Feeder::new();
    let mut scratch = Scratch::default();
    let mut total: Option<u64> = None;
    let mut told_total = false;
    let mut last_refused: Option<Vec<ByteRange>> = None;
    loop {
        stats.steps += 1;
        match decoder.step(sink, stop).map_err(DriveError::Codec)? {
            Step::Need(ranges) => {
                scratch.need.clear();
                scratch.need.extend_from_slice(ranges);
            }
            Step::NeedLen => {
                let len = *total.get_or_insert(source.len().map_err(DriveError::Source)?);
                decoder.feed(len, &[], true).map_err(DriveError::Codec)?;
                continue;
            }
            Step::Rows { rows, .. } => {
                stats.rows += rows as u64;
                if let Some(p) = pulse {
                    p.advance(rows as u64);
                }
                continue;
            }
            Step::Metadata(update) => {
                if update.is_header_ready() {
                    tell_total(pulse, decoder.info(), &mut told_total);
                }
                continue;
            }
            Step::Done(info) => return Ok((info, stats)),
            _ => continue,
        }
        if !told_total {
            tell_total(pulse, decoder.info(), &mut told_total);
        }
        if scratch.need.is_empty() {
            return Err(DriveError::EmptyRequest);
        }
        // Carried-over bytes first; if the decoder still refuses exactly
        // what it asks for, it is stalled.
        if feeder.has_carry() {
            if !feeder.flush(decoder).map_err(DriveError::Codec)? {
                if last_refused.as_deref() == Some(&scratch.need[..]) {
                    return Err(DriveError::Stalled);
                }
                last_refused = Some(scratch.need.clone());
                continue;
            }
            last_refused = None;
            continue;
        }
        // Resolve sequential requests against the source length. The
        // length is asked for only when needed: a buffered stream source
        // answers it by consuming to EOF.
        scratch.resolved.clear();
        let mut announce_eof = false;
        for i in 0..scratch.need.len() {
            let r = scratch.need[i];
            if r.is_empty() {
                let len = *total.get_or_insert(source.len().map_err(DriveError::Source)?);
                if r.offset >= len {
                    announce_eof = true;
                } else {
                    let n = (len - r.offset).min(opts.sequential_chunk as u64);
                    scratch.resolved.push(ByteRange::new(r.offset, n));
                }
            } else {
                scratch.resolved.push(r);
            }
        }
        if announce_eof {
            let len = total.expect("set when resolving a sequential request");
            decoder.feed(len, &[], true).map_err(DriveError::Codec)?;
        }
        if scratch.resolved.is_empty() {
            continue;
        }
        coalesce_into(&scratch.resolved, opts.max_gap, &mut scratch.coalesced);
        stats.rounds += 1;
        // Reuse fetch buffers across rounds.
        let n = scratch.coalesced.len();
        scratch
            .buffers
            .resize_with(n.max(scratch.buffers.len()), Default::default);
        for (i, r) in scratch.coalesced.iter().enumerate() {
            let (off, buf) = &mut scratch.buffers[i];
            *off = r.offset;
            buf.clear();
            buf.resize(r.length as usize, 0);
        }
        {
            let mut reqs: Vec<(u64, &mut [u8])> = scratch.buffers[..n]
                .iter_mut()
                .map(|(off, buf)| (*off, &mut buf[..]))
                .collect();
            scratch.requests.clear();
            source.read_many_at(&mut reqs).map_err(DriveError::Source)?;
        }
        for (offset, buf) in &scratch.buffers[..n] {
            stats.requests += 1;
            stats.bytes_fetched += buf.len() as u64;
            let eof = total.is_some_and(|len| offset + buf.len() as u64 >= len);
            feeder
                .offer(decoder, *offset, buf, eof)
                .map_err(DriveError::Codec)?;
        }
    }
}

fn tell_total(pulse: Option<&dyn Pulse>, info: Option<&zencodec::ImageInfo>, told: &mut bool) {
    if *told {
        return;
    }
    if let (Some(p), Some(info)) = (pulse, info) {
        // A pulse that cannot take a total records the issue itself.
        let _ = p.set_total(Total::Exact(info.height as u64));
        *told = true;
    }
}

/// Run `decoder` over an in-memory image.
///
/// The decoder is fed sub-slices of `bytes` directly: no fetch buffer, no
/// carry. A sequential request gets at most `chunk` bytes from its offset
/// (so chunk boundaries are exercised when `chunk` is small); a sized range
/// gets exactly its bytes. A partial accept is simply re-offered from the
/// slice on the next request. The final bytes are fed with `eof`.
pub fn drive_from_slice<D>(
    decoder: &mut D,
    bytes: &[u8],
    sink: &mut dyn DecodeRowSink,
    pulse: Option<&dyn Pulse>,
    chunk: usize,
) -> Result<(OutputInfo, DriveStats), DriveError<D::Error, zencodec::source::SourceError>>
where
    D: Drivable + ?Sized,
{
    let stop = pulse.map(|p| p as &dyn Stop);
    let chunk = chunk.max(1);
    let len = bytes.len() as u64;
    let mut stats = DriveStats::default();
    let mut told_total = false;
    let mut need: Vec<ByteRange> = Vec::new();
    let mut zero_rounds = 0u32;
    // Sequential position: the next byte to offer with `feed_and_step`,
    // set only once the decoder has asked for sequential bytes. Nothing is
    // offered before it asks, so a random-access decoder sees only the
    // ranges it requests.
    let mut pos: Option<u64> = None;
    loop {
        stats.steps += 1;
        let step = match pos {
            // Offer the next window and step in one call, so a decoder can
            // read it in place.
            Some(p) => {
                let start = p.min(len) as usize;
                let end = (p.saturating_add(chunk as u64)).min(len) as usize;
                let window = &bytes[start..end];
                let eof = end as u64 == len;
                let (accepted, step) = decoder
                    .feed_and_step(p, window, eof, sink, stop)
                    .map_err(DriveError::Codec)?;
                stats.requests += 1;
                stats.bytes_fetched += accepted as u64;
                pos = Some(p + accepted as u64);
                if accepted > 0 || window.is_empty() {
                    zero_rounds = 0;
                }
                step
            }
            None => decoder.step(sink, stop).map_err(DriveError::Codec)?,
        };
        match step {
            Step::Need(ranges) => {
                need.clear();
                need.extend_from_slice(ranges);
            }
            Step::NeedLen => {
                decoder.feed(len, &[], true).map_err(DriveError::Codec)?;
                continue;
            }
            Step::Rows { rows, .. } => {
                stats.rows += rows as u64;
                if let Some(p) = pulse {
                    p.advance(rows as u64);
                }
                continue;
            }
            Step::Metadata(update) => {
                if update.is_header_ready() {
                    tell_total(pulse, decoder.info(), &mut told_total);
                }
                continue;
            }
            Step::Done(info) => return Ok((info, stats)),
            _ => continue,
        }
        if !told_total {
            tell_total(pulse, decoder.info(), &mut told_total);
        }
        if need.is_empty() {
            return Err(DriveError::EmptyRequest);
        }
        stats.rounds += 1;
        // One sequential request: continue offering windows from there.
        if need.len() == 1 && need[0].is_empty() {
            let at = need[0].offset;
            if pos == Some(at) {
                zero_rounds += 1;
                if zero_rounds >= 3 {
                    return Err(DriveError::Stalled);
                }
            }
            pos = Some(at);
            continue;
        }
        // Sized ranges: feed exactly those bytes from the slice.
        pos = None;
        let mut accepted_any = false;
        for r in &need {
            if r.offset >= len {
                decoder.feed(len, &[], true).map_err(DriveError::Codec)?;
                accepted_any = true;
                continue;
            }
            let start = r.offset as usize;
            let end = if r.is_empty() {
                (r.offset.saturating_add(chunk as u64)).min(len)
            } else if r.end() > len {
                return Err(DriveError::Source(
                    zencodec::source::SourceError::UnexpectedEof {
                        offset: r.offset,
                        len: r.length,
                        total: Some(len),
                    },
                ));
            } else {
                r.end()
            } as usize;
            let piece = &bytes[start..end];
            let eof = end as u64 == len;
            stats.requests += 1;
            stats.bytes_fetched += piece.len() as u64;
            let accepted = decoder
                .feed(r.offset, piece, eof)
                .map_err(DriveError::Codec)?;
            accepted_any |= accepted > 0 || piece.is_empty();
        }
        if accepted_any {
            zero_rounds = 0;
        } else {
            zero_rounds += 1;
            if zero_rounds >= 2 {
                return Err(DriveError::Stalled);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesce_merges_within_gap_and_keeps_sequential() {
        let ranges = [
            ByteRange::new(500, 100),
            ByteRange::new(100, 50),
            ByteRange::from_offset(7),
            ByteRange::new(180, 20),
        ];
        let out = coalesce_byte_ranges(&ranges, 64);
        assert_eq!(
            out,
            [
                ByteRange::from_offset(7),
                ByteRange::new(100, 100),
                ByteRange::new(500, 100)
            ]
        );
        assert_eq!(coalesce_byte_ranges(&ranges, 0).len(), 4);
    }
}
