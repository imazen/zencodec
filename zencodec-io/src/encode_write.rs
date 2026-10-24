//! Write a streaming encoder's output to a `std::io::Write` as it is
//! produced.

use core::fmt;
use std::io::{self, Write};

use zencodec::UnsupportedOperation;
use zencodec::encode::{AnimationFrameEncoder, Encoder, OutputTiming};
use zenpixels::PixelSlice;

/// Write every byte `enc` has pending to `w`, then consume it. Returns how
/// many bytes were written.
///
/// The drain step of a streaming encode, without the footgun of forgetting
/// [`consume_output`](Encoder::consume_output) and sending the same bytes
/// twice. On an I/O error nothing is consumed, and the bytes `w` accepted
/// before the error are unknown: abort the transport.
pub fn write_pending<E: Encoder>(enc: &mut E, w: &mut impl Write) -> io::Result<usize> {
    let n = enc.pending_output().len();
    if n > 0 {
        w.write_all(enc.pending_output())?;
        enc.consume_output(n);
    }
    Ok(n)
}

/// [`write_pending`] for an [`AnimationFrameEncoder`]: write the bytes of the
/// frames pushed so far.
pub fn write_pending_animation<A: AnimationFrameEncoder>(
    enc: &mut A,
    w: &mut impl Write,
) -> io::Result<usize> {
    let n = enc.pending_output().len();
    if n > 0 {
        w.write_all(enc.pending_output())?;
        enc.consume_output(n);
    }
    Ok(n)
}

/// Failure of [`encode_to_writer`].
#[derive(Debug)]
#[non_exhaustive]
pub enum EncodeWriteError<E> {
    /// The encoder failed.
    Encoder(E),
    /// Writing the output failed.
    Io(io::Error),
}

impl<E: fmt::Display> fmt::Display for EncodeWriteError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encoder(e) => write!(f, "encoder: {e}"),
            Self::Io(e) => write!(f, "write: {e}"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for EncodeWriteError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Encoder(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}

/// What [`encode_to_writer`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeWriteReport {
    /// What the encoder's [`begin`](Encoder::begin) reported;
    /// [`OutputTiming::AT_FINISH`] when it declined row-level encode and the
    /// image went through [`encode`](Encoder::encode) instead.
    pub timing: OutputTiming,
    /// Bytes written to the writer.
    pub bytes: u64,
}

/// Encode `pixels` into `w`, sending bytes as the encoder produces them.
///
/// Declares the image with [`begin`](Encoder::begin), pushes it in strips,
/// and writes pending output after `begin` (the header) and after every
/// strip, so an encoder that streams sends its first bytes before it has
/// seen most rows and holds little output. An encoder that reports
/// [`OutputTiming::AT_FINISH`] gets the same rows and writes everything at
/// the end. One whose configuration cannot encode from rows (`begin`
/// declines with [`UnsupportedOperation::RowLevelEncode`]) encodes the image
/// in one call instead.
///
/// Strips are a multiple of the encoder's
/// [`preferred_strip_height`](Encoder::preferred_strip_height), at least 16
/// rows, so a one-row preference doesn't turn into one write per row.
///
/// On an error after bytes were written, the output is a prefix of a file
/// that will never be finished: abort the transport rather than end it
/// normally.
pub fn encode_to_writer<E: Encoder>(
    mut enc: E,
    pixels: PixelSlice<'_>,
    w: &mut impl Write,
) -> Result<EncodeWriteReport, EncodeWriteError<E::Error>> {
    let (width, height, desc) = (pixels.width(), pixels.rows(), pixels.descriptor());
    let timing = match enc.begin(width, height, desc) {
        Ok(t) => t,
        Err(e) if is_row_level_refusal(&e) => {
            // A failed `begin` leaves the encoder as it was.
            let out = enc.encode(pixels).map_err(EncodeWriteError::Encoder)?;
            w.write_all(out.data()).map_err(EncodeWriteError::Io)?;
            return Ok(EncodeWriteReport {
                timing: OutputTiming::AT_FINISH,
                bytes: out.len() as u64,
            });
        }
        Err(e) => return Err(EncodeWriteError::Encoder(e)),
    };
    let mut bytes = write_pending(&mut enc, w).map_err(EncodeWriteError::Io)? as u64;
    let unit = enc.preferred_strip_height().max(1);
    let strip = unit * 16u32.div_ceil(unit);
    let mut y = 0;
    while y < height {
        let h = strip.min(height - y);
        let rows = pixels.sub_rows(y, h);
        enc.push_rows(rows).map_err(EncodeWriteError::Encoder)?;
        bytes += write_pending(&mut enc, w).map_err(EncodeWriteError::Io)? as u64;
        y += h;
    }
    let rest = enc.finish().map_err(EncodeWriteError::Encoder)?;
    w.write_all(rest.data()).map_err(EncodeWriteError::Io)?;
    bytes += rest.len() as u64;
    Ok(EncodeWriteReport { timing, bytes })
}

fn is_row_level_refusal<E: core::error::Error + 'static>(e: &E) -> bool {
    use zencodec::CodecErrorExt as _;
    match e.error_category() {
        // An envelope is authoritative even when native detail has no cause chain.
        Some(category) => {
            category
                == zencodec::ErrorCategory::Request(zencodec::RequestError::Unsupported(
                    UnsupportedOperation::RowLevelEncode,
                ))
        }
        None => e.unsupported_operation() == Some(&UnsupportedOperation::RowLevelEncode),
    }
}
