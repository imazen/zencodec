//! A scheduling counterexample, using native QOI plus deliberately retained raw
//! input. The retaining wrapper is a control, not a claim about QOI internals.
#![forbid(unsafe_code)]
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::task::{Context, Poll};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use zenbitmaps::{QoiDecoderConfig, QoiEncoderConfig};
use zencodec::UnsupportedOperation;
use zencodec::decode::DecoderConfig;
use zencodec::encode::{EncodeJob, EncodeOutput, Encoder, EncoderConfig, OutputTiming};
use zencodec_io::tokio_io::{BytesSource, Inline, decode_async, transcode_async};
use zencodec_io::{DriveOptions, EncoderSink};
use zenpixels::{PixelDescriptor, PixelSlice};

#[derive(Default)]
struct State {
    raw_bytes: AtomicUsize,
    finalized: AtomicBool,
    streaming: AtomicBool,
    writer_open: AtomicBool,
}

struct Retaining<E> {
    inner: E,
    retain: bool,
    raw: Vec<u8>,
    state: Arc<State>,
}
impl<E: Encoder> Encoder for Retaining<E> {
    type Error = E::Error;
    fn reject(op: UnsupportedOperation) -> Self::Error {
        E::reject(op)
    }
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
        self.inner.encode(pixels)
    }
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<OutputTiming, Self::Error> {
        let timing = self.inner.begin(width, height, descriptor)?;
        self.state.streaming.store(timing.streams(), SeqCst);
        Ok(timing)
    }
    fn push_rows(&mut self, pixels: PixelSlice<'_>) -> Result<(), Self::Error> {
        if self.retain {
            for y in 0..pixels.rows() {
                self.raw.extend_from_slice(pixels.row(y));
            }
            self.state.raw_bytes.store(self.raw.len(), SeqCst);
        }
        self.inner.push_rows(pixels)
    }
    fn preferred_strip_height(&self) -> u32 {
        self.inner.preferred_strip_height()
    }
    fn pending_output(&self) -> &[u8] {
        self.inner.pending_output()
    }
    fn consume_output(&mut self, n: usize) {
        self.inner.consume_output(n);
    }
    fn finish(self) -> Result<EncodeOutput, Self::Error> {
        let output = self.inner.finish()?;
        drop(self.raw);
        self.state.raw_bytes.store(0, SeqCst);
        self.state.finalized.store(true, SeqCst);
        Ok(output)
    }
}

struct GateWriter {
    state: Arc<State>,
    at_stall: Option<(usize, bool)>,
    bytes: Vec<u8>,
}
impl AsyncWrite for GateWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if !self.state.writer_open.load(SeqCst) {
            self.at_stall = Some((
                self.state.raw_bytes.load(SeqCst),
                self.state.finalized.load(SeqCst),
            ));
            return Poll::Pending;
        }
        let n = bytes.len().min(97);
        self.bytes.extend_from_slice(&bytes[..n]);
        Poll::Ready(Ok(n))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn fixture() -> Arc<[u8]> {
    let raw = [32, 64, 128, 255].repeat(512 * 512);
    QoiEncoderConfig::new()
        .encode(PixelSlice::new(&raw, 512, 512, 2048, PixelDescriptor::RGBA8_SRGB).unwrap())
        .unwrap()
        .data()
        .into()
    // The original raw fixture is dropped before the observed drive starts.
}

#[tokio::test]
async fn output_timing_does_not_establish_raw_storage_release() {
    let encoded = fixture();
    for (retain, finish_first) in [(true, false), (true, true), (false, false)] {
        let state = Arc::new(State::default());
        let encoder = Retaining {
            inner: QoiEncoderConfig::new().job().encoder().unwrap(),
            retain,
            raw: Vec::new(),
            state: state.clone(),
        };
        let decoder = QoiDecoderConfig::new()
            .job()
            .into_incremental_decoder(&[])
            .unwrap();
        let mut source = BytesSource(encoded.clone());
        let mut writer = GateWriter {
            state: state.clone(),
            at_stall: None,
            bytes: Vec::new(),
        };
        let mut work = Box::pin(async {
            if finish_first {
                // A finite admitted-job control: complete into owned encoded
                // storage, releasing raw state before waiting on the writer.
                // This is NOT a generic bounded-output spool implementation.
                let (_, sink) = decode_async(
                    decoder,
                    EncoderSink::new(encoder),
                    &mut source,
                    Inline,
                    DriveOptions::default(),
                    None,
                )
                .await
                .unwrap();
                let output = sink.finish().unwrap();
                writer.write_all(output.data()).await.unwrap();
            } else {
                transcode_async(
                    decoder,
                    encoder,
                    &mut source,
                    &mut writer,
                    Inline,
                    DriveOptions::default(),
                    None,
                )
                .await
                .unwrap();
            }
        });
        std::future::poll_fn(|cx| {
            assert!(work.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        state.writer_open.store(true, SeqCst);
        work.await;
        let (raw, finalized) = writer.at_stall.expect("writer must actually stall");
        assert!(state.streaming.load(SeqCst));
        assert_eq!(
            writer.bytes.as_slice(),
            encoded.as_ref(),
            "scheduling changed QOI bytes"
        );
        if finish_first {
            assert_eq!(raw, 0, "finalization must release the retained raw input");
            assert!(finalized);
        }
        if !retain {
            assert_eq!(raw, 0);
        }
        // Observe the existing driver without enshrining its retention as a
        // permanent expectation. Raw length is a logical observation, not RSS.
        println!(
            "retain={retain} finish_first={finish_first} raw_bytes_at_stall={raw} finalized_at_stall={finalized} encoded_bytes={} exact=true",
            encoded.len()
        );
    }
}
