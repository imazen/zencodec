// Adapted from imazen/zenpixels fde59b96632557592cffa9b7301acd8db96ed078.
//! Strided planes and the layout of a planar picture.
//!
//! [`Plane`] is a two-dimensional array of samples with a row stride,
//! generic over who owns the samples: [`PlaneRef`] borrows them, [`PlaneMut`]
//! borrows them mutably, [`PlaneBuf`] owns a `Vec`, [`AlignedPlane`] owns
//! [`AlignedSamples`], and any other owner
//! (shared or accounted storage) implements [`SampleStorage`]. Rows are
//! contiguous `&[T]` slices, so kernels read them without per-sample
//! dispatch. Samples are native-endian: an adapter for big-endian samples
//! (FFmpeg's `*BE` formats) swaps them into a plane; a plane never holds
//! byte-swapped samples. A plane says nothing about what its samples mean.
//!
//! [`PlanarLayout`] says what they mean: each plane's [`PlaneRole`] (which
//! component, or for semi-planar chroma which two interleaved components), its
//! [`PlaneEncoding`] (integer codes per [`SampleEncoding`], or scaled floats),
//! and the picture's chroma subsampling and siting. Each plane has its own
//! encoding, so alpha or extra planes can differ in depth from color.
//!
//! Matrix, range, primaries, transfer and ICC are not part of the layout: they
//! travel with the enclosing descriptor (for example `zenpixels::Cicp`), as for
//! packed pixels. Range applies to integer code planes; a float plane's scale
//! and offset already say where its nominal values lie.
//!
//! ```
//! use zenpixels::SampleType;
//! use zencodec::{plane::*, sample::*};
//!
//! // NV12: a luma plane and one plane of interleaved Cb, Cr.
//! let u8_codes = PlaneEncoding::Code(SampleEncoding::new(SampleType::U8, 8, 0).unwrap());
//! let layout = PlanarLayout::new(
//!     vec![
//!         PlaneSpec::new(PlaneRole::Luma, u8_codes),
//!         PlaneSpec::new(PlaneRole::CbCr, u8_codes),
//!     ],
//!     ChromaSubsampling::Yuv420,
//!     ChromaSiting::Left,
//! )
//! .unwrap();
//! assert_eq!(layout.plane_size(1, 5, 3), Some((3, 2)));
//!
//! let y = AlignedPlane::<u8>::alloc(5, 3).unwrap();
//! let uv = AlignedPlane::<u8>::alloc(3 * PlaneRole::CbCr.components(), 2).unwrap();
//! layout
//!     .check(&[AnyPlane::U8(y.view()), AnyPlane::U8(uv.view())], 5, 3)
//!     .unwrap();
//! ```

use crate::sample::{ChromaSiting, ChromaSubsampling, SampleEncoding};
use alloc::{boxed::Box, vec::Vec};
use core::fmt;
use zenpixels::SampleType as ChannelType;

mod sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
    impl Sealed for f32 {}
}

/// The sample type of a plane: `u8`, `u16` or `f32`.
///
/// Sealed; `f16` can be added when a consumer exchanges half-float planes.
pub trait PlaneSample:
    sealed::Sealed + bytemuck::Pod + Default + PartialEq + fmt::Debug + Send + Sync + 'static
{
    /// The channel type of this sample.
    const CHANNEL_TYPE: ChannelType;
}
impl PlaneSample for u8 {
    const CHANNEL_TYPE: ChannelType = ChannelType::U8;
}
impl PlaneSample for u16 {
    const CHANNEL_TYPE: ChannelType = ChannelType::U16;
}
impl PlaneSample for f32 {
    const CHANNEL_TYPE: ChannelType = ChannelType::F32;
}

/// Read access to the samples of a plane's owner.
///
/// Implement this for an owner type (for example accounted shared storage) to
/// use it as `Plane<YourOwner>`. `samples` must return the same slice on every
/// call: geometry is checked once, at construction, and an owner whose samples
/// shrink afterwards makes row access panic.
///
/// Slices, references, boxes and `Vec` implement it, and the trait stays
/// dyn-compatible, so an owner can also be hidden as
/// `Plane<Box<dyn SampleStorage<Sample = T> + Send + Sync>>`. Planes of one
/// picture normally share one owner type per sample type; borrowed views
/// ([`Plane::view`], [`AnyPlane`]) erase the owner at no cost.
pub trait SampleStorage {
    /// The sample type.
    type Sample: PlaneSample;
    /// The samples, including any padding.
    fn samples(&self) -> &[Self::Sample];
}

