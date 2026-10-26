//! Collection registration shares the set's defaults without sharing a job.
use super::CodecSet;
use crate::{
    At, ImageFormat, StopToken,
    collection::{
        Collection, CollectionFactory, CollectionOpenError, Discovery, ErasedCollection,
        OpenRequest, SelectionError,
    },
    operation::{CheckedCollection, CollectionOperationError, PolicyResolver},
    source::DynReadAt,
};
use alloc::{boxed::Box, sync::Arc};
use enough::{Stop, StopReason};

impl CodecSet {
    /// Register an immutable native collection factory. First registration wins,
    /// including failures: opening never silently falls back to another factory.
    /// This does not register a still-image decoder or change `detect` behavior.
    pub fn with_collection(mut self, factory: impl CollectionFactory + 'static) -> Self {
        self.collections.push(Arc::new(factory));
        self
    }

    /// Open one checked collection borrowing the source, independently of this set.
    ///
    /// The factory must honor configured limits and decode policy or explicitly
    /// refuse. This is stronger than the legacy best-effort policy setters on
    /// still-image jobs. The publication resolver runs on each selected output.
    /// The set's stop/pulse and this call's stop all remain active throughout the
    /// opened operation; the first observed stop is latched. No implicit source
    /// staging or format probing occurs. Register concrete factories separately.
    pub fn open_collection<'a, R: PolicyResolver>(
        &self,
        format: ImageFormat,
        source: &'a mut dyn DynReadAt,
        request: OpenRequest,
        resolver: R,
        stop: StopToken,
    ) -> Result<CheckedCollection<Discovery<ErasedCollection<'a>>, R>, At<CollectionOperationError>>
    {
        let stop = StopToken::new(CollectionStop {
            pulse: self.pulse.clone(),
            inherited: self.stop.clone(),
            request: stop,
        });
        CheckedCollection::open(stop, resolver, |stop| {
            let factory = self
                .collections
                .iter()
                .find(|factory| factory.formats().contains(&format))
                .ok_or(CollectionOpenError::NotRegistered(format))?;
            let collection = factory.open(
                source,
                format,
                request.view,
                self.limits,
                self.decode_policy,
                stop,
            )?;
            if collection.view() != request.view {
                return Err(Box::new(SelectionError::UnsupportedComposition));
            }
            Ok(Discovery::new(collection, request.discovery_attempts))
        })
    }
}

struct CollectionStop {
    pulse: Option<how_far::SharedPulse>,
    inherited: Option<StopToken>,
    request: StopToken,
}
impl Stop for CollectionStop {
    fn may_stop(&self) -> bool {
        self.pulse.as_ref().is_some_and(Stop::may_stop)
            || self.inherited.as_ref().is_some_and(Stop::may_stop)
            || self.request.may_stop()
    }
    fn check(&self) -> Result<(), StopReason> {
        if let Some(pulse) = &self.pulse {
            pulse.check()?;
        }
        if let Some(stop) = &self.inherited {
            stop.check()?;
        }
        self.request.check()
    }
}
