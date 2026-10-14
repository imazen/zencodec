//! Shared partial-acceptance bookkeeping for every driver.

use alloc::vec::Vec;

use crate::drivable::Drivable;

/// Offers chunks to a decoder and keeps whatever it did not accept.
///
/// `feed` may accept fewer bytes than offered when the decoder's input
/// buffer is bounded. Every driver in this crate goes through a `Feeder`
/// so the leftover is re-offered before any new bytes are fetched, and a
/// decoder that refuses bytes it keeps asking for is reported as stalled
/// instead of looping.
#[derive(Debug, Default)]
pub struct Feeder {
    carry: Vec<(u64, Vec<u8>, bool)>,
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
            self.carry.push((offset, bytes.to_vec(), eof));
            return Ok(false);
        }
        let accepted = dec.feed(offset, bytes, eof)?;
        if accepted < bytes.len() {
            self.carry
                .push((offset + accepted as u64, bytes[accepted..].to_vec(), eof));
            return Ok(false);
        }
        Ok(true)
    }

    /// Re-offer carried bytes. Returns `true` when nothing is left over.
    pub fn flush<D: Drivable + ?Sized>(&mut self, dec: &mut D) -> Result<bool, D::Error> {
        while let Some((offset, bytes, eof)) = self.carry.first_mut() {
            let accepted = dec.feed(*offset, bytes, *eof)?;
            if accepted == 0 {
                return Ok(false);
            }
            if accepted >= bytes.len() {
                self.carry.remove(0);
            } else {
                bytes.drain(..accepted);
                *offset += accepted as u64;
                return Ok(false);
            }
        }
        Ok(true)
    }
}
