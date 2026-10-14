//! Every driver shape over the same two decoders: a random-access tiled
//! format and a sequential one, both written as coroutines.

#![cfg(feature = "std")]

use std::io::Cursor;

use zencodec::decode::{
    BatchMeta, DecodeRowSink, IncrementalDecode, MetadataUpdate, OutputInfo, SinkError, Step,
};
use zencodec::source::{AccessPattern, ReadAt, SourceError};
use zencodec::{ImageFormat, ImageInfo};
use zencodec_coroutine::{
    BoxedCoroutineDecoder, CoroutineContext, CoroutineDecoder, CoroutineError,
};
use zencodec_io::{
    BufferPolicy, BufferedReadAt, CollectSink, DriveError, DriveOptions, Feeder, PumpError,
    drive_from_read_at, pump_read_to_sink,
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

#[test]
fn stream_pump_decodes_as_bytes_arrive() {
    let file = sequential_file(6);
    for buffer in [1usize, 5, 512] {
        let mut dec = sequential_decoder();
        let mut sink = CollectSink::new();
        let info =
            pump_read_to_sink(Cursor::new(&file), &mut dec, &mut sink, None, buffer).unwrap();
        assert_eq!((info.width, info.height), (2, 6));
        for y in 0..6 {
            assert_eq!(
                &sink.bytes()[y * 8..y * 8 + 8],
                &[y as u8; 8],
                "buffer {buffer}"
            );
        }
        assert!(sink.finished());
    }
}

#[test]
fn stream_pump_emits_rows_before_the_stream_ends() {
    // A reader that counts how many bytes have been handed out; rows must
    // appear while bytes remain unread.
    struct Counting<'a> {
        data: &'a [u8],
        pos: usize,
    }
    impl std::io::Read for Counting<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = (self.data.len() - self.pos).min(buf.len()).min(8);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }
    struct FirstRowAt {
        inner: CollectSink,
        first_row_seen: Option<u64>,
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
                self.first_row_seen = Some(self.bytes_read.get() as u64);
            }
            self.inner.provide_next_buffer(y, height, width, descriptor)
        }
        fn finish(&mut self) -> Result<(), SinkError> {
            self.inner.finish()
        }
    }
    let file = sequential_file(32);
    let mut dec = sequential_decoder();
    let bytes_read = std::rc::Rc::new(std::cell::Cell::new(0usize));
    struct Tap<'a>(Counting<'a>, std::rc::Rc<std::cell::Cell<usize>>);
    impl std::io::Read for Tap<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.read(buf)?;
            self.1.set(self.1.get() + n);
            Ok(n)
        }
    }
    let mut sink = FirstRowAt {
        inner: CollectSink::new(),
        first_row_seen: None,
        bytes_read: bytes_read.clone(),
    };
    let reader = Tap(
        Counting {
            data: &file,
            pos: 0,
        },
        bytes_read.clone(),
    );
    pump_read_to_sink(reader, &mut dec, &mut sink, None, 8).unwrap();
    let first = sink.first_row_seen.unwrap();
    assert!(
        first <= 16,
        "first row should be written after the header and one row ({first} bytes read of {})",
        file.len()
    );
    assert_eq!(sink.inner.rows_written(), 32);
}

