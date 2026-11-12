//! Shared test helpers.
#![allow(dead_code)]

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use zencodec::decode::{
    DecodeCodec, DecodeEvent, DecodePlan, DecodeSpec, DecodeStep, Decoder, Sink,
};
use zencodec::encode::{
    EncodeCodec, EncodeEvent, EncodeInput, EncodePlan, EncodeSpec, EncodeStep, Encoder,
    OutputBuffer,
};
use zencodec::simple::{DecodeRequest, WholeImageDecode};
use zencodec::{
    ErrorKind, FormatInfo, FrameHeader, ImageFormat, ImageInfo, Input, InputRequest, Layout,
    Metadata, MetadataKinds, Pass, Pulse, Result, SharedPulse, Stop, StopReason, error,
};
use zencodec_fixtures::{bias::BiasCodec, rle::RleCodec, rows::RowsCodec};
use zencodec_io::{Collect, Frame, RangeRead, ReadAt};
use zenpixels::plane::PlanarSlice;
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice, PixelSliceMut};

pub fn decoders() -> Vec<Arc<dyn DecodeCodec>> {
    vec![
        Arc::new(RowsCodec),
        Arc::new(RleCodec::codec()),
        Arc::new(BiasCodec),
    ]
}

pub fn encoders() -> Vec<Arc<dyn EncodeCodec>> {
    vec![
        Arc::new(RowsCodec),
        Arc::new(RleCodec::codec()),
        Arc::new(BiasCodec),
    ]
}

/// Tight sample bytes with a pattern that differs per row, column and channel
/// and never falls below 3, so the bias codec has a nonzero bias to get wrong.
pub fn pattern(width: u32, height: u32, descriptor: PixelDescriptor, seed: u8) -> Vec<u8> {
    let bpp = descriptor.bytes_per_pixel();
    let mut out = Vec::with_capacity(width as usize * height as usize * bpp);
    for y in 0..height {
        for x in 0..width {
            for c in 0..bpp as u32 {
                let v = (x * 7 + y * 13 + c * 29 + u32::from(seed) * 31) % 200;
                out.push(v as u8 + 3);
            }
        }
    }
    out
}

/// The same samples in rows `pad` bytes wider than they need to be.
pub fn padded(
    tight: &[u8],
    width: u32,
    height: u32,
    descriptor: PixelDescriptor,
    pad: usize,
) -> (Vec<u8>, usize) {
    let row = width as usize * descriptor.bytes_per_pixel();
    let stride = row + pad;
    let mut out = vec![0xEE; stride * height as usize];
    for y in 0..height as usize {
        out[y * stride..y * stride + row].copy_from_slice(&tight[y * row..(y + 1) * row]);
    }
    (out, stride)
}

pub fn slice<'a>(
    bytes: &'a [u8],
    width: u32,
    height: u32,
    stride: usize,
    descriptor: PixelDescriptor,
) -> PixelSlice<'a> {
    PixelSlice::new(bytes, width, height, stride, descriptor).expect("a valid slice")
}

/// A frame's samples as tight bytes.
pub fn tight(buffer: &PixelBuffer) -> Vec<u8> {
    let view = buffer.as_slice();
    let row = view.width() as usize * view.descriptor().bytes_per_pixel();
    (0..view.rows())
        .flat_map(|y| view.row(y)[..row].to_vec())
        .collect()
}

pub fn packed(frame: &Frame) -> Vec<u8> {
    tight(frame.pixels.packed().expect("a packed frame"))
}

/// A source that hands out at most `chunk` bytes per read and counts its calls.
pub struct Drip<'a> {
    pub bytes: &'a [u8],
    pub chunk: usize,
    pub reads: usize,
    pub range_calls: usize,
}

impl<'a> Drip<'a> {
    pub fn new(bytes: &'a [u8], chunk: usize) -> Self {
        Self {
            bytes,
            chunk,
            reads: 0,
            range_calls: 0,
        }
    }
}

