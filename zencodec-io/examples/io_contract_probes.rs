//! Observational probes for issues 160–162. Values describe behavior; they
//! deliberately do not turn a known defect into a regression expectation.
//! Run with `just io-contract-probes`.
#![forbid(unsafe_code)]

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::task::{Context, Poll};

use zencodec::decode::{
    BatchMeta, DecodeRowSink, Event, IncrementalDecode, Input, OutputInfo, Progress, Request,
    StepError,
};
use zencodec::encode::{EncodeOutput, Encoder, OutputBuffer, OutputTiming};
use zencodec::source::{AccessPattern, ReadAt};
use zencodec::{ImageFormat, ImageInfo, UnsupportedOperation};
use zencodec_io::tokio_io::{BytesSource, Inline, transcode_async};
use zencodec_io::{BufferPolicy, BufferedReadAt, DriveOptions, FileReadAt, encode_to_writer};
use zenpixels::{PixelDescriptor, PixelSlice};

const DESC: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;
const WIDTH: u32 = 256;
const HEIGHT: u32 = 1024;

#[derive(Debug)]
struct Error(UnsupportedOperation);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// A valid one-shot-only implementation: all row methods use trait defaults.
struct OneShot;
impl Encoder for OneShot {
    type Error = Error;
    fn reject(op: UnsupportedOperation) -> Error {
        Error(op)
    }
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Error> {
        let mut out = Vec::new();
        for y in 0..pixels.rows() {
            out.extend_from_slice(pixels.row(y));
        }
        Ok(EncodeOutput::new(out, ImageFormat::Pnm))
    }
}

#[derive(Default)]
struct Counts {
    produced: AtomicUsize,
    peak_pending: AtomicUsize,
    first_write_rows: AtomicUsize,
    write_polls: AtomicUsize,
    jobs: AtomicUsize,
}

struct Raw {
    out: OutputBuffer,
    counts: Arc<Counts>,
}
impl Encoder for Raw {
    type Error = Error;
    fn reject(op: UnsupportedOperation) -> Error {
        Error(op)
    }
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Error> {
        OneShot.encode(pixels)
    }
    fn begin(&mut self, _: u32, _: u32, _: PixelDescriptor) -> Result<OutputTiming, Error> {
        Ok(OutputTiming::streaming(0))
    }
    fn push_rows(&mut self, pixels: PixelSlice<'_>) -> Result<(), Error> {
        for y in 0..pixels.rows() {
            self.out.push(pixels.row(y));
        }
        self.counts
            .peak_pending
            .fetch_max(self.out.pending().len(), SeqCst);
        Ok(())
    }
    fn pending_output(&self) -> &[u8] {
        self.out.pending()
    }
    fn consume_output(&mut self, n: usize) {
        self.out.consume(n);
    }
    fn finish(self) -> Result<EncodeOutput, Error> {
        Ok(EncodeOutput::new(
            self.out.into_remaining(),
            ImageFormat::Pnm,
        ))
    }
}

/// One encoded byte expands to a whole image. A step emits `band` rows;
/// HEIGHT emulates a whole-file adapter's indivisible push decode.
struct Burst {
    y: u32,
    band: u32,
    started: bool,
    counts: Arc<Counts>,
    request: [Request; 1],
}
impl IncrementalDecode for Burst {
    type Error = Error;
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn enough::Stop,
    ) -> Result<Progress, StepError<Error>> {
        stop.check().map_err(StepError::Cancelled)?;
        if !self.started {
            if input.bytes.is_empty() {
                return Ok(Progress::need(0));
            }
            sink.begin(WIDTH, HEIGHT, DESC).map_err(StepError::Sink)?;
            self.started = true;
            return Ok(Progress::new(
                1,
                Event::Metadata(zencodec::decode::MetadataUpdate::HeaderReady),
            ));
        }
        if self.y == HEIGHT {
            sink.finish().map_err(StepError::Sink)?;
            return Ok(Progress::new(
                0,
                Event::Done(OutputInfo::full_decode(WIDTH, HEIGHT, DESC)),
            ));
        }
        let n = self.band.min(HEIGHT - self.y);
        let mut dst = sink
            .provide_next_buffer(self.y, n, WIDTH, DESC)
            .map_err(StepError::Sink)?;
        for r in 0..n {
            dst.row_mut(r).fill((self.y + r) as u8);
        }
        let meta = BatchMeta::final_rows(self.y);
        self.y += n;
        self.counts.produced.store(self.y as usize, SeqCst);
        Ok(Progress::new(0, Event::Rows { meta, rows: n }))
    }
    fn requested(&self) -> &[Request] {
        &self.request
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        Some(DESC)
    }
    fn access_pattern(&self) -> AccessPattern {
        AccessPattern::Sequential
    }
}

