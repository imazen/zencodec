//! Blocking sources and destinations.

use crate::drive::{Batch, DriveOptions, Next, RangeRead, Session, Window};
use crate::memory::decode_slice;
use std::io::{self, Read, Seek, SeekFrom, Write};
use zencodec::decode::{Decoder, Sink};
use zencodec::encode::{OutputBuffer, Patch};
use zencodec::{CodecError, Error, ErrorKind, OutputAccess, Pulse, Result};

/// A source that can be read at a position.
pub trait ReadAt {
    /// Read up to `out.len()` bytes at `offset`; `Ok(0)` at the end.
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize>;

    /// The total length, when the source knows it.
    fn total_len(&mut self) -> io::Result<Option<u64>> {
        Ok(None)
    }

    /// The whole input, when it is already in memory. A driver borrows it
    /// instead of reading: no window, no copy.
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }

    /// Append everything from `offset` to the end of the input to `out`,
    /// or `max` bytes if that comes first. Returns the bytes appended.
    ///
    /// A driver calls this when a decoder needs the whole file. The default
    /// reads in pieces through [`read_at`](Self::read_at), filling memory
    /// before each piece. A source over a reader overrides it with the
    /// reader's own read-to-end, which fills nothing.
    fn read_to_end(&mut self, offset: u64, max: usize, out: &mut Vec<u8>) -> io::Result<usize> {
        let start = out.len();
        if let Some(total) = self.total_len()? {
            let rest = total.saturating_sub(offset).min(max as u64);
            reserve(out, rest as usize)?;
        }
        let mut piece = PIECE;
        loop {
            let got = out.len() - start;
            let want = piece.min(max - got);
            if want == 0 {
                break;
            }
            let at = out.len();
            reserve(out, want)?;
            out.resize(at + want, 0);
            let n = self.read_at(offset + got as u64, &mut out[at..])?;
            out.truncate(at + n);
            if n == 0 {
                break;
            }
            piece = piece.saturating_mul(2).min(PIECE_MAX);
        }
        Ok(out.len() - start)
    }

    /// Read several spans. A source with slow round trips overrides this to
    /// fetch them together. A span the input ends inside is shortened.
    fn read_ranges(&mut self, ranges: &mut [RangeRead]) -> io::Result<()> {
        for range in ranges {
            let mut got = 0;
            while got < range.buf.len() {
                match self.read_at(range.offset + got as u64, &mut range.buf[got..])? {
                    0 => break,
                    n => got += n,
                }
            }
            range.buf.truncate(got);
        }
        Ok(())
    }
}

/// The first piece a default read-to-end asks for, and the largest.
pub(crate) const PIECE: usize = 64 * 1024;
pub(crate) const PIECE_MAX: usize = 8 * 1024 * 1024;

/// Make room for `more` bytes, reporting failure instead of aborting.
pub(crate) fn reserve(out: &mut Vec<u8>, more: usize) -> io::Result<()> {
    out.try_reserve(more)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))
}

impl ReadAt for &[u8] {
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        let from = (offset.min(self.len() as u64)) as usize;
        let n = out.len().min(self.len() - from);
        out[..n].copy_from_slice(&self[from..from + n]);
        Ok(n)
    }

    fn total_len(&mut self) -> io::Result<Option<u64>> {
        Ok(Some(<[u8]>::len(self) as u64))
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(self)
    }
}

/// A seekable reader as a positioned source.
#[derive(Debug)]
pub struct SeekRead<R>(pub R);

impl<R: Read + Seek> ReadAt for SeekRead<R> {
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        self.0.seek(SeekFrom::Start(offset))?;
        self.0.read(out)
    }

    fn total_len(&mut self) -> io::Result<Option<u64>> {
        self.0.seek(SeekFrom::End(0)).map(Some)
    }

    fn read_to_end(&mut self, offset: u64, max: usize, out: &mut Vec<u8>) -> io::Result<usize> {
        let end = self.0.seek(SeekFrom::End(0))?;
        self.0.seek(SeekFrom::Start(offset))?;
        reserve(out, end.saturating_sub(offset).min(max as u64) as usize)?;
        (&mut self.0).take(max as u64).read_to_end(out)
    }
}

/// A forward-only reader as a positioned source. A read behind the current
/// position fails: pair it with a decoder whose plan is sequential.
#[derive(Debug)]
pub struct Sequential<R> {
    reader: R,
    position: u64,
}

impl<R: Read> Sequential<R> {
    /// Wrap a reader positioned at the start of the image.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            position: 0,
        }
    }
}

