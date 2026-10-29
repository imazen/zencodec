//! Checked consumer for the shared collection mechanism.
use super::{OperationPolicy, OperationReport, Outcome, PolicySet, resolve_image};
use crate::{
    At, SharedPulse,
    collection::{Cause, Collection, Composition, ImageDescription, ImageId, ImageRequest},
    image::{ImageOutput, OutputError},
};
use alloc::boxed::Box;
use core::fmt;
use enough::{Stop, StopReason};

#[derive(Debug)]
struct Finished;
impl fmt::Display for Finished {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("selected-image operation has finished")
    }
}
impl core::error::Error for Finished {}

/// The boundary at which a checked collection request failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CollectionStage {
    /// Native adapter creation.
    Open,
    /// Starting or advancing discovery.
    Discovery,
    /// Native selected-image reconstruction.
    Decode,
    /// Publication decision for the retained output.
    Policy,
    /// Cooperative cancellation or timeout.
    Stopped,
    /// A call after terminal cancellation or unwind.
    Finished,
    /// Establishing and validating selected geometry before reconstruction.
    Describe,
    /// Adapter processing of the held output before publication policy.
    Transform,
}

/// Safe default formatting; concrete causes and prior decisions remain available.
#[non_exhaustive]
pub struct CollectionOperationError {
    /// Boundary that failed; the concrete cause remains in the error chain.
    pub stage: CollectionStage,
    /// Bounded prior decisions, excluding arbitrary input or callback payloads.
    pub report: Box<OperationReport>,
    cause: Cause,
    observed_stop: Option<StopReason>,
}
impl CollectionOperationError {
    /// First pulse reason observed through this operation's shared token.
    ///
    /// The original native, source or application failure remains the cause,
    /// even when it coincides with a pulse. `None` means the token has not
    /// reported a pulse; it does not classify arbitrary backend errors.
    pub const fn stop_reason(&self) -> Option<StopReason> {
        self.observed_stop
    }
    fn with_stop(mut self, reason: Option<StopReason>) -> Self {
        self.observed_stop = reason;
        self
    }
}
impl fmt::Debug for CollectionOperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CollectionOperationError")
            .field("stage", &self.stage)
            .field("report", &self.report)
            .field("observed_stop", &self.observed_stop)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for CollectionOperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "selected-image request failed during {:?}", self.stage)
    }
}
impl core::error::Error for CollectionOperationError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
fn failure(
    stage: CollectionStage,
    report: &OperationReport,
    cause: impl Into<Cause>,
) -> CollectionOperationError {
    CollectionOperationError {
        stage,
        report: Box::new(report.clone()),
        cause: cause.into(),
        observed_stop: None,
    }
}

/// The first native or wrapper observation stops this opened operation, even
/// when a custom pulse callback would subsequently return Ok. Policy failure and
/// request refusal do not change this token. No application callback runs while
/// the first reason is being stored.
struct LatchedStop {
    source: SharedPulse,
    reason: spin::Once<StopReason>,
}
impl Stop for LatchedStop {
    fn may_stop(&self) -> bool {
        self.source.may_stop()
    }
    fn check(&self) -> Result<(), StopReason> {
        if let Some(reason) = self.reason.get() {
            return Err(*reason);
        }
        if let Err(reason) = self.source.check() {
            // Preserve the first recorded reason, including future variants.
            return Err(*self.reason.call_once(|| reason));
        }
        // Another caller may have stopped while this callback returned Ok.
        self.reason.get().map_or(Ok(()), |reason| Err(*reason))
    }
}

