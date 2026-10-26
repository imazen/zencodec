//! File-backed random-access source.

#[cfg(not(any(unix, windows)))]
use std::io::{Read, Seek, SeekFrom};

use crate::StdSourceError;
use zencodec::source::{ReadAt, SourceError};

/// [`ReadAt`] over a file, using positional reads on Unix and Windows and
/// seek + read elsewhere. Unix reads leave a shared file cursor unchanged;
/// Windows `seek_read` can update it. No implementation uses `unsafe`.
///
/// The caller must keep the opened file's bytes stable for the decoder operation.
/// This adapter captures its length but neither locks the file nor detects every
/// in-place replacement. An open handle and positional reads do not establish
/// the coherent-representation contract of [`ReadAt`].
pub struct FileReadAt {
    file: std::fs::File,
    len: u64,
}

impl FileReadAt {
    /// Create a source over an already-open file.
    pub fn new(file: std::fs::File) -> Result<Self, StdSourceError> {
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }

    /// Open `path` and create a source over it.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StdSourceError> {
        Self::new(std::fs::File::open(path)?)
    }
}

impl ReadAt for FileReadAt {
    type Error = StdSourceError;

    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(self.len)
    }

    fn len_hint(&self) -> Option<u64> {
        Some(self.len)
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
            }
            .into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            self.file.read_exact_at(out, offset).map_err(Into::into)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            let mut filled = 0;
            while filled < out.len() {
                match self
                    .file
                    .seek_read(&mut out[filled..], offset + filled as u64)
                {
                    Ok(0) => {
                        return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
                    }
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(())
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.file.seek(SeekFrom::Start(offset))?;
            self.file.read_exact(out).map_err(Into::into)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[cfg(unix)]
    use std::io::{Seek, SeekFrom};
    use std::{format, vec::Vec};

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(name: &str) -> (Self, std::fs::File) {
            let dir = std::env::var_os("TMPDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::PathBuf::from(
                        std::env::var_os("HOME")
                            .or_else(|| std::env::var_os("USERPROFILE"))
                            .expect("home directory"),
                    )
                    .join("tmp")
                });
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("zencodec-file-{name}-{}", std::process::id()));
            let file = std::fs::OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            (Self(path), file)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_file(&self.0).unwrap();
        }
    }

    #[test]
    fn file_ranges_eof_overflow_and_truncation() {
        let (_fixture, mut file) = Fixture::new("bounds");
        let bytes: Vec<u8> = (0..=255).collect();
        file.write_all(&bytes).unwrap();
        let mut source = FileReadAt::new(file.try_clone().unwrap()).unwrap();
        let mut out = [0; 7];
        for at in [200, 4, 249, 0, 100] {
            source.read_exact_at(at, &mut out).unwrap();
            assert_eq!(out, bytes[at as usize..at as usize + 7]);
        }
        assert_eq!(source.read_at(253, &mut out).unwrap(), 3);
        assert_eq!(&out[..3], &[253, 254, 255]);
        assert_eq!(source.read_at(256, &mut out).unwrap(), 0);
        for at in [250, u64::MAX] {
            assert!(matches!(
                source.read_exact_at(at, &mut out),
                Err(StdSourceError::Source(SourceError::UnexpectedEof { .. }))
            ));
        }
        file.set_len(2).unwrap();
        assert!(source.read_exact_at(0, &mut out).is_err());
    }

    #[test]
    fn native_file_errors_retain_kind_and_os_code() {
        let (fixture, file) = Fixture::new("errors");
        let missing = fixture.0.with_extension("missing");
        assert!(!missing.exists());
        let error = FileReadAt::open(&missing).err().expect("missing file");
        let native = zencodec::find_cause::<std::io::Error>(&error).unwrap();
        assert_eq!(native.kind(), std::io::ErrorKind::NotFound);
        assert!(native.raw_os_error().is_some());

        // The source's length snapshot cannot hide a later native read failure.
        file.set_len(8).unwrap();
        let mut source = FileReadAt::new(file.try_clone().unwrap()).unwrap();
        file.set_len(0).unwrap();
        let error = source.read_exact_at(0, &mut [0; 8]).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<std::io::Error>(&error)
                .unwrap()
                .kind(),
            std::io::ErrorKind::UnexpectedEof
        );
    }

    #[cfg(unix)]
    #[test]
    fn positional_reads_preserve_a_shared_cursor_and_large_offsets() {
        let (_fixture, mut file) = Fixture::new("cursor");
        let offset = (1u64 << 32) + 123;
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(b"large-offset").unwrap();
        file.seek(SeekFrom::Start(17)).unwrap();
        let mut source = FileReadAt::new(file.try_clone().unwrap()).unwrap();
        assert_eq!(
            file.stream_position().unwrap(),
            17,
            "construction moved the shared cursor"
        );
        let mut out = [0; 12];
        source.read_exact_at(offset, &mut out).unwrap();
        assert_eq!(&out, b"large-offset");
        source.read_exact_at(0, &mut out).unwrap();
        assert_eq!(out, [0; 12]);
        assert_eq!(file.stream_position().unwrap(), 17);
        // Independent sources sharing the OS handle may read different offsets
        // concurrently, with no seek/read pair to race.
        std::thread::scope(|scope| {
            for at in [0, offset] {
                let mut source = FileReadAt::new(file.try_clone().unwrap()).unwrap();
                scope.spawn(move || {
                    for _ in 0..1000 {
                        let mut out = [0; 12];
                        source.read_exact_at(at, &mut out).unwrap();
                        assert_eq!(&out, if at == 0 { &[0; 12] } else { b"large-offset" });
                    }
                });
            }
        });
        assert_eq!(file.stream_position().unwrap(), 17);
    }
}
