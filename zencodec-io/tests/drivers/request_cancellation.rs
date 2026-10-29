// Application-side helper: the closure cancels this operation's control only.
// Construct before returning the future so dropping an unpolled request cancels too.
struct RequestCancellation<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> RequestCancellation<F> {
    fn disarm(&mut self) {
        self.0 = None;
    }
}
impl<F: FnOnce()> Drop for RequestCancellation<F> {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            cancel();
        }
    }
}

fn cancel_on_request_drop<T, E>(
    operation: impl std::future::Future<Output = Result<T, E>>,
    cancel: impl FnOnce(),
) -> impl std::future::Future<Output = Result<T, E>> {
    let mut guard = RequestCancellation(Some(cancel));
    async move {
        let result = operation.await;
        // An error also stops any remaining operation-local work. Keep its
        // original typed error. Successful completion disarms cancellation.
        if result.is_ok() {
            guard.disarm();
        }
        result
    }
}

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
