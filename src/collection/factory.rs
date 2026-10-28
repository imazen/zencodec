//! Registered factories keep configuration separate from an opened source.
use super::{Cause, Composition, ErasedCollection};
use crate::{
    DecodePolicy, ImageFormat, ResourceLimits, SharedPulse, source::DynReadAt,
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
    pub pulse: SharedPulse,
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
    pub fn new(format: ImageFormat, view: Composition, pulse: SharedPulse) -> Self {
        Self {
            format,
            view,
            limits: None,
            policy: None,
            pulse,
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
    /// Honor applicable limits and decode policy or refuse before affected work;
    /// `ResourceLimits::max_output_bytes` is encode-only. Required output storage
    /// covers every returned color and auxiliary payload, including padding.
    /// Admit before native writes; never evade refusal with ordinary allocation.
    /// Unsupported storage returns `CollectionOpenError::UnsupportedOutputStorage`.
    ///
    /// Do not implicitly buffer the source or substitute another view. Native
    /// parsing and workspace remain separate admission responsibilities;
    /// publication policy cannot retroactively bound reconstruction. Preserve
    /// concrete causes and location traces.
    fn open_with<'a>(
        &self,
        source: &'a mut dyn DynReadAt,
        context: CollectionOpenContext,
    ) -> Result<ErasedCollection<'a>, Cause>;
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
        })
    }
}
impl core::error::Error for CollectionOpenError {}
