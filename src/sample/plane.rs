// Adapted from imazen/zenpixels fde59b96632557592cffa9b7301acd8db96ed078.
use super::SampleEncoding;
use crate::plane::AnyPlane;
use core::fmt;

/// Chroma resolution relative to the complete luma plane.
///
/// This is the one definition for the zen crates: `zencodec::frame`
/// re-exports it instead of defining its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChromaSubsampling {
    /// One luma plane and no chroma planes (AV1 `I400`, HEVC 4:0:0).
    Mono,
    /// Equal horizontal and vertical resolution.
    Yuv444,
    /// Half horizontal resolution, rounded up at an odd edge.
    Yuv422,
    /// Half horizontal and vertical resolution, rounded up at odd edges.
    Yuv420,
}

impl ChromaSubsampling {
    /// Chroma plane size for a luma size, rounded up at odd edges; `None`
    /// for [`Mono`](Self::Mono).
    pub const fn chroma_size(self, width: u32, height: u32) -> Option<(u32, u32)> {
        match self {
            Self::Mono => None,
            Self::Yuv444 => Some((width, height)),
            Self::Yuv422 => Some((width.div_ceil(2), height)),
            Self::Yuv420 => Some((width.div_ceil(2), height.div_ceil(2))),
        }
    }
}

/// Declared chroma sample location. This is a fact, not a fallback policy:
/// `Unknown` is kept, never defaulted to `Center`. A conversion that needs
/// siting refuses `Unknown` on a subsampled axis or takes it from the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ChromaSiting {
    /// The source did not establish a location.
    #[default]
    Unknown,
    /// Centered horizontally and vertically in the subsampled luma cell
    /// (JPEG style).
    Center,
    /// Horizontally co-sited with the left luma sample, vertically centered
    /// between rows (MPEG-2 style 4:2:0).
    Left,
    /// Co-sited with the top-left luma sample.
    TopLeft,
}

#[derive(Clone, Copy, Debug)]
enum Words<'a> {
    U8(&'a [u8]),
    U16(&'a [u16]),
}

/// Integer codes of one component in a borrowed plane, without normalization
/// or color claims.
///
/// Code depth and placement come from [`crate::sample::SampleEncoding`];
/// padding bits are ignored on reads. The enclosing format supplies roles,
/// range, matrix, transfer and sample location. Nothing is copied or allocated.
///
/// The plane is a [`crate::plane::Plane`] of `u8` or `u16` samples. A plane may
/// hold several interleaved components per position, such as the Cb/Cr plane
/// of NV12 or P010, or interleaved RGB(A); [`component`](Self::component)
/// reads one of them:
///
/// ```
/// use zenpixels::SampleType;
/// use zencodec::{plane::{AnyPlane, PlaneRef}, sample::{CodePlane, SampleEncoding}};
/// // P010 chroma: one interleaved Cb/Cr plane, 10 high bits per sample.
/// let p010 = SampleEncoding::new(SampleType::U16, 10, 6).unwrap();
/// let uv: [u16; 4] = [512 << 6, 100 << 6, 300 << 6, 1023 << 6]; // Cb Cr Cb Cr
/// let plane = AnyPlane::U16(PlaneRef::new(&uv[..], 4, 1, 4).unwrap());
/// let cb = CodePlane::component(plane, 2, 0, p010).unwrap();
/// let cr = CodePlane::component(plane, 2, 1, p010).unwrap();
/// assert_eq!((cb.code(1, 0), cr.code(1, 0)), (Some(300), Some(1023)));
/// ```
#[derive(Clone, Copy, Debug)]
pub struct CodePlane<'a> {
    // Starts at this component's first word.
    words: Words<'a>,
    width: usize,
    height: usize,
    stride_words: usize,
    step_words: usize,
    encoding: SampleEncoding,
}

/// Codes cannot be read from a plane as requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodePlaneError {
    /// The plane's sample type differs from the sample encoding's storage, or
    /// is not an integer type.
    StorageMismatch,
    /// The component index is not below the component count, or the plane's
    /// width does not hold whole positions.
    Components,
    /// A requested read extends beyond the active plane.
    OutsideActivePlane,
}
impl fmt::Display for CodePlaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StorageMismatch => "code plane storage disagrees with sample encoding",
            Self::Components => "component index or plane width disagrees with the component count",
            Self::OutsideActivePlane => "code plane read extends beyond the active plane",
        })
    }
}
impl core::error::Error for CodePlaneError {}

