// Deliberately ignores the borrowed Pulse to test the driver's own boundary.
struct BatchCancellationProbe {
    calls: Arc<AtomicUsize>,
    pulse: RecordingPulse,
    cancel_at: Option<usize>,
    fail: bool,
}
impl IncrementalDecode for BatchCancellationProbe {
    type Error = std::io::Error;
    fn step(
        &mut self,
        _: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<Progress, StepError<Self::Error>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.cancel_at == Some(call) {
            self.pulse.cancel();
        }
        if self.fail {
            return Err(StepError::Codec(std::io::Error::from(
                std::io::ErrorKind::InvalidData,
            )));
        }
        let event = match call {
            0 => {
                sink.begin(1, 1, RGBA).map_err(StepError::Sink)?;
                Event::Metadata(MetadataUpdate::HeaderReady)
            }
            1 => {
                sink.provide_next_buffer(0, 1, 1, RGBA)
                    .map_err(StepError::Sink)?
                    .row_mut(0)
                    .copy_from_slice(&[1, 2, 3, 255]);
                Event::Rows {
                    meta: BatchMeta::final_rows(0),
                    rows: 1,
                }
            }
            2 => {
                sink.finish().map_err(StepError::Sink)?;
                Event::Done(OutputInfo::full_decode(1, 1, RGBA))
            }
            _ => panic!("driver stepped completed codec"),
        };
        Ok(Progress::new(0, event))
    }
    fn requested(&self) -> &[InputRequest] {
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

#[tokio::test]
async fn async_batch_driver_owns_cancellation_between_native_steps() {
    for (queued_cancel, cancel_at, fail, expected_calls) in [
        (true, None, false, 0),
        (false, Some(0), false, 1),
        (false, Some(2), false, 3),
        (false, Some(0), true, 1),
        (false, None, false, 3),
    ] {
        let pulse = RecordingPulse::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let decoder = BatchCancellationProbe {
            calls: calls.clone(),
            pulse: pulse.clone(),
            cancel_at,
            fail,
        };
        let worker_pulse = pulse.clone();
        let compute = move |job: zencodec_io::tokio_io::Job| {
            if queued_cancel {
                worker_pulse.cancel();
            }
            job();
        };
        let result = decode_async(
            decoder,
            CollectSink::new(),
            &mut BytesSource(Arc::from([])),
            compute,
            DriveOptions::default(),
            Some(pulse.shared()),
        )
        .await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected_calls,
            "queued={queued_cancel}, cancel_at={cancel_at:?}, fail={fail}"
        );
        if fail {
            assert!(
                matches!(result, Err(AsyncDriveError::Step(StepError::Codec(ref e))) if e.kind() == std::io::ErrorKind::InvalidData)
            );
        } else if queued_cancel || cancel_at.is_some() {
            assert!(matches!(
                result,
                Err(AsyncDriveError::Cancelled(enough::StopReason::Cancelled))
                    | Err(AsyncDriveError::Step(StepError::Cancelled(
                        enough::StopReason::Cancelled
                    )))
            ));
        } else {
            let (_, sink) = result.unwrap();
            assert_eq!(sink.bytes(), &[1, 2, 3, 255]);
        }
    }
}
