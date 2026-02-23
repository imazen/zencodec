//! Opaque pixel buffer abstraction.
//!
//! Provides format-aware pixel storage that carries its own metadata,
//! eliminating the need to match on 13 [`PixelData`](crate::PixelData) variants.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use imgref::ImgRef;
use rgb::alt::BGRA;
use rgb::{Gray, Rgb, Rgba};

use crate::color::{ColorContext, WorkingColorSpace};
use crate::pixel::{GrayAlpha, PixelData};

// ---------------------------------------------------------------------------
// Descriptor enums
// ---------------------------------------------------------------------------

/// Channel storage type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum ChannelType {
    /// 8-bit unsigned integer (1 byte per channel).
    U8 = 1,
    /// 16-bit unsigned integer (2 bytes per channel).
    U16 = 2,
    /// 32-bit floating point (4 bytes per channel).
    F32 = 4,
}

impl ChannelType {
    /// Byte size of a single channel value.
    #[inline]
    pub const fn byte_size(self) -> usize {
        self as usize
    }
}

/// Channel layout (number and meaning of channels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum ChannelLayout {
    /// Single luminance channel.
    Gray = 1,
    /// Luminance + alpha.
    GrayAlpha = 2,
    /// Red, green, blue.
    Rgb = 3,
    /// Red, green, blue, alpha.
    Rgba = 4,
    /// Blue, green, red, alpha (Windows/DirectX byte order).
    Bgra = 5,
}

impl ChannelLayout {
    /// Number of channels in this layout.
    #[inline]
    pub const fn channels(self) -> usize {
        match self {
            Self::Gray => 1,
            Self::GrayAlpha => 2,
            Self::Rgb => 3,
            Self::Rgba | Self::Bgra => 4,
        }
    }

    /// Whether this layout includes an alpha channel.
    #[inline]
    pub const fn has_alpha(self) -> bool {
        matches!(self, Self::GrayAlpha | Self::Rgba | Self::Bgra)
    }
}

/// Alpha channel interpretation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum AlphaMode {
    /// No alpha channel.
    None = 0,
    /// Straight (unassociated) alpha.
    Straight = 1,
    /// Premultiplied (associated) alpha.
    Premultiplied = 2,
}

/// Electro-optical transfer function.
///
/// When a pixel buffer's transfer function is not known (e.g. raw decoded data
/// without CICP metadata), use [`Unknown`](Self::Unknown). Consumers that need
/// color-correct operations (resize, blend, blur) must check for `Unknown` and
/// resolve it from [`ImageInfo::cicp`](crate::ImageInfo) or an ICC profile
/// before processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum TransferFunction {
    /// Linear light (gamma 1.0).
    Linear = 0,
    /// sRGB transfer curve (IEC 61966-2-1).
    Srgb = 1,
    /// BT.709 transfer curve.
    Bt709 = 2,
    /// Perceptual Quantizer (SMPTE ST 2084, HDR10).
    Pq = 3,
    /// Hybrid Log-Gamma (ARIB STD-B67, HLG).
    Hlg = 4,
    /// Transfer function is not known.
    ///
    /// This is the default for pixel data where the source transfer function
    /// has not been established. Check CICP metadata or the ICC profile to
    /// determine the actual transfer function before performing color-sensitive
    /// operations.
    Unknown = 255,
}

impl TransferFunction {
    /// Map CICP `transfer_characteristics` code to a [`TransferFunction`].
    ///
    /// Returns `None` for unrecognized or unsupported codes.
    pub const fn from_cicp(tc: u8) -> Option<Self> {
        match tc {
            1 => Some(Self::Bt709),
            8 => Some(Self::Linear),
            13 => Some(Self::Srgb),
            16 => Some(Self::Pq),
            18 => Some(Self::Hlg),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// PixelDescriptor
// ---------------------------------------------------------------------------

/// Compact pixel format descriptor (4 bytes).
///
/// Describes the format of pixel data without carrying the data itself.
/// Used to tag [`PixelBuffer`] and [`PixelSlice`] with their format.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub struct PixelDescriptor {
    /// Channel storage type (u8, u16, f32).
    pub channel_type: ChannelType,
    /// Channel layout (gray, RGB, RGBA, etc.).
    pub layout: ChannelLayout,
    /// Alpha interpretation.
    pub alpha: AlphaMode,
    /// Transfer function (sRGB, linear, PQ, etc.).
    pub transfer: TransferFunction,
}

impl PixelDescriptor {
    /// Create a pixel format descriptor.
    pub const fn new(
        channel_type: ChannelType,
        layout: ChannelLayout,
        alpha: AlphaMode,
        transfer: TransferFunction,
    ) -> Self {
        Self {
            channel_type,
            layout,
            alpha,
            transfer,
        }
    }

    // Named constants ---------------------------------------------------------

    /// 8-bit sRGB RGB.
    pub const RGB8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Rgb,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Srgb,
    };

    /// 8-bit sRGB RGBA with straight alpha.
    pub const RGBA8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Rgba,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Srgb,
    };

    /// 16-bit sRGB RGB.
    pub const RGB16_SRGB: Self = Self {
        channel_type: ChannelType::U16,
        layout: ChannelLayout::Rgb,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Srgb,
    };

    /// 16-bit sRGB RGBA with straight alpha.
    pub const RGBA16_SRGB: Self = Self {
        channel_type: ChannelType::U16,
        layout: ChannelLayout::Rgba,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Srgb,
    };

    /// Linear-light f32 RGB.
    pub const RGBF32_LINEAR: Self = Self {
        channel_type: ChannelType::F32,
        layout: ChannelLayout::Rgb,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Linear,
    };

    /// Linear-light f32 RGBA with straight alpha.
    pub const RGBAF32_LINEAR: Self = Self {
        channel_type: ChannelType::F32,
        layout: ChannelLayout::Rgba,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Linear,
    };

