//! Native component reconstruction. Output is unclipped, source-encoded RGB.
//!
//! Matrix/range reconstruction, transfer decoding, primary conversion and display
//! rendering are different operations. This module does the first operation;
//! it never guesses a matrix from primaries or labels the result as sRGB.

use crate::plane::{Plane, PlaneError};
use std::fmt;
use zenpixels::Cicp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Subsampling {
    Monochrome,
    Yuv420,
    Yuv422,
    Yuv444,
}

impl Subsampling {
    fn shifts(self) -> (usize, usize) {
        match self {
            Self::Yuv420 => (1, 1),
            Self::Yuv422 => (1, 0),
            Self::Monochrome | Self::Yuv444 => (0, 0),
        }
    }
}

/// Location of chroma samples relative to luma sample centers, in luma pixels.
/// A nonsubsampled axis always has zero offset. No default is implied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChromaLocation {
    Unknown,
    /// Halfway between adjacent luma samples on each subsampled axis.
    Center,
    /// Horizontally colocated, vertically centered (AV1 CSP_VERTICAL).
    Left,
    /// Colocated on both axes (AV1 CSP_COLOCATED).
    TopLeft,
}

/// Validated borrowed native components. Slots are Y/Cb/Cr, or G/B/R for
/// identity matrix. All components have the same code precision; their word
/// storage, padding and alignment may differ. Alpha is not a luma component.
#[derive(Clone, Copy, Debug)]
pub struct YuvView<'a> {
    planes: [Option<Plane<'a>>; 3],
    subsampling: Subsampling,
    location: ChromaLocation,
    color: Cicp,
    origin: (usize, usize),
    size: (usize, usize),
    bits: u8,
}

impl<'a> YuvView<'a> {
    #[cfg(feature = "av1-encode")]
    pub(crate) fn uncropped_planes(self) -> Option<[Option<Plane<'a>>; 3]> {
        let y = self.planes[0]?;
        (self.origin == (0, 0) && self.size == (y.width(), y.height())).then_some(self.planes)
    }

    pub fn new(
        luma: Plane<'a>,
        chroma: Option<[Plane<'a>; 2]>,
        subsampling: Subsampling,
        location: ChromaLocation,
        color: Cicp,
    ) -> Result<Self, ColorError> {
        let size = (luma.width(), luma.height());
        if size.0 == 0 || size.1 == 0 {
            return Err(ColorError::EmptyFrame);
        }
        let bits = luma.encoding().code_bits();
        if !(8..=16).contains(&bits) {
            return Err(ColorError::UnsupportedPrecision(bits));
        }
        if (subsampling == Subsampling::Monochrome) != chroma.is_none() {
            return Err(ColorError::ComponentLayout);
        }
        let (sx, sy) = subsampling.shifts();
        let expected = (size.0.div_ceil(1 << sx), size.1.div_ceil(1 << sy));
        let planes = match chroma {
            None => [Some(luma), None, None],
            Some([cb, cr]) => {
                for p in [cb, cr] {
                    if (p.width(), p.height()) != expected {
                        return Err(ColorError::ComponentLayout);
                    }
                    if p.encoding().code_bits() != bits {
                        return Err(ColorError::MixedPrecision);
                    }
                }
                [Some(luma), Some(cb), Some(cr)]
            }
        };
        Ok(Self {
            planes,
            subsampling,
            location,
            color,
            origin: (0, 0),
            size,
            bits,
        })
    }

    pub fn width(self) -> usize {
        self.size.0
    }
    pub fn height(self) -> usize {
        self.size.1
    }
    pub fn bit_depth(self) -> u8 {
        self.bits
    }
    pub fn subsampling(self) -> Subsampling {
        self.subsampling
    }
    pub fn chroma_location(self) -> ChromaLocation {
        self.location
    }
    pub fn color(self) -> Cicp {
        self.color
    }

    /// Original complete component in Y/Cb/Cr (or identity G/B/R) order.
    /// A crop retains these planes and its reconstruction phase. Missing
    /// monochrome chroma and indices above two return `None`.
    pub fn plane(self, index: usize) -> Option<Plane<'a>> {
        self.planes.get(index).copied().flatten()
    }

    /// Explicit caller interpretation, for example when container metadata
    /// supplies a location absent from the bitstream. The original bitstream
    /// claim remains available on the owning decoder frame.
    pub fn with_chroma_location(mut self, location: ChromaLocation) -> Self {
        self.location = location;
        self
    }

