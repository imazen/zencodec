//! Collection registration shares the set's defaults without sharing a job.
use super::CodecSet;
use crate::{
    At, ImageFormat, SharedPulse,
    collection::{
        Collection, CollectionFactory, CollectionOpenContext, CollectionOpenError, Discovery,
        ErasedCollection, OpenRequest, SelectionError,
    },
    operation::{CheckedCollection, CollectionOperationError, PolicyResolver},
    source::DynReadAt,
};
use alloc::{boxed::Box, sync::Arc};

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
    /// The set's pulse/pulse and this call's pulse all remain active throughout the
    /// opened operation; the first observed pulse is latched. No implicit source
    /// staging or format probing occurs. Register concrete factories separately.
    pub fn open_collection<'a, R: PolicyResolver>(
        &self,
        format: ImageFormat,
        source: &'a mut dyn DynReadAt,
        request: OpenRequest,
        resolver: R,
        pulse: SharedPulse,
    ) -> Result<CheckedCollection<Discovery<ErasedCollection<'a>>, R>, At<CollectionOperationError>>
    {
        use crate::Pulse as _;
        let inherited = self.pulse.clone().unwrap_or_default();
        let pulse = how_far::WithStop::new(&pulse, inherited)
            .share()
            .expect("owned operation pulses are shareable");
        CheckedCollection::open(pulse, resolver, |pulse| {
            let factory = self
                .collections
                .iter()
                .find(|factory| factory.formats().contains(&format))
                .ok_or(CollectionOpenError::NotRegistered(format))?;
            let mut context = CollectionOpenContext::new(format, request.view, pulse);
            context.limits = self.limits;
            context.policy = self.decode_policy;
            context.output_storage = request.output_storage;
            let collection = factory.open_with(source, context)?;
            if collection.view() != request.view {
                return Err(Box::new(SelectionError::UnsupportedComposition));
            }
            Ok(Discovery::new(collection, request.discovery_attempts))
        })
    }
}
