//! Concrete sources for the three replay classes.
use crate::{PassSource, Replay, SourceIdentity};

use std::{
    fs::File,
    io::{self, Read},
    os::unix::fs::{FileExt, MetadataExt},
};
use zencodec_io::row_encode::{Description, Raster, Source};
use zenpixels::{PixelSlice, PixelSliceMut};

/// Validate the entire request before changing a destination or consuming input.
fn validate_rows(desc: &Description, first: u32, rows: &PixelSliceMut<'_>) -> io::Result<usize> {
    if rows.width() != desc.width
        || rows.descriptor() != desc.descriptor
        || rows.color_context() != desc.color_context.as_ref()
        || first
            .checked_add(rows.rows())
            .is_none_or(|end| end > desc.height)
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    (desc.width as usize)
        .checked_mul(desc.descriptor.bytes_per_pixel())
        .ok_or_else(|| io::ErrorKind::InvalidInput.into())
}

fn check(pulse: &dyn zencodec::Pulse) -> io::Result<()> {
    pulse.check().map_err(|reason| {
        io::Error::new(
            io::ErrorKind::Interrupted,
            zencodec::At::wrap(zencodec::CodecError::new(None, reason.into())),
        )
    })
}

/// A caller-borrowed complete raster. Delegates reads to the existing
/// `row_encode::Raster` (stride and color-context checks unchanged).
pub struct Borrowed<'a>(Raster<'a>);
impl<'a> Borrowed<'a> {
    /// Hold the borrow for the whole operation.
    pub const fn new(pixels: PixelSlice<'a>) -> Self {
        Self(Raster::new(pixels))
    }
}
impl PassSource for Borrowed<'_> {
    type Error = <Raster<'static> as Source>::Error;
    fn description(&self) -> Description {
        self.0.description()
    }
    fn replay(&self) -> Replay {
        Replay::Positional
    }
    fn read_rows(
        &mut self,
        first: u32,
        rows: PixelSliceMut<'_>,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), Self::Error> {
        self.0.read_rows(first, rows, stop)
    }
}

/// Any existing one-pass `row_encode::Source`, treated conservatively as forward.
pub struct Sequential<S>(pub S);
impl<S: Source> PassSource for Sequential<S> {
    type Error = S::Error;
    fn description(&self) -> Description {
        self.0.description()
    }
    fn read_rows(
        &mut self,
        first: u32,
        rows: PixelSliceMut<'_>,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), Self::Error> {
        self.0.read_rows(first, rows, stop)
    }
}

/// Raw rows in a file at `origin` with `stride` bytes per row (padding allowed).
/// Positional reads; identity is (device, inode, length, mtime ns).
pub struct FileRows {
    file: File,
    origin: u64,
    stride: u64,
    desc: Description,
}
impl FileRows {
    /// Rows of `desc` at `origin`, `stride` bytes apart.
    pub fn new(file: File, origin: u64, stride: u64, desc: Description) -> Self {
        Self {
            file,
            origin,
            stride,
            desc,
        }
    }
}
impl PassSource for FileRows {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.desc.clone()
    }
    fn replay(&self) -> Replay {
        Replay::Positional
    }
    fn identity(&mut self) -> io::Result<Option<SourceIdentity>> {
        let m = self.file.metadata()?;
        let mtime = (m.mtime() as u64).wrapping_mul(1_000_000_000) ^ m.mtime_nsec() as u64;
        Ok(Some(SourceIdentity([m.dev(), m.ino(), m.len(), mtime])))
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> io::Result<()> {
        let local_pulse = pulse.unwrap_or(&zencodec::NoPulse);
        let active = validate_rows(&self.desc, first, &rows)? as u64;
        if self.stride < active {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if rows.rows() != 0 {
            // Validate the last active byte as well as the starting offset.
            u64::from(first + rows.rows() - 1)
                .checked_mul(self.stride)
                .and_then(|offset| offset.checked_add(self.origin))
                .and_then(|offset| offset.checked_add(active))
                .ok_or(io::ErrorKind::InvalidInput)?;
        }
        for y in 0..rows.rows() {
            check(local_pulse)?;
            let at = u64::from(first + y)
                .checked_mul(self.stride)
                .and_then(|o| o.checked_add(self.origin))
                .ok_or(io::ErrorKind::InvalidInput)?;
            self.file.read_exact_at(rows.row_mut(y), at)?;
        }
        check(local_pulse)?;
        Ok(())
    }
}

/// A forward byte stream of rows with `stride` bytes per row; padding is read
/// and discarded. Any non-sequential request is an error.
pub struct ReaderRows<R> {
    reader: R,
    stride: usize,
    desc: Description,
    next: u32,
    skip: Vec<u8>,
}
impl<R: Read> ReaderRows<R> {
    /// Rows of `desc`, `stride` bytes apart, from the reader's current position.
    pub fn new(reader: R, stride: usize, desc: Description) -> Self {
        Self {
            reader,
            stride,
            desc,
            next: 0,
            skip: Vec::new(),
        }
    }
}
impl<R: Read> PassSource for ReaderRows<R> {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.desc.clone()
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> io::Result<()> {
        let local_pulse = pulse.unwrap_or(&zencodec::NoPulse);
        let active = validate_rows(&self.desc, first, &rows)?;
        let padding = self
            .stride
            .checked_sub(active)
            .ok_or(io::ErrorKind::InvalidInput)?;
        if first != self.next {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "forward source cannot seek",
            ));
        }
        check(local_pulse)?;
        // File pitch can be much larger than active samples. Retain at most one
        // bounded padding chunk, allocated before reading any part of a row.
        let chunk = padding.min(4096);
        if self.skip.len() < chunk {
            self.skip
                .try_reserve_exact(chunk - self.skip.len())
                .map_err(|_| io::ErrorKind::OutOfMemory)?;
            self.skip.resize(chunk, 0);
        }
        for y in 0..rows.rows() {
            check(local_pulse)?;
            let row = rows.row_mut(y);
            self.reader.read_exact(row)?;
            let mut remaining = padding;
            while remaining != 0 {
                check(local_pulse)?;
                let len = remaining.min(self.skip.len());
                self.reader.read_exact(&mut self.skip[..len])?;
                remaining -= len;
            }
            self.next += 1;
        }
        check(local_pulse)?;
        Ok(())
    }
}