    /// Crop in current-view luma coordinates, retaining the original chroma
    /// phase and neighboring samples. Chained odd-origin crops do not rebase
    /// chroma to zero or clamp reconstruction at the crop edge.
    pub fn crop(self, x: usize, y: usize, width: usize, height: usize) -> Result<Self, ColorError> {
        if width == 0 || height == 0 {
            return Err(ColorError::EmptyFrame);
        }
        if x > self.size.0 || y > self.size.1 || width > self.size.0 - x || height > self.size.1 - y
        {
            return Err(ColorError::OutsideFrame);
        }
        Ok(Self {
            origin: (self.origin.0 + x, self.origin.1 + y),
            size: (width, height),
            ..self
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum Matrix {
    Mono,
    Identity,
    Ncl {
        r_cr: f64,
        g_cb: f64,
        g_cr: f64,
        b_cb: f64,
    },
}

/// Prepared scalar reconstruction with bilinear chroma interpolation and edge
/// replication at the original frame boundary. It allocates no pixel storage.
///
/// This is a deterministic filter policy, not a claim that a codec mandates
/// bilinear upsampling. Future filters must be named rather than silently
/// replacing these pixels. Unknown siting on subsampled axes is an error.
pub struct YuvToRgb<'a> {
    view: YuvView<'a>,
    matrix: Matrix,
    y_offset: f64,
    y_scale: f64,
    c_offset: f64,
    c_scale: f64,
}

impl<'a> YuvToRgb<'a> {
    pub fn new(view: YuvView<'a>) -> Result<Self, ColorError> {
        let matrix = if view.subsampling == Subsampling::Monochrome {
            Matrix::Mono
        } else {
            if view.subsampling != Subsampling::Yuv444 && view.location == ChromaLocation::Unknown {
                return Err(ColorError::UnknownChromaLocation);
            }
            match view.color.matrix_coefficients {
                0 if view.subsampling != Subsampling::Yuv444 => {
                    return Err(ColorError::IdentitySubsampling);
                }
                0 => Matrix::Identity,
                code => {
                    let (kr, kb) = match code {
                        1 => (0.2126, 0.0722),
                        5 | 6 => (0.299, 0.114),
                        9 => (0.2627, 0.0593),
                        _ => return Err(ColorError::UnsupportedMatrix(code)),
                    };
                    let kg = 1.0 - kr - kb;
                    Matrix::Ncl {
                        r_cr: 2.0 * (1.0 - kr),
                        g_cb: -2.0 * kb * (1.0 - kb) / kg,
                        g_cr: -2.0 * kr * (1.0 - kr) / kg,
                        b_cb: 2.0 * (1.0 - kb),
                    }
                }
            }
        };
        let max = f64::from((1_u32 << view.bits) - 1);
        let scale = f64::from(1_u32 << (view.bits - 8));
        let (y_offset, y_scale, c_scale) = if view.color.full_range {
            (0.0, 1.0 / max, 1.0 / max)
        } else {
            (16.0 * scale, 1.0 / (219.0 * scale), 1.0 / (224.0 * scale))
        };
        Ok(Self {
            view,
            matrix,
            y_offset,
            y_scale,
            c_offset: 128.0 * scale,
            c_scale,
        })
    }

    pub fn width(&self) -> usize {
        self.view.width()
    }
    pub fn height(&self) -> usize {
        self.view.height()
    }

    /// Description of the source-encoded RGB output: unchanged primaries and
    /// transfer, identity matrix, full-scale floating components. Values may
    /// extend outside [0,1]. This does not manufacture an ICC profile.
    pub fn output_color(&self) -> Cicp {
        Cicp::new(
            self.view.color.color_primaries,
            self.view.color.transfer_characteristics,
            0,
            true,
        )
    }

    /// Write one tightly packed RGB f32 row in source encoding. The destination
    /// must have exactly `width()` RGB triples; all validation precedes writes.
    /// No transfer decoding, clipping, gamut mapping, alpha or tone mapping.
    pub fn write_row(&self, row: usize, destination: &mut [[f32; 3]]) -> Result<(), ColorError> {
        self.write_row_impl(row, destination, |rgb| rgb.map(|v| v as f32))
    }

    // Integer packing must not round through f32 near a U16 half-step.
    pub(crate) fn write_row_f64(
        &self,
        row: usize,
        destination: &mut [[f64; 3]],
    ) -> Result<(), ColorError> {
        self.write_row_impl(row, destination, |rgb| rgb)
    }

    fn write_row_impl<T>(
        &self,
        row: usize,
        destination: &mut [T],
        convert: impl Fn([f64; 3]) -> T,
    ) -> Result<(), ColorError> {
        if row >= self.height() {
            return Err(ColorError::OutsideFrame);
        }
        if destination.len() != self.width() {
            return Err(ColorError::OutputWidth);
        }
        let y = self.view.origin.1 + row;
        let luma = self.view.planes[0].expect("validated luma");
        for (column, rgb) in destination.iter_mut().enumerate() {
            let x = self.view.origin.0 + column;
            let first = (f64::from(luma.code(x, y).expect("validated coordinates"))
                - self.y_offset)
                * self.y_scale;
            let values = match self.matrix {
                Matrix::Mono => [first; 3],
                Matrix::Identity => {
                    let b = self.chroma(1, x, y);
                    let r = self.chroma(2, x, y);
                    [
                        (r - self.y_offset) * self.y_scale,
                        first,
                        (b - self.y_offset) * self.y_scale,
                    ]
                }
                Matrix::Ncl {
                    r_cr,
                    g_cb,
                    g_cr,
                    b_cb,
                } => {
                    let cb = (self.chroma(1, x, y) - self.c_offset) * self.c_scale;
                    let cr = (self.chroma(2, x, y) - self.c_offset) * self.c_scale;
                    [
                        first + r_cr * cr,
                        first + g_cb * cb + g_cr * cr,
                        first + b_cb * cb,
                    ]
                }
            };
            *rgb = convert(values);
        }
        Ok(())
    }

    fn chroma(&self, component: usize, x: usize, y: usize) -> f64 {
        let p = self.view.planes[component].expect("validated chroma");
        let (sx, sy) = self.view.subsampling.shifts();
        let (center_x, center_y) = match self.view.location {
            ChromaLocation::Center => (true, true),
            ChromaLocation::Left => (false, true),
            ChromaLocation::Unknown | ChromaLocation::TopLeft => (false, false),
        };
        let (x0, x1, fx) = chroma_axis(x, sx, center_x, p.width());
        let (y0, y1, fy) = chroma_axis(y, sy, center_y, p.height());
        let sample = |xx, yy| f64::from(p.code(xx, yy).expect("validated chroma coordinates"));
        let top = sample(x0, y0) * (1.0 - fx) + sample(x1, y0) * fx;
        let bottom = sample(x0, y1) * (1.0 - fx) + sample(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

// Integer coordinates preserve phase even when usize exceeds exact f64 integers.
fn chroma_axis(position: usize, shift: usize, centered: bool, len: usize) -> (usize, usize, f64) {
    if shift == 0 {
        return (position, position, 0.0);
    }
    let base = position / 2;
    let (left, fraction) = if centered {
        if position == 0 {
            return (0, 0, 0.0);
        }
        if position.is_multiple_of(2) {
            (base - 1, 0.75)
        } else {
            (base, 0.25)
        }
    } else {
        (base, if position.is_multiple_of(2) { 0.0 } else { 0.5 })
    };
    (
        left.min(len - 1),
        left.saturating_add(1).min(len - 1),
        fraction,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColorError {
    Plane(PlaneError),
    EmptyFrame,
    ComponentLayout,
    MixedPrecision,
    UnsupportedPrecision(u8),
    UnknownChromaLocation,
    IdentitySubsampling,
    UnsupportedMatrix(u8),
    OutsideFrame,
    OutputWidth,
}

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plane(error) => error.fmt(f),
            Self::EmptyFrame => f.write_str("video frame or crop has zero area"),
            Self::ComponentLayout => f.write_str("component geometry disagrees with subsampling"),
            Self::MixedPrecision => f.write_str("component code precisions differ"),
            Self::UnsupportedPrecision(bits) => {
                write!(f, "unsupported {bits}-bit component precision")
            }
            Self::UnknownChromaLocation => {
                f.write_str("subsampled chroma needs an explicit sample location")
            }
            Self::IdentitySubsampling => f.write_str("identity matrix requires 4:4:4 components"),
            Self::UnsupportedMatrix(code) => {
                write!(f, "unsupported CICP matrix coefficient {code}")
            }
            Self::OutsideFrame => f.write_str("row or crop lies outside the frame"),
            Self::OutputWidth => f.write_str("output RGB row width does not match the conversion"),
        }
    }
}
impl std::error::Error for ColorError {}
impl From<PlaneError> for ColorError {
    fn from(error: PlaneError) -> Self {
        Self::Plane(error)
    }
}
