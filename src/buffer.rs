//! Opaque pixel buffer abstraction.
//!
//! Provides format-aware pixel storage that carries its own metadata.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;

#[cfg(feature = "codec")]
use imgref::ImgRef;
#[cfg(feature = "codec")]
use rgb::alt::BGRA;
#[cfg(feature = "codec")]
use rgb::{Gray, Rgb, Rgba};

use crate::color::{ColorContext, WorkingColorSpace};
#[cfg(feature = "codec")]
use crate::pixel::{GrayAlpha8, GrayAlpha16, GrayAlphaF32};

#[cfg(feature = "codec")]
use imgref::ImgVec;

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
    /// IEEE 754 half-precision float (2 bytes per channel).
    ///
    /// Used by AVIF, JXL, GPU pipelines. 10 mantissa bits provide
    /// ~3 decimal digits of precision (vs 23 bits / ~7 digits for f32).
    F16 = 5,
    /// Signed 16-bit integer (2 bytes per channel).
    ///
    /// Used for fixed-point processing pipelines (e.g., i16 resize kernels).
    I16 = 6,
}

impl ChannelType {
    /// Byte size of a single channel value.
    #[inline]
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    pub const fn byte_size(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 | Self::F16 | Self::I16 => 2,
            Self::F32 => 4,
            _ => 0,
        }
    }

    /// Whether this is [`U8`](Self::U8).
    #[inline]
    pub const fn is_u8(self) -> bool {
        matches!(self, Self::U8)
    }

    /// Whether this is [`U16`](Self::U16).
    #[inline]
    pub const fn is_u16(self) -> bool {
        matches!(self, Self::U16)
    }

    /// Whether this is [`F32`](Self::F32).
    #[inline]
    pub const fn is_f32(self) -> bool {
        matches!(self, Self::F32)
    }

    /// Whether this is [`F16`](Self::F16).
    #[inline]
    pub const fn is_f16(self) -> bool {
        matches!(self, Self::F16)
    }

    /// Whether this is [`I16`](Self::I16).
    #[inline]
    pub const fn is_i16(self) -> bool {
        matches!(self, Self::I16)
    }

    /// Whether this is an integer type ([`U8`](Self::U8), [`U16`](Self::U16), or [`I16`](Self::I16)).
    #[inline]
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    pub const fn is_integer(self) -> bool {
        matches!(self, Self::U8 | Self::U16 | Self::I16)
    }

    /// Whether this is a floating-point type ([`F32`](Self::F32) or [`F16`](Self::F16)).
    #[inline]
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    pub const fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F16)
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
///
/// Used inside `Option<AlphaMode>` on [`PixelDescriptor`]:
/// - `None` (the `Option`) means no alpha-position channel exists (RGB, Gray).
/// - `Some(AlphaMode::Undefined)` means padding bytes exist but have no semantic
///   value (RGBX, BGRX).
/// - `Some(AlphaMode::Straight | Premultiplied | Opaque)` means a real alpha channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum AlphaMode {
    /// Alpha bytes exist but values are undefined padding.
    ///
    /// Used for RGBX/BGRX — the 4th byte is in memory but should not be
    /// interpreted as alpha. Algorithms must not read or blend these bytes.
    Undefined = 0,
    /// Straight (unassociated) alpha — may contain transparent pixels.
    Straight = 1,
    /// Premultiplied (associated) alpha — may contain transparent pixels.
    ///
    /// Color channels are pre-scaled by alpha: `rgb_stored = rgb_actual * alpha`.
    Premultiplied = 2,
    /// Alpha channel present with all values fully opaque (255 for U8, 1.0 for F32).
    ///
    /// This is an optimization hint: the alpha bytes are valid and readable,
    /// but algorithms can skip alpha handling because no pixel has transparency.
    ///
    /// Unlike [`Undefined`](Self::Undefined), the alpha channel contains actual
    /// 255 values (not undefined padding). Data can be safely reinterpreted as
    /// RGBA without modification, and algorithms may read alpha to confirm
    /// opaqueness.
    ///
    /// Use cases:
    /// - JPEG decodes to RGBA with `a=255` — mark as `Opaque` so downstream
    ///   code can skip premultiply/unpremultiply and alpha blending.
    /// - After a full-image opacity check (`all(|p| p.a == 255)`), cache the
    ///   result by setting `Opaque` to avoid re-scanning.
    ///
    /// **Caller responsibility:** If you mutate alpha bytes in an `Opaque` buffer,
    /// you must clear the flag via `with_alpha_mode(Some(AlphaMode::Straight))`
    /// before passing the buffer to other code.
    Opaque = 3,
}

/// Signal range for pixel values.
///
/// Distinguishes full-range (0–255 for 8-bit) from narrow/limited range
/// (16–235 luma, 16–240 chroma for 8-bit) as defined by ITU-R BT.601/709/2020.
///
/// Video codecs (HEVC, AV1, VP9) commonly use narrow range internally.
/// Image codecs almost always use full range. The CICP `full_range` flag
/// maps directly to this enum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum SignalRange {
    /// Full range: 0–2^N-1 (e.g. 0–255 for 8-bit).
    #[default]
    Full = 0,
    /// Narrow (limited/studio) range: 16–235 luma, 16–240 chroma (for 8-bit).
    Narrow = 1,
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

/// Color primaries (CIE xy chromaticities of R, G, B).
///
/// Tracks the gamut of pixel data independently of transfer function.
/// This is critical for the cost model: P3→sRGB gamut clipping is lossy
/// even when both use sRGB transfer, and BT.2020→sRGB clips even more.
///
/// Discriminant values match CICP `ColorPrimaries` codes (ITU-T H.273).
///
/// Note: this does not replace [`Cicp`](crate::Cicp) — use `Cicp` when
/// you need full color description including matrix coefficients and range.
/// `ColorPrimaries` is for lightweight gamut tracking in `PixelDescriptor`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum ColorPrimaries {
    /// BT.709 / sRGB (CICP 1). Standard for SDR web content.
    #[default]
    Bt709 = 1,
    /// BT.2020 / BT.2100 (CICP 9). Wide gamut for HDR.
    Bt2020 = 9,
    /// Display P3 (CICP 12). Apple ecosystem, wide gamut SDR.
    DisplayP3 = 12,
    /// Primaries not known.
    Unknown = 255,
}

impl ColorPrimaries {
    /// Map a CICP `color_primaries` code to a [`ColorPrimaries`].
    ///
    /// Returns `None` for unrecognized codes. Use [`Unknown`](Self::Unknown)
    /// when you need a fallback value.
    pub const fn from_cicp(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Bt709),
            9 => Some(Self::Bt2020),
            12 => Some(Self::DisplayP3),
            _ => None,
        }
    }

    /// Convert to the CICP `color_primaries` code.
    ///
    /// Returns `None` for [`Unknown`](Self::Unknown) since there is no
    /// standard CICP code for "unknown primaries" (CICP uses 2="Unspecified").
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    pub const fn to_cicp(self) -> Option<u8> {
        match self {
            Self::Bt709 => Some(1),
            Self::Bt2020 => Some(9),
            Self::DisplayP3 => Some(12),
            Self::Unknown => None,
            _ => None,
        }
    }

    /// Whether `self` fully contains the gamut of `other`.
    ///
    /// Gamut hierarchy: BT.2020 ⊃ Display P3 ⊃ BT.709.
    /// Unknown is not contained by (or containing) anything.
    ///
    /// This is used by the cost model: converting from a wider gamut to
    /// a narrower one is lossy (clipping), while the reverse is lossless.
    pub const fn contains(self, other: Self) -> bool {
        // Self must be at least as wide as other.
        // Width order: Unknown=0, Bt709=1, DisplayP3=2, Bt2020=3
        self.gamut_width() >= other.gamut_width()
            && !matches!(self, Self::Unknown)
            && !matches!(other, Self::Unknown)
    }

    /// Internal gamut width ranking (larger = wider gamut).
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    const fn gamut_width(self) -> u8 {
        match self {
            Self::Bt709 => 1,
            Self::DisplayP3 => 2,
            Self::Bt2020 => 3,
            Self::Unknown => 0,
            _ => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Display impls for descriptor enums
// ---------------------------------------------------------------------------

impl fmt::Display for ChannelType {
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::U8 => f.write_str("U8"),
            Self::U16 => f.write_str("U16"),
            Self::F32 => f.write_str("F32"),
            Self::F16 => f.write_str("F16"),
            Self::I16 => f.write_str("I16"),
            _ => write!(f, "ChannelType({})", *self as u8),
        }
    }
}

impl fmt::Display for ChannelLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gray => f.write_str("Gray"),
            Self::GrayAlpha => f.write_str("GrayAlpha"),
            Self::Rgb => f.write_str("RGB"),
            Self::Rgba => f.write_str("RGBA"),
            Self::Bgra => f.write_str("BGRA"),
        }
    }
}

impl fmt::Display for AlphaMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undefined => f.write_str("undefined"),
            Self::Straight => f.write_str("straight"),
            Self::Premultiplied => f.write_str("premultiplied"),
            Self::Opaque => f.write_str("opaque"),
        }
    }
}

impl fmt::Display for TransferFunction {
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Linear => f.write_str("linear"),
            Self::Srgb => f.write_str("sRGB"),
            Self::Bt709 => f.write_str("BT.709"),
            Self::Pq => f.write_str("PQ"),
            Self::Hlg => f.write_str("HLG"),
            Self::Unknown => f.write_str("unknown"),
            _ => write!(f, "TransferFunction({})", *self as u8),
        }
    }
}

impl fmt::Display for ColorPrimaries {
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bt709 => f.write_str("BT.709"),
            Self::Bt2020 => f.write_str("BT.2020"),
            Self::DisplayP3 => f.write_str("Display P3"),
            Self::Unknown => f.write_str("unknown"),
            _ => write!(f, "ColorPrimaries({})", *self as u8),
        }
    }
}

impl fmt::Display for SignalRange {
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => f.write_str("full"),
            Self::Narrow => f.write_str("narrow"),
            _ => write!(f, "SignalRange({})", *self as u8),
        }
    }
}

// ---------------------------------------------------------------------------
// PixelFormat — match-friendly physical layout enum
// ---------------------------------------------------------------------------

/// Physical pixel layout for match-based format dispatch.
///
/// Captures channel type and layout only — NOT transfer function, primaries,
/// or signal range. Use this for `match`-based dispatch instead of chaining
/// `if descriptor.layout_compatible(...)` checks.
///
/// Every variant corresponds to a named [`PixelDescriptor`] constant.
/// Use [`PixelFormat::descriptor()`] to get the base descriptor, or
/// [`PixelDescriptor::pixel_format()`] to go the other direction.
///
/// # Mixed-alpha note
///
/// Today `ChannelType` describes all channels uniformly. The `rgb` crate
/// supports `Rgba<u16, u8>` (mixed alpha types), but no codec in this
/// ecosystem produces those. `PixelDescriptor` is `#[non_exhaustive]`, so
/// if mixed-alpha enters the ecosystem we can add an
/// `alpha_channel_type: Option<ChannelType>` field without breaking
/// existing code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelFormat {
    /// 8-bit RGB (3 bytes/pixel).
    Rgb8,
    /// 8-bit RGBA with alpha (4 bytes/pixel).
    Rgba8,
    /// 16-bit RGB (6 bytes/pixel).
    Rgb16,
    /// 16-bit RGBA with alpha (8 bytes/pixel).
    Rgba16,
    /// 32-bit float RGB (12 bytes/pixel).
    RgbF32,
    /// 32-bit float RGBA with alpha (16 bytes/pixel).
    RgbaF32,
    /// 8-bit grayscale (1 byte/pixel).
    Gray8,
    /// 16-bit grayscale (2 bytes/pixel).
    Gray16,
    /// 32-bit float grayscale (4 bytes/pixel).
    GrayF32,
    /// 8-bit grayscale + alpha (2 bytes/pixel).
    GrayA8,
    /// 16-bit grayscale + alpha (4 bytes/pixel).
    GrayA16,
    /// 32-bit float grayscale + alpha (8 bytes/pixel).
    GrayAF32,
    /// 8-bit BGRA with alpha (4 bytes/pixel).
    Bgra8,
    /// 8-bit RGBX — opaque RGBA, padding byte ignored (4 bytes/pixel).
    Rgbx8,
    /// 8-bit BGRX — opaque BGRA, padding byte ignored (4 bytes/pixel).
    Bgrx8,
}

impl PixelFormat {
    /// Base descriptor with `Unknown` transfer, BT.709 primaries, full range.
    ///
    /// The returned descriptor has the correct channel type, layout, and alpha
    /// mode for this format, but uses default metadata. Use
    /// [`PixelDescriptor::with_transfer()`] etc. to set specific metadata.
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    pub const fn descriptor(self) -> PixelDescriptor {
        match self {
            Self::Rgb8 => PixelDescriptor::RGB8,
            Self::Rgba8 => PixelDescriptor::RGBA8,
            Self::Rgb16 => PixelDescriptor::RGB16,
            Self::Rgba16 => PixelDescriptor::RGBA16,
            Self::RgbF32 => PixelDescriptor::RGBF32,
            Self::RgbaF32 => PixelDescriptor::RGBAF32,
            Self::Gray8 => PixelDescriptor::GRAY8,
            Self::Gray16 => PixelDescriptor::GRAY16,
            Self::GrayF32 => PixelDescriptor::GRAYF32,
            Self::GrayA8 => PixelDescriptor::GRAYA8,
            Self::GrayA16 => PixelDescriptor::GRAYA16,
            Self::GrayAF32 => PixelDescriptor::GRAYAF32,
            Self::Bgra8 => PixelDescriptor::BGRA8,
            Self::Rgbx8 => PixelDescriptor::RGBX8,
            Self::Bgrx8 => PixelDescriptor::BGRX8,
            // Safety net for future variants — return a reasonable default.
            _ => PixelDescriptor::RGB8,
        }
    }

    /// Short human-readable name: `"RGB8"`, `"RGBA16"`, `"GrayA8"`, etc.
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rgb8 => "RGB8",
            Self::Rgba8 => "RGBA8",
            Self::Rgb16 => "RGB16",
            Self::Rgba16 => "RGBA16",
            Self::RgbF32 => "RgbF32",
            Self::RgbaF32 => "RgbaF32",
            Self::Gray8 => "Gray8",
            Self::Gray16 => "Gray16",
            Self::GrayF32 => "GrayF32",
            Self::GrayA8 => "GrayA8",
            Self::GrayA16 => "GrayA16",
            Self::GrayAF32 => "GrayAF32",
            Self::Bgra8 => "BGRA8",
            Self::Rgbx8 => "RGBX8",
            Self::Bgrx8 => "BGRX8",
            _ => "Unknown",
        }
    }

    /// Bytes per pixel for this format.
    #[inline]
    pub const fn bytes_per_pixel(self) -> usize {
        self.descriptor().bytes_per_pixel()
    }

    /// Whether this format carries meaningful alpha data.
    #[inline]
    pub const fn has_alpha(self) -> bool {
        self.descriptor().has_alpha()
    }

    /// Whether this format is grayscale.
    #[inline]
    pub const fn is_grayscale(self) -> bool {
        self.descriptor().is_grayscale()
    }

    /// Channel storage type for this format.
    #[inline]
    pub const fn channel_type(self) -> ChannelType {
        self.descriptor().channel_type
    }

    /// Channel layout for this format.
    #[inline]
    pub const fn channel_layout(self) -> ChannelLayout {
        self.descriptor().layout
    }
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ---------------------------------------------------------------------------
// PixelDescriptor
// ---------------------------------------------------------------------------