#[test]
fn stream_pump_rejects_random_access_with_a_pointer_to_the_fix() {
    let file = minitile();
    // A random-access decoder whose whole file fits its input store can
    // be served by a stream: the coroutine keeps what the pump reads
    // ahead and serves its own backward reads from the store.
    let mut dec = minitile_decoder();
    let mut sink = CollectSink::new();
    pump_read_to_sink(Cursor::new(&file), &mut dec, &mut sink, None, 16).unwrap();
    assert_eq!(sink.bytes(), &expected_pixels()[..]);

    // With a store smaller than the distance to the index, the pump
    // cannot buffer far enough: the error names the range and the fix.
    let mut dec: Dec = CoroutineDecoder::boxed_with_options(
        32,
        AccessPattern::RandomAccess,
        |mut ctx| async move {
            let mut header = [0u8; 16];
            ctx.read_exact_at(0, &mut header).await?;
            let index_at = u32::from_le_bytes(header[12..16].try_into().unwrap()) as u64;
            let mut index = [0u8; 16];
            ctx.read_exact_at(index_at, &mut index).await?;
            Ok(OutputInfo::full_decode(W, H, RGBA))
        },
    );
    let mut sink = CollectSink::new();
    let err = pump_read_to_sink(Cursor::new(&file), &mut dec, &mut sink, None, 16).unwrap_err();
    assert!(
        matches!(err, PumpError::NeedsRandomAccess(r) if r.offset == 80),
        "{err:?}"
    );

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
        matches!(err, SourceError::BeyondWindow { offset: 0, .. }),
        "{err:?}"
    );
    let mut past = [0u8; 2];
    let err = src.read_exact_at(255, &mut past).unwrap_err();
    assert!(matches!(err, SourceError::UnexpectedEof { .. }), "{err:?}");

    let mut spool = BufferedReadAt::new(Cursor::new(&data), BufferPolicy::Spool { max_bytes: 32 });
    let err = spool.read_exact_at(40, &mut buf).unwrap_err();
    assert!(
        matches!(err, SourceError::BeyondLimit { limit: 32, .. }),
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

/// A decoder that accepts at most `cap` bytes per feed and consumes them
/// one step at a time; used to pin partial-acceptance handling.
struct Trickle {
    cap: usize,
    held: Vec<u8>,
    consumed: usize,
    want: usize,
    need: [zencodec::decode::ByteRange; 1],
    done: bool,
}

#[derive(Debug)]
struct Never;
impl std::fmt::Display for Never {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "never")
    }
}
impl std::error::Error for Never {}

impl IncrementalDecode for Trickle {
    type Error = Never;
    fn feed(&mut self, _offset: u64, bytes: &[u8], _eof: bool) -> Result<usize, Never> {
        let take = bytes.len().min(self.cap.saturating_sub(self.held.len()));
        self.held.extend_from_slice(&bytes[..take]);
        Ok(take)
    }
    fn step(
        &mut self,
        sink: &mut dyn DecodeRowSink,
        _: Option<&dyn enough::Stop>,
    ) -> Result<Step<'_>, Never> {
        if self.done {
            return Ok(Step::Done(OutputInfo::full_decode(1, 1, RGBA)));
        }
        if !self.held.is_empty() {
            self.consumed += self.held.len();
            self.held.clear();
        }
        if self.consumed >= self.want {
            sink.begin(1, 1, RGBA).unwrap();
            sink.finish().unwrap();
            self.done = true;
            return Ok(Step::Done(OutputInfo::full_decode(1, 1, RGBA)));
        }
        self.need = [zencodec::decode::ByteRange::from_offset(
            self.consumed as u64,
        )];
        Ok(Step::Need(&self.need))
    }
    fn wrap_sink_error(_: SinkError) -> Never {
        Never
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
}

#[test]
fn feeder_re_offers_partial_acceptance() {
    let mut dec = Trickle {
        cap: 3,
        held: Vec::new(),
        consumed: 0,
        want: 10,
        need: [zencodec::decode::ByteRange::from_offset(0)],
        done: false,
    };
    let mut feeder = Feeder::new();
    assert!(!feeder.offer(&mut dec, 0, &[0u8; 10], true).unwrap());
    assert!(feeder.has_carry());
    let mut sink = CollectSink::new();
    let mut steps = 0;
    while !matches!(dec.step(&mut sink, None).unwrap(), Step::Done(_)) {
        steps += 1;
        feeder.flush(&mut dec).unwrap();
        assert!(steps < 20);
    }
    assert!(!feeder.has_carry());
    assert_eq!(dec.consumed, 10);

    // Through the drive loop, with a 2-byte source chunk and a 3-byte cap.
    let mut sink = CollectSink::new();
    let mut dec = Trickle {
        cap: 3,
        held: Vec::new(),
        consumed: 0,
        want: 10,
        need: [zencodec::decode::ByteRange::from_offset(0)],
        done: false,
    };
    let mut source = zencodec::source::SliceReadAt::new(&[7u8; 10]);
    let (_, stats) = drive_from_read_at(
        &mut dec,
        &mut source,
        &mut sink,
        None,
        DriveOptions::new(0, 2),
    )
    .unwrap();
    assert_eq!(dec.consumed, 10);
    assert_eq!(stats.bytes_fetched, 10, "nothing fetched twice");
}

