//! Image discovery and selection independent of codec and transport.
//!
//! Containment, composition, reduced levels and video timing are separate
//! concepts. One opened collection fixes the source operation and view; foreign
//! IDs/cursors and unsupported requests fail at runtime. Successful exhaustion
//! never substitutes for unsupported discovery.
//!
//! The selected output retains its allocation and interpretation. Source revision
//! consistency, native parsing/workspace limits and cancellation inside native
//! calls remain adapter obligations. Publication policy alone cannot prove them.
use crate::{At, image::ImageOutput};
use alloc::{boxed::Box, sync::Arc};
use core::{error::Error, fmt};

mod description;
mod discovery;
mod erased;
mod factory;
pub use description::ImageDescription;
pub use discovery::{Discovery, DiscoveryExhausted, DiscoveryMethod, DiscoveryUsage};
pub use erased::{ErasedCollection, ErasedCursor};
pub use factory::{CollectionFactory, CollectionOpenContext, CollectionOpenError, OpenRequest};

/// Retained concrete backend cause, including an existing location trace.
pub type Cause = Box<dyn Error + Send + Sync + 'static>;

/// Requirement refusal or a retained native failure.
/// Formatting omits arbitrary backend payloads; use Error::source to inspect them.
#[non_exhaustive]
pub enum CollectionError {
    /// A typed selection requirement could not be honored.
    Requirement(SelectionError),
    /// The original backend error, retained for explicit inspection.
    Backend(Cause),
}
impl fmt::Debug for CollectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Requirement(error) => f.debug_tuple("Requirement").field(error).finish(),
            Self::Backend(_) => f.debug_tuple("Backend").finish_non_exhaustive(),
        }
    }
}
impl fmt::Display for CollectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Requirement(error) => error.fmt(f),
            Self::Backend(_) => f.write_str("image collection backend failed"),
        }
    }
}
impl Error for CollectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Requirement(error) => error,
            Self::Backend(error) => error.as_ref(),
        })
    }
}
impl CollectionError {
    /// Classify a boxed requirement or preserve the existing backend cause/trace.
    #[track_caller]
    pub fn located(error: Cause) -> At<Self> {
        At::wrap(match error.downcast::<SelectionError>() {
            Ok(error) => Self::Requirement(*error),
            Err(error) => Self::Backend(error),
        })
    }
}

/// Opaque identity scoped to one opened collection and its fixed view.
///
/// Retaining it does not keep a transport revision current or guarantee cheap
/// seeking. It is not a serializable index, timestamp or reference-frame slot.
#[derive(Clone, Debug)]
pub struct ImageId {
    origin: Arc<()>,
    key: u64,
}
impl PartialEq for ImageId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.origin, &other.origin) && self.key == other.key
    }
}
impl Eq for ImageId {}
impl core::hash::Hash for ImageId {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::hash::Hash::hash(&Arc::as_ptr(&self.origin), state);
        core::hash::Hash::hash(&self.key, state);
    }
}

/// One opened source operation. Transport version checks remain the source's
/// obligation; an operation identity does not prove that a remote object held.
#[derive(Default)]
pub struct Origin(Arc<()>);
impl Origin {
    /// Start a new identity namespace. Cloned IDs keep this identity alive.
    pub fn new() -> Self {
        Self::default()
    }
    /// Issue an identity for the adapter's stable key in this opened view.
    pub fn image(&self, key: u64) -> ImageId {
        ImageId {
            origin: self.0.clone(),
            key,
        }
    }
    /// Recover a key only if the identity belongs to this origin.
    pub fn key(&self, id: &ImageId) -> Result<u64, SelectionError> {
        if Arc::ptr_eq(&self.0, &id.origin) {
            Ok(id.key)
        } else {
            Err(SelectionError::ForeignImage)
        }
    }
}

/// The view fixed before discovery starts; it cannot change under issued IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Composition {
    /// Request the displayed composition. A native raw layer cannot substitute.
    Composited,
    /// Request an exposed layer before compositing. A flattened result cannot
    /// satisfy this requirement, even if the codec accepts a similarly named flag.
    Uncomposited,
}

/// A selection requirement that could not be honored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectionError {
    /// The identity was issued by a different opened source or view.
    ForeignImage,
    /// The cursor was issued by a different opened source or view.
    ForeignCursor,
    /// A native image/header discovery allowance was exhausted.
    DiscoveryLimit,
    /// Native containment would revisit an ancestor.
    Cycle,
    /// The adapter cannot supply the required composited or raw view.
    UnsupportedComposition,
    /// The adapter cannot discover containment relations.
    UnsupportedChildren,
    /// The adapter cannot honor the required stored region.
    UnsupportedRegion,
    /// The required region is empty, overflows, or leaves the selected image.
    InvalidRegion {
        /// The original, unclipped request.
        requested: Rect,
        /// Width of the selected stored image.
        width: u32,
        /// Height of the selected stored image.
        height: u32,
    },
    /// The adapter cannot establish source geometry without reconstructing pixels.
    UnsupportedDescription,
}
impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Self::InvalidRegion {
            requested,
            width,
            height,
        } = self
        {
            return write!(
                f,
                "region {requested:?} is empty or outside the selected {width}x{height} image"
            );
        }
        f.write_str(match self {
            Self::ForeignImage => "image belongs to another source operation",
            Self::ForeignCursor => "discovery cursor belongs to another source operation",
            Self::DiscoveryLimit => "image discovery allowance exceeded",
            Self::Cycle => "cyclic image containment",
            Self::UnsupportedComposition => "requested composition is not implemented",
            Self::UnsupportedChildren => "child-image discovery is not implemented",
            Self::UnsupportedRegion => "required stored region is not implemented",
            Self::InvalidRegion { .. } => unreachable!(),
            Self::UnsupportedDescription => {
                "selected description without pixel reconstruction is not implemented"
            }
        })
    }
}
impl Error for SelectionError {}

