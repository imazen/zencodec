//! `encode_to_writer` and `write_pending` against a small streaming encoder.

use std::io::{self, Write};

use zencodec::encode::{EncodeOutput, Encoder, OutputBuffer, OutputTiming};
use zencodec::{ImageFormat, UnsupportedOperation};
use zencodec_io::{EncodeWriteError, encode_to_writer, write_pending};
use zenpixels::{PixelDescriptor, PixelSlice};

#[derive(Debug)]
enum EncErr {
    Unsupported(UnsupportedOperation),
    Invalid,
}
impl std::fmt::Display for EncErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EncErr {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unsupported(op) => Some(op),
            Self::Invalid => None,
        }
    }
}

/// A 4-byte header of width and height, then the rows. Streams with lag 0,
/// unless `decline` makes `begin` refuse row-level encode.
struct Raw {
    out: OutputBuffer,
    begun: bool,
    decline: bool,
    strips: u32,
}

impl Raw {
    fn new(decline: bool) -> Self {
        Self {
            out: OutputBuffer::new(),
            begun: false,
            decline,
            strips: 0,
        }
    }
}

fn header(w: u32, h: u32) -> [u8; 4] {
    [w as u8, (w >> 8) as u8, h as u8, (h >> 8) as u8]
}

impl Encoder for Raw {
    type Error = EncErr;
    fn reject(op: UnsupportedOperation) -> EncErr {
        EncErr::Unsupported(op)
    }
    fn preferred_strip_height(&self) -> u32 {
        1
    }
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, EncErr> {
        if self.begun {
            return Err(EncErr::Invalid);
        }
        let mut v = header(pixels.width(), pixels.rows()).to_vec();
        for y in 0..pixels.rows() {
            v.extend_from_slice(pixels.row(y));
        }
        Ok(EncodeOutput::new(v, ImageFormat::Pnm))
    }
    fn begin(&mut self, w: u32, h: u32, _d: PixelDescriptor) -> Result<OutputTiming, EncErr> {
        if self.decline {
            return Err(EncErr::Unsupported(UnsupportedOperation::RowLevelEncode));
        }
        self.begun = true;
        self.out.push(&header(w, h));
        Ok(OutputTiming::streaming(0))
    }
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), EncErr> {
        self.strips += 1;
        for y in 0..rows.rows() {
            self.out.push(rows.row(y));
        }
        Ok(())
    }
    fn pending_output(&self) -> &[u8] {
        self.out.pending()
    }
    fn consume_output(&mut self, n: usize) {
        self.out.consume(n);
    }
    fn take_output(&mut self, into: &mut Vec<u8>) {
        self.out.take_into(into);
    }
    fn finish(self) -> Result<EncodeOutput, EncErr> {
        Ok(EncodeOutput::new(
            self.out.into_remaining(),
            ImageFormat::Pnm,
        ))
    }
}

fn image(w: u32, h: u32) -> Vec<u8> {
    (0..w * h * 4).map(|i| (i * 13 % 251) as u8).collect()
}

fn slice(data: &[u8], w: u32, h: u32) -> PixelSlice<'_> {
    PixelSlice::new(data, w, h, w as usize * 4, PixelDescriptor::RGBA8_SRGB).unwrap()
}

/// Records the size of each write.
#[derive(Default)]
struct Recorder {
    data: Vec<u8>,
    writes: Vec<usize>,
}
impl Write for Recorder {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.data.extend_from_slice(buf);
        self.writes.push(buf.len());
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn streams_in_strips_and_matches_one_shot() {
    let px = image(5, 70);
    let one = Raw::new(false).encode(slice(&px, 5, 70)).unwrap();
    let mut w = Recorder::default();
    let report = encode_to_writer(Raw::new(false), slice(&px, 5, 70), &mut w).unwrap();
    assert_eq!(report.timing, OutputTiming::streaming(0));
    assert_eq!(report.bytes, w.data.len() as u64);
    assert_eq!(w.data, one.data());
    // Header first, then strips of 16 rows (a one-row preference rounds up):
    // 16, 16, 16, 16 and 6 rows.
    assert_eq!(w.writes[0], 4);
    assert_eq!(
        &w.writes[1..6],
        &[16 * 20, 16 * 20, 16 * 20, 16 * 20, 6 * 20]
    );
}

#[test]
fn a_declined_begin_falls_back_to_one_shot() {
    let px = image(3, 9);
    let mut w = Recorder::default();
    let report = encode_to_writer(Raw::new(true), slice(&px, 3, 9), &mut w).unwrap();
    assert_eq!(report.timing, OutputTiming::AT_FINISH);
    assert_eq!(w.writes.len(), 1);
    assert_eq!(
        w.data,
        Raw::new(false).encode(slice(&px, 3, 9)).unwrap().data()
    );
}

/// Fails after accepting `left` bytes.
struct Failing {
    left: usize,
}
impl Write for Failing {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::other("closed"));
        }
        let n = buf.len().min(self.left);
        self.left -= n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_write_error_surfaces_as_io() {
    let px = image(5, 70);
    let err = encode_to_writer(
        Raw::new(false),
        slice(&px, 5, 70),
        &mut Failing { left: 100 },
    )
    .unwrap_err();
    assert!(matches!(err, EncodeWriteError::Io(_)), "{err}");
}

#[test]
fn write_pending_consumes_what_it_writes() {
    let mut enc = Raw::new(false);
    enc.begin(4, 4, PixelDescriptor::RGBA8_SRGB).unwrap();
    let mut out = Vec::new();
    assert_eq!(write_pending(&mut enc, &mut out).unwrap(), 4);
    assert!(enc.pending_output().is_empty());
    assert_eq!(write_pending(&mut enc, &mut out).unwrap(), 0);
    assert_eq!(out.len(), 4);
}

#[test]
fn take_output_hands_over_the_allocation() {
    let mut enc = Raw::new(false);
    enc.begin(4, 4, PixelDescriptor::RGBA8_SRGB).unwrap();
    let ptr = enc.pending_output().as_ptr();
    let mut got = Vec::new();
    enc.take_output(&mut got);
    assert_eq!(got.as_ptr(), ptr);
    assert_eq!(got, header(4, 4));
    assert!(enc.pending_output().is_empty());
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn write_pending_async_matches_the_blocking_drain() {
    let px = image(5, 20);
    let mut enc = Raw::new(false);
    enc.begin(5, 20, PixelDescriptor::RGBA8_SRGB).unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut n = zencodec_io::tokio_io::write_pending_async(&mut enc, &mut out)
        .await
        .unwrap();
    for y in (0..20).step_by(7) {
        let h = 7.min(20 - y);
        enc.push_rows(slice(&px, 5, 20).sub_rows(y, h)).unwrap();
        n += zencodec_io::tokio_io::write_pending_async(&mut enc, &mut out)
            .await
            .unwrap();
    }
    assert!(enc.pending_output().is_empty());
    out.extend_from_slice(enc.finish().unwrap().data());
    assert_eq!(n, out.len());
    assert_eq!(
        out,
        Raw::new(false).encode(slice(&px, 5, 20)).unwrap().data()
    );
}
