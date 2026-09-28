use std::io::{self, Cursor, Read, Write};
use zencodec_media::ivf::{IvfReader, IvfWriter};
use zencodec_media::time::{TimeBase, Timestamp};

const FIXTURE: &[u8] = include_bytes!("../corpus/av1/av1-10-420-full-17x13.ivf");

fn declared_fixture() -> Vec<u8> {
    // Native streaming output deliberately has an unknown frame count. Add the
    // known count explicitly when testing the seekable-file declaration.
    let mut bytes = FIXTURE.to_vec();
    bytes[24..28].copy_from_slice(&4_u32.to_le_bytes());
    bytes
}

struct Fragmented<'a> {
    data: &'a [u8],
    reads: usize,
}
impl Read for Fragmented<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.reads.is_multiple_of(7) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let n = output.len().min(self.data.len()).min(1 + self.reads % 11);
        output[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

#[test]
fn short_network_reads_match_direct_reads_and_writer_never_seeks() {
    let fixture = declared_fixture();
    let mut direct = IvfReader::new(Cursor::new(&fixture), 1 << 20).unwrap();
    let mut fragmented = IvfReader::new(
        Fragmented {
            data: &fixture,
            reads: 0,
        },
        1 << 20,
    )
    .unwrap();
    let info = fragmented.info();
    assert_eq!(
        (info.width(), info.height(), info.declared_frames()),
        (17, 13, Some(4))
    );
    assert_eq!(info.time_base(), TimeBase::new(1001, 30000).unwrap());
    // Vec implements Write; the writer imposes no Seek requirement.
    let mut writer = IvfWriter::new(Vec::new(), 17, 13, info.time_base()).unwrap();
    let mut offset = 32;
    for i in 0..4 {
        let a = direct.next_packet().unwrap().unwrap();
        let b = fragmented.next_packet().unwrap().unwrap();
        assert_eq!(a.data, b.data);
        assert_eq!(b.byte_offset, offset + 12);
        assert_eq!(b.timestamp.ticks(), i);
        assert_eq!(b.ordinal, i as u64);
        writer.write_packet(&b.data, b.timestamp).unwrap();
        offset += 12 + b.data.len() as u64;
    }
    assert!(fragmented.next_packet().unwrap().is_none());
    assert!(fragmented.next_packet().unwrap().is_none());
    assert_eq!(writer.packets_written(), 4);
    let encoded = writer.finish().unwrap();
    assert_eq!(&encoded[32..], &FIXTURE[32..]);
    let mut readback = IvfReader::new(encoded.as_slice(), 1 << 20).unwrap();
    assert_eq!(readback.info().declared_frames(), None);
    let mut count = 0;
    while readback.next_packet().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 4);
}

#[test]
fn every_truncation_is_an_error_when_a_frame_count_is_declared() {
    let fixture = declared_fixture();
    for end in 0..fixture.len() {
        let Ok(mut reader) = IvfReader::new(&fixture[..end], 1 << 20) else {
            continue;
        };
        let error = loop {
            match reader.next_packet() {
                Ok(Some(_)) => {}
                Ok(None) => panic!("truncated input accepted at byte {end}"),
                Err(error) => break error,
            }
        };
        assert!(matches!(
            error.kind(),
            io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData
        ));
        assert!(
            reader.next_packet().is_err(),
            "partial input must poison the parser"
        );
    }
}

#[test]
fn length_limits_apply_before_reading_or_allocating_a_payload() {
    struct HeaderOnly {
        data: Cursor<Vec<u8>>,
        beyond: bool,
    }
    impl Read for HeaderOnly {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.data.position() == 44 {
                self.beyond = true;
                panic!("read oversized payload");
            }
            self.data.read(buf)
        }
    }
    let mut header = FIXTURE[..44].to_vec();
    header[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    let input = HeaderOnly {
        data: Cursor::new(header),
        beyond: false,
    };
    let mut reader = IvfReader::new(input, 4096).unwrap();
    assert_eq!(
        reader.next_packet().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!reader.into_inner().beyond);
}