/// Write access to the samples of a plane's owner.
pub trait SampleStorageMut: SampleStorage {
    /// The samples, including any padding.
    fn samples_mut(&mut self) -> &mut [Self::Sample];
}

impl<T: PlaneSample> SampleStorage for [T] {
    type Sample = T;
    fn samples(&self) -> &[T] {
        self
    }
}
impl<T: PlaneSample> SampleStorageMut for [T] {
    fn samples_mut(&mut self) -> &mut [T] {
        self
    }
}
impl<S: SampleStorage + ?Sized> SampleStorage for &S {
    type Sample = S::Sample;
    fn samples(&self) -> &[S::Sample] {
        (**self).samples()
    }
}
impl<S: SampleStorage + ?Sized> SampleStorage for &mut S {
    type Sample = S::Sample;
    fn samples(&self) -> &[S::Sample] {
        (**self).samples()
    }
}
impl<S: SampleStorageMut + ?Sized> SampleStorageMut for &mut S {
    fn samples_mut(&mut self) -> &mut [S::Sample] {
        (**self).samples_mut()
    }
}
// `Vec` and `Box` storage. With the `allocator_api` feature on Rust 1.100 or newer
// these cover every allocator, which includes the global one, so existing uses
// keep compiling; before 1.100 the feature changes nothing.
macro_rules! owned_storage {
    ($($alloc:ident)?) => {
        impl<S: SampleStorage + ?Sized $(, $alloc: core::alloc::Allocator)?> SampleStorage
            for Box<S $(, $alloc)?>
        {
            type Sample = S::Sample;
            fn samples(&self) -> &[S::Sample] {
                (**self).samples()
            }
        }
        impl<S: SampleStorageMut + ?Sized $(, $alloc: core::alloc::Allocator)?> SampleStorageMut
            for Box<S $(, $alloc)?>
        {
            fn samples_mut(&mut self) -> &mut [S::Sample] {
                (**self).samples_mut()
            }
        }
        impl<T: PlaneSample $(, $alloc: core::alloc::Allocator)?> SampleStorage
            for Vec<T $(, $alloc)?>
        {
            type Sample = T;
            fn samples(&self) -> &[T] {
                self
            }
        }
        impl<T: PlaneSample $(, $alloc: core::alloc::Allocator)?> SampleStorageMut
            for Vec<T $(, $alloc)?>
        {
            fn samples_mut(&mut self) -> &mut [T] {
                self
            }
        }
        impl<T: PlaneSample $(, $alloc: core::alloc::Allocator)?> Plane<Vec<T $(, $alloc)?>> {
            /// Give up the `Vec`, for an owner that takes a plain `Vec` and a stride
            /// (an `ImgVec`, yuvutils-rs' owned planes). Read
            /// [`width`](Plane::width), [`height`](Plane::height) and
            /// [`stride`](Plane::stride) first.
            ///
            /// # Errors
            ///
            /// The plane itself, unchanged, when its first row isn't at index 0.
            /// Today's constructors always put it there, but the first row may start
            /// later in a future aligned form, so this stays fallible. Borrow it with
            /// [`view`](Plane::view) instead, or copy it.
            pub fn try_into_vec(self) -> Result<Vec<T $(, $alloc)?>, Self> {
                if self.start == 0 {
                    Ok(self.buf)
                } else {
                    Err(self)
                }
            }
        }
    };
}
owned_storage!();

/// One allocation unit of [`AlignedSamples`]. Its alignment is what keeps the
/// buffer aligned without an offset, a custom allocator, or `unsafe`: `Vec`
/// allocates and frees it with this layout.
#[derive(Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
#[repr(C, align(64))]
struct Chunk([u8; 64]);

/// Owned samples whose first element is on a 64-byte boundary, on any allocator.
///
/// 64 bytes covers every interleaved pixel type, so the samples can always be
/// viewed as `&[Rgb<u16>]`, `&[Rgba<f32>]` and the like with `bytemuck`, and index
/// 0 is always the first sample. Clones stay aligned. It derefs to `[T]`, so it
/// also serves as an `image::ImageBuffer` container.
///
/// The price is that it cannot become a `Vec<T>` without a copy: a `Vec` has to
/// free memory with the layout it allocated. Adopt a `Vec<T>` directly when the
/// next owner needs one.
#[derive(Clone)]
pub struct AlignedSamples<T> {
    chunks: Vec<Chunk>,
    len: usize,
    _sample: core::marker::PhantomData<T>,
}

