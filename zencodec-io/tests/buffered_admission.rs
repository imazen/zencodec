#![cfg(feature = "std")]

use std::io::{self, Read};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec::source::{ReadAt, SourceError};
use zencodec_io::{BufferPolicy, BufferedReadAt};

struct Counted {
    inner: io::Cursor<Vec<u8>>,
    reads: Arc<AtomicUsize>,
    bytes: Arc<AtomicUsize>,
}
impl Read for Counted {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let n = self.inner.read(out)?;
        self.bytes.fetch_add(n, Ordering::Relaxed);
        Ok(n)
    }
}
fn source(
    len: usize,
    policy: BufferPolicy,
) -> (BufferedReadAt<Counted>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let reads = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicUsize::new(0));
    let reader = Counted {
        inner: io::Cursor::new((0..len).map(|n| n as u8).collect()),
        reads: reads.clone(),
        bytes: bytes.clone(),
    };
    (BufferedReadAt::new(reader, policy), reads, bytes)
}

#[test]
fn an_impossible_exact_spool_request_does_not_read_or_advance() {
    let (mut source, reads, bytes) = source(32, BufferPolicy::Spool { max_bytes: 8 });
    let error = source.read_exact_at(0, &mut [0; 9]).unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<SourceError>(&error),
        Some(SourceError::BeyondLimit {
            needed: 9,
            limit: 8
        })
    ));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(bytes.load(Ordering::Relaxed), 0);
    assert_eq!(source.watermark(), 0);
    let mut admitted = [0; 8];
    source.read_exact_at(0, &mut admitted).unwrap();
    assert_eq!(admitted, [0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn exact_cap_eof_succeeds_and_excess_probe_is_terminal() {
    for cap in [0, 1, 7, 32] {
        let (mut exact, _, bytes) = source(cap, BufferPolicy::Spool { max_bytes: cap });
        assert_eq!(exact.len().unwrap(), cap as u64);
        assert_eq!(exact.len_hint(), Some(cap as u64));
        assert_eq!(bytes.load(Ordering::Relaxed), cap);
        assert_eq!(exact.len().unwrap(), cap as u64);

        let (mut excess, reads, bytes) = source(cap + 2, BufferPolicy::Spool { max_bytes: cap });
        assert!(excess.len().is_err());
        assert_eq!(bytes.load(Ordering::Relaxed), cap + 1);
        assert_eq!(excess.watermark(), (cap + 1) as u64);
        let before = reads.load(Ordering::Relaxed);
        assert!(excess.len().is_err());
        assert!(excess.read_at(0, &mut [0; 1]).is_err());
        assert_eq!(reads.load(Ordering::Relaxed), before);
    }
}

#[test]
fn overflow_is_rejected_without_consumption() {
    let (mut source, reads, _) = source(32, BufferPolicy::Window { window_bytes: 8 });
    assert!(source.read_at(u64::MAX, &mut [0; 1]).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