#[test]
fn stalled_decoder_is_reported_not_spun() {
    let mut dec = Trickle {
        cap: 0,
        held: Vec::new(),
        consumed: 0,
        want: 1,
        need: [zencodec::decode::ByteRange::from_offset(0)],
        done: false,
    };
    let mut source = zencodec::source::SliceReadAt::new(&[1u8; 4]);
    let mut sink = CollectSink::new();
    let err = drive_from_read_at(
        &mut dec,
        &mut source,
        &mut sink,
        None,
        DriveOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, DriveError::Stalled), "{err:?}");
}

#[cfg(feature = "tokio")]
mod tokio_bridge {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use zencodec::decode::BoxedError;
    use zencodec::source::DynReadAt;
    use zencodec_io::thread::{
        ChannelRowSink, Chunk, SourceReply, SourceRequest, StripEvent, ThreadError, ThreadOptions,
        decode_on_thread, fetch_from_async_read, fetch_from_bytes, pull_decode_on_thread,
    };

    /// A fetcher standing in for an HTTP client: each range becomes its
    /// own task, so one `Step::Need` with two ranges is two concurrent
    /// requests. Records how many requests ran and on which thread.
    fn concurrent_fetcher(
        file: Arc<[u8]>,
        requests: Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
    ) -> impl FnMut(
        SourceRequest,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<SourceReply, BoxedError>> + Send>,
    > {
        move |req| {
            let file = file.clone();
            let requests = requests.clone();
            let max_in_flight = max_in_flight.clone();
            Box::pin(async move {
                let total = file.len() as u64;
                match req {
                    SourceRequest::Len => Ok(SourceReply::Len(total)),
                    SourceRequest::Ranges(ranges) => {
                        let in_flight = Arc::new(AtomicUsize::new(0));
                        let mut set = tokio::task::JoinSet::new();
                        for r in ranges {
                            let file = file.clone();
                            let in_flight = in_flight.clone();
                            let max_in_flight = max_in_flight.clone();
                            requests.fetch_add(1, Ordering::SeqCst);
                            set.spawn(async move {
                                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                                max_in_flight.fetch_max(now, Ordering::SeqCst);
                                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                                let len = if r.is_empty() {
                                    (total - r.offset).min(16)
                                } else {
                                    r.length
                                };
                                let end = (r.offset + len).min(total);
                                in_flight.fetch_sub(1, Ordering::SeqCst);
                                Chunk {
                                    offset: r.offset,
                                    bytes: file[r.offset as usize..end as usize].to_vec(),
                                    eof: end == total,
                                }
                            });
                        }
                        let mut chunks = Vec::new();
                        while let Some(c) = set.join_next().await {
                            chunks.push(c.unwrap());
                        }
                        Ok(SourceReply::Chunks(chunks))
                    }
                    _ => Err(Box::new(ThreadError::ProtocolMismatch) as BoxedError),
                }
            })
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
    async fn resumable_decoder_runs_off_runtime_with_concurrent_fetches() {
        let file: Arc<[u8]> = minitile().into();
        let requests = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        let sink = ThreadNameSink {
            inner: CollectSink::new(),
            names: Vec::new(),
        };
        let (info, sink) = decode_on_thread(
            minitile_decoder(),
            sink,
            None,
            ThreadOptions::default(),
            concurrent_fetcher(file, requests.clone(), max_in_flight.clone()),
        )
        .await
        .unwrap();
        assert_eq!((info.width, info.height), (W, H));
        assert_eq!(sink.inner.bytes(), &expected_pixels()[..]);
        assert_eq!(
            requests.load(Ordering::SeqCst),
            4,
            "header, index, two tiles"
        );
        assert_eq!(
            max_in_flight.load(Ordering::SeqCst),
            2,
            "both tiles fetched concurrently"
        );
        assert!(
            sink.names
                .iter()
                .all(|n| n.as_deref() == Some("zencodec-decode")),
            "decode ran on the dedicated thread, not a runtime worker: {:?}",
            sink.names
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn strips_stream_back_to_the_async_side_with_backpressure() {
        let file: Arc<[u8]> = sequential_file(8).into();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let sink = ChannelRowSink::new(tx, false);
        let decode = decode_on_thread(
            sequential_decoder(),
            sink,
            None,
            ThreadOptions::default(),
            fetch_from_bytes(file, 8),
        );
        let consumer = async move {
            let mut events = Vec::new();
            while let Some(ev) = rx.recv().await {
                // Slow consumer: the bounded channel throttles the thread.
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
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
        assert!(matches!(events.last(), Some(StripEvent::Finish)));
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
    async fn sequential_decoder_over_an_async_reader() {
        let file = sequential_file(5);
        let reader = tokio::io::BufReader::new(std::io::Cursor::new(file));
        let (info, sink) = decode_on_thread(
            sequential_decoder(),
            CollectSink::new(),
            None,
            ThreadOptions::default(),
            fetch_from_async_read(reader, 3),
        )
        .await
        .unwrap();
        assert_eq!(info.height, 5);
        assert_eq!(sink.rows_written(), 5);
        assert_eq!(&sink.bytes()[32..40], &[4u8; 8]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pull_native_decoder_blocks_on_the_async_fetcher() {
        let file: Arc<[u8]> = minitile().into();
        let requests = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        // A pull-native decode: straight-line reads against a DynReadAt.
        let run = |src: &mut dyn DynReadAt,
                   sink: &mut dyn DecodeRowSink|
         -> Result<OutputInfo, BoxedError> {
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
        };
        let (info, sink) = pull_decode_on_thread(
            run,
            CollectSink::new(),
            ThreadOptions::default(),
            concurrent_fetcher(file, requests.clone(), max_in_flight.clone()),
        )
        .await
        .unwrap();
        assert_eq!((info.width, info.height), (W, H));
        assert_eq!(sink.bytes(), &expected_pixels()[..]);
        assert_eq!(requests.load(Ordering::SeqCst), 4);
        assert_eq!(
            max_in_flight.load(Ordering::SeqCst),
            2,
            "read_many_at became concurrent fetches"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_the_future_stops_the_thread() {
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
        let dropped = Arc::new(AtomicBool::new(false));
        let sink = DropFlag(dropped.clone(), CollectSink::new());
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
        let gate = Arc::new(tokio::sync::Mutex::new(Some(gate_rx)));
        let decode = decode_on_thread(
            minitile_decoder(),
            sink,
            None,
            ThreadOptions::default(),
            move |_req| {
                let gate = gate.clone();
                async move {
                    // Never answers: the client "hung".
                    if let Some(rx) = gate.lock().await.take() {
                        let _ = rx.await;
                    }
                    Err::<SourceReply, BoxedError>("gate".into())
                }
            },
        );
        let timed_out = tokio::time::timeout(std::time::Duration::from_millis(50), decode).await;
        assert!(timed_out.is_err(), "the fetch never answers");
        // The future is dropped by the timeout; the thread sees the
        // closed channel and exits, dropping the sink.
        for _ in 0..200 {
            if dropped.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(
            dropped.load(Ordering::SeqCst),
            "decode thread did not exit after the driver went away"
        );
        drop(gate_tx);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fetch_errors_surface_and_pool_spawner_is_used() {
        let used_pool = Arc::new(AtomicBool::new(false));
        let flag = used_pool.clone();
        let opts = ThreadOptions::default().with_spawner(move |work| {
            flag.store(true, Ordering::SeqCst);
            std::thread::spawn(work);
        });
        let err = decode_on_thread(
            minitile_decoder(),
            CollectSink::new(),
            None,
            opts,
            |_req| async { Err::<SourceReply, BoxedError>("503 from origin".into()) },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, ThreadError::Fetch(ref e) if e.to_string().contains("503")),
            "{err:?}"
        );
        assert!(used_pool.load(Ordering::SeqCst));
    }
}