impl<T: PlaneSample> AlignedSamples<T> {
    /// Allocate `len` zeroed samples, fallibly.
    ///
    /// # Errors
    ///
    /// [`PlaneError::GeometryOverflow`] when the size overflows,
    /// [`PlaneError::AllocationFailed`] when the allocator refuses.
    pub fn try_zeroed(len: usize) -> Result<Self, PlaneError> {
        let chunk_count = len
            .checked_mul(core::mem::size_of::<T>())
            .ok_or(PlaneError::GeometryOverflow)?
            .div_ceil(core::mem::size_of::<Chunk>());
        // Goes through `alloc_zeroed`, so large buffers get untouched zero pages.
        let chunks: Vec<Chunk> = bytemuck::allocation::try_zeroed_vec(chunk_count)
            .map_err(|()| PlaneError::AllocationFailed)?;
        Ok(Self {
            chunks,
            len,
            _sample: core::marker::PhantomData,
        })
    }
}

impl<T: PlaneSample> core::ops::Deref for AlignedSamples<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &bytemuck::cast_slice(&self.chunks)[..self.len]
    }
}

impl<T: PlaneSample> core::ops::DerefMut for AlignedSamples<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut bytemuck::cast_slice_mut(&mut self.chunks)[..self.len]
    }
}

impl<T> fmt::Debug for AlignedSamples<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AlignedSamples(len: {})", self.len)
    }
}

impl<T: PlaneSample> SampleStorage for AlignedSamples<T> {
    type Sample = T;
    fn samples(&self) -> &[T] {
        self
    }
}

impl<T: PlaneSample> SampleStorageMut for AlignedSamples<T> {
    fn samples_mut(&mut self) -> &mut [T] {
        self
    }
}

/// A plane's geometry does not fit its storage, or a request leaves the plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlaneError {
    /// A nonempty plane's stride is shorter than its row.
    ShortStride,
    /// Storage ends before the last active sample.
    ShortBuffer,
    /// Computing the plane's extent overflows addressable storage.
    GeometryOverflow,
    /// A requested rectangle or row extends beyond the plane.
    OutsidePlane,
    /// Allocating plane storage failed.
    AllocationFailed,
}

impl fmt::Display for PlaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ShortStride => "plane stride is shorter than its row",
            Self::ShortBuffer => "plane storage ends before the last active sample",
            Self::GeometryOverflow => "plane geometry overflows addressable storage",
            Self::OutsidePlane => "request extends beyond the plane",
            Self::AllocationFailed => "plane allocation failed",
        })
    }
}
impl core::error::Error for PlaneError {}

/// A strided two-dimensional array of samples, generic over its owner.
///
/// `width` and `stride` count samples, not bytes, so a stride always holds whole
/// samples. A plane holding two interleaved components (see
/// [`PlaneRole::CbCr`]) is `components * positions` samples wide. The last row
/// need not have padding after it.
#[derive(Clone, Copy)]
pub struct Plane<C> {
    buf: C,
    start: usize,
    width: usize,
    height: usize,
    stride: usize,
}

/// A borrowed plane.
pub type PlaneRef<'a, T> = Plane<&'a [T]>;
/// A mutably borrowed plane.
pub type PlaneMut<'a, T> = Plane<&'a mut [T]>;
/// An owned plane.
pub type PlaneBuf<T> = Plane<Vec<T>>;
/// A plane that owns [`AlignedSamples`]; see [`Plane::alloc`].
pub type AlignedPlane<T> = Plane<AlignedSamples<T>>;

impl<C> fmt::Debug for Plane<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Plane")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("stride", &self.stride)
            .finish_non_exhaustive()
    }
}

/// Samples from the first active sample through the last one.
fn extent(start: usize, width: usize, height: usize, stride: usize) -> Result<usize, PlaneError> {
    if width == 0 || height == 0 {
        return Ok(start);
    }
    if stride < width {
        return Err(PlaneError::ShortStride);
    }
    (height - 1)
        .checked_mul(stride)
        .and_then(|n| n.checked_add(width))
        .and_then(|n| n.checked_add(start))
        .ok_or(PlaneError::GeometryOverflow)
}

impl<C: SampleStorage> Plane<C> {
    /// Adopt storage holding `height` rows of `width` samples, `stride` samples
    /// apart, starting at the first sample. No samples are copied or read.
    ///
    /// # Errors
    ///
    /// [`PlaneError::ShortStride`] when a nonempty plane's stride is shorter
    /// than its width, [`PlaneError::ShortBuffer`] when the storage ends
    /// before the last active sample, [`PlaneError::GeometryOverflow`] when the
    /// extent overflows.
    pub fn new(buf: C, width: usize, height: usize, stride: usize) -> Result<Self, PlaneError> {
        Self::with_start(buf, 0, width, height, stride)
    }

