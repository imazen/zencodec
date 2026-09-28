//! Borrowed native integer components with checked geometry and interpretation.

use std::fmt;
use zenpixels::ChannelType;
use zenpixels::sample::SampleEncoding;

/// Native-endian words borrowed from a mapped decoder frame or owned allocation.
#[derive(Clone, Copy, Debug)]
pub enum Samples<'a> {
    U8(&'a [u8]),
    U16(&'a [u16]),
}

impl Samples<'_> {
    pub fn len(self) -> usize {
        match self {
            Self::U8(data) => data.len(),
            Self::U16(data) => data.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn storage(self) -> ChannelType {
        match self {
            Self::U8(_) => ChannelType::U8,
            Self::U16(_) => ChannelType::U16,
        }
    }

    fn slice(self, start: usize, end: usize) -> Self {
        match self {
            Self::U8(data) => Self::U8(&data[start..end]),
            Self::U16(data) => Self::U16(&data[start..end]),
        }
    }

    pub fn word(self, index: usize) -> Option<u16> {
        match self {
            Self::U8(data) => data.get(index).copied().map(u16::from),
            Self::U16(data) => data.get(index).copied(),
        }
    }
}

/// One component plane. Strides are bytes; row access excludes padding.
///
/// This is deliberately not a Gray16 image: a plane's Y/Cb/Cr/alpha role and
/// color belong to its enclosing frame. Construction is constant-time and
/// checks storage geometry, not sample conformance or original source precision.
#[derive(Clone, Copy, Debug)]
pub struct Plane<'a> {
    samples: Samples<'a>,
    width: usize,
    height: usize,
    stride_samples: usize,
    encoding: SampleEncoding,
}

impl<'a> Plane<'a> {
    pub fn new(
        samples: Samples<'a>,
        width: usize,
        height: usize,
        stride_bytes: usize,
        encoding: SampleEncoding,
    ) -> Result<Self, PlaneError> {
        if samples.storage() != encoding.storage() {
            return Err(PlaneError::StorageMismatch);
        }
        let sample_bytes = encoding.storage().byte_size();
        if !stride_bytes.is_multiple_of(sample_bytes) {
            return Err(PlaneError::MisalignedStride);
        }
        let stride_samples = stride_bytes / sample_bytes;
        if width != 0 && height != 0 {
            if stride_samples < width {
                return Err(PlaneError::ShortStride);
            }
            let required = (height - 1)
                .checked_mul(stride_samples)
                .and_then(|offset| offset.checked_add(width))
                .ok_or(PlaneError::GeometryOverflow)?;
            if required > samples.len() {
                return Err(PlaneError::ShortBuffer);
            }
        }
        Ok(Self {
            samples,
            width,
            height,
            stride_samples,
            encoding,
        })
    }

    pub fn width(self) -> usize {
        self.width
    }

    pub fn height(self) -> usize {
        self.height
    }

    pub fn stride_bytes(self) -> usize {
        // This value was obtained by exact division of a usize stride.
        self.stride_samples * self.encoding.storage().byte_size()
    }

    pub fn encoding(self) -> SampleEncoding {
        self.encoding
    }

    pub fn row(self, y: usize) -> Result<Samples<'a>, PlaneError> {
        if y >= self.height {
            return Err(PlaneError::OutsidePlane);
        }
        if self.width == 0 {
            return Ok(self.samples.slice(0, 0));
        }
        let start = y * self.stride_samples;
        Ok(self.samples.slice(start, start + self.width))
    }

    /// Read a code value, ignoring non-signal padding bits.
    pub fn code(self, x: usize, y: usize) -> Result<u16, PlaneError> {
        if x >= self.width {
            return Err(PlaneError::OutsidePlane);
        }
        let word = self.row(y)?.word(x).ok_or(PlaneError::OutsidePlane)?;
        let mask = ((1_u32 << self.encoding.code_bits()) - 1) as u16;
        Ok((word >> self.encoding.bit_shift()) & mask)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlaneError {
    StorageMismatch,
    MisalignedStride,
    ShortStride,
    ShortBuffer,
    GeometryOverflow,
    OutsidePlane,
}

impl fmt::Display for PlaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StorageMismatch => "plane storage disagrees with its sample encoding",
            Self::MisalignedStride => "plane byte stride does not contain whole samples",
            Self::ShortStride => "plane stride is shorter than its visible row",
            Self::ShortBuffer => "plane storage does not reach the last visible sample",
            Self::GeometryOverflow => "plane geometry overflows addressable storage",
            Self::OutsidePlane => "sample or row lies outside the visible plane",
        })
    }
}

impl std::error::Error for PlaneError {}
