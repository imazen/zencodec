//! Private Unix positional-read prototype and deterministic read workload.
#![allow(dead_code)]
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use zencodec::source::{ReadAt, SourceError};
pub const FILE_BYTES: usize = 64 * 1024 * 1024;
pub struct Positional {
    file: File,
    len: u64,
}
impl Positional {
    pub fn new(file: File) -> io::Result<Self> {
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}
impl ReadAt for Positional {
    type Error = SourceError;
    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(self.len)
    }
    fn len_hint(&self) -> Option<u64> {
        Some(self.len)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        if offset
            .checked_add(out.len() as u64)
            .is_none_or(|n| n > self.len)
        {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(self.len),
            });
        }
        self.file
            .read_exact_at(out, offset)
            .map_err(|e| SourceError::Io {
                message: e.to_string(),
            })
    }
}
pub fn byte_at(i: usize) -> u8 {
    ((i.wrapping_mul(37) ^ (i >> 8)) % 251) as u8
}
pub fn fixture(path: &Path) {
    if path.exists() {
        assert_eq!(std::fs::metadata(path).unwrap().len(), FILE_BYTES as u64);
    } else {
        let bytes: Vec<u8> = (0..FILE_BYTES).map(byte_at).collect();
        std::fs::write(path, bytes).unwrap();
    }
}
pub fn offsets(size: usize, random: bool) -> Vec<u64> {
    (0..128)
        .map(|i| {
            let block = if random {
                (i * 104729 + 1237) % (FILE_BYTES / size)
            } else {
                i
            };
            (block * size) as u64
        })
        .collect()
}
pub fn verify<S: ReadAt + ?Sized>(source: &mut S, size: usize, offsets: &[u64]) {
    let mut out = vec![0; size];
    for &offset in offsets {
        source.read_exact_at(offset, &mut out).unwrap();
        assert!(
            out.iter()
                .enumerate()
                .all(|(i, &b)| b == byte_at(offset as usize + i))
        );
    }
}

/// Keep the frozen controls' read algorithms while giving every comparison arm
/// the same lossless error envelope as the current file source.
pub struct NativeErrors<S>(pub S);
impl<S: ReadAt> ReadAt for NativeErrors<S>
where
    S::Error: Into<zencodec_io::StdSourceError>,
{
    type Error = zencodec_io::StdSourceError;
    fn len(&mut self) -> Result<u64, Self::Error> {
        self.0.len().map_err(Into::into)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        self.0.read_exact_at(offset, out).map_err(Into::into)
    }
}

/// Frozen seek/read control from zencodec-io 9ca5fd1f, kept explicit so the
/// comparison label does not silently change when FileReadAt improves.
pub struct SeekRead {
    file: File,
    len: u64,
}
impl SeekRead {
    pub fn open(path: &Path) -> io::Result<Self> {
        use std::io::{Seek, SeekFrom};
        let mut file = File::open(path)?;
        let len = file.seek(SeekFrom::End(0))?;
        Ok(Self { file, len })
    }
}
impl ReadAt for SeekRead {
    type Error = SourceError;
    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(self.len)
    }
    fn len_hint(&self) -> Option<u64> {
        Some(self.len)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        use std::io::{Read, Seek, SeekFrom};
        if offset
            .checked_add(out.len() as u64)
            .is_none_or(|n| n > self.len)
        {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(self.len),
            });
        }
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read_exact(out))
            .map_err(|e| SourceError::Io {
                message: e.to_string(),
            })
    }
}
