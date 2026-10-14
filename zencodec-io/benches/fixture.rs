//! A synthetic RGBA8 image whose decode is a memcpy per strip, written
//! three ways: direct sink writes, a hand-written step decoder, and
//! coroutines (sequential and range-addressed). Shared by the benches and
//! the allocation audit.

#![allow(dead_code)]

use zencodec::decode::{
    BatchMeta, DecodeRowSink, Event, IncrementalDecode, Input, MetadataUpdate, OutputInfo,
    Progress, Request, StepError,
};
use zencodec::source::AccessPattern;
use zencodec::{ImageFormat, ImageInfo};
use zencodec_coroutine::{BoxedCoroutineDecoder, CoroutineDecoder, CoroutineError};
use zenpixels::{PixelDescriptor, PixelSlice};

pub const DESC: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;
pub const HEADER: usize = 16;

/// `"SYN1"`, width, height (u32 LE), 4 bytes pad, then raw RGBA8 rows.
pub fn synthetic_file(w: u32, h: u32) -> Vec<u8> {
    let mut f = Vec::with_capacity(HEADER + (w as usize * h as usize * 4));
    f.extend_from_slice(b"SYN1");
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&h.to_le_bytes());
    f.extend_from_slice(&[0u8; 4]);
    for y in 0..h {
        for x in 0..w {
            f.extend_from_slice(&[x as u8, y as u8, (x ^ y) as u8, 255]);
        }
    }
    f
}

fn dims(file: &[u8]) -> (u32, u32) {
    (
        u32::from_le_bytes(file[4..8].try_into().unwrap()),
        u32::from_le_bytes(file[8..12].try_into().unwrap()),
    )
}

/// Baseline: what a decoder pays with no driver at all.
pub fn direct_decode(file: &[u8], strip_rows: u32, sink: &mut dyn DecodeRowSink) {
    let (w, h) = dims(file);
    let row = w as usize * 4;
    sink.begin(w, h, DESC).unwrap();
    let mut y = 0;
    while y < h {
        let rows = (h - y).min(strip_rows);
        let start = HEADER + y as usize * row;
        let mut dst = sink.provide_next_buffer(y, rows, w, DESC).unwrap();
        for r in 0..rows {
            let s = start + r as usize * row;
            dst.row_mut(r).copy_from_slice(&file[s..s + row]);
        }
        y += rows;
    }
    sink.finish().unwrap();
}

#[derive(Debug)]
pub struct SynError(pub String);
impl std::fmt::Display for SynError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SynError {}
impl From<CoroutineError> for SynError {
    fn from(e: CoroutineError) -> Self {
        SynError(e.to_string())
    }
}

/// The hand-written template: no input buffer at all. Each step decodes
/// one strip straight from the driver's input when the whole strip is
/// there, and otherwise asks for it with one sequential request.
pub struct HandWritten {
    strip_rows: u32,
    info: Option<ImageInfo>,
    begun: bool,
    next_row: u32,
    done: Option<OutputInfo>,
    failed: bool,
    need: [Request; 1],
}

impl HandWritten {
    pub fn new(strip_rows: u32) -> Self {
        Self {
            strip_rows,
            info: None,
            begun: false,
            next_row: 0,
            done: None,
            failed: false,
            need: [Request::at_least(0, HEADER)],
        }
    }