pub use crate::image::{ImageGeometry, Rect};

/// Required stored extent, independent of display orientation or layer placement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegionRequest {
    /// The complete selected image.
    #[default]
    Full,
    /// An exact nonempty region; do not clip or substitute the whole image.
    Stored(Rect),
}

impl From<Rect> for RegionRequest {
    fn from(rect: Rect) -> Self {
        Self::Stored(rect)
    }
}

impl RegionRequest {
    /// Validate the required rectangle against the selected image, without clipping.
    pub fn resolve(self, width: u32, height: u32) -> Result<Rect, SelectionError> {
        let rect = match self {
            Self::Full => Rect {
                x: 0,
                y: 0,
                width,
                height,
            },
            Self::Stored(rect) => rect,
        };
        if rect.width == 0
            || rect.height == 0
            || rect.x.checked_add(rect.width).is_none_or(|end| end > width)
            || rect
                .y
                .checked_add(rect.height)
                .is_none_or(|end| end > height)
        {
            return Err(SelectionError::InvalidRegion {
                requested: rect,
                width,
                height,
            });
        }
        Ok(rect)
    }
}

/// Intent belongs to one decode, allowing independent requests for pages and
/// contained images with different geometry. Composition fixes discovery instead.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ImageRequest {
    /// Required extent in stored coordinates.
    pub region: RegionRequest,
}

impl ImageRequest {
    /// Request the complete selected image.
    pub const fn new() -> Self {
        Self {
            region: RegionRequest::Full,
        }
    }
    /// Require a stored region. Unsupported native selection returns an error.
    pub fn with_region(mut self, region: impl Into<RegionRequest>) -> Self {
        self.region = region.into();
        self
    }
}

/// Opening the concrete adapter fixes its view. Discovery can then be lazy and
/// selection IDs remain stable while the source operation lives.
/// Adapters receive the operation StopToken at open and must check it
/// during discovery/replay and before returning an ID, exhaustion or output.
/// This is cooperative cancellation, not a hard native-call latency bound.
pub trait Collection {
    /// Adapter-owned discovery state. Foreign cursors must be rejected.
    type Cursor;
    /// The view fixed at open. This is descriptive, not authorization.
    fn view(&self) -> Composition;
    /// Begin lazy discovery of top-level images.
    fn images(&mut self) -> Result<Self::Cursor, At<CollectionError>>;
    /// Begin containment discovery. Unsupported discovery is an error, not exhaustion.
    fn children(&mut self, image: &ImageId) -> Result<Self::Cursor, At<CollectionError>>;
    /// Return the next stable identity, or successful exhaustion.
    fn next(&mut self, cursor: &mut Self::Cursor) -> Result<Option<ImageId>, At<CollectionError>>;
    /// Establish selected source geometry without reconstructing pixel samples.
    /// Discovery may have parsed earlier headers or traversed compressed data.
    /// This call must validate its facts, honor the source revision and stop
    /// token, and reject foreign identities. It neither advances a cursor nor
    /// promises that a later decode succeeds. Unsupported inspection refuses;
    /// it must never fall back to decoding a full raster for these facts.
    fn describe(&mut self, image: &ImageId) -> Result<ImageDescription, At<CollectionError>>;
    /// Decode the required selected extent and retain immutable interpretation with its owner.
    /// This mechanism does not perform application publication policy; use
    /// [`crate::operation::CheckedCollection`] for the ordinary checked path.
    fn decode(
        &mut self,
        image: &ImageId,
        request: &ImageRequest,
    ) -> Result<ImageOutput, At<CollectionError>>;
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::format;
    use std::collections::HashSet;

    #[test]
    fn collection_identity_survives_origin_drop_without_aliasing_another_source() {
        let a = Origin::new();
        let b = Origin::new();
        let first = a.image(7);
        let repeated = a.image(7);
        let other = b.image(7);
        let ids = HashSet::from([first.clone(), repeated.clone(), other.clone()]);
        assert_eq!(ids.len(), 2);
        assert_eq!(a.key(&other), Err(SelectionError::ForeignImage));
        drop(a);
        assert!(ids.contains(&repeated));
        assert_eq!(first, repeated);
        assert_ne!(first, other);
    }

    #[test]
    fn collection_backend_payload_is_explicit_while_original_trace_is_retained() {
        #[derive(Debug)]
        struct PrivateCause;
        impl fmt::Display for PrivateCause {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("untrusted metadata payload")
            }
        }
        impl Error for PrivateCause {}
        let native = At::wrap(PrivateCause);
        let original = format!("{native:?}");
        let error = CollectionError::located(Box::new(native));
        assert!(!format!("{error}").contains("untrusted metadata payload"));
        assert!(!format!("{error:?}").contains("PrivateCause"));
        let retained = crate::find_cause::<At<PrivateCause>>(&error).unwrap();
        assert_eq!(format!("{retained:?}"), original);
    }
}
