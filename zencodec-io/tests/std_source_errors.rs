#![cfg(feature = "std")]

use std::fmt;
use std::io::{self, Read};
use zencodec::source::{DynReadAt, ReadAt, ReadAtShim, SourceError};
use zencodec_io::{BufferPolicy, BufferedReadAt};

#[derive(Debug)]
struct Disconnected(u32);
impl fmt::Display for Disconnected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "connection {} closed", self.0)
    }
}
impl std::error::Error for Disconnected {}

struct FailsAfterPrefix {
    remaining: usize,
}
impl Read for FailsAfterPrefix {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                Disconnected(17),
            ));
        }
        let count = out.len().min(self.remaining);
        out[..count].fill(9);
        self.remaining -= count;
        Ok(count)
    }
}

#[test]
fn reader_cause_and_payload_survive_partial_input_and_dyn_erasure() {
    let mut source = BufferedReadAt::new(
        FailsAfterPrefix { remaining: 6 },
        BufferPolicy::Spool { max_bytes: 32 },
    );
    source.read_exact_at(0, &mut [0; 4]).unwrap();
    let error = source.read_exact_at(4, &mut [0; 4]).unwrap_err();
    assert_eq!(source.watermark(), 6);
    let native = zencodec::find_cause::<io::Error>(&error).unwrap();
    assert_eq!(native.kind(), io::ErrorKind::ConnectionReset);
    assert_eq!(
        native
            .get_ref()
            .unwrap()
            .downcast_ref::<Disconnected>()
            .unwrap()
            .0,
        17
    );

    let mut erased: Box<dyn DynReadAt> = Box::new(ReadAtShim(source));
    let error = erased.read_exact_at(6, &mut [0; 1]).unwrap_err();
    let native = zencodec::find_cause::<io::Error>(error.as_ref()).unwrap();
    assert_eq!(native.kind(), io::ErrorKind::ConnectionReset);
    assert_eq!(
        native
            .get_ref()
            .unwrap()
            .downcast_ref::<Disconnected>()
            .unwrap()
            .0,
        17
    );
}

#[test]
fn structural_refusals_remain_distinct_from_native_io() {
    let mut source = BufferedReadAt::new(
        io::Cursor::new([1, 2]),
        BufferPolicy::Spool { max_bytes: 8 },
    );
    let error = source.read_exact_at(0, &mut [0; 3]).unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<SourceError>(&error),
        Some(SourceError::UnexpectedEof {
            offset: 0,
            len: 3,
            total: Some(2)
        })
    ));
    assert!(zencodec::find_cause::<io::Error>(&error).is_none());
}

#[test]
fn erased_local_source_retains_borrowed_input_and_thread_local_reader_state() {
    use std::{cell::Cell, rc::Rc};
    struct LocalReader<'a> {
        input: io::Cursor<&'a [u8]>,
        reads: Rc<Cell<usize>>,
    }
    impl Read for LocalReader<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.reads.set(self.reads.get() + 1);
            self.input.read(out)
        }
    }
    let bytes = [2, 3, 5, 7, 11, 13];
    let reads = Rc::new(Cell::new(0));
    let reader = LocalReader {
        input: io::Cursor::new(&bytes),
        reads: reads.clone(),
    };
    let mut source: Box<dyn DynReadAt + '_> =
        BufferedReadAt::new(reader, BufferPolicy::Spool { max_bytes: 6 }).into();
    let mut first = [0; 2];
    let mut last = [0; 2];
    source
        .read_many_at(&mut [(4, &mut last), (0, &mut first)])
        .unwrap();
    assert_eq!(first, [2, 3]);
    assert_eq!(last, [11, 13]);
    let before = reads.get();
    source.read_exact_at(1, &mut first).unwrap();
    assert_eq!(first, [3, 5]);
    assert_eq!(
        reads.get(),
        before,
        "cached local read touched the reader again"
    );
    assert!(before > 0);
}

#[test]
fn explicitly_send_erased_source_can_borrow_input_across_a_scoped_thread() {
    use zencodec::source::SliceReadAt;
    let bytes = vec![2, 3, 5, 7, 11, 13];
    let expected = bytes.as_slice();
    let mut source: Box<dyn DynReadAt + Send + '_> = SliceReadAt::new(&bytes).into();
    let (first, last) = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let mut first = [0; 2];
                let mut last = [0; 2];
                source
                    .read_many_at(&mut [(0, &mut first), (4, &mut last)])
                    .unwrap();
                assert_eq!(source.as_slice(), Some(expected));
                (first, last)
            })
            .join()
            .unwrap()
    });
    assert_eq!(first, [2, 3]);
    assert_eq!(last, [11, 13]);
}
