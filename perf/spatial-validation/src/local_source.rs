//! Private erasure candidate for thread-local positional sources.
//! The existing DynReadAt requires Send. Native synchronous collections do not
//! need that requirement; the HTTP simulator deliberately uses local state.
use zencodec::{collection::Cause, source::ReadAt};

pub trait LocalSource {
    fn len(&mut self) -> Result<u64, Cause>;
    fn len_hint(&self) -> Option<u64>;
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Cause>;
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Cause>;
    fn read_many_at(&mut self, requests: &mut [(u64, &mut [u8])]) -> Result<(), Cause>;
    fn as_slice(&self) -> Option<&[u8]>;
}
impl<S: ReadAt + ?Sized> LocalSource for S {
    fn len(&mut self) -> Result<u64, Cause> {
        ReadAt::len(self).map_err(|e| Box::new(e) as Cause)
    }
    fn len_hint(&self) -> Option<u64> {
        ReadAt::len_hint(self)
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Cause> {
        ReadAt::read_at(self, offset, out).map_err(|e| Box::new(e) as Cause)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Cause> {
        ReadAt::read_exact_at(self, offset, out).map_err(|e| Box::new(e) as Cause)
    }
    fn read_many_at(&mut self, requests: &mut [(u64, &mut [u8])]) -> Result<(), Cause> {
        ReadAt::read_many_at(self, requests).map_err(|e| Box::new(e) as Cause)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        ReadAt::as_slice(self)
    }
}