impl<R: Read> ReadAt for Sequential<R> {
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        if offset < self.position {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "backward read on a forward-only source",
            ));
        }
        // Skip ahead by reading and discarding.
        let mut skip = offset - self.position;
        let mut scratch = [0u8; 4096];
        while skip > 0 {
            let want = skip.min(scratch.len() as u64) as usize;
            match self.reader.read(&mut scratch[..want])? {
                0 => return Ok(0),
                n => {
                    skip -= n as u64;
                    self.position += n as u64;
                }
            }
        }
        let n = self.reader.read(out)?;
        self.position += n as u64;
        Ok(n)
    }

    fn read_to_end(&mut self, offset: u64, max: usize, out: &mut Vec<u8>) -> io::Result<usize> {
        // An empty read skips ahead to `offset`, or fails if it is behind.
        self.read_at(offset, &mut [])?;
        if self.position < offset {
            return Ok(0);
        }
        let n = (&mut self.reader).take(max as u64).read_to_end(out)?;
        self.position += n as u64;
        Ok(n)
    }
}

pub(crate) fn source_error(e: io::Error) -> Error {
    CodecError::new(ErrorKind::Source).with_native(e).at()
}

pub(crate) fn sink_error(e: io::Error) -> Error {
    CodecError::new(ErrorKind::Sink).with_native(e).at()
}

/// Run a prepared decoder over a positioned source.
///
/// A source that is already in memory is borrowed, not read.
/// Returns the decoder, whose [`info`](Decoder::info) is now complete.
pub fn decode_read_at(
    decoder: Box<dyn Decoder>,
    source: &mut dyn ReadAt,
    sink: &mut dyn Sink,
    opts: &DriveOptions,
    pulse: &dyn Pulse,
) -> Result<Box<dyn Decoder>> {
    if let Some(bytes) = source.as_slice() {
        return decode_slice(decoder, bytes, sink, pulse);
    }
    let mut session = Session::new(decoder, sink, opts);
    loop {
        match session.run(usize::MAX, pulse)? {
            Batch::Done => return Ok(session.decoder),
            Batch::Yield => {}
            Batch::Read(next) => read_next(&mut session.window, source, next)?,
        }
    }
}

/// Perform the read a session asked for.
fn read_next(window: &mut Window, source: &mut dyn ReadAt, next: Next) -> Result<()> {
    match next {
        Next::Ready => {}
        Next::Read { offset, len } => {
            let n = source
                .read_at(offset, window.spare(len))
                .map_err(source_error)?;
            window.commit(len, n);
        }
        Next::ToEnd { offset, max } => {
            source
                .read_to_end(offset, max, window.tail())
                .map_err(source_error)?;
            window.ended(max)?;
        }
        Next::Ranges(mut reads) => {
            let wanted: Vec<usize> = reads.iter().map(|r| r.buf.len()).collect();
            source.read_ranges(&mut reads).map_err(source_error)?;
            window.store_ranges(reads, &wanted);
        }
        Next::Len => match source.total_len().map_err(source_error)? {
            Some(total) => window.set_total(total),
            None => {
                return Err(CodecError::new(ErrorKind::Source)
                    .with_message("the source cannot report its length")
                    .at());
            }
        },
    }
    Ok(())
}

/// Where encoded bytes go.
pub trait Destination {
    /// Whether [`patch`](Self::patch) works.
    fn access(&self) -> OutputAccess;

    /// Append bytes.
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// Overwrite bytes written earlier.
    fn patch(&mut self, patch: &Patch) -> io::Result<()>;

    /// Push buffered bytes through.
    fn flush(&mut self) -> io::Result<()>;
}

/// A writer that cannot go back.
#[derive(Debug)]
pub struct Forward<W>(pub W);

impl<W: Write> Destination for Forward<W> {
    fn access(&self) -> OutputAccess {
        OutputAccess::Forward
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    fn patch(&mut self, _: &Patch) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "patch on a forward-only destination",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// A writer that can seek back to patch what it wrote.
#[derive(Debug)]
pub struct SeekBack<W> {
    writer: W,
    /// Where the image starts in the writer.
    origin: u64,
}

impl<W: Write + Seek> SeekBack<W> {
    /// Wrap a writer positioned where the image should start.
    pub fn new(mut writer: W) -> io::Result<Self> {
        let origin = writer.stream_position()?;
        Ok(Self { writer, origin })
    }

    /// The writer, positioned after the image.
    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W: Write + Seek> Destination for SeekBack<W> {
    fn access(&self) -> OutputAccess {
        OutputAccess::Patchable
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)
    }

    fn patch(&mut self, patch: &Patch) -> io::Result<()> {
        let end = self.writer.stream_position()?;
        self.writer
            .seek(SeekFrom::Start(self.origin + patch.offset))?;
        self.writer.write_all(&patch.bytes)?;
        self.writer.seek(SeekFrom::Start(end))?;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Move everything an encoder has ready to a destination: pending bytes,
/// then patches. Returns the bytes appended.
pub fn write_output(output: &mut OutputBuffer, destination: &mut dyn Destination) -> Result<u64> {
    let chunk = output.take();
    let written = chunk.len() as u64;
    if !chunk.is_empty() {
        destination.write_all(&chunk).map_err(sink_error)?;
    }
    output.recycle(chunk);
    for patch in output.take_patches() {
        destination.patch(&patch).map_err(sink_error)?;
    }
    Ok(written)
}
