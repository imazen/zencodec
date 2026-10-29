include!("../../../docs/examples/request_cancellation.rs");

#[tokio::test]
async fn request_guard_covers_success_error_unpolled_drop_and_unwind() {
    let unrelated = RecordingPulse::default();
    for success in [false, true] {
        let pulse = RecordingPulse::default();
        let cancel = pulse.clone();
        let result =
            cancel_on_request_drop(async { if success { Ok(7) } else { Err(42) } }, move || {
                cancel.cancel()
            })
            .await;
        assert_eq!(result, if success { Ok(7) } else { Err(42) });
        assert_eq!(enough::Stop::check(&pulse).is_ok(), success);
    }
    let pulse = RecordingPulse::default();
    let cancel = pulse.clone();
    let unpolled = cancel_on_request_drop(std::future::pending::<Result<(), ()>>(), move || {
        cancel.cancel()
    });
    drop(unpolled);
    assert!(enough::Stop::check(&pulse).is_err());

    let pulse = RecordingPulse::default();
    let cancel = pulse.clone();
    let task = tokio::spawn(cancel_on_request_drop(
        async {
            panic!("controlled request panic");
            #[allow(unreachable_code)]
            Ok::<(), ()>(())
        },
        move || cancel.cancel(),
    ));
    assert!(task.await.unwrap_err().is_panic());
    assert!(enough::Stop::check(&pulse).is_err());
    assert!(enough::Stop::check(&unrelated).is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_drop_reaches_driver_worker_without_cancelling_parent() {
    struct HeldStep {
        started: Option<tokio::sync::oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
        exited: Option<tokio::sync::oneshot::Sender<()>>,
        observed: Option<tokio::sync::oneshot::Sender<()>>,
    }
    impl Drop for HeldStep {
        fn drop(&mut self) {
            if let Some(done) = self.exited.take() {
                let _ = done.send(());
            }
        }
    }
    impl IncrementalDecode for HeldStep {
        type Error = Never;
        fn step(
            &mut self,
            _: Input<'_>,
            _: &mut dyn DecodeRowSink,
            pulse: Option<&dyn zencodec::Pulse>,
        ) -> Result<Progress, StepError<Never>> {
            let pulse = pulse.expect("request wrapper supplies its local control");
            pulse.advance(3);
            self.started.take().unwrap().send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
            let reason = pulse
                .check()
                .expect_err("task drop must reach the running step");
            assert_eq!(reason, enough::StopReason::Cancelled);
            self.observed.take().unwrap().send(()).unwrap();
            Err(StepError::Cancelled(reason))
        }
        fn requested(&self) -> &[InputRequest] {
            &[]
        }
        fn info(&self) -> Option<&ImageInfo> {
            None
        }
        fn output_descriptor(&self) -> Option<PixelDescriptor> {
            None
        }
    }
    for supplied_parent in [false, true] {
        let parent = RecordingPulse::default();
        let local = RecordingPulse::default();
        let cancel = local.clone();
        let inner: &dyn zencodec::Pulse = if supplied_parent {
            &parent
        } else {
            &zencodec::NoPulse
        };
        // Combine, never replace: caller cancellation/reporting survives, but
        // dropping this request only signals the additional local control.
        let combined = how_far::WithStop::new(inner, local);
        let shared = how_far::Pulse::share(&combined).unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (exited, gone) = tokio::sync::oneshot::channel();
        let (observed, checked) = tokio::sync::oneshot::channel();
        let decoder = HeldStep {
            started: Some(started),
            release: wait,
            exited: Some(exited),
            observed: Some(observed),
        };
        let request = tokio::spawn(cancel_on_request_drop(
            async move {
                let mut source = BytesSource(Vec::new().into());
                decode_async(
                    decoder,
                    CollectSink::new(),
                    &mut source,
                    |job| {
                        tokio::task::spawn_blocking(job);
                    },
                    DriveOptions::default(),
                    Some(shared),
                )
                .await
            },
            move || cancel.cancel(),
        ));
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert!(enough::Stop::check(&parent).is_ok());
        assert_eq!(parent.advanced(), if supplied_parent { 3 } else { 0 });
        release.send(()).unwrap();
        // A worker panic also drops its decoder; require the checkpoint receipt
        // separately so cleanup alone cannot masquerade as cancellation success.
        tokio::time::timeout(Duration::from_secs(5), checked)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), gone)
            .await
            .unwrap()
            .unwrap();
    }
}