impl ReadAt for Drip<'_> {
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        let from = (offset as usize).min(self.bytes.len());
        let n = out.len().min(self.chunk).min(self.bytes.len() - from);
        out[..n].copy_from_slice(&self.bytes[from..from + n]);
        Ok(n)
    }

    fn total_len(&mut self) -> io::Result<Option<u64>> {
        Ok(Some(self.bytes.len() as u64))
    }

    fn read_ranges(&mut self, ranges: &mut [RangeRead]) -> io::Result<()> {
        self.range_calls += 1;
        for range in ranges {
            let from = (range.offset as usize).min(self.bytes.len());
            let n = range.buf.len().min(self.bytes.len() - from);
            range.buf.truncate(n);
            range.buf.copy_from_slice(&self.bytes[from..from + n]);
        }
        Ok(())
    }
}

/// A sink that records every call before passing it on.
#[derive(Default)]
pub struct Recording {
    pub inner: Collect,
    pub log: Vec<&'static str>,
    /// Lend at most this many rows at a time; 0 lends what is asked.
    pub max_loan: u32,
}

impl Sink for Recording {
    fn begin(&mut self, frame: &FrameHeader) -> Result<()> {
        self.log.push("begin");
        self.inner.begin(frame)
    }
    fn lend(&mut self, y: u32, rows: u32, pass: Pass) -> Result<PixelSliceMut<'_>> {
        self.log.push("lend");
        let rows = if self.max_loan == 0 {
            rows
        } else {
            rows.min(self.max_loan)
        };
        self.inner.lend(y, rows, pass)
    }
    fn frame(&mut self, pixels: PixelBuffer) -> Result<()> {
        self.log.push("frame");
        self.inner.frame(pixels)
    }
    fn planes(&mut self, y: u32, strip: PlanarSlice<'_>) -> Result<()> {
        self.log.push("planes");
        self.inner.planes(y, strip)
    }
    fn end(&mut self) -> Result<()> {
        self.log.push("end");
        self.inner.end()
    }
}

/// A stop switch for tests.
#[derive(Clone, Default)]
pub struct Switch {
    stopped: Arc<AtomicBool>,
    pub checks: Arc<AtomicUsize>,
    /// Stop on the nth check, counted from 1; 0 never trips by itself.
    trip_at: usize,
}

impl Switch {
    pub fn tripping_at(check: usize) -> Self {
        Self {
            trip_at: check,
            ..Self::default()
        }
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
    /// An owned pulse, for work that leaves this thread.
    pub fn pulse(&self) -> SharedPulse {
        SharedPulse::new(how_far::StopOnly::new(self.clone()))
    }
    /// A pulse that borrows the switch. It cannot be shared.
    pub fn borrowed(&self) -> how_far::StopOnly<'_> {
        how_far::StopOnly::borrowed(self)
    }
}

