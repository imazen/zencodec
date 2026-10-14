//! Drive a resumable decoder from a random-access source or a slice.

use alloc::vec::Vec;
use core::fmt;

use enough::Stop;
use zencodec::decode::{ByteRange, DecodeRowSink, OutputInfo, Step};

use crate::drivable::Drivable;
use zencodec::source::{ReadAt, SliceReadAt};

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
    let mut sized: Vec<ByteRange> = ranges.iter().copied().filter(|r| !r.is_empty()).collect();
    let mut out: Vec<ByteRange> = ranges.iter().copied().filter(|r| r.is_empty()).collect();
    if sized.is_empty() {
        return out;
    }
    sized.sort_by_key(|r| (r.offset, r.length));
    let mut cur = sized[0];
    for r in sized.into_iter().skip(1) {
        if r.offset <= cur.end().saturating_add(max_gap) {
            let end = cur.end().max(r.end());
            cur.length = end - cur.offset;
        } else {
            out.push(cur);
            cur = r;
        }
    }
    out.push(cur);
    out
}

/// Run `decoder` to completion over `source`, writing rows into `sink`.
///
/// Each [`Step::Need`] round is coalesced and fetched with one batched
/// [`read_many_at`](ReadAt::read_many_at), so a source that serves ranges
/// concurrently overlaps their round trips. Sequential requests fetch
/// `sequential_chunk` bytes from the cursor. The final chunk of the source
/// is fed with `eof`.
pub fn drive_from_read_at<D, S>(
    decoder: &mut D,
    source: &mut S,
    sink: &mut dyn DecodeRowSink,
    stop: Option<&dyn Stop>,
    opts: DriveOptions,
) -> Result<(OutputInfo, DriveStats), DriveError<D::Error, S::Error>>
where
    D: Drivable + ?Sized,
    S: ReadAt + ?Sized,
{
    let mut stats = DriveStats::default();
    let mut feeder = Feeder::new();
    let mut total: Option<u64> = None;
    let mut last_refused: Option<Vec<ByteRange>> = None;
    loop {
        stats.steps += 1;
        let need: Vec<ByteRange> = match decoder.step(sink, stop).map_err(DriveError::Codec)? {
            Step::Need(ranges) => ranges.to_vec(),
            Step::NeedLen => {
                let len = *total.get_or_insert(source.len().map_err(DriveError::Source)?);
                decoder.feed(len, &[], true).map_err(DriveError::Codec)?;
                continue;
            }
            Step::Rows { rows, .. } => {
                stats.rows += rows as u64;
                continue;
            }
            Step::Metadata(_) => continue,
            Step::Done(info) => return Ok((info, stats)),
            _ => continue,
        };
        if need.is_empty() {
            return Err(DriveError::EmptyRequest);
        }
        // Carried-over bytes first; if the decoder still refuses exactly
        // what it asks for, it is stalled.
        if feeder.has_carry() {
            if !feeder.flush(decoder).map_err(DriveError::Codec)? {
                if last_refused.as_deref() == Some(&need[..]) {
                    return Err(DriveError::Stalled);
                }
                last_refused = Some(need);
                continue;
            }
            last_refused = None;
            continue;
        }
        // Resolve sequential requests against the source length. The
        // length is asked for only when needed: a buffered stream source
        // answers it by consuming to EOF.
        let mut resolved: Vec<ByteRange> = Vec::with_capacity(need.len());
        let mut announce_eof = false;
        for r in &need {
            if r.is_empty() {
                let len = *total.get_or_insert(source.len().map_err(DriveError::Source)?);
                if r.offset >= len {
                    announce_eof = true;
                } else {
                    let n = (len - r.offset).min(opts.sequential_chunk as u64);
                    resolved.push(ByteRange::new(r.offset, n));
                }
            } else {
                resolved.push(*r);
            }
        }
        if announce_eof {
            let len = total.expect("set when resolving a sequential request");
            decoder.feed(len, &[], true).map_err(DriveError::Codec)?;
        }
        if resolved.is_empty() {
            continue;
        }
        let coalesced = coalesce_byte_ranges(&resolved, opts.max_gap);
        stats.rounds += 1;
        let mut buffers: Vec<(u64, Vec<u8>)> = coalesced
            .iter()
            .map(|r| (r.offset, alloc::vec![0u8; r.length as usize]))
            .collect();
        {
            let mut reqs: Vec<(u64, &mut [u8])> = buffers
                .iter_mut()
                .map(|(off, buf)| (*off, &mut buf[..]))
                .collect();
            source.read_many_at(&mut reqs).map_err(DriveError::Source)?;
        }
        for (offset, buf) in &buffers {
            stats.requests += 1;
            stats.bytes_fetched += buf.len() as u64;
            let eof = total.is_some_and(|len| offset + buf.len() as u64 >= len);
            feeder
                .offer(decoder, *offset, buf, eof)
                .map_err(DriveError::Codec)?;
        }
    }
}

/// Run `decoder` over an in-memory image, offering it `chunk` bytes per
/// sequential request (so chunk boundaries are exercised) and serving
/// range requests directly from the slice.
pub fn drive_from_slice<D>(
    decoder: &mut D,
    bytes: &[u8],
    sink: &mut dyn DecodeRowSink,
    stop: Option<&dyn Stop>,
    chunk: usize,
) -> Result<(OutputInfo, DriveStats), DriveError<D::Error, zencodec::source::SourceError>>
where
    D: Drivable + ?Sized,
{
    let mut source = SliceReadAt::new(bytes);
    drive_from_read_at(
        decoder,
        &mut source,
        sink,
        stop,
        DriveOptions::new(0, chunk.max(1)),
    )
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
