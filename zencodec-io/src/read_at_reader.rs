//! Synchronous parser bridge over an existing positional source.

use alloc::boxed::Box;
use std::io::{self, Read, Seek, SeekFrom};
use zencodec::source::ReadAt;

use crate::StdSourceError;

/// Adapt a positional source to [`Read`] and [`Seek`] with an independent cursor.
///
/// This is for synchronous parsers that already require standard I/O. It adds
/// no buffering, prefetch, retry or asynchronous suspension. Wrap it in
/// [`std::io::BufReader`] only when its extra reads fit the transport's plan.
/// Batched range requests remain available through [`ReadAt::read_many_at`];
/// a parser making individual `Read` calls cannot express such a batch here.
///
/// End-relative seeks call [`ReadAt::len`], which can consume a sequential source
/// to discover its length. Other seeks and [`Seek::stream_position`] do no I/O.
/// Seeking past EOF is allowed. A failed read or seek leaves the logical cursor
/// unchanged, but may have changed the source's state or part of the read buffer.
/// Whether retry is valid remains a source contract.
///
/// Native `std::io::Error` values (including [`StdSourceError::Io`]) are returned
/// unchanged. Other errors retain their concrete value as an I/O payload, with
/// an underlying I/O kind when present. [`zencodec::find_cause`] inspects these
/// payloads as well as ordinary error chains.
#[derive(Debug)]
pub struct ReadAtReader<S> {
    source: S,
    position: u64,
}

impl<S> ReadAtReader<S> {
    /// Take ownership of a source, starting the logical cursor at zero.
    pub const fn new(source: S) -> Self {
        Self {
            source,
            position: 0,
        }
    }

    /// Recover the source. The bridge retains no buffered bytes.
    pub fn into_inner(self) -> S {
        self.source
    }
}

fn into_io(error: impl core::error::Error + Send + Sync + 'static) -> io::Error {
    let error: Box<dyn core::error::Error + Send + Sync> = Box::new(error);
    let error = match error.downcast::<io::Error>() {
        Ok(native) => return *native,
        Err(other) => other,
    };
    let error = match error.downcast::<StdSourceError>() {
        Ok(source) => match *source {
            StdSourceError::Io(native) => return native,
            other => return io::Error::other(other),
        },
        Err(other) => other,
    };
    let kind = zencodec::find_cause::<io::Error>(error.as_ref())
        .map_or(io::ErrorKind::Other, io::Error::kind);
    io::Error::new(kind, error)
}

impl<S: ReadAt> Read for ReadAtReader<S> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let n = self.source.read_at(self.position, out).map_err(into_io)?;
        if n > out.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "positional source returned a count larger than its buffer",
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
        let position = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::Current(n) => self.position.checked_add_signed(n),
            SeekFrom::End(n) => self.source.len().map_err(into_io)?.checked_add_signed(n),
        }
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "seek outside u64 offset space")
        })?;
        self.position = position;
        Ok(position)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}
