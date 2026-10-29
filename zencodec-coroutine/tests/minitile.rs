//! A tiny tiled format decoded by a coroutine, driven every way a driver
//! can drive it. Also pins the failure modes a coroutine decoder must not
//! have: no panic after an error, truncation is an error, consumed input
//! is released, sequential reads ask for one contiguous span, gap bytes
//! between requested ranges are not retained.

use enough::StopReason;
use zencodec::decode::{
    BatchMeta, DecodeRowSink, Event, IncrementalDecode, Input, InputRequest, MetadataUpdate,
    OutputInfo, SinkError, StepError,
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
        ctx.announce(MetadataUpdate::HeaderReady).await?;
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

fn stop() -> &'static dyn zencodec::Pulse {
    &zencodec::how_far::NoPulse
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
    assert_eq!(stats.reads, 3, "{stats:?}");
    assert_eq!(src.calls, 3);
    assert_eq!(src.ranges_served, 4);
    assert_eq!(stats.bytes_read, 16 + 16 + 64);
    assert_eq!(dec.retained_input(), 0, "consumed input is released");
    assert_eq!(dec.info().map(|i| (i.width, i.height)), Some((W, H)));
}

#[test]
fn one_large_window_serves_several_reads_in_one_step() {
    // The whole file offered at once: header, index and both tiles come
    // from the same input; gap bytes are consumed but not retained.
    let file = minitile();
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    let (_, stats) = drive_from_slice(&mut dec, &file, &mut sink, None, file.len()).unwrap();
    assert_eq!(sink.bytes(), &expected_pixels()[..]);
    assert_eq!(dec.retained_input(), 0);
    assert!(stats.steps <= 8, "{stats:?}");
}

#[test]
fn sequential_decode_from_one_byte_windows() {
    let mut file = b"SEQ1".to_vec();
    file.extend_from_slice(&[7u8; 8]);
    for window in [1usize, 3, 64] {
        let mut dec = sequential_decoder();
        let mut sink = CollectSink::new();
        let (info, _) = drive_from_slice(&mut dec, &file, &mut sink, None, window).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(sink.bytes(), &[7u8; 8]);
    }
}

#[test]
fn sequential_reads_ask_for_the_rest_of_the_read() {
    let mut dec = sequential_decoder();
    let mut sink = CollectSink::new();
    let p = dec.step(Input::empty(), &mut sink, Some(stop())).unwrap();
    assert_eq!(p.event, Event::Need);
    assert_eq!(dec.requested(), &[InputRequest::at_least(0, 4)]);
    let p = dec
        .step(Input::new(0, b"SE"), &mut sink, Some(stop()))
        .unwrap();
    assert_eq!((p.consumed, p.event), (2, Event::Need));
    assert_eq!(
        dec.requested(),
        &[InputRequest::at_least(2, 2)],
        "still short: one contiguous request for the rest of the read"
    );
    let p = dec
        .step(Input::new(2, b"Q1"), &mut sink, Some(stop()))
        .unwrap();
    assert_eq!(p.consumed, 2);
    assert_eq!(p.event, Event::Metadata(MetadataUpdate::HeaderReady));
    let p = dec.step(Input::empty(), &mut sink, Some(stop())).unwrap();
    assert_eq!(p.event, Event::Need);
    assert_eq!(dec.requested(), &[InputRequest::at_least(4, 8)]);
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
    assert_eq!(
        dec.step(Input::empty(), &mut sink, Some(stop()))
            .unwrap()
            .event,
        Event::Need
    );
    let e = dec
        .step(Input::new(0, &[1, 2, 3, 4]), &mut sink, Some(stop()))
        .unwrap_err();
    assert!(e.to_string().contains("boom"));
    assert!(matches!(e, StepError::Codec(_)));
    for _ in 0..3 {
        assert!(matches!(
            dec.step(Input::new(4, &[9]), &mut sink, Some(stop())),
            Err(StepError::Finished)
        ));
    }
    assert!(dec.requested().is_empty());
}

