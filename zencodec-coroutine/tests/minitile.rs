//! A tiny tiled format decoded by a coroutine, driven every way a driver
//! can drive it. Also pins the failure modes a coroutine decoder must not
//! have: no panic after an error, truncation is an error, consumed input
//! is released, sequential reads are sequential requests.

use enough::StopReason;
use zencodec::decode::{
    BatchMeta, ByteRange, DecodeRowSink, IncrementalDecode, MetadataUpdate, OutputInfo, SinkError,
    Step,
};
use zencodec::source::{AccessPattern, ReadAt, SliceReadAt, SourceError};
use zencodec::{ImageFormat, ImageInfo};
use zencodec_coroutine::{CoroutineContext, CoroutineDecoder, CoroutineError};
use zencodec_io::{CollectSink, DriveError, DriveOptions, drive_from_read_at, drive_from_slice};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

const W: u32 = 4;
const H: u32 = 4;
const RGBA: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;

/// MiniTile layout (128 bytes):
/// - 0..16   header: "MTIL", width, height, index_offset (all u32 LE)
/// - 32..64  tile 0 (4x2 RGBA8, red)
/// - 80..96  index: [t0_off, t0_len, t1_off, t1_len]
/// - 96..128 tile 1 (4x2 RGBA8, blue)
fn minitile() -> Vec<u8> {
    let mut f = vec![0u8; 128];
    f[0..4].copy_from_slice(b"MTIL");
    f[4..8].copy_from_slice(&W.to_le_bytes());
    f[8..12].copy_from_slice(&H.to_le_bytes());
    f[12..16].copy_from_slice(&80u32.to_le_bytes());
    for p in f[32..64].as_chunks_mut::<4>().0 {
        p.copy_from_slice(&[255, 0, 0, 255]);
    }
    for (i, v) in [32u32, 32, 96, 32].iter().enumerate() {
        f[80 + i * 4..84 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    for p in f[96..128].as_chunks_mut::<4>().0 {
        p.copy_from_slice(&[0, 0, 255, 255]);
    }
    f
}

fn expected_pixels() -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..8 {
        v.extend_from_slice(&[255, 0, 0, 255]);
    }
    for _ in 0..8 {
        v.extend_from_slice(&[0, 0, 255, 255]);
    }
    v
}

type Dec = zencodec_coroutine::BoxedCoroutineDecoder<CoroutineError>;

/// Random-access MiniTile decoder: header, then the trailing index, then
/// both tiles in one batched read.
fn minitile_decoder() -> Dec {
    CoroutineDecoder::boxed(|mut ctx: CoroutineContext| async move {
        let mut header = [0u8; 16];
        ctx.read_exact_at(0, &mut header).await?;
        assert_eq!(&header[0..4], b"MTIL");
        let w = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let h = u32::from_le_bytes(header[8..12].try_into().unwrap());
        let index_at = u32::from_le_bytes(header[12..16].try_into().unwrap()) as u64;
        ctx.set_info(ImageInfo::new(w, h, ImageFormat::Png));
        ctx.set_output_descriptor(RGBA);
        ctx.announce(MetadataUpdate::HeaderReady).await?;

        let mut index = [0u8; 16];
        ctx.read_exact_at(index_at, &mut index).await?;
        let f = |i: usize| u32::from_le_bytes(index[i * 4..i * 4 + 4].try_into().unwrap());
        let (t0, l0, t1, l1) = (f(0) as u64, f(1) as usize, f(2) as u64, f(3) as usize);

        let mut tile0 = vec![0u8; l0];
        let mut tile1 = vec![0u8; l1];
        {
            let mut reqs = [(t0, tile0.as_mut_slice()), (t1, tile1.as_mut_slice())];
            ctx.read_many_at(&mut reqs).await?;
        }
        let stride = w as usize * 4;
        let s0 = PixelSlice::new(&tile0, w, 2, stride, RGBA).unwrap();
        ctx.write_rows(BatchMeta::final_rows(0), &s0).await?;
        let s1 = PixelSlice::new(&tile1, w, 2, stride, RGBA).unwrap();
        ctx.write_rows(BatchMeta::final_rows(2), &s1).await?;
        Ok(OutputInfo::full_decode(w, h, RGBA))
    })
}

/// Sequential decoder: "SEQ1" then 8 bytes of one 2x1 RGBA row.
fn sequential_decoder() -> Dec {
    CoroutineDecoder::boxed_with_options(64, AccessPattern::Sequential, |mut ctx| async move {
        let mut magic = [0u8; 4];
        ctx.read_exact(&mut magic).await?;
        assert_eq!(&magic, b"SEQ1");
        ctx.set_info(ImageInfo::new(2, 1, ImageFormat::Png));
        ctx.set_output_descriptor(RGBA);
        let mut row = [0u8; 8];
        ctx.read_exact(&mut row).await?;
        let s = PixelSlice::new(&row, 2, 1, 8, RGBA).unwrap();
        ctx.write_rows(BatchMeta::final_rows(0), &s).await?;
        Ok(OutputInfo::full_decode(2, 1, RGBA))
    })
}

/// A `ReadAt` that counts calls, so batching is observable.
struct CountingSource<'a> {
    inner: SliceReadAt<'a>,
    calls: usize,
    ranges_served: usize,
}