/// Compact pixel format descriptor (5 bytes).
///
/// Describes the format of pixel data without carrying the data itself.
/// Used to tag [`PixelBuffer`] and [`PixelSlice`] with their format.
///
/// Tracks channel type, layout, alpha mode, transfer function, and color
/// primaries. The primaries field enables gamut-aware cost modeling: the
/// negotiation system can detect lossy gamut clipping (P3→sRGB) separately
/// from transfer function changes.
///
/// Note: this does not replace [`Cicp`](crate::Cicp) for full color
/// description (which also includes matrix coefficients and range).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub struct PixelDescriptor {
    /// Channel storage type (u8, u16, f16, i16, f32).
    pub channel_type: ChannelType,
    /// Channel layout (gray, RGB, RGBA, etc.).
    pub layout: ChannelLayout,
    /// Alpha interpretation.
    ///
    /// - `None` — layout has no alpha-position channel (RGB, Gray).
    /// - `Some(AlphaMode::Undefined)` — padding bytes exist but values are
    ///   unspecified (RGBX, BGRX).
    /// - `Some(AlphaMode::Straight | Premultiplied | Opaque)` — real alpha.
    pub alpha: Option<AlphaMode>,
    /// Transfer function (sRGB, linear, PQ, etc.).
    pub transfer: TransferFunction,
    /// Color primaries (gamut). Defaults to BT.709/sRGB.
    ///
    /// Used by the cost model to detect lossy gamut conversions.
    /// PQ/HLG content should typically use [`Bt2020`](ColorPrimaries::Bt2020).
    pub primaries: ColorPrimaries,
    /// Signal range (full vs narrow/limited).
    ///
    /// Defaults to [`Full`](SignalRange::Full). Video-origin content
    /// (HEVC, AV1, VP9) may use [`Narrow`](SignalRange::Narrow) range.
    /// Maps to the CICP `full_range` flag.
    pub signal_range: SignalRange,
}

impl PixelDescriptor {
    /// Create a pixel format descriptor with BT.709 primaries (default).
    ///
    /// For HDR or wide-gamut content, use [`new_full`](Self::new_full) or
    /// [`with_primaries`](Self::with_primaries) to set the correct primaries.
    pub const fn new(
        channel_type: ChannelType,
        layout: ChannelLayout,
        alpha: Option<AlphaMode>,
        transfer: TransferFunction,
    ) -> Self {
        Self {
            channel_type,
            layout,
            alpha,
            transfer,
            primaries: ColorPrimaries::Bt709,
            signal_range: SignalRange::Full,
        }
    }

    /// Create a pixel format descriptor with explicit primaries.
    pub const fn new_full(
        channel_type: ChannelType,
        layout: ChannelLayout,
        alpha: Option<AlphaMode>,
        transfer: TransferFunction,
        primaries: ColorPrimaries,
    ) -> Self {
        Self {
            channel_type,
            layout,
            alpha,
            transfer,
            primaries,
            signal_range: SignalRange::Full,
        }
    }

    // Named constants ---------------------------------------------------------

    /// 8-bit sRGB RGB.
    pub const RGB8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Rgb,
        alpha: None,
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 8-bit sRGB RGBA with straight alpha.
    pub const RGBA8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Rgba,
        alpha: Some(AlphaMode::Straight),
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 16-bit sRGB RGB.
    pub const RGB16_SRGB: Self = Self {
        channel_type: ChannelType::U16,
        layout: ChannelLayout::Rgb,
        alpha: None,
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 16-bit sRGB RGBA with straight alpha.
    pub const RGBA16_SRGB: Self = Self {
        channel_type: ChannelType::U16,
        layout: ChannelLayout::Rgba,
        alpha: Some(AlphaMode::Straight),
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// Linear-light f32 RGB.
    pub const RGBF32_LINEAR: Self = Self {
        channel_type: ChannelType::F32,
        layout: ChannelLayout::Rgb,
        alpha: None,
        transfer: TransferFunction::Linear,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// Linear-light f32 RGBA with straight alpha.
    pub const RGBAF32_LINEAR: Self = Self {
        channel_type: ChannelType::F32,
        layout: ChannelLayout::Rgba,
        alpha: Some(AlphaMode::Straight),
        transfer: TransferFunction::Linear,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 8-bit sRGB grayscale.
    pub const GRAY8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Gray,
        alpha: None,
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 16-bit sRGB grayscale.
    pub const GRAY16_SRGB: Self = Self {
        channel_type: ChannelType::U16,
        layout: ChannelLayout::Gray,
        alpha: None,
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// Linear-light f32 grayscale.
    pub const GRAYF32_LINEAR: Self = Self {
        channel_type: ChannelType::F32,
        layout: ChannelLayout::Gray,
        alpha: None,
        transfer: TransferFunction::Linear,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 8-bit sRGB grayscale with straight alpha.
    pub const GRAYA8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::GrayAlpha,
        alpha: Some(AlphaMode::Straight),
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 16-bit sRGB grayscale with straight alpha.
    pub const GRAYA16_SRGB: Self = Self {
        channel_type: ChannelType::U16,
        layout: ChannelLayout::GrayAlpha,
        alpha: Some(AlphaMode::Straight),
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// Linear-light f32 grayscale with straight alpha.
    pub const GRAYAF32_LINEAR: Self = Self {
        channel_type: ChannelType::F32,
        layout: ChannelLayout::GrayAlpha,
        alpha: Some(AlphaMode::Straight),
        transfer: TransferFunction::Linear,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 8-bit sRGB BGRA with straight alpha.
    pub const BGRA8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Bgra,
        alpha: Some(AlphaMode::Straight),
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 8-bit sRGB RGBX (padding byte, not alpha).
    ///
    /// Same memory layout as RGBA8 but the fourth byte is padding
    /// (`Some(AlphaMode::Undefined)`). Useful for SIMD-friendly 32-bit RGB
    /// processing without alpha semantics.
    pub const RGBX8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Rgba,
        alpha: Some(AlphaMode::Undefined),
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
    };

    /// 8-bit sRGB BGRX (padding byte, not alpha).
    ///
    /// Same memory layout as BGRA8 but the fourth byte is padding
    /// (`Some(AlphaMode::Undefined)`). Useful for Windows surfaces and DirectX
    /// where the alpha byte is present but meaningless.
    pub const BGRX8_SRGB: Self = Self {
        channel_type: ChannelType::U8,
        layout: ChannelLayout::Bgra,
        alpha: Some(AlphaMode::Undefined),
        transfer: TransferFunction::Srgb,
        primaries: ColorPrimaries::Bt709,
        signal_range: SignalRange::Full,
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

    /// 8-bit RGBX (opaque RGBA, padding byte ignored), transfer function unknown.
    pub const RGBX8: Self = Self::RGBX8_SRGB.with_transfer(TransferFunction::Unknown);

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
            primaries: self.primaries,
            signal_range: self.signal_range,
        }
    }

    /// Return a copy of this descriptor with different color primaries.
    ///
    /// Use when resolving from CICP metadata or converting to a wider/narrower gamut:
    ///
    /// ```
    /// # use zencodec_types::{PixelDescriptor, ColorPrimaries};
    /// let desc = PixelDescriptor::RGBF32_LINEAR;
    /// let p3 = desc.with_primaries(ColorPrimaries::DisplayP3);
    /// assert_eq!(p3.primaries, ColorPrimaries::DisplayP3);
    /// ```
    #[inline]
    pub const fn with_primaries(self, primaries: ColorPrimaries) -> Self {
        Self {
            channel_type: self.channel_type,
            layout: self.layout,
            alpha: self.alpha,
            transfer: self.transfer,
            primaries,
            signal_range: self.signal_range,
        }
    }

    /// Return a copy of this descriptor with a different signal range.
    ///
    /// Use when decoding video-origin content that uses narrow (limited) range:
    ///
    /// ```
    /// # use zencodec_types::{PixelDescriptor, SignalRange};
    /// let desc = PixelDescriptor::RGB8;
    /// let narrow = desc.with_signal_range(SignalRange::Narrow);
    /// assert!(narrow.is_narrow_range());
    /// ```
    #[inline]
    pub const fn with_signal_range(self, signal_range: SignalRange) -> Self {
        Self {
            channel_type: self.channel_type,
            layout: self.layout,
            alpha: self.alpha,
            transfer: self.transfer,
            primaries: self.primaries,
            signal_range,
        }
    }

    /// Whether this format uses narrow (limited/studio) signal range.
    ///
    /// Full-range uses 0–2^N-1. Narrow-range reserves headroom and footroom
    /// (e.g. 16–235 for 8-bit luma).
    #[inline]
    pub const fn is_narrow_range(self) -> bool {
        matches!(self.signal_range, SignalRange::Narrow)
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
    /// Returns `true` for [`Straight`](AlphaMode::Straight),
    /// [`Premultiplied`](AlphaMode::Premultiplied), and
    /// [`Opaque`](AlphaMode::Opaque) — all modes where the alpha channel
    /// contains valid, readable values.
    ///
    /// Returns `false` for `None` (no alpha-position channel) and
    /// `Some(AlphaMode::Undefined)` (padding byte).
    ///
    /// To distinguish opaque-alpha from transparent-alpha, match on
    /// [`alpha`](Self::alpha) directly:
    ///
    /// ```
    /// # use zencodec_types::{PixelDescriptor, AlphaMode};
    /// let desc = PixelDescriptor::RGBA8;
    /// match desc.alpha {
    ///     None => { /* no alpha channel at all */ }
    ///     Some(AlphaMode::Undefined) => { /* padding byte */ }
    ///     Some(AlphaMode::Opaque) => { /* alpha present, all 255 */ }
    ///     Some(AlphaMode::Straight | AlphaMode::Premultiplied) => { /* transparency */ }
    ///     Some(_) => { /* future variants */ }
    /// }
    /// ```
    #[inline]
    pub const fn has_alpha(self) -> bool {
        matches!(
            self.alpha,
            Some(AlphaMode::Straight | AlphaMode::Premultiplied | AlphaMode::Opaque)
        )
    }

    /// Whether this format is fully opaque (no transparency possible).
    ///
    /// Returns `true` when there is no alpha channel (`None`), the alpha
    /// bytes are undefined padding (`Undefined`), or alpha is all-255 (`Opaque`).
    ///
    /// Use this to decide whether alpha handling can be skipped entirely.
    #[inline]
    pub const fn is_opaque(self) -> bool {
        matches!(
            self.alpha,
            None | Some(AlphaMode::Undefined | AlphaMode::Opaque)
        )
    }

    /// Whether this format may contain transparent pixels.
    ///
    /// Returns `true` for [`Straight`](AlphaMode::Straight) and
    /// [`Premultiplied`](AlphaMode::Premultiplied).
    ///
    /// Returns `false` for `None`, `Undefined`, and `Opaque`.
    #[inline]
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    pub const fn may_have_transparency(self) -> bool {
        matches!(
            self.alpha,
            Some(AlphaMode::Straight | AlphaMode::Premultiplied)
        )
    }

    /// The alpha mode, if any.
    ///
    /// Returns `None` for formats without an alpha-position channel (RGB, Gray).
    /// Returns `Some(mode)` for formats with an alpha byte (including padding).
    #[inline]
    pub const fn alpha_mode(self) -> Option<AlphaMode> {
        self.alpha
    }

    /// Whether this format is grayscale (Gray or GrayAlpha layout).
    #[inline]
    pub const fn is_grayscale(self) -> bool {
        matches!(self.layout, ChannelLayout::Gray | ChannelLayout::GrayAlpha)
    }

    /// Whether this format uses BGR/BGRA channel order.
    #[inline]
    pub const fn is_bgr(self) -> bool {
        matches!(self.layout, ChannelLayout::Bgra)
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

    /// Returns the physical pixel format if it matches a known layout.
    ///
    /// Returns `None` for non-standard combinations (e.g., I16 Rgb,
    /// F16 Gray). The full descriptor is still valid — this just means
    /// there's no standard [`PixelFormat`] variant for it.
    #[allow(unreachable_patterns)] // non_exhaustive: future variants
    pub const fn pixel_format(&self) -> Option<PixelFormat> {
        match (self.channel_type, self.layout, self.alpha) {
            (ChannelType::U8, ChannelLayout::Rgb, None) => Some(PixelFormat::Rgb8),
            (
                ChannelType::U8,
                ChannelLayout::Rgba,
                Some(AlphaMode::Straight | AlphaMode::Premultiplied | AlphaMode::Opaque),
            ) => Some(PixelFormat::Rgba8),
            (ChannelType::U16, ChannelLayout::Rgb, None) => Some(PixelFormat::Rgb16),
            (
                ChannelType::U16,
                ChannelLayout::Rgba,
                Some(AlphaMode::Straight | AlphaMode::Premultiplied | AlphaMode::Opaque),
            ) => Some(PixelFormat::Rgba16),
            (ChannelType::F32, ChannelLayout::Rgb, None) => Some(PixelFormat::RgbF32),
            (
                ChannelType::F32,
                ChannelLayout::Rgba,
                Some(AlphaMode::Straight | AlphaMode::Premultiplied | AlphaMode::Opaque),
            ) => Some(PixelFormat::RgbaF32),
            (ChannelType::U8, ChannelLayout::Gray, None) => Some(PixelFormat::Gray8),
            (ChannelType::U16, ChannelLayout::Gray, None) => Some(PixelFormat::Gray16),
            (ChannelType::F32, ChannelLayout::Gray, None) => Some(PixelFormat::GrayF32),
            (
                ChannelType::U8,
                ChannelLayout::GrayAlpha,
                Some(AlphaMode::Straight | AlphaMode::Premultiplied | AlphaMode::Opaque),
            ) => Some(PixelFormat::GrayA8),
            (
                ChannelType::U16,
                ChannelLayout::GrayAlpha,
                Some(AlphaMode::Straight | AlphaMode::Premultiplied | AlphaMode::Opaque),
            ) => Some(PixelFormat::GrayA16),
            (
                ChannelType::F32,
                ChannelLayout::GrayAlpha,
                Some(AlphaMode::Straight | AlphaMode::Premultiplied | AlphaMode::Opaque),
            ) => Some(PixelFormat::GrayAF32),
            (
                ChannelType::U8,
                ChannelLayout::Bgra,
                Some(AlphaMode::Straight | AlphaMode::Premultiplied | AlphaMode::Opaque),
            ) => Some(PixelFormat::Bgra8),
            // RGBX: RGBA layout with undefined padding byte
            (ChannelType::U8, ChannelLayout::Rgba, None | Some(AlphaMode::Undefined)) => {
                Some(PixelFormat::Rgbx8)
            }
            // BGRX: BGRA layout with undefined padding byte
            (ChannelType::U8, ChannelLayout::Bgra, None | Some(AlphaMode::Undefined)) => {
                Some(PixelFormat::Bgrx8)
            }
            _ => None,
        }
    }
}

impl fmt::Display for PixelDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Base name from pixel_format, or fallback to "layout/channel_type"
        match self.pixel_format() {
            Some(pf) => f.write_str(pf.name())?,
            None => write!(f, "{}/{}", self.layout, self.channel_type)?,
        }
        // Transfer: shown if not Unknown
        if !matches!(self.transfer, TransferFunction::Unknown) {
            write!(f, "/{}", self.transfer)?;
        }
        // Primaries: shown if not BT.709 (the default)
        if !matches!(self.primaries, ColorPrimaries::Bt709) {
            write!(f, "/{}", self.primaries)?;
        }
        // Signal range: shown if Narrow
        if matches!(self.signal_range, SignalRange::Narrow) {
            write!(f, "/{}", self.signal_range)?;
        }
        // Alpha mode: shown for Undefined and Opaque (Straight is default for alpha
        // formats, None means no alpha channel — both obvious from base name)
        match self.alpha {
            Some(AlphaMode::Undefined) => f.write_str("/undefined")?,
            Some(AlphaMode::Opaque) => f.write_str("/opaque")?,
            _ => {}
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Padded pixel types (32-bit SIMD-friendly)
// ---------------------------------------------------------------------------

/// 32-bit RGB pixel with padding byte (RGBx).
///
/// Same memory layout as `Rgba<u8>` but the 4th byte is padding,
/// not alpha. Use this for SIMD-friendly 32-bit RGB processing
/// without alpha semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Rgbx {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Padding byte. Value is unspecified and should be ignored.
    pub x: u8,
}

/// 32-bit BGR pixel with padding byte (BGRx).
///
/// Same memory layout as `BGRA<u8>` but the 4th byte is padding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Bgrx {
    /// Blue channel.
    pub b: u8,
    /// Green channel.
    pub g: u8,
    /// Red channel.
    pub r: u8,
    /// Padding byte. Value is unspecified and should be ignored.
    pub x: u8,
}

// ---------------------------------------------------------------------------
// Pixel trait
// ---------------------------------------------------------------------------

/// Compile-time pixel format descriptor.
///
/// Implemented for pixel types to associate them with their
/// [`PixelDescriptor`]. This enables typed [`PixelSlice`] construction
/// where the type system enforces format correctness.
///
/// The trait is open (not sealed) — custom pixel types can implement it.
/// The `new_typed()` constructors include a compile-time assertion that
/// `size_of::<P>() == P::DESCRIPTOR.bytes_per_pixel()` to catch bad impls.
pub trait Pixel: Copy + 'static {
    /// The pixel format descriptor for this type.
    const DESCRIPTOR: PixelDescriptor;
}

impl Pixel for Rgbx {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::RGBX8;
}

impl Pixel for Bgrx {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::BGRX8;
}

#[cfg(feature = "codec")]
impl Pixel for Rgb<u8> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::RGB8;
}

#[cfg(feature = "codec")]
impl Pixel for Rgba<u8> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::RGBA8;
}

#[cfg(feature = "codec")]
impl Pixel for Gray<u8> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::GRAY8;
}

#[cfg(feature = "codec")]
impl Pixel for Rgb<u16> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::RGB16;
}

#[cfg(feature = "codec")]
impl Pixel for Rgba<u16> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::RGBA16;
}

#[cfg(feature = "codec")]
impl Pixel for Gray<u16> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::GRAY16;
}

