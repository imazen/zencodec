//! What a grid of samples is, before any sample moves.
//!
//! The sample carriers themselves are zenpixels types and are not wrapped:
//! packed rows are [`PixelSlice`], planar strips are
//! [`PlanarSlice`](zenpixels::plane::PlanarSlice), an owned frame is
//! [`PixelBuffer`](zenpixels::PixelBuffer).

use crate::error::{ErrorKind, Result, error_msg};
use alloc::sync::Arc;
use zenpixels::plane::PlanarLayout;
use zenpixels::{AlphaMode, ColorContext, PixelDescriptor, PixelSlice, PixelSliceMut};

/// Packed pixels or separate planes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Layout {
    /// Interleaved channels; the descriptor says which and how.
    Packed(PixelDescriptor),
    /// Separate planes.
    Planar {
        /// Roles, encodings, subsampling and siting.
        planes: Arc<PlanarLayout>,
        /// How the alpha plane relates to color; `Some` exactly when there is one.
        alpha: Option<AlphaMode>,
    },
}

impl Layout {
    /// The packed descriptor, if packed.
    pub const fn descriptor(&self) -> Option<PixelDescriptor> {
        match self {
            Self::Packed(d) => Some(*d),
            Self::Planar { .. } => None,
        }
    }

    /// Whether the samples are in separate planes.
    pub const fn is_planar(&self) -> bool {
        matches!(self, Self::Planar { .. })
    }
}

/// The size, layout and meaning of a frame's samples.
///
/// `color` describes the samples as they cross the contract, not what the
/// source file declared. A planar raster always carries it: YCbCr without a
/// matrix and range is not an image.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Raster {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Packed or planar.
    pub layout: Layout,
    /// What the samples mean.
    pub color: Option<Arc<ColorContext>>,
}

impl Raster {
    /// Packed pixels.
    pub const fn packed(width: u32, height: u32, descriptor: PixelDescriptor) -> Self {
        Self {
            width,
            height,
            layout: Layout::Packed(descriptor),
            color: None,
        }
    }

    /// The size, format and color of packed pixels in hand.
    pub fn of(pixels: &PixelSlice<'_>) -> Self {
        Self {
            width: pixels.width(),
            height: pixels.rows(),
            layout: Layout::Packed(pixels.descriptor()),
            color: pixels.color_context().cloned(),
        }
    }

    /// Separate planes, with the color context that gives them meaning.
    pub fn planar(
        width: u32,
        height: u32,
        planes: Arc<PlanarLayout>,
        alpha: Option<AlphaMode>,
        color: Arc<ColorContext>,
    ) -> Self {
        Self {
            width,
            height,
            layout: Layout::Planar { planes, alpha },
            color: Some(color),
        }
    }

    /// State what the samples mean.
    pub fn with_color(mut self, color: Arc<ColorContext>) -> Self {
        self.color = Some(color);
        self
    }

    /// The packed descriptor, if packed.
    pub const fn descriptor(&self) -> Option<PixelDescriptor> {
        self.layout.descriptor()
    }

    /// Bytes in one tightly packed row; `None` when planar or overflowing.
    pub fn row_bytes(&self) -> Option<usize> {
        let descriptor = self.layout.descriptor()?;
        (self.width as usize).checked_mul(descriptor.bytes_per_pixel())
    }
}

/// How long a frame is shown, as a fraction of a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameDuration {
    /// Numerator, in seconds.
    pub numerator: u32,
    /// Denominator; never zero.
    pub denominator: core::num::NonZeroU32,
}

impl FrameDuration {
    /// A duration in milliseconds.
    pub const fn from_millis(millis: u32) -> Self {
        Self {
            numerator: millis,
            denominator: core::num::NonZeroU32::new(1000).unwrap(),
        }
    }

    /// Milliseconds, rounded down; `None` on overflow.
    pub const fn as_millis(&self) -> Option<u32> {
        let ms = self.numerator as u64 * 1000 / self.denominator.get() as u64;
        if ms > u32::MAX as u64 {
            None
        } else {
            Some(ms as u32)
        }
    }
}

/// Everything fixed about one frame before its rows move.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FrameHeader {
    /// The samples.
    pub raster: Raster,
    /// Position in the sequence, from zero.
    pub index: u32,
    /// How long it is shown; `None` for a still.
    pub duration: Option<FrameDuration>,
}

impl FrameHeader {
    /// The only frame of a still.
    pub const fn still(raster: Raster) -> Self {
        Self {
            raster,
            index: 0,
            duration: None,
        }
    }

    /// A frame of a sequence.
    pub const fn in_sequence(raster: Raster, index: u32, duration: Option<FrameDuration>) -> Self {
        Self {
            raster,
            index,
            duration,
        }
    }
}

/// Whether rows are final or a preview that later rows replace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pass {
    /// These rows will not change.
    #[default]
    Final,
    /// A coarser preview; the number orders previews of the same rows.
    Provisional(u16),
}

/// Copy rows between strided buffers of the same width and format.
///
/// Copies `src.rows()` rows; `dst` must have at least that many.
#[track_caller]
pub fn copy_rows(src: &PixelSlice<'_>, dst: &mut PixelSliceMut<'_>) -> Result<()> {
    if src.width() != dst.width() || src.descriptor() != dst.descriptor() || src.rows() > dst.rows()
    {
        return Err(error_msg(
            ErrorKind::Protocol,
            "row copy between buffers of different shape",
        ));
    }
    let row_bytes = src.width() as usize * src.descriptor().bytes_per_pixel();
    for y in 0..src.rows() {
        dst.row_mut(y)[..row_bytes].copy_from_slice(&src.row(y)[..row_bytes]);
    }
    Ok(())
}