impl ReadAt for CountingSource<'_> {
    type Error = SourceError;
    fn len(&mut self) -> Result<u64, SourceError> {
        self.inner.len()
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
        self.calls += 1;
        self.ranges_served += 1;
        self.inner.read_exact_at(offset, out)
    }
    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), SourceError> {
        self.calls += 1;
        self.ranges_served += reqs.len();
        self.inner.read_many_at(reqs)
    }
}

#[test]
fn random_access_decode_batches_tiles_in_one_round() {
    let file = minitile();
    let mut src = CountingSource {
        inner: SliceReadAt::new(&file),
        calls: 0,
        ranges_served: 0,
    };
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    let (info, stats) =
        drive_from_read_at(&mut dec, &mut src, &mut sink, None, DriveOptions::default()).unwrap();
    assert_eq!((info.width, info.height), (W, H));
    assert_eq!(sink.bytes(), &expected_pixels()[..]);
    assert!(sink.finished());
    // header, index, tiles: three rounds; the two tiles in one batched call.
    assert_eq!(stats.rounds, 3, "{stats:?}");
    assert_eq!(src.calls, 3);
    assert_eq!(src.ranges_served, 4);
    assert_eq!(stats.bytes_fetched, 16 + 16 + 64);
    assert_eq!(dec.retained_input(), 0, "consumed input is released");
    assert_eq!(dec.info().map(|i| (i.width, i.height)), Some((W, H)));
}

#[test]
fn sequential_decode_from_one_byte_chunks() {
    let mut file = b"SEQ1".to_vec();
    file.extend_from_slice(&[7u8; 8]);
    for chunk in [1usize, 3, 64] {
        let mut dec = sequential_decoder();
        let mut sink = CollectSink::new();
        let (info, stats) = drive_from_slice(&mut dec, &file, &mut sink, None, chunk).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(sink.bytes(), &[7u8; 8]);
        assert!(
            stats.rounds >= 12usize.div_ceil(chunk) as u64,
            "{chunk}: {stats:?}"
        );
    }
}

#[test]
fn sequential_reads_are_zero_length_requests() {
    let mut dec = sequential_decoder();
    let mut sink = CollectSink::new();
    assert_eq!(
        dec.step(&mut sink, None).unwrap(),
        Step::Need(&[ByteRange::from_offset(0)])
    );
    assert_eq!(dec.feed(0, b"SE", false).unwrap(), 2);
    assert_eq!(
        dec.step(&mut sink, None).unwrap(),
        Step::Need(&[ByteRange::from_offset(2)]),
        "still short: ask from the first missing byte, not for a sized range"
    );
    assert_eq!(dec.feed(2, b"Q1", false).unwrap(), 2);
    assert_eq!(
        dec.step(&mut sink, None).unwrap(),
        Step::Need(&[ByteRange::from_offset(4)])
    );
}

#[test]
fn error_is_terminal_and_never_panics() {
    let mut dec: Dec = CoroutineDecoder::boxed(|mut ctx| async move {
        let mut b = [0u8; 4];
        ctx.read_exact_at(0, &mut b).await?;
        Err(CoroutineError::Source(SourceError::Io {
            message: "boom".into(),
        }))
    });
    let mut sink = CollectSink::new();
    assert!(matches!(dec.step(&mut sink, None).unwrap(), Step::Need(_)));
    dec.feed(0, &[1, 2, 3, 4], false).unwrap();
    let e = dec.step(&mut sink, None).unwrap_err();
    assert!(e.to_string().contains("boom"));
    for _ in 0..3 {
        assert!(matches!(
            dec.step(&mut sink, None),
            Err(CoroutineError::Finished)
        ));
    }
    assert_eq!(
        dec.feed(4, &[9], false).unwrap(),
        1,
        "late bytes are swallowed, not refused"
    );
}

