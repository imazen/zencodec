use core::num::NonZeroUsize;

/// Maximum codec steps in one uninterrupted async compute turn.
///
/// A step is indivisible. This bounds scheduling work units, not wall time,
/// memory, rows or pending encoded output. Yielding never asks an encoder to
/// drain: output backpressure needs a separate retention-qualified strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkQuota(NonZeroUsize);

impl WorkQuota {
    /// A nonzero step quota; zero cannot make progress and returns `None`.
    pub const fn new(steps: usize) -> Option<Self> {
        match NonZeroUsize::new(steps) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// Compatibility setting: finish the available batch without a quota yield.
    pub const fn unlimited() -> Self {
        Self(NonZeroUsize::MAX)
    }

    /// Maximum number of complete steps per turn.
    pub const fn max_steps(self) -> usize {
        self.0.get()
    }
}

impl Default for WorkQuota {
    fn default() -> Self {
        Self::unlimited()
    }
}

/// Give the calling executor one scheduling opportunity without depending on
/// a particular runtime or putting I/O backpressure on the worker.
#[cfg(feature = "tokio")]
pub(crate) async fn yield_once() {
    let mut yielded = false;
    core::future::poll_fn(|cx| {
        if yielded {
            core::task::Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            core::task::Poll::Pending
        }
    })
    .await;
}