    /// 8-bit sRGB grayscale.
    pub const GRAY8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Gray,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Srgb,
    };

    /// 16-bit sRGB grayscale.
    pub const GRAY16_SRGB: Self = Self {
        channel_type: ChannelType::U16,
        layout: ChannelLayout::Gray,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Srgb,
    };

    /// Linear-light f32 grayscale.
    pub const GRAYF32_LINEAR: Self = Self {
        channel_type: ChannelType::F32,
        layout: ChannelLayout::Gray,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Linear,
    };

    /// 8-bit sRGB grayscale with straight alpha.
    pub const GRAYA8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::GrayAlpha,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Srgb,
    };

    /// 16-bit sRGB grayscale with straight alpha.
    pub const GRAYA16_SRGB: Self = Self {
        channel_type: ChannelType::U16,
        layout: ChannelLayout::GrayAlpha,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Srgb,
    };

    /// Linear-light f32 grayscale with straight alpha.
    pub const GRAYAF32_LINEAR: Self = Self {
        channel_type: ChannelType::F32,
        layout: ChannelLayout::GrayAlpha,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Linear,
    };

    /// 8-bit sRGB BGRA with straight alpha.
    pub const BGRA8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Bgra,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Srgb,
    };

    /// 8-bit sRGB BGRX (opaque BGRA, padding byte ignored).
    ///
    /// Same memory layout as BGRA8 but the fourth byte is padding
    /// (`AlphaMode::None`). Useful for Windows surfaces and DirectX
    /// where the alpha byte is present but meaningless.
    pub const BGRX8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Bgra,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Srgb,
    };

    // Transfer-agnostic constants -----------------------------------------------
    //
    // Same channel type and layout as the explicitly-tagged constants above, but
    // with `TransferFunction::Unknown`. Use these when the transfer function is
    // not yet known (e.g. raw decoded data before CICP is consulted).

    /// 8-bit RGB, transfer function unknown.
    pub const RGB8: Self = Self::RGB8_SRGB.with_transfer(TransferFunction::Unknown);

    /// 8-bit RGBA with straight alpha, transfer function unknown.
    pub const RGBA8: Self = Self::RGBA8_SRGB.with_transfer(TransferFunction::Unknown);

    /// 16-bit RGB, transfer function unknown.
    pub const RGB16: Self = Self::RGB16_SRGB.with_transfer(TransferFunction::Unknown);

    /// 16-bit RGBA with straight alpha, transfer function unknown.
    pub const RGBA16: Self = Self::RGBA16_SRGB.with_transfer(TransferFunction::Unknown);

    /// f32 RGB, transfer function unknown.
    pub const RGBF32: Self = Self::RGBF32_LINEAR.with_transfer(TransferFunction::Unknown);

    /// f32 RGBA with straight alpha, transfer function unknown.
    pub const RGBAF32: Self = Self::RGBAF32_LINEAR.with_transfer(TransferFunction::Unknown);

    /// 8-bit grayscale, transfer function unknown.
    pub const GRAY8: Self = Self::GRAY8_SRGB.with_transfer(TransferFunction::Unknown);

    /// 16-bit grayscale, transfer function unknown.
    pub const GRAY16: Self = Self::GRAY16_SRGB.with_transfer(TransferFunction::Unknown);

    /// f32 grayscale, transfer function unknown.
    pub const GRAYF32: Self = Self::GRAYF32_LINEAR.with_transfer(TransferFunction::Unknown);

    /// 8-bit grayscale with straight alpha, transfer function unknown.
    pub const GRAYA8: Self = Self::GRAYA8_SRGB.with_transfer(TransferFunction::Unknown);

    /// 16-bit grayscale with straight alpha, transfer function unknown.
    pub const GRAYA16: Self = Self::GRAYA16_SRGB.with_transfer(TransferFunction::Unknown);

    /// f32 grayscale with straight alpha, transfer function unknown.
    pub const GRAYAF32: Self = Self::GRAYAF32_LINEAR.with_transfer(TransferFunction::Unknown);

    /// 8-bit BGRA with straight alpha, transfer function unknown.
    pub const BGRA8: Self = Self::BGRA8_SRGB.with_transfer(TransferFunction::Unknown);

    /// 8-bit BGRX (opaque BGRA, padding byte ignored), transfer function unknown.
    pub const BGRX8: Self = Self::BGRX8_SRGB.with_transfer(TransferFunction::Unknown);

    // Methods -----------------------------------------------------------------

    /// Return a copy of this descriptor with a different transfer function.
    ///
    /// Useful for resolving `Unknown` once CICP/ICC metadata is available:
    ///
    /// ```
    /// # use zencodec_types::{PixelDescriptor, TransferFunction};
    /// let desc = PixelDescriptor::RGB8; // Unknown transfer
    /// let resolved = desc.with_transfer(TransferFunction::Srgb);
    /// assert_eq!(resolved, PixelDescriptor::RGB8_SRGB);
    /// ```
    #[inline]
    pub const fn with_transfer(self, transfer: TransferFunction) -> Self {
        Self {
            channel_type: self.channel_type,
            layout: self.layout,
            alpha: self.alpha,
            transfer,
        }
    }

    /// Check if this descriptor matches the layout and type of another,
    /// ignoring transfer function and alpha mode.
    ///
    /// Useful for format negotiation: two descriptors are layout-compatible
    /// if they have the same channel count, order, and storage type, even
    /// if they differ in gamma or alpha interpretation.
    #[inline]
    pub const fn layout_compatible(&self, other: &PixelDescriptor) -> bool {
        self.channel_type as u8 == other.channel_type as u8
            && self.layout as u8 == other.layout as u8
    }

    /// Minimum byte alignment required for the channel type (1, 2, or 4).
    #[inline]
    pub const fn min_alignment(self) -> usize {
        self.channel_type.byte_size()
    }

    /// Bytes per pixel.
    #[inline]
    pub const fn bytes_per_pixel(self) -> usize {
        self.channel_type.byte_size() * self.layout.channels()
    }

    /// Number of channels.
    #[inline]
    pub const fn channels(self) -> u8 {
        self.layout.channels() as u8
    }

    /// Whether this format carries meaningful alpha data.
    ///
    /// Returns `false` for formats like BGRX where the layout has an
    /// alpha-position channel but `AlphaMode::None` indicates it's padding.
    /// Use [`ChannelLayout::has_alpha()`] to check if the layout includes
    /// an alpha-position channel regardless of whether it carries data.
    #[inline]
    pub const fn has_alpha(self) -> bool {
        !matches!(self.alpha, AlphaMode::None)
    }

    /// Whether the transfer function is [`Linear`](TransferFunction::Linear).
    ///
    /// Returns `false` for [`Unknown`](TransferFunction::Unknown) — callers
    /// must resolve the transfer function before assuming linearity.
    #[inline]
    pub const fn is_linear(self) -> bool {
        matches!(self.transfer, TransferFunction::Linear)
    }

    /// Whether the transfer function is [`Unknown`](TransferFunction::Unknown).
    ///
    /// When true, the caller must consult CICP/ICC metadata to determine
    /// the actual transfer function before performing color-sensitive
    /// operations. Use [`with_transfer()`](Self::with_transfer) to set
    /// the correct value once known.
    #[inline]
    pub const fn is_unknown_transfer(self) -> bool {
        matches!(self.transfer, TransferFunction::Unknown)
    }

    /// Compute the tightly-packed byte stride for a given width.
    ///
    /// The returned stride equals `width * bytes_per_pixel()` and is
    /// guaranteed to be a multiple of `bytes_per_pixel()`.
    #[inline]
    pub const fn aligned_stride(self, width: u32) -> usize {
        width as usize * self.bytes_per_pixel()
    }

    /// Compute a SIMD-friendly byte stride for a given width.
    ///
    /// The stride is a multiple of `lcm(bytes_per_pixel, simd_align)`,
    /// ensuring every row start is both pixel-aligned and SIMD-aligned.
    ///
    /// `simd_align` must be a power of 2 (e.g. 16, 32, 64).
    #[inline]
    pub const fn simd_aligned_stride(self, width: u32, simd_align: usize) -> usize {
        let bpp = self.bytes_per_pixel();
        let raw = width as usize * bpp;
        let align = lcm(bpp, simd_align);
        align_up_general(raw, align)
    }
}

// ---------------------------------------------------------------------------
// BufferError
// ---------------------------------------------------------------------------

/// Errors from pixel buffer operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BufferError {
    /// Data pointer is not aligned for the channel type.
    AlignmentViolation,
    /// Data slice is too small for the given dimensions and stride.
    InsufficientData,
    /// Stride is smaller than `width * bytes_per_pixel`.
    StrideTooSmall,
    /// Stride is not a multiple of `bytes_per_pixel`.
    ///
    /// Every row must start on a pixel boundary. If stride is not a
    /// multiple of bpp, rows after the first will be misaligned.
    StrideNotPixelAligned,
    /// Width or height is zero or causes overflow.
    InvalidDimensions,
    /// Descriptor does not match any [`PixelData`] variant.
    FormatMismatch,
}

impl fmt::Display for BufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlignmentViolation => write!(f, "data is not aligned for the channel type"),
            Self::InsufficientData => {
                write!(f, "data slice is too small for the given dimensions")
            }
            Self::StrideTooSmall => write!(f, "stride is smaller than width * bytes_per_pixel"),
            Self::StrideNotPixelAligned => {
                write!(f, "stride is not a multiple of bytes_per_pixel")
            }
            Self::InvalidDimensions => write!(f, "width or height is zero or causes overflow"),
            Self::FormatMismatch => write!(f, "pixel format has no matching PixelData variant"),
        }
    }
}

impl core::error::Error for BufferError {}

// ---------------------------------------------------------------------------
// PixelSlice (borrowed, immutable)
// ---------------------------------------------------------------------------

/// Borrowed view of pixel data.
///
/// Represents a contiguous region of pixel rows, possibly a sub-region
/// of a larger buffer. All rows share the same stride.
///
/// Optionally carries [`ColorContext`] and [`WorkingColorSpace`] to
/// track source color metadata and current color space through the
/// processing pipeline.
#[non_exhaustive]
pub struct PixelSlice<'a> {
    data: &'a [u8],
    width: u32,
    rows: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    working_space: WorkingColorSpace,
    color: Option<Arc<ColorContext>>,
}

impl<'a> PixelSlice<'a> {
    /// Create a new pixel slice with validation.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is too small, the stride is too small,
    /// or the data is not aligned for the channel type.
    pub fn new(
        data: &'a [u8],
        width: u32,
        rows: u32,
        stride: usize,
        descriptor: PixelDescriptor,
    ) -> Result<Self, BufferError> {
        let bpp = descriptor.bytes_per_pixel();
        let min_stride = (width as usize)
            .checked_mul(bpp)
            .ok_or(BufferError::InvalidDimensions)?;
        if stride < min_stride {
            return Err(BufferError::StrideTooSmall);
        }
        if bpp > 0 && !stride.is_multiple_of(bpp) {
            return Err(BufferError::StrideNotPixelAligned);
        }
        if rows > 0 {
            let required = required_bytes(rows, stride, min_stride)?;
            if data.len() < required {
                return Err(BufferError::InsufficientData);
            }
        }
        let align = descriptor.min_alignment();
        if !(data.as_ptr() as usize).is_multiple_of(align) {
            return Err(BufferError::AlignmentViolation);
        }
        Ok(Self {
            data,
            width,
            rows,
            stride,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
        })
    }

    /// Replace the descriptor, preserving all other fields.
    ///
    /// Used by `PixelData::as_pixel_slice()` to keep the transfer-agnostic
    /// descriptor from decoded data instead of the convention-based one
    /// from the `From<ImgRef>` impl.
    #[inline]
    pub(crate) fn with_descriptor(mut self, descriptor: PixelDescriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    /// Image width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Number of rows in this slice.
    #[inline]
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Byte stride between row starts.
    #[inline]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Pixel format descriptor.
    #[inline]
    pub fn descriptor(&self) -> PixelDescriptor {
        self.descriptor
    }

    /// Source color context (ICC/CICP metadata), if set.
    #[inline]
    pub fn color_context(&self) -> Option<&Arc<ColorContext>> {
        self.color.as_ref()
    }

    /// Return a copy of this slice with a color context attached.
    #[inline]
    pub fn with_color_context(mut self, ctx: Arc<ColorContext>) -> Self {
        self.color = Some(ctx);
        self
    }

    /// Current working color space.
    #[inline]
    pub fn working_space(&self) -> WorkingColorSpace {
        self.working_space
    }

    /// Return a copy of this slice with a different working color space.
    #[inline]
    pub fn with_working_space(mut self, ws: WorkingColorSpace) -> Self {
        self.working_space = ws;
        self
    }

    /// Whether rows are tightly packed (no stride padding).
    ///
    /// When true, the entire pixel data is contiguous in memory and
    /// [`as_contiguous_bytes()`](Self::as_contiguous_bytes) returns `Some`.
    #[inline]
    pub fn is_contiguous(&self) -> bool {
        self.stride == self.width as usize * self.descriptor.bytes_per_pixel()
    }

    /// Zero-copy access to the raw pixel bytes when rows are tightly packed.
    ///
    /// Returns `Some(&[u8])` if `stride == width * bpp` (no padding),
    /// `None` if rows have stride padding.
    ///
    /// Use this to avoid `collect_contiguous_bytes()` copies when passing
    /// pixel data to FFI or other APIs that need a flat buffer.
    #[inline]
    pub fn as_contiguous_bytes(&self) -> Option<&'a [u8]> {
        if self.is_contiguous() {
            let total = self.rows as usize * self.stride;
            Some(&self.data[..total])
        } else {
            None
        }
    }

