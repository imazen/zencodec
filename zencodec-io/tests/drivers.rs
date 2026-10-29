//! Every driver shape over the same two decoders: a random-access tiled
//! format and a sequential one, both written as coroutines. Hand-written
//! decoders pin partial consumption and the protocol checks that stop a
//! misbehaving decoder from spinning a driver.

#![cfg(feature = "std")]

use how_far::Report as _;
use std::io::Cursor;

use zencodec::decode::{
    BatchMeta, DecodeRowSink, Event, IncrementalDecode, Input, InputRequest, MetadataUpdate,
    OutputInfo, Progress, SinkError, StepError,
};
use zencodec::source::{AccessPattern, ReadAt, SliceReadAt, SourceError};
use zencodec::{ImageFormat, ImageInfo};
use zencodec_coroutine::{
    BoxedCoroutineDecoder, CoroutineContext, CoroutineDecoder, CoroutineError,
};
use zencodec_io::{
    BufferPolicy, BufferedReadAt, CollectSink, DriveError, DriveOptions, ProtocolViolation,
    PumpError, StdSourceError, drive_from_read_at, drive_from_slice, pump_read_to_sink,
};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

const W: u32 = 4;
const H: u32 = 4;
const RGBA: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;

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

type Dec = BoxedCoroutineDecoder<CoroutineError>;

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
        ctx.write_rows(
            BatchMeta::final_rows(0),
            &PixelSlice::new(&tile0, w, 2, stride, RGBA).unwrap(),
        )
        .await?;
        ctx.write_rows(
            BatchMeta::final_rows(2),
            &PixelSlice::new(&tile1, w, 2, stride, RGBA).unwrap(),
        )
        .await?;
        Ok(OutputInfo::full_decode(w, h, RGBA))
    })
}

/// "SEQ1" + N rows of 2 RGBA pixels, each row 8 bytes, written one row at a time.
fn sequential_file(rows: u32) -> Vec<u8> {
    let mut f = b"SEQ1".to_vec();
    f.extend_from_slice(&rows.to_le_bytes());
    for y in 0..rows {
        f.extend_from_slice(&[y as u8; 8]);
    }
    f
}

fn sequential_decoder() -> Dec {
    CoroutineDecoder::boxed_with_options(64, AccessPattern::Sequential, |mut ctx| async move {
        let mut magic = [0u8; 8];
        ctx.read_exact(&mut magic).await?;
        assert_eq!(&magic[..4], b"SEQ1");
        let rows = u32::from_le_bytes(magic[4..8].try_into().unwrap());
        ctx.set_info(ImageInfo::new(2, rows, ImageFormat::Png));
        ctx.set_output_descriptor(RGBA);
        ctx.announce(MetadataUpdate::HeaderReady).await?;
        for y in 0..rows {
            let mut row = [0u8; 8];
            ctx.read_exact(&mut row).await?;
            let s = PixelSlice::new(&row, 2, 1, 8, RGBA).unwrap();
            ctx.write_rows(BatchMeta::final_rows(y), &s).await?;
        }
        Ok(OutputInfo::full_decode(2, rows, RGBA))
    })
}

/// A `std::io::Read` that hands out at most `chunk` bytes per call.
struct Dribble<'a> {
    data: &'a [u8],
    pos: usize,
    chunk: usize,
}

impl std::io::Read for Dribble<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = (self.data.len() - self.pos).min(buf.len()).min(self.chunk);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// A `ReadAt` that is not in memory as far as drivers can tell, so the
/// read path runs instead of the zero-copy slice path.
struct Opaque<'a>(SliceReadAt<'a>);

impl ReadAt for Opaque<'_> {
    type Error = SourceError;
    fn len(&mut self) -> Result<u64, SourceError> {
        self.0.len()
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
        self.0.read_exact_at(offset, out)
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, SourceError> {
        self.0.read_at(offset, out)
    }
}

#[test]
fn stream_pump_decodes_as_bytes_arrive() {
    let file = sequential_file(6);
    for chunk in [1usize, 5, 512] {
        let mut dec = sequential_decoder();
        let mut sink = CollectSink::new();
        let reader = Dribble {
            data: &file,
            pos: 0,
            chunk,
        };
        let info = pump_read_to_sink(reader, &mut dec, &mut sink, None, 512, 1 << 20).unwrap();
        assert_eq!((info.width, info.height), (2, 6));
        for y in 0..6 {
            assert_eq!(
                &sink.bytes()[y * 8..y * 8 + 8],
                &[y as u8; 8],
                "chunk {chunk}"
            );
        }
        assert!(sink.finished());
    }
}

#[test]
fn stream_pump_emits_rows_before_the_stream_ends() {
    struct FirstRowAt {
        inner: CollectSink,
        first_row_seen: Option<usize>,
        bytes_read: std::rc::Rc<std::cell::Cell<usize>>,
    }
    impl DecodeRowSink for FirstRowAt {
        fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
            self.inner.begin(w, h, d)
        }
        fn provide_next_buffer(
            &mut self,
            y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            if self.first_row_seen.is_none() {
                self.first_row_seen = Some(self.bytes_read.get());
            }
            self.inner.provide_next_buffer(y, height, width, descriptor)
        }
        fn finish(&mut self) -> Result<(), SinkError> {
            self.inner.finish()
        }
    }
    struct Tap<'a>(Dribble<'a>, std::rc::Rc<std::cell::Cell<usize>>);
    impl std::io::Read for Tap<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.read(buf)?;
            self.1.set(self.1.get() + n);
            Ok(n)
        }
    }
    let file = sequential_file(32);
    let mut dec = sequential_decoder();
    let bytes_read = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let mut sink = FirstRowAt {
        inner: CollectSink::new(),
        first_row_seen: None,
        bytes_read: bytes_read.clone(),
    };
    let reader = Tap(
        Dribble {
            data: &file,
            pos: 0,
            chunk: 8,
        },
        bytes_read.clone(),
    );
    pump_read_to_sink(reader, &mut dec, &mut sink, None, 512, 1 << 20).unwrap();
    let first = sink.first_row_seen.unwrap();
    assert!(
        first <= 16,
        "first row after the header and one row, not after {first} of {} bytes",
        file.len()
    );
    assert_eq!(sink.inner.rows_written(), 32);
}

#[test]
fn stream_pump_rejects_random_access_with_a_pointer_to_the_fix() {
    let file = minitile();
    // Header, then the index ahead of it, then two tiles: the pump skips
    // forward to the index and to the second tile, but the first tile is
    // behind the stream position by then.
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    let err =
        pump_read_to_sink(Cursor::new(&file), &mut dec, &mut sink, None, 512, 1 << 20).unwrap_err();
    assert!(
        matches!(err, PumpError::NeedsRandomAccess(InputRequest::Range(r)) if r.offset == 32),
        "{err:?}"
    );
    assert!(err.to_string().contains("random access"), "{err}");

    // The fix: buffer the stream and drive it as a source.
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    let mut source =
        BufferedReadAt::new(Cursor::new(&file), BufferPolicy::Spool { max_bytes: 4096 });
    drive_from_read_at(
        &mut dec,
        &mut source,
        &mut sink,
        None,
        DriveOptions::default(),
    )
    .unwrap();
    assert_eq!(sink.bytes(), &expected_pixels()[..]);
}

#[test]
fn stream_pump_enforces_its_window_limit() {
    let file = sequential_file(4);
    let mut dec: Dec = CoroutineDecoder::boxed_with_options(
        1 << 20,
        AccessPattern::Sequential,
        |mut ctx| async move {
            let mut big = vec![0u8; 4096];
            ctx.read_exact(&mut big).await?;
            Ok(OutputInfo::full_decode(1, 1, RGBA))
        },
    );
    let mut sink = CollectSink::new();
    let err =
        pump_read_to_sink(Cursor::new(&file), &mut dec, &mut sink, None, 512, 1024).unwrap_err();
    assert!(
        matches!(
            err,
            PumpError::WindowLimit {
                needed: 4096,
                limit: 1024
            }
        ),
        "{err:?}"
    );
}

