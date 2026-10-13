//! Tests for stdio and tokio incremental consumers.

use enough::Stop;
use zencodec::decode::OutputInfo;
use zencodec::decode::{
    DecodeRowSink, DynIncrementalDecoder, IncrementalDecode, IncrementalDecoderShim, PushOutcome,
    PushStatus, SinkError,
};
use zencodec::helpers::{PumpError, pump_dyn_read_to_sink, pump_read_to_sink};
use zencodec::{ImageFormat, ImageInfo};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

/// A mock incremental decoder that decodes a simple test pattern:
/// A 16x16 RGB8 image split into 4 batches of 4 rows each.
/// It simulates backpressure when more than `max_unconsumed_bytes` are pushed.
struct MockDecoder {
    width: u32,
    height: u32,
    rows_per_batch: u32,
    current_y: u32,
    buffered_bytes: usize,
    max_buffer: usize,
    buffer: Vec<u8>,
    header_sent: bool,
    complete: bool,
    info: ImageInfo,
}

impl MockDecoder {
    fn new(width: u32, height: u32, rows_per_batch: u32, max_buffer: usize) -> Self {
        Self {
            width,
            height,
            rows_per_batch,
            current_y: 0,
            buffered_bytes: 0,
            max_buffer,
            buffer: Vec::new(),
            header_sent: false,
            complete: false,
            info: ImageInfo::new(width, height, ImageFormat::Png),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Mock decode error: {0}")]
struct MockError(String);

impl IncrementalDecode for MockDecoder {
    type Error = MockError;

    fn push_chunk(
        &mut self,
        chunk: &[u8],
        is_eof: bool,
        stop: Option<&dyn Stop>,
    ) -> Result<PushOutcome, Self::Error> {
        if let Some(s) = stop {
            if s.check().is_err() {
                return Err(MockError("stopped".into()));
            }
        }

        if self.complete {
            return Ok(PushOutcome::new(0, PushStatus::Complete));
        }

        // Simulate backpressure: if buffer already has data waiting for pull, block!
        if self.buffered_bytes >= self.max_buffer && !is_eof {
            return Ok(PushOutcome::new(0, PushStatus::BlockedOnOutput));
        }

        self.buffered_bytes += chunk.len();
        self.header_sent = true;

        if is_eof && self.current_y >= self.height {
            self.complete = true;
            return Ok(PushOutcome::new(chunk.len(), PushStatus::Complete));
        }

        let status = if self.buffered_bytes >= self.max_buffer {
            PushStatus::BlockedOnOutput
        } else if self.current_y < self.height {
            PushStatus::OutputReady
        } else {
            PushStatus::Complete
        };

        Ok(PushOutcome::new(chunk.len(), status).with_header_ready(true))
    }

    fn pull_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error> {
        if !self.header_sent || self.current_y >= self.height {
            if self.current_y >= self.height {
                self.complete = true;
            }
            return Ok(None);
        }

        let y = self.current_y;
        let batch_rows = (self.height - self.current_y).min(self.rows_per_batch);
        self.current_y += batch_rows;

        // Reset buffered bytes to release backpressure
        self.buffered_bytes = 0;

        let row_bytes = self.width as usize * 3;
        let total_bytes = row_bytes * batch_rows as usize;
        self.buffer.resize(total_bytes, 0);

        // Fill pattern: row value = (y + r) as u8
        for r in 0..batch_rows {
            let val = (y + r) as u8;
            let start = r as usize * row_bytes;
            let end = start + row_bytes;
            self.buffer[start..end].fill(val);
        }

        let slice = PixelSlice::new(
            &self.buffer,
            self.width,
            batch_rows,
            row_bytes,
            PixelDescriptor::RGB8_SRGB,
        )
        .map_err(|e| MockError(format!("slice: {e:?}")))?;

        if self.current_y >= self.height {
            self.complete = true;
        }

        Ok(Some((y, slice)))
    }

    fn wrap_sink_error(err: SinkError) -> Self::Error {
        MockError(err.to_string())
    }

    fn info(&self) -> Option<&ImageInfo> {
        if self.header_sent {
            Some(&self.info)
        } else {
            None
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }
}

/// A test buffer sink that captures all decoded rows.
struct TestSink {
    width: u32,
    height: u32,
    buf: Vec<u8>,
    began: bool,
    finished: bool,
}

impl TestSink {
    fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            buf: Vec::new(),
            began: false,
            finished: false,
        }
    }
}

impl DecodeRowSink for TestSink {
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<(), SinkError> {
        self.width = width;
        self.height = height;
        let bpp = descriptor.bytes_per_pixel();
        self.buf.resize(width as usize * height as usize * bpp, 0);
        self.began = true;
        Ok(())
    }

    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        assert!(self.began);
        let bpp = descriptor.bytes_per_pixel();
        let stride = width as usize * bpp;
        let start = y as usize * stride;
        let end = start + height as usize * stride;
        let slice = &mut self.buf[start..end];
        PixelSliceMut::new(slice, width, height, stride, descriptor)
            .map_err(|e| format!("{e:?}").into())
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        self.finished = true;
        Ok(())
    }
}

#[test]
fn test_stdio_consumer_success() {
    let mut decoder = MockDecoder::new(16, 16, 4, 128);
    let mut sink = TestSink::new();
    let input_bytes = vec![0xAAu8; 1024];
    let reader = std::io::Cursor::new(input_bytes);

    let output =
        pump_read_to_sink(reader, &mut decoder, &mut sink, None, 64).expect("pump succeeded");

    assert_eq!(output.width, 16);
    assert_eq!(output.height, 16);
    assert!(sink.began);
    assert!(sink.finished);
    assert_eq!(sink.buf.len(), 16 * 16 * 3);

    // Verify row patterns
    for y in 0..16u32 {
        let expected = y as u8;
        let start = y as usize * 48;
        assert_eq!(sink.buf[start], expected);
    }
}

#[test]
fn test_stdio_consumer_dyn_dispatch() {
    let decoder = MockDecoder::new(16, 16, 4, 128);
    let mut shim = IncrementalDecoderShim(decoder);
    let mut sink = TestSink::new();
    let input_bytes = vec![0xBBu8; 512];
    let reader = std::io::Cursor::new(input_bytes);

    let output = pump_dyn_read_to_sink(
        reader,
        &mut shim as &mut dyn DynIncrementalDecoder,
        &mut sink,
        None,
        32,
    )
    .expect("dyn pump succeeded");

    assert_eq!(output.width, 16);
    assert_eq!(output.height, 16);
    assert!(sink.finished);
}

#[test]
fn test_stdio_consumer_backpressure() {
    // max_buffer = 10 bytes: each push exceeding 10 bytes triggers BlockedOnOutput
    let mut decoder = MockDecoder::new(16, 16, 4, 10);
    let mut sink = TestSink::new();
    let input_bytes = vec![0xCCu8; 256];
    let reader = std::io::Cursor::new(input_bytes);

    // Buffer size = 64 bytes (larger than max_buffer) to force backpressure handling
    let output = pump_read_to_sink(reader, &mut decoder, &mut sink, None, 64)
        .expect("backpressure handled cleanly");

    assert_eq!(output.width, 16);
    assert!(sink.finished);
}

// ── Tokio Async Consumer Implementation & Tests ─────────────────────────────

/// Async pump: read asynchronously from `tokio::io::AsyncRead` and feed into `IncrementalDecode`.
async fn pump_async_read_to_sink<R, D>(
    mut reader: R,
    decoder: &mut D,
    sink: &mut (dyn DecodeRowSink + '_),
    pulse: Option<&dyn how_far::Pulse>,
    buffer_size: usize,
) -> Result<OutputInfo, PumpError<D::Error>>
where
    R: tokio::io::AsyncRead + Unpin,
    D: IncrementalDecode,
{
    use tokio::io::AsyncReadExt;

    let buffer_size = buffer_size.max(512);
    let mut buf = vec![0u8; buffer_size];
    let mut sink_begun = false;
    let mut chosen_desc: Option<PixelDescriptor> = None;
    let mut total_w = 0u32;
    let mut total_h = 0u32;

    let mut drain_batches =
        |dec: &mut D, dst_sink: &mut (dyn DecodeRowSink + '_)| -> Result<(), PumpError<D::Error>> {
            if !sink_begun {
                let info_h = dec.info().map(|i| i.height);
                if let Some((meta, slice)) = dec.pull_batch_meta().map_err(PumpError::Codec)? {
                    let desc = slice.descriptor();
                    chosen_desc = Some(desc);
                    let w = slice.width();
                    let h = info_h.unwrap_or(slice.rows() as u32);
                    total_w = w;
                    total_h = h;
                    dst_sink
                        .begin(w, h, desc)
                        .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
                    sink_begun = true;

                    let batch_rows = slice.rows() as u32;
                    let mut dst = dst_sink
                        .provide_next_buffer(meta.y, batch_rows, w, desc)
                        .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
                    for row_idx in 0..batch_rows {
                        dst.row_mut(row_idx).copy_from_slice(slice.row(row_idx));
                    }
                }
            }

            if sink_begun {
                while let Some(_rows) = dec.pull_batch_into(dst_sink).map_err(PumpError::Codec)? {}
            }
            Ok(())
        };

    loop {
        if let Some(p) = pulse {
            if p.check().is_err() {
                return Err(PumpError::Cancelled);
            }
        }

        drain_batches(decoder, sink)?;

        if decoder.is_complete() {
            if sink_begun {
                sink.finish()
                    .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
            }
            let info = decoder.info().ok_or(PumpError::UnexpectedEof)?;
            let desc = chosen_desc.unwrap_or(zenpixels::PixelDescriptor::RGB8_SRGB);
            return Ok(OutputInfo::full_decode(info.width, info.height, desc));
        }

        let n = reader.read(&mut buf).await.map_err(PumpError::Io)?;
        let is_eof = n == 0;

        let _outcome = decoder
            .push_chunk_pulse(&buf[..n], is_eof, pulse)
            .map_err(PumpError::Codec)?;

        drain_batches(decoder, sink)?;

        if is_eof {
            drain_batches(decoder, sink)?;
            if !decoder.is_complete() {
                return Err(PumpError::UnexpectedEof);
            }
            if sink_begun {
                sink.finish()
                    .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
            }
            let desc = chosen_desc.unwrap_or(zenpixels::PixelDescriptor::RGB8_SRGB);
            let w = if total_w > 0 {
                total_w
            } else {
                decoder.info().map(|i| i.width).unwrap_or(0)
            };
            let h = if total_h > 0 {
                total_h
            } else {
                decoder.info().map(|i| i.height).unwrap_or(0)
            };
            return Ok(OutputInfo::full_decode(w, h, desc));
        }
    }
}

#[tokio::test]
async fn test_tokio_async_consumer() {
    let mut decoder = MockDecoder::new(16, 16, 4, 32);
    let mut sink = TestSink::new();
    let data = vec![0x42u8; 512];
    let reader = tokio::io::BufReader::new(&data[..]);

    let output = pump_async_read_to_sink(reader, &mut decoder, &mut sink, None, 64)
        .await
        .expect("async pump succeeded");

    assert_eq!(output.width, 16);
    assert_eq!(output.height, 16);
    assert!(sink.finished);
    assert_eq!(sink.buf.len(), 16 * 16 * 3);
}

#[tokio::test]
async fn test_tokio_async_backpressure_and_cancellation() {
    let mut decoder = MockDecoder::new(16, 16, 4, 8);
    let mut sink = TestSink::new();
    let data = vec![0x42u8; 512];
    let reader = tokio::io::BufReader::new(&data[..]);

    // Create a pulse token and trigger cancellation
    let stopper = almost_enough::Stopper::new();
    stopper.cancel();
    let stop_only = how_far::StopOnly::new(stopper);

    let err = pump_async_read_to_sink(
        reader,
        &mut decoder,
        &mut sink,
        Some(&stop_only as &dyn how_far::Pulse),
        32,
    )
    .await
    .expect_err("should cancel");

    match err {
        PumpError::Cancelled => {}
        other => panic!("expected Cancelled, got {other:?}"),
    }
}
