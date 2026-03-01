//! Rich pixel format descriptors for pipeline operations.
//!
//! Provides [`ColorModel`], [`ByteOrder`], [`Subsampling`], [`YuvMatrix`] enums
//! and a [`PixelFormat`] struct that is a superset of [`PixelDescriptor`](crate::PixelDescriptor).

use crate::buffer::{AlphaMode, ChannelType, PixelDescriptor, TransferFunction};

// ---------------------------------------------------------------------------
// ColorModel
// ---------------------------------------------------------------------------

/// What the channels represent. Separate from channel count or byte order.
///
/// Native channel order per model:
/// - `Gray`: `[V]`
/// - `Rgb`:  `[R, G, B]` (or `[B, G, R]` when [`ByteOrder::Bgr`])
/// - `YCbCr`: `[Y, Cb, Cr]`
/// - `Oklab`: `[L, a, b]`
/// - `Xyz`: `[X, Y, Z]`
/// - `Lab`: `[L*, a*, b*]`
///
/// Alpha, when present, is always the last channel (interleaved)
/// or a separate plane (planar).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum ColorModel {
    /// Single-channel luminance/gray.
    Gray = 0,
    /// Three-channel red, green, blue.
    Rgb = 1,
    /// Three-channel luma + chroma (Y, Cb, Cr).
    YCbCr = 2,
    /// Oklab perceptual color space (L, a, b).
    Oklab = 3,
    /// CIE 1931 XYZ tristimulus.
    Xyz = 4,
    /// CIE L*a*b* perceptual color space.
    Lab = 5,
}

impl ColorModel {
    /// Number of color channels (excluding alpha).
    ///
    /// `Gray` = 1, all others = 3.
    #[inline]
    pub const fn color_channels(self) -> u8 {
        match self {
            Self::Gray => 1,
            Self::Rgb | Self::YCbCr | Self::Oklab | Self::Xyz | Self::Lab => 3,
        }
    }
}

// ---------------------------------------------------------------------------
// ByteOrder
// ---------------------------------------------------------------------------

/// RGB-family byte order. Only meaningful when color model is [`ColorModel::Rgb`].
/// Ignored for all other color models.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
#[repr(u8)]
pub enum ByteOrder {
    /// Standard order: R, G, B (+ A if present).
    #[default]
    Native = 0,
    /// Windows/DirectX order: B, G, R (+ A if present).
    Bgr = 1,
}

// ---------------------------------------------------------------------------
// Subsampling
// ---------------------------------------------------------------------------

/// Chroma subsampling (planar YCbCr).
///
/// Describes how chroma planes are downsampled relative to the luma plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
#[repr(u8)]
pub enum Subsampling {
    /// 4:4:4 — no subsampling, full resolution chroma.
    #[default]
    S444 = 0,
    /// 4:2:2 — horizontal half resolution chroma.
    S422 = 1,
    /// 4:2:0 — both horizontal and vertical half resolution chroma.
    S420 = 2,
    /// 4:1:1 — horizontal quarter resolution chroma (DV, some JPEG).
    S411 = 3,
}

impl Subsampling {
    /// Horizontal subsampling factor (1 = full, 2 = half, 4 = quarter).
    #[inline]
    pub const fn h_factor(self) -> u8 {
        match self {
            Self::S444 => 1,
            Self::S422 | Self::S420 => 2,
            Self::S411 => 4,
        }
    }

    /// Vertical subsampling factor (1 = full, 2 = half).
    #[inline]
    pub const fn v_factor(self) -> u8 {
        match self {
            Self::S420 => 2,
            Self::S444 | Self::S422 | Self::S411 => 1,
        }
    }

    /// Whether any subsampling is applied (not 4:4:4).
    #[inline]
    pub const fn is_subsampled(self) -> bool {
        !matches!(self, Self::S444)
    }
}

// ---------------------------------------------------------------------------
// YuvMatrix
// ---------------------------------------------------------------------------

