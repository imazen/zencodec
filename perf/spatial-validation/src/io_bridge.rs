use std::io::{self, Read, Seek, SeekFrom};
use zencodec::source::ReadAt;

/// A synchronous parser bridge with an independent logical cursor. It adds no
/// prefetch, buffering, retry or asynchronous suspension. Seek-from-end may
/// consume a sequential source to discover its length.
pub struct ReadAtReader<S> {
    source: S,
    position: u64,
}
impl<S> ReadAtReader<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            position: 0,
        }
    }
    pub fn into_inner(self) -> S {
        self.source
    }
}
impl<S: ReadAt> Read for ReadAtReader<S> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let n = self
            .source
            .read_at(self.position, out)
            .map_err(io::Error::other)?;
        if n > out.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid source read count",
            ));
        }
        self.position = self.position.checked_add(n as u64).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "source position overflow")
        })?;
        Ok(n)
    }
}
impl<S: ReadAt> Seek for ReadAtReader<S> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let value = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::Current(n) => self.position.checked_add_signed(n),
            SeekFrom::End(n) => self
                .source
                .len()
                .map_err(io::Error::other)?
                .checked_add_signed(n),
        }
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "seek outside u64 offset space")
        })?;
        self.position = value;
        Ok(value)
    }
    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}
