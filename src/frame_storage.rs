//! Storage contracts shared by image and video outputs.
//!
//! Exported through `crate::frame`. Mapping borrows the backend owner; neither
//! this module nor its owners depend on media sessions or I/O drivers.

use crate::frame::{FrameInterpretation, FrameTiming, SampleEncoding};
use alloc::{boxed::Box, sync::Arc};
use core::fmt;
use zenpixels::ChannelType;

/// Facts about one presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Presentation {
    /// A newly coded, visible key picture (rav1d-safe `is_keyframe`). Not a
    /// random-access promise; container random-access evidence remains separate.
    pub keyframe: bool,
    /// The picture belongs to an earlier presentation (AV1 show-existing).
    pub show_existing: bool,
}

impl Presentation {
    /// A new key picture.
    pub const KEYFRAME: Self = Self {
        keyframe: true,
        show_existing: false,
    };

    /// A replay of an earlier picture.
    pub const SHOW_EXISTING: Self = Self {
        keyframe: false,
        show_existing: true,
    };
}

/// Borrowed samples of one plane, in their storage word.
#[derive(Clone, Copy, Debug)]
pub enum PlaneSamples<'a> {
    /// 8-bit words.
    U8(&'a [u8]),
    /// 16-bit words.
    U16(&'a [u16]),
}

impl PlaneSamples<'_> {
    /// Number of words.
    pub fn len(&self) -> usize {
        match self {
            Self::U8(s) => s.len(),
            Self::U16(s) => s.len(),
        }
    }

    /// Whether there are no words.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The storage word.
    pub fn storage(&self) -> ChannelType {
        match self {
            Self::U8(_) => ChannelType::U8,
            Self::U16(_) => ChannelType::U16,
        }
    }

    fn word(&self, i: usize) -> Option<u16> {
        match self {
            Self::U8(s) => s.get(i).map(|&v| u16::from(v)),
            Self::U16(s) => s.get(i).copied(),
        }
    }
}

/// One plane borrowed from a mapping. The stride is in bytes, as in
/// zenpixels; rows exclude padding. Construction checks the geometry, not
/// the samples.
#[derive(Clone, Copy, Debug)]
pub struct PlaneView<'a> {
    samples: PlaneSamples<'a>,
    width: usize,
    height: usize,
    stride_samples: usize,
    encoding: SampleEncoding,
}

/// A plane whose storage does not fit its geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlaneError {
    /// The word type disagrees with the sample encoding.
    StorageMismatch,
    /// The byte stride is not a whole number of words.
    MisalignedStride,
    /// The stride is shorter than a row.
    ShortStride,
    /// The storage ends before the last sample.
    ShortBuffer,
    /// The geometry overflows addressable storage.
    GeometryOverflow,
}

impl fmt::Display for PlaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StorageMismatch => "plane storage disagrees with its sample encoding",
            Self::MisalignedStride => "plane byte stride does not hold whole samples",
            Self::ShortStride => "plane stride is shorter than its row",
            Self::ShortBuffer => "plane storage does not reach the last sample",
            Self::GeometryOverflow => "plane geometry overflows addressable storage",
        })
    }
}

impl core::error::Error for PlaneError {}

