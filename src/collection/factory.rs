//! Registered factories keep configuration separate from an opened source.
use super::{Cause, Composition, ErasedCollection};
use crate::{
    At, DecodePolicy, ImageFormat, ResourceLimits, StopToken, source::DynReadAt,
    storage::StorageBudget,
};

/// Resolved requirements for one native collection opening.
///
/// These are operation requirements, not capability queries or estimates. A
/// factory must enforce applicable fields or refuse before affected work.
/// Discovery-call accounting remains in the checked collection wrapper.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CollectionOpenContext {
    /// The matched registration, without another format-detection pass.
    pub format: ImageFormat,
    /// Fixed view for all identities issued by this opening.
    pub view: Composition,
    /// Inherited resource requirements; `None` keeps the native template.
    pub limits: Option<ResourceLimits>,
    /// Inherited decode requirements; `None` keeps the native template.
    pub policy: Option<DecodePolicy>,
    /// Shared latched cancellation, retained throughout the native operation.
    pub stop: StopToken,
    /// Required pool for retained output payloads, including row padding.
    ///
    /// Admission must precede native output allocation. Published owners and
    /// their clones retain the charge until the last owner drops. This does not
    /// bound native workspace, caller input, metadata or explicit caller copies.
    /// `None` keeps any native template's output-storage configuration.
    pub output_storage: Option<StorageBudget>,
}
impl CollectionOpenContext {
    /// Create an opening with no shared resource, policy or storage overrides.
    pub fn new(format: ImageFormat, view: Composition, stop: StopToken) -> Self {
        Self {
            format,
            view,
            limits: None,
            policy: None,
            stop,
            output_storage: None,
        }
    }
}

/// Immutable native configuration registered with [`crate::CodecSet`].
///
/// Factories are shareable; an opened collection and its borrowed source need
/// not be `Send`. Returned collections must own or clone any configuration they
/// retain, so they can outlive the registry. Native dependencies stay in adapters.
pub trait CollectionFactory: Send + Sync {
    /// Formats served by this factory, in addition to any still-image configs.
    fn formats(&self) -> &[ImageFormat];

    /// Open with all resolved requirements and one shared cancellation token.
    ///
    /// Required output storage must cover every returned color and auxiliary
    /// payload. Never copy into ordinary storage to evade a refusal. Parsing and
    /// workspace remain separate admission responsibilities. Unsupported storage
    /// receives a typed refusal before entering the legacy opening path.
    fn open_with<'a>(
        &self,
        source: &'a mut dyn DynReadAt,
        context: CollectionOpenContext,
    ) -> Result<ErasedCollection<'a>, Cause> {
        if context.output_storage.is_some() {
            return Err(At::wrap(CollectionOpenError::UnsupportedOutputStorage).into());
        }
        self.open(
            source,
            context.format,
            context.view,
            context.limits,
            context.policy,
            context.stop,
        )
    }

    /// Compatibility entry for adapters migrating to [`Self::open_with`].
    ///
    /// `format` is the matched registration, not a fresh format-detection result.
    /// Supplied limits and decode policy are requirements: honor them throughout
    /// the collection or refuse before the affected work. Do not ignore applicable
    /// fields; `ResourceLimits::max_output_bytes` is encode-only. Do not
    /// buffer the complete source implicitly, or substitute another view.
    /// `None` keeps the native template's configuration. Native parsing/workspace
    /// admission remains this adapter's responsibility; publication policy cannot
    /// retroactively bound reconstruction. Preserve concrete causes and traces.
    fn open<'a>(
        &self,
        _source: &'a mut dyn DynReadAt,
        format: ImageFormat,
        _view: Composition,
        _limits: Option<ResourceLimits>,
        _policy: Option<DecodePolicy>,
        _stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause> {
        Err(At::wrap(CollectionOpenError::UnsupportedFactoryContract(format)).into())
    }
}

/// A fixed view and allowance for one opened collection.
///
/// The allowance counts calls into discovery, not native parse work or bytes.
/// Resource and decode-policy requirements come from the registry; application
/// publication decisions use the resolver passed to `open_collection`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct OpenRequest {
    /// Required composited or raw view, fixed before issuing any identity.
    pub view: Composition,
    /// Calls allowed into `images`, `children`, `next` and `describe` in total.
    pub discovery_attempts: u64,
    /// Optional required pool shared by all returned output payloads.
    ///
    /// Unsupported adapters refuse at opening. This is not a whole-operation
    /// memory bound, and it does not permit ignoring other resource limits.
    pub output_storage: Option<StorageBudget>,
}
impl OpenRequest {
    /// Require a view and an explicit operation-wide discovery allowance.
    pub const fn new(view: Composition, discovery_attempts: u64) -> Self {
        Self {
            view,
            discovery_attempts,
            output_storage: None,
        }
    }

