#![cfg(feature = "std")]

use std::io::{self, BufReader, Read, Seek, SeekFrom};
use zencodec::source::{ReadAt, SliceReadAt, SourceError};
use zencodec_io::{BufferPolicy, BufferedReadAt, ReadAtReader, StdSourceError};

#[test]
fn standard_helpers_match_cursor_across_seek_and_buffer_boundaries() {
    let bytes: Vec<u8> = (0..137).map(|n| (n * 19) as u8).collect();
    for capacity in [1, 3, 16, 4096] {
        let mut r = BufReader::with_capacity(capacity, ReadAtReader::new(SliceReadAt::new(&bytes)));
        let mut reference = io::Cursor::new(&bytes);
        for to in [
            SeekFrom::Start(0),
            SeekFrom::Start(15),
            SeekFrom::Current(-3),
            SeekFrom::End(-11),
            SeekFrom::End(3),
            SeekFrom::Start(0),
        ] {
            assert_eq!(r.seek(to).unwrap(), reference.seek(to).unwrap());
            let mut actual = [0; 7];
            let mut expected = [0; 7];
            assert_eq!(
                r.read(&mut actual).unwrap(),
                reference.read(&mut expected).unwrap()
            );
            assert_eq!(actual, expected);
            assert_eq!(
                r.stream_position().unwrap(),
                reference.stream_position().unwrap()
            );
        }
        r.rewind().unwrap();
        let mut collected = Vec::new();
        r.read_to_end(&mut collected).unwrap();
        assert_eq!(collected, bytes);
    }
}

#[test]
fn seek_does_not_spool_until_end_is_requested() {
    let source = BufferedReadAt::new(
        io::Cursor::new([1, 2, 3, 4]),
        BufferPolicy::Spool { max_bytes: 4 },
    );
    let mut reader = ReadAtReader::new(source);
    assert_eq!(reader.seek(SeekFrom::Start(3)).unwrap(), 3);
    assert_eq!(reader.stream_position().unwrap(), 3);
    assert_eq!(reader.read(&mut []).unwrap(), 0);
    let source = reader.into_inner();
    assert_eq!(source.watermark(), 0);
    let mut reader = ReadAtReader::new(source);
    assert_eq!(reader.seek(SeekFrom::End(-1)).unwrap(), 3);
    let mut last = [0];
    reader.read_exact(&mut last).unwrap();
    assert_eq!(last, [4]);
    assert_eq!(reader.into_inner().watermark(), 4);
}

#[derive(Debug)]
struct TransportGone;
impl std::fmt::Display for TransportGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("transport gone")
    }
}
impl std::error::Error for TransportGone {}

struct Fails<E>(Option<E>, usize);
impl<E: std::error::Error + Send + Sync + 'static> ReadAt for Fails<E> {
    type Error = E;
    fn len(&mut self) -> Result<u64, E> {
        self.1 += 1;
        Err(self.0.take().expect("bridge must not retry"))
    }
    fn read_exact_at(&mut self, _: u64, out: &mut [u8]) -> Result<(), E> {
        self.1 += 1;
        if let Some(v) = out.first_mut() {
            *v = 73;
        }
        Err(self.0.take().expect("bridge must not retry"))
    }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize, E> {
        self.read_exact_at(at, out).map(|_| out.len())
    }
}