    /// Get the raw pixel bytes, copying only if stride padding exists.
    ///
    /// Returns `Cow::Borrowed` when rows are contiguous (zero-copy),
    /// `Cow::Owned` when stride padding must be stripped.
    pub fn contiguous_bytes(&self) -> alloc::borrow::Cow<'a, [u8]> {
        if let Some(bytes) = self.as_contiguous_bytes() {
            alloc::borrow::Cow::Borrowed(bytes)
        } else {
            let bpp = self.descriptor.bytes_per_pixel();
            let row_bytes = self.width as usize * bpp;
            let mut buf = Vec::with_capacity(row_bytes * self.rows as usize);
            for y in 0..self.rows {
                buf.extend_from_slice(self.row(y));
            }
            alloc::borrow::Cow::Owned(buf)
        }
    }

    /// Pixel bytes for row `y` (no padding, exactly `width * bpp` bytes).
    ///
    /// # Panics
    ///
    /// Panics if `y >= rows`.
    #[inline]
    pub fn row(&self, y: u32) -> &[u8] {
        assert!(
            y < self.rows,
            "row index {y} out of bounds (rows: {})",
            self.rows
        );
        let start = y as usize * self.stride;
        let len = self.width as usize * self.descriptor.bytes_per_pixel();
        &self.data[start..start + len]
    }

    /// Full stride bytes for row `y` (including any padding).
    ///
    /// # Panics
    ///
    /// Panics if `y >= rows` or if the underlying data does not contain
    /// a full stride for this row (can happen on the last row of a
    /// cropped view).
    #[inline]
    pub fn row_with_stride(&self, y: u32) -> &[u8] {
        assert!(
            y < self.rows,
            "row index {y} out of bounds (rows: {})",
            self.rows
        );
        let start = y as usize * self.stride;
        &self.data[start..start + self.stride]
    }

    /// Borrow a sub-range of rows.
    ///
    /// # Panics
    ///
    /// Panics if `y + count > rows`.
    pub fn sub_rows(&self, y: u32, count: u32) -> PixelSlice<'_> {
        assert!(
            y.checked_add(count).is_some_and(|end| end <= self.rows),
            "sub_rows({y}, {count}) out of bounds (rows: {})",
            self.rows
        );
        if count == 0 {
            return PixelSlice {
                data: &[],
                width: self.width,
                rows: 0,
                stride: self.stride,
                descriptor: self.descriptor,
                working_space: self.working_space,
                color: self.color.clone(),
            };
        }
        let bpp = self.descriptor.bytes_per_pixel();
        let start = y as usize * self.stride;
        let end = (y as usize + count as usize - 1) * self.stride + self.width as usize * bpp;
        PixelSlice {
            data: &self.data[start..end],
            width: self.width,
            rows: count,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
        }
    }

    /// Zero-copy crop view. Adjusts the data pointer and width; stride
    /// remains the same as the parent.
    ///
    /// # Panics
    ///
    /// Panics if the crop region is out of bounds.
    pub fn crop_view(&self, x: u32, y: u32, w: u32, h: u32) -> PixelSlice<'_> {
        assert!(
            x.checked_add(w).is_some_and(|end| end <= self.width),
            "crop x={x} w={w} exceeds width {}",
            self.width
        );
        assert!(
            y.checked_add(h).is_some_and(|end| end <= self.rows),
            "crop y={y} h={h} exceeds rows {}",
            self.rows
        );
        if h == 0 || w == 0 {
            return PixelSlice {
                data: &[],
                width: w,
                rows: h,
                stride: self.stride,
                descriptor: self.descriptor,
                working_space: self.working_space,
                color: self.color.clone(),
            };
        }
        let bpp = self.descriptor.bytes_per_pixel();
        let start = y as usize * self.stride + x as usize * bpp;
        let end = (y as usize + h as usize - 1) * self.stride + (x as usize + w as usize) * bpp;
        PixelSlice {
            data: &self.data[start..end],
            width: w,
            rows: h,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
        }
    }
}

impl fmt::Debug for PixelSlice<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PixelSlice({}x{}, {:?} {:?})",
            self.width, self.rows, self.descriptor.layout, self.descriptor.channel_type
        )
    }
}

// ---------------------------------------------------------------------------
// PixelSliceMut (borrowed, mutable)
// ---------------------------------------------------------------------------

/// Mutable borrowed view of pixel data.
///
/// Same semantics as [`PixelSlice`] but allows writing to rows.
#[non_exhaustive]
pub struct PixelSliceMut<'a> {
    data: &'a mut [u8],
    width: u32,
    rows: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    working_space: WorkingColorSpace,
    color: Option<Arc<ColorContext>>,
}

impl<'a> PixelSliceMut<'a> {
    /// Create a new mutable pixel slice with validation.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is too small, the stride is too small,
    /// or the data is not aligned for the channel type.
    pub fn new(
        data: &'a mut [u8],
        width: u32,
        rows: u32,
        stride: usize,
        descriptor: PixelDescriptor,
    ) -> Result<Self, BufferError> {
        let bpp = descriptor.bytes_per_pixel();
        let min_stride = (width as usize)
            .checked_mul(bpp)
            .ok_or(BufferError::InvalidDimensions)?;
        if stride < min_stride {
            return Err(BufferError::StrideTooSmall);
        }
        if bpp > 0 && !stride.is_multiple_of(bpp) {
            return Err(BufferError::StrideNotPixelAligned);
        }
        if rows > 0 {
            let required = required_bytes(rows, stride, min_stride)?;
            if data.len() < required {
                return Err(BufferError::InsufficientData);
            }
        }
        let align = descriptor.min_alignment();
        if !(data.as_ptr() as usize).is_multiple_of(align) {
            return Err(BufferError::AlignmentViolation);
        }
        Ok(Self {
            data,
            width,
            rows,
            stride,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
        })
    }

    /// Image width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Number of rows in this slice.
    #[inline]
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Byte stride between row starts.
    #[inline]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Pixel format descriptor.
    #[inline]
    pub fn descriptor(&self) -> PixelDescriptor {
        self.descriptor
    }

    /// Source color context (ICC/CICP metadata), if set.
    #[inline]
    pub fn color_context(&self) -> Option<&Arc<ColorContext>> {
        self.color.as_ref()
    }

    /// Return a copy of this slice with a color context attached.
    #[inline]
    pub fn with_color_context(mut self, ctx: Arc<ColorContext>) -> Self {
        self.color = Some(ctx);
        self
    }

    /// Current working color space.
    #[inline]
    pub fn working_space(&self) -> WorkingColorSpace {
        self.working_space
    }

    /// Return a copy of this slice with a different working color space.
    #[inline]
    pub fn with_working_space(mut self, ws: WorkingColorSpace) -> Self {
        self.working_space = ws;
        self
    }

    /// Pixel bytes for row `y` (immutable, no padding).
    ///
    /// # Panics
    ///
    /// Panics if `y >= rows`.
    #[inline]
    pub fn row(&self, y: u32) -> &[u8] {
        assert!(
            y < self.rows,
            "row index {y} out of bounds (rows: {})",
            self.rows
        );
        let start = y as usize * self.stride;
        let len = self.width as usize * self.descriptor.bytes_per_pixel();
        &self.data[start..start + len]
    }

    /// Mutable pixel bytes for row `y` (no padding).
    ///
    /// # Panics
    ///
    /// Panics if `y >= rows`.
    #[inline]
    pub fn row_mut(&mut self, y: u32) -> &mut [u8] {
        assert!(
            y < self.rows,
            "row index {y} out of bounds (rows: {})",
            self.rows
        );
        let start = y as usize * self.stride;
        let len = self.width as usize * self.descriptor.bytes_per_pixel();
        &mut self.data[start..start + len]
    }

