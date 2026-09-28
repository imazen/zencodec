//! Bounded adapters for codecs that need a complete byte slice or seeking.
//!
//! Native packet decoding can read incrementally. Image codecs whose current
//! API takes `Cow<[u8]>` need `read_bounded`, which deliberately materializes
//! the encoded file. `SeekableSource` gives nonseekable input an immutable
//! snapshot, spilling to a caller-selected disk directory above a memory cap.

use enough::Stop;
use std::{
    fmt,
    fs::File,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::Path,
};

const CHUNK: usize = 32 * 1024;

/// Read at most `max_bytes` into fallibly allocated storage. One extra byte is
/// consumed to distinguish an exact-size file from an oversized one. Interrupted
/// reads are retried; all other I/O errors are preserved. A Stop token is checked
/// between reads, not inside a blocking `Read`; network readers need timeouts.
pub fn read_bounded<R: Read>(
    mut reader: R,
    max_bytes: usize,
    stop: Option<&dyn Stop>,
) -> Result<Vec<u8>, SourceError> {
    let mut output = Vec::new();
    pump(&mut reader, max_bytes as u64, stop, |bytes| {
        output
            .try_reserve_exact(bytes.len())
            .map_err(|_| SourceError::Allocation)?;
        output.extend_from_slice(bytes);
        Ok(())
    })?;
    Ok(output)
}

enum Storage {
    Memory(Cursor<Vec<u8>>),
    File(File),
}

/// A completed, immutable input snapshot implementing Read + Seek. Construction
/// never reports a usable source until EOF; input errors/cancellation discard the
/// partial snapshot. Temporary disk storage is automatically removed on close.
pub struct SeekableSource {
    storage: Storage,
    length: u64,
}

impl SeekableSource {
    /// Materialize a source with independent memory and total-byte limits.
    /// The supplied directory must be on a suitable disk filesystem. It is only
    /// opened if the stream exceeds `memory_bytes`; zero forces disk storage for
    /// nonempty sources. A zero total limit accepts only an empty input.
    pub fn read_from<R: Read>(
        mut reader: R,
        memory_bytes: usize,
        max_bytes: u64,
        directory: impl AsRef<Path>,
        stop: Option<&dyn Stop>,
    ) -> Result<Self, SourceError> {
        let mut storage = Storage::Memory(Cursor::new(Vec::new()));
        let mut length = 0_u64;
        pump(&mut reader, max_bytes, stop, |bytes| {
            let next = length
                .checked_add(bytes.len() as u64)
                .ok_or(SourceError::Limit)?;
            if let Storage::Memory(memory) = &mut storage {
                if next <= memory_bytes as u64 {
                    memory
                        .get_mut()
                        .try_reserve_exact(bytes.len())
                        .map_err(|_| SourceError::Allocation)?;
                    memory.get_mut().extend_from_slice(bytes);
                } else {
                    let mut file =
                        tempfile::tempfile_in(directory.as_ref()).map_err(SourceError::Io)?;
                    file.write_all(memory.get_ref()).map_err(SourceError::Io)?;
                    file.write_all(bytes).map_err(SourceError::Io)?;
                    storage = Storage::File(file);
                }
            } else if let Storage::File(file) = &mut storage {
                file.write_all(bytes).map_err(SourceError::Io)?;
            }
            length = next;
            Ok(())
        })?;
        let mut result = Self { storage, length };
        result.seek(SeekFrom::Start(0)).map_err(SourceError::Io)?;
        Ok(result)
    }

    pub fn len(&self) -> u64 {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn is_on_disk(&self) -> bool {
        matches!(self.storage, Storage::File(_))
    }

    /// Obtain a whole encoded byte array for existing image-codec APIs. Disk
    /// storage must fit the caller's separate memory budget. The current seek
    /// position does not affect the returned complete snapshot.
    pub fn into_bytes(
        self,
        max_bytes: usize,
        stop: Option<&dyn Stop>,
    ) -> Result<Vec<u8>, SourceError> {
        check(stop)?;
        if self.length > max_bytes as u64 {
            return Err(SourceError::Limit);
        }
        match self.storage {
            Storage::Memory(memory) => Ok(memory.into_inner()),
            Storage::File(mut file) => {
                file.seek(SeekFrom::Start(0)).map_err(SourceError::Io)?;
                read_bounded(file, max_bytes, stop)
            }
        }
    }
}

impl Read for SeekableSource {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match &mut self.storage {
            Storage::Memory(m) => m.read(bytes),
            Storage::File(f) => f.read(bytes),
        }
    }
}
impl Seek for SeekableSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match &mut self.storage {
            Storage::Memory(m) => m.seek(position),
            Storage::File(f) => f.seek(position),
        }
    }
}

fn check(stop: Option<&dyn Stop>) -> Result<(), SourceError> {
    if let Some(stop) = stop {
        stop.check().map_err(|_| SourceError::Cancelled)?;
    }
    Ok(())
}

fn pump<R: Read, F: FnMut(&[u8]) -> Result<(), SourceError>>(
    reader: &mut R,
    max_bytes: u64,
    stop: Option<&dyn Stop>,
    mut accept: F,
) -> Result<(), SourceError> {
    let mut buffer = [0; CHUNK];
    let mut used = 0_u64;
    loop {
        check(stop)?;
        let amount = usize::try_from((max_bytes - used).saturating_add(1).min(CHUNK as u64))
            .expect("chunk fits usize");
        let n = match reader.read(&mut buffer[..amount]) {
            Ok(0) => return check(stop),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(SourceError::Io(e)),
        };
        used = used.checked_add(n as u64).ok_or(SourceError::Limit)?;
        if used > max_bytes {
            return Err(SourceError::Limit);
        }
        check(stop)?;
        accept(&buffer[..n])?;
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum SourceError {
    Limit,
    Allocation,
    Cancelled,
    Io(io::Error),
}
impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit => f.write_str("encoded source exceeds the byte budget"),
            Self::Allocation => f.write_str("encoded source allocation failed"),
            Self::Cancelled => f.write_str("encoded source read cancelled"),
            Self::Io(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for SourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
