//! Actual forward-only and seek-back destinations for synchronous output.
//! A wrapper binds access and the output origin to the retained writer.
use std::io::{self, Seek, SeekFrom, Write};

mod sealed {
    pub trait Sealed {}
}
/// Writer access derived by wrapping an actual writer in [`Forward`] or [`SeekBack`].
/// Sealed so callers cannot invent seek support independently of the writer.
pub trait Destination: Write + sealed::Sealed {
    /// Whether this destination can overwrite relative output positions.
    fn can_patch(&self) -> bool;
    /// Seek relative to the output origin; fails for a forward-only writer.
    fn position(&mut self, relative: u64) -> io::Result<()>;
}

/// Forward-only output, including sockets and other `Write` implementations.
pub struct Forward<W>(pub W);
impl<W> sealed::Sealed for Forward<W> {}
impl<W: Write> Write for Forward<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
impl<W: Write> Destination for Forward<W> {
    fn can_patch(&self) -> bool {
        false
    }
    fn position(&mut self, _: u64) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Seekable output whose offsets are relative to the writer's initial position.
/// Existing prefixes remain outside all output patches.
pub struct SeekBack<W> {
    writer: W,
    start: u64,
}
impl<W> sealed::Sealed for SeekBack<W> {}
impl<W: Write + Seek> SeekBack<W> {
    /// Capture the current position as the output origin.
    pub fn new(mut writer: W) -> io::Result<Self> {
        let start = writer.stream_position()?;
        Ok(Self { writer, start })
    }
}
impl<W: Write> Write for SeekBack<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writer.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}
impl<W: Write + Seek> Destination for SeekBack<W> {
    fn can_patch(&self) -> bool {
        true
    }
    fn position(&mut self, relative: u64) -> io::Result<()> {
        let absolute = self
            .start
            .checked_add(relative)
            .ok_or(io::ErrorKind::InvalidInput)?;
        let actual = self.writer.seek(SeekFrom::Start(absolute))?;
        if actual != absolute {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        Ok(())
    }
}
