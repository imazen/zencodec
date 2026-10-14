//! Shared partial-acceptance bookkeeping for every driver.

use alloc::vec::Vec;

use crate::drivable::Drivable;

/// Offers chunks to a decoder and keeps whatever it did not accept.
///
/// `feed` may accept fewer bytes than offered when the decoder's input
/// buffer is bounded. Every driver in this crate goes through a `Feeder`
/// so the leftover is re-offered before any new bytes are fetched, and a
/// decoder that refuses bytes it keeps asking for is reported as stalled
/// instead of looping. A carried chunk is consumed by advancing a head
/// index, never by shifting its bytes, so a large remainder costs one copy
/// when it is carried and nothing per re-offer.
#[derive(Debug, Default)]
pub struct Feeder {
    carry: Vec<Carried>,
}

#[derive(Debug)]
struct Carried {
    offset: u64,
    bytes: Vec<u8>,
    /// `bytes[head..]` is still unaccepted.
    head: usize,
    eof: bool,
}

impl Feeder {
    /// A feeder with nothing carried over.
    pub const fn new() -> Self {
        Self { carry: Vec::new() }
    }

    /// Whether bytes are still waiting to be accepted.
    pub fn has_carry(&self) -> bool {
        !self.carry.is_empty()
    }

    /// Offer carried bytes first, then `bytes`. Anything not accepted is
    /// kept for the next call. Returns `true` when nothing is left over.
    pub fn offer<D: Drivable + ?Sized>(
        &mut self,
        dec: &mut D,
        offset: u64,
        bytes: &[u8],
        eof: bool,
    ) -> Result<bool, D::Error> {
        if !self.flush(dec)? {
            self.carry.push(Carried {
                offset,
                bytes: bytes.to_vec(),
                head: 0,
                eof,
            });
            return Ok(false);
        }
        let accepted = dec.feed(offset, bytes, eof)?;
        if accepted < bytes.len() {
            self.carry.push(Carried {
                offset: offset + accepted as u64,
                bytes: bytes[accepted..].to_vec(),
                head: 0,
                eof,
            });
            return Ok(false);
        }
        Ok(true)
    }

    /// Re-offer carried bytes. Returns `true` when nothing is left over.
    pub fn flush<D: Drivable + ?Sized>(&mut self, dec: &mut D) -> Result<bool, D::Error> {
        while let Some(c) = self.carry.first_mut() {
            let live = &c.bytes[c.head..];
            let accepted = dec.feed(c.offset, live, c.eof)?;
            if accepted == 0 {
                return Ok(false);
            }
            if accepted >= live.len() {
                self.carry.remove(0);
            } else {
                c.head += accepted;
                c.offset += accepted as u64;
                return Ok(false);
            }
        }
        Ok(true)
    }
}