#[test]
fn truncated_input_is_an_eof_error_not_a_hang() {
    let mut dec = sequential_decoder();
    let mut sink = CollectSink::new();
    let _ = dec.step(Input::empty(), &mut sink, Some(stop())).unwrap();
    let p = dec
        .step(Input::new(0, b"SEQ1"), &mut sink, Some(stop()))
        .unwrap();
    assert_eq!(p.consumed, 4);
    let _ = dec.step(Input::empty(), &mut sink, Some(stop())).unwrap();
    match dec.step(
        Input::new(4, &[1, 2, 3]).with_total_len(7),
        &mut sink,
        Some(stop()),
    ) {
        Err(StepError::Codec(CoroutineError::Source(SourceError::UnexpectedEof {
            offset,
            len,
            total,
        }))) => {
            assert_eq!((offset, len, total), (4, 8, Some(7)));
        }
        other => panic!("expected UnexpectedEof, got {other:?}"),
    }
    // The same through the slice driver: every input carries the length.
    let mut dec = sequential_decoder();
    let mut sink = CollectSink::new();
    let err = drive_from_slice(&mut dec, b"SEQ1abc", &mut sink, None, 64).unwrap_err();
    assert!(
        matches!(
            err,
            DriveError::Step(StepError::Codec(CoroutineError::Source(
                SourceError::UnexpectedEof { .. }
            )))
        ),
        "{err:?}"
    );
}

#[test]
fn random_read_past_the_end_fails() {
    let file = minitile();
    let truncated = &file[..100];
    // The slice driver gives the length: the decoder reports the overrun.
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    let err = drive_from_slice(&mut dec, truncated, &mut sink, None, 64).unwrap_err();
    assert!(
        matches!(
            err,
            DriveError::Step(StepError::Codec(CoroutineError::Source(
                SourceError::UnexpectedEof { offset: 96, .. }
            )))
        ),
        "{err:?}"
    );
    // A source that does not announce its length reports the overrun itself.
    let mut src = CountingSource {
        inner: SliceReadAt::new(truncated),
        calls: 0,
        ranges_served: 0,
    };
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    let err = drive_from_read_at(&mut dec, &mut src, &mut sink, None, DriveOptions::default())
        .unwrap_err();
    assert!(
        matches!(
            err,
            DriveError::Source(SourceError::UnexpectedEof { offset: 96, .. })
        ),
        "{err:?}"
    );
}

#[test]
fn input_cap_bounds_memory() {
    // Cap of 8 bytes; the decoder reads 4 then 8 sequentially. A 12-byte
    // input is taken in parts so retention never exceeds the cap.
    let make = |cap| -> Dec {
        CoroutineDecoder::boxed_with_options(cap, AccessPattern::Sequential, |mut ctx| async move {
            let mut a = [0u8; 4];
            ctx.read_exact(&mut a).await?;
            let mut b = [0u8; 8];
            ctx.read_exact(&mut b).await?;
            ctx.set_info(ImageInfo::new(2, 1, ImageFormat::Png));
            ctx.set_output_descriptor(RGBA);
            let s = PixelSlice::new(&b, 2, 1, 8, RGBA).unwrap();
            ctx.write_rows(BatchMeta::final_rows(0), &s).await?;
            Ok(OutputInfo::full_decode(2, 1, RGBA))
        })
    };
    let twelve: Vec<u8> = (1..=12).collect();
    let mut dec = make(8);
    let mut sink = CollectSink::new();
    let _ = dec.step(Input::empty(), &mut sink, Some(stop())).unwrap();
    let p = dec
        .step(
            Input::new(0, &twelve).with_total_len(12),
            &mut sink,
            Some(stop()),
        )
        .unwrap();
    assert_eq!(p.consumed, 12);
    assert!(matches!(p.event, Event::Rows { rows: 1, .. }), "{p:?}");
    assert_eq!(sink.bytes(), &twelve[4..]);
    assert_eq!(dec.retained_input(), 0);
    assert!(
        dec.step(Input::empty(), &mut sink, Some(stop()))
            .unwrap()
            .event
            .is_done()
    );

    // A single read larger than the cap can never complete: an error, not a
    // stall.
    let mut dec = make(6);
    let mut sink = CollectSink::new();
    let err = drive_from_slice(&mut dec, &twelve, &mut sink, None, 64).unwrap_err();
    assert!(
        matches!(
            err,
            DriveError::Step(StepError::Codec(CoroutineError::ReadExceedsInputCap {
                len: 8,
                cap: 6
            }))
        ),
        "{err:?}"
    );
}