    fn with_start(
        buf: C,
        start: usize,
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, PlaneError> {
        if extent(start, width, height, stride)? > buf.samples().len() {
            return Err(PlaneError::ShortBuffer);
        }
        Ok(Self {
            buf,
            start,
            width,
            height,
            stride,
        })
    }

    /// Samples per row.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Samples from one row start to the next.
    pub fn stride(&self) -> usize {
        self.stride
    }

    // First sample of row `y`. An empty row starts at `start`, which is
    // within the storage even when `y * stride` is not.
    fn row_start(&self, y: usize) -> usize {
        if self.width == 0 {
            self.start
        } else {
            self.start + y * self.stride
        }
    }

    /// Row `y`, without padding; `None` past the last row.
    pub fn row(&self, y: usize) -> Option<&[C::Sample]> {
        if y >= self.height {
            return None;
        }
        let s = self.row_start(y);
        Some(&self.buf.samples()[s..s + self.width])
    }

    /// Every row, top to bottom, without padding.
    pub fn rows(&self) -> impl DoubleEndedIterator<Item = &[C::Sample]> + ExactSizeIterator {
        (0..self.height).map(move |y| {
            let s = self.row_start(y);
            &self.buf.samples()[s..s + self.width]
        })
    }

    /// The samples from the first active sample through the last one, including
    /// padding between rows but not after the last row. Empty for an empty
    /// plane. For handing the plane to an API that takes a slice and a stride.
    pub fn strided_samples(&self) -> &[C::Sample] {
        let end = extent(self.start, self.width, self.height, self.stride)
            .expect("checked at construction");
        &self.buf.samples()[self.start..end]
    }

    /// Borrow this plane.
    pub fn view(&self) -> PlaneRef<'_, C::Sample> {
        Plane {
            buf: self.strided_samples(),
            start: 0,
            width: self.width,
            height: self.height,
            stride: self.stride,
        }
    }

    /// Borrow the `width` x `height` samples at column `x`, row `y`.
    ///
    /// # Errors
    ///
    /// [`PlaneError::OutsidePlane`] when the rectangle leaves the plane.
    pub fn sub(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Result<PlaneRef<'_, C::Sample>, PlaneError> {
        let (start, width, height) = self.sub_rect(x, y, width, height)?;
        Plane::with_start(self.buf.samples(), start, width, height, self.stride)
    }

    fn sub_rect(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Result<(usize, usize, usize), PlaneError> {
        let fits = |o: usize, n: usize, max: usize| o.checked_add(n).is_some_and(|e| e <= max);
        if !fits(x, width, self.width) || !fits(y, height, self.height) {
            return Err(PlaneError::OutsidePlane);
        }
        if width == 0 || height == 0 {
            return Ok((self.start, width, height));
        }
        Ok((self.start + y * self.stride + x, width, height))
    }
}

impl<C: SampleStorageMut> Plane<C> {
    /// Row `y`, without padding; `None` past the last row.
    pub fn row_mut(&mut self, y: usize) -> Option<&mut [C::Sample]> {
        if y >= self.height {
            return None;
        }
        let s = self.row_start(y);
        Some(&mut self.buf.samples_mut()[s..s + self.width])
    }

    /// Every row, top to bottom, without padding.
    pub fn rows_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = &mut [C::Sample]> + ExactSizeIterator {
        let (width, height, stride) = (self.width, self.height, self.stride);
        let end = extent(self.start, width, height, stride).expect("checked at construction");
        let samples = &mut self.buf.samples_mut()[self.start..end];
        RowsMut {
            samples,
            width,
            stride,
            rows: height,
        }
    }

    /// Mutably borrow this plane.
    pub fn view_mut(&mut self) -> PlaneMut<'_, C::Sample> {
        let (width, height, stride) = (self.width, self.height, self.stride);
        let end = extent(self.start, width, height, stride).expect("checked at construction");
        Plane {
            buf: &mut self.buf.samples_mut()[self.start..end],
            start: 0,
            width,
            height,
            stride,
        }
    }

    /// Mutably borrow the `width` x `height` samples at column `x`, row `y`.
    ///
    /// # Errors
    ///
    /// [`PlaneError::OutsidePlane`] when the rectangle leaves the plane.
    pub fn sub_mut(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Result<PlaneMut<'_, C::Sample>, PlaneError> {
        let (start, width, height) = self.sub_rect(x, y, width, height)?;
        let stride = self.stride;
        Plane::with_start(self.buf.samples_mut(), start, width, height, stride)
    }
}

