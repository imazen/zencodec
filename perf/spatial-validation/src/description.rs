//! Selected-image geometry before raster reconstruction. This candidate does
//! not reuse ImageInfo's container-wide sequence and metadata-completeness claims.
use zencodec::{
    At,
    collection::{Collection, CollectionError, ImageId},
    image::{ImageGeometry, Placement, Rect},
};
use zenpixels::Orientation;

/// Source geometry for one issued identity, independent of decoded output.
/// No output format, complete metadata inventory or workspace bound is implied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Description {
    pub stored_size: (u32, u32),
    pub stored_orientation: Option<Orientation>,
    pub placement: Option<Placement>,
}
impl Description {
    pub fn validate(self) -> Result<Self, crate::tiff_region::Failure> {
        let geometry = ImageGeometry {
            canvas_width: self.stored_size.0,
            canvas_height: self.stored_size.1,
            region: Rect {
                x: 0,
                y: 0,
                width: self.stored_size.0,
                height: self.stored_size.1,
            },
        };
        geometry.validate()?;
        if let Some(placement) = self.placement {
            placement.visible(geometry)?;
        }
        Ok(self)
    }
}

/// Explicit selected description. Native discovery can still require parsing
/// earlier headers. Describing an issued ID must not reconstruct its pixels.
pub trait Describe: Collection {
    fn describe(&mut self, image: &ImageId) -> Result<Description, At<CollectionError>>;
}