#[test]
fn gap_bytes_between_ranges_are_not_retained() {
    // A tile walk where every input is a large window containing unrequested
    // bytes: the store holds only what reads still need.
    let n = 40usize;
    let tile = 16usize;
    let mut file = vec![0u8; n * tile * 4];
    for (i, b) in file.iter_mut().enumerate() {
        *b = i as u8;
    }
    let file2 = file.clone();
    let mut dec: Dec = CoroutineDecoder::boxed_with_options(
        64,
        AccessPattern::RandomAccess,
        move |mut ctx| async move {
            ctx.set_info(ImageInfo::new(1, 1, ImageFormat::Png));
            ctx.set_output_descriptor(RGBA);
            let mut sum = 0u64;
            for i in 0..n {
                let mut t = [0u8; 16];
                ctx.read_exact_at((i * tile * 4) as u64, &mut t).await?;
                sum += t.iter().map(|&v| v as u64).sum::<u64>();
            }
            let expected: u64 = (0..n)
                .map(|i| {
                    file2[i * tile * 4..i * tile * 4 + tile]
                        .iter()
                        .map(|&v| v as u64)
                        .sum::<u64>()
                })
                .sum();
            assert_eq!(sum, expected);
            let px = [0u8; 4];
            ctx.write_rows(
                BatchMeta::final_rows(0),
                &PixelSlice::new(&px, 1, 1, 4, RGBA).unwrap(),
            )
            .await?;
            Ok(OutputInfo::full_decode(1, 1, RGBA))
        },
    );
    let mut sink = CollectSink::new();
    let mut src = CountingSource {
        inner: SliceReadAt::new(&file),
        calls: 0,
        ranges_served: 0,
    };
    drive_from_read_at(
        &mut dec,
        &mut src,
        &mut sink,
        None,
        DriveOptions::new(4096, 64 * 1024),
    )
    .unwrap();
    assert_eq!(dec.retained_input(), 0);
}

#[test]
fn one_strip_per_step_and_begin_before_rows() {
    let file = minitile();
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    let mut rows_events = 0;
    let mut input_at = 0usize;
    let mut input_len = 0usize;
    loop {
        let input = Input::new(input_at as u64, &file[input_at..input_at + input_len])
            .with_total_len(file.len() as u64);
        let p = dec.step(input, &mut sink, Some(stop())).unwrap();
        input_at += p.consumed;
        input_len -= p.consumed;
        match p.event {
            Event::Need => {
                let r = match dec.requested()[0] {
                    InputRequest::Range(r) => r,
                    other => panic!("{other:?}"),
                };
                input_at = r.offset as usize;
                input_len = r.length as usize;
            }
            Event::Rows { meta, rows } => {
                rows_events += 1;
                assert_eq!(rows, 2);
                assert_eq!(sink.rows_written(), 2 * rows_events);
                assert_eq!(meta.y, 2 * (rows_events as u32 - 1));
            }
            Event::Metadata(MetadataUpdate::HeaderReady) => {
                assert_eq!(sink.width(), W, "begin ran at HeaderReady, before any rows");
            }
            Event::Done(_) => break,
            _ => {}
        }
    }
    assert_eq!(rows_events, 2);
    assert_eq!(sink.bytes(), &expected_pixels()[..]);
}