#[cfg(feature = "codec")]
impl Pixel for Rgb<f32> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::RGBF32;
}

#[cfg(feature = "codec")]
impl Pixel for Rgba<f32> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::RGBAF32;
}

#[cfg(feature = "codec")]
impl Pixel for Gray<f32> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::GRAYF32;
}

#[cfg(feature = "codec")]
impl Pixel for BGRA<u8> {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::BGRA8;
}

#[cfg(feature = "codec")]
impl Pixel for GrayAlpha8 {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::GRAYA8;
}

#[cfg(feature = "codec")]
impl Pixel for GrayAlpha16 {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::GRAYA16;
}

#[cfg(feature = "codec")]
impl Pixel for GrayAlphaF32 {
    const DESCRIPTOR: PixelDescriptor = PixelDescriptor::GRAYAF32;
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
    /// Descriptor bytes_per_pixel mismatch in `reinterpret()`.
    ///
    /// The new descriptor has a different `bytes_per_pixel()` than the
    /// current one, so reinterpreting the buffer would be invalid.
    IncompatibleDescriptor,
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
            Self::IncompatibleDescriptor => {
                write!(f, "new descriptor has different bytes_per_pixel")
            }
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
/// The type parameter `P` tracks the pixel format at compile time:
/// - `PixelSlice<'a, Rgb<u8>>` — known to be RGB8 pixels
/// - `PixelSlice<'a>` (= `PixelSlice<'a, ()>`) — type-erased, format in descriptor
///
/// Use [`new_typed()`](PixelSlice::new_typed) to create a typed slice, or
/// [`erase()`](PixelSlice::erase) / [`try_typed()`](PixelSlice::try_typed)
/// to convert between typed and erased forms.
///
/// Optionally carries [`ColorContext`] and [`WorkingColorSpace`] to
/// track source color metadata and current color space through the
/// processing pipeline.
#[non_exhaustive]
pub struct PixelSlice<'a, P = ()> {
    data: &'a [u8],
    width: u32,
    rows: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    working_space: WorkingColorSpace,
    color: Option<Arc<ColorContext>>,
    _pixel: PhantomData<P>,
}

impl<'a> PixelSlice<'a> {
    /// Create a new pixel slice with validation.
    ///
    /// `stride_bytes` is the byte distance between the start of consecutive rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is too small, the stride is too small,
    /// or the data is not aligned for the channel type.
    pub fn new(
        data: &'a [u8],
        width: u32,
        rows: u32,
        stride_bytes: usize,
        descriptor: PixelDescriptor,
    ) -> Result<Self, BufferError> {
        validate_slice(
            data.len(),
            data.as_ptr(),
            width,
            rows,
            stride_bytes,
            &descriptor,
        )?;
        Ok(Self {
            data,
            width,
            rows,
            stride: stride_bytes,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
            _pixel: PhantomData,
        })
    }
}

impl<'a, P> PixelSlice<'a, P> {
    /// Erase the pixel type, returning a type-erased slice.
    ///
    /// This is a zero-cost operation that just changes the type parameter.
    pub fn erase(self) -> PixelSlice<'a> {
        PixelSlice {
            data: self.data,
            width: self.width,
            rows: self.rows,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color,
            _pixel: PhantomData,
        }
    }

    /// Try to reinterpret as a typed pixel slice.
    ///
    /// Succeeds if the descriptors are layout-compatible (same channel type
    /// and layout). Transfer function and alpha mode are metadata, not
    /// layout constraints.
    pub fn try_typed<Q: Pixel>(self) -> Option<PixelSlice<'a, Q>> {
        if self.descriptor.layout_compatible(&Q::DESCRIPTOR) {
            Some(PixelSlice {
                data: self.data,
                width: self.width,
                rows: self.rows,
                stride: self.stride,
                descriptor: self.descriptor,
                working_space: self.working_space,
                color: self.color,
                _pixel: PhantomData,
            })
        } else {
            None
        }
    }

    /// Replace the descriptor with a layout-compatible one.
    ///
    /// Use after a transform that changes pixel metadata without changing
    /// the buffer layout (e.g., transfer function change, alpha mode change,
    /// signal range expansion).
    ///
    /// For per-field updates, prefer the specific setters: [`with_transfer()`](Self::with_transfer),
    /// [`with_primaries()`](Self::with_primaries), [`with_signal_range()`](Self::with_signal_range),
    /// [`with_alpha_mode()`](Self::with_alpha_mode).
    ///
    /// # Panics
    ///
    /// Panics if the new descriptor is not layout-compatible (different
    /// `channel_type` or `layout`). Use [`reinterpret()`](Self::reinterpret)
    /// for genuine layout changes.
    #[inline]
    pub fn with_descriptor(mut self, descriptor: PixelDescriptor) -> Self {
        assert!(
            self.descriptor.layout_compatible(&descriptor),
            "with_descriptor() cannot change physical layout ({} -> {}); \
             use reinterpret() for layout changes",
            self.descriptor,
            descriptor
        );
        self.descriptor = descriptor;
        self
    }

    /// Reinterpret the buffer with a different physical layout.
    ///
    /// Unlike [`with_descriptor()`](Self::with_descriptor), this allows
    /// changing `channel_type` and `layout`. The new descriptor must have
    /// the same `bytes_per_pixel()` as the current one.
    ///
    /// Use cases: treating RGBA8 data as BGRA8, RGBX8 as RGBA8.
    pub fn reinterpret(mut self, descriptor: PixelDescriptor) -> Result<Self, BufferError> {
        if self.descriptor.bytes_per_pixel() != descriptor.bytes_per_pixel() {
            return Err(BufferError::IncompatibleDescriptor);
        }
        self.descriptor = descriptor;
        Ok(self)
    }

    /// Return a copy with a different transfer function.
    #[inline]
    pub fn with_transfer(mut self, tf: TransferFunction) -> Self {
        self.descriptor.transfer = tf;
        self
    }

    /// Return a copy with different color primaries.
    #[inline]
    pub fn with_primaries(mut self, cp: ColorPrimaries) -> Self {
        self.descriptor.primaries = cp;
        self
    }

    /// Return a copy with a different signal range.
    #[inline]
    pub fn with_signal_range(mut self, sr: SignalRange) -> Self {
        self.descriptor.signal_range = sr;
        self
    }

    /// Return a copy with a different alpha mode.
    #[inline]
    pub fn with_alpha_mode(mut self, am: Option<AlphaMode>) -> Self {
        self.descriptor.alpha = am;
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
    pub fn sub_rows(&self, y: u32, count: u32) -> PixelSlice<'_, P> {
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
                _pixel: PhantomData,
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
            _pixel: PhantomData,
        }
    }

    /// Zero-copy crop view. Adjusts the data pointer and width; stride
    /// remains the same as the parent.
    ///
    /// # Panics
    ///
    /// Panics if the crop region is out of bounds.
    pub fn crop_view(&self, x: u32, y: u32, w: u32, h: u32) -> PixelSlice<'_, P> {
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
                _pixel: PhantomData,
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
            _pixel: PhantomData,
        }
    }
}

impl<'a, P: Pixel> PixelSlice<'a, P> {
    /// Create a typed pixel slice.
    ///
    /// `stride_pixels` is the number of pixels per row (>= width).
    /// The byte stride is `stride_pixels * size_of::<P>()`.
    ///
    /// # Compile-time safety
    ///
    /// Includes a compile-time assertion that `size_of::<P>()` matches
    /// `P::DESCRIPTOR.bytes_per_pixel()`, catching bad `Pixel` impls.
    pub fn new_typed(
        data: &'a [u8],
        width: u32,
        rows: u32,
        stride_pixels: u32,
    ) -> Result<Self, BufferError> {
        const { assert!(core::mem::size_of::<P>() == P::DESCRIPTOR.bytes_per_pixel()) }
        let stride_bytes = stride_pixels as usize * core::mem::size_of::<P>();
        validate_slice(
            data.len(),
            data.as_ptr(),
            width,
            rows,
            stride_bytes,
            &P::DESCRIPTOR,
        )?;
        Ok(Self {
            data,
            width,
            rows,
            stride: stride_bytes,
            descriptor: P::DESCRIPTOR,
            working_space: WorkingColorSpace::Native,
            color: None,
            _pixel: PhantomData,
        })
    }
}

impl<'a, P: Pixel> PixelSliceMut<'a, P> {
    /// Create a typed mutable pixel slice.
    ///
    /// `stride_pixels` is the number of pixels per row (>= width).
    /// The byte stride is `stride_pixels * size_of::<P>()`.
    pub fn new_typed(
        data: &'a mut [u8],
        width: u32,
        rows: u32,
        stride_pixels: u32,
    ) -> Result<Self, BufferError> {
        const { assert!(core::mem::size_of::<P>() == P::DESCRIPTOR.bytes_per_pixel()) }
        let stride_bytes = stride_pixels as usize * core::mem::size_of::<P>();
        validate_slice(
            data.len(),
            data.as_ptr(),
            width,
            rows,
            stride_bytes,
            &P::DESCRIPTOR,
        )?;
        Ok(Self {
            data,
            width,
            rows,
            stride: stride_bytes,
            descriptor: P::DESCRIPTOR,
            working_space: WorkingColorSpace::Native,
            color: None,
            _pixel: PhantomData,
        })
    }
}

impl<P: Pixel> PixelBuffer<P> {
    /// Allocate a typed zero-filled buffer for the given dimensions.
    ///
    /// The descriptor is derived from `P::DESCRIPTOR`.
    pub fn new_typed(width: u32, height: u32) -> Self {
        const { assert!(core::mem::size_of::<P>() == P::DESCRIPTOR.bytes_per_pixel()) }
        let descriptor = P::DESCRIPTOR;
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
            _pixel: PhantomData,
        }
    }
}

#[cfg(feature = "codec")]
impl<P: Pixel + bytemuck::NoUninit> PixelBuffer<P> {
    /// Construct from a typed pixel `Vec`.
    ///
    /// Zero-copy when `P` has alignment 1 (u8-component types like `Rgb<u8>`).
    /// Copies the data for types with higher alignment (`Rgb<u16>`, `Rgb<f32>`, etc.)
    /// because `Vec` tracks allocation alignment and `Vec<u8>` requires alignment 1.
    ///
    /// # Errors
    ///
    /// Returns [`BufferError::InvalidDimensions`] if `pixels.len() != width * height`.
    pub fn from_pixels(pixels: Vec<P>, width: u32, height: u32) -> Result<Self, BufferError> {
        const { assert!(core::mem::size_of::<P>() == P::DESCRIPTOR.bytes_per_pixel()) }
        let expected = width as usize * height as usize;
        if pixels.len() != expected {
            return Err(BufferError::InvalidDimensions);
        }
        let descriptor = P::DESCRIPTOR;
        let stride = descriptor.aligned_stride(width);
        let data: Vec<u8> = pixels_to_bytes(pixels);
        Ok(Self {
            data,
            offset: 0,
            width,
            height,
            stride,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
            _pixel: PhantomData,
        })
    }

    /// Construct from a typed `ImgVec`.
    ///
    /// Zero-copy when `P` has alignment 1 (u8-component types).
    /// Copies for higher-alignment types.
    pub fn from_imgvec(img: ImgVec<P>) -> Self {
        const { assert!(core::mem::size_of::<P>() == P::DESCRIPTOR.bytes_per_pixel()) }
        let width = img.width() as u32;
        let height = img.height() as u32;
        let stride_pixels = img.stride();
        let descriptor = P::DESCRIPTOR;
        let stride_bytes = stride_pixels * core::mem::size_of::<P>();
        let (buf, ..) = img.into_contiguous_buf();
        let data: Vec<u8> = pixels_to_bytes(buf);
        Self {
            data,
            offset: 0,
            width,
            height,
            stride: stride_bytes,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
            _pixel: PhantomData,
        }
    }
}

#[cfg(feature = "codec")]
impl<P: Pixel + bytemuck::AnyBitPattern + bytemuck::NoUninit> PixelBuffer<P> {
    /// Borrow the buffer as an [`ImgRef`].
    ///
    /// Zero-copy: reinterprets the raw bytes as typed pixels via
    /// [`bytemuck::cast_slice`].
    ///
    /// # Panics
    ///
    /// Panics if the stride is not pixel-aligned (always succeeds for
    /// buffers created via `new_typed()`, `from_pixels()`, or `from_imgvec()`).
    pub fn as_imgref(&self) -> ImgRef<'_, P> {
        let total_bytes = if self.height == 0 {
            0
        } else {
            (self.height as usize - 1) * self.stride
                + self.width as usize * core::mem::size_of::<P>()
        };
        let data = &self.data[self.offset..self.offset + total_bytes];
        let pixels: &[P] = bytemuck::cast_slice(data);
        let stride_px = self.stride / core::mem::size_of::<P>();
        imgref::Img::new_stride(pixels, self.width as usize, self.height as usize, stride_px)
    }

    /// Borrow the buffer as a mutable [`ImgRefMut`](imgref::ImgRefMut).
    ///
    /// Zero-copy: reinterprets the raw bytes as typed pixels.
    pub fn as_imgref_mut(&mut self) -> imgref::ImgRefMut<'_, P> {
        let total_bytes = if self.height == 0 {
            0
        } else {
            (self.height as usize - 1) * self.stride
                + self.width as usize * core::mem::size_of::<P>()
        };
        let offset = self.offset;
        let data = &mut self.data[offset..offset + total_bytes];
        let pixels: &mut [P] = bytemuck::cast_slice_mut(data);
        let stride_px = self.stride / core::mem::size_of::<P>();
        imgref::Img::new_stride(pixels, self.width as usize, self.height as usize, stride_px)
    }
}

