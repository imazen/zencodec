use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    sync::atomic::{AtomicBool, Ordering},
};
use zencodec_media::source::{SeekableSource, SourceError, read_bounded};

struct Fragments<R> {
    inner: R,
    chunk: usize,
    interrupted: bool,
}
impl<R: Read> Read for Fragments<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(io::ErrorKind::Interrupted.into());
        }
        self.interrupted = false;
        let n = out.len().min(self.chunk);
        self.inner.read(&mut out[..n])
    }
}

#[test]
fn source_limits_are_exact_and_excess_consumption_is_at_most_one_byte() {
    let bytes: Vec<_> = (0..100_000).map(|i| (i % 251) as u8).collect();
    for limit in [0, 1, 100, 32768, 99999, 100000, 100001] {
        let mut reader = Fragments {
            inner: Cursor::new(&bytes),
            chunk: 7,
            interrupted: false,
        };
        let result = read_bounded(&mut reader, limit, None);
        if limit < bytes.len() {
            assert!(matches!(result, Err(SourceError::Limit)));
            assert_eq!(reader.inner.position(), limit as u64 + 1);
        } else {
            assert_eq!(result.unwrap(), bytes);
        }
    }
    assert_eq!(read_bounded(&[][..], 0, None).unwrap(), Vec::<u8>::new());
}

#[test]
fn memory_and_disk_snapshots_support_identical_seeking_and_cleanup() {
    let root = std::env::var_os("TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/var/tmp"));
    let dir = tempfile::tempdir_in(root).unwrap();
    let bytes: Vec<_> = (0..80_001)
        .map(|i| ((i * 53 + i / 37) % 256) as u8)
        .collect();
    for memory in [0, 1, 32768, 80000, 80001, 100000] {
        let input = Fragments {
            inner: bytes.as_slice(),
            chunk: 13,
            interrupted: false,
        };
        let mut snapshot =
            SeekableSource::read_from(input, memory, bytes.len() as u64, dir.path(), None).unwrap();
        assert_eq!(snapshot.len(), bytes.len() as u64);
        assert_eq!(snapshot.is_on_disk(), memory < bytes.len());
        assert!(!snapshot.is_empty());
        assert_eq!(snapshot.stream_position().unwrap(), 0);
        for position in [0, 79999, 40000, 1, 80000] {
            snapshot.seek(SeekFrom::Start(position)).unwrap();
            let mut b = [0];
            snapshot.read_exact(&mut b).unwrap();
            assert_eq!(b[0], bytes[position as usize]);
        }
        snapshot.seek(SeekFrom::End(-7)).unwrap();
        let mut end = Vec::new();
        snapshot.read_to_end(&mut end).unwrap();
        assert_eq!(end, &bytes[bytes.len() - 7..]);
        assert_eq!(snapshot.into_bytes(bytes.len(), None).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    let snapshot =
        SeekableSource::read_from(bytes.as_slice(), 0, bytes.len() as u64, dir.path(), None)
            .unwrap();
    assert!(matches!(
        snapshot.into_bytes(bytes.len() - 1, None),
        Err(SourceError::Limit)
    ));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let empty = SeekableSource::read_from(&[][..], 0, 0, dir.path(), None).unwrap();
    assert!(empty.is_empty());
    assert!(!empty.is_on_disk());
}

#[test]
fn spool_errors_and_cancellation_discard_partial_sources() {
    struct Token(AtomicBool);
    impl enough::Stop for Token {
        fn check(&self) -> Result<(), enough::StopReason> {
            if self.0.load(Ordering::Relaxed) {
                Err(enough::StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }
    struct CancelRead<'a>(&'a Token);
    impl Read for CancelRead<'_> {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            b[0] = 42;
            self.0.0.store(true, Ordering::Relaxed);
            Ok(1)
        }
    }
    let token = Token(AtomicBool::new(false));
    assert!(matches!(
        read_bounded(CancelRead(&token), 100, Some(&token)),
        Err(SourceError::Cancelled)
    ));
    struct Fails;
    impl Read for Fails {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::TimedOut.into())
        }
    }
    assert!(
        matches!(read_bounded(Fails,100,None),Err(SourceError::Io(e)) if e.kind()==io::ErrorKind::TimedOut)
    );
    let missing = std::path::Path::new("/definitely-missing-zencodec-media-spool-directory");
    assert!(matches!(
        SeekableSource::read_from(&[1, 2][..], 0, 2, missing, None),
        Err(SourceError::Io(_))
    ));
    // No disk access when the actual input fits memory.
    let snapshot = SeekableSource::read_from(&[1, 2][..], 2, 2, missing, None).unwrap();
    assert_eq!(snapshot.into_bytes(2, None).unwrap(), [1, 2]);
}