    fn step_inner(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn enough::Stop,
    ) -> Result<Progress, StepError<SynError>> {
        stop.check().map_err(StepError::Cancelled)?;
        let Some(info) = &self.info else {
            if input.offset == 0 && input.bytes.len() >= HEADER {
                let (w, h) = dims(input.bytes);
                self.info = Some(ImageInfo::new(w, h, ImageFormat::Png));
                return Ok(Progress::new(
                    HEADER,
                    Event::Metadata(MetadataUpdate::HeaderReady),
                ));
            }
            if input.total_len.is_some_and(|t| t < HEADER as u64) {
                return Err(StepError::Codec(SynError("short header".into())));
            }
            self.need = [Request::at_least(0, HEADER)];
            return Ok(Progress::need(0));
        };
        let (w, h) = (info.width, info.height);
        if !self.begun {
            sink.begin(w, h, DESC).map_err(StepError::Sink)?;
            self.begun = true;
        }
        if self.next_row >= h {
            sink.finish().map_err(StepError::Sink)?;
            let out = OutputInfo::full_decode(w, h, DESC);
            self.done = Some(out.clone());
            return Ok(Progress::new(0, Event::Done(out)));
        }
        let row = w as usize * 4;
        let rows = (h - self.next_row).min(self.strip_rows);
        let len = rows as usize * row;
        let at = HEADER as u64 + self.next_row as u64 * row as u64;
        if input.offset == at && input.bytes.len() >= len {
            let mut dst = sink
                .provide_next_buffer(self.next_row, rows, w, DESC)
                .map_err(StepError::Sink)?;
            for r in 0..rows as usize {
                dst.row_mut(r as u32)
                    .copy_from_slice(&input.bytes[r * row..(r + 1) * row]);
            }
            let meta = BatchMeta::final_rows(self.next_row);
            self.next_row += rows;
            return Ok(Progress::new(len, Event::Rows { meta, rows }));
        }
        if input.total_len.is_some_and(|t| t < at + len as u64) {
            return Err(StepError::Codec(SynError("truncated".into())));
        }
        self.need = [Request::at_least(at, len)];
        Ok(Progress::need(0))
    }
}

impl IncrementalDecode for HandWritten {
    type Error = SynError;
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn enough::Stop,
    ) -> Result<Progress, StepError<SynError>> {
        if let Some(d) = &self.done {
            return Ok(Progress::new(0, Event::Done(d.clone())));
        }
        if self.failed {
            return Err(StepError::Finished);
        }
        let r = self.step_inner(input, sink, stop);
        self.failed = r.is_err();
        r
    }
    fn requested(&self) -> &[Request] {
        &self.need
    }
    fn info(&self) -> Option<&ImageInfo> {
        self.info.as_ref()
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        Some(DESC)
    }
    fn access_pattern(&self) -> AccessPattern {
        AccessPattern::Sequential
    }
}

pub type Coro = BoxedCoroutineDecoder<SynError>;

/// Sequential coroutine: `read_exact` per strip, `write_rows` per strip.
pub fn coroutine_sequential(strip_rows: u32) -> Coro {
    CoroutineDecoder::boxed_with_options(
        4 << 20,
        AccessPattern::Sequential,
        move |mut ctx| async move {
            let mut header = [0u8; HEADER];
            ctx.read_exact(&mut header).await?;
            let (w, h) = dims(&header);
            ctx.set_info(ImageInfo::new(w, h, ImageFormat::Png));
            ctx.set_output_descriptor(DESC);
            ctx.announce(MetadataUpdate::HeaderReady).await?;
            let row = w as usize * 4;
            let mut strip = vec![0u8; row * strip_rows as usize];
            let mut y = 0;
            while y < h {
                let rows = (h - y).min(strip_rows);
                let n = rows as usize * row;
                ctx.read_exact(&mut strip[..n]).await?;
                let s = PixelSlice::new(&strip[..n], w, rows, row, DESC).unwrap();
                ctx.write_rows(BatchMeta::final_rows(y), &s).await?;
                y += rows;
            }
            Ok(OutputInfo::full_decode(w, h, DESC))
        },
    )
}

/// Range-addressed coroutine: `read_exact_at` per strip.
pub fn coroutine_ranges(strip_rows: u32) -> Coro {
    CoroutineDecoder::boxed_with_options(
        4 << 20,
        AccessPattern::RandomAccess,
        move |mut ctx| async move {
            let mut header = [0u8; HEADER];
            ctx.read_exact_at(0, &mut header).await?;
            let (w, h) = dims(&header);
            ctx.set_info(ImageInfo::new(w, h, ImageFormat::Png));
            ctx.set_output_descriptor(DESC);
            ctx.announce(MetadataUpdate::HeaderReady).await?;
            let row = w as usize * 4;
            let mut strip = vec![0u8; row * strip_rows as usize];
            let mut y = 0;
            while y < h {
                let rows = (h - y).min(strip_rows);
                let n = rows as usize * row;
                let at = HEADER as u64 + y as u64 * row as u64;
                ctx.read_exact_at(at, &mut strip[..n]).await?;
                let s = PixelSlice::new(&strip[..n], w, rows, row, DESC).unwrap();
                ctx.write_rows(BatchMeta::final_rows(y), &s).await?;
                y += rows;
            }
            Ok(OutputInfo::full_decode(w, h, DESC))
        },
    )
}
