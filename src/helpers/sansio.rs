use alloc::vec;
use core::error::Error as StdError;

use zenpixels::PixelDescriptor;

use crate::cost::OutputInfo;
use crate::decode::{BoxedError, DynIncrementalDecoder, IncrementalDecode};
use crate::sink::DecodeRowSink;

/// Error produced when pumping an I/O source into an incremental decoder and sink.
#[derive(Debug)]
pub enum PumpError<E> {
    /// An I/O error occurred while reading from the input source.
    Io(std::io::Error),
    /// A codec decoding error occurred.
    Codec(E),
    /// The decode operation was cancelled by a stop/pulse token.
    Cancelled,
    /// Unexpected EOF: stream ended before the image was completely decoded.
    UnexpectedEof,
}

impl<E: core::fmt::Display> core::fmt::Display for PumpError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error during incremental decode: {e}"),
            Self::Codec(e) => write!(f, "Codec error during incremental decode: {e}"),
            Self::Cancelled => write!(f, "Incremental decode cancelled"),
            Self::UnexpectedEof => write!(f, "Unexpected end-of-file during incremental decode"),
        }
    }
}

impl<E: StdError + 'static> StdError for PumpError<E> {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Codec(e) => Some(e),
            _ => None,
        }
    }
}

/// Pump bytes from a [`std::io::Read`] into an [`IncrementalDecode`] and flush batches into a [`DecodeRowSink`].
/// Pump bytes from a [`std::io::Read`] into an [`IncrementalDecode`] and flush batches into a [`DecodeRowSink`].
///
/// Honors backpressure (`PushStatus::BlockedOnOutput`) by draining output batches
/// and retrying unconsumed input suffixes before reading further bytes. Checks `pulse` for
/// cancellation and progress reporting.
pub fn pump_read_to_sink<R, D>(
    mut reader: R,
    decoder: &mut D,
    sink: &mut (dyn DecodeRowSink + '_),
    pulse: Option<&dyn how_far::Pulse>,
    buffer_size: usize,
) -> Result<OutputInfo, PumpError<D::Error>>
where
    R: std::io::Read,
    D: IncrementalDecode,
{
    let buffer_size = buffer_size.max(512);
    let mut buf = vec![0u8; buffer_size];
    let mut buf_pos = 0usize;
    let mut buf_len = 0usize;
    let mut is_reader_eof = false;

    let mut sink_begun = false;
    let mut chosen_desc: Option<PixelDescriptor> = None;
    let mut total_w = 0u32;
    let mut total_h = 0u32;

    // Helper to drain batches into the sink
    let mut drain_batches =
        |dec: &mut D, dst_sink: &mut (dyn DecodeRowSink + '_)| -> Result<(), PumpError<D::Error>> {
            if !sink_begun {
                if let (Some(info), Some(desc)) = (dec.info(), dec.output_descriptor()) {
                    total_w = info.width;
                    total_h = info.height;
                    chosen_desc = Some(desc);
                    dst_sink
                        .begin(info.width, info.height, desc)
                        .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
                    sink_begun = true;
                } else {
                    let info_h = dec.info().map(|i| i.height);
                    if let Some((meta, slice)) = dec.pull_batch_meta().map_err(PumpError::Codec)? {
                        let desc = slice.descriptor();
                        chosen_desc = Some(desc);
                        let w = slice.width();
                        let h = info_h.unwrap_or(slice.rows());
                        total_w = w;
                        total_h = h;
                        dst_sink
                            .begin(w, h, desc)
                            .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
                        sink_begun = true;

                        let batch_rows = slice.rows();
                        let mut dst = dst_sink
                            .provide_next_buffer(meta.y, batch_rows, w, desc)
                            .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
                        for row_idx in 0..batch_rows {
                            dst.row_mut(row_idx).copy_from_slice(slice.row(row_idx));
                        }
                    }
                }
            }

            // Pull any subsequent batches directly into the sink
            if sink_begun {
                while let Some(_rows) = dec.pull_batch_into(dst_sink).map_err(PumpError::Codec)? {
                    if let Some(p) = pulse
                        && p.check().is_err()
                    {
                        return Err(PumpError::Cancelled);
                    }
                }
            }
            Ok(())
        };

    loop {
        // 1. Check cancellation
        if let Some(p) = pulse
            && p.check().is_err()
        {
            return Err(PumpError::Cancelled);
        }

        // 2. Drain any available batches
        drain_batches(decoder, sink)?;

        // 3. If decoder has completed, finalize
        if decoder.is_complete() {
            if sink_begun {
                sink.finish()
                    .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
            }
            let desc = chosen_desc.unwrap_or(zenpixels::PixelDescriptor::RGB8_SRGB);
            let w = if total_w > 0 {
                total_w
            } else {
                decoder.info().map(|i| i.width).unwrap_or(0)
            };
            let h = if total_h > 0 {
                total_h
            } else {
                decoder.info().map(|i| i.height).unwrap_or(0)
            };
            return Ok(OutputInfo::full_decode(w, h, desc));
        }

        // 4. Fill buffer if empty
        if buf_pos >= buf_len {
            if is_reader_eof {
                let _outcome = decoder
                    .push_chunk_pulse(&[], true, pulse)
                    .map_err(PumpError::Codec)?;
                drain_batches(decoder, sink)?;
                if !decoder.is_complete() {
                    return Err(PumpError::UnexpectedEof);
                }
                if sink_begun {
                    sink.finish()
                        .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
                }
                let desc = chosen_desc.unwrap_or(zenpixels::PixelDescriptor::RGB8_SRGB);
                let w = if total_w > 0 {
                    total_w
                } else {
                    decoder.info().map(|i| i.width).unwrap_or(0)
                };
                let h = if total_h > 0 {
                    total_h
                } else {
                    decoder.info().map(|i| i.height).unwrap_or(0)
                };
                return Ok(OutputInfo::full_decode(w, h, desc));
            }

            let n = reader.read(&mut buf).map_err(PumpError::Io)?;
            if n == 0 {
                is_reader_eof = true;
            }
            buf_pos = 0;
            buf_len = n;
        }

        // 5. Push available chunk into decoder
        let chunk = &buf[buf_pos..buf_len];
        let outcome = decoder
            .push_chunk_pulse(chunk, is_reader_eof, pulse)
            .map_err(PumpError::Codec)?;

        buf_pos += outcome.bytes_consumed;

        // 6. Handle backpressure / drain output
        drain_batches(decoder, sink)?;

        // If no bytes were consumed and more input is needed, shift unconsumed remainder and read more
        if outcome.bytes_consumed == 0
            && outcome.status == crate::traits::PushStatus::NeedMoreInput
            && !is_reader_eof
        {
            let remaining = buf_len - buf_pos;
            buf.copy_within(buf_pos..buf_len, 0);
            buf_pos = 0;
            buf_len = remaining;
            let n = reader.read(&mut buf[remaining..]).map_err(PumpError::Io)?;
            if n == 0 {
                is_reader_eof = true;
            }
            buf_len += n;
        }
    }
}

/// Pump bytes from a [`std::io::Read`] into a [`DynIncrementalDecoder`] and flush batches into a [`DecodeRowSink`].
pub fn pump_dyn_read_to_sink<R>(
    mut reader: R,
    decoder: &mut (dyn DynIncrementalDecoder + '_),
    sink: &mut (dyn DecodeRowSink + '_),
    pulse: Option<&dyn how_far::Pulse>,
    buffer_size: usize,
) -> Result<OutputInfo, PumpError<BoxedError>>
where
    R: std::io::Read,
{
    let buffer_size = buffer_size.max(512);
    let mut buf = vec![0u8; buffer_size];
    let mut buf_pos = 0usize;
    let mut buf_len = 0usize;
    let mut is_reader_eof = false;

    let mut sink_begun = false;
    let mut chosen_desc: Option<PixelDescriptor> = None;
    let mut total_w = 0u32;
    let mut total_h = 0u32;

    let mut drain_batches = |dec: &mut (dyn DynIncrementalDecoder + '_),
                             dst_sink: &mut (dyn DecodeRowSink + '_)|
     -> Result<(), PumpError<BoxedError>> {
        if !sink_begun {
            if let (Some(info), Some(desc)) = (dec.info(), dec.output_descriptor()) {
                total_w = info.width;
                total_h = info.height;
                chosen_desc = Some(desc);
                dst_sink
                    .begin(info.width, info.height, desc)
                    .map_err(|e| PumpError::Codec(e as BoxedError))?;
                sink_begun = true;
            } else {
                let info_h = dec.info().map(|i| i.height);
                if let Some((meta, slice)) = dec.pull_batch_meta().map_err(PumpError::Codec)? {
                    let desc = slice.descriptor();
                    chosen_desc = Some(desc);
                    let w = slice.width();
                    let h = info_h.unwrap_or(slice.rows());
                    total_w = w;
                    total_h = h;
                    dst_sink
                        .begin(w, h, desc)
                        .map_err(|e| PumpError::Codec(e as BoxedError))?;
                    sink_begun = true;

                    let batch_rows = slice.rows();
                    let mut dst = dst_sink
                        .provide_next_buffer(meta.y, batch_rows, w, desc)
                        .map_err(|e| PumpError::Codec(e as BoxedError))?;
                    for row_idx in 0..batch_rows {
                        dst.row_mut(row_idx).copy_from_slice(slice.row(row_idx));
                    }
                }
            }
        }

        if sink_begun {
            while let Some(_rows) = dec.pull_batch_into(dst_sink).map_err(PumpError::Codec)? {
                if let Some(p) = pulse
                    && p.check().is_err()
                {
                    return Err(PumpError::Cancelled);
                }
            }
        }
        Ok(())
    };

    loop {
        if let Some(p) = pulse
            && p.check().is_err()
        {
            return Err(PumpError::Cancelled);
        }

        drain_batches(decoder, sink)?;

        if decoder.is_complete() {
            if sink_begun {
                sink.finish()
                    .map_err(|e| PumpError::Codec(e as BoxedError))?;
            }
            let desc = chosen_desc.unwrap_or(zenpixels::PixelDescriptor::RGB8_SRGB);
            let w = if total_w > 0 {
                total_w
            } else {
                decoder.info().map(|i| i.width).unwrap_or(0)
            };
            let h = if total_h > 0 {
                total_h
            } else {
                decoder.info().map(|i| i.height).unwrap_or(0)
            };
            return Ok(OutputInfo::full_decode(w, h, desc));
        }

        if buf_pos >= buf_len {
            if is_reader_eof {
                let _outcome = decoder
                    .push_chunk_pulse(&[], true, pulse)
                    .map_err(PumpError::Codec)?;
                drain_batches(decoder, sink)?;
                if !decoder.is_complete() {
                    return Err(PumpError::UnexpectedEof);
                }
                if sink_begun {
                    sink.finish()
                        .map_err(|e| PumpError::Codec(e as BoxedError))?;
                }
                let desc = chosen_desc.unwrap_or(zenpixels::PixelDescriptor::RGB8_SRGB);
                let w = if total_w > 0 {
                    total_w
                } else {
                    decoder.info().map(|i| i.width).unwrap_or(0)
                };
                let h = if total_h > 0 {
                    total_h
                } else {
                    decoder.info().map(|i| i.height).unwrap_or(0)
                };
                return Ok(OutputInfo::full_decode(w, h, desc));
            }

            let n = reader.read(&mut buf).map_err(PumpError::Io)?;
            if n == 0 {
                is_reader_eof = true;
            }
            buf_pos = 0;
            buf_len = n;
        }

        let chunk = &buf[buf_pos..buf_len];
        let outcome = decoder
            .push_chunk_pulse(chunk, is_reader_eof, pulse)
            .map_err(PumpError::Codec)?;

        buf_pos += outcome.bytes_consumed;

        drain_batches(decoder, sink)?;

        if outcome.bytes_consumed == 0
            && outcome.status == crate::traits::PushStatus::NeedMoreInput
            && !is_reader_eof
        {
            let remaining = buf_len - buf_pos;
            buf.copy_within(buf_pos..buf_len, 0);
            buf_pos = 0;
            buf_len = remaining;
            let n = reader.read(&mut buf[remaining..]).map_err(PumpError::Io)?;
            if n == 0 {
                is_reader_eof = true;
            }
            buf_len += n;
        }
    }
}

/// Coalesce adjacent, overlapping, or close byte ranges into contiguous requests.
///
/// Reduces HTTP Range request round-trips by merging ranges separated by gaps
/// less than or equal to `max_gap` bytes.
///
/// Empty ranges (`length == 0`) are ignored.
///
/// # Example
///
/// ```
/// use zencodec::decode::{ByteRange, coalesce_byte_ranges};
///
/// let ranges = [
///     ByteRange::new(100, 50),  // 100..150
///     ByteRange::new(180, 20),  // 180..200 (gap of 30 bytes)
///     ByteRange::new(500, 100), // 500..600 (gap of 300 bytes)
/// ];
///
/// // With max_gap = 64, the first two merge:
/// let coalesced = coalesce_byte_ranges(&ranges, 64);
/// assert_eq!(coalesced.len(), 2);
/// assert_eq!(coalesced[0], ByteRange::new(100, 100)); // 100..200
/// assert_eq!(coalesced[1], ByteRange::new(500, 100)); // 500..600
/// ```
pub fn coalesce_byte_ranges(
    ranges: &[crate::traits::ByteRange],
    max_gap: u64,
) -> alloc::vec::Vec<crate::traits::ByteRange> {
    let mut valid: alloc::vec::Vec<crate::traits::ByteRange> =
        ranges.iter().copied().filter(|r| !r.is_empty()).collect();

    if valid.is_empty() {
        return alloc::vec::Vec::new();
    }

    valid.sort_by_key(|r| (r.offset, r.length));

    let mut coalesced = alloc::vec::Vec::with_capacity(valid.len());
    let mut cur = valid[0];

    for r in valid.into_iter().skip(1) {
        let cur_end = cur.end();
        if r.offset <= cur_end.saturating_add(max_gap) {
            let new_end = cur_end.max(r.end());
            cur.length = new_end.saturating_sub(cur.offset);
        } else {
            coalesced.push(cur);
            cur = r;
        }
    }
    coalesced.push(cur);
    coalesced
}
