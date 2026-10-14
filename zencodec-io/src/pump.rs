//! Sequential stream pump: `std::io::Read` into a resumable decoder.

use alloc::vec;
use core::fmt;

use enough::Stop;
use zencodec::decode::{ByteRange, DecodeRowSink, OutputInfo, Step};

use crate::drivable::Drivable;

use crate::feeder::Feeder;

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
/// Reads at most `buffer_size` bytes per round and feeds them at the
/// stream cursor; the decoder starts producing rows as soon as it has
/// enough. Only sequential requests (`Step::Need` at the cursor) can be
/// served; a request elsewhere is [`PumpError::NeedsRandomAccess`].
pub fn pump_read_to_sink<R, D>(
    mut reader: R,
    decoder: &mut D,
    sink: &mut dyn DecodeRowSink,
    stop: Option<&dyn Stop>,
    buffer_size: usize,
) -> Result<OutputInfo, PumpError<D::Error>>
where
    R: std::io::Read,
    D: Drivable + ?Sized,
{
    let mut buf = vec![0u8; buffer_size.max(512)];
    let mut cursor = 0u64;
    let mut eof = false;
    let mut feeder = Feeder::new();
    let mut refused_at: Option<u64> = None;
    loop {
        let need = match decoder.step(sink, stop).map_err(PumpError::Codec)? {
            Step::Need(ranges) => ranges
                .first()
                .copied()
                .unwrap_or(ByteRange::from_offset(cursor)),
            Step::NeedLen => return Err(PumpError::NeedsLength),
            Step::Done(info) => return Ok(info),
            _ => continue,
        };
        if feeder.has_carry() {
            if !feeder.flush(decoder).map_err(PumpError::Codec)? {
                if refused_at == Some(need.offset) {
                    // Full and still waiting: a sized request ahead of the
                    // cursor means the decoder cannot buffer the stream up
                    // to the bytes it wants.
                    return Err(if need.is_empty() {
                        PumpError::Stalled
                    } else {
                        PumpError::NeedsRandomAccess(need)
                    });
                }
                refused_at = Some(need.offset);
                continue;
            }
            refused_at = None;
            continue;
        }
        // Anything the decoder wants must lie at or beyond the cursor; a
        // sized request behind it cannot be served, and one ahead of it is
        // reached by reading forward (the decoder discards what it does
        // not want).
        if need.offset < cursor && !need.is_empty() {
            return Err(PumpError::NeedsRandomAccess(need));
        }
        if eof {
            decoder.feed(cursor, &[], true).map_err(PumpError::Codec)?;
            if need.offset >= cursor {
                // Nothing more will come; let the decoder report truncation.
                continue;
            }
            return Err(PumpError::NeedsRandomAccess(need));
        }
        let n = reader.read(&mut buf).map_err(PumpError::Io)?;
        if n == 0 {
            eof = true;
            decoder.feed(cursor, &[], true).map_err(PumpError::Codec)?;
            continue;
        }
        feeder
            .offer(decoder, cursor, &buf[..n], false)
            .map_err(PumpError::Codec)?;
        cursor += n as u64;
    }
}