#[test]
fn truncated_input_is_an_eof_error_not_a_hang() {
    let mut dec = sequential_decoder();
    let mut sink = CollectSink::new();
    let _ = dec.step(&mut sink, None).unwrap();
    assert_eq!(dec.feed(0, b"SEQ1", false).unwrap(), 4);
    let _ = dec.step(&mut sink, None).unwrap();
    assert_eq!(dec.feed(4, &[1, 2, 3], true).unwrap(), 3);
    match dec.step(&mut sink, None) {
        Err(CoroutineError::Source(SourceError::UnexpectedEof { offset, len, total })) => {
            assert_eq!((offset, len, total), (4, 8, Some(7)));
        }
        other => panic!("expected UnexpectedEof, got {other:?}"),
    }
    // The same through the slice driver: it announces the length.
    let mut dec = sequential_decoder();
    let mut sink = CollectSink::new();
    let err = drive_from_slice(&mut dec, b"SEQ1abc", &mut sink, None, 64).unwrap_err();
    assert!(matches!(
        err,
        DriveError::Codec(CoroutineError::Source(SourceError::UnexpectedEof { .. }))
    ));
}

#[test]
fn random_read_past_announced_end_fails() {
    let file = minitile();
    let mut truncated = file.clone();
    truncated.truncate(100);
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    // The driver fetches sized ranges without asking the source's length
    // first, so the source reports the overrun.
    let err = drive_from_slice(&mut dec, &truncated, &mut sink, None, 64).unwrap_err();
    assert!(
        matches!(
            err,
            DriveError::Source(SourceError::UnexpectedEof { offset: 96, .. })
        ),
        "{err:?}"
    );
    // A decoder told the length up front reports the overrun itself.
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    dec.feed(100, &[], true).unwrap();
    let err = drive_from_slice(&mut dec, &truncated, &mut sink, None, 64).unwrap_err();
    assert!(
        matches!(
            err,
            DriveError::Codec(CoroutineError::Source(SourceError::UnexpectedEof {
                offset: 96,
                ..
            }))
        ),
        "{err:?}"
    );
}

#[test]
fn input_cap_bounds_memory_and_feed_accepts_partially() {
    // Cap of 8 bytes; the decoder reads 4 then 8 sequentially.
    let mut dec: Dec =
        CoroutineDecoder::boxed_with_options(8, AccessPattern::Sequential, |mut ctx| async move {
            let mut a = [0u8; 4];
            ctx.read_exact(&mut a).await?;
            let mut b = [0u8; 8];
            ctx.read_exact(&mut b).await?;
            ctx.set_info(ImageInfo::new(2, 1, ImageFormat::Png));
            ctx.set_output_descriptor(RGBA);
            let s = PixelSlice::new(&b, 2, 1, 8, RGBA).unwrap();
            ctx.write_rows(BatchMeta::final_rows(0), &s).await?;
            Ok(OutputInfo::full_decode(2, 1, RGBA))
        });
    let mut sink = CollectSink::new();
    let _ = dec.step(&mut sink, None).unwrap();
    let twelve: Vec<u8> = (1..=12).collect();
    assert_eq!(
        dec.feed(0, &twelve, true).unwrap(),
        8,
        "cap: only 8 accepted"
    );
    assert_eq!(dec.retained_input(), 8);
    assert!(matches!(dec.step(&mut sink, None).unwrap(), Step::Need(_)));
    assert_eq!(
        dec.retained_input(),
        4,
        "the 4-byte read released its bytes"
    );
    assert_eq!(dec.feed(8, &twelve[8..], true).unwrap(), 4);
    assert!(matches!(
        dec.step(&mut sink, None).unwrap(),
        Step::Rows { rows: 1, .. }
    ));
    assert_eq!(sink.bytes(), &twelve[4..]);
    assert_eq!(dec.retained_input(), 0);
    assert!(matches!(dec.step(&mut sink, None).unwrap(), Step::Done(_)));
}

#[test]
fn one_strip_per_step_is_the_backpressure() {
    let file = minitile();
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    let mut rows_events = 0;
    let mut src = SliceReadAt::new(&file);
    loop {
        match dec.step(&mut sink, None).unwrap() {
            Step::Need(ranges) => {
                let ranges = ranges.to_vec();
                for r in ranges {
                    let mut buf = vec![0u8; r.length as usize];
                    src.read_exact_at(r.offset, &mut buf).unwrap();
                    dec.feed(r.offset, &buf, false).unwrap();
                }
            }
            Step::Rows { meta, rows } => {
                rows_events += 1;
                assert_eq!(rows, 2);
                assert_eq!(sink.rows_written(), 2 * rows_events);
                assert_eq!(meta.y, 2 * (rows_events as u32 - 1));
            }
            Step::Metadata(MetadataUpdate::HeaderReady) => {
                assert_eq!(sink.width(), W, "begin ran at HeaderReady, before any rows");
            }
            Step::Done(_) => break,
            _ => {}
        }
    }
    assert_eq!(rows_events, 2);
    assert_eq!(sink.bytes(), &expected_pixels()[..]);
}

