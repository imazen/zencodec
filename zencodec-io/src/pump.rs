//! Sequential stream pump: `std::io::Read` into a resumable decoder.

use alloc::vec;
use core::fmt;

use enough::Stop;
use how_far::{Pulse, Total};
use zencodec::decode::{ByteRange, DecodeRowSink, OutputInfo, Step};

use crate::drivable::Drivable;

/// Failure of [`pump_read_to_sink`].
#[derive(Debug)]
#[non_exhaustive]
pub enum PumpError<E> {
    /// Reading the stream failed.
    Io(std::io::Error),
    /// The decoder returned an error.
    Codec(E),
    /// The decoder asked for a byte range behind or ahead of the stream
    /// cursor, or for the total length. A stream cannot serve those; wrap it
    /// in [`BufferedReadAt`](crate::BufferedReadAt) and use
    /// [`drive_from_read_at`](crate::drive_from_read_at).
    NeedsRandomAccess(ByteRange),
    /// The decoder asked for the total length, which a stream cannot give
    /// without consuming to EOF.
    NeedsLength,
    /// The decoder refused bytes it keeps asking for.
    Stalled,
}

impl<E: fmt::Display> fmt::Display for PumpError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error during incremental decode: {e}"),
            Self::Codec(e) => write!(f, "codec error during incremental decode: {e}"),
            Self::NeedsRandomAccess(r) => write!(
                f,
                "decoder needs random access ({} bytes at {}); a sequential stream cannot serve it",
                r.length, r.offset
            ),
            Self::NeedsLength => {
                write!(f, "decoder needs the total length; a stream cannot give it")
            }
            Self::Stalled => write!(f, "decoder refused bytes it keeps requesting"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for PumpError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Codec(e) => Some(e),
            _ => None,
        }
    }
}

/// Pump a sequential stream into `decoder`, writing rows into `sink`.
///
/// Reads up to `buffer_size` bytes at a time and offers them with
/// [`feed_and_step`](crate::Drivable::feed_and_step), so a decoder can
/// decode straight from the read buffer; only the bytes it did not accept
/// stay in the buffer and are offered again with the next read. Rows are
/// emitted as soon as the decoder has enough input. Only sequential
/// requests can be served; a sized request behind the cursor is
/// [`PumpError::NeedsRandomAccess`].
pub fn pump_read_to_sink<R, D>(
    mut reader: R,
    decoder: &mut D,
    sink: &mut dyn DecodeRowSink,
    pulse: Option<&dyn Pulse>,
    buffer_size: usize,
) -> Result<OutputInfo, PumpError<D::Error>>
where
    R: std::io::Read,
    D: Drivable + ?Sized,
{
    let stop = pulse.map(|p| p as &dyn Stop);
    let mut told_total = false;
    // Start small and double up to `buffer_size` only while reads fill
    // the buffer, so a tiny stream does not pay for a large zeroed buffer.
    let max_buf = buffer_size.max(512);
    let mut buf = vec![0u8; max_buf.min(8 * 1024)];
    // `buf[lo..hi]` is read but not yet accepted; `buf[lo]` is at `base`.
    let (mut lo, mut hi) = (0usize, 0usize);
    let mut base = 0u64;
    let mut eof = false;
    let mut refused: Option<u64> = None;
    loop {
        let (accepted, step) = decoder
            .feed_and_step(base, &buf[lo..hi], eof, sink, stop)
            .map_err(PumpError::Codec)?;
        lo += accepted;
        base += accepted as u64;
        let need = match step {
            Step::Need(ranges) => ranges
                .first()
                .copied()
                .unwrap_or(ByteRange::from_offset(base)),
            Step::NeedLen => return Err(PumpError::NeedsLength),
            Step::Rows { rows, .. } => {
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
            Step::Done(info) => return Ok(info),
            _ => continue,
        };
        tell_total(pulse, decoder.info(), &mut told_total);
        if !need.is_empty() && need.offset < base {
            return Err(PumpError::NeedsRandomAccess(need));
        }
        if accepted == 0 && lo < hi {
            // The decoder refuses bytes while still asking: its input is
            // full. A sized request means it cannot buffer the stream up
            // to the bytes it wants.
            if refused == Some(need.offset) {
                return Err(if need.is_empty() {
                    PumpError::Stalled
                } else {
                    PumpError::NeedsRandomAccess(need)
                });
            }
            refused = Some(need.offset);
            continue;
        }
        refused = None;
        if eof {
            // Everything has been offered with `eof`; a decoder that still
            // asks has been told the length and should have failed.
            return Err(if need.is_empty() {
                PumpError::Stalled
            } else {
                PumpError::NeedsRandomAccess(need)
            });
        }
        // Keep the unaccepted remainder, then read more after it.
        if lo > 0 {
            buf.copy_within(lo..hi, 0);
            hi -= lo;
            lo = 0;
        }
        if hi == buf.len() {
            buf.resize(buf.len() * 2, 0);
        }
        let n = reader.read(&mut buf[hi..]).map_err(PumpError::Io)?;
        if n == 0 {
            eof = true;
        }
        hi += n;
        if hi == buf.len() && buf.len() < max_buf {
            buf.resize((buf.len() * 2).min(max_buf), 0);
        }
    }
}

fn tell_total(pulse: Option<&dyn Pulse>, info: Option<&zencodec::ImageInfo>, told: &mut bool) {
    if *told {
        return;
    }
    if let (Some(p), Some(info)) = (pulse, info) {
        let _ = p.set_total(Total::Exact(info.height as u64));
        *told = true;
    }
}
