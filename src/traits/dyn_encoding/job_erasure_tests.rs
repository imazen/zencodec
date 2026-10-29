//! Configured erasure must move the retained operation before native startup.
use super::*;
use crate::UnsupportedOperation;
use crate::encode::{EncodeDeclaration, EncodePlan, OutputTiming};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

struct Owned {
    drops: Arc<AtomicUsize>,
    plans: AtomicUsize,
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
// Deliberately not Clone: erasure must consume the exact configured operation.
struct Job {
    owned: Box<Owned>,
    starts: Arc<AtomicUsize>,
}
impl EncodeJob for Job {
    type Error = UnsupportedOperation;
    type Enc = Started;
    type AnimationFrameEnc = ();
    fn with_pulse(self, _: how_far::SharedPulse) -> Self {
        self
    }
    fn with_limits(self, _: ResourceLimits) -> Self {
        self
    }
    fn with_metadata(self, _: Metadata) -> Self {
        self
    }
    fn extensions(&self) -> Option<&dyn Any> {
        Some(self.owned.as_ref())
    }
    fn plan_encode(&self, _: &EncodeDeclaration<'_>) -> Result<EncodePlan, BoxedError> {
        self.owned.plans.fetch_add(1, Ordering::SeqCst);
        Ok(EncodePlan::rows(OutputTiming::AT_FINISH))
    }
    fn encoder(self) -> Result<Started, Self::Error> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(Started(self.owned))
    }
    fn animation_frame_encoder(self) -> Result<(), Self::Error> {
        Err(UnsupportedOperation::AnimationEncode)
    }
}
struct Started(Box<Owned>);
impl Encoder for Started {
    type Error = UnsupportedOperation;
    fn reject(error: Self::Error) -> Self::Error {
        error
    }
    fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
        assert_eq!(self.0.plans.load(Ordering::SeqCst), 1);
        Err(UnsupportedOperation::RowLevelEncode)
    }
}
fn job() -> (Job, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let drops = Arc::new(AtomicUsize::new(0));
    let starts = Arc::new(AtomicUsize::new(0));
    (
        Job {
            owned: Box::new(Owned {
                drops: drops.clone(),
                plans: AtomicUsize::new(0),
            }),
            starts: starts.clone(),
        },
        drops,
        starts,
    )
}
#[test]
fn configured_job_erasure_keeps_prepared_owner_without_startup_or_clone() {
    let (job, drops, starts) = job();
    let address = job.owned.as_ref() as *const Owned;
    job.plan_encode(&EncodeDeclaration::new(1, 1, PixelDescriptor::RGB8_SRGB))
        .unwrap();
    let erased: Box<dyn DynEncodeJob + Send> = job.into_dyn_job();
    let retained = erased
        .extensions()
        .unwrap()
        .downcast_ref::<Owned>()
        .unwrap();
    assert_eq!(retained as *const Owned, address);
    assert_eq!(retained.plans.load(Ordering::SeqCst), 1);
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let encoder = erased.into_encoder().unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(encoder);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
#[test]
fn dropping_erased_job_releases_owner_without_native_startup() {
    let (job, drops, starts) = job();
    drop(job.into_dyn_job());
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