#[test]
fn every_driver_decodes_both_shapes() {
    let tiled = minitile();
    let seq = sequential_file(5);
    let seq_pixels: Vec<u8> = (0..5u8).flat_map(|y| [y; 8]).collect();
    for window in [1usize, 3, 16, 4096] {
        let mut sink = CollectSink::new();
        drive_from_slice(&mut minitile_decoder(), &tiled, &mut sink, None, window).unwrap();
        assert_eq!(sink.bytes(), &expected_pixels()[..], "window {window}");
        let mut sink = CollectSink::new();
        drive_from_slice(&mut sequential_decoder(), &seq, &mut sink, None, window).unwrap();
        assert_eq!(sink.bytes(), &seq_pixels[..], "window {window}");
    }
    for read_size in [1usize, 7, 64 * 1024] {
        let opts = DriveOptions::new(0, read_size);
        let mut sink = CollectSink::new();
        let mut src = Opaque(SliceReadAt::new(&tiled));
        drive_from_read_at(&mut minitile_decoder(), &mut src, &mut sink, None, opts).unwrap();
        assert_eq!(sink.bytes(), &expected_pixels()[..], "read {read_size}");
        let mut sink = CollectSink::new();
        let mut src = Opaque(SliceReadAt::new(&seq));
        drive_from_read_at(&mut sequential_decoder(), &mut src, &mut sink, None, opts).unwrap();
        assert_eq!(sink.bytes(), &seq_pixels[..], "read {read_size}");
    }
}

#[test]
fn slice_driver_is_zero_copy_for_in_memory_sources() {
    // A SliceReadAt reports itself as a slice: the read_at driver hands
    // the decoder sub-slices instead of reading into a window.
    let file = minitile();
    let mut src = SliceReadAt::new(&file);
    let mut sink = CollectSink::new();
    let (_, stats) = drive_from_read_at(
        &mut minitile_decoder(),
        &mut src,
        &mut sink,
        None,
        DriveOptions::default(),
    )
    .unwrap();
    assert_eq!(sink.bytes(), &expected_pixels()[..]);
    assert!(stats.steps > 0);
}

#[test]
fn buffered_window_serves_forward_and_refuses_far_back() {
    let data: Vec<u8> = (0..=255u8).collect();
    let mut src = BufferedReadAt::new(
        Cursor::new(&data),
        BufferPolicy::Window { window_bytes: 16 },
    );
    let mut buf = [0u8; 4];
    src.read_exact_at(100, &mut buf).unwrap();
    assert_eq!(buf, [100, 101, 102, 103]);
    assert_eq!(src.watermark(), 104);
    src.read_exact_at(96, &mut buf).unwrap();
    assert_eq!(buf, [96, 97, 98, 99], "within the window");
    let err = src.read_exact_at(0, &mut buf).unwrap_err();
    assert!(
        matches!(
            err,
            StdSourceError::Source(SourceError::BeyondWindow { offset: 0, .. })
        ),
        "{err:?}"
    );
    let mut past = [0u8; 2];
    let err = src.read_exact_at(255, &mut past).unwrap_err();
    assert!(
        matches!(
            err,
            StdSourceError::Source(SourceError::UnexpectedEof { .. })
        ),
        "{err:?}"
    );

    let mut spool = BufferedReadAt::new(Cursor::new(&data), BufferPolicy::Spool { max_bytes: 32 });
    let err = spool.read_exact_at(40, &mut buf).unwrap_err();
    assert!(
        matches!(
            err,
            StdSourceError::Source(SourceError::BeyondLimit { limit: 32, .. })
        ),
        "{err:?}"
    );

    let mut spool =
        BufferedReadAt::new(Cursor::new(&data), BufferPolicy::Spool { max_bytes: 1024 });
    assert_eq!(spool.len().unwrap(), 256);
    spool.read_exact_at(0, &mut buf).unwrap();
    assert_eq!(buf, [0, 1, 2, 3]);
}

#[test]
fn buffered_policy_for_pattern_matches_declared_access() {
    assert!(matches!(
        BufferPolicy::for_pattern(AccessPattern::Sequential, 1 << 20),
        BufferPolicy::Window { .. }
    ));
    assert!(matches!(
        BufferPolicy::for_pattern(AccessPattern::RandomAccess, 1 << 20),
        BufferPolicy::Spool { max_bytes } if max_bytes == 1 << 20
    ));
    let mut dec = minitile_decoder();
    let policy = BufferPolicy::for_pattern(dec.access_pattern(), 4096);
    let file = minitile();
    let mut source = BufferedReadAt::new(Cursor::new(&file), policy);
    let mut sink = CollectSink::new();
    drive_from_read_at(
        &mut dec,
        &mut source,
        &mut sink,
        None,
        DriveOptions::default(),
    )
    .unwrap();
    assert_eq!(sink.bytes(), &expected_pixels()[..]);
}

/// Takes at most `cap` bytes per step, collecting them, until `total`
/// bytes have arrived; then writes one row and finishes. Pins that every
/// driver re-offers what a decoder did not take.
struct Trickle {
    cap: usize,
    total: usize,
    got: Vec<u8>,
    req: [InputRequest; 1],
    begun: bool,
    done: bool,
}

impl Trickle {
    fn new(cap: usize, total: usize) -> Self {
        Self {
            cap,
            total,
            got: Vec::new(),
            req: [InputRequest::at_least(0, 1)],
            begun: false,
            done: false,
        }
    }
}

#[derive(Debug)]
struct Never;
impl std::fmt::Display for Never {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("never")
    }
}
impl std::error::Error for Never {}

impl IncrementalDecode for Trickle {
    type Error = Never;
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        _stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<Progress, StepError<Never>> {
        if self.done {
            return Ok(Progress::new(
                0,
                Event::Done(OutputInfo::full_decode(1, 1, RGBA)),
            ));
        }
        let pos = self.got.len() as u64;
        if self.got.len() == self.total {
            if !self.begun {
                sink.begin(1, 1, RGBA).map_err(StepError::Sink)?;
                self.begun = true;
                let mut dst = sink
                    .provide_next_buffer(0, 1, 1, RGBA)
                    .map_err(StepError::Sink)?;
                dst.row_mut(0).copy_from_slice(&[1, 2, 3, 4]);
                return Ok(Progress::new(
                    0,
                    Event::Rows {
                        meta: BatchMeta::final_rows(0),
                        rows: 1,
                    },
                ));
            }
            sink.finish().map_err(StepError::Sink)?;
            self.done = true;
            return Ok(Progress::new(
                0,
                Event::Done(OutputInfo::full_decode(1, 1, RGBA)),
            ));
        }
        if input.offset != pos || input.bytes.is_empty() {
            self.req = [InputRequest::at_least(pos, 1)];
            return Ok(Progress::need(0));
        }
        let n = input
            .bytes
            .len()
            .min(self.cap)
            .min(self.total - self.got.len());
        self.got.extend_from_slice(&input.bytes[..n]);
        self.req = [InputRequest::at_least(self.got.len() as u64, 1)];
        Ok(Progress::new(n, Event::Need))
    }
    fn requested(&self) -> &[InputRequest] {
        &self.req
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        Some(RGBA)
    }
    fn access_pattern(&self) -> AccessPattern {
        AccessPattern::Sequential
    }
}

