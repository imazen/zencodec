//! Candidate contracts exercised by native codecs before public promotion.
//! An item, its relationship, and its requested composition are distinct.
use std::{error::Error, fmt, sync::Arc};

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

/// One opened source operation. Transport version checks remain the source's
/// obligation; an operation identity does not prove that a remote object held.
#[derive(Default)]
pub struct Origin(Arc<()>);
impl Origin {
    pub fn image(&self, key: u64) -> ImageId {
        ImageId {
            origin: self.0.clone(),
            key,
        }
    }
    pub fn key(&self, id: &ImageId) -> Result<u64, SelectionError> {
        if Arc::ptr_eq(&self.0, &id.origin) {
            Ok(id.key)
        } else {
            Err(SelectionError::ForeignImage)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationKind {
    /// An independent top-level image. This does not assert animation timing.
    TopLevel,
    /// A contained image/directory; not necessarily a reduced-resolution image.
    SubImage,
}

#[derive(Clone, Debug)]
pub struct Relation {
    pub parent: Option<ImageId>,
    pub image: ImageId,
    pub kind: RelationKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Composition {
    /// Request the displayed composition. A native raw layer cannot substitute.
    Composited,
    /// Request an exposed layer before compositing. A flattened result cannot
    /// satisfy this requirement, even if the codec accepts a similarly named flag.
    Uncomposited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionError {
    ForeignImage,
    ForeignCursor,
    DiscoveryLimit,
    Cycle,
    UnsupportedComposition,
    UnsupportedChildren,
}
impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ForeignImage => "image belongs to another source operation",
            Self::ForeignCursor => "discovery cursor belongs to another source operation",
            Self::DiscoveryLimit => "image discovery allowance exceeded",
            Self::Cycle => "cyclic image containment",
            Self::UnsupportedComposition => "requested composition is not implemented",
            Self::UnsupportedChildren => "child-image discovery is not implemented",
        })
    }
}
impl Error for SelectionError {}
