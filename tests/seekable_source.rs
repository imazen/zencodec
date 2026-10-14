//! Tests for the seekable `ReadAt` source layer, buffered-stream
//! adaptation, range driving, and provisional-pass finalization.

#![cfg(feature = "std")]

use std::fmt;
use std::io::Cursor;

use enough::Stop;
use zencodec::decode::{
    BatchMeta, DecodeRowSink, IncrementalDecode, OutputInfo, PushOutcome, PushStatus,
    SeekableDecode, SinkError,
};
use zencodec::source::{
    AccessPattern, BufferPolicy, BufferedReadAt, DriveError, DynReadAt, ReadAt, ReadAtShim,
    SeekableIncrementalAdapter, SliceReadAt, SourceError, drive_ranges,
};
use zencodec::{ImageFormat, ImageInfo};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

#[derive(Debug)]
struct MockError(String);

impl fmt::Display for MockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mock error: {}", self.0)
    }
}

impl std::error::Error for MockError {}

/// Trivial seekable codec over a synthetic format: `[w, h, gray pixels..]`
/// stored at offset 2, deliberately read in two ranges (header, then
/// pixels) so range reads are exercised.
struct MockSeekable {
    info: Option<ImageInfo>,
}

impl MockSeekable {
    fn new() -> Self {
        Self { info: None }
    }
}

impl SeekableDecode for MockSeekable {
    type Error = MockError;

    fn run(
        &mut self,
        source: &mut dyn DynReadAt,
        sink: &mut dyn DecodeRowSink,
        _stop: Option<&dyn Stop>,
    ) -> Result<OutputInfo, Self::Error> {
        let mut header = [0u8; 2];
        source
            .read_exact_at(0, &mut header)
            .map_err(|e| MockError(e.to_string()))?;
        let (w, h) = (u32::from(header[0]), u32::from(header[1]));
        self.info = Some(ImageInfo::new(w, h, ImageFormat::Png));
        let desc = PixelDescriptor::GRAY8_SRGB;
        let mut pixels = vec![0u8; (w * h) as usize];
        source
            .read_exact_at(2, &mut pixels)
            .map_err(|e| MockError(e.to_string()))?;
        sink.begin(w, h, desc)
            .map_err(|e| MockError(e.to_string()))?;
        let mut dst = sink
            .provide_next_buffer(0, h, w, desc)
            .map_err(|e| MockError(e.to_string()))?;
        for row in 0..h {
            let start = (row * w) as usize;
            dst.row_mut(row)
                .copy_from_slice(&pixels[start..start + w as usize]);
        }
        drop(dst);
        sink.finish().map_err(|e| MockError(e.to_string()))?;
        Ok(OutputInfo::full_decode(w, h, desc))
    }

    fn wrap_sink_error(err: SinkError) -> Self::Error {
        MockError(err.to_string())
    }

    fn wrap_source_error(err: zencodec::decode::BoxedError) -> Self::Error {
        MockError(err.to_string())
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.info.as_ref()
    }

    fn access_pattern(&self) -> AccessPattern {
        AccessPattern::RandomAccess
    }
}

#[derive(Default)]
struct CollectSink {
    buf: Vec<u8>,
    width: u32,
    height: u32,
    began: bool,
    finished: bool,
}

impl DecodeRowSink for CollectSink {
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        _descriptor: PixelDescriptor,
    ) -> Result<(), SinkError> {
        self.width = width;
        self.height = height;
        self.began = true;
        Ok(())
    }

    fn provide_next_buffer(
        &mut self,
        _y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        let stride = width as usize * descriptor.bytes_per_pixel();
        self.buf.resize(height as usize * stride, 0);
        Ok(
            PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                .expect("collect sink buffer"),
        )
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        self.finished = true;
        Ok(())
    }
}

fn synthetic_image() -> Vec<u8> {
    // 4x3 gray image, pixels 10, 20, 30, ...
    let mut data = vec![4u8, 3u8];
    data.extend((1..=12u8).map(|i| i * 10));
    data
}

#[test]
fn slice_read_at_reads_and_eof() {
    let data = synthetic_image();
    let mut src = SliceReadAt::new(&data);
    assert_eq!(src.len().unwrap(), data.len() as u64);
    let mut header = [0u8; 2];
    src.read_exact_at(0, &mut header).unwrap();
    assert_eq!(header, [4, 3]);
    let mut one = [0u8; 1];
    assert!(matches!(
        src.read_exact_at(data.len() as u64, &mut one),
        Err(SourceError::UnexpectedEof { .. })
    ));
}