impl<T: PlaneSample> Plane<&mut [T]> {
    /// Split into rows `0..y` and rows `y..`, for filling strips or
    /// handing disjoint rows to threads.
    ///
    /// # Errors
    ///
    /// [`PlaneError::OutsidePlane`] when `y` is past the last row.
    pub fn split_at_row(self, y: usize) -> Result<(Self, Self), PlaneError> {
        if y > self.height {
            return Err(PlaneError::OutsidePlane);
        }
        let Plane {
            buf,
            start,
            width,
            height,
            stride,
        } = self;
        let at = if y == 0 || width == 0 {
            start
        } else {
            start + y * stride
        };
        let at = at.min(buf.len());
        let (top, bottom) = buf.split_at_mut(at);
        Ok((
            Plane {
                buf: top,
                start,
                width,
                height: y,
                stride,
            },
            Plane {
                buf: bottom,
                start: 0,
                width,
                height: height - y,
                stride,
            },
        ))
    }
}

impl<'a, T: PlaneSample> Plane<&'a [T]> {
    /// [`Plane::strided_samples`] for the borrowed lifetime, so an adapter can
    /// hand the samples to another crate's borrowed frame type (slice from the
    /// first sample, stride in samples) without copying.
    pub fn into_strided_samples(self) -> &'a [T] {
        let end = extent(self.start, self.width, self.height, self.stride)
            .expect("checked at construction");
        &self.buf[self.start..end]
    }
}

impl<'a, T: PlaneSample> Plane<&'a mut [T]> {
    /// The mutable samples from the first active sample through the last one,
    /// for the borrowed lifetime; see [`into_strided_samples`](Plane::into_strided_samples).
    pub fn into_strided_samples_mut(self) -> &'a mut [T] {
        let end = extent(self.start, self.width, self.height, self.stride)
            .expect("checked at construction");
        &mut self.buf[self.start..end]
    }
}

struct RowsMut<'a, T> {
    samples: &'a mut [T],
    width: usize,
    stride: usize,
    rows: usize,
}

impl<'a, T> Iterator for RowsMut<'a, T> {
    type Item = &'a mut [T];
    fn next(&mut self) -> Option<&'a mut [T]> {
        if self.rows == 0 {
            return None;
        }
        self.rows -= 1;
        if self.width == 0 {
            return Some(&mut []);
        }
        let samples = core::mem::take(&mut self.samples);
        if self.rows == 0 {
            return Some(&mut samples[..self.width]);
        }
        let (row, rest) = samples.split_at_mut(self.stride);
        self.samples = rest;
        Some(&mut row[..self.width])
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.rows, Some(self.rows))
    }
}

impl<T> DoubleEndedIterator for RowsMut<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.rows == 0 {
            return None;
        }
        self.rows -= 1;
        if self.width == 0 {
            return Some(&mut []);
        }
        let samples = core::mem::take(&mut self.samples);
        let (rest, last) = samples.split_at_mut(self.rows * self.stride);
        self.samples = rest;
        Some(&mut last[..self.width])
    }
}

impl<T> ExactSizeIterator for RowsMut<'_, T> {}

/// Row starts on 64 bytes, the widest SIMD vector and the x86/Neoverse line.
const ROW_ALIGN: usize = 64;

impl<T: PlaneSample> Plane<AlignedSamples<T>> {
    /// Allocate a zeroed `width` x `height` plane for SIMD kernels.
    ///
    /// Every row starts on a 64-byte boundary, with row 0 at index 0. The stride
    /// is the row rounded up to 64 bytes, plus 64 more when that is a multiple of
    /// 4096: such strides put a column's rows in one cache set, which cost
    /// cache-resident 12-row vertical filters 19-48% on Zen 4
    /// (`benchmarks/stride_aliasing_2026-11-01.md` in the zenpixels repo). The
    /// allocation is fallible, and clones keep the alignment.
    ///
    /// # Errors
    ///
    /// [`PlaneError::GeometryOverflow`] when the size overflows,
    /// [`PlaneError::AllocationFailed`] when the allocator refuses.
    pub fn alloc(width: usize, height: usize) -> Result<Self, PlaneError> {
        let size = core::mem::size_of::<T>();
        let row_bytes = width
            .checked_mul(size)
            .and_then(|b| b.checked_next_multiple_of(ROW_ALIGN))
            .ok_or(PlaneError::GeometryOverflow)?;
        let stride_bytes = if row_bytes != 0 && row_bytes % 4096 == 0 {
            row_bytes + ROW_ALIGN
        } else {
            row_bytes
        };
        let stride = stride_bytes / size;
        let len = stride
            .checked_mul(height)
            .ok_or(PlaneError::GeometryOverflow)?;
        Self::with_start(AlignedSamples::try_zeroed(len)?, 0, width, height, stride)
    }
}

