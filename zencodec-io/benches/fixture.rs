//! A synthetic RGBA8 image whose decode is a memcpy per strip, written
//! three ways: direct sink writes, a hand-written feed/step decoder, and
//! coroutines (sequential and range-addressed). Shared by the benches and
//! the allocation audit.

#![allow(dead_code)]

use zencodec::decode::{
    BatchMeta, ByteRange, DecodeRowSink, IncrementalDecode, MetadataUpdate, OutputInfo, SinkError,
    Step,
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

/// The hand-written template: sequential buffer, cursor, strips written
/// straight into the sink, consumed bytes released by advancing a head
/// index (never by shifting the buffer), with amortised compaction.
pub struct HandWritten {
    strip_rows: u32,
    buf: Vec<u8>,
    /// Live bytes are `buf[head..]`, starting at absolute `buf_start`.
    head: usize,
    buf_start: u64,
    total: Option<u64>,
    info: Option<ImageInfo>,
    begun: bool,
    next_row: u32,
    done: Option<OutputInfo>,
    need: [ByteRange; 1],
}

impl HandWritten {
    pub fn new(strip_rows: u32) -> Self {
        Self {
            strip_rows,
            buf: Vec::new(),
            head: 0,
            buf_start: 0,
            total: None,
            info: None,
            begun: false,
            next_row: 0,
            done: None,
            need: [ByteRange::from_offset(0)],
        }
    }
    fn live_len(&self) -> usize {
        self.buf.len() - self.head
    }
    fn release_before(&mut self, offset: u64) {
        if offset > self.buf_start {
            let drop = ((offset - self.buf_start) as usize).min(self.live_len());
            self.head += drop;
            self.buf_start += drop as u64;
            // Compact once the dead prefix dominates: O(live) every O(n) bytes.
            if self.head > self.buf.len() / 2 && self.head >= 64 * 1024 {
                self.buf.drain(..self.head);
                self.head = 0;
            }
        }
    }
}

impl IncrementalDecode for HandWritten {
    type Error = SynError;
    fn feed(&mut self, offset: u64, bytes: &[u8], eof: bool) -> Result<usize, SynError> {
        let cursor = self.buf_start + self.live_len() as u64;
        if eof {
            self.total = Some(offset + bytes.len() as u64);
        }
        if offset + (bytes.len() as u64) <= cursor || offset > cursor {
            return Ok(bytes.len());
        }
        let skip = (cursor - offset) as usize;
        self.buf.extend_from_slice(&bytes[skip..]);
        Ok(bytes.len())
    }
    fn step(
        &mut self,
        sink: &mut dyn DecodeRowSink,
        stop: Option<&dyn enough::Stop>,
    ) -> Result<Step<'_>, SynError> {
        if let Some(d) = &self.done {
            return Ok(Step::Done(d.clone()));
        }
        if let Some(s) = stop {
            s.check().map_err(|e| SynError(format!("{e:?}")))?;
        }
        let cursor = self.buf_start + self.live_len() as u64;
        let at_end = self.total.is_some_and(|t| cursor >= t);
        if self.info.is_none() {
            if self.live_len() < HEADER {
                if at_end {
                    return Err(SynError("short header".into()));
                }
                self.need = [ByteRange::from_offset(cursor)];
                return Ok(Step::Need(&self.need));
            }
            let (w, h) = dims(&self.buf[self.head..]);
            self.info = Some(ImageInfo::new(w, h, ImageFormat::Png));
            return Ok(Step::Metadata(MetadataUpdate::HeaderReady));
        }
        let (w, h) = {
            let i = self.info.as_ref().unwrap();
            (i.width, i.height)
        };
        if !self.begun {
            sink.begin(w, h, DESC)
                .map_err(|e| SynError(e.to_string()))?;
            self.begun = true;
        }
        if self.next_row >= h {
            sink.finish().map_err(|e| SynError(e.to_string()))?;
            let info = OutputInfo::full_decode(w, h, DESC);
            self.done = Some(info.clone());
            return Ok(Step::Done(info));
        }
        let row = w as usize * 4;
        let rows = (h - self.next_row).min(self.strip_rows);
        let strip_start = HEADER as u64 + self.next_row as u64 * row as u64;
        let strip_end = strip_start + rows as u64 * row as u64;
        self.release_before(strip_start);
        if cursor < strip_end {
            if at_end {
                return Err(SynError("truncated".into()));
            }
            self.need = [ByteRange::from_offset(cursor)];
            return Ok(Step::Need(&self.need));
        }
        let start = self.head + (strip_start - self.buf_start) as usize;
        {
            let mut dst = sink
                .provide_next_buffer(self.next_row, rows, w, DESC)
                .map_err(|e| SynError(e.to_string()))?;
            for r in 0..rows {
                let s = start + r as usize * row;
                dst.row_mut(r).copy_from_slice(&self.buf[s..s + row]);
            }
        }
        let meta = BatchMeta::final_rows(self.next_row);
        self.next_row += rows;
        self.release_before(strip_end);
        Ok(Step::Rows { meta, rows })
    }
    fn wrap_sink_error(err: SinkError) -> SynError {
        SynError(err.to_string())
    }
    fn info(&self) -> Option<&ImageInfo> {
        self.info.as_ref()
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        Some(DESC)
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