/// YCbCr matrix coefficients.
///
/// Defines the luma/chroma weight matrix for RGB ↔ YCbCr conversion.
/// Only meaningful when color model is [`ColorModel::YCbCr`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
#[repr(u8)]
pub enum YuvMatrix {
    /// Identity / not applicable (RGB, Gray, Oklab, etc.).
    #[default]
    Identity = 0,
    /// BT.601: Y = 0.299R + 0.587G + 0.114B (JPEG, WebP, SD video).
    Bt601 = 1,
    /// BT.709: Y = 0.2126R + 0.7152G + 0.0722B (AVIF, HEIC, HD video).
    Bt709 = 2,
    /// BT.2020: Y = 0.2627R + 0.6780G + 0.0593B (4K/8K HDR).
    Bt2020 = 3,
}

impl YuvMatrix {
    /// RGB-to-Y luma coefficients `[Kr, Kg, Kb]`.
    ///
    /// Returns `[1.0, 0.0, 0.0]` for `Identity` (passthrough — Y = R).
    #[inline]
    pub const fn rgb_to_y_coeffs(self) -> [f64; 3] {
        match self {
            Self::Identity => [1.0, 0.0, 0.0],
            Self::Bt601 => [0.299, 0.587, 0.114],
            Self::Bt709 => [0.2126, 0.7152, 0.0722],
            Self::Bt2020 => [0.2627, 0.6780, 0.0593],
        }
    }

