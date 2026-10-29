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

pub(crate) fn cancel_on_request_drop<T, E>(
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