/// One checked opened collection, with reusable policy and bounded diagnostics.
///
/// Every selected output passes through the resolver before it is returned.
/// Cancellation and unwind terminate the operation; ordinary request/policy
/// refusal is recoverable. Native terminal errors remain the adapter's duty.
/// This is post-reconstruction publication, not native allocation admission.
pub struct CheckedCollection<C, R> {
    collection: Option<C>,
    view: Composition,
    resolver: R,
    preset: PolicySet,
    pulse: SharedPulse,
    report: OperationReport,
    observed_stop: Option<StopReason>,
}
impl<C: Collection, R: OperationPolicy> CheckedCollection<C, R> {
    /// Bind the same token to the native adapter and publication checks. The
    /// factory is adapter setup; ordinary selection methods need no callback args.
    pub fn open(
        pulse: SharedPulse,
        resolver: R,
        open: impl FnOnce(SharedPulse) -> Result<C, Cause>,
    ) -> Result<Self, At<CollectionOperationError>> {
        use crate::Pulse as _;
        let latch = LatchedStop {
            source: pulse.clone(),
            reason: spin::Once::new(),
        };
        let pulse = how_far::WithStop::replacing(&pulse, latch)
            .share()
            .expect("owned SharedPulse and owned latch are shareable");
        let report = OperationReport::default();
        let run = || {
            check(&pulse, &report)?;
            let collection =
                open(pulse.clone()).map_err(|e| failure(CollectionStage::Open, &report, e));
            // Observe the final seam on failed opening too. Preserve the native
            // cause while making a coincident pulse available without downcasts.
            let stopped = check(&pulse, &report);
            let collection = match (collection, stopped) {
                (Ok(collection), Ok(())) => collection,
                (Err(error), Ok(())) => return Err(error),
                (Ok(_), Err(stopped)) => return Err(stopped),
                (Err(error), Err(stopped)) => return Err(error.with_stop(stopped.stop_reason())),
            };
            Ok(Self {
                view: collection.view(),
                collection: Some(collection),
                resolver,
                preset: PolicySet::new(),
                pulse,
                report,
                observed_stop: None,
            })
        };
        run().map_err(At::wrap)
    }
    /// Configure the preset used when the callback returns `Decision::UsePreset`.
    /// During migration, these requirements and the policy argument
    /// both apply. A bound policy supplies preferences in place of these.
    #[deprecated(note = "Pass PolicySet or BoundPolicy as the operation policy argument")]
    pub fn with_policy_set(mut self, preset: PolicySet) -> Self {
        self.preset = preset;
        self
    }
    /// The operation's bounded decision history across selected outputs.
    pub fn report(&self) -> &OperationReport {
        &self.report
    }
    /// Whether cancellation or unwind has dropped the native collection.
    pub fn is_finished(&self) -> bool {
        self.collection.is_none()
    }

