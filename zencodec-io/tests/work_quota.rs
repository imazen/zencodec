#![cfg(feature = "tokio")]

use core::{
    future::{Future, poll_fn},
    pin::Pin,
    task::{Context, Poll},
};
use enough::Stop;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec::decode::{
    BatchMeta, DecodeRowSink, Event, IncrementalDecode, Input, MetadataUpdate, OutputInfo,
    Progress, Request, StepError,
};
use zencodec::encode::{EncodeOutput, Encoder, OutputTiming};
use zencodec::{ImageFormat, ImageInfo};
use zencodec_io::tokio_io::{BytesSource, Inline, Job, decode_async, transcode_async};
use zencodec_io::{CollectSink, DriveOptions, WorkQuota};
use zenpixels::{PixelDescriptor, PixelSlice};

const ROWS: u32 = 37;
const RGBA: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;
#[derive(Debug)]
struct Failed;
impl core::fmt::Display for Failed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("authored codec failure")
    }
}
impl core::error::Error for Failed {}

struct Burst {
    next: usize,
    fail: bool,
    steps: Arc<AtomicUsize>,
}
impl IncrementalDecode for Burst {
    type Error = Failed;
    fn step(
        &mut self,
        _: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        _: &dyn Stop,
    ) -> Result<Progress, StepError<Failed>> {
        self.steps.fetch_add(1, Ordering::SeqCst);
        let phase = self.next;
        self.next += 1;
        let event = match phase {
            0 => {
                sink.begin(1, ROWS, RGBA).map_err(StepError::Sink)?;
                Event::Metadata(MetadataUpdate::HeaderReady)
            }
            n if n <= ROWS as usize => {
                let row = n as u32 - 1;
                sink.provide_next_buffer(row, 1, 1, RGBA)
                    .map_err(StepError::Sink)?
                    .row_mut(0)
                    .copy_from_slice(&[row as u8, 1, 2, 255]);
                Event::Rows {
                    meta: BatchMeta::final_rows(row),
                    rows: 1,
                }
            }
            _ if self.fail => return Err(StepError::Codec(Failed)),
            _ => {
                sink.finish().map_err(StepError::Sink)?;
                Event::Done(OutputInfo::full_decode(1, ROWS, RGBA))
            }
        };
        Ok(Progress::new(0, event))
    }
    fn requested(&self) -> &[Request] {
        &[]
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        Some(RGBA)
    }
    fn access_pattern(&self) -> zencodec::source::AccessPattern {
        zencodec::source::AccessPattern::Sequential
    }
}

fn expected() -> Vec<u8> {
    (0..ROWS).flat_map(|y| [y as u8, 1, 2, 255]).collect()
}
fn opts(steps: usize) -> DriveOptions {
    DriveOptions::default().with_work_quota(WorkQuota::new(steps).unwrap())
}

#[tokio::test]
async fn every_compute_job_obeys_its_step_quota_and_pixels_are_identical() {
    for quota in [1, 2, 7, 64, usize::MAX] {
        let steps = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let compute = {
            let steps = steps.clone();
            let peak = peak.clone();
            move |job: Job| {
                let before = steps.load(Ordering::SeqCst);
                job();
                peak.fetch_max(steps.load(Ordering::SeqCst) - before, Ordering::SeqCst);
            }
        };
        let decoder = Burst {
            next: 0,
            fail: false,
            steps: steps.clone(),
        };
        let (_, sink) = decode_async(
            decoder,
            CollectSink::new(),
            &mut BytesSource(Arc::from([])),
            compute,
            opts(quota),
            None,
        )
        .await
        .unwrap();
        assert_eq!(sink.bytes(), expected());
        assert_eq!(steps.load(Ordering::SeqCst), ROWS as usize + 2);
        assert!(peak.load(Ordering::SeqCst) <= quota);
    }
}

#[tokio::test]
async fn inline_compute_returns_pending_after_one_quota() {
    let steps = Arc::new(AtomicUsize::new(0));
    let decoder = Burst {
        next: 0,
        fail: false,
        steps: steps.clone(),
    };
    let mut source = BytesSource(Arc::from([]));
    let future = decode_async(
        decoder,
        CollectSink::new(),
        &mut source,
        Inline,
        opts(7),
        None,
    );
    let mut future = core::pin::pin!(future);
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        assert_eq!(steps.load(Ordering::SeqCst), 7);
        Poll::Ready(())
    })
    .await;
    let (_, sink) = future.await.unwrap();
    assert_eq!(sink.bytes(), expected());
}

struct Raw {
    bytes: Vec<u8>,
    consumed: usize,
}
impl Encoder for Raw {
    type Error = Failed;
    fn reject(_: zencodec::UnsupportedOperation) -> Failed {
        Failed
    }
    fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Failed> {
        Err(Failed)
    }
    fn begin(&mut self, _: u32, _: u32, _: PixelDescriptor) -> Result<OutputTiming, Failed> {
        self.bytes.extend_from_slice(b"RAW!");
        Ok(OutputTiming::streaming(0))
    }
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), Failed> {
        for y in 0..rows.rows() {
            self.bytes.extend_from_slice(rows.row(y));
        }
        Ok(())
    }
    fn pending_output(&self) -> &[u8] {
        &self.bytes[self.consumed..]
    }
    fn consume_output(&mut self, n: usize) {
        self.consumed += n;
    }
    fn finish(self) -> Result<EncodeOutput, Failed> {
        Ok(EncodeOutput::new(
            self.bytes[self.consumed..].to_vec(),
            ImageFormat::Custom(&RAW_FORMAT),
        ))
    }
}
// The authored wire format is intentionally only a byte oracle.
static RAW_FORMAT: zencodec::ImageFormatDefinition = zencodec::ImageFormatDefinition::new(
    "quota-raw",
    None,
    "Quota byte oracle",
    "qraw",
    &["qraw"],
    "application/x-quota-raw",
    &["application/x-quota-raw"],
    true,
    false,
    true,
    false,
    4,
    |bytes| bytes.starts_with(b"RAW!"),
);

struct Writer {
    steps: Arc<AtomicUsize>,
    first_write: Option<usize>,
    bytes: Vec<u8>,
}
impl tokio::io::AsyncWrite for Writer {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let steps = self.steps.load(Ordering::SeqCst);
        self.first_write.get_or_insert(steps);
        self.bytes.extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn quota_yields_do_not_add_output_backpressure_or_publish_failed_batches() {
    for quota in [1, 7, 64, usize::MAX] {
        for fail in [false, true] {
            let steps = Arc::new(AtomicUsize::new(0));
            let mut writer = Writer {
                steps: steps.clone(),
                first_write: None,
                bytes: Vec::new(),
            };
            let decoder = Burst {
                next: 0,
                fail,
                steps,
            };
            let result = transcode_async(
                decoder,
                Raw {
                    bytes: Vec::new(),
                    consumed: 0,
                },
                &mut BytesSource(Arc::from([])),
                &mut writer,
                Inline,
                opts(quota),
                None,
            )
            .await;
            if fail {
                assert!(result.is_err());
                assert_eq!(writer.first_write, None);
                assert!(writer.bytes.is_empty());
            } else {
                assert_eq!(result.unwrap(), 4 + ROWS as u64 * 4);
                assert_eq!(writer.first_write, Some(ROWS as usize + 2));
                assert_eq!(writer.bytes, [b"RAW!".as_slice(), &expected()].concat());
            }
        }
    }
}
