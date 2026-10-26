//! Registered factories keep configuration separate from an opened source.
use super::{Cause, Composition, ErasedCollection};
use crate::{DecodePolicy, ImageFormat, ResourceLimits, StopToken, source::DynReadAt};

/// Immutable native configuration registered with [`crate::CodecSet`].
///
/// Factories are shareable; an opened collection and its borrowed source need
/// not be `Send`. Returned collections must own or clone any configuration they
/// retain, so they can outlive the registry. Native dependencies stay in adapters.
pub trait CollectionFactory: Send + Sync {
    /// Formats served by this factory, in addition to any still-image configs.
    fn formats(&self) -> &[ImageFormat];

    /// Open the requested format and view with the operation's shared stop token.
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
        source: &'a mut dyn DynReadAt,
        format: ImageFormat,
        view: Composition,
        limits: Option<ResourceLimits>,
        policy: Option<DecodePolicy>,
        stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause>;
}

/// A fixed view and allowance for one opened collection.
///
/// The allowance counts calls into discovery, not native parse work or bytes.
/// Resource and decode-policy requirements come from the registry; application
/// publication decisions use the resolver passed to `open_collection`.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct OpenRequest {
    /// Required composited or raw view, fixed before issuing any identity.
    pub view: Composition,
    /// Calls allowed into `images`, `children`, `next` and `describe` in total.
    pub discovery_attempts: u64,
}
impl OpenRequest {
    /// Require a view and an explicit operation-wide discovery allowance.
    pub const fn new(view: Composition, discovery_attempts: u64) -> Self {
        Self {
            view,
            discovery_attempts,
        }
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
        })
    }
}
impl core::error::Error for CollectionOpenError {}