#[test]
fn buffered_window_serves_forward_and_rejects_far_back() {
    let data: Vec<u8> = (0..200u8).collect();
    let mut src = BufferedReadAt::new(
        Cursor::new(data.clone()),
        BufferPolicy::Window { window_bytes: 32 },
    );
    // Forward read near the end pulls the stream forward.
    let mut tail = [0u8; 8];
    src.read_exact_at(160, &mut tail).unwrap();
    assert_eq!(tail, data[160..168]);
    // Offset 0 has been evicted from the 32-byte window.
    let mut head = [0u8; 4];
    assert!(matches!(
        src.read_exact_at(0, &mut head),
        Err(SourceError::BeyondWindow { .. })
    ));
    // A read inside the retained window still works.
    let mut recent = [0u8; 4];
    src.read_exact_at(164, &mut recent).unwrap();
    assert_eq!(recent, data[164..168]);
}

#[test]
fn buffered_cache_cap_and_spool_len() {
    let data: Vec<u8> = (0..100u8).collect();
    let mut capped = BufferedReadAt::new(
        Cursor::new(data.clone()),
        BufferPolicy::Cache { max_bytes: 16 },
    );
    let mut big = [0u8; 32];
    assert!(matches!(
        capped.read_exact_at(0, &mut big),
        Err(SourceError::BeyondLimit { .. })
    ));

    let mut spool = BufferedReadAt::new(
        Cursor::new(data.clone()),
        BufferPolicy::Spool { max_bytes: 4096 },
    );
    // len() drains the spool and reports the true total.
    assert_eq!(spool.len().unwrap(), 100);
    // Arbitrary seeks are served from the spool afterwards.
    let mut v = [0u8; 4];
    spool.read_exact_at(50, &mut v).unwrap();
    assert_eq!(v, data[50..54]);
    spool.read_exact_at(0, &mut v).unwrap();
    assert_eq!(v, data[0..4]);
}

#[test]
fn buffered_read_many_serves_in_stream_order() {
    let data: Vec<u8> = (0..64u8).collect();
    let mut src = BufferedReadAt::new(
        Cursor::new(data.clone()),
        BufferPolicy::Spool { max_bytes: 4096 },
    );
    let mut a = [0u8; 4];
    let mut b = [0u8; 4];
    {
        let mut reqs: Vec<(u64, &mut [u8])> = vec![(40, &mut a[..]), (4, &mut b[..])];
        src.read_many_at(&mut reqs).unwrap();
    }
    assert_eq!(a, data[40..44]);
    assert_eq!(b, data[4..8]);
}

/// Incremental decoder that needs two disjoint ranges after its header.
struct RangeDecoder {
    requested: Vec<zencodec::decode::ByteRange>,
    got: Vec<(u64, Vec<u8>)>,
    info: ImageInfo,
}

impl RangeDecoder {
    fn new() -> Self {
        Self {
            requested: vec![
                zencodec::decode::ByteRange::new(10, 4),
                zencodec::decode::ByteRange::new(30, 4),
            ],
            got: Vec::new(),
            info: ImageInfo::new(4, 1, ImageFormat::Png),
        }
    }
}

impl IncrementalDecode for RangeDecoder {
    type Error = MockError;

    fn push_chunk(
        &mut self,
        chunk: &[u8],
        _is_eof: bool,
        _stop: Option<&dyn Stop>,
    ) -> Result<PushOutcome, Self::Error> {
        let _ = chunk;
        Ok(PushOutcome::new(chunk.len(), PushStatus::NeedRanges))
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
        self.got.push((offset, chunk.to_vec()));
        let done = self.got.iter().any(|(o, _)| *o == 10) && self.got.iter().any(|(o, _)| *o == 30);
        // Coalesced delivery (one wide span) also satisfies both requests.
        let covered = self
            .got
            .iter()
            .any(|(o, b)| *o <= 10 && *o + b.len() as u64 >= 34);
        if done || covered {
            self.requested = Vec::new();
            Ok(PushOutcome::new(chunk.len(), PushStatus::OutputReady))
        } else {
            Ok(PushOutcome::new(chunk.len(), PushStatus::NeedRanges))
        }
    }

    fn pull_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error> {
        Ok(None)
    }

    fn wrap_sink_error(err: SinkError) -> Self::Error {
        MockError(err.to_string())
    }

    fn info(&self) -> Option<&ImageInfo> {
        Some(&self.info)
    }

    fn is_complete(&self) -> bool {
        false
    }
}

struct CountingSource<'a> {
    inner: SliceReadAt<'a>,
    many_calls: u64,
    single_calls: u64,
}

impl ReadAt for CountingSource<'_> {
    type Error = SourceError;

    fn len(&mut self) -> Result<u64, Self::Error> {
        self.inner.len()
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        self.single_calls += 1;
        self.inner.read_exact_at(offset, out)
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error> {
        self.many_calls += 1;
        for (offset, out) in reqs.iter_mut() {
            self.inner.read_exact_at(*offset, out)?;
        }
        Ok(())
    }
}