    /// Borrow a mutable sub-range of rows.
    ///
    /// # Panics
    ///
    /// Panics if `y + count > rows`.
    pub fn sub_rows_mut(&mut self, y: u32, count: u32) -> PixelSliceMut<'_> {
        assert!(
            y.checked_add(count).is_some_and(|end| end <= self.rows),
            "sub_rows_mut({y}, {count}) out of bounds (rows: {})",
            self.rows
        );
        if count == 0 {
            return PixelSliceMut {
                data: &mut [],
                width: self.width,
                rows: 0,
                stride: self.stride,
                descriptor: self.descriptor,
                working_space: self.working_space,
                color: self.color.clone(),
            };
        }
        let bpp = self.descriptor.bytes_per_pixel();
        let start = y as usize * self.stride;
        let end = (y as usize + count as usize - 1) * self.stride + self.width as usize * bpp;
        PixelSliceMut {
            data: &mut self.data[start..end],
            width: self.width,
            rows: count,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
        }
    }
}

impl fmt::Debug for PixelSliceMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PixelSliceMut({}x{}, {:?} {:?})",
            self.width, self.rows, self.descriptor.layout, self.descriptor.channel_type
        )
    }
}

// ---------------------------------------------------------------------------
// PixelBuffer (owned, pool-friendly)
// ---------------------------------------------------------------------------

/// Owned pixel buffer with format metadata.
///
/// Wraps a `Vec<u8>` with an optional alignment offset so that pixel
/// rows start at the correct alignment for the channel type. The
/// backing vec can be recovered with [`into_vec`](Self::into_vec) for
/// pool reuse.
#[non_exhaustive]
pub struct PixelBuffer {
    data: Vec<u8>,
    /// Byte offset from `data` start to the first aligned pixel.
    offset: usize,
    width: u32,
    height: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    working_space: WorkingColorSpace,
    color: Option<Arc<ColorContext>>,
}

impl PixelBuffer {
    /// Allocate a zero-filled buffer for the given dimensions and format.
    pub fn new(width: u32, height: u32, descriptor: PixelDescriptor) -> Self {
        let stride = descriptor.aligned_stride(width);
        let total = stride * height as usize;
        let align = descriptor.min_alignment();
        let alloc_size = total + align - 1;
        let data = vec![0u8; alloc_size];
        let offset = align_offset(data.as_ptr(), align);
        Self {
            data,
            offset,
            width,
            height,
            stride,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
        }
    }

    /// Allocate a SIMD-aligned buffer for the given dimensions and format.
    ///
    /// Row stride is a multiple of `lcm(bpp, simd_align)`, ensuring every
    /// row start is both pixel-aligned and SIMD-aligned when the buffer
    /// itself starts at a SIMD-aligned address.
    ///
    /// `simd_align` must be a power of 2 (e.g. 16, 32, 64).
    pub fn new_simd_aligned(
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
        simd_align: usize,
    ) -> Self {
        let stride = descriptor.simd_aligned_stride(width, simd_align);
        let total = stride * height as usize;
        let alloc_size = total + simd_align - 1;
        let data = vec![0u8; alloc_size];
        let offset = align_offset(data.as_ptr(), simd_align);
        Self {
            data,
            offset,
            width,
            height,
            stride,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
        }
    }

    /// Wrap an existing `Vec<u8>` as a pixel buffer.
    ///
    /// The vec must be large enough to hold `aligned_stride(width) * height`
    /// bytes (plus any alignment offset). Stride is computed from the
    /// descriptor—rows are assumed tightly packed.
    ///
    /// # Errors
    ///
    /// Returns [`BufferError::InsufficientData`] if the vec is too small.
    pub fn from_vec(
        data: Vec<u8>,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<Self, BufferError> {
        let stride = descriptor.aligned_stride(width);
        let total = stride
            .checked_mul(height as usize)
            .ok_or(BufferError::InvalidDimensions)?;
        let align = descriptor.min_alignment();
        let offset = align_offset(data.as_ptr(), align);
        if data.len() < offset + total {
            return Err(BufferError::InsufficientData);
        }
        Ok(Self {
            data,
            offset,
            width,
            height,
            stride,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
        })
    }

    /// Consume the buffer and return the backing `Vec<u8>` for pool reuse.
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    /// Image width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Byte stride between row starts.
    #[inline]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Pixel format descriptor.
    #[inline]
    pub fn descriptor(&self) -> PixelDescriptor {
        self.descriptor
    }

    /// Source color context (ICC/CICP metadata), if set.
    #[inline]
    pub fn color_context(&self) -> Option<&Arc<ColorContext>> {
        self.color.as_ref()
    }

    /// Set the color context on this buffer.
    #[inline]
    pub fn with_color_context(mut self, ctx: Arc<ColorContext>) -> Self {
        self.color = Some(ctx);
        self
    }

    /// Current working color space.
    #[inline]
    pub fn working_space(&self) -> WorkingColorSpace {
        self.working_space
    }

    /// Set the working color space.
    #[inline]
    pub fn with_working_space(mut self, ws: WorkingColorSpace) -> Self {
        self.working_space = ws;
        self
    }

    /// Borrow the full buffer as an immutable [`PixelSlice`].
    pub fn as_slice(&self) -> PixelSlice<'_> {
        let total = self.stride * self.height as usize;
        PixelSlice {
            data: &self.data[self.offset..self.offset + total],
            width: self.width,
            rows: self.height,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
        }
    }

    /// Borrow the full buffer as a mutable [`PixelSliceMut`].
    pub fn as_slice_mut(&mut self) -> PixelSliceMut<'_> {
        let total = self.stride * self.height as usize;
        let offset = self.offset;
        PixelSliceMut {
            data: &mut self.data[offset..offset + total],
            width: self.width,
            rows: self.height,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
        }
    }

    /// Borrow a range of rows as an immutable [`PixelSlice`].
    ///
    /// # Panics
    ///
    /// Panics if `y + count > height`.
    pub fn rows(&self, y: u32, count: u32) -> PixelSlice<'_> {
        assert!(
            y.checked_add(count).is_some_and(|end| end <= self.height),
            "rows({y}, {count}) out of bounds (height: {})",
            self.height
        );
        if count == 0 {
            return PixelSlice {
                data: &[],
                width: self.width,
                rows: 0,
                stride: self.stride,
                descriptor: self.descriptor,
                working_space: self.working_space,
                color: self.color.clone(),
            };
        }
        let bpp = self.descriptor.bytes_per_pixel();
        let start = self.offset + y as usize * self.stride;
        let end = self.offset
            + (y as usize + count as usize - 1) * self.stride
            + self.width as usize * bpp;
        PixelSlice {
            data: &self.data[start..end],
            width: self.width,
            rows: count,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
        }
    }

    /// Borrow a range of rows as a mutable [`PixelSliceMut`].
    ///
    /// # Panics
    ///
    /// Panics if `y + count > height`.
    pub fn rows_mut(&mut self, y: u32, count: u32) -> PixelSliceMut<'_> {
        assert!(
            y.checked_add(count).is_some_and(|end| end <= self.height),
            "rows_mut({y}, {count}) out of bounds (height: {})",
            self.height
        );
        if count == 0 {
            return PixelSliceMut {
                data: &mut [],
                width: self.width,
                rows: 0,
                stride: self.stride,
                descriptor: self.descriptor,
                working_space: self.working_space,
                color: self.color.clone(),
            };
        }
        let bpp = self.descriptor.bytes_per_pixel();
        let start = self.offset + y as usize * self.stride;
        let end = self.offset
            + (y as usize + count as usize - 1) * self.stride
            + self.width as usize * bpp;
        PixelSliceMut {
            data: &mut self.data[start..end],
            width: self.width,
            rows: count,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
        }
    }

    /// Zero-copy sub-region view (immutable).
    ///
    /// # Panics
    ///
    /// Panics if the crop region is out of bounds.
    pub fn crop_view(&self, x: u32, y: u32, w: u32, h: u32) -> PixelSlice<'_> {
        assert!(
            x.checked_add(w).is_some_and(|end| end <= self.width),
            "crop x={x} w={w} exceeds width {}",
            self.width
        );
        assert!(
            y.checked_add(h).is_some_and(|end| end <= self.height),
            "crop y={y} h={h} exceeds height {}",
            self.height
        );
        if h == 0 || w == 0 {
            return PixelSlice {
                data: &[],
                width: w,
                rows: h,
                stride: self.stride,
                descriptor: self.descriptor,
                working_space: self.working_space,
                color: self.color.clone(),
            };
        }
        let bpp = self.descriptor.bytes_per_pixel();
        let start = self.offset + y as usize * self.stride + x as usize * bpp;
        let end = self.offset
            + (y as usize + h as usize - 1) * self.stride
            + (x as usize + w as usize) * bpp;
        PixelSlice {
            data: &self.data[start..end],
            width: w,
            rows: h,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
        }
    }

    /// Copy a sub-region into a new, tightly-packed [`PixelBuffer`].
    ///
    /// # Panics
    ///
    /// Panics if the crop region is out of bounds.
    pub fn crop_copy(&self, x: u32, y: u32, w: u32, h: u32) -> PixelBuffer {
        let src = self.crop_view(x, y, w, h);
        let mut dst = PixelBuffer::new(w, h, self.descriptor);
        dst.working_space = self.working_space;
        dst.color = self.color.clone();
        let bpp = self.descriptor.bytes_per_pixel();
        let row_bytes = w as usize * bpp;
        for row_y in 0..h {
            let src_row = src.row(row_y);
            let dst_start = dst.offset + row_y as usize * dst.stride;
            dst.data[dst_start..dst_start + row_bytes].copy_from_slice(&src_row[..row_bytes]);
        }
        dst
    }
}

