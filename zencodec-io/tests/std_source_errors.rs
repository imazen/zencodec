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