#[test]
fn drive_ranges_fetches_one_batched_round() {
    let data: Vec<u8> = (0..64u8).collect();
    let mut dec = RangeDecoder::new();
    // Initial sequential push lands in NeedRanges.
    let outcome = dec.push_chunk(&data[..8], false, None).unwrap();
    assert_eq!(outcome.status, PushStatus::NeedRanges);

    let mut src = CountingSource {
        inner: SliceReadAt::new(&data),
        many_calls: 0,
        single_calls: 0,
    };
    // max_gap = 0 keeps the two ranges as one batched read_many call.
    let stats = drive_ranges(&mut dec, &mut src, 0, None).unwrap();
    assert_eq!(stats.rounds, 1);
    assert_eq!(src.many_calls, 1, "ranges must be fetched in one batch");
    assert_eq!(stats.bytes_fetched, 8);
    assert_eq!(dec.got.len(), 2);
    assert_eq!(dec.got[0].1, data[10..14]);
    assert_eq!(dec.got[1].1, data[30..34]);

    // A decoder that never asks for ranges is a driver error, not a spin.
    let mut done = RangeDecoder::new();
    done.requested = Vec::new();
    let mut src2 = SliceReadAt::new(&data);
    assert!(matches!(
        drive_ranges(&mut done, &mut src2, 0, None),
        Err(DriveError::EmptyRangeRequest)
    ));
}

#[test]
fn seekable_adapter_matches_direct_run() {
    let data = synthetic_image();

    // Direct seekable run.
    let mut direct = MockSeekable::new();
    let mut source: Box<dyn DynReadAt> = Box::new(ReadAtShim(SliceReadAt::new(&data)));
    let mut sink = CollectSink::default();
    direct.run(&mut *source, &mut sink, None).unwrap();
    assert!(sink.began && sink.finished);
    assert_eq!(sink.buf.len(), 12);
    let expected: Vec<u8> = (1..=12u8).map(|i| i * 10).collect();
    // CollectSink keeps the last strip; the mock writes one strip.
    assert_eq!(sink.buf, expected);

    // Via the push adapter, fed in odd-sized chunks.
    let mut adapter = SeekableIncrementalAdapter::new(MockSeekable::new());
    for chunk in data.chunks(3) {
        let outcome = adapter.push_chunk(chunk, false, None).unwrap();
        assert_eq!(outcome.status, PushStatus::NeedMoreInput);
    }
    let outcome = adapter.push_chunk(&[], true, None).unwrap();
    assert_eq!(outcome.status, PushStatus::OutputReady);
    let (y, slice) = adapter.pull_batch().unwrap().expect("one batch");
    assert_eq!(y, 0);
    assert_eq!(slice.rows(), 3);
    let mut got = Vec::new();
    for row in 0..slice.rows() {
        got.extend_from_slice(slice.row(row));
    }
    assert_eq!(got, expected);
    assert!(adapter.pull_batch().unwrap().is_none());
    assert!(adapter.is_complete());
    assert_eq!(
        adapter.access_pattern(),
        AccessPattern::RandomAccess,
        "adapter must forward the codec's declared access pattern"
    );
}

#[test]
fn provisional_finalizer_forwards_only_final_rows() {
    use zencodec::decode::ProvisionalFinalizer;

    let desc = PixelDescriptor::GRAY8_SRGB;
    let mut sink = CollectSink::default();
    let mut fin = ProvisionalFinalizer::new();
    fin.begin(2, 1, desc, &mut sink).unwrap();

    // Provisional pass: held, nothing reaches the sink.
    let provisional = [7u8, 7u8];
    let slice = PixelSlice::new(&provisional, 2, 1, 2, desc).unwrap();
    let forwarded = fin
        .accept(BatchMeta::new(0, false).with_pass(0), slice, &mut sink)
        .unwrap();
    assert_eq!(forwarded, None);
    assert_eq!(fin.pending_rows(), 1);
    assert!(sink.buf.is_empty());

    // Final pass: supersedes the provisional bytes and forwards.
    let final_bytes = [42u8, 43u8];
    let slice = PixelSlice::new(&final_bytes, 2, 1, 2, desc).unwrap();
    let forwarded = fin
        .accept(BatchMeta::new(0, true).with_pass(1), slice, &mut sink)
        .unwrap();
    assert_eq!(forwarded, Some(1));
    assert_eq!(fin.pending_rows(), 0);
    assert_eq!(sink.buf, final_bytes);
    fin.finish(&mut sink).unwrap();
    assert!(sink.finished);
}

#[test]
fn seekable_capability_defaults_off() {
    let caps = zencodec::decode::DecodeCapabilities::EMPTY;
    assert!(!caps.seekable());
    assert!(!caps.supports(zencodec::UnsupportedOperation::SeekableDecode));
    let caps = caps.with_seekable(true);
    assert!(caps.seekable());
    assert!(caps.supports(zencodec::UnsupportedOperation::SeekableDecode));
}