/// A borrowed plane of any sample type, for interfaces that carry planes of
/// different sample types side by side.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum AnyPlane<'a> {
    /// 8-bit samples.
    U8(PlaneRef<'a, u8>),
    /// 16-bit samples.
    U16(PlaneRef<'a, u16>),
    /// 32-bit float samples.
    F32(PlaneRef<'a, f32>),
}

impl AnyPlane<'_> {
    /// Samples per row.
    pub fn width(&self) -> usize {
        match self {
            Self::U8(p) => p.width(),
            Self::U16(p) => p.width(),
            Self::F32(p) => p.width(),
        }
    }

    /// Rows.
    pub fn height(&self) -> usize {
        match self {
            Self::U8(p) => p.height(),
            Self::U16(p) => p.height(),
            Self::F32(p) => p.height(),
        }
    }

    /// The sample type.
    pub fn channel_type(&self) -> ChannelType {
        match self {
            Self::U8(_) => ChannelType::U8,
            Self::U16(_) => ChannelType::U16,
            Self::F32(_) => ChannelType::F32,
        }
    }
}

/// What a plane holds.
///
/// A gray or monochrome picture is a `Luma` plane. Semi-planar chroma (NV12,
/// P010, NV21) is one plane holding two components per position, in the order
/// the variant names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlaneRole {
    /// Y, or gray.
    Luma,
    /// Cb (U).
    Cb,
    /// Cr (V).
    Cr,
    /// Cb and Cr interleaved, Cb first (NV12, P010).
    CbCr,
    /// Cr and Cb interleaved, Cr first (NV21).
    CrCb,
    /// R.
    Red,
    /// G.
    Green,
    /// B.
    Blue,
    /// Alpha.
    Alpha,
}

impl PlaneRole {
    /// Samples stored per position: 2 for interleaved chroma, else 1. A plane
    /// holding this role is `positions * components()` samples wide.
    pub const fn components(self) -> usize {
        match self {
            Self::CbCr | Self::CrCb => 2,
            _ => 1,
        }
    }

    const fn is_chroma(self) -> bool {
        matches!(self, Self::Cb | Self::Cr | Self::CbCr | Self::CrCb)
    }

    const fn is_rgb(self) -> bool {
        matches!(self, Self::Red | Self::Green | Self::Blue)
    }
}

/// A float encoding is not finite, or its scale is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FloatEncodingError {
    /// Scale or offset is NaN or infinite.
    NotFinite,
    /// Scale is zero.
    ZeroScale,
}

impl fmt::Display for FloatEncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFinite => "float plane scale and offset must be finite",
            Self::ZeroScale => "float plane scale must not be zero",
        })
    }
}
impl core::error::Error for FloatEncodingError {}

/// How float samples relate to nominal component values.
///
/// `stored = nominal * scale + offset`. Nominal luma, red, green, blue and
/// alpha run from 0 to 1, and nominal Cb and Cr from -0.5 to 0.5, all full
/// range; stored values may lie outside. For example, zenjpeg's centered
/// planes are `scale = 255` with `offset = -128` for Y and `offset = 0` for
/// Cb and Cr. A decoder that keeps limited-range levels in floats states them
/// here too, so a float plane never needs a separate range.
#[derive(Clone, Copy, Debug)]
pub struct FloatEncoding {
    scale: f32,
    offset: f32,
}

impl FloatEncoding {
    /// Stored values are the nominal values.
    pub const NOMINAL: Self = Self {
        scale: 1.0,
        offset: 0.0,
    };

    /// Stored values are `nominal * scale + offset`.
    ///
    /// # Errors
    ///
    /// [`FloatEncodingError::NotFinite`] for a NaN or infinite value,
    /// [`FloatEncodingError::ZeroScale`] for a zero scale.
    pub fn new(scale: f32, offset: f32) -> Result<Self, FloatEncodingError> {
        if !scale.is_finite() || !offset.is_finite() {
            return Err(FloatEncodingError::NotFinite);
        }
        if scale == 0.0 {
            return Err(FloatEncodingError::ZeroScale);
        }
        // One representation of zero, so equal encodings compare equal.
        let offset = if offset == 0.0 { 0.0 } else { offset };
        Ok(Self { scale, offset })
    }