impl<'a> PlaneView<'a> {
    /// A `width` x `height` plane over `samples` with rows `stride_bytes`
    /// apart.
    ///
    /// # Errors
    ///
    /// A [`PlaneError`] when the storage does not fit the geometry.
    pub fn new(
        samples: PlaneSamples<'a>,
        width: usize,
        height: usize,
        stride_bytes: usize,
        encoding: SampleEncoding,
    ) -> Result<Self, PlaneError> {
        if samples.storage() != encoding.storage() {
            return Err(PlaneError::StorageMismatch);
        }
        let bps = encoding.bytes_per_sample();
        if !stride_bytes.is_multiple_of(bps) {
            return Err(PlaneError::MisalignedStride);
        }
        let stride_samples = stride_bytes / bps;
        if width != 0 && height != 0 {
            if stride_samples < width {
                return Err(PlaneError::ShortStride);
            }
            let needed = (height - 1)
                .checked_mul(stride_samples)
                .and_then(|o| o.checked_add(width))
                .ok_or(PlaneError::GeometryOverflow)?;
            if needed > samples.len() {
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

    /// Width in samples.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Distance between rows, in bytes.
    pub fn stride_bytes(&self) -> usize {
        self.stride_samples * self.encoding.bytes_per_sample()
    }

    /// The sample encoding.
    pub fn encoding(&self) -> SampleEncoding {
        self.encoding
    }

    /// Borrow the strided storage through the last active sample, including
    /// padding between rows but excluding trailing allocation space.
    ///
    /// Empty geometry returns an empty slice. This does not copy, normalize,
    /// shift or mask samples: consumers must honor [`Self::encoding`] and
    /// [`Self::stride_bytes`]. Padding has no sample meaning. The borrow is
    /// bounded by the native mapping guard, just like [`Self::row`].
    pub fn as_strided_samples(&self) -> PlaneSamples<'a> {
        let end = if self.width == 0 || self.height == 0 {
            0
        } else {
            // Non-empty geometry and its storage span were checked at new.
            (self.height - 1) * self.stride_samples + self.width
        };
        match self.samples {
            PlaneSamples::U8(s) => PlaneSamples::U8(&s[..end]),
            PlaneSamples::U16(s) => PlaneSamples::U16(&s[..end]),
        }
    }

    /// Row `y`, without padding. A zero-width row is empty regardless of stride.
    pub fn row(&self, y: usize) -> Option<PlaneSamples<'a>> {
        if y >= self.height {
            return None;
        }
        // Empty rows address no storage, even when their logical row number or
        // unused stride would overflow. Non-empty geometry was checked at new.
        let start = if self.width == 0 {
            0
        } else {
            y * self.stride_samples
        };
        let end = start + self.width;
        Some(match self.samples {
            PlaneSamples::U8(s) => PlaneSamples::U8(&s[start..end]),
            PlaneSamples::U16(s) => PlaneSamples::U16(&s[start..end]),
        })
    }

    /// The code value at (x, y): the stored word shifted and masked per the
    /// encoding.
    pub fn code(&self, x: usize, y: usize) -> Option<u16> {
        if x >= self.width {
            return None;
        }
        let w = self.row(y)?.word(x)?;
        let mask = ((1u32 << self.encoding.code_bits()) - 1) as u16;
        Some((w >> self.encoding.bit_shift()) & mask)
    }
}

/// A picture's planes, mapped. Views borrow the mapping, which borrows the
/// picture, so storage stays put while it is read (rav1d-safe: "a mapped
/// plane owns a borrow guard").
pub trait MappedPlanes {
    /// Number of planes.
    fn plane_count(&self) -> usize;

    /// Plane `index`.
    fn plane(&self, index: usize) -> Option<PlaneView<'_>>;
}

/// A picture could not be mapped.
#[derive(Debug)]
#[non_exhaustive]
pub enum MapError {
    /// The storage is not readable on the CPU.
    Unavailable,
    /// The backend refused. Preserve its concrete error and any location trace
    /// instead of formatting them into a message.
    Backend(Box<dyn core::error::Error + Send + Sync + 'static>),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("picture cannot be mapped"),
            Self::Backend(s) => write!(f, "backend mapping failed: {s}"),
        }
    }
}

impl core::error::Error for MapError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Unavailable => None,
            Self::Backend(error) => Some(&**error),
        }
    }
}

/// Backend-owned pixel storage: rav1d's refcounted picture, an owned set of
/// planes, a pool slot.
pub trait Picture: Send + Sync {
    /// Map the planes for reading.
    ///
    /// # Errors
    ///
    /// A [`MapError`] when the storage cannot be read on the CPU.
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError>;
}

/// One presentation of a decoded picture. Cheap to clone; outlives the
/// decoder; may share its picture with another frame.
#[derive(Clone)]
pub struct Frame {
    picture: Arc<dyn Picture>,
    interpretation: Arc<FrameInterpretation>,
    timing: FrameTiming,
    presentation: Presentation,
}