impl fmt::Debug for PixelBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PixelBuffer({}x{}, {:?} {:?})",
            self.width, self.height, self.descriptor.layout, self.descriptor.channel_type
        )
    }
}

// ---------------------------------------------------------------------------
// ImgRef → PixelSlice (zero-copy From impls)
// ---------------------------------------------------------------------------

macro_rules! impl_from_imgref {
    ($pixel:ty, $descriptor:expr) => {
        impl<'a> From<ImgRef<'a, $pixel>> for PixelSlice<'a> {
            fn from(img: ImgRef<'a, $pixel>) -> Self {
                use rgb::ComponentBytes;
                let bytes = img.buf().as_bytes();
                let byte_stride = img.stride() * core::mem::size_of::<$pixel>();
                PixelSlice {
                    data: bytes,
                    width: img.width() as u32,
                    rows: img.height() as u32,
                    stride: byte_stride,
                    descriptor: $descriptor,
                    working_space: WorkingColorSpace::Native,
                    color: None,
                }
            }
        }
    };
}

// u8 types are conventionally sRGB, f32 types are conventionally linear.
// u16 types have no standard convention so use transfer-agnostic descriptors.
impl_from_imgref!(Rgb<u8>, PixelDescriptor::RGB8_SRGB);
impl_from_imgref!(Rgba<u8>, PixelDescriptor::RGBA8_SRGB);
impl_from_imgref!(Rgb<u16>, PixelDescriptor::RGB16);
impl_from_imgref!(Rgba<u16>, PixelDescriptor::RGBA16);
impl_from_imgref!(Rgb<f32>, PixelDescriptor::RGBF32_LINEAR);
impl_from_imgref!(Rgba<f32>, PixelDescriptor::RGBAF32_LINEAR);
impl_from_imgref!(Gray<u8>, PixelDescriptor::GRAY8_SRGB);
impl_from_imgref!(Gray<u16>, PixelDescriptor::GRAY16);
impl_from_imgref!(Gray<f32>, PixelDescriptor::GRAYF32_LINEAR);
impl_from_imgref!(BGRA<u8>, PixelDescriptor::BGRA8_SRGB);

// ---------------------------------------------------------------------------
// ImgRefMut → PixelSliceMut (zero-copy From impls)
// ---------------------------------------------------------------------------

macro_rules! impl_from_imgref_mut {
    ($pixel:ty, $descriptor:expr) => {
        impl<'a> From<imgref::ImgRefMut<'a, $pixel>> for PixelSliceMut<'a> {
            fn from(img: imgref::ImgRefMut<'a, $pixel>) -> Self {
                use rgb::ComponentBytes;
                let width = img.width() as u32;
                let rows = img.height() as u32;
                let byte_stride = img.stride() * core::mem::size_of::<$pixel>();
                let buf = img.into_buf();
                let bytes = buf.as_bytes_mut();
                PixelSliceMut {
                    data: bytes,
                    width,
                    rows,
                    stride: byte_stride,
                    descriptor: $descriptor,
                    working_space: WorkingColorSpace::Native,
                    color: None,
                }
            }
        }
    };
}

impl_from_imgref_mut!(Rgb<u8>, PixelDescriptor::RGB8_SRGB);
impl_from_imgref_mut!(Rgba<u8>, PixelDescriptor::RGBA8_SRGB);
impl_from_imgref_mut!(Rgb<u16>, PixelDescriptor::RGB16);
impl_from_imgref_mut!(Rgba<u16>, PixelDescriptor::RGBA16);
impl_from_imgref_mut!(Rgb<f32>, PixelDescriptor::RGBF32_LINEAR);
impl_from_imgref_mut!(Rgba<f32>, PixelDescriptor::RGBAF32_LINEAR);
impl_from_imgref_mut!(Gray<u8>, PixelDescriptor::GRAY8_SRGB);
impl_from_imgref_mut!(Gray<u16>, PixelDescriptor::GRAY16);
impl_from_imgref_mut!(Gray<f32>, PixelDescriptor::GRAYF32_LINEAR);
impl_from_imgref_mut!(BGRA<u8>, PixelDescriptor::BGRA8_SRGB);

// ---------------------------------------------------------------------------
// PixelData → PixelBuffer (From, always copies)
// ---------------------------------------------------------------------------

impl From<PixelData> for PixelBuffer {
    fn from(pixels: PixelData) -> Self {
        let width = pixels.width();
        let height = pixels.height();
        let descriptor = pixels.descriptor();
        let data = pixels.to_bytes();
        let stride = descriptor.aligned_stride(width);
        Self {
            data,
            offset: 0,
            width,
            height,
            stride,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
        }
    }
}

// ---------------------------------------------------------------------------
// PixelBuffer → PixelData (TryFrom, always copies)
// ---------------------------------------------------------------------------

impl TryFrom<PixelBuffer> for PixelData {
    type Error = BufferError;