    /// Stored units per nominal unit.
    pub const fn scale(self) -> f32 {
        self.scale
    }

    /// The stored value of nominal zero.
    pub const fn offset(self) -> f32 {
        self.offset
    }
}

impl PartialEq for FloatEncoding {
    fn eq(&self, other: &Self) -> bool {
        self.scale.to_bits() == other.scale.to_bits()
            && self.offset.to_bits() == other.offset.to_bits()
    }
}
impl Eq for FloatEncoding {}
impl core::hash::Hash for FloatEncoding {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.scale.to_bits().hash(state);
        self.offset.to_bits().hash(state);
    }
}

/// How a plane's samples encode component values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlaneEncoding {
    /// Integer codes in `u8` or `u16` samples; range comes from the enclosing
    /// signal.
    Code(SampleEncoding),
    /// Floating-point samples (`f32`); the encoding describes values, not the
    /// sample width.
    Float(FloatEncoding),
}

impl PlaneEncoding {
    /// The sample type this encoding is stored in.
    pub const fn channel_type(self) -> ChannelType {
        match self {
            Self::Code(e) => e.storage(),
            Self::Float(_) => ChannelType::F32,
        }
    }
}

/// One plane of a [`PlanarLayout`]: what it holds and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlaneSpec {
    role: PlaneRole,
    encoding: PlaneEncoding,
}

impl PlaneSpec {
    /// A plane holding `role`, encoded as `encoding`.
    pub const fn new(role: PlaneRole, encoding: PlaneEncoding) -> Self {
        Self { role, encoding }
    }

    /// What the plane holds.
    pub const fn role(self) -> PlaneRole {
        self.role
    }

    /// How its samples encode values.
    pub const fn encoding(self) -> PlaneEncoding {
        self.encoding
    }
}

/// A layout is inconsistent, or planes do not match it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutError {
    /// The layout has no planes.
    Empty,
    /// A component appears in more than one plane.
    DuplicateRole(PlaneRole),
    /// YCbCr and RGB components are mixed.
    MixedFamilies,
    /// A component the others require is missing (Luma with chroma, or one
    /// of R, G, B).
    MissingRole(PlaneRole),
    /// Subsampling disagrees with the planes: chroma planes need a
    /// subsampling other than `Mono`, a luma-only layout needs `Mono`, and
    /// planar RGB needs `Yuv444`.
    Subsampling,
    /// The number of planes differs from the layout.
    PlaneCount {
        /// Planes in the layout.
        expected: usize,
        /// Planes supplied.
        actual: usize,
    },
    /// A plane's sample type differs from its encoding.
    SampleType {
        /// Plane index.
        index: usize,
    },
    /// A plane's size differs from what the layout gives for the picture.
    Dimensions {
        /// Plane index.
        index: usize,
        /// Samples per row and rows the layout requires.
        expected: (usize, usize),
        /// Samples per row and rows supplied.
        actual: (usize, usize),
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("planar layout has no planes"),
            Self::DuplicateRole(r) => write!(f, "planar layout holds {r:?} twice"),
            Self::MixedFamilies => f.write_str("planar layout mixes YCbCr and RGB components"),
            Self::MissingRole(r) => write!(f, "planar layout lacks {r:?}"),
            Self::Subsampling => f.write_str("chroma subsampling disagrees with the planes"),
            Self::PlaneCount { expected, actual } => {
                write!(f, "layout has {expected} planes, {actual} supplied")
            }
            Self::SampleType { index } => {
                write!(f, "plane {index} sample type differs from its encoding")
            }
            Self::Dimensions {
                index,
                expected,
                actual,
            } => write!(
                f,
                "plane {index} is {}x{} samples, layout requires {}x{}",
                actual.0, actual.1, expected.0, expected.1
            ),
        }
    }
}
impl core::error::Error for LayoutError {}

/// The planes of a planar picture, with chroma subsampling and siting.
///
/// Plane order is the order of [`PlanarLayout::planes`]; it is not implied by
/// the roles. Color signaling travels with the enclosing descriptor.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlanarLayout {
    planes: Vec<PlaneSpec>,
    subsampling: ChromaSubsampling,
    siting: ChromaSiting,
}

