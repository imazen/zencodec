//! Write a streaming encoder's output to a `std::io::Write` as it is
//! produced.

use std::io::{self, Write};

use zencodec::encode::{AnimationFrameEncoder, Encoder};

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
