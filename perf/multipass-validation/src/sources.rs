//! Concrete sources for the three replay classes.
use crate::{PassSource, Replay, SourceIdentity};
use enough::Stop;
use std::{
    fs::File,
    io::{self, Read},
    os::unix::fs::{FileExt, MetadataExt},
};
use zencodec_io::row_encode::{Description, Raster, Source};
use zenpixels::{PixelSlice, PixelSliceMut};

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
        stop: &dyn Stop,
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
        stop: &dyn Stop,
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
        stop: &dyn Stop,
    ) -> io::Result<()> {
        for y in 0..rows.rows() {
            stop.check()
                .map_err(|r| io::Error::new(io::ErrorKind::Interrupted, format!("{r:?}")))?;
            let at = u64::from(first + y)
                .checked_mul(self.stride)
                .and_then(|o| o.checked_add(self.origin))
                .ok_or(io::ErrorKind::InvalidInput)?;
            self.file.read_exact_at(rows.row_mut(y), at)?;
        }
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
        stop: &dyn Stop,
    ) -> io::Result<()> {
        if first != self.next {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "forward source cannot seek",
            ));
        }
        for y in 0..rows.rows() {
            stop.check()
                .map_err(|r| io::Error::new(io::ErrorKind::Interrupted, format!("{r:?}")))?;
            let row = rows.row_mut(y);
            self.reader.read_exact(row)?;
            let pad = self.stride - row.len();
            if pad != 0 {
                self.skip.resize(pad, 0);
                self.reader.read_exact(&mut self.skip)?;
            }
            self.next += 1;
        }
        Ok(())
    }
}
