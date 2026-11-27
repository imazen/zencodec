//! `write_pending` and `take_output` against a small streaming encoder.

use zencodec::encode::{EncodeOutput, Encoder, OutputBuffer, OutputTiming};
use zencodec::{ImageFormat, UnsupportedOperation};
use zencodec_io::write_pending;
use zenpixels::{PixelFormat, PixelView};

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

/// A 4-byte header of width and height, then the rows. Streams with lag 0.
struct Raw {
    out: OutputBuffer,
    begun: bool,
}

impl Raw {
    fn new() -> Self {
        Self {
            out: OutputBuffer::new(),
            begun: false,
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
    fn encode(self, pixels: PixelView<'_>) -> Result<EncodeOutput, EncErr> {
        if self.begun {
            return Err(EncErr::Invalid);
        }
        let mut v = header(pixels.width(), pixels.height()).to_vec();
        for y in 0..pixels.height() {
            v.extend_from_slice(pixels.row(y));
        }
        Ok(EncodeOutput::new(v, ImageFormat::Pnm))
    }
    fn begin(&mut self, w: u32, h: u32, _d: PixelFormat) -> Result<OutputTiming, EncErr> {
        self.begun = true;
        self.out.push(&header(w, h));
        Ok(OutputTiming::streaming(0))
    }
    fn push_rows(&mut self, rows: PixelView<'_>) -> Result<(), EncErr> {
        for y in 0..rows.height() {
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

#[cfg(feature = "tokio")]
fn image(w: u32, h: u32) -> Vec<u8> {
    (0..w * h * 4).map(|i| (i * 13 % 251) as u8).collect()
}

#[cfg(feature = "tokio")]
fn slice(data: &[u8], w: u32, h: u32) -> PixelView<'_> {
    PixelView::new(data, w, h, w as usize * 4, &zenpixels::formats::RGBA8_SRGB).unwrap()
}

#[test]
fn write_pending_consumes_what_it_writes() {
    let mut enc = Raw::new();
    enc.begin(4, 4, PixelFormat::RGBA8_SRGB).unwrap();
    let mut out = Vec::new();
    assert_eq!(write_pending(&mut enc, &mut out).unwrap(), 4);
    assert!(enc.pending_output().is_empty());
    assert_eq!(write_pending(&mut enc, &mut out).unwrap(), 0);
    assert_eq!(out.len(), 4);
}

#[test]
fn take_output_hands_over_the_allocation() {
    let mut enc = Raw::new();
    enc.begin(4, 4, PixelFormat::RGBA8_SRGB).unwrap();
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
    let mut enc = Raw::new();
    enc.begin(5, 20, PixelFormat::RGBA8_SRGB).unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut n = zencodec_io::tokio_io::write_pending_async(&mut enc, &mut out)
        .await
        .unwrap();
    for y in (0..20).step_by(7) {
        let h = 7.min(20 - y);
        enc.push_rows(slice(&px, 5, 20).sub_rows(y, h).unwrap())
            .unwrap();
        n += zencodec_io::tokio_io::write_pending_async(&mut enc, &mut out)
            .await
            .unwrap();
    }
    assert!(enc.pending_output().is_empty());
    out.extend_from_slice(enc.finish().unwrap().data());
    assert_eq!(n, out.len());
    assert_eq!(out, Raw::new().encode(slice(&px, 5, 20)).unwrap().data());
}