    /// Require admission of every returned color and auxiliary payload.
    /// The pool can be shared across operations; retained images keep its charge.
    pub fn with_output_storage(mut self, storage: StorageBudget) -> Self {
        self.output_storage = Some(storage);
        self
    }
}

/// An opening requirement that the selected registration cannot honor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CollectionOpenError {
    /// No factory serves this format. Still-image decoders are registered separately.
    NotRegistered(ImageFormat),
    /// This adapter needs an already-resident slice; it did not stage the source.
    ResidentInputRequired,
    /// The adapter cannot enforce the supplied resource-limit configuration.
    UnsupportedLimits {
        /// Required fields the adapter cannot enforce. Qualified fields and
        /// fields that do not apply to decoding have their no-op/default values.
        /// This bounded structured data contains no source or metadata payload.
        unsupported: ResourceLimits,
    },
    /// The adapter cannot enforce the supplied decode-policy configuration.
    UnsupportedDecodePolicy {
        /// Explicit requirements the adapter cannot enforce. Handled fields
        /// are `None`; this is not permission to retry with weaker requirements.
        unsupported: DecodePolicy,
    },
    /// The adapter cannot admit returned output payloads to the required pool.
    UnsupportedOutputStorage,
    /// A migration-only factory implements the context opening path, not legacy.
    UnsupportedFactoryContract(ImageFormat),
}
impl core::fmt::Display for CollectionOpenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotRegistered(_) => "no collection factory registered for format",
            Self::ResidentInputRequired => "this collection requires already-resident input",
            Self::UnsupportedLimits { .. } => {
                "collection factory cannot enforce configured resource limits"
            }
            Self::UnsupportedDecodePolicy { .. } => {
                "collection factory cannot enforce configured decode policy"
            }
            Self::UnsupportedOutputStorage => {
                "collection factory cannot enforce required output storage"
            }
            Self::UnsupportedFactoryContract(_) => {
                "collection factory requires the context opening contract"
            }
        })
    }
}
impl core::error::Error for CollectionOpenError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodecSet, Unstoppable, operation::PolicySet, source::ReadAtShim};
    use alloc::sync::Arc;
    use core::{
        convert::Infallible,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct Untouched;
    impl crate::source::ReadAt for Untouched {
        type Error = Infallible;
        fn len(&mut self) -> Result<u64, Infallible> {
            panic!("source was queried")
        }
        fn read_at(&mut self, _: u64, _: &mut [u8]) -> Result<usize, Infallible> {
            panic!("source was read")
        }
        fn read_exact_at(&mut self, _: u64, _: &mut [u8]) -> Result<(), Infallible> {
            panic!("source was read exactly")
        }
    }
    struct Legacy(Arc<AtomicUsize>);
    impl CollectionFactory for Legacy {
        fn formats(&self) -> &[ImageFormat] {
            &[ImageFormat::Jxl]
        }
        fn open<'a>(
            &self,
            _: &'a mut dyn DynReadAt,
            _: ImageFormat,
            _: Composition,
            _: Option<ResourceLimits>,
            _: Option<DecodePolicy>,
            _: StopToken,
        ) -> Result<ErasedCollection<'a>, Cause> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(At::wrap(CollectionOpenError::ResidentInputRequired).into())
        }
    }
    #[test]
    fn collection_output_storage_refuses_before_legacy_work_or_fallback() {
        let calls = Arc::new(AtomicUsize::new(0));
        let codecs = CodecSet::new()
            .with_collection(Legacy(calls.clone()))
            .with_collection(Legacy(calls.clone()));
        let storage = StorageBudget::new(7, 1);
        let request =
            OpenRequest::new(Composition::Uncomposited, 8).with_output_storage(storage.clone());
        let mut source = ReadAtShim(Untouched);
        let error = codecs
            .open_collection(
                ImageFormat::Jxl,
                &mut source,
                request.clone(),
                PolicySet::new(),
                StopToken::new(Unstoppable),
            )
            .err()
            .expect("unsupported storage was accepted");
        assert_eq!(
            crate::find_cause::<CollectionOpenError>(&error),
            Some(&CollectionOpenError::UnsupportedOutputStorage)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            (storage.reserved_bytes(), storage.reserved_objects()),
            (0, 0)
        );
        // Cloning a request shares the pool rather than multiplying its allowance.
        let held = request.output_storage.as_ref().unwrap().bytes(7).unwrap();
        assert_eq!(storage.reserved_bytes(), 7);
        assert!(storage.bytes(1).is_err());
        drop(held);
        assert_eq!(storage.reserved_bytes(), 0);
    }
}