#[test]
fn cancellation_keeps_the_stop_reason_and_is_terminal() {
    let stopper = almost_enough::Stopper::new();
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    use almost_enough::Cancellable;
    assert_eq!(
        dec.step(
            Input::empty(),
            &mut sink,
            Some(&zencodec::how_far::StopOnly::borrowed(&stopper))
        )
        .unwrap()
        .event,
        Event::Need
    );
    stopper.stop();
    match dec.step(
        Input::empty(),
        &mut sink,
        Some(&zencodec::how_far::StopOnly::borrowed(&stopper)),
    ) {
        Err(StepError::Cancelled(StopReason::Cancelled)) => {}
        other => panic!("expected Cancelled, got {other:?}"),
    }
    assert!(matches!(
        dec.step(Input::empty(), &mut sink, Some(stop())),
        Err(StepError::Finished)
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
fn provisional_rows_reach_only_a_sink_that_accepts_them() {
    let progressive = |mut ctx: CoroutineContext| async move {
        ctx.set_info(ImageInfo::new(1, 1, ImageFormat::Png));
        ctx.set_output_descriptor(RGBA);
        // Announcing the header calls the sink's `begin`, which is when
        // `accepts_provisional` is read.
        ctx.announce(MetadataUpdate::HeaderReady).await?;
        let coarse = [128u8; 4];
        let fine = [200u8; 4];
        if ctx.accepts_provisional() {
            let s = PixelSlice::new(&coarse, 1, 1, 4, RGBA).unwrap();
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

    // A final-only sink sees the final row once, through provide_next_buffer.
    let mut dec: Dec = CoroutineDecoder::boxed(progressive);
    let mut sink = CollectSink::new();
    drive_from_slice(&mut dec, &[], &mut sink, None, 16).unwrap();
    assert_eq!(sink.rows_written(), 1);
    assert_eq!(sink.bytes(), &[200u8; 4]);
}

#[test]
fn rows_before_the_header_are_an_error() {
    let mut dec: Dec = CoroutineDecoder::boxed(|mut ctx| async move {
        let px = [0u8; 4];
        ctx.write_rows(
            BatchMeta::final_rows(0),
            &PixelSlice::new(&px, 1, 1, 4, RGBA).unwrap(),
        )
        .await?;
        Ok(OutputInfo::full_decode(1, 1, RGBA))
    });
    let mut sink = CollectSink::new();
    let err = drive_from_slice(&mut dec, &[], &mut sink, None, 16).unwrap_err();
    assert!(
        matches!(
            err,
            DriveError::Step(StepError::Codec(CoroutineError::RowsBeforeHeader))
        ),
        "{err:?}"
    );
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
    let data = [0u8, 0, 0, 0, 0, 42, 43];
    let mut src = CountingSource {
        inner: SliceReadAt::new(&data),
        calls: 0,
        ranges_served: 0,
    };
    let mut sink = CollectSink::new();
    let (_, stats) =
        drive_from_read_at(&mut dec, &mut src, &mut sink, None, DriveOptions::default()).unwrap();
    assert_eq!(sink.bytes(), &[42, 43, 0]);
    assert_eq!(stats.bytes_read, 2, "only the tail was read");
}

#[test]
fn stalled_coroutine_is_an_error() {
    let mut dec: Dec = CoroutineDecoder::boxed(|_ctx| async move {
        core::future::pending::<()>().await;
        Ok(OutputInfo::full_decode(1, 1, RGBA))
    });
    let mut sink = CollectSink::new();
    assert!(matches!(
        dec.step(Input::empty(), &mut sink, Some(stop())),
        Err(StepError::Codec(CoroutineError::Stalled))
    ));
}

/// A coroutine that polls `write_rows` once by hand (as a `select` or a
/// timeout would) and returns in the same poll: the row it deposited must
/// still reach the sink, before `Done`.
#[test]
fn rows_deposited_in_the_final_poll_are_delivered() {
    use core::future::Future as _;
    let mut dec: Dec =
        CoroutineDecoder::boxed_with_options(64, AccessPattern::Sequential, |mut ctx| async move {
            ctx.set_info(ImageInfo::new(2, 1, ImageFormat::Png));
            ctx.set_output_descriptor(RGBA);
            ctx.announce(MetadataUpdate::HeaderReady).await?;
            let row = [1u8, 2, 3, 4, 5, 6, 7, 8];
            let s = PixelSlice::new(&row, 2, 1, 8, RGBA).unwrap();
            {
                let fut = ctx.write_rows(BatchMeta::final_rows(0), &s);
                let mut fut = core::pin::pin!(fut);
                core::future::poll_fn(|cx| {
                    let _ = fut.as_mut().poll(cx);
                    core::task::Poll::Ready(())
                })
                .await;
            }
            Ok(OutputInfo::full_decode(2, 1, RGBA))
        });
    let mut sink = CollectSink::new();
    let (info, stats) = drive_from_slice(&mut dec, &[], &mut sink, None, 64).unwrap();
    assert_eq!((info.width, info.height), (2, 1));
    assert_eq!(stats.rows, 1);
    assert_eq!(sink.rows_written(), 1);
    assert_eq!(sink.bytes(), &[1, 2, 3, 4, 5, 6, 7, 8]);
}
