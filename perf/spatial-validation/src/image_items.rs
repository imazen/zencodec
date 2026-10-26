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
    UnsupportedRegion,
    InvalidRegion {
        requested: Rect,
        width: u32,
        height: u32,
    },
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
        })
    }
}
impl Error for SelectionError {}

pub use zencodec::image::{ImageGeometry, Rect};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RegionRequest {
    #[default]
    Full,
    Stored(Rect),
}

impl From<Rect> for RegionRequest {
    fn from(rect: Rect) -> Self {
        Self::Stored(rect)
    }
}

impl RegionRequest {
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
pub struct ImageRequest {
    pub region: RegionRequest,
}
