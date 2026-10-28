//! Native experiment uses the shared identity and selection requirements.
pub use zencodec::collection::{
    Composition, ImageId, ImageRequest, Origin, RegionRequest, SelectionError,
};
pub use zencodec::image::{ImageGeometry, Rect};

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