impl PlanarLayout {
    /// Check and adopt a plane list.
    ///
    /// Each component appears once. YCbCr layouts hold Luma, optionally both
    /// Cb and Cr (separately or in one interleaved plane), and optionally
    /// Alpha; RGB layouts hold Red, Green, Blue and optionally Alpha; an alpha-only
    /// layout holds Alpha alone, with `Mono` subsampling.
    ///
    /// # Errors
    ///
    /// A [`LayoutError`] naming the first inconsistency.
    pub fn new(
        planes: Vec<PlaneSpec>,
        subsampling: ChromaSubsampling,
        siting: ChromaSiting,
    ) -> Result<Self, LayoutError> {
        if planes.is_empty() {
            return Err(LayoutError::Empty);
        }
        let has = |r: PlaneRole| planes.iter().any(|p| p.role == r);
        let (mut cb, mut cr) = (false, false);
        for (i, p) in planes.iter().enumerate() {
            if planes[..i].iter().any(|q| q.role == p.role) {
                return Err(LayoutError::DuplicateRole(p.role));
            }
            let (holds_cb, holds_cr) = match p.role {
                PlaneRole::Cb => (true, false),
                PlaneRole::Cr => (false, true),
                PlaneRole::CbCr | PlaneRole::CrCb => (true, true),
                _ => (false, false),
            };
            if (holds_cb && cb) || (holds_cr && cr) {
                return Err(LayoutError::DuplicateRole(p.role));
            }
            cb |= holds_cb;
            cr |= holds_cr;
        }
        let rgb = planes.iter().any(|p| p.role.is_rgb());
        let ycbcr = has(PlaneRole::Luma) || cb || cr;
        if rgb && ycbcr {
            return Err(LayoutError::MixedFamilies);
        }
        if rgb {
            for r in [PlaneRole::Red, PlaneRole::Green, PlaneRole::Blue] {
                if !has(r) {
                    return Err(LayoutError::MissingRole(r));
                }
            }
            if subsampling != ChromaSubsampling::Yuv444 {
                return Err(LayoutError::Subsampling);
            }
        } else {
            // A lone alpha plane is a valid image (`image`'s `A8`); anything else needs luma.
            let alpha_only = planes.len() == 1 && has(PlaneRole::Alpha);
            if !has(PlaneRole::Luma) && !alpha_only {
                return Err(LayoutError::MissingRole(PlaneRole::Luma));
            }
            if cb != cr {
                let missing = if cb { PlaneRole::Cr } else { PlaneRole::Cb };
                return Err(LayoutError::MissingRole(missing));
            }
            if cb == (subsampling == ChromaSubsampling::Mono) {
                return Err(LayoutError::Subsampling);
            }
        }
        Ok(Self {
            planes,
            subsampling,
            siting,
        })
    }

    /// The planes, in storage order.
    pub fn planes(&self) -> &[PlaneSpec] {
        &self.planes
    }

    /// Chroma subsampling; `Mono` without chroma, `Yuv444` for planar RGB.
    pub fn subsampling(&self) -> ChromaSubsampling {
        self.subsampling
    }

    /// Chroma siting.
    pub fn siting(&self) -> ChromaSiting {
        self.siting
    }

    /// Positions (not samples) per row and rows of plane `index` in a
    /// `width` x `height` picture; `None` for an index past the last plane.
    /// Chroma planes are rounded up; multiply the width by
    /// [`PlaneRole::components`] for samples per row.
    pub fn plane_size(&self, index: usize, width: u32, height: u32) -> Option<(u32, u32)> {
        let spec = self.planes.get(index)?;
        if spec.role.is_chroma() {
            self.subsampling.chroma_size(width, height)
        } else {
            Some((width, height))
        }
    }

    /// Check that `planes` hold this layout for a `width` x `height` picture:
    /// one plane per spec, in order, each with its encoding's sample type and
    /// exactly the layout's samples per row and rows.
    ///
    /// # Errors
    ///
    /// A [`LayoutError`] naming the first plane that does not match.
    pub fn check(
        &self,
        planes: &[AnyPlane<'_>],
        width: u32,
        height: u32,
    ) -> Result<(), LayoutError> {
        if planes.len() != self.planes.len() {
            return Err(LayoutError::PlaneCount {
                expected: self.planes.len(),
                actual: planes.len(),
            });
        }
        for (index, (plane, spec)) in planes.iter().zip(&self.planes).enumerate() {
            if plane.channel_type() != spec.encoding.channel_type() {
                return Err(LayoutError::SampleType { index });
            }
            let (w, h) = self
                .plane_size(index, width, height)
                .expect("index within the layout");
            let expected = ((w as usize) * spec.role.components(), h as usize);
            let actual = (plane.width(), plane.height());
            if actual != expected {
                return Err(LayoutError::Dimensions {
                    index,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }
}