/// Type-erased construction and `try_as_imgref` for PixelBuffer.
#[cfg(feature = "codec")]
impl PixelBuffer {
    /// Zero-copy construction from typed pixels, returning an erased `PixelBuffer`.
    ///
    /// Equivalent to `PixelBuffer::<P>::from_pixels(pixels, w, h)?.into()` but
    /// avoids the intermediate typed `PixelBuffer`.
    ///
    /// # Errors
    ///
    /// Returns [`BufferError::InvalidDimensions`] if `pixels.len() != width * height`.
    pub fn from_pixels_erased<P: Pixel + bytemuck::NoUninit>(
        pixels: Vec<P>,
        width: u32,
        height: u32,
    ) -> Result<Self, BufferError> {
        PixelBuffer::<P>::from_pixels(pixels, width, height).map(PixelBuffer::from)
    }

    /// Try to borrow the buffer as a typed [`ImgRef`].
    ///
    /// Returns `None` if the descriptor is not layout-compatible with `P`.
    pub fn try_as_imgref<P: Pixel + bytemuck::AnyBitPattern>(&self) -> Option<ImgRef<'_, P>> {
        if !self.descriptor.layout_compatible(&P::DESCRIPTOR) {
            return None;
        }
        let pixel_size = core::mem::size_of::<P>();
        if pixel_size == 0 || !self.stride.is_multiple_of(pixel_size) {
            return None;
        }
        let total_bytes = if self.height == 0 {
            0
        } else {
            (self.height as usize - 1) * self.stride + self.width as usize * pixel_size
        };
        let data = &self.data[self.offset..self.offset + total_bytes];
        let pixels: &[P] = bytemuck::cast_slice(data);
        let stride_px = self.stride / pixel_size;
        Some(imgref::Img::new_stride(
            pixels,
            self.width as usize,
            self.height as usize,
            stride_px,
        ))
    }

    /// Try to borrow the buffer as a typed mutable [`ImgRefMut`](imgref::ImgRefMut).
    ///
    /// Returns `None` if the descriptor is not layout-compatible with `P`.
    pub fn try_as_imgref_mut<P: Pixel + bytemuck::AnyBitPattern + bytemuck::NoUninit>(
        &mut self,
    ) -> Option<imgref::ImgRefMut<'_, P>> {
        if !self.descriptor.layout_compatible(&P::DESCRIPTOR) {
            return None;
        }
        let pixel_size = core::mem::size_of::<P>();
        if pixel_size == 0 || !self.stride.is_multiple_of(pixel_size) {
            return None;
        }
        let total_bytes = if self.height == 0 {
            0
        } else {
            (self.height as usize - 1) * self.stride + self.width as usize * pixel_size
        };
        let offset = self.offset;
        let data = &mut self.data[offset..offset + total_bytes];
        let pixels: &mut [P] = bytemuck::cast_slice_mut(data);
        let stride_px = self.stride / pixel_size;
        Some(imgref::Img::new_stride(
            pixels,
            self.width as usize,
            self.height as usize,
            stride_px,
        ))
    }
}

impl<P> fmt::Debug for PixelSlice<'_, P> {
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
/// The type parameter `P` tracks pixel format at compile time.
#[non_exhaustive]
pub struct PixelSliceMut<'a, P = ()> {
    data: &'a mut [u8],
    width: u32,
    rows: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    working_space: WorkingColorSpace,
    color: Option<Arc<ColorContext>>,
    _pixel: PhantomData<P>,
}

impl<'a> PixelSliceMut<'a> {
    /// Create a new mutable pixel slice with validation.
    ///
    /// `stride_bytes` is the byte distance between the start of consecutive rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is too small, the stride is too small,
    /// or the data is not aligned for the channel type.
    pub fn new(
        data: &'a mut [u8],
        width: u32,
        rows: u32,
        stride_bytes: usize,
        descriptor: PixelDescriptor,
    ) -> Result<Self, BufferError> {
        validate_slice(
            data.len(),
            data.as_ptr(),
            width,
            rows,
            stride_bytes,
            &descriptor,
        )?;
        Ok(Self {
            data,
            width,
            rows,
            stride: stride_bytes,
            descriptor,
            working_space: WorkingColorSpace::Native,
            color: None,
            _pixel: PhantomData,
        })
    }
}

impl<'a, P> PixelSliceMut<'a, P> {
    /// Erase the pixel type, returning a type-erased mutable slice.
    pub fn erase(self) -> PixelSliceMut<'a> {
        PixelSliceMut {
            data: self.data,
            width: self.width,
            rows: self.rows,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color,
            _pixel: PhantomData,
        }
    }

    /// Try to reinterpret as a typed mutable pixel slice.
    ///
    /// Succeeds if the descriptors are layout-compatible.
    pub fn try_typed<Q: Pixel>(self) -> Option<PixelSliceMut<'a, Q>> {
        if self.descriptor.layout_compatible(&Q::DESCRIPTOR) {
            Some(PixelSliceMut {
                data: self.data,
                width: self.width,
                rows: self.rows,
                stride: self.stride,
                descriptor: self.descriptor,
                working_space: self.working_space,
                color: self.color,
                _pixel: PhantomData,
            })
        } else {
            None
        }
    }

    /// Replace the descriptor with a layout-compatible one.
    ///
    /// See [`PixelSlice::with_descriptor()`] for details.
    #[inline]
    pub fn with_descriptor(mut self, descriptor: PixelDescriptor) -> Self {
        assert!(
            self.descriptor.layout_compatible(&descriptor),
            "with_descriptor() cannot change physical layout ({} -> {}); \
             use reinterpret() for layout changes",
            self.descriptor,
            descriptor
        );
        self.descriptor = descriptor;
        self
    }

    /// Reinterpret the buffer with a different physical layout.
    ///
    /// See [`PixelSlice::reinterpret()`] for details.
    pub fn reinterpret(mut self, descriptor: PixelDescriptor) -> Result<Self, BufferError> {
        if self.descriptor.bytes_per_pixel() != descriptor.bytes_per_pixel() {
            return Err(BufferError::IncompatibleDescriptor);
        }
        self.descriptor = descriptor;
        Ok(self)
    }

    /// Return a copy with a different transfer function.
    #[inline]
    pub fn with_transfer(mut self, tf: TransferFunction) -> Self {
        self.descriptor.transfer = tf;
        self
    }

    /// Return a copy with different color primaries.
    #[inline]
    pub fn with_primaries(mut self, cp: ColorPrimaries) -> Self {
        self.descriptor.primaries = cp;
        self
    }

    /// Return a copy with a different signal range.
    #[inline]
    pub fn with_signal_range(mut self, sr: SignalRange) -> Self {
        self.descriptor.signal_range = sr;
        self
    }

    /// Return a copy with a different alpha mode.
    #[inline]
    pub fn with_alpha_mode(mut self, am: Option<AlphaMode>) -> Self {
        self.descriptor.alpha = am;
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
    pub fn sub_rows_mut(&mut self, y: u32, count: u32) -> PixelSliceMut<'_, P> {
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
                _pixel: PhantomData,
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
            _pixel: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------------
// In-place 32-bit pixel format conversions on PixelSliceMut
// ---------------------------------------------------------------------------

/// Helper: iterate over pixel rows, calling `f` on each 4-byte pixel.
fn for_each_pixel_4bpp(
    data: &mut [u8],
    width: u32,
    rows: u32,
    stride: usize,
    mut f: impl FnMut(&mut [u8; 4]),
) {
    let row_bytes = width as usize * 4;
    for y in 0..rows as usize {
        let row_start = y * stride;
        let row = &mut data[row_start..row_start + row_bytes];
        for chunk in row.chunks_exact_mut(4) {
            let px: &mut [u8; 4] = chunk.try_into().unwrap();
            f(px);
        }
    }
}

impl<'a> PixelSliceMut<'a, Rgbx> {
    /// Byte-swap R↔B channels in place, converting to BGRX.
    pub fn swap_to_bgrx(self) -> PixelSliceMut<'a, Bgrx> {
        let width = self.width;
        let rows = self.rows;
        let stride = self.stride;
        let ws = self.working_space;
        let color = self.color;
        let data = self.data;
        for_each_pixel_4bpp(data, width, rows, stride, |px| {
            px.swap(0, 2);
        });
        PixelSliceMut {
            data,
            width,
            rows,
            stride,
            descriptor: PixelDescriptor::BGRX8_SRGB,
            working_space: ws,
            color,
            _pixel: PhantomData,
        }
    }
}

#[cfg(feature = "codec")]
impl<'a> PixelSliceMut<'a, Rgbx> {
    /// Upgrade to RGBA by setting all padding bytes to 255 (fully opaque).
    pub fn upgrade_to_rgba(self) -> PixelSliceMut<'a, Rgba<u8>> {
        let width = self.width;
        let rows = self.rows;
        let stride = self.stride;
        let ws = self.working_space;
        let color = self.color;
        let data = self.data;
        for_each_pixel_4bpp(data, width, rows, stride, |px| {
            px[3] = 255;
        });
        PixelSliceMut {
            data,
            width,
            rows,
            stride,
            descriptor: PixelDescriptor::RGBA8_SRGB,
            working_space: ws,
            color,
            _pixel: PhantomData,
        }
    }
}

impl<'a> PixelSliceMut<'a, Bgrx> {
    /// Byte-swap B↔R channels in place, converting to RGBX.
    pub fn swap_to_rgbx(self) -> PixelSliceMut<'a, Rgbx> {
        let width = self.width;
        let rows = self.rows;
        let stride = self.stride;
        let ws = self.working_space;
        let color = self.color;
        let data = self.data;
        for_each_pixel_4bpp(data, width, rows, stride, |px| {
            px.swap(0, 2);
        });
        PixelSliceMut {
            data,
            width,
            rows,
            stride,
            descriptor: PixelDescriptor::RGBX8_SRGB,
            working_space: ws,
            color,
            _pixel: PhantomData,
        }
    }
}

#[cfg(feature = "codec")]
impl<'a> PixelSliceMut<'a, Bgrx> {
    /// Upgrade to BGRA by setting all padding bytes to 255 (fully opaque).
    pub fn upgrade_to_bgra(self) -> PixelSliceMut<'a, BGRA<u8>> {
        let width = self.width;
        let rows = self.rows;
        let stride = self.stride;
        let ws = self.working_space;
        let color = self.color;
        let data = self.data;
        for_each_pixel_4bpp(data, width, rows, stride, |px| {
            px[3] = 255;
        });
        PixelSliceMut {
            data,
            width,
            rows,
            stride,
            descriptor: PixelDescriptor::BGRA8_SRGB,
            working_space: ws,
            color,
            _pixel: PhantomData,
        }
    }
}

#[cfg(feature = "codec")]
impl<'a> PixelSliceMut<'a, Rgba<u8>> {
    /// Matte alpha against a solid RGB background, producing RGBX.
    ///
    /// Each pixel is composited: `out = src * alpha/255 + bg * (255 - alpha)/255`.
    /// The alpha byte becomes padding.
    pub fn matte_to_rgbx(self, bg: Rgb<u8>) -> PixelSliceMut<'a, Rgbx> {
        let width = self.width;
        let rows = self.rows;
        let stride = self.stride;
        let ws = self.working_space;
        let color = self.color;
        let data = self.data;
        for_each_pixel_4bpp(data, width, rows, stride, |px| {
            let a = px[3] as u16;
            let inv_a = 255 - a;
            px[0] = ((px[0] as u16 * a + bg.r as u16 * inv_a + 127) / 255) as u8;
            px[1] = ((px[1] as u16 * a + bg.g as u16 * inv_a + 127) / 255) as u8;
            px[2] = ((px[2] as u16 * a + bg.b as u16 * inv_a + 127) / 255) as u8;
            px[3] = 0;
        });
        PixelSliceMut {
            data,
            width,
            rows,
            stride,
            descriptor: PixelDescriptor::RGBX8_SRGB,
            working_space: ws,
            color,
            _pixel: PhantomData,
        }
    }

    /// Strip alpha to RGBX without compositing (just mark as padding).
    ///
    /// The alpha byte value is preserved in memory but semantically ignored.
    /// Use when you know alpha is already 255 or don't care about the values.
    pub fn strip_alpha_to_rgbx(self) -> PixelSliceMut<'a, Rgbx> {
        PixelSliceMut {
            data: self.data,
            width: self.width,
            rows: self.rows,
            stride: self.stride,
            descriptor: PixelDescriptor::RGBX8_SRGB,
            working_space: self.working_space,
            color: self.color,
            _pixel: PhantomData,
        }
    }

    /// Byte-swap R↔B channels in place, converting to BGRA.
    pub fn swap_to_bgra(self) -> PixelSliceMut<'a, BGRA<u8>> {
        let width = self.width;
        let rows = self.rows;
        let stride = self.stride;
        let ws = self.working_space;
        let color = self.color;
        let data = self.data;
        for_each_pixel_4bpp(data, width, rows, stride, |px| {
            px.swap(0, 2);
        });
        PixelSliceMut {
            data,
            width,
            rows,
            stride,
            descriptor: PixelDescriptor::BGRA8_SRGB,
            working_space: ws,
            color,
            _pixel: PhantomData,
        }
    }
}

#[cfg(feature = "codec")]
impl<'a> PixelSliceMut<'a, BGRA<u8>> {
    /// Matte alpha against a solid RGB background, producing BGRX.
    ///
    /// Each pixel is composited: `out = src * alpha/255 + bg * (255 - alpha)/255`.
    /// The alpha byte becomes padding.
    pub fn matte_to_bgrx(self, bg: Rgb<u8>) -> PixelSliceMut<'a, Bgrx> {
        let width = self.width;
        let rows = self.rows;
        let stride = self.stride;
        let ws = self.working_space;
        let color = self.color;
        let data = self.data;
        for_each_pixel_4bpp(data, width, rows, stride, |px| {
            let a = px[3] as u16;
            let inv_a = 255 - a;
            // BGRA layout: [B, G, R, A]
            px[0] = ((px[0] as u16 * a + bg.b as u16 * inv_a + 127) / 255) as u8;
            px[1] = ((px[1] as u16 * a + bg.g as u16 * inv_a + 127) / 255) as u8;
            px[2] = ((px[2] as u16 * a + bg.r as u16 * inv_a + 127) / 255) as u8;
            px[3] = 0;
        });
        PixelSliceMut {
            data,
            width,
            rows,
            stride,
            descriptor: PixelDescriptor::BGRX8_SRGB,
            working_space: ws,
            color,
            _pixel: PhantomData,
        }
    }

    /// Strip alpha to BGRX without compositing (just mark as padding).
    pub fn strip_alpha_to_bgrx(self) -> PixelSliceMut<'a, Bgrx> {
        PixelSliceMut {
            data: self.data,
            width: self.width,
            rows: self.rows,
            stride: self.stride,
            descriptor: PixelDescriptor::BGRX8_SRGB,
            working_space: self.working_space,
            color: self.color,
            _pixel: PhantomData,
        }
    }

    /// Byte-swap B↔R channels in place, converting to RGBA.
    pub fn swap_to_rgba(self) -> PixelSliceMut<'a, Rgba<u8>> {
        let width = self.width;
        let rows = self.rows;
        let stride = self.stride;
        let ws = self.working_space;
        let color = self.color;
        let data = self.data;
        for_each_pixel_4bpp(data, width, rows, stride, |px| {
            px.swap(0, 2);
        });
        PixelSliceMut {
            data,
            width,
            rows,
            stride,
            descriptor: PixelDescriptor::RGBA8_SRGB,
            working_space: ws,
            color,
            _pixel: PhantomData,
        }
    }
}