#[test]
fn native_kind_os_code_and_custom_payload_survive_without_retry() {
    let mut direct = ReadAtReader::new(Fails(Some(io::Error::from_raw_os_error(2)), 0));
    let mut out = [0; 3];
    let error = direct.read(&mut out).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(2));
    assert_eq!(direct.stream_position().unwrap(), 0);
    assert_eq!(out, [73, 0, 0]);
    assert_eq!(direct.into_inner().1, 1);

    let native = io::Error::new(io::ErrorKind::Interrupted, TransportGone);
    let mut standard = ReadAtReader::new(Fails(Some(StdSourceError::Io(native)), 0));
    standard.seek(SeekFrom::Start(9)).unwrap();
    let error = standard.read(&mut out).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(error.get_ref().unwrap().is::<TransportGone>());
    assert!(zencodec::find_cause::<TransportGone>(&error).is_some());
    assert_eq!(standard.stream_position().unwrap(), 9);
    assert_eq!(standard.into_inner().1, 1);

    let mut custom = ReadAtReader::new(Fails(Some(TransportGone), 0));
    custom.seek(SeekFrom::Start(7)).unwrap();
    let error = custom.seek(SeekFrom::End(0)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(zencodec::find_cause::<TransportGone>(&error).is_some());
    assert_eq!(custom.stream_position().unwrap(), 7);
    assert_eq!(custom.into_inner().1, 1);
}

#[derive(Debug)]
struct Context(io::Error);
impl std::fmt::Display for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("range request failed")
    }
}
impl std::error::Error for Context {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn nested_transport_context_is_retained_and_standard_eof_is_reported() {
    let cause = Context(io::Error::new(
        io::ErrorKind::ConnectionReset,
        TransportGone,
    ));
    let mut reader = ReadAtReader::new(Fails(Some(cause), 0));
    let error = reader.read(&mut [0]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
    assert!(zencodec::find_cause::<Context>(&error).is_some());
    assert!(zencodec::find_cause::<TransportGone>(&error).is_some());
    assert_eq!(reader.into_inner().1, 1);

    let mut reader = ReadAtReader::new(SliceReadAt::new(&[1, 2]));
    let mut out = [0; 3];
    assert_eq!(
        reader.read_exact(&mut out).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(reader.stream_position().unwrap(), 2);
    assert_eq!(out, [1, 2, 0]);
}

#[test]
fn spool_refusal_retains_structural_cause_and_seek_position() {
    let source = BufferedReadAt::new(
        io::Cursor::new([1, 2, 3, 4]),
        BufferPolicy::Spool { max_bytes: 3 },
    );
    let mut reader = ReadAtReader::new(source);
    reader.seek(SeekFrom::Start(2)).unwrap();
    let error = reader.seek(SeekFrom::End(0)).unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<StdSourceError>(&error),
        Some(StdSourceError::Source(_))
    ));
    assert!(matches!(
        zencodec::find_cause::<SourceError>(&error),
        Some(SourceError::BeyondLimit { .. })
    ));
    assert_eq!(reader.stream_position().unwrap(), 2);
    assert!(reader.read(&mut [0]).is_err());
}

struct InvalidCount(usize);
impl ReadAt for InvalidCount {
    type Error = io::Error;
    fn len(&mut self) -> io::Result<u64> {
        panic!("unexpected length query")
    }
    fn read_exact_at(&mut self, _: u64, _: &mut [u8]) -> io::Result<()> {
        panic!("unexpected exact read")
    }
    fn read_at(&mut self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        Ok(self.0)
    }
}

#[test]
fn invalid_source_counts_and_offset_overflow_cannot_advance_the_cursor() {
    for (count, position) in [(3, 0), (1, u64::MAX)] {
        let mut reader = ReadAtReader::new(InvalidCount(count));
        reader.seek(SeekFrom::Start(position)).unwrap();
        assert_eq!(
            reader.read(&mut [0; 2]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(reader.stream_position().unwrap(), position);
        assert_eq!(reader.read(&mut []).unwrap(), 0);
    }
    let mut reader = ReadAtReader::new(InvalidCount(0));
    for to in [SeekFrom::Current(-1), SeekFrom::Current(i64::MIN)] {
        assert_eq!(
            reader.seek(to).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(reader.stream_position().unwrap(), 0);
    }
    reader.seek(SeekFrom::Start(u64::MAX)).unwrap();
    assert_eq!(
        reader.seek(SeekFrom::Current(1)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(reader.read(&mut [0]).unwrap(), 0);
    assert_eq!(reader.stream_position().unwrap(), u64::MAX);
}
