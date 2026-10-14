//! File-backed random-access source.

use alloc::string::ToString;
use std::io::{Read, Seek, SeekFrom};

use zencodec::source::{ReadAt, SourceError};

/// [`ReadAt`] over a file, using seek + read per request. Correct on every
/// platform with no `unsafe`; a driver that wants `pread` concurrency can
/// supply its own source.
pub struct FileReadAt {
    file: std::fs::File,
    len: u64,
}

impl FileReadAt {
    /// Create a source over an already-open file.
    pub fn new(mut file: std::fs::File) -> Result<Self, SourceError> {
        let len = file.seek(SeekFrom::End(0)).map_err(io_err)?;
        Ok(Self { file, len })
    }

    /// Open `path` and create a source over it.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, SourceError> {
        Self::new(std::fs::File::open(path).map_err(io_err)?)
    }
}

fn io_err(e: std::io::Error) -> SourceError {
    SourceError::Io {
        message: e.to_string(),
    }
}

impl ReadAt for FileReadAt {
    type Error = SourceError;

    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(self.len)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        if offset
            .checked_add(out.len() as u64)
            .is_none_or(|end| end > self.len)
        {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(self.len),
            });
        }
        self.file.seek(SeekFrom::Start(offset)).map_err(io_err)?;
        self.file.read_exact(out).map_err(io_err)
    }
}