impl Frame {
    /// A frame over `picture`.
    pub fn new(
        picture: Arc<dyn Picture>,
        interpretation: Arc<FrameInterpretation>,
        timing: FrameTiming,
        presentation: Presentation,
    ) -> Self {
        Self {
            picture,
            interpretation,
            timing,
            presentation,
        }
    }

    /// The same picture and interpretation presented again with other
    /// timing (AV1 show-existing, VP8 altref replay).
    pub fn represent(&self, timing: FrameTiming, presentation: Presentation) -> Self {
        Self {
            picture: Arc::clone(&self.picture),
            interpretation: Arc::clone(&self.interpretation),
            timing,
            presentation,
        }
    }

    /// How to read the samples.
    pub fn interpretation(&self) -> &Arc<FrameInterpretation> {
        &self.interpretation
    }

    /// This presentation's timing.
    pub fn timing(&self) -> FrameTiming {
        self.timing
    }

    /// Facts about this presentation.
    pub fn presentation(&self) -> Presentation {
        self.presentation
    }

    /// Map the planes for reading.
    ///
    /// # Errors
    ///
    /// A [`MapError`] when the storage cannot be read on the CPU.
    pub fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        self.picture.map()
    }

    /// Whether both frames present the same picture storage.
    pub fn shares_picture(&self, other: &Frame) -> bool {
        Arc::ptr_eq(&self.picture, &other.picture)
    }

    /// The backing picture.
    pub fn picture(&self) -> &Arc<dyn Picture> {
        &self.picture
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("timing", &self.timing)
            .field("presentation", &self.presentation)
            .field("epoch", &self.interpretation.config_epoch)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strided_span_borrows_raw_storage_without_trailing_space() {
        let bytes = [1, 2, 231, 232, 3, 4, 233, 234];
        let plane = PlaneView::new(PlaneSamples::U8(&bytes), 2, 2, 4, SampleEncoding::U8).unwrap();
        let PlaneSamples::U8(span) = plane.as_strided_samples() else {
            panic!("byte storage changed")
        };
        assert_eq!(span.as_ptr(), bytes.as_ptr());
        assert_eq!(span, &bytes[..6]);
        assert_eq!(plane.code(1, 1), Some(4));

        let words = [0x0043, 0xffc7, 0xdead, 0xbeef, 0x008b, 0x010d, 0xffff];
        let encoding = SampleEncoding::new(ChannelType::U16, 10, 6).unwrap();
        let plane = PlaneView::new(PlaneSamples::U16(&words), 2, 2, 8, encoding).unwrap();
        let PlaneSamples::U16(span) = plane.as_strided_samples() else {
            panic!("word storage changed")
        };
        assert_eq!(span.as_ptr(), words.as_ptr());
        assert_eq!(span, &words[..6]);
        assert_eq!(plane.code(0, 0), Some(1));
        assert_eq!(plane.code(1, 0), Some(1023));
        assert_eq!(plane.code(1, 1), Some(4));
    }

    #[test]
    fn strided_empty_geometry_does_not_expose_or_address_storage() {
        for (width, height, stride) in [(0, usize::MAX, usize::MAX), (usize::MAX, 0, 0)] {
            let plane = PlaneView::new(
                PlaneSamples::U8(&[42]),
                width,
                height,
                stride,
                SampleEncoding::U8,
            )
            .unwrap();
            assert!(plane.as_strided_samples().is_empty());
        }
    }

    #[test]
    fn empty_width_rows_remain_empty_with_any_stride() {
        let plane = PlaneView::new(
            PlaneSamples::U8(&[]),
            0,
            usize::MAX,
            usize::MAX,
            SampleEncoding::U8,
        )
        .unwrap();
        for y in [0, 1, 2, usize::MAX - 1] {
            assert!(plane.row(y).unwrap().is_empty());
            assert_eq!(plane.code(0, y), None);
        }
        assert!(plane.row(usize::MAX).is_none());
        let encoding = SampleEncoding::new(ChannelType::U16, 10, 6).unwrap();
        let words = PlaneView::new(
            PlaneSamples::U16(&[]),
            0,
            usize::MAX,
            usize::MAX - 1,
            encoding,
        )
        .unwrap();
        assert!(words.row(usize::MAX - 1).unwrap().is_empty());
    }
}