#[test]
fn extended_headers_and_invalid_clocks_are_not_misinterpreted() {
    let mut bytes = FIXTURE.to_vec();
    bytes.splice(32..32, [1, 2, 3, 4, 5]);
    bytes[6..8].copy_from_slice(&37_u16.to_le_bytes());
    let mut reader = IvfReader::new(bytes.as_slice(), 1 << 20).unwrap();
    assert_eq!(reader.next_packet().unwrap().unwrap().byte_offset, 49);
    for range in [16..20, 20..24] {
        let mut invalid = FIXTURE.to_vec();
        invalid[range].fill(0);
        assert!(IvfReader::new(invalid.as_slice(), 1 << 20).is_err());
    }
}

struct FailingSink {
    bytes: usize,
    fail_at: usize,
    fail_flush: bool,
}
impl Write for FailingSink {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.bytes == self.fail_at {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let n = data.len().min(self.fail_at - self.bytes).min(3);
        self.bytes += n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            Err(io::ErrorKind::BrokenPipe.into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn sink_failures_poison_output_and_finalization_errors_surface() {
    let clock = TimeBase::new(1, 25).unwrap();
    let mut writer = IvfWriter::new(
        FailingSink {
            bytes: 0,
            fail_at: 45,
            fail_flush: false,
        },
        17,
        13,
        clock,
    )
    .unwrap();
    assert_eq!(
        writer
            .write_packet(&[1, 2, 3], Timestamp::new(0, clock))
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(writer.packets_written(), 0);
    assert!(writer.write_packet(&[1], Timestamp::new(1, clock)).is_err());
    assert!(writer.finish().is_err());
    let writer = IvfWriter::new(
        FailingSink {
            bytes: 0,
            fail_at: usize::MAX,
            fail_flush: true,
        },
        17,
        13,
        clock,
    )
    .unwrap();
    assert!(matches!(writer.finish(),Err(error) if error.kind()==io::ErrorKind::BrokenPipe));
}

#[test]
fn mismatched_output_clock_is_rejected_before_any_packet_bytes() {
    let clock = TimeBase::new(1, 25).unwrap();
    let mut writer = IvfWriter::new(Vec::new(), 17, 13, clock).unwrap();
    assert!(
        writer
            .write_packet(&[1], Timestamp::new(0, TimeBase::new(1, 30).unwrap()))
            .is_err()
    );
    assert!(writer.write_packet(&[], Timestamp::new(0, clock)).is_err());
    assert_eq!(writer.finish().unwrap().len(), 32);
}

#[test]
fn ffmpeg_unknown_frame_count_and_signed_timestamps_are_preserved() {
    let mut bytes = FIXTURE.to_vec();
    bytes[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[36..44].copy_from_slice(&(-10_i64).to_le_bytes());
    let mut reader = IvfReader::new(bytes.as_slice(), 1 << 20).unwrap();
    assert_eq!(reader.info().declared_frames(), None);
    assert_eq!(
        reader.next_packet().unwrap().unwrap().timestamp.ticks(),
        -10
    );
    for _ in 1..4 {
        assert!(reader.next_packet().unwrap().is_some());
    }
    assert!(reader.next_packet().unwrap().is_none());
    let clock = TimeBase::new(1, 25).unwrap();
    let mut writer = IvfWriter::new(Vec::new(), 17, 13, clock).unwrap();
    for ticks in [i64::MIN, -1, 0, i64::MAX] {
        writer
            .write_packet(&[1], Timestamp::new(ticks, clock))
            .unwrap();
    }
    let bytes = writer.finish().unwrap();
    let mut reader = IvfReader::new(bytes.as_slice(), 16).unwrap();
    for ticks in [i64::MIN, -1, 0, i64::MAX] {
        assert_eq!(
            reader.next_packet().unwrap().unwrap().timestamp.ticks(),
            ticks
        );
    }
}