#[test]
fn drivers_re_offer_what_the_decoder_did_not_take() {
    let data: Vec<u8> = (0..200u8).collect();
    for cap in [1usize, 3, 64] {
        let mut dec = Trickle::new(cap, data.len());
        let mut sink = CollectSink::new();
        drive_from_slice(&mut dec, &data, &mut sink, None, 50).unwrap();
        assert_eq!(dec.got, data, "slice, cap {cap}");

        let mut dec = Trickle::new(cap, data.len());
        let mut sink = CollectSink::new();
        let mut src = Opaque(SliceReadAt::new(&data));
        drive_from_read_at(
            &mut dec,
            &mut src,
            &mut sink,
            None,
            DriveOptions::new(0, 37),
        )
        .unwrap();
        assert_eq!(dec.got, data, "read_at, cap {cap}");

        let mut dec = Trickle::new(cap, data.len());
        let mut sink = CollectSink::new();
        let reader = Dribble {
            data: &data,
            pos: 0,
            chunk: 13,
        };
        pump_read_to_sink(reader, &mut dec, &mut sink, None, 512, 1024).unwrap();
        assert_eq!(dec.got, data, "pump, cap {cap}");
        assert_eq!(sink.bytes(), &[1, 2, 3, 4]);
    }
}

/// Decoders that break the protocol in one specific way.
#[derive(Clone, Copy, Debug)]
enum Misbehave {
    /// `Need` with an empty request list.
    EmptyRequest,
    /// `Need` for bytes it was just given, consuming none.
    Spin,
    /// `Need` for bytes past the announced end.
    PastEnd,
}

struct Bad {
    how: Misbehave,
    req: Vec<InputRequest>,
}

