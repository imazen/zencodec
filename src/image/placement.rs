//! Signed presentation placement without changing local sample bounds.
use super::{GeometryError, ImageGeometry, Rect};

/// Position of a selected image on a separate presentation canvas.
/// Coordinates precede residual orientation; placement does not blend samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Nonempty presentation canvas width and height.
    pub canvas: (u32, u32),
    /// Position of the selected image's stored (0, 0), before orientation.
    pub origin: (i64, i64),
}

/// Matching visible rectangles in sample-buffer and presentation coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacementOverlap {
    /// Coordinates relative to the returned sample buffer, not the full image.
    pub samples: Rect,
    /// Corresponding rectangle on the presentation canvas.
    pub canvas: Rect,
}

/// A placement cannot be interpreted without invalid or overflowing geometry.
#[derive(Debug)]
#[non_exhaustive]
pub enum PlacementError {
    /// Invalid selected-image geometry.
    Region(GeometryError),
    /// A presentation canvas has a zero dimension.
    EmptyCanvas,
    /// The complete selected image exceeds signed coordinate range.
    CoordinateOverflow,
}
impl core::fmt::Display for PlacementError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Region(error) => error.fmt(f),
            Self::EmptyCanvas => f.write_str("placement requires a nonempty presentation canvas"),
            Self::CoordinateOverflow => f.write_str("placed image exceeds signed coordinate range"),
        }
    }
}
impl core::error::Error for PlacementError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Region(error) => Some(error),
            _ => None,
        }
    }
}
impl Placement {
    /// Return the visible sample/canvas rectangles without altering the owner.
    /// `None` means a valid image lies wholly outside this canvas, not EOF.
    /// A region request remains in selected-image coordinates. This operation
    /// never turns a partially decoded region into a full-image reconstruction.
    pub fn visible(
        self,
        geometry: ImageGeometry,
    ) -> Result<Option<PlacementOverlap>, PlacementError> {
        geometry.validate().map_err(PlacementError::Region)?;
        if self.canvas.0 == 0 || self.canvas.1 == 0 {
            return Err(PlacementError::EmptyCanvas);
        }
        // Validate the whole selected-image placement, not only a small region
        // that happens to avoid overflow. Positive sizes cannot underflow.
        self.origin
            .0
            .checked_add(i64::from(geometry.canvas_width))
            .ok_or(PlacementError::CoordinateOverflow)?;
        self.origin
            .1
            .checked_add(i64::from(geometry.canvas_height))
            .ok_or(PlacementError::CoordinateOverflow)?;
        let region = geometry.region;
        let x = self.origin.0 + i64::from(region.x);
        let y = self.origin.1 + i64::from(region.y);
        let left = x.max(0);
        let top = y.max(0);
        let right = (x + i64::from(region.width)).min(i64::from(self.canvas.0));
        let bottom = (y + i64::from(region.height)).min(i64::from(self.canvas.1));
        if left >= right || top >= bottom {
            return Ok(None);
        }
        // Each successful intersection lies inside both validated rectangles.
        let width = u32::try_from(right - left).map_err(|_| PlacementError::CoordinateOverflow)?;
        let height = u32::try_from(bottom - top).map_err(|_| PlacementError::CoordinateOverflow)?;
        Ok(Some(PlacementOverlap {
            samples: Rect {
                x: u32::try_from(left - x).map_err(|_| PlacementError::CoordinateOverflow)?,
                y: u32::try_from(top - y).map_err(|_| PlacementError::CoordinateOverflow)?,
                width,
                height,
            },
            canvas: Rect {
                x: left as u32,
                y: top as u32,
                width,
                height,
            },
        }))
    }
}
