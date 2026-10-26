//! Selected source geometry, before allocating reconstructed samples.
use super::{CollectionError, ImageGeometry, Rect};
use crate::{At, image::Placement};
use alloc::boxed::Box;
use zenpixels::Orientation;

/// Established source facts for one selected identity and composition view.
///
/// This is not a complete metadata inventory, rendered pixel format, decoder
/// capability promise or workspace estimate. Later reconstruction can still
/// fail. Stored geometry is local to the selected image, before orientation;
/// a layer's separate presentation canvas belongs in `placement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageDescription {
    /// Nonempty local stored width and height.
    pub stored_size: (u32, u32),
    /// Established stored orientation. `None` does not mean identity.
    pub stored_orientation: Option<Orientation>,
    /// Established signed placement on a separate presentation canvas.
    pub placement: Option<Placement>,
}
impl ImageDescription {
    /// Describe local stored dimensions, leaving other facts unestablished.
    pub const fn new(stored_size: (u32, u32)) -> Self {
        Self {
            stored_size,
            stored_orientation: None,
            placement: None,
        }
    }
    /// Establish stored orientation without applying it to the dimensions.
    pub const fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.stored_orientation = Some(orientation);
        self
    }
    /// Establish separate placement without clipping local samples.
    pub const fn with_placement(mut self, placement: Placement) -> Self {
        self.placement = Some(placement);
        self
    }
    /// Check nonempty geometry and signed placement without decoding pixels.
    /// Native adapters and the checked consumer validate before returning facts.
    #[track_caller]
    pub fn validate(&self) -> Result<(), At<CollectionError>> {
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
        geometry
            .validate()
            .map_err(|e| CollectionError::located(Box::new(e)))?;
        if let Some(placement) = self.placement {
            placement
                .visible(geometry)
                .map_err(|e| CollectionError::located(Box::new(e)))?;
        }
        Ok(())
    }
}