impl IncrementalDecode for Bad {
    type Error = Never;
    fn step(
        &mut self,
        input: Input<'_>,
        _sink: &mut dyn DecodeRowSink,
        _stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<Progress, StepError<Never>> {
        self.req = match self.how {
            Misbehave::EmptyRequest => Vec::new(),
            Misbehave::Spin => vec![InputRequest::at_least(0, 4)],
            Misbehave::PastEnd => vec![InputRequest::range(
                input.total_len.unwrap_or(1 << 20) + 10,
                4,
            )],
        };
        Ok(Progress::need(0))
    }
    fn requested(&self) -> &[InputRequest] {
        &self.req
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        None
    }
    fn access_pattern(&self) -> AccessPattern {
        AccessPattern::RandomAccess
    }
}

#[test]
fn protocol_violations_are_errors_not_hangs() {
    let data = [0u8; 64];
    for how in [Misbehave::EmptyRequest, Misbehave::Spin, Misbehave::PastEnd] {
        let expect = |v: &ProtocolViolation| match how {
            Misbehave::EmptyRequest => matches!(v, ProtocolViolation::EmptyRequest),
            Misbehave::Spin => matches!(v, ProtocolViolation::NoProgress(_)),
            Misbehave::PastEnd => matches!(v, ProtocolViolation::NeedPastEnd { .. }),
        };
        let mut sink = CollectSink::new();
        let mut dec = Bad {
            how,
            req: Vec::new(),
        };
        match drive_from_slice(&mut dec, &data, &mut sink, None, 64) {
            Err(DriveError::Protocol(v)) => assert!(expect(&v), "slice {how:?}: {v:?}"),
            other => panic!("slice {how:?}: {other:?}"),
        }
        let mut dec = Bad {
            how,
            req: Vec::new(),
        };
        let mut src = Opaque(SliceReadAt::new(&data));
        match drive_from_read_at(&mut dec, &mut src, &mut sink, None, DriveOptions::default()) {
            Err(DriveError::Protocol(v)) => {
                // Without a length hint the read_at driver cannot see the
                // end until it reads; a past-end range is then the
                // source's error.
                assert!(expect(&v), "read_at {how:?}: {v:?}")
            }
            Err(DriveError::Source(SourceError::UnexpectedEof { .. }))
                if matches!(how, Misbehave::PastEnd) => {}
            other => panic!("read_at {how:?}: {other:?}"),
        }
        let mut dec = Bad {
            how,
            req: Vec::new(),
        };
        match pump_read_to_sink(Cursor::new(&data), &mut dec, &mut sink, None, 512, 1024) {
            Err(PumpError::Protocol(v)) => assert!(expect(&v), "pump {how:?}: {v:?}"),
            other => panic!("pump {how:?}: {other:?}"),
        }
    }
}

#[test]
fn truncated_streams_are_codec_errors_in_every_driver() {
    let file = sequential_file(6);
    let cut = &file[..file.len() - 3];
    let is_eof = |e: &StepError<CoroutineError>| {
        matches!(
            e,
            StepError::Codec(CoroutineError::Source(SourceError::UnexpectedEof { .. }))
        )
    };
    let mut sink = CollectSink::new();
    match drive_from_slice(&mut sequential_decoder(), cut, &mut sink, None, 7) {
        Err(DriveError::Step(e)) => assert!(is_eof(&e), "{e:?}"),
        other => panic!("{other:?}"),
    }
    let mut sink = CollectSink::new();
    let mut src = Opaque(SliceReadAt::new(cut));
    match drive_from_read_at(
        &mut sequential_decoder(),
        &mut src,
        &mut sink,
        None,
        DriveOptions::new(0, 7),
    ) {
        Err(DriveError::Step(e)) => assert!(is_eof(&e), "{e:?}"),
        other => panic!("{other:?}"),
    }
    let mut sink = CollectSink::new();
    match pump_read_to_sink(
        Cursor::new(cut),
        &mut sequential_decoder(),
        &mut sink,
        None,
        512,
        1024,
    ) {
        Err(PumpError::Step(e)) => assert!(is_eof(&e), "{e:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_tripped_pulse_cancels_every_driver() {
    let file = sequential_file(6);
    let pulse = RecordingPulse::default();
    pulse.cancel();
    let cancelled = |e: &StepError<CoroutineError>| {
        matches!(e, StepError::Cancelled(enough::StopReason::Cancelled))
    };
    let mut sink = CollectSink::new();
    match drive_from_slice(
        &mut sequential_decoder(),
        &file,
        &mut sink,
        Some(&pulse),
        64,
    ) {
        Err(DriveError::Step(e)) => assert!(cancelled(&e), "{e:?}"),
        other => panic!("{other:?}"),
    }
    let mut src = Opaque(SliceReadAt::new(&file));
    match drive_from_read_at(
        &mut sequential_decoder(),
        &mut src,
        &mut sink,
        Some(&pulse),
        DriveOptions::default(),
    ) {
        Err(DriveError::Step(e)) => assert!(cancelled(&e), "{e:?}"),
        other => panic!("{other:?}"),
    }
    match pump_read_to_sink(
        Cursor::new(&file),
        &mut sequential_decoder(),
        &mut sink,
        Some(&pulse),
        512,
        1024,
    ) {
        Err(PumpError::Step(e)) => assert!(cancelled(&e), "{e:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn drivers_report_total_and_progress() {
    let file = sequential_file(6);
    let pulse = RecordingPulse::default();
    let mut dec = sequential_decoder();
    let mut sink = CollectSink::new();
    drive_from_slice(&mut dec, &file, &mut sink, Some(&pulse), 5).unwrap();
    assert_eq!(pulse.total(), Some(how_far::Total::Exact(6)));
    assert_eq!(pulse.advanced(), 6);

    let pulse = RecordingPulse::default();
    let mut dec = sequential_decoder();
    let mut sink = CollectSink::new();
    pump_read_to_sink(
        Cursor::new(&file),
        &mut dec,
        &mut sink,
        Some(&pulse),
        512,
        1024,
    )
    .unwrap();
    assert_eq!(pulse.total(), Some(how_far::Total::Exact(6)));
    assert_eq!(pulse.advanced(), 6);

    // A plain Stop still works through the StopOnly adapter.
    let stopper = almost_enough::Stopper::new();
    let adapter = how_far::StopOnly::borrowed(&stopper);
    let mut dec = sequential_decoder();
    let mut sink = CollectSink::new();
    drive_from_slice(&mut dec, &file, &mut sink, Some(&adapter), 64).unwrap();
    assert_eq!(sink.rows_written(), 6);
}

mod pulse_support {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use enough::{Stop, StopReason};
    use how_far::{Child, Execution, PhaseSpec, PlanError, Pulse, Report, SharedPulse, Total};

    #[derive(Default, Debug)]
    struct Inner {
        cancelled: AtomicBool,
        advanced: AtomicU64,
        splits: AtomicU64,
        shares: AtomicU64,
        total: Mutex<Option<Total>>,
    }

    #[derive(Clone, Default, Debug)]
    pub struct RecordingPulse(Arc<Inner>);

    impl RecordingPulse {
        #[allow(dead_code)] // used by the tokio cancellation tests only
        pub fn cancel(&self) {
            self.0.cancelled.store(true, Ordering::SeqCst);
        }
        pub fn advanced(&self) -> u64 {
            self.0.advanced.load(Ordering::SeqCst)
        }
        pub fn total(&self) -> Option<Total> {
            *self.0.total.lock().unwrap()
        }
        pub fn splits(&self) -> u64 {
            self.0.splits.load(Ordering::SeqCst)
        }
        pub fn shares(&self) -> u64 {
            self.0.shares.load(Ordering::SeqCst)
        }
        pub fn shared(&self) -> SharedPulse {
            SharedPulse::new(self.clone())
        }
    }

    impl Stop for RecordingPulse {
        fn check(&self) -> Result<(), StopReason> {
            if self.0.cancelled.load(Ordering::SeqCst) {
                Err(StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }

    impl Report for RecordingPulse {
        fn advance(&self, completed: u64) {
            self.0.advanced.fetch_add(completed, Ordering::SeqCst);
        }
    }

    impl Pulse for RecordingPulse {
        fn split(
            &self,
            _execution: Execution,
            _parts: &[PhaseSpec<'_>],
        ) -> Result<Vec<Child<'_>>, PlanError> {
            self.0.splits.fetch_add(1, Ordering::SeqCst);
            Err(PlanError::Unsupported)
        }
        fn set_total(&self, total: Total) -> Result<(), PlanError> {
            *self.0.total.lock().unwrap() = Some(total);
            Ok(())
        }
        fn share(&self) -> Result<SharedPulse, PlanError> {
            self.0.shares.fetch_add(1, Ordering::SeqCst);
            Ok(self.shared())
        }
    }
}
use pulse_support::RecordingPulse;

/// A job-owned control is distinct from the optional driver control. This
/// adapter double refuses any replacement of `None` with a synthetic no-op.
struct RetainedControl<D> {
    decoder: D,
    pulse: how_far::SharedPulse,
}
impl<D: IncrementalDecode> IncrementalDecode for RetainedControl<D> {
    type Error = D::Error;
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Progress, StepError<Self::Error>> {
        assert!(
            pulse.is_none(),
            "driver must preserve absent per-call control"
        );
        enough::Stop::check(&self.pulse).map_err(StepError::Cancelled)?;
        self.decoder.step(input, sink, pulse)
    }
    fn requested(&self) -> &[InputRequest] {
        self.decoder.requested()
    }
    fn info(&self) -> Option<&ImageInfo> {
        self.decoder.info()
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.decoder.output_descriptor()
    }
    fn access_pattern(&self) -> AccessPattern {
        self.decoder.access_pattern()
    }
}
impl<D: zencodec::decode::ControlledIncrementalDecode> zencodec::decode::ControlledIncrementalDecode
    for RetainedControl<D>
{
    fn step_controlled(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<zencodec::decode::ControlledProgress, StepError<Self::Error>> {
        assert!(pulse.is_none());
        enough::Stop::check(&self.pulse).map_err(StepError::Cancelled)?;
        self.decoder.step_controlled(input, sink, pulse)
    }
    fn resume(
        &mut self,
        boundary: zencodec::decode::BoundaryId,
        action: zencodec::decode::BoundaryAction,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), StepError<Self::Error>> {
        assert!(pulse.is_none());
        enough::Stop::check(&self.pulse).map_err(StepError::Cancelled)?;
        self.decoder.resume(boundary, action, pulse)
    }
}

#[test]
fn no_per_call_control_during_resume_keeps_a_retained_stop_terminal() {
    use zencodec::decode::{BoundaryAction, InterpretationFacts, InterpretationState};
    use zencodec_io::{ByteStepper, ControlledEvent, ControlledStepper, SliceCursor, Yield};
    let pulse = RecordingPulse::default();
    let native =
        CoroutineDecoder::controlled(32, AccessPattern::RandomAccess, |mut ctx| async move {
            ctx.set_info(ImageInfo::new(2, 1, ImageFormat::Png));
            ctx.set_output_descriptor(RGBA);
            ctx.interpretation(InterpretationFacts::new(
                2,
                1,
                Some(RGBA),
                InterpretationState::Final,
            ))
            .await?;
            ctx.write_rows(
                BatchMeta::final_rows(0),
                &PixelSlice::new(&[9; 8], 2, 1, 8, RGBA).unwrap(),
            )
            .await?;
            Ok::<_, CoroutineError>(OutputInfo::full_decode(2, 1, RGBA))
        });
    let retained = RetainedControl {
        decoder: native,
        pulse: pulse.shared(),
    };
    let mut sink = CollectSink::new();
    let mut stage = ControlledStepper::new(retained, &mut sink);
    let mut cursor = SliceCursor::new(8);
    let boundary = loop {
        match cursor.next(&[], &mut stage, None).unwrap() {
            Yield::Output(ControlledEvent::Suspended(boundary)) => break boundary,
            Yield::Output(ControlledEvent::Decode(_)) => {}
            other => panic!("expected mandatory suspension: {other:?}"),
        }
    };
    assert!(stage.sink().descriptor().is_none());
    pulse.cancel();
    assert!(matches!(
        stage.resume(boundary.id(), BoundaryAction::AdoptInterpretation, None),
        Err(StepError::Cancelled(enough::StopReason::Cancelled))
    ));
    assert!(matches!(
        stage.step(Input::empty(), None),
        Err(StepError::Finished)
    ));
    assert!(stage.sink().descriptor().is_none());
    assert_eq!(stage.sink().rows_written(), 0);
}

#[test]
fn none_preserves_absence_and_retained_cancellation_in_every_sync_image_driver() {
    let file = sequential_file(6);
    for driver in 0..3 {
        for cancelled in [false, true] {
            let pulse = RecordingPulse::default();
            let mut decoder = RetainedControl {
                decoder: sequential_decoder(),
                pulse: pulse.shared(),
            };
            if cancelled {
                pulse.cancel();
            }
            let mut sink = CollectSink::new();
            let result = match driver {
                0 => drive_from_slice(&mut decoder, &file, &mut sink, None, 5)
                    .map(|(info, _)| info)
                    .map_err(|e| match e {
                        DriveError::Step(e) => e,
                        other => panic!("unexpected source/protocol failure: {other:?}"),
                    }),
                1 => drive_from_read_at(
                    &mut decoder,
                    &mut Opaque(SliceReadAt::new(&file)),
                    &mut sink,
                    None,
                    DriveOptions::new(0, 5),
                )
                .map(|(info, _)| info)
                .map_err(|e| match e {
                    DriveError::Step(e) => e,
                    other => panic!("unexpected source/protocol failure: {other:?}"),
                }),
                _ => pump_read_to_sink(Cursor::new(&file), &mut decoder, &mut sink, None, 5, 128)
                    .map_err(|e| match e {
                        PumpError::Step(e) => e,
                        other => panic!("unexpected source/protocol failure: {other:?}"),
                    }),
            };
            if cancelled {
                assert!(matches!(
                    result,
                    Err(StepError::Cancelled(enough::StopReason::Cancelled))
                ));
                assert!(sink.descriptor().is_none());
                assert_eq!(sink.rows_written(), 0);
                assert!(!sink.finished());
            } else {
                let info = result.unwrap();
                assert_eq!((info.width, info.height), (2, 6));
                assert_eq!(sink.bytes(), &file[8..]);
                assert!(sink.finished());
            }
        }
    }
}

struct ReportingStage;
impl zencodec_io::ByteStepper for ReportingStage {
    type Output = how_far::SharedPulse;
    type Error = enough::StopReason;
    fn step(
        &mut self,
        _: Input<'_>,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<zencodec_io::Stepped<Self::Output>, Self::Error> {
        let pulse = pulse.expect("cursor preserves the supplied full Pulse");
        pulse.check()?;
        assert!(matches!(
            pulse.split(
                how_far::Execution::Sequence,
                &[how_far::PhaseSpec::new(
                    "decode",
                    1,
                    how_far::Total::Unknown
                )],
            ),
            Err(how_far::PlanError::Unsupported)
        ));
        pulse.set_total(how_far::Total::Exact(9)).unwrap();
        pulse.advance(4);
        let shared = pulse.share().unwrap();
        Ok(zencodec_io::Stepped {
            consumed: 0,
            step: zencodec_io::Step::Finished(shared),
        })
    }
    fn requested(&self) -> &[InputRequest] {
        &[]
    }
}

#[test]
fn every_sync_cursor_preserves_some_reporting_and_typed_cancellation() {
    use zencodec_io::{DriveFailure, PumpFailure, ReadAtCursor, ReadCursor, SliceCursor};
    for driver in 0..3 {
        for cancelled in [false, true] {
            let pulse = RecordingPulse::default();
            if cancelled {
                pulse.cancel();
            }
            let mut stage = ReportingStage;
            let result = match driver {
                0 => SliceCursor::new(8)
                    .next(&[], &mut stage, Some(&pulse))
                    .map_err(|e| match e {
                        DriveFailure::Step(e) => e,
                        other => panic!("{other:?}"),
                    }),
                1 => ReadAtCursor::new(DriveOptions::default())
                    .next(&mut Opaque(SliceReadAt::new(&[])), &mut stage, Some(&pulse))
                    .map_err(|e| match e {
                        DriveFailure::Step(e) => e,
                        other => panic!("{other:?}"),
                    }),
                _ => ReadCursor::new(8, 128)
                    .next(&mut Cursor::new(&[]), &mut stage, Some(&pulse))
                    .map_err(|e| match e {
                        PumpFailure::Step(e) => e,
                        other => panic!("{other:?}"),
                    }),
            };
            if cancelled {
                assert!(matches!(result, Err(enough::StopReason::Cancelled)));
                assert_eq!(
                    (pulse.splits(), pulse.shares(), pulse.advanced()),
                    (0, 0, 0)
                );
                assert_eq!(pulse.total(), None);
            } else {
                let output = result.unwrap();
                assert!(output.is_finished());
                let shared = output.into_inner();
                shared.advance(5);
                assert_eq!(
                    (pulse.splits(), pulse.shares(), pulse.advanced()),
                    (1, 1, 9)
                );
                assert_eq!(pulse.total(), Some(how_far::Total::Exact(9)));
                pulse.cancel();
                assert_eq!(
                    enough::Stop::check(&shared),
                    Err(enough::StopReason::Cancelled)
                );
            }
        }
    }
}

#[cfg(feature = "tokio")]
mod tokio_drivers {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use zencodec::decode::BoxedError;
    use zencodec::source::DynReadAt;
    use zencodec_io::tokio_io::{
        AsyncDriveError, AsyncReadSource, AsyncSource, BytesSource, ChannelRowSink, Compute,
        Inline, StripEvent, decode_async, pull_decode_on_thread, transcode_async,
    };

    #[tokio::test]
    async fn no_driver_pulse_preserves_absence_and_retained_async_cancellation() {
        let file = sequential_file(6);
        for cancelled in [false, true] {
            let pulse = RecordingPulse::default();
            let decoder = RetainedControl {
                decoder: sequential_decoder(),
                pulse: pulse.shared(),
            };
            if cancelled {
                pulse.cancel();
            }
            let mut source = BytesSource(file.clone().into());
            let result = decode_async(
                decoder,
                CollectSink::new(),
                &mut source,
                Inline,
                DriveOptions::new(0, 5),
                None,
            )
            .await;
            if cancelled {
                assert!(matches!(
                    result,
                    Err(AsyncDriveError::Step(StepError::Cancelled(
                        enough::StopReason::Cancelled
                    )))
                ));
            } else {
                let (info, sink) = result.unwrap();
                assert_eq!((info.width, info.height), (2, 6));
                assert_eq!(sink.bytes(), &file[8..]);
                assert!(sink.finished());
            }
        }
    }

    #[tokio::test]
    async fn async_cursor_preserves_some_reporting_and_typed_cancellation() {
        use zencodec_io::{DriveFailure, tokio_io::AsyncCursor};
        for cancelled in [false, true] {
            let pulse = RecordingPulse::default();
            if cancelled {
                pulse.cancel();
            }
            let mut source = BytesSource(Vec::new().into());
            let mut cursor = AsyncCursor::new(DriveOptions::default());
            let result = cursor
                .next(&mut source, &mut ReportingStage, Some(&pulse))
                .await;
            if cancelled {
                assert!(matches!(
                    result,
                    Err(DriveFailure::Step(enough::StopReason::Cancelled))
                ));
                assert_eq!(
                    (pulse.splits(), pulse.shares(), pulse.advanced()),
                    (0, 0, 0)
                );
                assert_eq!(pulse.total(), None);
            } else {
                let output = result.unwrap();
                assert!(output.is_finished());
                let shared = output.into_inner();
                shared.advance(5);
                assert_eq!(
                    (pulse.splits(), pulse.shares(), pulse.advanced()),
                    (1, 1, 9)
                );
                assert_eq!(pulse.total(), Some(how_far::Total::Exact(9)));
                pulse.cancel();
                assert_eq!(
                    enough::Stop::check(&shared),
                    Err(enough::StopReason::Cancelled)
                );
            }
        }
    }

    /// An object store stand-in: every range is its own task, so a round
    /// with two ranges is two requests in flight.
    struct ConcurrentSource {
        file: Arc<[u8]>,
        requests: Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
    }

    impl ConcurrentSource {
        fn new(file: Vec<u8>) -> Self {
            Self {
                file: file.into(),
                requests: Arc::default(),
                max_in_flight: Arc::default(),
            }
        }
    }

    impl AsyncSource for ConcurrentSource {
        async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
            let start = (offset as usize).min(self.file.len());
            let n = (self.file.len() - start).min(out.len());
            out[..n].copy_from_slice(&self.file[start..start + n]);
            Ok(n)
        }
        async fn read_ranges(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
            let in_flight = Arc::new(AtomicUsize::new(0));
            let mut set = tokio::task::JoinSet::new();
            for (i, (offset, out)) in reqs.iter().enumerate() {
                let (file, in_flight, max) = (
                    self.file.clone(),
                    in_flight.clone(),
                    self.max_in_flight.clone(),
                );
                let (offset, len) = (*offset as usize, out.len());
                self.requests.fetch_add(1, Ordering::SeqCst);
                set.spawn(async move {
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    (i, file[offset..offset + len].to_vec())
                });
            }
            while let Some(r) = set.join_next().await {
                let (i, bytes) = r.map_err(|e| Box::new(e) as BoxedError)?;
                reqs[i].1.copy_from_slice(&bytes);
            }
            Ok(())
        }
        fn len_hint(&self) -> Option<u64> {
            Some(self.file.len() as u64)
        }
        async fn len(&mut self) -> Result<u64, BoxedError> {
            Ok(self.file.len() as u64)
        }
    }

    /// Runs each batch on a fresh named thread.
    fn named_threads() -> impl Compute {
        |job: zencodec_io::tokio_io::Job| {
            std::thread::Builder::new()
                .name("zencodec-decode".into())
                .spawn(job)
                .unwrap();
        }
    }

    /// Records the thread each sink call ran on.
    struct ThreadNameSink {
        inner: CollectSink,
        names: Vec<Option<String>>,
    }
    impl DecodeRowSink for ThreadNameSink {
        fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
            self.names
                .push(std::thread::current().name().map(String::from));
            self.inner.begin(w, h, d)
        }
        fn provide_next_buffer(
            &mut self,
            y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            self.names
                .push(std::thread::current().name().map(String::from));
            self.inner.provide_next_buffer(y, height, width, descriptor)
        }
        fn finish(&mut self) -> Result<(), SinkError> {
            self.inner.finish()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn decode_async_fetches_ranges_concurrently_and_decodes_off_runtime() {
        let mut source = ConcurrentSource::new(minitile());
        let sink = ThreadNameSink {
            inner: CollectSink::new(),
            names: Vec::new(),
        };
        let (info, sink) = decode_async(
            minitile_decoder(),
            sink,
            &mut source,
            named_threads(),
            DriveOptions::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!((info.width, info.height), (W, H));
        assert_eq!(sink.inner.bytes(), &expected_pixels()[..]);
        assert_eq!(
            source.requests.load(Ordering::SeqCst),
            4,
            "header, index, two tiles"
        );
        assert_eq!(
            source.max_in_flight.load(Ordering::SeqCst),
            2,
            "both tiles in flight together"
        );
        assert!(
            sink.names
                .iter()
                .all(|n| n.as_deref() == Some("zencodec-decode")),
            "decode ran on the compute threads: {:?}",
            sink.names
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn decode_async_inline_on_a_single_threaded_runtime() {
        let file = sequential_file(5);
        let mut source = AsyncReadSource::new(std::io::Cursor::new(file));
        let (info, sink) = decode_async(
            sequential_decoder(),
            CollectSink::new(),
            &mut source,
            Inline,
            DriveOptions::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(info.height, 5);
        assert_eq!(&sink.bytes()[32..40], &[4u8; 8]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn decode_async_emits_rows_while_the_upload_is_still_running() {
        // The writer sends the header and two rows, then waits until the
        // sink has seen two rows before sending the rest. A driver that
        // waited for a full read buffer, or for the end, deadlocks here.
        let file = sequential_file(6);
        let (client, server) = tokio::io::duplex(64);
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel::<()>();
        struct Notify {
            inner: CollectSink,
            at: u64,
            tx: Option<tokio::sync::oneshot::Sender<()>>,
        }
        impl DecodeRowSink for Notify {
            fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
                self.inner.begin(w, h, d)
            }
            fn provide_next_buffer(
                &mut self,
                y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                if self.inner.rows_written() + height as u64 >= self.at
                    && let Some(tx) = self.tx.take()
                {
                    let _ = tx.send(());
                }
                self.inner.provide_next_buffer(y, height, width, descriptor)
            }
            fn finish(&mut self) -> Result<(), SinkError> {
                self.inner.finish()
            }
        }
        let writer = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut client = client;
            client.write_all(&file[..8 + 16]).await.unwrap();
            seen_rx.await.unwrap();
            client.write_all(&file[8 + 16..]).await.unwrap();
        });
        let mut source = AsyncReadSource::new(server);
        let sink = Notify {
            inner: CollectSink::new(),
            at: 2,
            tx: Some(seen_tx),
        };
        let decode = decode_async(
            sequential_decoder(),
            sink,
            &mut source,
            Inline,
            DriveOptions::default(),
            None,
        );
        let (info, sink) = tokio::time::timeout(Duration::from_secs(5), decode)
            .await
            .expect("rows must flow before the upload ends")
            .unwrap();
        writer.await.unwrap();
        assert_eq!(info.height, 6);
        assert_eq!(sink.inner.rows_written(), 6);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn strips_stream_back_to_the_async_side_with_backpressure() {
        let mut source = BytesSource(sequential_file(8).into());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let sink = ChannelRowSink::new(tx, false);
        let compute = |job: zencodec_io::tokio_io::Job| {
            tokio::task::spawn_blocking(job);
        };
        let decode = decode_async(
            sequential_decoder(),
            sink,
            &mut source,
            compute,
            DriveOptions::default(),
            None,
        );
        let consumer = async move {
            let mut events = Vec::new();
            while let Some(ev) = rx.recv().await {
                tokio::time::sleep(Duration::from_millis(2)).await;
                let finished = matches!(ev, StripEvent::Finish);
                events.push(ev);
                if finished {
                    break;
                }
            }
            events
        };
        let ((info, _sink), events) = tokio::join!(async { decode.await.unwrap() }, consumer);
        assert_eq!(info.height, 8);
        assert!(matches!(
            events[0],
            StripEvent::Begin {
                width: 2,
                height: 8,
                ..
            }
        ));
        let strips: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                StripEvent::Strip { meta, pixels } => {
                    Some((meta.y, pixels.as_slice().row(0).to_vec()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(strips.len(), 8);
        for (y, row) in strips {
            assert_eq!(row, vec![y as u8; 8]);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn many_decodes_share_a_small_pool() {
        // Sixteen concurrent decodes over a two-worker runtime and the
        // blocking pool: none holds a thread while waiting for bytes.
        let mut set = tokio::task::JoinSet::new();
        for i in 0..16u32 {
            set.spawn(async move {
                let rows = 4 + i % 5;
                let mut source = BytesSource(sequential_file(rows).into());
                let compute = |job: zencodec_io::tokio_io::Job| {
                    tokio::task::spawn_blocking(job);
                };
                let (info, sink) = decode_async(
                    sequential_decoder(),
                    CollectSink::new(),
                    &mut source,
                    compute,
                    DriveOptions::new(0, 8),
                    None,
                )
                .await
                .unwrap();
                assert_eq!(info.height, rows);
                assert_eq!(sink.rows_written(), rows as u64);
            });
        }
        while let Some(r) = set.join_next().await {
            r.unwrap();
        }
    }

    /// Every async driver's future is `Send` for any decoder, sink and
    /// source that are, so it can go to `tokio::spawn`.
    #[allow(dead_code)]
    fn async_drivers_are_send() {
        fn send<T: Send>(_: T) {}
        send(async {
            let mut src = BytesSource(Vec::new().into());
            let _ = decode_async(
                sequential_decoder(),
                CollectSink::new(),
                &mut src,
                Inline,
                DriveOptions::default(),
                None,
            )
            .await;
        });
        send(async {
            let mut src = BytesSource(Vec::new().into());
            let mut out = Vec::new();
            let enc = RawEncoder {
                require_worker_finish: false,
                out: Vec::new(),
                drained: 0,
                peak_pending: Arc::default(),
            };
            let _ = transcode_async(
                sequential_decoder(),
                enc,
                &mut src,
                &mut out,
                Inline,
                DriveOptions::default(),
                None,
            )
            .await;
        });
        send(async {
            let mut src = BytesSource(Vec::new().into());
            let _ =
                pull_decode_on_thread(pull_tiles, CollectSink::new(), &mut src, Inline, None).await;
        });
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn source_errors_surface_and_the_pool_is_used() {
        struct Failing;
        impl AsyncSource for Failing {
            async fn read_at(&mut self, _: u64, _: &mut [u8]) -> Result<usize, BoxedError> {
                Err("503 from origin".into())
            }
            async fn read_ranges(&mut self, _: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
                Err("503 from origin".into())
            }
            async fn len(&mut self) -> Result<u64, BoxedError> {
                Err("503 from origin".into())
            }
        }
        let used = Arc::new(AtomicBool::new(false));
        let flag = used.clone();
        let compute = move |job: zencodec_io::tokio_io::Job| {
            flag.store(true, Ordering::SeqCst);
            std::thread::spawn(job);
        };
        let err = decode_async(
            minitile_decoder(),
            CollectSink::new(),
            &mut Failing,
            compute,
            DriveOptions::default(),
            None,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, AsyncDriveError::Source(ref e) if e.to_string().contains("503")),
            "{err:?}"
        );
        assert!(used.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_the_future_while_the_source_hangs_drops_decoder_and_sink() {
        struct DropFlag(Arc<AtomicBool>, CollectSink);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        impl DecodeRowSink for DropFlag {
            fn provide_next_buffer(
                &mut self,
                y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                self.1.provide_next_buffer(y, height, width, descriptor)
            }
        }
        struct Hung;
        impl AsyncSource for Hung {
            async fn read_at(&mut self, _: u64, _: &mut [u8]) -> Result<usize, BoxedError> {
                std::future::pending().await
            }
            async fn read_ranges(&mut self, _: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
                std::future::pending().await
            }
            async fn len(&mut self) -> Result<u64, BoxedError> {
                std::future::pending().await
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let sink = DropFlag(dropped.clone(), CollectSink::new());
        let mut source = Hung;
        let decode = decode_async(
            minitile_decoder(),
            sink,
            &mut source,
            Inline,
            DriveOptions::default(),
            None,
        );
        let timed_out = tokio::time::timeout(Duration::from_millis(30), decode).await;
        assert!(timed_out.is_err());
        assert!(
            dropped.load(Ordering::SeqCst),
            "between batches the future owns the sink; dropping it drops the sink"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pulse_cancels_inside_a_batch() {
        // The whole file is on hand, so one batch would decode every row;
        // the sink trips the pulse at the second row, and the decoder stops
        // at its next step.
        #[derive(Debug)]
        struct TripAt {
            inner: CollectSink,
            pulse: RecordingPulse,
        }
        impl DecodeRowSink for TripAt {
            fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
                self.inner.begin(w, h, d)
            }
            fn provide_next_buffer(
                &mut self,
                y: u32,
                height: u32,
                width: u32,
                descriptor: PixelDescriptor,
            ) -> Result<PixelSliceMut<'_>, SinkError> {
                if y == 1 {
                    self.pulse.cancel();
                }
                self.inner.provide_next_buffer(y, height, width, descriptor)
            }
        }
        let pulse = RecordingPulse::default();
        let mut source = BytesSource(sequential_file(64).into());
        let sink = TripAt {
            inner: CollectSink::new(),
            pulse: pulse.clone(),
        };
        let err = decode_async(
            sequential_decoder(),
            sink,
            &mut source,
            Inline,
            DriveOptions::default(),
            Some(pulse.shared()),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                err,
                AsyncDriveError::Step(StepError::Cancelled(enough::StopReason::Cancelled))
            ),
            "{err:?}"
        );
        assert!(pulse.advanced() < 64, "stopped early: {}", pulse.advanced());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn decode_async_reports_total_and_progress() {
        let pulse = RecordingPulse::default();
        let mut source = BytesSource(sequential_file(8).into());
        let (info, _) = decode_async(
            sequential_decoder(),
            CollectSink::new(),
            &mut source,
            Inline,
            DriveOptions::default(),
            Some(pulse.shared()),
        )
        .await
        .unwrap();
        assert_eq!(info.height, 8);
        assert_eq!(pulse.total(), Some(how_far::Total::Exact(8)));
        assert_eq!(pulse.advanced(), 8);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_drivers_report_protocol_violations() {
        let mut source = BytesSource(vec![0u8; 64].into());
        let err = decode_async(
            Bad {
                how: Misbehave::Spin,
                req: Vec::new(),
            },
            CollectSink::new(),
            &mut source,
            Inline,
            DriveOptions::default(),
            None,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                err,
                AsyncDriveError::Protocol(ProtocolViolation::NoProgress(_))
            ),
            "{err:?}"
        );
    }

    /// A streaming encoder stand-in: writes a 4-byte header at `begin`
    /// and each row's bytes as it is pushed, and makes them available as
    /// pending output.
    struct RawEncoder {
        require_worker_finish: bool,
        out: Vec<u8>,
        drained: usize,
        peak_pending: Arc<AtomicUsize>,
    }

    #[derive(Debug)]
    struct EncErr;
    impl std::fmt::Display for EncErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("enc")
        }
    }
    impl std::error::Error for EncErr {}

    impl zencodec::encode::Encoder for RawEncoder {
        type Error = EncErr;
        fn reject(_op: zencodec::UnsupportedOperation) -> EncErr {
            EncErr
        }
        fn encode(self, _pixels: PixelSlice<'_>) -> Result<zencodec::encode::EncodeOutput, EncErr> {
            Err(EncErr)
        }
        fn begin(
            &mut self,
            w: u32,
            h: u32,
            _d: PixelDescriptor,
        ) -> Result<zencodec::encode::OutputTiming, EncErr> {
            self.out.extend_from_slice(&[w as u8, h as u8, 0, 0]);
            Ok(zencodec::encode::OutputTiming::streaming(0))
        }
        fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), EncErr> {
            for r in 0..rows.rows() {
                self.out.extend_from_slice(rows.row(r));
            }
            let pending = self.out.len() - self.drained;
            self.peak_pending.fetch_max(pending, Ordering::SeqCst);
            Ok(())
        }
        fn pending_output(&self) -> &[u8] {
            &self.out[self.drained..]
        }
        fn consume_output(&mut self, n: usize) {
            self.drained += n;
        }
        fn finish(self) -> Result<zencodec::encode::EncodeOutput, EncErr> {
            if self.require_worker_finish {
                assert_eq!(std::thread::current().name(), Some("zencodec-decode"));
            }
            Ok(zencodec::encode::EncodeOutput::new(
                self.out[self.drained..].to_vec(),
                ImageFormat::Png,
            ))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transcode_finalizes_on_compute_and_preserves_bytes() {
        let file = sequential_file(6);
        let mut source = BytesSource(file.clone().into());
        let encoder = RawEncoder {
            require_worker_finish: true,
            out: Vec::new(),
            drained: 0,
            peak_pending: Arc::new(AtomicUsize::new(0)),
        };
        let mut out = Vec::new();
        let written = transcode_async(
            sequential_decoder(),
            encoder,
            &mut source,
            &mut out,
            named_threads(),
            DriveOptions::new(0, 16),
            None,
        )
        .await
        .unwrap();
        assert_eq!(&out[..4], &[2, 6, 0, 0]);
        assert_eq!(&out[4..], &file[8..]);
        assert_eq!(written as usize, out.len());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transcode_async_writes_encoded_bytes_as_rows_arrive() {
        let rows = 32u32;
        let file = sequential_file(rows);
        let mut source = BytesSource(file.into());
        let peak = Arc::new(AtomicUsize::new(0));
        let encoder = RawEncoder {
            require_worker_finish: false,
            out: Vec::new(),
            drained: 0,
            peak_pending: peak.clone(),
        };
        let mut out = Vec::new();
        // read_size 16: each batch decodes about two rows.
        let written = transcode_async(
            sequential_decoder(),
            encoder,
            &mut source,
            &mut out,
            Inline,
            DriveOptions::new(0, 16),
            None,
        )
        .await
        .unwrap();
        let mut expected = vec![2u8, rows as u8, 0, 0];
        for y in 0..rows {
            expected.extend_from_slice(&[y as u8; 8]);
        }
        assert_eq!(out, expected);
        assert_eq!(written, expected.len() as u64);
        assert!(
            peak.load(Ordering::SeqCst) < 64,
            "pending output drained per batch, peak {}",
            peak.load(Ordering::SeqCst)
        );
    }

    fn pull_tiles(
        src: &mut dyn DynReadAt,
        sink: &mut dyn DecodeRowSink,
    ) -> Result<OutputInfo, BoxedError> {
        let mut header = [0u8; 16];
        src.read_exact_at(0, &mut header)?;
        let index_at = u32::from_le_bytes(header[12..16].try_into().unwrap()) as u64;
        let mut index = [0u8; 16];
        src.read_exact_at(index_at, &mut index)?;
        let f = |i: usize| u32::from_le_bytes(index[i * 4..i * 4 + 4].try_into().unwrap());
        let mut t0 = vec![0u8; f(1) as usize];
        let mut t1 = vec![0u8; f(3) as usize];
        src.read_many_at(&mut [(f(0) as u64, &mut t0[..]), (f(2) as u64, &mut t1[..])])?;
        sink.begin(W, H, RGBA)?;
        for (y, tile) in [(0u32, &t0), (2, &t1)] {
            let mut dst = sink.provide_next_buffer(y, 2, W, RGBA)?;
            for r in 0..2 {
                dst.row_mut(r)
                    .copy_from_slice(&tile[r as usize * 16..r as usize * 16 + 16]);
            }
        }
        sink.finish()?;
        assert_eq!(src.len()?, 128);
        Ok(OutputInfo::full_decode(W, H, RGBA))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pull_native_decoder_blocks_on_the_async_source() {
        let mut source = ConcurrentSource::new(minitile());
        let (info, sink) = pull_decode_on_thread(
            pull_tiles,
            CollectSink::new(),
            &mut source,
            named_threads(),
            None,
        )
        .await
        .unwrap();
        assert_eq!((info.width, info.height), (W, H));
        assert_eq!(sink.bytes(), &expected_pixels()[..]);
        assert_eq!(source.requests.load(Ordering::SeqCst), 2, "the two tiles");
        assert_eq!(
            source.max_in_flight.load(Ordering::SeqCst),
            2,
            "read_many_at became concurrent reads"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pull_native_read_at_fills_from_a_trickling_source() {
        // An AsyncRead that returns a few bytes at a time: the bridge's
        // read_exact_at must still see full reads.
        let file = minitile();
        let (client, server) = tokio::io::duplex(8);
        let writer = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut client = client;
            for c in file.chunks(5) {
                // The decode stops reading after the index; then the
                // reader goes away and writes fail.
                if client.write_all(c).await.is_err() {
                    break;
                }
            }
        });
        let run = |src: &mut dyn DynReadAt,
                   _sink: &mut dyn DecodeRowSink|
         -> Result<OutputInfo, BoxedError> {
            let mut header = [0u8; 16];
            src.read_exact_at(0, &mut header)?;
            assert_eq!(&header[..4], b"MTIL");
            let mut index = [0u8; 16];
            src.read_exact_at(80, &mut index)?;
            assert_eq!(u32::from_le_bytes(index[..4].try_into().unwrap()), 32);
            Ok(OutputInfo::full_decode(W, H, RGBA))
        };
        let mut source = AsyncReadSource::new(server);
        pull_decode_on_thread(run, CollectSink::new(), &mut source, named_threads(), None)
            .await
            .unwrap();
        drop(source);
        writer.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_a_pull_native_decode_waiting_on_a_hung_source() {
        struct Hung;
        impl AsyncSource for Hung {
            async fn read_at(&mut self, _: u64, _: &mut [u8]) -> Result<usize, BoxedError> {
                std::future::pending().await
            }
            async fn len(&mut self) -> Result<u64, BoxedError> {
                std::future::pending().await
            }
        }
        let pulse = RecordingPulse::default();
        let p = pulse.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            p.cancel();
        });
        let start = Instant::now();
        let run = |src: &mut dyn DynReadAt,
                   _sink: &mut dyn DecodeRowSink|
         -> Result<OutputInfo, BoxedError> {
            let mut header = [0u8; 16];
            src.read_exact_at(0, &mut header)?;
            Ok(OutputInfo::full_decode(W, H, RGBA))
        };
        let err = pull_decode_on_thread(
            run,
            CollectSink::new(),
            &mut Hung,
            named_threads(),
            Some(pulse.shared()),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                err,
                AsyncDriveError::Cancelled(enough::StopReason::Cancelled)
            ),
            "{err:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_while_blocked_on_a_stuck_strip_consumer() {
        let mut source = BytesSource(sequential_file(8).into());
        let pulse = RecordingPulse::default();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let sink = ChannelRowSink::new(tx, false).with_pulse(pulse.shared());
        let p = pulse.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            p.cancel();
        });
        let start = Instant::now();
        let compute = |job: zencodec_io::tokio_io::Job| {
            tokio::task::spawn_blocking(job);
        };
        // The consumer never reads: the batch blocks in the sink.
        let err = decode_async(
            sequential_decoder(),
            sink,
            &mut source,
            compute,
            DriveOptions::default(),
            Some(pulse.shared()),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                err,
                AsyncDriveError::Step(StepError::Sink(_) | StepError::Cancelled(_))
            ),
            "{err:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "took {:?}",
            start.elapsed()
        );
        drop(rx);
    }
}

/// Asks for a range, then claims far more bytes than it was offered.
struct Greedy {
    step: u32,
    req: Vec<InputRequest>,
}

impl IncrementalDecode for Greedy {
    type Error = Never;
    fn step(
        &mut self,
        input: Input<'_>,
        _sink: &mut dyn DecodeRowSink,
        _stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<Progress, StepError<Never>> {
        self.step += 1;
        match self.step {
            1 => {
                self.req = vec![InputRequest::range(4, 8)];
                Ok(Progress::need(0))
            }
            2 => Ok(Progress::new(
                input.bytes.len() + 100,
                Event::Metadata(MetadataUpdate::HeaderReady),
            )),
            _ => Ok(Progress::new(
                0,
                Event::Done(OutputInfo::full_decode(1, 1, RGBA)),
            )),
        }
    }
    fn requested(&self) -> &[InputRequest] {
        &self.req
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        None
    }
    fn access_pattern(&self) -> AccessPattern {
        AccessPattern::RandomAccess
    }
}

#[test]
fn a_decoder_that_overclaims_a_fetched_range_does_not_panic_the_driver() {
    let data = [7u8; 64];
    let mut src = Opaque(SliceReadAt::new(&data));
    let mut sink = CollectSink::new();
    let mut dec = Greedy {
        step: 0,
        req: Vec::new(),
    };
    // The fetched-buffer offset is clamped like the window's: the drive
    // ends (here at Done) instead of indexing past the fetched bytes.
    let r = drive_from_read_at(&mut dec, &mut src, &mut sink, None, DriveOptions::default());
    assert!(r.is_ok(), "{r:?}");
}