/// Alternate Pending and a short write. Every pending interval verifies
/// that the producer stayed stopped. This is deterministic, without sleeps.
struct SlowWriter {
    counts: Arc<Counts>,
    data: Vec<u8>,
    stalled_at: Option<usize>,
}
impl tokio::io::AsyncWrite for SlowWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let rows = self.counts.produced.load(SeqCst);
        if self.counts.write_polls.fetch_add(1, SeqCst) == 0 {
            self.counts.first_write_rows.store(rows, SeqCst);
        }
        match self.stalled_at.take() {
            None => {
                self.stalled_at = Some(rows);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Some(before) => {
                assert_eq!(rows, before, "producer advanced while writer was pending");
                let n = bytes.len().min(8191);
                self.data.extend_from_slice(&bytes[..n]);
                Poll::Ready(Ok(n))
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

async fn burst(band: u32, read_size: usize, pooled: bool) {
    let counts = Arc::new(Counts::default());
    let decoder = Burst {
        y: 0,
        band,
        started: false,
        counts: counts.clone(),
        request: [Request::at_least(0, 1)],
    };
    let encoder = Raw {
        out: OutputBuffer::new(),
        counts: counts.clone(),
    };
    let mut source = BytesSource(vec![1u8].into());
    let mut writer = SlowWriter {
        counts: counts.clone(),
        data: Vec::new(),
        stalled_at: None,
    };
    let opts = DriveOptions::new(0, read_size);
    let written = if pooled {
        let c = counts.clone();
        transcode_async(
            decoder,
            encoder,
            &mut source,
            &mut writer,
            move |job| {
                c.jobs.fetch_add(1, SeqCst);
                tokio::task::spawn_blocking(job);
            },
            opts,
            None,
        )
        .await
        .unwrap()
    } else {
        transcode_async(
            decoder,
            encoder,
            &mut source,
            &mut writer,
            Inline,
            opts,
            None,
        )
        .await
        .unwrap()
    };
    assert_eq!(written as usize, WIDTH as usize * HEIGHT as usize * 4);
    for y in 0..HEIGHT as usize {
        assert!(
            writer.data[y * WIDTH as usize * 4..(y + 1) * WIDTH as usize * 4]
                .iter()
                .all(|&b| b == y as u8)
        );
    }
    println!(
        "burst band={band} read_size={read_size} pooled={pooled} first_write_rows={} peak_pending_bytes={} jobs={} parity=true stall=true",
        counts.first_write_rows.load(SeqCst),
        counts.peak_pending.load(SeqCst),
        counts.jobs.load(SeqCst)
    );
}

fn budgets() {
    let mut source = BufferedReadAt::new(io::repeat(7), BufferPolicy::Spool { max_bytes: 8 });
    let result = source.read_exact_at(0, &mut [0u8; 1024]);
    println!(
        "spool cap=8 requested=1024 consumed={} result={result:?}",
        source.watermark()
    );
    println!(
        "sequential_policy cap=8 selected={:?}",
        BufferPolicy::for_pattern(AccessPattern::Sequential, 8)
    );
    println!(
        "backtrack_policy needed=1024 cap=8 selected={:?}",
        BufferPolicy::for_pattern(AccessPattern::BoundedBacktrack { window_bytes: 1024 }, 8)
    );
}

fn file_error_provenance() {
    let scratch = std::env::var_os("TMPDIR").expect("set TMPDIR for the file error probe");
    let missing = std::path::PathBuf::from(scratch).join(format!(
        "zencodec-missing-error-probe-{}",
        std::process::id()
    ));
    assert!(!missing.exists(), "probe requires an absent path");
    let native = std::fs::File::open(&missing).expect_err("absent file");
    assert_eq!(native.kind(), io::ErrorKind::NotFound);
    let adapted = FileReadAt::open(&missing).err().expect("absent file");
    println!(
        "file_error native_kind={:?} native_cause_preserved={}",
        native.kind(),
        zencodec::find_cause::<io::Error>(&adapted).is_some()
    );
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    file_error_provenance();
    let data = [7u8; 4];
    let px = PixelSlice::new(&data, 1, 1, 4, DESC).unwrap();
    assert_eq!(OneShot.encode(px.clone()).unwrap().data(), data);
    let mut out = Vec::new();
    println!(
        "one_shot_writer result={:?} written={}",
        encode_to_writer(OneShot, px, &mut out),
        out.len()
    );
    budgets();
    for band in [1, 16, HEIGHT] {
        for read_size in [1, 64 * 1024] {
            for pooled in [false, true] {
                burst(band, read_size, pooled).await;
            }
        }
    }
}

/// Shared only with the benchmark target, not a library API.
#[allow(dead_code)]
pub(crate) fn bench_transcode(suite: &mut zenbench::Suite) {
    for band in [1, 16, HEIGHT] {
        suite.group(format!("one-byte-expansion/{band}-row-step"), |g| {
            g.config()
                .max_time(std::time::Duration::from_secs(2))
                .max_rounds(30);
            for pooled in [false, true] {
                let label = if pooled { "spawn_blocking" } else { "inline" };
                g.bench(label, move |b| {
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .build()
                        .unwrap();
                    b.iter(move || {
                        rt.block_on(async {
                            let counts = Arc::new(Counts::default());
                            let decoder = Burst {
                                y: 0,
                                band,
                                started: false,
                                counts: counts.clone(),
                                request: [Request::at_least(0, 1)],
                            };
                            let encoder = Raw {
                                out: OutputBuffer::new(),
                                counts,
                            };
                            let mut source = BytesSource(vec![1u8].into());
                            let mut out = Vec::new();
                            if pooled {
                                transcode_async(
                                    decoder,
                                    encoder,
                                    &mut source,
                                    &mut out,
                                    |job| {
                                        tokio::task::spawn_blocking(job);
                                    },
                                    DriveOptions::default(),
                                    None,
                                )
                                .await
                                .unwrap();
                            } else {
                                transcode_async(
                                    decoder,
                                    encoder,
                                    &mut source,
                                    &mut out,
                                    Inline,
                                    DriveOptions::default(),
                                    None,
                                )
                                .await
                                .unwrap();
                            }
                            std::hint::black_box(out)
                        })
                    })
                });
            }
        });
    }
}
