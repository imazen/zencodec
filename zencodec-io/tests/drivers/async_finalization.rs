// Included inside tokio_drivers: these fixtures test effects, not codec fidelity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CancelAt {
    Never,
    Prefix,
    QueuedFinish,
    Finish,
    FinishError,
    Tail,
    Flush,
}

struct FinalizationProbe {
    pulse: RecordingPulse,
    at: CancelAt,
    calls: Arc<AtomicUsize>,
    remaining: u32,
    pending: Vec<u8>,
}
impl zencodec::encode::Encoder for FinalizationProbe {
    type Error = EncErr;
    fn reject(_: zencodec::UnsupportedOperation) -> EncErr {
        EncErr
    }
    fn encode(self, _: PixelSlice<'_>) -> Result<zencodec::encode::EncodeOutput, EncErr> {
        Err(EncErr)
    }
    fn begin(
        &mut self,
        _: u32,
        h: u32,
        _: PixelDescriptor,
    ) -> Result<zencodec::encode::OutputTiming, EncErr> {
        self.remaining = h;
        Ok(zencodec::encode::OutputTiming::streaming(
            h.saturating_sub(1),
        ))
    }
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), EncErr> {
        self.remaining -= rows.rows();
        if self.remaining == 0 {
            self.pending.extend_from_slice(b"prefix");
        }
        Ok(())
    }
    fn pending_output(&self) -> &[u8] {
        &self.pending
    }
    fn consume_output(&mut self, n: usize) {
        self.pending.drain(..n);
    }
    fn finish(self) -> Result<zencodec::encode::EncodeOutput, EncErr> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.at, CancelAt::Finish | CancelAt::FinishError) {
            self.pulse.cancel();
        }
        if self.at == CancelAt::FinishError {
            return Err(EncErr);
        }
        Ok(zencodec::encode::EncodeOutput::new(
            b"tail".to_vec(),
            ImageFormat::Png,
        ))
    }
}
struct PublicationWriter {
    pulse: RecordingPulse,
    at: CancelAt,
    wrote_prefix: Arc<AtomicBool>,
    bytes: Vec<u8>,
    flushes: usize,
}
impl tokio::io::AsyncWrite for PublicationWriter {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.bytes.extend_from_slice(bytes);
        if bytes == b"prefix" {
            self.wrote_prefix.store(true, Ordering::SeqCst);
            if self.at == CancelAt::Prefix {
                self.pulse.cancel();
            }
        }
        if bytes == b"tail" && self.at == CancelAt::Tail {
            self.pulse.cancel();
        }
        std::task::Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.flushes += 1;
        if self.at == CancelAt::Flush {
            self.pulse.cancel();
        }
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}
#[tokio::test]
async fn transcode_cancellation_prevents_later_finalization_effects() {
    for at in [
        CancelAt::Never,
        CancelAt::Prefix,
        CancelAt::QueuedFinish,
        CancelAt::Finish,
        CancelAt::FinishError,
        CancelAt::Tail,
        CancelAt::Flush,
    ] {
        let pulse = RecordingPulse::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let wrote_prefix = Arc::new(AtomicBool::new(false));
        let encoder = FinalizationProbe {
            pulse: pulse.clone(),
            at,
            calls: calls.clone(),
            remaining: 0,
            pending: Vec::new(),
        };
        let mut writer = PublicationWriter {
            pulse: pulse.clone(),
            at,
            wrote_prefix: wrote_prefix.clone(),
            bytes: Vec::new(),
            flushes: 0,
        };
        let worker_pulse = pulse.clone();
        let compute = move |job: zencodec_io::tokio_io::Job| {
            // Model a stop after dispatch but before the queued job executes.
            if at == CancelAt::QueuedFinish && wrote_prefix.load(Ordering::SeqCst) {
                worker_pulse.cancel();
            }
            job();
        };
        let mut source = BytesSource(sequential_file(6).into());
        let result = transcode_async(
            sequential_decoder(),
            encoder,
            &mut source,
            &mut writer,
            compute,
            DriveOptions::default(),
            Some(pulse.shared()),
        )
        .await;
        if at == CancelAt::Never {
            assert_eq!(result.unwrap(), 10);
        } else if at == CancelAt::FinishError {
            let error = result.unwrap_err();
            assert!(zencodec::find_cause::<EncErr>(&error).is_some());
            assert!(matches!(error, AsyncDriveError::Step(StepError::Sink(_))));
        } else {
            assert!(
                matches!(
                    result,
                    Err(AsyncDriveError::Cancelled(enough::StopReason::Cancelled))
                ),
                "{at:?}: {result:?}"
            );
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            usize::from(!matches!(at, CancelAt::Prefix | CancelAt::QueuedFinish)),
            "{at:?}"
        );
        assert_eq!(
            writer.bytes.as_slice(),
            if matches!(
                at,
                CancelAt::Prefix
                    | CancelAt::QueuedFinish
                    | CancelAt::Finish
                    | CancelAt::FinishError
            ) {
                &b"prefix"[..]
            } else {
                &b"prefixtail"[..]
            },
            "{at:?}"
        );
        assert_eq!(
            writer.flushes,
            usize::from(matches!(at, CancelAt::Never | CancelAt::Flush)),
            "{at:?}"
        );
    }
}