    fn try_from(buf: PixelBuffer) -> Result<Self, BufferError> {
        let w = buf.width as usize;
        let h = buf.height as usize;
        let slice = buf.as_slice();

        match (buf.descriptor.channel_type, buf.descriptor.layout) {
            (ChannelType::U8, ChannelLayout::Rgb) => {
                let pixels = collect_rows(&slice, w, |c| Rgb {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                });
                Ok(PixelData::Rgb8(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::U8, ChannelLayout::Rgba) => {
                let pixels = collect_rows(&slice, w, |c| Rgba {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                    a: c[3],
                });
                Ok(PixelData::Rgba8(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::U8, ChannelLayout::Gray) => {
                let pixels = collect_rows(&slice, w, |c| Gray::new(c[0]));
                Ok(PixelData::Gray8(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::U8, ChannelLayout::GrayAlpha) => {
                let pixels = collect_rows(&slice, w, |c| GrayAlpha::new(c[0], c[1]));
                Ok(PixelData::GrayAlpha8(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::U8, ChannelLayout::Bgra) => {
                let pixels = collect_rows(&slice, w, |c| BGRA {
                    b: c[0],
                    g: c[1],
                    r: c[2],
                    a: c[3],
                });
                Ok(PixelData::Bgra8(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::U16, ChannelLayout::Rgb) => {
                let pixels = collect_rows(&slice, w, |c| Rgb {
                    r: parse_u16(c),
                    g: parse_u16(&c[2..]),
                    b: parse_u16(&c[4..]),
                });
                Ok(PixelData::Rgb16(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::U16, ChannelLayout::Rgba) => {
                let pixels = collect_rows(&slice, w, |c| Rgba {
                    r: parse_u16(c),
                    g: parse_u16(&c[2..]),
                    b: parse_u16(&c[4..]),
                    a: parse_u16(&c[6..]),
                });
                Ok(PixelData::Rgba16(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::U16, ChannelLayout::Gray) => {
                let pixels = collect_rows(&slice, w, |c| Gray::new(parse_u16(c)));
                Ok(PixelData::Gray16(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::U16, ChannelLayout::GrayAlpha) => {
                let pixels = collect_rows(&slice, w, |c| {
                    GrayAlpha::new(parse_u16(c), parse_u16(&c[2..]))
                });
                Ok(PixelData::GrayAlpha16(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::F32, ChannelLayout::Rgb) => {
                let pixels = collect_rows(&slice, w, |c| Rgb {
                    r: parse_f32(c),
                    g: parse_f32(&c[4..]),
                    b: parse_f32(&c[8..]),
                });
                Ok(PixelData::RgbF32(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::F32, ChannelLayout::Rgba) => {
                let pixels = collect_rows(&slice, w, |c| Rgba {
                    r: parse_f32(c),
                    g: parse_f32(&c[4..]),
                    b: parse_f32(&c[8..]),
                    a: parse_f32(&c[12..]),
                });
                Ok(PixelData::RgbaF32(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::F32, ChannelLayout::Gray) => {
                let pixels = collect_rows(&slice, w, |c| Gray::new(parse_f32(c)));
                Ok(PixelData::GrayF32(imgref::ImgVec::new(pixels, w, h)))
            }
            (ChannelType::F32, ChannelLayout::GrayAlpha) => {
                let pixels = collect_rows(&slice, w, |c| {
                    GrayAlpha::new(parse_f32(c), parse_f32(&c[4..]))
                });
                Ok(PixelData::GrayAlphaF32(imgref::ImgVec::new(pixels, w, h)))
            }
            _ => Err(BufferError::FormatMismatch),
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Round `val` up to the next multiple of `align` (must be a power of 2).
const fn align_up(val: usize, align: usize) -> usize {
    (val + align - 1) & !(align - 1)
}

/// Round `val` up to the next multiple of `align` (any positive integer).
const fn align_up_general(val: usize, align: usize) -> usize {
    let rem = val % align;
    if rem == 0 { val } else { val + (align - rem) }
}

/// Greatest common divisor (Euclidean algorithm).
const fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Least common multiple.
const fn lcm(a: usize, b: usize) -> usize {
    a / gcd(a, b) * b
}

/// Compute the byte offset needed to align `ptr` to `align`.
fn align_offset(ptr: *const u8, align: usize) -> usize {
    let addr = ptr as usize;
    align_up(addr, align) - addr
}

/// Minimum bytes needed: `(rows - 1) * stride + min_stride`.
fn required_bytes(rows: u32, stride: usize, min_stride: usize) -> Result<usize, BufferError> {
    let preceding = (rows as usize - 1)
        .checked_mul(stride)
        .ok_or(BufferError::InvalidDimensions)?;
    preceding
        .checked_add(min_stride)
        .ok_or(BufferError::InvalidDimensions)
}

/// Collect typed pixels from a PixelSlice by parsing each pixel's bytes.
fn collect_rows<T>(slice: &PixelSlice<'_>, width: usize, parse: impl Fn(&[u8]) -> T) -> Vec<T> {
    let bpp = slice.descriptor.bytes_per_pixel();
    let mut pixels = Vec::with_capacity(width * slice.rows as usize);
    for y in 0..slice.rows {
        let row = slice.row(y);
        for chunk in row.chunks_exact(bpp) {
            pixels.push(parse(chunk));
        }
    }
    pixels
}

#[inline]
fn parse_u16(bytes: &[u8]) -> u16 {
    u16::from_ne_bytes([bytes[0], bytes[1]])
}

#[inline]
fn parse_f32(bytes: &[u8]) -> f32 {
    f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec;

    // --- PixelDescriptor arithmetic ---

    #[test]
    fn channel_type_byte_size() {
        assert_eq!(ChannelType::U8.byte_size(), 1);
        assert_eq!(ChannelType::U16.byte_size(), 2);
        assert_eq!(ChannelType::F32.byte_size(), 4);
    }

    #[test]
    fn channel_layout_channels() {
        assert_eq!(ChannelLayout::Gray.channels(), 1);
        assert_eq!(ChannelLayout::GrayAlpha.channels(), 2);
        assert_eq!(ChannelLayout::Rgb.channels(), 3);
        assert_eq!(ChannelLayout::Rgba.channels(), 4);
        assert_eq!(ChannelLayout::Bgra.channels(), 4);
    }

    #[test]
    fn channel_layout_has_alpha() {
        assert!(!ChannelLayout::Gray.has_alpha());
        assert!(ChannelLayout::GrayAlpha.has_alpha());
        assert!(!ChannelLayout::Rgb.has_alpha());
        assert!(ChannelLayout::Rgba.has_alpha());
        assert!(ChannelLayout::Bgra.has_alpha());
    }

    #[test]
    fn descriptor_bytes_per_pixel() {
        assert_eq!(PixelDescriptor::RGB8_SRGB.bytes_per_pixel(), 3);
        assert_eq!(PixelDescriptor::RGBA8_SRGB.bytes_per_pixel(), 4);
        assert_eq!(PixelDescriptor::RGB16_SRGB.bytes_per_pixel(), 6);
        assert_eq!(PixelDescriptor::RGBA16_SRGB.bytes_per_pixel(), 8);
        assert_eq!(PixelDescriptor::RGBF32_LINEAR.bytes_per_pixel(), 12);
        assert_eq!(PixelDescriptor::RGBAF32_LINEAR.bytes_per_pixel(), 16);
        assert_eq!(PixelDescriptor::GRAY8_SRGB.bytes_per_pixel(), 1);
        assert_eq!(PixelDescriptor::GRAY16_SRGB.bytes_per_pixel(), 2);
        assert_eq!(PixelDescriptor::GRAYF32_LINEAR.bytes_per_pixel(), 4);
        assert_eq!(PixelDescriptor::GRAYA8_SRGB.bytes_per_pixel(), 2);
        assert_eq!(PixelDescriptor::BGRA8_SRGB.bytes_per_pixel(), 4);
        assert_eq!(PixelDescriptor::BGRX8_SRGB.bytes_per_pixel(), 4);
    }

    #[test]
    fn descriptor_alignment() {
        assert_eq!(PixelDescriptor::RGB8_SRGB.min_alignment(), 1);
        assert_eq!(PixelDescriptor::RGB16_SRGB.min_alignment(), 2);
        assert_eq!(PixelDescriptor::RGBF32_LINEAR.min_alignment(), 4);
    }

    #[test]
    fn descriptor_aligned_stride() {
        // RGB8: width=10, bpp=3 → stride=30
        assert_eq!(PixelDescriptor::RGB8_SRGB.aligned_stride(10), 30);
        // RGB16: width=10, bpp=6 → stride=60
        assert_eq!(PixelDescriptor::RGB16_SRGB.aligned_stride(10), 60);
        // RGBF32: width=10, bpp=12 → stride=120
        assert_eq!(PixelDescriptor::RGBF32_LINEAR.aligned_stride(10), 120);
        // Gray8: width=1, bpp=1 → stride=1
        assert_eq!(PixelDescriptor::GRAY8_SRGB.aligned_stride(1), 1);
    }

    #[test]
    fn descriptor_simd_aligned_stride() {
        // RGB8 bpp=3 with simd=64 → lcm(3,64)=192 → next multiple of 192
        // width=10, raw=30 → align_up_general(30, 192) = 192
        assert_eq!(PixelDescriptor::RGB8_SRGB.simd_aligned_stride(10, 64), 192);
        // RGBA8 bpp=4 with simd=64 → lcm(4,64)=64
        // width=10, raw=40 → align_up_general(40, 64) = 64
        assert_eq!(PixelDescriptor::RGBA8_SRGB.simd_aligned_stride(10, 64), 64);
        // RGBF32 bpp=12 with simd=64 → lcm(12,64)=192
        // width=10, raw=120 → align_up_general(120, 192) = 192
        assert_eq!(
            PixelDescriptor::RGBF32_LINEAR.simd_aligned_stride(10, 64),
            192
        );
        // RGBAF32 bpp=16 with simd=64 → lcm(16,64)=64
        // width=10, raw=160 → align_up_general(160, 64) = 192
        assert_eq!(
            PixelDescriptor::RGBAF32_LINEAR.simd_aligned_stride(10, 64),
            192
        );
        // Gray8 bpp=1 with simd=64 → lcm(1,64)=64
        // width=100, raw=100 → 128
        assert_eq!(
            PixelDescriptor::GRAY8_SRGB.simd_aligned_stride(100, 64),
            128
        );
    }

    #[test]
    fn stride_not_pixel_aligned_rejected() {
        // RGB8 bpp=3, stride=32 is not a multiple of 3
        let data = [0u8; 128];
        let err = PixelSlice::new(&data, 10, 1, 32, PixelDescriptor::RGB8_SRGB);
        assert_eq!(err.unwrap_err(), BufferError::StrideNotPixelAligned);

        // stride=33 IS a multiple of 3 → accepted
        let ok = PixelSlice::new(&data, 10, 1, 33, PixelDescriptor::RGB8_SRGB);
        assert!(ok.is_ok());
    }

    #[test]
    fn stride_pixel_aligned_accepted() {
        // RGBA8 bpp=4, stride=48 is a multiple of 4
        let data = [0u8; 256];
        let ok = PixelSlice::new(&data, 10, 2, 48, PixelDescriptor::RGBA8_SRGB);
        assert!(ok.is_ok());
        let s = ok.unwrap();
        assert_eq!(s.stride(), 48);
    }

    #[test]
    fn pixel_buffer_simd_aligned() {
        let buf = PixelBuffer::new_simd_aligned(10, 5, PixelDescriptor::RGBA8_SRGB, 64);
        assert_eq!(buf.width(), 10);
        assert_eq!(buf.height(), 5);
        // RGBA8 bpp=4, lcm(4,64)=64, raw=40 → stride=64
        assert_eq!(buf.stride(), 64);
        // First row should be 64-byte aligned
        let slice = buf.as_slice();
        assert_eq!(slice.data.as_ptr() as usize % 64, 0);
    }

    #[test]
    fn descriptor_channels_and_alpha() {
        assert_eq!(PixelDescriptor::RGB8_SRGB.channels(), 3);
        assert!(!PixelDescriptor::RGB8_SRGB.has_alpha());
        assert_eq!(PixelDescriptor::RGBA8_SRGB.channels(), 4);
        assert!(PixelDescriptor::RGBA8_SRGB.has_alpha());
        assert!(PixelDescriptor::BGRA8_SRGB.has_alpha());
        // BGRX8 has Bgra layout (4 channels) but AlphaMode::None
        assert_eq!(PixelDescriptor::BGRX8_SRGB.channels(), 4);
        assert!(PixelDescriptor::BGRX8_SRGB.layout.has_alpha()); // layout says yes
        assert_eq!(PixelDescriptor::BGRX8_SRGB.alpha, AlphaMode::None); // but alpha is None
    }

    #[test]
    fn descriptor_is_linear() {
        assert!(!PixelDescriptor::RGB8_SRGB.is_linear());
        assert!(PixelDescriptor::RGBF32_LINEAR.is_linear());
        assert!(!PixelDescriptor::RGB8.is_linear()); // Unknown is not linear
    }

    #[test]
    fn descriptor_is_unknown_transfer() {
        assert!(PixelDescriptor::RGB8.is_unknown_transfer());
        assert!(PixelDescriptor::RGBF32.is_unknown_transfer());
        assert!(!PixelDescriptor::RGB8_SRGB.is_unknown_transfer());
        assert!(!PixelDescriptor::RGBF32_LINEAR.is_unknown_transfer());
    }

    #[test]
    fn descriptor_with_transfer() {
        // Resolve Unknown → Srgb
        let desc = PixelDescriptor::RGB8;
        assert!(desc.is_unknown_transfer());
        let resolved = desc.with_transfer(TransferFunction::Srgb);
        assert_eq!(resolved, PixelDescriptor::RGB8_SRGB);
        assert!(!resolved.is_unknown_transfer());

        // Resolve Unknown → Linear
        let desc = PixelDescriptor::RGBF32;
        let resolved = desc.with_transfer(TransferFunction::Linear);
        assert_eq!(resolved, PixelDescriptor::RGBF32_LINEAR);
        assert!(resolved.is_linear());

        // Unknown constants are layout-compatible with explicit ones
        assert!(PixelDescriptor::RGB8.layout_compatible(&PixelDescriptor::RGB8_SRGB));
        assert!(PixelDescriptor::RGBF32.layout_compatible(&PixelDescriptor::RGBF32_LINEAR));
    }

    #[test]
    fn transfer_unknown_variant() {
        assert_eq!(TransferFunction::Unknown as u8, 255);
        assert_ne!(TransferFunction::Unknown, TransferFunction::Srgb);
        assert_ne!(TransferFunction::Unknown, TransferFunction::Linear);
    }

    #[test]
    fn transfer_from_cicp() {
        assert_eq!(
            TransferFunction::from_cicp(1),
            Some(TransferFunction::Bt709)
        );
        assert_eq!(
            TransferFunction::from_cicp(8),
            Some(TransferFunction::Linear)
        );
        assert_eq!(
            TransferFunction::from_cicp(13),
            Some(TransferFunction::Srgb)
        );
        assert_eq!(TransferFunction::from_cicp(16), Some(TransferFunction::Pq));
        assert_eq!(TransferFunction::from_cicp(18), Some(TransferFunction::Hlg));
        assert_eq!(TransferFunction::from_cicp(99), None);
    }

    // --- PixelBuffer allocation and row access ---

    #[test]
    fn pixel_buffer_new_rgb8() {
        let buf = PixelBuffer::new(10, 5, PixelDescriptor::RGB8_SRGB);
        assert_eq!(buf.width(), 10);
        assert_eq!(buf.height(), 5);
        assert_eq!(buf.stride(), 30);
        assert_eq!(buf.descriptor(), PixelDescriptor::RGB8_SRGB);
        // All zeros
        let slice = buf.as_slice();
        assert_eq!(slice.row(0), &[0u8; 30]);
        assert_eq!(slice.row(4), &[0u8; 30]);
    }

    #[test]
    fn pixel_buffer_from_vec() {
        let data = vec![0u8; 30 * 5];
        let buf = PixelBuffer::from_vec(data, 10, 5, PixelDescriptor::RGB8_SRGB).unwrap();
        assert_eq!(buf.width(), 10);
        assert_eq!(buf.height(), 5);
    }

    #[test]
    fn pixel_buffer_from_vec_too_small() {
        let data = vec![0u8; 10];
        let err = PixelBuffer::from_vec(data, 10, 5, PixelDescriptor::RGB8_SRGB);
        assert_eq!(err.unwrap_err(), BufferError::InsufficientData);
    }

    #[test]
    fn pixel_buffer_into_vec_roundtrip() {
        let buf = PixelBuffer::new(4, 4, PixelDescriptor::RGBA8_SRGB);
        let v = buf.into_vec();
        // Can re-wrap it
        let buf2 = PixelBuffer::from_vec(v, 4, 4, PixelDescriptor::RGBA8_SRGB).unwrap();
        assert_eq!(buf2.width(), 4);
    }

    #[test]
    fn pixel_buffer_write_and_read() {
        let mut buf = PixelBuffer::new(2, 2, PixelDescriptor::RGB8_SRGB);
        {
            let mut slice = buf.as_slice_mut();
            let row = slice.row_mut(0);
            row[0] = 255;
            row[1] = 128;
            row[2] = 64;
        }
        let slice = buf.as_slice();
        assert_eq!(&slice.row(0)[..3], &[255, 128, 64]);
        assert_eq!(&slice.row(1)[..3], &[0, 0, 0]);
    }

    // --- PixelSlice crop_view ---

    #[test]
    fn pixel_slice_crop_view() {
        // 4x4 RGB8 buffer, fill each row with row index
        let mut buf = PixelBuffer::new(4, 4, PixelDescriptor::RGB8_SRGB);
        {
            let mut slice = buf.as_slice_mut();
            for y in 0..4u32 {
                let row = slice.row_mut(y);
                for byte in row.iter_mut() {
                    *byte = y as u8;
                }
            }
        }
        // Crop 2x2 starting at (1, 1)
        let crop = buf.crop_view(1, 1, 2, 2);
        assert_eq!(crop.width(), 2);
        assert_eq!(crop.rows(), 2);
        // Row 0 of crop = row 1 of original, should be all 1s
        assert_eq!(crop.row(0), &[1, 1, 1, 1, 1, 1]);
        // Row 1 of crop = row 2 of original, should be all 2s
        assert_eq!(crop.row(1), &[2, 2, 2, 2, 2, 2]);
    }

    #[test]
    fn pixel_slice_crop_copy() {
        let mut buf = PixelBuffer::new(4, 4, PixelDescriptor::RGB8_SRGB);
        {
            let mut slice = buf.as_slice_mut();
            for y in 0..4u32 {
                let row = slice.row_mut(y);
                for (i, byte) in row.iter_mut().enumerate() {
                    *byte = (y * 100 + i as u32) as u8;
                }
            }
        }
        let cropped = buf.crop_copy(1, 1, 2, 2);
        assert_eq!(cropped.width(), 2);
        assert_eq!(cropped.height(), 2);
        // Row 0: original row 1, pixels 1-2 → bytes [103,104,105, 106,107,108]
        assert_eq!(cropped.as_slice().row(0), &[103, 104, 105, 106, 107, 108]);
    }

    #[test]
    fn pixel_slice_sub_rows() {
        let mut buf = PixelBuffer::new(2, 4, PixelDescriptor::GRAY8_SRGB);
        {
            let mut slice = buf.as_slice_mut();
            for y in 0..4u32 {
                let row = slice.row_mut(y);
                row[0] = y as u8 * 10;
                row[1] = y as u8 * 10 + 1;
            }
        }
        let sub = buf.rows(1, 2);
        assert_eq!(sub.rows(), 2);
        assert_eq!(sub.row(0), &[10, 11]);
        assert_eq!(sub.row(1), &[20, 21]);
    }

    // --- PixelSlice validation ---

    #[test]
    fn pixel_slice_stride_too_small() {
        let data = [0u8; 100];
        let err = PixelSlice::new(&data, 10, 1, 2, PixelDescriptor::RGB8_SRGB);
        assert_eq!(err.unwrap_err(), BufferError::StrideTooSmall);
    }

    #[test]
    fn pixel_slice_insufficient_data() {
        let data = [0u8; 10];
        let err = PixelSlice::new(&data, 10, 1, 30, PixelDescriptor::RGB8_SRGB);
        assert_eq!(err.unwrap_err(), BufferError::InsufficientData);
    }

    #[test]
    fn pixel_slice_zero_rows() {
        let data = [0u8; 0];
        let slice = PixelSlice::new(&data, 10, 0, 30, PixelDescriptor::RGB8_SRGB).unwrap();
        assert_eq!(slice.rows(), 0);
    }

    // --- PixelData → descriptor() roundtrip ---

    #[test]
    fn pixel_data_descriptor_matches() {
        use imgref::ImgVec;

        // PixelData::descriptor() returns Unknown transfer — the actual TF
        // is only known from CICP/ICC metadata, not from the pixel type.
        let cases: Vec<(PixelData, PixelDescriptor)> = vec![
            (
                PixelData::Rgb8(ImgVec::new(vec![Rgb { r: 0, g: 0, b: 0 }], 1, 1)),
                PixelDescriptor::RGB8,
            ),
            (
                PixelData::Rgba8(ImgVec::new(
                    vec![Rgba {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: 0,
                    }],
                    1,
                    1,
                )),
                PixelDescriptor::RGBA8,
            ),
            (
                PixelData::Rgb16(ImgVec::new(vec![Rgb { r: 0, g: 0, b: 0 }], 1, 1)),
                PixelDescriptor::RGB16,
            ),
            (
                PixelData::Rgba16(ImgVec::new(
                    vec![Rgba {
                        r: 0u16,
                        g: 0,
                        b: 0,
                        a: 0,
                    }],
                    1,
                    1,
                )),
                PixelDescriptor::RGBA16,
            ),
            (
                PixelData::RgbF32(ImgVec::new(
                    vec![Rgb {
                        r: 0.0f32,
                        g: 0.0,
                        b: 0.0,
                    }],
                    1,
                    1,
                )),
                PixelDescriptor::RGBF32,
            ),
            (
                PixelData::RgbaF32(ImgVec::new(
                    vec![Rgba {
                        r: 0.0f32,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }],
                    1,
                    1,
                )),
                PixelDescriptor::RGBAF32,
            ),
            (
                PixelData::Gray8(ImgVec::new(vec![Gray::new(0u8)], 1, 1)),
                PixelDescriptor::GRAY8,
            ),
            (
                PixelData::Gray16(ImgVec::new(vec![Gray::new(0u16)], 1, 1)),
                PixelDescriptor::GRAY16,
            ),
            (
                PixelData::GrayF32(ImgVec::new(vec![Gray::new(0.0f32)], 1, 1)),
                PixelDescriptor::GRAYF32,
            ),
            (
                PixelData::Bgra8(ImgVec::new(
                    vec![BGRA {
                        b: 0,
                        g: 0,
                        r: 0,
                        a: 0,
                    }],
                    1,
                    1,
                )),
                PixelDescriptor::BGRA8,
            ),
            (
                PixelData::GrayAlpha8(ImgVec::new(vec![GrayAlpha::new(0u8, 0)], 1, 1)),
                PixelDescriptor::GRAYA8,
            ),
            (
                PixelData::GrayAlpha16(ImgVec::new(vec![GrayAlpha::new(0u16, 0)], 1, 1)),
                PixelDescriptor::GRAYA16,
            ),
            (
                PixelData::GrayAlphaF32(ImgVec::new(vec![GrayAlpha::new(0.0f32, 0.0)], 1, 1)),
                PixelDescriptor::GRAYAF32,
            ),
        ];

        for (data, expected) in cases {
            assert_eq!(
                data.descriptor(),
                expected,
                "descriptor mismatch for {:?}",
                data
            );
        }
    }

    // --- ImgRef → PixelSlice → row access ---

    #[test]
    fn imgref_to_pixel_slice_rgb8() {
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 10,
                g: 20,
                b: 30,
            },
            Rgb {
                r: 40,
                g: 50,
                b: 60,
            },
            Rgb {
                r: 70,
                g: 80,
                b: 90,
            },
            Rgb {
                r: 100,
                g: 110,
                b: 120,
            },
        ];
        let img = imgref::Img::new(pixels.as_slice(), 2, 2);
        let slice: PixelSlice<'_> = img.into();
        assert_eq!(slice.width(), 2);
        assert_eq!(slice.rows(), 2);
        assert_eq!(slice.row(0), &[10, 20, 30, 40, 50, 60]);
        assert_eq!(slice.row(1), &[70, 80, 90, 100, 110, 120]);
    }

    #[test]
    fn imgref_to_pixel_slice_gray16() {
        let pixels = vec![Gray::new(1000u16), Gray::new(2000u16)];
        let img = imgref::Img::new(pixels.as_slice(), 2, 1);
        let slice: PixelSlice<'_> = img.into();
        assert_eq!(slice.width(), 2);
        assert_eq!(slice.rows(), 1);
        assert_eq!(slice.descriptor(), PixelDescriptor::GRAY16);
        // Bytes should be native-endian u16
        let row = slice.row(0);
        assert_eq!(row.len(), 4);
        let v0 = u16::from_ne_bytes([row[0], row[1]]);
        let v1 = u16::from_ne_bytes([row[2], row[3]]);
        assert_eq!(v0, 1000);
        assert_eq!(v1, 2000);
    }

    // --- PixelBuffer → PixelData TryFrom roundtrip ---

    #[test]
    fn pixel_buffer_to_pixel_data_rgb8() {
        let pixels = vec![
            Rgb {
                r: 10u8,
                g: 20,
                b: 30,
            },
            Rgb {
                r: 40u8,
                g: 50,
                b: 60,
            },
        ];
        let img = imgref::ImgVec::new(pixels, 2, 1);
        let data = PixelData::Rgb8(img);
        let buf = PixelBuffer::from(data);
        let data2 = PixelData::try_from(buf).unwrap();
        if let PixelData::Rgb8(img) = data2 {
            assert_eq!(img.width(), 2);
            assert_eq!(img.height(), 1);
            assert_eq!(
                img.buf()[0],
                Rgb {
                    r: 10,
                    g: 20,
                    b: 30
                }
            );
            assert_eq!(
                img.buf()[1],
                Rgb {
                    r: 40,
                    g: 50,
                    b: 60
                }
            );
        } else {
            panic!("expected Rgb8");
        }
    }

    #[test]
    fn pixel_buffer_to_pixel_data_rgba16() {
        let pixels = vec![Rgba {
            r: 1000u16,
            g: 2000,
            b: 3000,
            a: 4000,
        }];
        let img = imgref::ImgVec::new(pixels, 1, 1);
        let data = PixelData::Rgba16(img);
        let buf = PixelBuffer::from(data);
        let data2 = PixelData::try_from(buf).unwrap();
        if let PixelData::Rgba16(img) = data2 {
            assert_eq!(
                img.buf()[0],
                Rgba {
                    r: 1000,
                    g: 2000,
                    b: 3000,
                    a: 4000
                }
            );
        } else {
            panic!("expected Rgba16");
        }
    }

    #[test]
    fn pixel_buffer_to_pixel_data_gray_alpha_f32() {
        let pixels = vec![GrayAlpha::new(0.5f32, 0.75)];
        let img = imgref::ImgVec::new(pixels, 1, 1);
        let data = PixelData::GrayAlphaF32(img);
        let buf = PixelBuffer::from(data);
        let data2 = PixelData::try_from(buf).unwrap();
        if let PixelData::GrayAlphaF32(img) = data2 {
            let px = &img.buf()[0];
            assert!((px.v - 0.5).abs() < 1e-6);
            assert!((px.a - 0.75).abs() < 1e-6);
        } else {
            panic!("expected GrayAlphaF32");
        }
    }

    #[test]
    fn pixel_buffer_to_pixel_data_bgra8() {
        let pixels = vec![BGRA {
            b: 10,
            g: 20,
            r: 30,
            a: 40,
        }];
        let img = imgref::ImgVec::new(pixels, 1, 1);
        let data = PixelData::Bgra8(img);
        let buf = PixelBuffer::from(data);
        let data2 = PixelData::try_from(buf).unwrap();
        if let PixelData::Bgra8(img) = data2 {
            let px = &img.buf()[0];
            assert_eq!((px.b, px.g, px.r, px.a), (10, 20, 30, 40));
        } else {
            panic!("expected Bgra8");
        }
    }

    #[test]
    fn pixel_buffer_format_mismatch() {
        // U16 + Bgra has no PixelData variant
        let desc = PixelDescriptor {
            channel_type: ChannelType::U16,
            layout: ChannelLayout::Bgra,
            alpha: AlphaMode::Straight,
            transfer: TransferFunction::Srgb,
        };
        let buf = PixelBuffer::new(1, 1, desc);
        let err = PixelData::try_from(buf);
        assert_eq!(err.unwrap_err(), BufferError::FormatMismatch);
    }

    // --- Debug formatting ---

    #[test]
    fn debug_formats() {
        let buf = PixelBuffer::new(10, 5, PixelDescriptor::RGB8_SRGB);
        assert_eq!(format!("{buf:?}"), "PixelBuffer(10x5, Rgb U8)");

        let slice = buf.as_slice();
        assert_eq!(format!("{slice:?}"), "PixelSlice(10x5, Rgb U8)");

        let mut buf = PixelBuffer::new(3, 3, PixelDescriptor::RGBA16_SRGB);
        let slice_mut = buf.as_slice_mut();
        assert_eq!(format!("{slice_mut:?}"), "PixelSliceMut(3x3, Rgba U16)");
    }

    // --- BufferError Display ---

    #[test]
    fn buffer_error_display() {
        let msg = format!("{}", BufferError::StrideTooSmall);
        assert!(msg.contains("stride"));
    }

    // --- Edge cases ---

    #[test]
    fn bgrx8_srgb_properties() {
        let d = PixelDescriptor::BGRX8_SRGB;
        assert_eq!(d.channel_type, ChannelType::U8);
        assert_eq!(d.layout, ChannelLayout::Bgra);
        assert_eq!(d.alpha, AlphaMode::None);
        assert_eq!(d.transfer, TransferFunction::Srgb);
        assert_eq!(d.bytes_per_pixel(), 4);
        assert_eq!(d.min_alignment(), 1);
        // Layout-compatible with BGRA8
        assert!(d.layout_compatible(&PixelDescriptor::BGRA8_SRGB));
        // BGRX has no meaningful alpha — the fourth byte is padding
        assert!(!d.has_alpha());
        // BGRA does have meaningful alpha
        assert!(PixelDescriptor::BGRA8_SRGB.has_alpha());
        // The layout itself reports an alpha-position channel
        assert!(d.layout.has_alpha());
    }

    #[test]
    fn zero_size_buffer() {
        let buf = PixelBuffer::new(0, 0, PixelDescriptor::RGB8_SRGB);
        assert_eq!(buf.width(), 0);
        assert_eq!(buf.height(), 0);
        let slice = buf.as_slice();
        assert_eq!(slice.rows(), 0);
    }

    #[test]
    fn crop_empty() {
        let buf = PixelBuffer::new(4, 4, PixelDescriptor::RGB8_SRGB);
        let crop = buf.crop_view(0, 0, 0, 0);
        assert_eq!(crop.width(), 0);
        assert_eq!(crop.rows(), 0);
    }

    #[test]
    fn sub_rows_empty() {
        let buf = PixelBuffer::new(4, 4, PixelDescriptor::RGB8_SRGB);
        let sub = buf.rows(2, 0);
        assert_eq!(sub.rows(), 0);
    }
}