    fn run<T>(
        &mut self,
        operation: impl FnOnce(
            &mut C,
            &mut R,
            PolicySet,
            &SharedPulse,
            &mut OperationReport,
        ) -> Result<T, CollectionOperationError>,
    ) -> Result<T, At<CollectionOperationError>> {
        // An unwind drops native state and cannot leave a half-mutated operation
        // callable after catch_unwind. Ordinary errors remain request-local;
        // the native adapter retains responsibility for its own terminal state.
        let Some(mut collection) = self.collection.take() else {
            return Err(At::wrap(
                failure(CollectionStage::Finished, &self.report, Finished)
                    .with_stop(self.observed_stop),
            ));
        };
        if let Err(stopped) = check(&self.pulse, &self.report) {
            self.observed_stop = stopped.stop_reason();
            return Err(At::wrap(stopped));
        }
        let result = operation(
            &mut collection,
            &mut self.resolver,
            self.preset,
            &self.pulse,
            &mut self.report,
        );
        match check(&self.pulse, &self.report) {
            Ok(()) => {
                self.collection = Some(collection);
                result.map_err(At::wrap)
            }
            Err(stopped) => {
                // Preserve the first native/policy cause; drop a successful
                // unpublished owner if cancellation arrived at the final seam.
                self.observed_stop = stopped.stop_reason();
                match result {
                    Ok(_) => Err(At::wrap(stopped)),
                    Err(error) => Err(At::wrap(error.with_stop(self.observed_stop))),
                }
            }
        }
    }
    /// Begin top-level discovery, checking the shared operation pulse token.
    pub fn images(&mut self) -> Result<C::Cursor, At<CollectionOperationError>> {
        self.run(|c, _, _, _, report| {
            c.images()
                .map_err(|e| failure(CollectionStage::Discovery, report, e))
        })
    }
    /// Begin child discovery. Unsupported containment returns an error.
    pub fn children(&mut self, image: &ImageId) -> Result<C::Cursor, At<CollectionOperationError>> {
        self.run(|c, _, _, _, report| {
            c.children(image)
                .map_err(|e| failure(CollectionStage::Discovery, report, e))
        })
    }
    /// Advance discovery. Successful exhaustion is distinct from refusal.
    pub fn next(
        &mut self,
        cursor: &mut C::Cursor,
    ) -> Result<Option<ImageId>, At<CollectionOperationError>> {
        self.run(|c, _, _, _, report| {
            c.next(cursor)
                .map_err(|e| failure(CollectionStage::Discovery, report, e))
        })
    }
    /// Inspect source geometry without pixel reconstruction. This does not run
    /// publication policy or establish complete metadata/resource admission.
    /// Source failures retain their cause; cancellation finishes the operation.
    pub fn describe(
        &mut self,
        image: &ImageId,
    ) -> Result<ImageDescription, At<CollectionOperationError>> {
        self.run(|c, _, _, _, report| {
            let description = c
                .describe(image)
                .map_err(|e| failure(CollectionStage::Describe, report, e))?;
            description
                .validate()
                .map_err(|e| failure(CollectionStage::Describe, report, e))?;
            Ok(description)
        })
    }
    /// Reconstruct the required extent, resolve policy on that exact owner,
    /// and return it only after the final cancellation check.
    pub fn decode(
        &mut self,
        image: &ImageId,
        request: &ImageRequest,
    ) -> Result<Outcome<ImageOutput>, At<CollectionOperationError>> {
        self.decode_with(image, request, |image, _| Ok::<_, Cause>(image))
    }
    /// Adapter hook: reconstruct, transform the held owner, then resolve policy
    /// once on the completed output. Ordinary native decode uses the same path
    /// with an identity transform. A render adapter can bind its own prepared
    /// color/geometry request here without publishing an intermediate raster.
    ///
    /// The transform receives the operation's latched pulse token. It must honor
    /// its configured requirements, preserve or update interpretation together
    /// with the samples, and return a fully owned output or a concrete error.
    /// This wrapper does not prove arbitrary transform math, admit its workspace
    /// or turn native decode into a bounded region operation. Transform errors
    /// are request-local; cancellation and unwind terminate the collection.
    /// No fallback, retry or policy callback runs on a failed transform.
    pub fn decode_with<E: Into<Cause>>(
        &mut self,
        image: &ImageId,
        request: &ImageRequest,
        transform: impl FnOnce(ImageOutput, &dyn crate::Pulse) -> Result<ImageOutput, E>,
    ) -> Result<Outcome<ImageOutput>, At<CollectionOperationError>> {
        let view = self.view;
        self.run(|c, resolver, preset, pulse, report| {
            let check_view = |actual| {
                if actual == view {
                    Ok(())
                } else {
                    Err(failure(
                        CollectionStage::Decode,
                        report,
                        At::wrap(OutputError::Composition {
                            expected: view,
                            actual,
                        }),
                    ))
                }
            };
            check_view(c.view())?;
            let image = c
                .decode(image, request)
                .map_err(|e| failure(CollectionStage::Decode, report, e))?;
            check_view(c.view())?;
            let image = image
                .in_collection_view(view)
                .map_err(|e| failure(CollectionStage::Decode, report, e))?;
            check(pulse, report)?;
            let image = transform(image, pulse)
                .map_err(|e| failure(CollectionStage::Transform, report, e))?;
            check(pulse, report)?;
            match resolve_image(image, resolver, preset, Some(pulse)) {
                Ok(output) => {
                    for &record in output.report().decisions() {
                        report.record(record);
                    }
                    Ok(output)
                }
                Err(cause) => {
                    report.record(cause.error().record());
                    Err(failure(CollectionStage::Policy, report, cause))
                }
            }
        })
    }
    /// Discover and decode the first image through the same checked path.
    /// Returns `None` only for successful empty top-level discovery.
    pub fn first_image(
        &mut self,
        request: &ImageRequest,
    ) -> Result<Option<Outcome<ImageOutput>>, At<CollectionOperationError>> {
        let mut cursor = self.images()?;
        self.next(&mut cursor)?
            .map(|id| self.decode(&id, request))
            .transpose()
    }
}

fn check(pulse: &SharedPulse, report: &OperationReport) -> Result<(), CollectionOperationError> {
    pulse.check().map_err(|reason| {
        failure(
            CollectionStage::Stopped,
            report,
            crate::CodecError::new(None, reason.into()),
        )
        .with_stop(Some(reason))
    })
}

#[cfg(test)]
#[path = "collection_view_tests.rs"]
mod view_tests;

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    #[test]
    fn collection_stop_race_preserves_one_original_reason_for_every_observer() {
        struct Race {
            calls: Arc<AtomicUsize>,
            entered: Barrier,
        }
        impl Stop for Race {
            fn check(&self) -> Result<(), StopReason> {
                let call = self.calls.fetch_add(1, Ordering::Relaxed);
                self.entered.wait();
                Err(if call == 0 {
                    StopReason::TimedOut
                } else {
                    StopReason::Cancelled
                })
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let pulse = Arc::new(LatchedStop {
            source: SharedPulse::new(how_far::StopOnly::new(Race {
                calls: calls.clone(),
                entered: Barrier::new(2),
            })),
            reason: spin::Once::new(),
        });
        let a = pulse.clone();
        let b = pulse.clone();
        let a = std::thread::spawn(move || a.check());
        let b = std::thread::spawn(move || b.check());
        let reason = a.join().unwrap().unwrap_err();
        assert_eq!(b.join().unwrap(), Err(reason));
        for _ in 0..4 {
            assert_eq!(pulse.check(), Err(reason));
        }
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }
}
