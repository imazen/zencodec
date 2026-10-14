//! Tests for stdio and tokio incremental consumers.

#![cfg(feature = "std")]

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
        if let Some(s) = stop
            && s.check().is_err()
        {
            return Err(MockError("stopped".into()));
        }

        if self.complete {
            return Ok(PushOutcome::new(0, PushStatus::Complete));
        }

        // Simulate backpressure: if buffer already has data waiting for pull, block!
        if self.buffered_bytes >= self.max_buffer && !is_eof {
            return Ok(PushOutcome::new(0, PushStatus::BlockedOnOutput));
        }

        let space = self.max_buffer.saturating_sub(self.buffered_bytes);
        let bytes_consumed = if space > 0 { chunk.len().min(space) } else { 0 };
        self.buffered_bytes += bytes_consumed;
        self.header_sent = true;

        if is_eof && self.current_y >= self.height {
            self.complete = true;
            return Ok(PushOutcome::new(bytes_consumed, PushStatus::Complete));
        }

        let status = if self.buffered_bytes >= self.max_buffer {
            PushStatus::BlockedOnOutput
        } else if self.current_y < self.height {
            PushStatus::OutputReady
        } else {
            PushStatus::Complete
        };

        Ok(PushOutcome::new(bytes_consumed, status).with_header_ready(true))
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
    let mut buf_pos = 0usize;
    let mut buf_len = 0usize;
    let mut is_reader_eof = false;

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
                    let h = info_h.unwrap_or(slice.rows());
                    total_w = w;
                    total_h = h;
                    dst_sink
                        .begin(w, h, desc)
                        .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
                    sink_begun = true;

                    let batch_rows = slice.rows();
                    let mut dst = dst_sink
                        .provide_next_buffer(meta.y, batch_rows, w, desc)
                        .map_err(|e| PumpError::Codec(D::wrap_sink_error(e)))?;
                    for row_idx in 0..batch_rows {
                        dst.row_mut(row_idx).copy_from_slice(slice.row(row_idx));
                    }
                }
            }

            if sink_begun {
                while let Some(_rows) = dec.pull_batch_into(dst_sink).map_err(PumpError::Codec)? {
                    if let Some(p) = pulse
                        && p.check().is_err()
                    {
                        return Err(PumpError::Cancelled);
                    }
                }
            }
            Ok(())
        };

    loop {
        if let Some(p) = pulse
            && p.check().is_err()
        {
            return Err(PumpError::Cancelled);
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

        if buf_pos >= buf_len {
            if is_reader_eof {
                let _outcome = decoder
                    .push_chunk_pulse(&[], true, pulse)
                    .map_err(PumpError::Codec)?;
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

            let n = reader.read(&mut buf).await.map_err(PumpError::Io)?;
            if n == 0 {
                is_reader_eof = true;
            }
            buf_pos = 0;
            buf_len = n;
        }

        let chunk = &buf[buf_pos..buf_len];
        let outcome = decoder
            .push_chunk_pulse(chunk, is_reader_eof, pulse)
            .map_err(PumpError::Codec)?;

        buf_pos += outcome.bytes_consumed;

        drain_batches(decoder, sink)?;

        if outcome.bytes_consumed == 0
            && outcome.status == PushStatus::NeedMoreInput
            && !is_reader_eof
        {
            let remaining = buf_len - buf_pos;
            buf.copy_within(buf_pos..buf_len, 0);
            buf_pos = 0;
            buf_len = remaining;
            let n = reader
                .read(&mut buf[remaining..])
                .await
                .map_err(PumpError::Io)?;
            if n == 0 {
                is_reader_eof = true;
            }
            buf_len += n;
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

#[test]
fn test_codec_set_with_pulse_propagation() {
    let stopper = almost_enough::Stopper::new();
    let shared = how_far::SharedPulse::new(how_far::StopOnly::new(stopper.clone()));
    let set = zencodec::CodecSet::new().with_pulse(shared);

    let mut inc = set.incremental_decoder(&[]);
    // Trigger cancellation before push
    stopper.cancel();
    assert!(stopper.should_stop());
    let err = inc
        .push_chunk(&[0x42; 64], false, None)
        .expect_err("should be cancelled");
    assert!(err.to_string().contains("cancelled"));
}

#[test]
fn test_stdio_consumer_partial_chunk_backpressure() {
    // Decoder with small max buffer (8 bytes), input stream with large chunks (64 bytes).
    // Tests that pump retains unconsumed suffixes across backpressure drains.
    let mut decoder = MockDecoder::new(16, 16, 4, 8);
    let mut sink = TestSink::new();
    let data = vec![0x42u8; 512];
    let reader = std::io::Cursor::new(data);

    let output = pump_read_to_sink(reader, &mut decoder, &mut sink, None, 64)
        .expect("pump with partial chunk backpressure succeeded");

    assert_eq!(output.width, 16);
    assert_eq!(output.height, 16);
    assert!(sink.finished);
    assert_eq!(sink.buf.len(), 16 * 16 * 3);
}

#[test]
fn test_one_byte_chunk_feeds() {
    // Test feeding input strictly 1 byte at a time through the pump.
    let mut decoder = MockDecoder::new(16, 16, 4, 16);
    let mut sink = TestSink::new();
    let data = vec![0x42u8; 256];
    let reader = std::io::Cursor::new(data);

    let output =
        pump_read_to_sink(reader, &mut decoder, &mut sink, None, 1).expect("1-byte pump succeeded");

    assert_eq!(output.width, 16);
    assert_eq!(output.height, 16);
    assert!(sink.finished);
    assert_eq!(sink.buf.len(), 16 * 16 * 3);
}

#[test]
fn test_codec_set_probe_handoff_byte_accounting() {
    let set = zencodec::CodecSet::new();
    let mut inc = set.incremental_decoder(&[]);

    // Push 4 bytes (probe buffer accepts, not enough to detect yet)
    let p1 = inc
        .push_chunk(&[0x42; 4], false, None)
        .expect("probe accepted");
    assert_eq!(p1.bytes_consumed, 4);
    assert_eq!(p1.status, PushStatus::NeedMoreInput);

    // Push next 4 bytes
    let p2 = inc
        .push_chunk(&[0x42; 4], false, None)
        .expect("probe accepted");
    assert_eq!(p2.bytes_consumed, 4);
    assert_eq!(p2.status, PushStatus::NeedMoreInput);
}

#[test]
fn test_metadata_update_semantics() {
    use zencodec::decode::MetadataUpdate;

    let p0 = PushOutcome::new(10, PushStatus::NeedMoreInput);
    assert_eq!(p0.metadata, MetadataUpdate::None);
    assert!(!p0.metadata.is_some());
    assert!(!p0.header_ready());

    let p1 = p0.with_metadata(MetadataUpdate::HeaderReady);
    assert_eq!(p1.metadata, MetadataUpdate::HeaderReady);
    assert!(p1.metadata.is_some());
    assert!(p1.metadata.is_header_ready());
    assert!(p1.header_ready());

    let p2 = p0.with_metadata(MetadataUpdate::ColorReady);
    assert_eq!(p2.metadata, MetadataUpdate::ColorReady);
    assert!(p2.metadata.is_some());
    assert!(p2.metadata.is_header_ready());
    assert!(p2.header_ready());

    let p3 = p0.with_metadata(MetadataUpdate::Updated);
    assert_eq!(p3.metadata, MetadataUpdate::Updated);
    assert!(p3.metadata.is_some());
    assert!(p3.metadata.is_header_ready());
    assert!(p3.header_ready());

    // Legacy helper compatibility:
    let p4 = p0.with_header_ready(true);
    assert_eq!(p4.metadata, MetadataUpdate::HeaderReady);
    assert!(p4.header_ready());
}

#[test]
fn test_dyn_incremental_decoder_downcasting() {
    let dec = MockDecoder::new(8, 8, 2, 16);
    let mut shim: Box<dyn DynIncrementalDecoder> = Box::new(IncrementalDecoderShim(dec));

    // Downcasting immutable
    let any_ref = shim.as_any();
    assert!(any_ref.downcast_ref::<MockDecoder>().is_some());

    // Downcasting mutable
    let any_mut = shim.as_any_mut();
    assert!(any_mut.downcast_mut::<MockDecoder>().is_some());

    // Downcasting consume
    let any_box = shim.into_any();
    assert!(any_box.downcast::<MockDecoder>().is_ok());
}

#[test]
fn test_byte_range_properties() {
    use zencodec::decode::ByteRange;

    let r = ByteRange::new(1000, 500);
    assert_eq!(r.offset, 1000);
    assert_eq!(r.length, 500);
    assert_eq!(r.end(), 1500);
    assert!(!r.is_empty());

    assert!(!r.contains(999));
    assert!(r.contains(1000));
    assert!(r.contains(1250));
    assert!(r.contains(1499));
    assert!(!r.contains(1500));
    assert!(!r.contains(2000));

    let empty = ByteRange::new(1000, 0);
    assert!(empty.is_empty());
    assert_eq!(empty.end(), 1000);
    assert!(!empty.contains(1000));
}

#[test]
fn test_coalesce_byte_ranges() {
    use zencodec::decode::{ByteRange, coalesce_byte_ranges};

    // Empty input
    assert!(coalesce_byte_ranges(&[], 64).is_empty());

    // Filter empty ranges
    let empty_ranges = [ByteRange::new(100, 0), ByteRange::new(200, 0)];
    assert!(coalesce_byte_ranges(&empty_ranges, 64).is_empty());

    // Single range
    let single = [ByteRange::new(100, 50)];
    let res = coalesce_byte_ranges(&single, 0);
    assert_eq!(res, vec![ByteRange::new(100, 50)]);

    // Directly adjacent ranges (gap = 0)
    let adjacent = [ByteRange::new(100, 50), ByteRange::new(150, 50)];
    let res = coalesce_byte_ranges(&adjacent, 0);
    assert_eq!(res, vec![ByteRange::new(100, 100)]);

    // Overlapping ranges
    let overlapping = [ByteRange::new(100, 60), ByteRange::new(120, 80)];
    let res = coalesce_byte_ranges(&overlapping, 0);
    assert_eq!(res, vec![ByteRange::new(100, 100)]);

    // Ranges within max_gap
    let within_gap = [
        ByteRange::new(100, 50),  // 100..150
        ByteRange::new(180, 20),  // 180..200 (gap = 30)
        ByteRange::new(500, 100), // 500..600 (gap = 300)
    ];
    let res = coalesce_byte_ranges(&within_gap, 64);
    assert_eq!(
        res,
        vec![ByteRange::new(100, 100), ByteRange::new(500, 100)]
    );

    // Unsorted order with gap coalescing
    let unsorted = [
        ByteRange::new(500, 100),
        ByteRange::new(180, 20),
        ByteRange::new(100, 50),
    ];
    let res = coalesce_byte_ranges(&unsorted, 64);
    assert_eq!(
        res,
        vec![ByteRange::new(100, 100), ByteRange::new(500, 100)]
    );
}

/// A mock range-seeking decoder (simulating Cloud-Optimized TIFF or JXL TOC).
///
/// Stage 0: Pushed probe bytes at offset 0. Parses header and learns table is at offset 100,000.
///          Returns `NeedRanges([100000..101000])`.
/// Stage 1: Pushed table range at offset 100,000. Parses table and learns tile #1 is at offset 200,000.
///          Returns `NeedRanges([200000..204096])`.
/// Stage 2: Pushed tile data at offset 200,000. Produces scanlines.
struct MockRangeDecoder {
    stage: u8,
    requested: Vec<zencodec::decode::ByteRange>,
    info: ImageInfo,
    complete: bool,
}

impl MockRangeDecoder {
    fn new() -> Self {
        Self {
            stage: 0,
            requested: Vec::new(),
            info: ImageInfo::new(16, 16, ImageFormat::Tiff),
            complete: false,
        }
    }
}

impl IncrementalDecode for MockRangeDecoder {
    type Error = MockError;

    fn push_chunk(
        &mut self,
        _chunk: &[u8],
        _is_eof: bool,
        _stop: Option<&dyn Stop>,
    ) -> Result<PushOutcome, Self::Error> {
        // Fallback sequential
        Ok(PushOutcome::new(0, PushStatus::NeedMoreInput))
    }

    fn requested_ranges(&self) -> &[zencodec::decode::ByteRange] {
        &self.requested
    }

    fn push_range(
        &mut self,
        offset: u64,
        chunk: &[u8],
        _is_eof: bool,
        _stop: Option<&dyn Stop>,
    ) -> Result<PushOutcome, Self::Error> {
        use zencodec::decode::ByteRange;

        match self.stage {
            0 => {
                // Expecting probe header at offset 0
                assert_eq!(offset, 0);
                self.stage = 1;
                self.requested = vec![ByteRange::new(100_000, 1000)];
                Ok(PushOutcome::new(chunk.len(), PushStatus::NeedRanges)
                    .with_metadata(zencodec::decode::MetadataUpdate::HeaderReady))
            }
            1 => {
                // Expecting metadata / IFD table at offset 100,000
                assert_eq!(offset, 100_000);
                self.stage = 2;
                self.requested = vec![ByteRange::new(200_000, 4096)];
                Ok(PushOutcome::new(chunk.len(), PushStatus::NeedRanges))
            }
            2 => {
                // Expecting spatial tile payload at offset 200,000
                assert_eq!(offset, 200_000);
                self.stage = 3;
                self.requested.clear();
                Ok(PushOutcome::new(chunk.len(), PushStatus::OutputReady))
            }
            _ => Ok(PushOutcome::new(0, PushStatus::Complete)),
        }
    }

    fn pull_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error> {
        if self.stage == 3 {
            self.stage = 4;
            self.complete = true;
            // Return dummy batch
            static ROWS: [u8; 16 * 16 * 3] = [0x55; 16 * 16 * 3];
            let slice = PixelSlice::new(&ROWS, 16, 16, 16 * 3, PixelDescriptor::RGB8_SRGB)
                .map_err(|e| MockError(format!("slice: {e:?}")))?;
            Ok(Some((0, slice)))
        } else {
            Ok(None)
        }
    }

    fn wrap_sink_error(err: SinkError) -> Self::Error {
        MockError(err.to_string())
    }

    fn info(&self) -> Option<&ImageInfo> {
        if self.stage >= 1 {
            Some(&self.info)
        } else {
            None
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }
}

#[test]
fn test_range_aware_decoder_handshake() {
    use zencodec::decode::{ByteRange, DynIncrementalDecoder, IncrementalDecoderShim};

    let dec = MockRangeDecoder::new();
    let mut shim: Box<dyn DynIncrementalDecoder> = Box::new(IncrementalDecoderShim(dec));

    // Stage 0: Probe at offset 0
    let o0 = shim
        .push_range(0, &[0x49, 0x49, 0x2A, 0x00], false, None)
        .unwrap();
    assert_eq!(o0.status, PushStatus::NeedRanges);
    assert!(o0.header_ready());
    assert_eq!(shim.requested_ranges(), &[ByteRange::new(100_000, 1000)]);

    // Stage 1: Fetch requested table range at 100,000
    let o1 = shim
        .push_range(100_000, &[0xAA; 1000], false, None)
        .unwrap();
    assert_eq!(o1.status, PushStatus::NeedRanges);
    assert_eq!(shim.requested_ranges(), &[ByteRange::new(200_000, 4096)]);

    // Stage 2: Fetch requested tile range at 200,000
    let o2 = shim
        .push_range(200_000, &[0xBB; 4096], false, None)
        .unwrap();
    assert_eq!(o2.status, PushStatus::OutputReady);
    assert!(shim.requested_ranges().is_empty());

    // Stage 3: Pull scanlines
    let batch = shim.pull_batch().unwrap();
    assert!(batch.is_some());
    let (y, slice) = batch.unwrap();
    assert_eq!(y, 0);
    assert_eq!(slice.rows(), 16);
    assert!(shim.is_complete());
}