#[test]
fn cancellation_keeps_the_stop_reason() {
    let stopper = almost_enough::Stopper::new();
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    use almost_enough::Cancellable;
    assert!(matches!(
        dec.step(&mut sink, Some(&stopper)).unwrap(),
        Step::Need(_)
    ));
    stopper.stop();
    match dec.step(&mut sink, Some(&stopper)) {
        Err(CoroutineError::Cancelled(StopReason::Cancelled)) => {}
        other => panic!("expected Cancelled, got {other:?}"),
    }
    assert!(matches!(
        dec.step(&mut sink, None),
        Err(CoroutineError::Finished)
    ));
}

/// A sink that records every pass it receives.
#[derive(Default)]
struct PassSink {
    passes: Vec<(BatchMeta, Vec<u8>)>,
    scratch: Vec<u8>,
}

impl DecodeRowSink for PassSink {
    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.provide_pass_buffer(BatchMeta::final_rows(y), height, width, descriptor)
    }
    fn accepts_provisional(&self) -> bool {
        true
    }
    fn provide_pass_buffer(
        &mut self,
        meta: BatchMeta,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        if let Some(last) = self.passes.last_mut() {
            last.1 = self.scratch.clone();
        }
        let stride = width as usize * descriptor.bytes_per_pixel();
        self.scratch = vec![0u8; stride * height as usize];
        self.passes.push((meta, Vec::new()));
        PixelSliceMut::new(&mut self.scratch, width, height, stride, descriptor)
            .map_err(|e| e.to_string().into())
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        if let Some(last) = self.passes.last_mut() {
            last.1 = self.scratch.clone();
        }
        Ok(())
    }
}

#[test]
fn provisional_rows_reach_a_sink_that_accepts_them() {
    let progressive = |mut ctx: CoroutineContext| async move {
        ctx.set_info(ImageInfo::new(1, 1, ImageFormat::Png));
        ctx.set_output_descriptor(RGBA);
        let coarse = [128u8; 4];
        let fine = [200u8; 4];
        let s = PixelSlice::new(&coarse, 1, 1, 4, RGBA).unwrap();
        if ctx.accepts_provisional() {
            ctx.write_rows(BatchMeta::new(0, false).with_pass(0), &s)
                .await?;
        }
        let s = PixelSlice::new(&fine, 1, 1, 4, RGBA).unwrap();
        ctx.write_rows(BatchMeta::new(0, true).with_pass(1), &s)
            .await?;
        Ok(OutputInfo::full_decode(1, 1, RGBA))
    };
    let mut dec: Dec = CoroutineDecoder::boxed(progressive);
    let mut sink = PassSink::default();
    drive_from_slice(&mut dec, &[], &mut sink, None, 16).unwrap();
    assert_eq!(sink.passes.len(), 2);
    assert!(!sink.passes[0].0.is_final);
    assert_eq!(sink.passes[0].1, [128u8; 4]);
    assert!(sink.passes[1].0.is_final);
    assert_eq!(sink.passes[1].1, [200u8; 4]);

    // A final-only sink sees the final row once.
    let mut dec: Dec = CoroutineDecoder::boxed(progressive);
    let mut sink = CollectSink::new();
    drive_from_slice(&mut dec, &[], &mut sink, None, 16).unwrap();
    assert_eq!(sink.rows_written(), 1);
    assert_eq!(sink.bytes(), &[200u8; 4]);
}

#[test]
fn len_request_is_served_by_drivers() {
    let mut dec: Dec = CoroutineDecoder::boxed(|mut ctx| async move {
        let len = ctx.len().await?;
        let mut tail = [0u8; 2];
        ctx.read_exact_at(len - 2, &mut tail).await?;
        ctx.set_info(ImageInfo::new(1, 1, ImageFormat::Png));
        ctx.set_output_descriptor(PixelDescriptor::RGB8_SRGB);
        let px = [tail[0], tail[1], 0];
        let s = PixelSlice::new(&px, 1, 1, 3, PixelDescriptor::RGB8_SRGB).unwrap();
        ctx.write_rows(BatchMeta::final_rows(0), &s).await?;
        Ok(OutputInfo::full_decode(1, 1, PixelDescriptor::RGB8_SRGB))
    });
    let mut sink = CollectSink::new();
    let data = [0u8, 0, 0, 0, 0, 42, 43];
    let (_, stats) = drive_from_slice(&mut dec, &data, &mut sink, None, 64).unwrap();
    assert_eq!(sink.bytes(), &[42, 43, 0]);
    assert_eq!(stats.bytes_fetched, 2, "only the tail was fetched");
}

#[test]
fn stalled_coroutine_is_an_error() {
    let mut dec: Dec = CoroutineDecoder::boxed(|_ctx| async move {
        core::future::pending::<()>().await;
        Ok(OutputInfo::full_decode(1, 1, RGBA))
    });
    let mut sink = CollectSink::new();
    assert!(matches!(
        dec.step(&mut sink, None),
        Err(CoroutineError::Stalled)
    ));
}