impl Stop for Switch {
    fn check(&self) -> std::result::Result<(), StopReason> {
        let n = self.checks.fetch_add(1, Ordering::SeqCst) + 1;
        if self.stopped.load(Ordering::SeqCst) || (self.trip_at != 0 && n >= self.trip_at) {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// A decoder that never looks at its pulse, and under which the test raises
/// a stop in the middle of a step: when the wrapped decoder reports `on`,
/// the switch is thrown and the step still returns success.
pub struct Deaf {
    pub inner: Box<dyn Decoder>,
    pub switch: Switch,
    pub on: DecodeEvent,
}

impl Deaf {
    pub fn over(codec: &dyn DecodeCodec, switch: &Switch, on: DecodeEvent) -> Box<dyn Decoder> {
        Box::new(Self {
            inner: codec.decoder(&DecodeSpec::new()).expect("admitted"),
            switch: switch.clone(),
            on,
        })
    }
}

impl Decoder for Deaf {
    fn requested(&self) -> &[InputRequest] {
        self.inner.requested()
    }
    fn info(&self) -> Option<&ImageInfo> {
        self.inner.info()
    }
    fn plan(&self) -> Option<&DecodePlan> {
        self.inner.plan()
    }
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn Sink,
        _: Option<&dyn Pulse>,
    ) -> Result<DecodeStep> {
        let step = self.inner.step(input, sink, None)?;
        if step.event == self.on {
            self.switch.stop();
        }
        Ok(step)
    }
}

/// An encoder codec that never looks at its pulse; the switch is thrown
/// during the step that completes the output.
pub struct DeafCodec {
    pub inner: Arc<dyn EncodeCodec>,
    pub switch: Switch,
}

impl EncodeCodec for DeafCodec {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn format(&self) -> ImageFormat {
        self.inner.format()
    }
    fn embeds(&self) -> MetadataKinds {
        self.inner.embeds()
    }
    fn encoder(&self, spec: &EncodeSpec) -> Result<Box<dyn Encoder>> {
        Ok(Box::new(DeafEncoder {
            inner: self.inner.encoder(spec)?,
            switch: self.switch.clone(),
        }))
    }
}

struct DeafEncoder {
    inner: Box<dyn Encoder>,
    switch: Switch,
}

impl Encoder for DeafEncoder {
    fn plan(&self) -> &EncodePlan {
        self.inner.plan()
    }
    fn step(&mut self, input: EncodeInput<'_>, _: Option<&dyn Pulse>) -> Result<EncodeStep> {
        let step = self.inner.step(input, None)?;
        if step.event == EncodeEvent::Done {
            self.switch.stop();
        }
        Ok(step)
    }
    fn output(&mut self) -> &mut OutputBuffer {
        self.inner.output()
    }
}

static TAIL_INFO: FormatInfo = FormatInfo::new("tail", "image/x-tail", "tail", 4, |b| {
    b.starts_with(b"TAIL")
});

/// The `TAIL` test format: `"TAIL" width:u8 height:u8`, gray rows, then a
/// two-byte EXIF block after the pixels.
pub const TAIL: ImageFormat = ImageFormat::new(&TAIL_INFO);

/// A whole-image decoder whose EXIF sits after the pixels, so a probe of a
/// prefix cannot settle it. It counts its probes, and `decode` works only
/// from what `probe` kept.
#[derive(Clone)]
pub struct Tail {
    pub probes: Arc<AtomicUsize>,
    /// Whether a probe of the whole file settles EXIF. `false` models a
    /// codec that never parses it.
    pub settles: bool,
}

impl Tail {
    pub fn new(settles: bool) -> Self {
        Self {
            probes: Arc::default(),
            settles,
        }
    }

    pub fn file(width: u8, height: u8) -> (Vec<u8>, Vec<u8>) {
        let pixels = pattern(width.into(), height.into(), PixelDescriptor::GRAY8_SRGB, 23);
        let mut file = b"TAIL".to_vec();
        file.extend_from_slice(&[width, height]);
        file.extend_from_slice(&pixels);
        file.extend_from_slice(b"EX");
        (file, pixels)
    }
}

impl WholeImageDecode for Tail {
    /// Width and height, parsed once.
    type Header = (u8, u8);

    fn name(&self) -> &'static str {
        "tail"
    }

    fn formats(&self) -> &[ImageFormat] {
        &[TAIL]
    }

    fn probe(&self, bytes: &[u8]) -> Result<Option<(ImageInfo, (u8, u8))>> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        if bytes.len() < 6 {
            return Ok(None);
        }
        let (width, height) = (bytes[4], bytes[5]);
        let total = 6 + usize::from(width) * usize::from(height) + 2;
        let mut info = ImageInfo::new(
            TAIL,
            width.into(),
            height.into(),
            Layout::Packed(PixelDescriptor::GRAY8_SRGB),
        );
        info = if self.settles && bytes.len() >= total {
            info.with_metadata(
                Metadata::none().with_exif(bytes[total - 2..total].to_vec()),
                MetadataKinds::ALL,
            )
        } else {
            info.with_metadata(
                Metadata::none(),
                MetadataKinds::ALL.difference(MetadataKinds::EXIF),
            )
        };
        Ok(Some((info, (width, height))))
    }

    fn decode(&self, request: DecodeRequest<'_, (u8, u8)>) -> Result<PixelBuffer> {
        let (width, height) = request.header;
        let len = usize::from(width) * usize::from(height);
        PixelBuffer::from_vec(
            request.bytes[6..6 + len].to_vec(),
            width.into(),
            height.into(),
            request.output,
        )
        .map_err(|_| error(ErrorKind::Internal))
    }
}