impl<P> fmt::Debug for PixelSliceMut<'_, P> {
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
///
/// The type parameter `P` tracks pixel format at compile time, same as
/// [`PixelSlice`].
#[non_exhaustive]
pub struct PixelBuffer<P = ()> {
    data: Vec<u8>,
    /// Byte offset from `data` start to the first aligned pixel.
    offset: usize,
    width: u32,
    height: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    working_space: WorkingColorSpace,
    color: Option<Arc<ColorContext>>,
    _pixel: PhantomData<P>,
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
            _pixel: PhantomData,
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
            _pixel: PhantomData,
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
            _pixel: PhantomData,
        })
    }
}

impl<P> PixelBuffer<P> {
    /// Erase the pixel type, returning a type-erased buffer.
    pub fn erase(self) -> PixelBuffer {
        PixelBuffer {
            data: self.data,
            offset: self.offset,
            width: self.width,
            height: self.height,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color,
            _pixel: PhantomData,
        }
    }

    /// Try to reinterpret as a typed pixel buffer.
    ///
    /// Succeeds if the descriptors are layout-compatible.
    pub fn try_typed<Q: Pixel>(self) -> Option<PixelBuffer<Q>> {
        if self.descriptor.layout_compatible(&Q::DESCRIPTOR) {
            Some(PixelBuffer {
                data: self.data,
                offset: self.offset,
                width: self.width,
                height: self.height,
                stride: self.stride,
                descriptor: self.descriptor,
                working_space: self.working_space,
                color: self.color,
                _pixel: PhantomData,
            })
        } else {
            None
        }
    }

    /// Replace the descriptor with a layout-compatible one.
    ///
    /// See [`PixelSlice::with_descriptor()`] for details.
    #[inline]
    pub fn with_descriptor(mut self, descriptor: PixelDescriptor) -> Self {
        assert!(
            self.descriptor.layout_compatible(&descriptor),
            "with_descriptor() cannot change physical layout ({} -> {}); \
             use reinterpret() for layout changes",
            self.descriptor,
            descriptor
        );
        self.descriptor = descriptor;
        self
    }

    /// Reinterpret the buffer with a different physical layout.
    ///
    /// See [`PixelSlice::reinterpret()`] for details.
    pub fn reinterpret(mut self, descriptor: PixelDescriptor) -> Result<Self, BufferError> {
        if self.descriptor.bytes_per_pixel() != descriptor.bytes_per_pixel() {
            return Err(BufferError::IncompatibleDescriptor);
        }
        self.descriptor = descriptor;
        Ok(self)
    }

    /// Return a copy with a different transfer function.
    #[inline]
    pub fn with_transfer(mut self, tf: TransferFunction) -> Self {
        self.descriptor.transfer = tf;
        self
    }

    /// Return a copy with different color primaries.
    #[inline]
    pub fn with_primaries(mut self, cp: ColorPrimaries) -> Self {
        self.descriptor.primaries = cp;
        self
    }

    /// Return a copy with a different signal range.
    #[inline]
    pub fn with_signal_range(mut self, sr: SignalRange) -> Self {
        self.descriptor.signal_range = sr;
        self
    }

    /// Return a copy with a different alpha mode.
    #[inline]
    pub fn with_alpha_mode(mut self, am: Option<AlphaMode>) -> Self {
        self.descriptor.alpha = am;
        self
    }

    /// Whether this buffer carries meaningful alpha data.
    #[inline]
    pub fn has_alpha(&self) -> bool {
        self.descriptor.has_alpha()
    }

    /// Whether this buffer is grayscale (Gray or GrayAlpha layout).
    #[inline]
    pub fn is_grayscale(&self) -> bool {
        self.descriptor.is_grayscale()
    }

    /// Consume the buffer and return the backing `Vec<u8>` for pool reuse.
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    /// Copy pixel data to a new contiguous byte `Vec` without stride padding.
    ///
    /// Returns exactly `width * height * bytes_per_pixel` bytes in row-major order.
    /// For buffers already tightly packed (stride == width * bpp), this is a single memcpy.
    /// For padded buffers, this strips the padding row by row.
    pub fn copy_to_contiguous_bytes(&self) -> Vec<u8> {
        let bpp = self.descriptor.bytes_per_pixel();
        let row_bytes = self.width as usize * bpp;
        let total = row_bytes * self.height as usize;

        // Fast path: already contiguous
        if self.stride == row_bytes {
            let start = self.offset;
            return self.data[start..start + total].to_vec();
        }

        // Slow path: strip padding
        let mut out = Vec::with_capacity(total);
        let slice = self.as_slice();
        for y in 0..self.height {
            out.extend_from_slice(slice.row(y));
        }
        out
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
    pub fn as_slice(&self) -> PixelSlice<'_, P> {
        let total = self.stride * self.height as usize;
        PixelSlice {
            data: &self.data[self.offset..self.offset + total],
            width: self.width,
            rows: self.height,
            stride: self.stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
            _pixel: PhantomData,
        }
    }

    /// Borrow the full buffer as a mutable [`PixelSliceMut`].
    pub fn as_slice_mut(&mut self) -> PixelSliceMut<'_, P> {
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
            _pixel: PhantomData,
        }
    }

    /// Borrow a range of rows as an immutable [`PixelSlice`].
    ///
    /// # Panics
    ///
    /// Panics if `y + count > height`.
    pub fn rows(&self, y: u32, count: u32) -> PixelSlice<'_, P> {
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
                _pixel: PhantomData,
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
            _pixel: PhantomData,
        }
    }

    /// Borrow a range of rows as a mutable [`PixelSliceMut`].
    ///
    /// # Panics
    ///
    /// Panics if `y + count > height`.
    pub fn rows_mut(&mut self, y: u32, count: u32) -> PixelSliceMut<'_, P> {
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
                _pixel: PhantomData,
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
            _pixel: PhantomData,
        }
    }

    /// Zero-copy sub-region view (immutable).
    ///
    /// # Panics
    ///
    /// Panics if the crop region is out of bounds.
    pub fn crop_view(&self, x: u32, y: u32, w: u32, h: u32) -> PixelSlice<'_, P> {
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
                _pixel: PhantomData,
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
            _pixel: PhantomData,
        }
    }

    /// Copy a sub-region into a new, tightly-packed [`PixelBuffer`].
    ///
    /// # Panics
    ///
    /// Panics if the crop region is out of bounds.
    pub fn crop_copy(&self, x: u32, y: u32, w: u32, h: u32) -> PixelBuffer<P> {
        let src = self.crop_view(x, y, w, h);
        let stride = self.descriptor.aligned_stride(w);
        let total = stride * h as usize;
        let align = self.descriptor.min_alignment();
        let alloc_size = total + align - 1;
        let data = vec![0u8; alloc_size];
        let offset = align_offset(data.as_ptr(), align);
        let mut dst = PixelBuffer {
            data,
            offset,
            width: w,
            height: h,
            stride,
            descriptor: self.descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
            _pixel: PhantomData,
        };
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

// ---------------------------------------------------------------------------
// Format conversion methods (type-erased PixelBuffer)
// ---------------------------------------------------------------------------

#[cfg(feature = "codec")]
impl PixelBuffer {
    /// Convert to RGB8, allocating a new buffer.
    ///
    /// 16-bit values are downscaled with proper rounding. Float values are
    /// clamped to [0.0, 1.0]. Gray is expanded with R=G=B. RGBA/BGRA variants
    /// discard alpha.
    pub fn to_rgb8(&self) -> PixelBuffer<Rgb<u8>> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = Vec::with_capacity(w * h * 3);
        let slice = self.as_slice();

        for y in 0..self.height {
            let row = slice.row(y);
            convert_row_to_rgb8(row, &self.descriptor, &mut out);
        }

        let descriptor = PixelDescriptor::RGB8_SRGB;
        let stride = descriptor.aligned_stride(self.width);
        PixelBuffer {
            data: out,
            offset: 0,
            width: self.width,
            height: self.height,
            stride,
            descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
            _pixel: PhantomData,
        }
    }

    /// Convert to RGBA8, allocating a new buffer.
    ///
    /// Gray is expanded with R=G=B, A=255. RGB gets A=255 added.
    /// 16-bit values are downscaled with proper rounding.
    /// Float values are clamped to [0.0, 1.0].
    pub fn to_rgba8(&self) -> PixelBuffer<Rgba<u8>> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = Vec::with_capacity(w * h * 4);
        let slice = self.as_slice();

        for y in 0..self.height {
            let row = slice.row(y);
            convert_row_to_rgba8(row, &self.descriptor, &mut out);
        }

        let descriptor = PixelDescriptor::RGBA8_SRGB;
        let stride = descriptor.aligned_stride(self.width);
        PixelBuffer {
            data: out,
            offset: 0,
            width: self.width,
            height: self.height,
            stride,
            descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
            _pixel: PhantomData,
        }
    }

    /// Convert to Gray8, allocating a new buffer.
    ///
    /// RGB uses BT.601 luminance: 0.299R + 0.587G + 0.114B.
    /// RGBA/BGRA ignore alpha.
    pub fn to_gray8(&self) -> PixelBuffer<Gray<u8>> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = Vec::with_capacity(w * h);
        let slice = self.as_slice();

        for y in 0..self.height {
            let row = slice.row(y);
            convert_row_to_gray8(row, &self.descriptor, &mut out);
        }

        let descriptor = PixelDescriptor::GRAY8_SRGB;
        let stride = descriptor.aligned_stride(self.width);
        PixelBuffer {
            data: out,
            offset: 0,
            width: self.width,
            height: self.height,
            stride,
            descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
            _pixel: PhantomData,
        }
    }

    /// Convert to BGRA8, allocating a new buffer.
    ///
    /// Channel reordering, 8-bit clamping/truncation, alpha fill.
    pub fn to_bgra8(&self) -> PixelBuffer<BGRA<u8>> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = Vec::with_capacity(w * h * 4);
        let slice = self.as_slice();

        for y in 0..self.height {
            let row = slice.row(y);
            convert_row_to_bgra8(row, &self.descriptor, &mut out);
        }

        let descriptor = PixelDescriptor::BGRA8_SRGB;
        let stride = descriptor.aligned_stride(self.width);
        PixelBuffer {
            data: out,
            offset: 0,
            width: self.width,
            height: self.height,
            stride,
            descriptor,
            working_space: self.working_space,
            color: self.color.clone(),
            _pixel: PhantomData,
        }
    }
}

impl<P> fmt::Debug for PixelBuffer<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PixelBuffer({}x{}, {:?} {:?})",
            self.width, self.height, self.descriptor.layout, self.descriptor.channel_type
        )
    }
}

// ---------------------------------------------------------------------------
// ImgRef → PixelSlice (zero-copy From impls) — codec feature only
// ---------------------------------------------------------------------------

#[cfg(feature = "codec")]
macro_rules! impl_from_imgref {
    ($pixel:ty, $descriptor:expr) => {
        impl<'a> From<ImgRef<'a, $pixel>> for PixelSlice<'a, $pixel> {
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
                    _pixel: PhantomData,
                }
            }
        }
    };
}

// u8 types are conventionally sRGB, f32 types are conventionally linear.
// u16 types have no standard convention so use transfer-agnostic descriptors.
#[cfg(feature = "codec")]
impl_from_imgref!(Rgb<u8>, PixelDescriptor::RGB8_SRGB);
#[cfg(feature = "codec")]
impl_from_imgref!(Rgba<u8>, PixelDescriptor::RGBA8_SRGB);
#[cfg(feature = "codec")]
impl_from_imgref!(Rgb<u16>, PixelDescriptor::RGB16);
#[cfg(feature = "codec")]
impl_from_imgref!(Rgba<u16>, PixelDescriptor::RGBA16);
#[cfg(feature = "codec")]
impl_from_imgref!(Rgb<f32>, PixelDescriptor::RGBF32_LINEAR);
#[cfg(feature = "codec")]
impl_from_imgref!(Rgba<f32>, PixelDescriptor::RGBAF32_LINEAR);
#[cfg(feature = "codec")]
impl_from_imgref!(Gray<u8>, PixelDescriptor::GRAY8_SRGB);
#[cfg(feature = "codec")]
impl_from_imgref!(Gray<u16>, PixelDescriptor::GRAY16);
#[cfg(feature = "codec")]
impl_from_imgref!(Gray<f32>, PixelDescriptor::GRAYF32_LINEAR);
#[cfg(feature = "codec")]
impl_from_imgref!(BGRA<u8>, PixelDescriptor::BGRA8_SRGB);

// ---------------------------------------------------------------------------
// ImgRefMut → PixelSliceMut (zero-copy From impls) — codec feature only
// ---------------------------------------------------------------------------

#[cfg(feature = "codec")]
macro_rules! impl_from_imgref_mut {
    ($pixel:ty, $descriptor:expr) => {
        impl<'a> From<imgref::ImgRefMut<'a, $pixel>> for PixelSliceMut<'a, $pixel> {
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
                    _pixel: PhantomData,
                }
            }
        }
    };
}

#[cfg(feature = "codec")]
impl_from_imgref_mut!(Rgb<u8>, PixelDescriptor::RGB8_SRGB);
#[cfg(feature = "codec")]
impl_from_imgref_mut!(Rgba<u8>, PixelDescriptor::RGBA8_SRGB);
#[cfg(feature = "codec")]
impl_from_imgref_mut!(Rgb<u16>, PixelDescriptor::RGB16);
#[cfg(feature = "codec")]
impl_from_imgref_mut!(Rgba<u16>, PixelDescriptor::RGBA16);
#[cfg(feature = "codec")]
impl_from_imgref_mut!(Rgb<f32>, PixelDescriptor::RGBF32_LINEAR);
#[cfg(feature = "codec")]
impl_from_imgref_mut!(Rgba<f32>, PixelDescriptor::RGBAF32_LINEAR);
#[cfg(feature = "codec")]
impl_from_imgref_mut!(Gray<u8>, PixelDescriptor::GRAY8_SRGB);
#[cfg(feature = "codec")]
impl_from_imgref_mut!(Gray<u16>, PixelDescriptor::GRAY16);
#[cfg(feature = "codec")]
impl_from_imgref_mut!(Gray<f32>, PixelDescriptor::GRAYF32_LINEAR);
#[cfg(feature = "codec")]
impl_from_imgref_mut!(BGRA<u8>, PixelDescriptor::BGRA8_SRGB);

// ---------------------------------------------------------------------------
// Typed → Erased blanket From impls (erase via From)
// ---------------------------------------------------------------------------

impl<'a, P: Pixel> From<PixelSlice<'a, P>> for PixelSlice<'a> {
    fn from(typed: PixelSlice<'a, P>) -> Self {
        typed.erase()
    }
}

impl<'a, P: Pixel> From<PixelSliceMut<'a, P>> for PixelSliceMut<'a> {
    fn from(typed: PixelSliceMut<'a, P>) -> Self {
        typed.erase()
    }
}

impl<P: Pixel> From<PixelBuffer<P>> for PixelBuffer {
    fn from(typed: PixelBuffer<P>) -> Self {
        typed.erase()
    }
}