    /// Map CICP `matrix_coefficients` code to a [`YuvMatrix`].
    ///
    /// Returns `None` for unrecognized codes.
    pub const fn from_cicp(mc: u8) -> Option<Self> {
        match mc {
            0 => Some(Self::Identity),
            5 | 6 => Some(Self::Bt601),
            1 => Some(Self::Bt709),
            9 => Some(Self::Bt2020),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// PixelFormat
// ---------------------------------------------------------------------------

/// Rich pixel format descriptor for pipeline operations.
///
/// Superset of [`PixelDescriptor`] — adds subsampling, YUV matrix, and planar flag.
/// Color primaries are NOT tracked here — use [`ImageInfo`](crate::ImageInfo)/[`Cicp`](crate::Cicp)/ICC for that.
///
/// This type is `Copy` and small (~8 bytes + padding).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PixelFormat {
    /// Channel storage type (u8, u16, f16, f32).
    pub channel_type: ChannelType,
    /// Color model (Gray, RGB, YCbCr, Oklab, etc.).
    pub color_model: ColorModel,
    /// Alpha interpretation.
    pub alpha: AlphaMode,
    /// Transfer function (sRGB, linear, PQ, etc.).
    pub transfer: TransferFunction,
    /// RGB byte order (Native or Bgr). Ignored for non-RGB models.
    pub byte_order: ByteOrder,
    /// Chroma subsampling. Default: S444 (no subsampling).
    pub subsampling: Subsampling,
    /// YCbCr matrix coefficients. Default: Identity (not applicable).
    pub yuv_matrix: YuvMatrix,
    /// Whether data is stored in separate planes (true) or interleaved (false).
    pub planar: bool,
}

impl PixelFormat {
    // --- Named presets -------------------------------------------------------

    /// 8-bit sRGB RGBA with straight alpha (interleaved).
    pub const SRGB_RGBA_U8: Self = Self {
        channel_type: ChannelType::U8,
        color_model: ColorModel::Rgb,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Srgb,
        byte_order: ByteOrder::Native,
        subsampling: Subsampling::S444,
        yuv_matrix: YuvMatrix::Identity,
        planar: false,
    };

    /// 8-bit sRGB RGB, no alpha (interleaved).
    pub const SRGB_RGB_U8: Self = Self {
        channel_type: ChannelType::U8,
        color_model: ColorModel::Rgb,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Srgb,
        byte_order: ByteOrder::Native,
        subsampling: Subsampling::S444,
        yuv_matrix: YuvMatrix::Identity,
        planar: false,
    };

    /// Linear f32 RGBA with straight alpha (interleaved).
    pub const LINEAR_RGBA_F32: Self = Self {
        channel_type: ChannelType::F32,
        color_model: ColorModel::Rgb,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Linear,
        byte_order: ByteOrder::Native,
        subsampling: Subsampling::S444,
        yuv_matrix: YuvMatrix::Identity,
        planar: false,
    };

    /// Linear f32 RGB, no alpha (interleaved).
    pub const LINEAR_RGB_F32: Self = Self {
        channel_type: ChannelType::F32,
        color_model: ColorModel::Rgb,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Linear,
        byte_order: ByteOrder::Native,
        subsampling: Subsampling::S444,
        yuv_matrix: YuvMatrix::Identity,
        planar: false,
    };

    /// 8-bit sRGB grayscale, no alpha.
    pub const GRAY_U8: Self = Self {
        channel_type: ChannelType::U8,
        color_model: ColorModel::Gray,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Srgb,
        byte_order: ByteOrder::Native,
        subsampling: Subsampling::S444,
        yuv_matrix: YuvMatrix::Identity,
        planar: false,
    };

    /// Linear f32 grayscale, no alpha.
    pub const GRAY_F32: Self = Self {
        channel_type: ChannelType::F32,
        color_model: ColorModel::Gray,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Linear,
        byte_order: ByteOrder::Native,
        subsampling: Subsampling::S444,
        yuv_matrix: YuvMatrix::Identity,
        planar: false,
    };

    /// YCbCr 4:2:0 BT.601 u8 (planar, JPEG/WebP default).
    pub const YCBCR_420_BT601_U8: Self = Self {
        channel_type: ChannelType::U8,
        color_model: ColorModel::YCbCr,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Srgb,
        byte_order: ByteOrder::Native,
        subsampling: Subsampling::S420,
        yuv_matrix: YuvMatrix::Bt601,
        planar: true,
    };

    /// YCbCr 4:2:0 BT.709 u8 (planar, AVIF/HEIC default).
    pub const YCBCR_420_BT709_U8: Self = Self {
        channel_type: ChannelType::U8,
        color_model: ColorModel::YCbCr,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Srgb,
        byte_order: ByteOrder::Native,
        subsampling: Subsampling::S420,
        yuv_matrix: YuvMatrix::Bt709,
        planar: true,
    };

    /// Oklab f32 (interleaved L, a, b).
    pub const OKLAB_F32: Self = Self {
        channel_type: ChannelType::F32,
        color_model: ColorModel::Oklab,
        alpha: AlphaMode::None,
        transfer: TransferFunction::Linear,
        byte_order: ByteOrder::Native,
        subsampling: Subsampling::S444,
        yuv_matrix: YuvMatrix::Identity,
        planar: false,
    };

    /// 8-bit sRGB BGRA with straight alpha (interleaved).
    pub const BGRA_U8: Self = Self {
        channel_type: ChannelType::U8,
        color_model: ColorModel::Rgb,
        alpha: AlphaMode::Straight,
        transfer: TransferFunction::Srgb,
        byte_order: ByteOrder::Bgr,
        subsampling: Subsampling::S444,
        yuv_matrix: YuvMatrix::Identity,
        planar: false,
    };

    // --- Query methods -------------------------------------------------------

    /// Total number of channels (color + alpha).
    #[inline]
    pub const fn channels(self) -> u8 {
        self.color_model.color_channels() + self.has_alpha() as u8
    }

    /// Whether an alpha channel is present.
    #[inline]
    pub const fn has_alpha(self) -> bool {
        !matches!(self.alpha, AlphaMode::None)
    }

    /// Whether the transfer function is linear.
    #[inline]
    pub const fn is_linear(self) -> bool {
        matches!(self.transfer, TransferFunction::Linear)
    }

    /// Whether the transfer function is HDR (PQ or HLG).
    #[inline]
    pub const fn is_hdr(self) -> bool {
        matches!(self.transfer, TransferFunction::Pq | TransferFunction::Hlg)
    }

    /// Whether chroma is subsampled (not 4:4:4).
    #[inline]
    pub const fn is_subsampled(self) -> bool {
        self.subsampling.is_subsampled()
    }

    /// Whether data is stored in separate planes.
    #[inline]
    pub const fn is_planar(self) -> bool {
        self.planar
    }

    /// Whether alpha is premultiplied.
    #[inline]
    pub const fn is_premultiplied(self) -> bool {
        matches!(self.alpha, AlphaMode::Premultiplied)
    }

    /// Whether the transfer function is perceptual (sRGB or BT.709).
    #[inline]
    pub const fn is_perceptual(self) -> bool {
        matches!(
            self.transfer,
            TransferFunction::Srgb | TransferFunction::Bt709
        )
    }

    /// Whether the color model is YCbCr.
    #[inline]
    pub const fn is_ycbcr(self) -> bool {
        matches!(self.color_model, ColorModel::YCbCr)
    }
}

// ---------------------------------------------------------------------------
// PixelDescriptor ↔ PixelFormat conversions
// ---------------------------------------------------------------------------

impl From<PixelDescriptor> for PixelFormat {
    /// Convert a [`PixelDescriptor`] to a [`PixelFormat`] with pipeline defaults
    /// (S444, Identity matrix, interleaved).
    fn from(desc: PixelDescriptor) -> Self {
        let (color_model, byte_order) = desc.color_model_and_byte_order();
        Self {
            channel_type: desc.channel_type,
            color_model,
            alpha: desc.alpha,
            transfer: desc.transfer,
            byte_order,
            subsampling: Subsampling::S444,
            yuv_matrix: YuvMatrix::Identity,
            planar: false,
        }
    }
}

impl From<PixelFormat> for PixelDescriptor {
    /// Convert a [`PixelFormat`] to a [`PixelDescriptor`], dropping pipeline-only fields
    /// (subsampling, YUV matrix, planar flag).
    fn from(pf: PixelFormat) -> Self {
        Self::from_color_model_and_byte_order(
            pf.channel_type,
            pf.color_model,
            pf.alpha,
            pf.transfer,
            pf.byte_order,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_model_channels() {
        assert_eq!(ColorModel::Gray.color_channels(), 1);
        assert_eq!(ColorModel::Rgb.color_channels(), 3);
        assert_eq!(ColorModel::YCbCr.color_channels(), 3);
        assert_eq!(ColorModel::Oklab.color_channels(), 3);
        assert_eq!(ColorModel::Xyz.color_channels(), 3);
        assert_eq!(ColorModel::Lab.color_channels(), 3);
    }

    #[test]
    fn subsampling_factors() {
        assert_eq!(Subsampling::S444.h_factor(), 1);
        assert_eq!(Subsampling::S444.v_factor(), 1);
        assert_eq!(Subsampling::S422.h_factor(), 2);
        assert_eq!(Subsampling::S422.v_factor(), 1);
        assert_eq!(Subsampling::S420.h_factor(), 2);
        assert_eq!(Subsampling::S420.v_factor(), 2);
        assert_eq!(Subsampling::S411.h_factor(), 4);
        assert_eq!(Subsampling::S411.v_factor(), 1);
    }

    #[test]
    fn subsampling_is_subsampled() {
        assert!(!Subsampling::S444.is_subsampled());
        assert!(Subsampling::S422.is_subsampled());
        assert!(Subsampling::S420.is_subsampled());
        assert!(Subsampling::S411.is_subsampled());
    }

    #[test]
    fn yuv_matrix_coeffs() {
        let bt601 = YuvMatrix::Bt601.rgb_to_y_coeffs();
        assert!((bt601[0] - 0.299).abs() < 1e-6);
        assert!((bt601[1] - 0.587).abs() < 1e-6);
        assert!((bt601[2] - 0.114).abs() < 1e-6);

        let bt709 = YuvMatrix::Bt709.rgb_to_y_coeffs();
        assert!((bt709[0] - 0.2126).abs() < 1e-6);
    }

    #[test]
    fn yuv_matrix_from_cicp() {
        assert_eq!(YuvMatrix::from_cicp(0), Some(YuvMatrix::Identity));
        assert_eq!(YuvMatrix::from_cicp(1), Some(YuvMatrix::Bt709));
        assert_eq!(YuvMatrix::from_cicp(5), Some(YuvMatrix::Bt601));
        assert_eq!(YuvMatrix::from_cicp(6), Some(YuvMatrix::Bt601));
        assert_eq!(YuvMatrix::from_cicp(9), Some(YuvMatrix::Bt2020));
        assert_eq!(YuvMatrix::from_cicp(99), None);
    }

    #[test]
    fn pixel_format_presets() {
        assert_eq!(PixelFormat::SRGB_RGBA_U8.channels(), 4);
        assert!(PixelFormat::SRGB_RGBA_U8.has_alpha());
        assert!(!PixelFormat::SRGB_RGBA_U8.is_linear());
        assert!(!PixelFormat::SRGB_RGBA_U8.is_hdr());
        assert!(!PixelFormat::SRGB_RGBA_U8.is_planar());

        assert_eq!(PixelFormat::LINEAR_RGBA_F32.channels(), 4);
        assert!(PixelFormat::LINEAR_RGBA_F32.is_linear());

        assert_eq!(PixelFormat::GRAY_U8.channels(), 1);
        assert!(!PixelFormat::GRAY_U8.has_alpha());

        assert!(PixelFormat::YCBCR_420_BT601_U8.is_ycbcr());
        assert!(PixelFormat::YCBCR_420_BT601_U8.is_subsampled());
        assert!(PixelFormat::YCBCR_420_BT601_U8.is_planar());
        assert_eq!(PixelFormat::YCBCR_420_BT601_U8.channels(), 3);
    }

    #[test]
    fn pixel_format_hdr_queries() {
        let pq = PixelFormat {
            transfer: TransferFunction::Pq,
            ..PixelFormat::LINEAR_RGBA_F32
        };
        assert!(pq.is_hdr());
        assert!(!pq.is_linear());

        let hlg = PixelFormat {
            transfer: TransferFunction::Hlg,
            ..PixelFormat::LINEAR_RGBA_F32
        };
        assert!(hlg.is_hdr());
    }

    #[test]
    fn pixel_format_perceptual() {
        assert!(PixelFormat::SRGB_RGBA_U8.is_perceptual());
        assert!(!PixelFormat::LINEAR_RGBA_F32.is_perceptual());
    }

    #[test]
    fn pixel_format_from_descriptor() {
        let desc = PixelDescriptor::RGB8_SRGB;
        let pf = PixelFormat::from(desc);
        assert_eq!(pf.color_model, ColorModel::Rgb);
        assert_eq!(pf.byte_order, ByteOrder::Native);
        assert_eq!(pf.channel_type, ChannelType::U8);
        assert_eq!(pf.alpha, AlphaMode::None);
        assert_eq!(pf.transfer, TransferFunction::Srgb);
        assert_eq!(pf.subsampling, Subsampling::S444);
        assert_eq!(pf.yuv_matrix, YuvMatrix::Identity);
        assert!(!pf.planar);
    }

    #[test]
    fn pixel_format_to_descriptor_roundtrip() {
        let pf = PixelFormat::SRGB_RGBA_U8;
        let desc = PixelDescriptor::from(pf);
        assert_eq!(desc, PixelDescriptor::RGBA8_SRGB);

        let pf2 = PixelFormat::from(desc);
        assert_eq!(pf2.color_model, pf.color_model);
        assert_eq!(pf2.byte_order, pf.byte_order);
        assert_eq!(pf2.channel_type, pf.channel_type);
        assert_eq!(pf2.alpha, pf.alpha);
        assert_eq!(pf2.transfer, pf.transfer);
    }

    #[test]
    fn pixel_format_bgra_roundtrip() {
        let desc = PixelDescriptor::BGRA8_SRGB;
        let pf = PixelFormat::from(desc);
        assert_eq!(pf.color_model, ColorModel::Rgb);
        assert_eq!(pf.byte_order, ByteOrder::Bgr);
        let desc2 = PixelDescriptor::from(pf);
        assert_eq!(desc2, desc);
    }

    #[test]
    fn pixel_format_size() {
        // Verify it stays small (should be <=12 bytes)
        assert!(
            core::mem::size_of::<PixelFormat>() <= 12,
            "PixelFormat is {} bytes, expected <= 12",
            core::mem::size_of::<PixelFormat>()
        );
    }
}