impl<'a> CodePlane<'a> {
    /// Codes of a plane holding one component per position.
    ///
    /// # Errors
    ///
    /// [`CodePlaneError::StorageMismatch`] when the plane's samples differ from
    /// `encoding`'s storage.
    pub fn new(plane: AnyPlane<'a>, encoding: SampleEncoding) -> Result<Self, CodePlaneError> {
        Self::component(plane, 1, 0, encoding)
    }

    /// Codes of component `index` in a plane holding `components`
    /// interleaved components per position. The plane's width must be a
    /// multiple of `components`; this view is `width / components` wide.
    ///
    /// # Errors
    ///
    /// [`CodePlaneError::Components`] for an index not below `components` or
    /// a width that is not a multiple of it, and
    /// [`CodePlaneError::StorageMismatch`] when the plane's samples differ from
    /// `encoding`'s storage.
    pub fn component(
        plane: AnyPlane<'a>,
        components: usize,
        index: usize,
        encoding: SampleEncoding,
    ) -> Result<Self, CodePlaneError> {
        if index >= components || !plane.width().is_multiple_of(components) {
            return Err(CodePlaneError::Components);
        }
        if plane.channel_type() != encoding.storage() {
            return Err(CodePlaneError::StorageMismatch);
        }
        let (width, height) = (plane.width() / components, plane.height());
        // A nonempty plane's span reaches past `index`; an empty one is never read.
        let skip = if width == 0 || height == 0 { 0 } else { index };
        let (words, stride_words) = match plane {
            AnyPlane::U8(p) => (Words::U8(&p.into_strided_samples()[skip..]), p.stride()),
            AnyPlane::U16(p) => (Words::U16(&p.into_strided_samples()[skip..]), p.stride()),
            AnyPlane::F32(_) => return Err(CodePlaneError::StorageMismatch),
        };
        Ok(Self {
            words,
            width,
            height,
            stride_words,
            step_words: components,
            encoding,
        })
    }

    /// Active samples per row.
    pub const fn width(self) -> usize {
        self.width
    }
    /// Number of active rows.
    pub const fn height(self) -> usize {
        self.height
    }
    /// Storage width, code depth and bit placement; not a normalized pixel format.
    pub const fn encoding(self) -> SampleEncoding {
        self.encoding
    }

    /// Extract one code, ignoring padding bits; outside the active plane is None.
    pub fn code(self, x: usize, y: usize) -> Option<u16> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = y * self.stride_words + x * self.step_words;
        let word = match self.words {
            Words::U8(s) => u16::from(s[i]),
            Words::U16(s) => s[i],
        };
        Some((word >> self.encoding.bit_shift()) & self.code_mask())
    }

    /// Extract `codes.len()` consecutive codes of row `y`, starting at column `x`.
    ///
    /// Padding bits are ignored, as in [`code`](Self::code). The extent is checked
    /// before writing; on error `codes` is unchanged. This reads only the requested
    /// samples and allocates nothing.
    pub fn read_codes(self, x: usize, y: usize, codes: &mut [u16]) -> Result<(), CodePlaneError> {
        if y >= self.height
            || x.checked_add(codes.len())
                .is_none_or(|end| end > self.width)
        {
            return Err(CodePlaneError::OutsideActivePlane);
        }
        let Some(last) = codes.len().checked_sub(1) else {
            return Ok(());
        };
        // Construction proved the last active sample is addressable, so these
        // products of smaller coordinates cannot overflow.
        let start = y * self.stride_words + x * self.step_words;
        let end = start + last * self.step_words + 1;
        let (shift, mask) = (self.encoding.bit_shift(), self.code_mask());
        match self.words {
            Words::U8(s) => {
                for (code, word) in codes
                    .iter_mut()
                    .zip(s[start..end].iter().step_by(self.step_words))
                {
                    *code = (u16::from(*word) >> shift) & mask;
                }
            }
            Words::U16(s) => {
                for (code, word) in codes
                    .iter_mut()
                    .zip(s[start..end].iter().step_by(self.step_words))
                {
                    *code = (*word >> shift) & mask;
                }
            }
        }
        Ok(())
    }

    const fn code_mask(self) -> u16 {
        ((1u32 << self.encoding.code_bits()) - 1) as u16
    }
}