// ---------------------------------------------------------------------------
// PixelData → PixelBuffer (From, always copies) — codec feature only
// Deprecated: exists for codecs still using PixelData internally.
// ---------------------------------------------------------------------------

#[cfg(feature = "codec")]
#[allow(deprecated)]
impl From<crate::pixel::PixelData> for PixelBuffer {
    fn from(pixels: crate::pixel::PixelData) -> Self {
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
            _pixel: PhantomData,
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

/// Validate slice parameters (shared by erased and typed constructors).
fn validate_slice(
    data_len: usize,
    data_ptr: *const u8,
    width: u32,
    rows: u32,
    stride_bytes: usize,
    descriptor: &PixelDescriptor,
) -> Result<(), BufferError> {
    let bpp = descriptor.bytes_per_pixel();
    let min_stride = (width as usize)
        .checked_mul(bpp)
        .ok_or(BufferError::InvalidDimensions)?;
    if stride_bytes < min_stride {
        return Err(BufferError::StrideTooSmall);
    }
    if bpp > 0 && !stride_bytes.is_multiple_of(bpp) {
        return Err(BufferError::StrideNotPixelAligned);
    }
    if rows > 0 {
        let required = required_bytes(rows, stride_bytes, min_stride)?;
        if data_len < required {
            return Err(BufferError::InsufficientData);
        }
    }
    let align = descriptor.min_alignment();
    if !(data_ptr as usize).is_multiple_of(align) {
        return Err(BufferError::AlignmentViolation);
    }
    Ok(())
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

#[cfg(feature = "codec")]
#[inline]
fn parse_u16(bytes: &[u8]) -> u16 {
    u16::from_ne_bytes([bytes[0], bytes[1]])
}

#[cfg(feature = "codec")]
#[inline]
fn parse_f32(bytes: &[u8]) -> f32 {
    f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Convert `Vec<P>` to `Vec<u8>`. Zero-copy when alignment matches (u8-component
/// types), copies via `cast_slice` otherwise.
#[cfg(feature = "codec")]
fn pixels_to_bytes<P: bytemuck::NoUninit>(pixels: Vec<P>) -> Vec<u8> {
    match bytemuck::try_cast_vec(pixels) {
        Ok(bytes) => bytes,
        Err((_err, pixels)) => bytemuck::cast_slice::<P, u8>(&pixels).to_vec(),
    }
}

// ---------------------------------------------------------------------------
// Row conversion helpers (descriptor-driven, no enum matching)
// ---------------------------------------------------------------------------

/// Convert 16-bit to 8-bit with proper rounding.
/// Maps 0→0 and 65535→255 exactly.
#[cfg(feature = "codec")]
#[inline]
fn u16_to_u8(v: u16) -> u8 {
    ((v as u32 * 255 + 32768) >> 16) as u8
}

/// BT.601 luminance from 8-bit RGB.
#[cfg(feature = "codec")]
#[inline]
fn rgb_to_luma(r: u8, g: u8, b: u8) -> u8 {
    ((77u32 * r as u32 + 150u32 * g as u32 + 29u32 * b as u32) >> 8) as u8
}

/// Clamp f32 to [0,1] and scale to u8.
#[cfg(feature = "codec")]
#[inline]
fn f32_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0) as u8
}

/// Convert one row of pixels to RGB8, appending to `out`.
#[cfg(feature = "codec")]
fn convert_row_to_rgb8(row: &[u8], desc: &PixelDescriptor, out: &mut Vec<u8>) {
    let bpp = desc.bytes_per_pixel();
    match (desc.channel_type, desc.layout) {
        // --- U8 ---
        (ChannelType::U8, ChannelLayout::Rgb) => out.extend_from_slice(row),
        (ChannelType::U8, ChannelLayout::Rgba) => {
            for chunk in row.chunks_exact(bpp) {
                out.extend_from_slice(&chunk[..3]);
            }
        }
        (ChannelType::U8, ChannelLayout::Gray) => {
            for &v in row {
                out.extend_from_slice(&[v, v, v]);
            }
        }
        (ChannelType::U8, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(2) {
                let v = chunk[0];
                out.extend_from_slice(&[v, v, v]);
            }
        }
        (ChannelType::U8, ChannelLayout::Bgra) => {
            for chunk in row.chunks_exact(bpp) {
                // BGRA order: b=0, g=1, r=2, a=3
                out.extend_from_slice(&[chunk[2], chunk[1], chunk[0]]);
            }
        }
        // --- U16 ---
        (ChannelType::U16, ChannelLayout::Rgb) => {
            for chunk in row.chunks_exact(bpp) {
                let r = u16_to_u8(parse_u16(chunk));
                let g = u16_to_u8(parse_u16(&chunk[2..]));
                let b = u16_to_u8(parse_u16(&chunk[4..]));
                out.extend_from_slice(&[r, g, b]);
            }
        }
        (ChannelType::U16, ChannelLayout::Rgba) => {
            for chunk in row.chunks_exact(bpp) {
                let r = u16_to_u8(parse_u16(chunk));
                let g = u16_to_u8(parse_u16(&chunk[2..]));
                let b = u16_to_u8(parse_u16(&chunk[4..]));
                out.extend_from_slice(&[r, g, b]);
            }
        }
        (ChannelType::U16, ChannelLayout::Gray) => {
            for chunk in row.chunks_exact(bpp) {
                let v = u16_to_u8(parse_u16(chunk));
                out.extend_from_slice(&[v, v, v]);
            }
        }
        (ChannelType::U16, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(bpp) {
                let v = u16_to_u8(parse_u16(chunk));
                out.extend_from_slice(&[v, v, v]);
            }
        }
        (ChannelType::U16, ChannelLayout::Bgra) => {
            for chunk in row.chunks_exact(bpp) {
                let b = u16_to_u8(parse_u16(chunk));
                let g = u16_to_u8(parse_u16(&chunk[2..]));
                let r = u16_to_u8(parse_u16(&chunk[4..]));
                out.extend_from_slice(&[r, g, b]);
            }
        }
        // --- F32 ---
        (ChannelType::F32, ChannelLayout::Rgb) => {
            for chunk in row.chunks_exact(bpp) {
                let r = f32_to_u8(parse_f32(chunk));
                let g = f32_to_u8(parse_f32(&chunk[4..]));
                let b = f32_to_u8(parse_f32(&chunk[8..]));
                out.extend_from_slice(&[r, g, b]);
            }
        }
        (ChannelType::F32, ChannelLayout::Rgba) => {
            for chunk in row.chunks_exact(bpp) {
                let r = f32_to_u8(parse_f32(chunk));
                let g = f32_to_u8(parse_f32(&chunk[4..]));
                let b = f32_to_u8(parse_f32(&chunk[8..]));
                out.extend_from_slice(&[r, g, b]);
            }
        }
        (ChannelType::F32, ChannelLayout::Gray) => {
            for chunk in row.chunks_exact(bpp) {
                let v = f32_to_u8(parse_f32(chunk));
                out.extend_from_slice(&[v, v, v]);
            }
        }
        (ChannelType::F32, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(bpp) {
                let v = f32_to_u8(parse_f32(chunk));
                out.extend_from_slice(&[v, v, v]);
            }
        }
        (ChannelType::F32, ChannelLayout::Bgra) => {
            for chunk in row.chunks_exact(bpp) {
                let b = f32_to_u8(parse_f32(chunk));
                let g = f32_to_u8(parse_f32(&chunk[4..]));
                let r = f32_to_u8(parse_f32(&chunk[8..]));
                out.extend_from_slice(&[r, g, b]);
            }
        }
        _ => {}
    }
}

/// Convert one row of pixels to RGBA8, appending to `out`.
#[cfg(feature = "codec")]
fn convert_row_to_rgba8(row: &[u8], desc: &PixelDescriptor, out: &mut Vec<u8>) {
    let bpp = desc.bytes_per_pixel();
    match (desc.channel_type, desc.layout) {
        // --- U8 ---
        (ChannelType::U8, ChannelLayout::Rgba) => out.extend_from_slice(row),
        (ChannelType::U8, ChannelLayout::Rgb) => {
            for chunk in row.chunks_exact(bpp) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
        }
        (ChannelType::U8, ChannelLayout::Gray) => {
            for &v in row {
                out.extend_from_slice(&[v, v, v, 255]);
            }
        }
        (ChannelType::U8, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(2) {
                let v = chunk[0];
                out.extend_from_slice(&[v, v, v, chunk[1]]);
            }
        }
        (ChannelType::U8, ChannelLayout::Bgra) => {
            for chunk in row.chunks_exact(bpp) {
                out.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
            }
        }
        // --- U16 ---
        (ChannelType::U16, ChannelLayout::Rgba) => {
            for chunk in row.chunks_exact(bpp) {
                let r = u16_to_u8(parse_u16(chunk));
                let g = u16_to_u8(parse_u16(&chunk[2..]));
                let b = u16_to_u8(parse_u16(&chunk[4..]));
                let a = u16_to_u8(parse_u16(&chunk[6..]));
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
        (ChannelType::U16, ChannelLayout::Rgb) => {
            for chunk in row.chunks_exact(bpp) {
                let r = u16_to_u8(parse_u16(chunk));
                let g = u16_to_u8(parse_u16(&chunk[2..]));
                let b = u16_to_u8(parse_u16(&chunk[4..]));
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
        (ChannelType::U16, ChannelLayout::Gray) => {
            for chunk in row.chunks_exact(bpp) {
                let v = u16_to_u8(parse_u16(chunk));
                out.extend_from_slice(&[v, v, v, 255]);
            }
        }
        (ChannelType::U16, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(bpp) {
                let v = u16_to_u8(parse_u16(chunk));
                let a = u16_to_u8(parse_u16(&chunk[2..]));
                out.extend_from_slice(&[v, v, v, a]);
            }
        }
        (ChannelType::U16, ChannelLayout::Bgra) => {
            for chunk in row.chunks_exact(bpp) {
                let b = u16_to_u8(parse_u16(chunk));
                let g = u16_to_u8(parse_u16(&chunk[2..]));
                let r = u16_to_u8(parse_u16(&chunk[4..]));
                let a = u16_to_u8(parse_u16(&chunk[6..]));
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
        // --- F32 ---
        (ChannelType::F32, ChannelLayout::Rgba) => {
            for chunk in row.chunks_exact(bpp) {
                let r = f32_to_u8(parse_f32(chunk));
                let g = f32_to_u8(parse_f32(&chunk[4..]));
                let b = f32_to_u8(parse_f32(&chunk[8..]));
                let a = f32_to_u8(parse_f32(&chunk[12..]));
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
        (ChannelType::F32, ChannelLayout::Rgb) => {
            for chunk in row.chunks_exact(bpp) {
                let r = f32_to_u8(parse_f32(chunk));
                let g = f32_to_u8(parse_f32(&chunk[4..]));
                let b = f32_to_u8(parse_f32(&chunk[8..]));
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
        (ChannelType::F32, ChannelLayout::Gray) => {
            for chunk in row.chunks_exact(bpp) {
                let v = f32_to_u8(parse_f32(chunk));
                out.extend_from_slice(&[v, v, v, 255]);
            }
        }
        (ChannelType::F32, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(bpp) {
                let v = f32_to_u8(parse_f32(chunk));
                let a = f32_to_u8(parse_f32(&chunk[4..]));
                out.extend_from_slice(&[v, v, v, a]);
            }
        }
        (ChannelType::F32, ChannelLayout::Bgra) => {
            for chunk in row.chunks_exact(bpp) {
                let b = f32_to_u8(parse_f32(chunk));
                let g = f32_to_u8(parse_f32(&chunk[4..]));
                let r = f32_to_u8(parse_f32(&chunk[8..]));
                let a = f32_to_u8(parse_f32(&chunk[12..]));
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
        _ => {}
    }
}

/// Convert one row of pixels to Gray8, appending to `out`.
/// RGB uses BT.601 luminance. Alpha is discarded.
#[cfg(feature = "codec")]
fn convert_row_to_gray8(row: &[u8], desc: &PixelDescriptor, out: &mut Vec<u8>) {
    let bpp = desc.bytes_per_pixel();
    match (desc.channel_type, desc.layout) {
        // --- U8 ---
        (ChannelType::U8, ChannelLayout::Gray) => out.extend_from_slice(row),
        (ChannelType::U8, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(2) {
                out.push(chunk[0]);
            }
        }
        (ChannelType::U8, ChannelLayout::Rgb) => {
            for chunk in row.chunks_exact(bpp) {
                out.push(rgb_to_luma(chunk[0], chunk[1], chunk[2]));
            }
        }
        (ChannelType::U8, ChannelLayout::Rgba) => {
            for chunk in row.chunks_exact(bpp) {
                out.push(rgb_to_luma(chunk[0], chunk[1], chunk[2]));
            }
        }
        (ChannelType::U8, ChannelLayout::Bgra) => {
            for chunk in row.chunks_exact(bpp) {
                // BGRA: b=0, g=1, r=2
                out.push(rgb_to_luma(chunk[2], chunk[1], chunk[0]));
            }
        }
        // --- U16 ---
        (ChannelType::U16, ChannelLayout::Gray) => {
            for chunk in row.chunks_exact(bpp) {
                out.push(u16_to_u8(parse_u16(chunk)));
            }
        }
        (ChannelType::U16, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(bpp) {
                out.push(u16_to_u8(parse_u16(chunk)));
            }
        }
        (ChannelType::U16, ChannelLayout::Rgb) => {
            for chunk in row.chunks_exact(bpp) {
                let r = u16_to_u8(parse_u16(chunk));
                let g = u16_to_u8(parse_u16(&chunk[2..]));
                let b = u16_to_u8(parse_u16(&chunk[4..]));
                out.push(rgb_to_luma(r, g, b));
            }
        }
        (ChannelType::U16, ChannelLayout::Rgba) => {
            for chunk in row.chunks_exact(bpp) {
                let r = u16_to_u8(parse_u16(chunk));
                let g = u16_to_u8(parse_u16(&chunk[2..]));
                let b = u16_to_u8(parse_u16(&chunk[4..]));
                out.push(rgb_to_luma(r, g, b));
            }
        }
        (ChannelType::U16, ChannelLayout::Bgra) => {
            for chunk in row.chunks_exact(bpp) {
                let b = u16_to_u8(parse_u16(chunk));
                let g = u16_to_u8(parse_u16(&chunk[2..]));
                let r = u16_to_u8(parse_u16(&chunk[4..]));
                out.push(rgb_to_luma(r, g, b));
            }
        }
        // --- F32 ---
        (ChannelType::F32, ChannelLayout::Gray) => {
            for chunk in row.chunks_exact(bpp) {
                out.push(f32_to_u8(parse_f32(chunk)));
            }
        }
        (ChannelType::F32, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(bpp) {
                out.push(f32_to_u8(parse_f32(chunk)));
            }
        }
        (ChannelType::F32, ChannelLayout::Rgb) => {
            for chunk in row.chunks_exact(bpp) {
                let r = f32_to_u8(parse_f32(chunk));
                let g = f32_to_u8(parse_f32(&chunk[4..]));
                let b = f32_to_u8(parse_f32(&chunk[8..]));
                out.push(rgb_to_luma(r, g, b));
            }
        }
        (ChannelType::F32, ChannelLayout::Rgba) => {
            for chunk in row.chunks_exact(bpp) {
                let r = f32_to_u8(parse_f32(chunk));
                let g = f32_to_u8(parse_f32(&chunk[4..]));
                let b = f32_to_u8(parse_f32(&chunk[8..]));
                out.push(rgb_to_luma(r, g, b));
            }
        }
        (ChannelType::F32, ChannelLayout::Bgra) => {
            for chunk in row.chunks_exact(bpp) {
                let b = f32_to_u8(parse_f32(chunk));
                let g = f32_to_u8(parse_f32(&chunk[4..]));
                let r = f32_to_u8(parse_f32(&chunk[8..]));
                out.push(rgb_to_luma(r, g, b));
            }
        }
        _ => {}
    }
}

/// Convert one row of pixels to BGRA8, appending to `out`.
#[cfg(feature = "codec")]
fn convert_row_to_bgra8(row: &[u8], desc: &PixelDescriptor, out: &mut Vec<u8>) {
    let bpp = desc.bytes_per_pixel();
    match (desc.channel_type, desc.layout) {
        // --- U8 ---
        (ChannelType::U8, ChannelLayout::Bgra) => out.extend_from_slice(row),
        (ChannelType::U8, ChannelLayout::Rgba) => {
            for chunk in row.chunks_exact(bpp) {
                out.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
            }
        }
        (ChannelType::U8, ChannelLayout::Rgb) => {
            for chunk in row.chunks_exact(bpp) {
                out.extend_from_slice(&[chunk[2], chunk[1], chunk[0], 255]);
            }
        }
        (ChannelType::U8, ChannelLayout::Gray) => {
            for &v in row {
                out.extend_from_slice(&[v, v, v, 255]);
            }
        }
        (ChannelType::U8, ChannelLayout::GrayAlpha) => {
            for chunk in row.chunks_exact(2) {
                let v = chunk[0];
                out.extend_from_slice(&[v, v, v, chunk[1]]);
            }
        }
        // Fall back: convert to RGBA8 first, then swizzle
        _ => {
            let start = out.len();
            convert_row_to_rgba8(row, desc, out);
            // Swizzle RGBA → BGRA in-place
            for i in (start..out.len()).step_by(4) {
                out.swap(i, i + 2); // R ↔ B
            }
        }
    }
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
        // BGRX8 has Bgra layout (4 channels) but alpha is Undefined padding
        assert_eq!(PixelDescriptor::BGRX8_SRGB.channels(), 4);
        assert!(PixelDescriptor::BGRX8_SRGB.layout.has_alpha()); // layout says yes
        assert_eq!(PixelDescriptor::BGRX8_SRGB.alpha, Some(AlphaMode::Undefined)); // padding
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
    fn descriptor_is_grayscale() {
        assert!(PixelDescriptor::GRAY8_SRGB.is_grayscale());
        assert!(PixelDescriptor::GRAY16_SRGB.is_grayscale());
        assert!(PixelDescriptor::GRAYF32_LINEAR.is_grayscale());
        assert!(PixelDescriptor::GRAYA8_SRGB.is_grayscale());
        assert!(PixelDescriptor::GRAYA16_SRGB.is_grayscale());
        assert!(PixelDescriptor::GRAYAF32_LINEAR.is_grayscale());
        assert!(!PixelDescriptor::RGB8_SRGB.is_grayscale());
        assert!(!PixelDescriptor::RGBA8_SRGB.is_grayscale());
        assert!(!PixelDescriptor::BGRA8_SRGB.is_grayscale());
    }

    #[test]
    fn descriptor_is_bgr() {
        assert!(PixelDescriptor::BGRA8_SRGB.is_bgr());
        assert!(PixelDescriptor::BGRX8_SRGB.is_bgr());
        assert!(!PixelDescriptor::RGB8_SRGB.is_bgr());
        assert!(!PixelDescriptor::RGBA8_SRGB.is_bgr());
        assert!(!PixelDescriptor::GRAY8_SRGB.is_bgr());
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
        assert_eq!(d.alpha, Some(AlphaMode::Undefined));
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

    // --- PixelFormat round-trip ---

    #[test]
    fn pixel_format_roundtrip_all_named_constants() {
        // Every transfer-agnostic named constant should round-trip through pixel_format()
        let cases: &[(PixelDescriptor, PixelFormat)] = &[
            (PixelDescriptor::RGB8, PixelFormat::Rgb8),
            (PixelDescriptor::RGBA8, PixelFormat::Rgba8),
            (PixelDescriptor::RGB16, PixelFormat::Rgb16),
            (PixelDescriptor::RGBA16, PixelFormat::Rgba16),
            (PixelDescriptor::RGBF32, PixelFormat::RgbF32),
            (PixelDescriptor::RGBAF32, PixelFormat::RgbaF32),
            (PixelDescriptor::GRAY8, PixelFormat::Gray8),
            (PixelDescriptor::GRAY16, PixelFormat::Gray16),
            (PixelDescriptor::GRAYF32, PixelFormat::GrayF32),
            (PixelDescriptor::GRAYA8, PixelFormat::GrayA8),
            (PixelDescriptor::GRAYA16, PixelFormat::GrayA16),
            (PixelDescriptor::GRAYAF32, PixelFormat::GrayAF32),
            (PixelDescriptor::BGRA8, PixelFormat::Bgra8),
            (PixelDescriptor::RGBX8, PixelFormat::Rgbx8),
            (PixelDescriptor::BGRX8, PixelFormat::Bgrx8),
        ];
        for (desc, expected_fmt) in cases {
            let fmt = desc.pixel_format();
            assert_eq!(fmt, Some(*expected_fmt), "pixel_format() for {desc}");
            // Round-trip: format → descriptor → layout_compatible with original
            let base = expected_fmt.descriptor();
            assert!(
                base.layout_compatible(desc),
                "descriptor from {expected_fmt} not layout-compatible with {desc}"
            );
        }
    }

    #[test]
    fn pixel_format_srgb_variants() {
        // sRGB-tagged variants should also resolve to the same PixelFormat
        assert_eq!(
            PixelDescriptor::RGB8_SRGB.pixel_format(),
            Some(PixelFormat::Rgb8)
        );
        assert_eq!(
            PixelDescriptor::RGBA8_SRGB.pixel_format(),
            Some(PixelFormat::Rgba8)
        );
        assert_eq!(
            PixelDescriptor::GRAY8_SRGB.pixel_format(),
            Some(PixelFormat::Gray8)
        );
    }

    #[test]
    fn pixel_format_none_for_exotic() {
        // I16 Rgb — no PixelFormat variant for this
        let exotic = PixelDescriptor::new(
            ChannelType::I16,
            ChannelLayout::Rgb,
            None,
            TransferFunction::Unknown,
        );
        assert_eq!(exotic.pixel_format(), None);
    }

    // --- Display snapshots ---

    #[test]
    fn display_channel_type() {
        assert_eq!(format!("{}", ChannelType::U8), "U8");
        assert_eq!(format!("{}", ChannelType::U16), "U16");
        assert_eq!(format!("{}", ChannelType::F32), "F32");
        assert_eq!(format!("{}", ChannelType::F16), "F16");
        assert_eq!(format!("{}", ChannelType::I16), "I16");
    }

    #[test]
    fn display_channel_layout() {
        assert_eq!(format!("{}", ChannelLayout::Rgb), "RGB");
        assert_eq!(format!("{}", ChannelLayout::Rgba), "RGBA");
        assert_eq!(format!("{}", ChannelLayout::Gray), "Gray");
        assert_eq!(format!("{}", ChannelLayout::GrayAlpha), "GrayAlpha");
        assert_eq!(format!("{}", ChannelLayout::Bgra), "BGRA");
    }

    #[test]
    fn display_alpha_mode() {
        assert_eq!(format!("{}", AlphaMode::Undefined), "undefined");
        assert_eq!(format!("{}", AlphaMode::Straight), "straight");
        assert_eq!(format!("{}", AlphaMode::Premultiplied), "premultiplied");
        assert_eq!(format!("{}", AlphaMode::Opaque), "opaque");
    }

    #[test]
    fn display_transfer_function() {
        assert_eq!(format!("{}", TransferFunction::Linear), "linear");
        assert_eq!(format!("{}", TransferFunction::Srgb), "sRGB");
        assert_eq!(format!("{}", TransferFunction::Bt709), "BT.709");
        assert_eq!(format!("{}", TransferFunction::Pq), "PQ");
        assert_eq!(format!("{}", TransferFunction::Hlg), "HLG");
        assert_eq!(format!("{}", TransferFunction::Unknown), "unknown");
    }

    #[test]
    fn display_color_primaries() {
        assert_eq!(format!("{}", ColorPrimaries::Bt709), "BT.709");
        assert_eq!(format!("{}", ColorPrimaries::Bt2020), "BT.2020");
        assert_eq!(format!("{}", ColorPrimaries::DisplayP3), "Display P3");
        assert_eq!(format!("{}", ColorPrimaries::Unknown), "unknown");
    }

    #[test]
    fn display_signal_range() {
        assert_eq!(format!("{}", SignalRange::Full), "full");
        assert_eq!(format!("{}", SignalRange::Narrow), "narrow");
    }

    #[test]
    fn display_pixel_descriptor() {
        // Transfer-agnostic (Unknown) — no transfer shown
        assert_eq!(format!("{}", PixelDescriptor::RGB8), "RGB8");
        // sRGB transfer shown
        assert_eq!(format!("{}", PixelDescriptor::RGB8_SRGB), "RGB8/sRGB");
        // Linear + non-default primaries
        let pq_bt2020 = PixelDescriptor::RGBA16
            .with_transfer(TransferFunction::Pq)
            .with_primaries(ColorPrimaries::Bt2020);
        assert_eq!(format!("{pq_bt2020}"), "RGBA16/PQ/BT.2020");
        // Narrow range shown
        let narrow = pq_bt2020.with_signal_range(SignalRange::Narrow);
        assert_eq!(format!("{narrow}"), "RGBA16/PQ/BT.2020/narrow");
        // Linear gray
        assert_eq!(
            format!("{}", PixelDescriptor::GRAYF32_LINEAR),
            "GrayF32/linear"
        );
        // Exotic format (no PixelFormat variant) — fallback to layout/channel_type
        let exotic = PixelDescriptor::new(
            ChannelType::I16,
            ChannelLayout::Rgb,
            None,
            TransferFunction::Linear,
        );
        assert_eq!(format!("{exotic}"), "RGB/I16/linear");
    }

    // --- ChannelType predicates ---

    #[test]
    fn channel_type_predicates() {
        assert!(ChannelType::U8.is_u8());
        assert!(!ChannelType::U8.is_u16());
        assert!(ChannelType::U16.is_u16());
        assert!(ChannelType::F32.is_f32());
        assert!(ChannelType::F16.is_f16());
        assert!(ChannelType::I16.is_i16());
        // Integer vs float
        assert!(ChannelType::U8.is_integer());
        assert!(ChannelType::U16.is_integer());
        assert!(ChannelType::I16.is_integer());
        assert!(!ChannelType::F32.is_integer());
        assert!(!ChannelType::F16.is_integer());
        assert!(ChannelType::F32.is_float());
        assert!(ChannelType::F16.is_float());
        assert!(!ChannelType::U8.is_float());
    }

    // --- with_descriptor assertion ---

    #[test]
    fn with_descriptor_metadata_change_succeeds() {
        let buf = PixelBuffer::new(2, 2, PixelDescriptor::RGB8_SRGB);
        // Changing transfer function is metadata-only — should succeed
        let buf2 = buf.with_descriptor(PixelDescriptor::RGB8);
        assert_eq!(buf2.descriptor(), PixelDescriptor::RGB8);
    }

    #[test]
    #[should_panic(expected = "with_descriptor() cannot change physical layout")]
    fn with_descriptor_layout_change_panics() {
        let buf = PixelBuffer::new(2, 2, PixelDescriptor::RGB8);
        // Trying to change from RGB8 to RGBA8 — different layout, should panic
        let _ = buf.with_descriptor(PixelDescriptor::RGBA8);
    }

    #[test]
    fn with_descriptor_slice_assertion() {
        let buf = PixelBuffer::new(2, 2, PixelDescriptor::RGB8_SRGB);
        let slice = buf.as_slice();
        // Metadata change OK
        let s2 = slice.with_descriptor(PixelDescriptor::RGB8);
        assert_eq!(s2.descriptor(), PixelDescriptor::RGB8);
    }

    #[test]
    #[should_panic(expected = "with_descriptor() cannot change physical layout")]
    fn with_descriptor_slice_layout_change_panics() {
        let buf = PixelBuffer::new(2, 2, PixelDescriptor::RGB8);
        let slice = buf.as_slice();
        let _ = slice.with_descriptor(PixelDescriptor::RGBA8);
    }

    // --- reinterpret ---

    #[test]
    fn reinterpret_same_bpp_succeeds() {
        // RGBA8 → BGRA8: same 4 bpp, different layout
        let buf = PixelBuffer::new(2, 2, PixelDescriptor::RGBA8);
        let buf2 = buf.reinterpret(PixelDescriptor::BGRA8).unwrap();
        assert_eq!(buf2.descriptor().layout, ChannelLayout::Bgra);
    }

    #[test]
    fn reinterpret_different_bpp_fails() {
        // RGB8 (3 bpp) → RGBA8 (4 bpp): different bytes_per_pixel
        let buf = PixelBuffer::new(2, 2, PixelDescriptor::RGB8);
        let err = buf.reinterpret(PixelDescriptor::RGBA8);
        assert_eq!(err.unwrap_err(), BufferError::IncompatibleDescriptor);
    }

    #[test]
    fn reinterpret_rgbx_to_rgba() {
        // RGBX8 → RGBA8: same bpp (4), reinterpret padding as alpha
        let buf = PixelBuffer::new(2, 2, PixelDescriptor::RGBX8);
        let buf2 = buf.reinterpret(PixelDescriptor::RGBA8).unwrap();
        assert!(buf2.descriptor().has_alpha());
    }

    // --- Per-field metadata setters ---

    #[test]
    fn per_field_setters() {
        let buf = PixelBuffer::new(2, 2, PixelDescriptor::RGB8);
        let buf = buf.with_transfer(TransferFunction::Srgb);
        assert_eq!(buf.descriptor().transfer, TransferFunction::Srgb);
        let buf = buf.with_primaries(ColorPrimaries::DisplayP3);
        assert_eq!(buf.descriptor().primaries, ColorPrimaries::DisplayP3);
        let buf = buf.with_signal_range(SignalRange::Narrow);
        assert!(buf.descriptor().is_narrow_range());
        let buf = buf.with_alpha_mode(Some(AlphaMode::Premultiplied));
        assert_eq!(buf.descriptor().alpha, Some(AlphaMode::Premultiplied));
    }

    // --- copy_to_contiguous_bytes ---

    #[test]
    fn copy_to_contiguous_bytes_tight() {
        let mut buf = PixelBuffer::new(2, 2, PixelDescriptor::RGB8_SRGB);
        {
            let mut s = buf.as_slice_mut();
            s.row_mut(0).copy_from_slice(&[1, 2, 3, 4, 5, 6]);
            s.row_mut(1).copy_from_slice(&[7, 8, 9, 10, 11, 12]);
        }
        let bytes = buf.copy_to_contiguous_bytes();
        assert_eq!(bytes, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn copy_to_contiguous_bytes_padded() {
        // Use SIMD-aligned buffer which will have stride padding for small widths
        let mut buf = PixelBuffer::new_simd_aligned(2, 2, PixelDescriptor::RGB8_SRGB, 16);
        let stride = buf.stride();
        // Stride should be >= 6 (2 pixels * 3 bytes) and aligned to lcm(3, 16) = 48
        assert!(stride >= 6);
        {
            let mut s = buf.as_slice_mut();
            s.row_mut(0).copy_from_slice(&[1, 2, 3, 4, 5, 6]);
            s.row_mut(1).copy_from_slice(&[7, 8, 9, 10, 11, 12]);
        }
        let bytes = buf.copy_to_contiguous_bytes();
        // Should only contain the actual pixel data, no padding
        assert_eq!(bytes.len(), 12);
        assert_eq!(bytes, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    // --- PixelFormat Display ---

    #[test]
    fn pixel_format_display() {
        assert_eq!(format!("{}", PixelFormat::Rgb8), "RGB8");
        assert_eq!(format!("{}", PixelFormat::Rgba16), "RGBA16");
        assert_eq!(format!("{}", PixelFormat::GrayA8), "GrayA8");
        assert_eq!(format!("{}", PixelFormat::Bgra8), "BGRA8");
    }

    // --- PixelFormat properties ---

    #[test]
    fn pixel_format_properties() {
        assert_eq!(PixelFormat::Rgb8.bytes_per_pixel(), 3);
        assert_eq!(PixelFormat::Rgba8.bytes_per_pixel(), 4);
        assert_eq!(PixelFormat::Gray8.bytes_per_pixel(), 1);
        assert_eq!(PixelFormat::Rgb16.bytes_per_pixel(), 6);
        assert!(!PixelFormat::Rgb8.has_alpha());
        assert!(PixelFormat::Rgba8.has_alpha());
        assert!(!PixelFormat::Rgbx8.has_alpha());
        assert!(PixelFormat::Bgra8.has_alpha());
        assert!(PixelFormat::Gray8.is_grayscale());
        assert!(PixelFormat::GrayA8.is_grayscale());
        assert!(!PixelFormat::Rgb8.is_grayscale());
        assert_eq!(PixelFormat::Rgb8.channel_type(), ChannelType::U8);
        assert_eq!(PixelFormat::RgbF32.channel_type(), ChannelType::F32);
    }

    // --- AlphaMode tests ---

    /// Helper: set alpha mode on a descriptor copy.
    fn set_alpha(mut desc: PixelDescriptor, alpha: Option<AlphaMode>) -> PixelDescriptor {
        desc.alpha = alpha;
        desc
    }

    #[test]
    fn alpha_mode_opaque_has_alpha() {
        // Opaque means alpha channel IS present (with valid 255 values)
        let desc = set_alpha(PixelDescriptor::RGBA8_SRGB, Some(AlphaMode::Opaque));
        assert!(desc.has_alpha());
    }

    #[test]
    fn alpha_mode_undefined_no_has_alpha() {
        // Undefined means padding byte — not meaningful alpha
        assert!(!PixelDescriptor::RGBX8_SRGB.has_alpha());
        assert!(!PixelDescriptor::BGRX8_SRGB.has_alpha());
    }

    #[test]
    fn alpha_mode_none_no_has_alpha() {
        // None (Option) means no alpha-position channel
        assert!(!PixelDescriptor::RGB8_SRGB.has_alpha());
        assert!(!PixelDescriptor::GRAY8_SRGB.has_alpha());
    }

    #[test]
    fn alpha_mode_opaque_is_opaque() {
        // None, Undefined, and Opaque are all "opaque" (no transparency)
        assert!(PixelDescriptor::RGB8_SRGB.is_opaque()); // None — no alpha channel
        assert!(PixelDescriptor::RGBX8_SRGB.is_opaque()); // Undefined — padding
        let opaque_rgba = set_alpha(PixelDescriptor::RGBA8_SRGB, Some(AlphaMode::Opaque));
        assert!(opaque_rgba.is_opaque()); // Opaque — alpha present but all 255

        // Straight and Premultiplied are NOT opaque
        assert!(!PixelDescriptor::RGBA8_SRGB.is_opaque());
        assert!(!set_alpha(PixelDescriptor::RGBA8_SRGB, Some(AlphaMode::Premultiplied)).is_opaque());
    }

    #[test]
    fn alpha_mode_opaque_may_have_transparency() {
        // Opaque should NOT report "may have transparency"
        let opaque_rgba = set_alpha(PixelDescriptor::RGBA8_SRGB, Some(AlphaMode::Opaque));
        assert!(!opaque_rgba.may_have_transparency());

        // Straight and Premultiplied DO may have transparency
        assert!(PixelDescriptor::RGBA8_SRGB.may_have_transparency());
        assert!(set_alpha(PixelDescriptor::RGBA8_SRGB, Some(AlphaMode::Premultiplied))
            .may_have_transparency());

        // None and Undefined do not have transparency
        assert!(!PixelDescriptor::RGB8_SRGB.may_have_transparency());
        assert!(!PixelDescriptor::RGBX8_SRGB.may_have_transparency());
    }

    #[test]
    fn alpha_mode_opaque_pixel_format_roundtrip() {
        // RGBA8 with Opaque should still resolve to PixelFormat::Rgba8
        assert_eq!(
            set_alpha(PixelDescriptor::RGBA8_SRGB, Some(AlphaMode::Opaque)).pixel_format(),
            Some(PixelFormat::Rgba8),
        );

        // BGRA8 with Opaque → Bgra8
        assert_eq!(
            set_alpha(PixelDescriptor::BGRA8_SRGB, Some(AlphaMode::Opaque)).pixel_format(),
            Some(PixelFormat::Bgra8),
        );

        // GrayA8 with Opaque → GrayA8
        assert_eq!(
            set_alpha(PixelDescriptor::GRAYA8, Some(AlphaMode::Opaque)).pixel_format(),
            Some(PixelFormat::GrayA8),
        );
    }

    #[test]
    fn alpha_mode_opaque_with_descriptor_compatible() {
        // Changing alpha mode from Straight to Opaque is a metadata change — should succeed
        let buf = PixelBuffer::new(2, 2, PixelDescriptor::RGBA8_SRGB);
        let opaque_desc = set_alpha(PixelDescriptor::RGBA8_SRGB, Some(AlphaMode::Opaque));
        let buf = buf.with_descriptor(opaque_desc);
        assert_eq!(buf.descriptor().alpha, Some(AlphaMode::Opaque));
    }

    #[test]
    fn alpha_mode_opaque_display() {
        let desc = set_alpha(PixelDescriptor::RGBA8_SRGB, Some(AlphaMode::Opaque));
        let s = format!("{desc}");
        // Should show "RGBA8/sRGB/opaque"
        assert_eq!(s, "RGBA8/sRGB/opaque");
    }

    #[test]
    fn alpha_mode_undefined_display() {
        // RGBX8 with Undefined should show "/undefined"
        let s = format!("{}", PixelDescriptor::RGBX8_SRGB);
        assert_eq!(s, "RGBX8/sRGB/undefined");
    }

    #[test]
    fn alpha_mode_straight_display_no_alpha_suffix() {
        // Straight is the default for alpha formats — should NOT show "/straight"
        let s = format!("{}", PixelDescriptor::RGBA8_SRGB);
        assert_eq!(s, "RGBA8/sRGB");
    }

    #[test]
    fn alpha_mode_accessor() {
        assert_eq!(PixelDescriptor::RGB8_SRGB.alpha_mode(), None);
        assert_eq!(PixelDescriptor::RGBA8_SRGB.alpha_mode(), Some(AlphaMode::Straight));
        assert_eq!(PixelDescriptor::RGBX8_SRGB.alpha_mode(), Some(AlphaMode::Undefined));
    }
}

#[cfg(all(test, feature = "codec"))]
mod codec_tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    use rgb::alt::BGRA;
    use rgb::{Gray, Rgb, Rgba};

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
        let slice: PixelSlice<'_, Rgb<u8>> = img.into();
        assert_eq!(slice.width(), 2);
        assert_eq!(slice.rows(), 2);
        assert_eq!(slice.row(0), &[10, 20, 30, 40, 50, 60]);
        assert_eq!(slice.row(1), &[70, 80, 90, 100, 110, 120]);
    }

    #[test]
    fn imgref_to_pixel_slice_gray16() {
        let pixels = vec![Gray::new(1000u16), Gray::new(2000u16)];
        let img = imgref::Img::new(pixels.as_slice(), 2, 1);
        let slice: PixelSlice<'_, Gray<u16>> = img.into();
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

    // --- PixelBuffer format conversion tests ---

    #[test]
    fn convert_rgb8_to_rgba8() {
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
        ];
        let buf = PixelBuffer::from_pixels(pixels, 2, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let rgba = erased.to_rgba8();
        assert_eq!(rgba.width(), 2);
        assert_eq!(rgba.height(), 1);
        let s = rgba.as_slice();
        assert_eq!(s.row(0), &[10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn convert_rgba8_to_rgb8() {
        let pixels: Vec<Rgba<u8>> = vec![Rgba {
            r: 10,
            g: 20,
            b: 30,
            a: 128,
        }];
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let rgb = erased.to_rgb8();
        let s = rgb.as_slice();
        assert_eq!(s.row(0), &[10, 20, 30]);
    }

    #[test]
    fn convert_gray8_to_rgb8() {
        let pixels = vec![Gray::new(128u8), Gray::new(64u8)];
        let buf = PixelBuffer::from_pixels(pixels, 2, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let rgb = erased.to_rgb8();
        let s = rgb.as_slice();
        assert_eq!(s.row(0), &[128, 128, 128, 64, 64, 64]);
    }

    #[test]
    fn convert_gray8_to_rgba8() {
        let pixels = vec![Gray::new(200u8)];
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let rgba = erased.to_rgba8();
        let s = rgba.as_slice();
        assert_eq!(s.row(0), &[200, 200, 200, 255]);
    }

    #[test]
    fn convert_bgra8_to_rgb8() {
        let pixels: Vec<BGRA<u8>> = vec![BGRA {
            b: 10,
            g: 20,
            r: 30,
            a: 255,
        }];
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let rgb = erased.to_rgb8();
        let s = rgb.as_slice();
        // rgb order: r=30, g=20, b=10
        assert_eq!(s.row(0), &[30, 20, 10]);
    }

    #[test]
    fn convert_bgra8_to_rgba8() {
        let pixels: Vec<BGRA<u8>> = vec![BGRA {
            b: 10,
            g: 20,
            r: 30,
            a: 128,
        }];
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let rgba = erased.to_rgba8();
        let s = rgba.as_slice();
        assert_eq!(s.row(0), &[30, 20, 10, 128]);
    }

    #[test]
    fn convert_rgb8_to_gray8() {
        // BT.601: luma = (77*r + 150*g + 29*b) >> 8
        let pixels: Vec<Rgb<u8>> = vec![Rgb { r: 255, g: 0, b: 0 }]; // pure red
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let gray = erased.to_gray8();
        let s = gray.as_slice();
        let expected = ((77u32 * 255) >> 8) as u8; // 76
        assert_eq!(s.row(0), &[expected]);
    }

    #[test]
    fn convert_rgb8_to_bgra8() {
        let pixels: Vec<Rgb<u8>> = vec![Rgb {
            r: 10,
            g: 20,
            b: 30,
        }];
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let bgra = erased.to_bgra8();
        let s = bgra.as_slice();
        // BGRA order: b=30, g=20, r=10, a=255
        assert_eq!(s.row(0), &[30, 20, 10, 255]);
    }

    #[test]
    fn convert_rgba8_to_bgra8() {
        let pixels: Vec<Rgba<u8>> = vec![Rgba {
            r: 10,
            g: 20,
            b: 30,
            a: 128,
        }];
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let bgra = erased.to_bgra8();
        let s = bgra.as_slice();
        assert_eq!(s.row(0), &[30, 20, 10, 128]);
    }

    #[test]
    fn convert_u16_to_rgb8() {
        // u16 65535 → u8 255, u16 0 → u8 0
        let pixels: Vec<Rgb<u16>> = vec![Rgb {
            r: 65535,
            g: 0,
            b: 32768,
        }];
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let rgb = erased.to_rgb8();
        let s = rgb.as_slice();
        let row = s.row(0);
        assert_eq!(row[0], 255); // 65535 → 255
        assert_eq!(row[1], 0); // 0 → 0
        // 32768 → (32768*255+32768)>>16 = (8388608)>>16 = 128
        assert_eq!(row[2], 128);
    }

    #[test]
    fn convert_f32_to_rgba8() {
        let pixels: Vec<Rgba<f32>> = vec![Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.5,
            a: 0.75,
        }];
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let rgba = erased.to_rgba8();
        let s = rgba.as_slice();
        let row = s.row(0);
        assert_eq!(row[0], 255); // 1.0
        assert_eq!(row[1], 0); // 0.0
        assert_eq!(row[2], 127); // 0.5 * 255 = 127.5 → 127
        assert_eq!(row[3], 191); // 0.75 * 255 = 191.25 → 191
    }

    #[test]
    fn convert_grayalpha8_to_rgba8() {
        // GrayAlpha8 needs manual buffer construction since GrayAlpha lacks bytemuck
        let mut buf = PixelBuffer::new(1, 1, PixelDescriptor::GRAYA8_SRGB);
        {
            let mut s = buf.as_slice_mut();
            let row = s.row_mut(0);
            row[0] = 100; // gray value
            row[1] = 200; // alpha
        }
        let rgba = buf.to_rgba8();
        let s = rgba.as_slice();
        assert_eq!(s.row(0), &[100, 100, 100, 200]);
    }

    #[test]
    fn convert_preserves_multirow() {
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
        let buf = PixelBuffer::from_pixels(pixels, 2, 2).unwrap();
        let erased: PixelBuffer = buf.into();
        let rgba = erased.to_rgba8();
        assert_eq!(rgba.width(), 2);
        assert_eq!(rgba.height(), 2);
        let s = rgba.as_slice();
        assert_eq!(s.row(0), &[10, 20, 30, 255, 40, 50, 60, 255]);
        assert_eq!(s.row(1), &[70, 80, 90, 255, 100, 110, 120, 255]);
    }

    #[test]
    fn convert_u16_gray_to_gray8() {
        let pixels = vec![Gray::new(65535u16), Gray::new(0u16)];
        let buf = PixelBuffer::from_pixels(pixels, 2, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let gray = erased.to_gray8();
        let s = gray.as_slice();
        assert_eq!(s.row(0), &[255, 0]);
    }

    #[test]
    fn convert_f32_rgb_to_gray8() {
        // Pure white → 255 via luma
        let pixels: Vec<Rgb<f32>> = vec![Rgb {
            r: 1.0,
            g: 1.0,
            b: 1.0,
        }];
        let buf = PixelBuffer::from_pixels(pixels, 1, 1).unwrap();
        let erased: PixelBuffer = buf.into();
        let gray = erased.to_gray8();
        let s = gray.as_slice();
        // luma of (255,255,255) = (77*255+150*255+29*255)>>8 = (65280)>>8 = 255
        assert_eq!(s.row(0), &[255]);
    }

    // --- from_pixels_erased ---

    #[test]
    fn from_pixels_erased_matches_manual() {
        let pixels1: Vec<Rgb<u8>> = vec![
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
        ];
        let pixels2 = pixels1.clone();

        // Manual: from_pixels + into
        let manual: PixelBuffer = PixelBuffer::from_pixels(pixels1, 2, 1).unwrap().into();

        // Erased: from_pixels_erased
        let erased = PixelBuffer::from_pixels_erased(pixels2, 2, 1).unwrap();

        assert_eq!(manual.width(), erased.width());
        assert_eq!(manual.height(), erased.height());
        assert_eq!(manual.descriptor(), erased.descriptor());
        assert_eq!(manual.as_slice().row(0), erased.as_slice().row(0));
    }

    #[test]
    fn from_pixels_erased_dimension_mismatch() {
        let pixels: Vec<Rgb<u8>> = vec![Rgb { r: 1, g: 2, b: 3 }];
        let err = PixelBuffer::from_pixels_erased(pixels, 2, 1);
        assert_eq!(err.unwrap_err(), BufferError::InvalidDimensions);
    }

}
