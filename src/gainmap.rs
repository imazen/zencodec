//! Cross-codec gain map types (ISO 21496-1).
//!
//! Canonical representation for gain map metadata used by JPEG (UltraHDR),
//! AVIF (tmap), JXL (jhgm), and HEIF. All gains and headroom values are
//! stored in **log2 domain** to match the ISO 21496-1 wire format and avoid
//! domain confusion between codecs.
//!
//! # Domain conventions
//!
//! | Field | Domain | Example |
//! |-------|--------|---------|
//! | `channels[i].min` | log2 | −1.0 means ½× brightness |
//! | `channels[i].max` | log2 | 2.0 means 4× brightness |
//! | `channels[i].gamma` | linear | 1.0 = linear gain map encoding |
//! | `channels[i].base_offset` | linear | 1/64 default |
//! | `channels[i].alternate_offset` | linear | 1/64 default |
//! | `base_hdr_headroom` | log2 | 0.0 = SDR (1:1) |
//! | `alternate_hdr_headroom` | log2 | 1.3 ≈ 2.46× peak brightness |

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::info::Cicp;

// =========================================================================
// Wire format
// =========================================================================

/// Wire format variant for ISO 21496-1 binary metadata.
///
/// The `GainMapMetadata` payload (flags + fractions) is identical in all contexts.
/// The difference is whether the payload is prefixed with a `version(u8)` byte:
///
/// | Context | Envelope | Use this variant |
/// |---------|----------|-----------------|
/// | JPEG APP2 | `min_ver(u16) + writer_ver(u16) + payload` | [`Iso21496Format::JpegApp2`] |
/// | JXL `jhgm` | `min_ver(u16) + writer_ver(u16) + payload` | [`Iso21496Format::JpegApp2`] |
/// | AVIF `tmap` | `version(u8) + min_ver(u16) + writer_ver(u16) + payload` | [`Iso21496Format::AvifTmap`] |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Iso21496Format {
    /// JPEG APP2 and JXL `jhgm` format: no version byte prefix.
    ///
    /// The payload starts with `minimum_version(u16)`. Used by libultrahdr
    /// for JPEG and by libjxl for the `gain_map_metadata` field in `jhgm` boxes.
    JpegApp2,
    /// AVIF `tmap` item format: includes `version(u8)` prefix.
    ///
    /// The `ToneMapImage` box wraps `GainMapMetadata` with a version byte.
    /// Used by libavif for the `tmap` derived image item payload.
    AvifTmap,
}

// =========================================================================
// Core types
// =========================================================================

/// Per-channel gain map parameters.
///
/// Gains (`min`, `max`) are in log2 domain. Gamma and offsets are in linear domain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GainMapChannel {
    /// Log2 of minimum gain (can be negative, e.g., −1.0 = half brightness).
    pub min: f64,
    /// Log2 of maximum gain (typically ≥ min).
    pub max: f64,
    /// Gamma applied to gain map values. Linear domain, must be > 0.
    pub gamma: f64,
    /// Offset added to base image values before gain application. Linear domain.
    pub base_offset: f64,
    /// Offset added to alternate image values before gain application. Linear domain.
    pub alternate_offset: f64,
}

impl Default for GainMapChannel {
    fn default() -> Self {
        Self {
            min: 0.0, // log2(1.0) = 0
            max: 0.0, // log2(1.0) = 0
            gamma: 1.0,
            base_offset: 1.0 / 64.0, // ISO 21496-1 default
            alternate_offset: 1.0 / 64.0,
        }
    }
}

impl GainMapChannel {
    /// Minimum gain in linear domain: 2^min.
    pub fn linear_min(&self) -> f64 {
        2.0f64.powf(self.min)
    }

    /// Maximum gain in linear domain: 2^max.
    pub fn linear_max(&self) -> f64 {
        2.0f64.powf(self.max)
    }
}

/// ISO 21496-1 gain map parameters. Canonical cross-codec representation.
///
/// Gains and headroom are in **log2 domain**. Gamma and offsets are in linear
/// domain. This matches the ISO 21496-1 wire format directly, avoiding the
/// domain confusion that occurs when converting between log2 and linear
/// representations.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GainMapParams {
    /// Per-channel parameters. `[0]` = R (or all channels if single-channel),
    /// `[1]` = G, `[2]` = B.
    pub channels: [GainMapChannel; 3],
    /// Log2 of base image HDR headroom. 0.0 = SDR (peak luminance ratio 1:1).
    pub base_hdr_headroom: f64,
    /// Log2 of alternate image HDR headroom.
    pub alternate_hdr_headroom: f64,
    /// Whether the gain map is encoded in the base image's color space.
    pub use_base_color_space: bool,
    /// ISO 21496-1 backward direction flag (bit 2 of flags byte).
    ///
    /// When `true`, the base and alternate roles are reversed: the base
    /// image is the HDR rendition and the alternate is SDR. This is the
    /// authoritative encoder signal — [`direction()`](Self::direction)
    /// derives the same information from headroom comparison as a fallback.
    pub backward_direction: bool,
}

impl Default for GainMapParams {
    fn default() -> Self {
        Self {
            channels: [GainMapChannel::default(); 3],
            base_hdr_headroom: 0.0,
            alternate_hdr_headroom: 0.0,
            use_base_color_space: true,
            backward_direction: false,
        }
    }
}

impl GainMapParams {
    /// Whether all three channels have identical parameters.
    pub fn is_single_channel(&self) -> bool {
        self.channels[0] == self.channels[1] && self.channels[1] == self.channels[2]
    }

    /// Derive direction from headroom comparison.
    ///
    /// The image with greater headroom is the HDR rendition.
    pub fn direction(&self) -> GainMapDirection {
        if self.base_hdr_headroom > self.alternate_hdr_headroom {
            GainMapDirection::BaseIsHdr
        } else {
            GainMapDirection::BaseIsSdr
        }
    }

    /// Base HDR headroom in linear domain: 2^base_hdr_headroom.
    pub fn linear_base_headroom(&self) -> f64 {
        2.0f64.powf(self.base_hdr_headroom)
    }

    /// Alternate HDR headroom in linear domain: 2^alternate_hdr_headroom.
    pub fn linear_alternate_headroom(&self) -> f64 {
        2.0f64.powf(self.alternate_hdr_headroom)
    }

    /// Validate parameters for correctness.
    ///
    /// Checks for NaN/infinity, positive gamma, and min ≤ max per channel.
    pub fn validate(&self) -> Result<(), GainMapParseError> {
        if !self.base_hdr_headroom.is_finite() {
            return Err(GainMapParseError::NonFiniteValue {
                field: "base_hdr_headroom",
            });
        }
        if !self.alternate_hdr_headroom.is_finite() {
            return Err(GainMapParseError::NonFiniteValue {
                field: "alternate_hdr_headroom",
            });
        }
        for (i, ch) in self.channels.iter().enumerate() {
            if !ch.min.is_finite() {
                return Err(GainMapParseError::NonFiniteValue {
                    field: "channel min",
                });
            }
            if !ch.max.is_finite() {
                return Err(GainMapParseError::NonFiniteValue {
                    field: "channel max",
                });
            }
            if !ch.gamma.is_finite() || ch.gamma <= 0.0 {
                return Err(GainMapParseError::InvalidGamma {
                    channel: i,
                    value: ch.gamma,
                });
            }
            if !ch.base_offset.is_finite() {
                return Err(GainMapParseError::NonFiniteValue {
                    field: "base_offset",
                });
            }
            if !ch.alternate_offset.is_finite() {
                return Err(GainMapParseError::NonFiniteValue {
                    field: "alternate_offset",
                });
            }
            if ch.min > ch.max {
                return Err(GainMapParseError::MinExceedsMax {
                    channel: i,
                    min: ch.min,
                    max: ch.max,
                });
            }
        }
        Ok(())
    }
}

/// Whether the base image is SDR or HDR. Derived from headroom comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GainMapDirection {
    /// Base image is SDR, alternate is HDR. Typical for JPEG and AVIF.
    BaseIsSdr,
    /// Base image is HDR, alternate is SDR. Typical for JXL.
    BaseIsHdr,
}

/// Complete gain map description: parameters + image properties + alternate color.
///
/// Returned from probing when a gain map is detected. Contains enough
/// information to describe the gain map without carrying pixel data.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GainMapInfo {
    /// ISO 21496-1 gain map parameters.
    pub params: GainMapParams,
    /// Gain map image width in pixels.
    pub width: u32,
    /// Gain map image height in pixels.
    pub height: u32,
    /// Number of gain map channels: 1 (luminance) or 3 (per-channel RGB).
    pub channels: u8,
    /// CICP color description of the alternate (typically HDR) rendition.
    pub alternate_cicp: Option<Cicp>,
    /// ICC profile of the alternate rendition.
    pub alternate_icc: Option<Arc<[u8]>>,
}

impl GainMapInfo {
    /// Create with required fields. Optional fields default to `None`.
    pub fn new(params: GainMapParams, width: u32, height: u32, channels: u8) -> Self {
        Self {
            params,
            width,
            height,
            channels,
            alternate_cicp: None,
            alternate_icc: None,
        }
    }

    /// Set the alternate rendition's CICP color description.
    pub fn with_alternate_cicp(mut self, cicp: Cicp) -> Self {
        self.alternate_cicp = Some(cicp);
        self
    }

    /// Set the alternate rendition's ICC profile.
    pub fn with_alternate_icc(mut self, icc: impl Into<Arc<[u8]>>) -> Self {
        self.alternate_icc = Some(icc.into());
        self
    }
}

/// Gain map detection state during probe.
///
/// Three-state presence indicator:
/// - `Unknown` — can't determine from bytes probed (gain map may be beyond probe window)
/// - `Absent` — definitively no gain map in this file
/// - `Available` — gain map found and metadata parsed
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum GainMapPresence {
    /// Cannot determine gain map presence from bytes probed so far.
    #[default]
    Unknown,
    /// File definitively has no gain map.
    Absent,
    /// Gain map present, metadata parsed.
    Available(Box<GainMapInfo>),
}

impl GainMapPresence {
    /// Whether a gain map is definitely present.
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    /// Whether a gain map is definitively absent.
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    /// Whether gain map presence is unknown.
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// Reference to the gain map info, if available.
    pub fn info(&self) -> Option<&GainMapInfo> {
        match self {
            Self::Available(info) => Some(info),
            _ => None,
        }
    }

    /// Consume and return the gain map info, if available.
    pub fn into_info(self) -> Option<Box<GainMapInfo>> {
        match self {
            Self::Available(info) => Some(info),
            _ => None,
        }
    }
}

// =========================================================================
// Gain map source (raw, pre-decode)
// =========================================================================

/// Raw gain map data extracted from container — not yet pixel-decoded.
///
/// Produced by codecs when gain map extraction is opted in. The caller
/// decodes the raw bitstream through the normal codec path (with limits,
/// cancellation, streaming). This avoids hidden nested decodes inside
/// the primary decoder.
///
/// # Recursion safety
///
/// The `depth` field tracks nesting level. Callers MUST reject
/// `depth >= MAX_DEPTH` (typically 1) to prevent infinite recursion —
/// a JXL gain map is a bare JXL codestream, and a JPEG UltraHDR gain
/// map is a full JPEG that could itself contain MPF references.
///
/// # Ownership
///
/// The `data` field is owned (`Vec<u8>`) for storage in
/// [`DecodeOutput`](crate::decode::DecodeOutput) extensions.
/// Codecs that can provide zero-copy access to the gain map bitstream
/// should offer a codec-specific API returning `&[u8]` for callers
/// that decode immediately without storing.
///
/// # Codec behavior
///
/// | Container | `format` | `data` contents |
/// |-----------|----------|-----------------|
/// | AVIF | `Avif` | Raw AV1 bitstream (OBUs) |
/// | JXL | `Jxl` | Bare JXL codestream (no container boxes) |
/// | JPEG (UltraHDR) | `Jpeg` | Complete JPEG file (MPF secondary image) |
/// | HEIC | — | Not produced — HEIC parser decodes gain map internally, use [`DecodedGainMap`] |
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GainMapSource {
    /// Raw encoded bitstream of the gain map image.
    pub data: alloc::vec::Vec<u8>,
    /// Codec format needed to decode `data`.
    pub format: crate::ImageFormat,
    /// ISO 21496-1 gain map metadata (parsed from container).
    pub metadata: GainMapInfo,
    /// Nesting depth. 0 = gain map of a primary image.
    /// Callers should reject `depth >= 1` to prevent recursion.
    pub depth: u8,
}

impl GainMapSource {
    /// Create a new gain map source.
    pub fn new(
        data: alloc::vec::Vec<u8>,
        format: crate::ImageFormat,
        metadata: GainMapInfo,
    ) -> Self {
        Self {
            data,
            format,
            metadata,
            depth: 0,
        }
    }

    /// Set the recursion depth.
    pub fn with_depth(mut self, depth: u8) -> Self {
        self.depth = depth;
        self
    }
}

// Decoded gain map (post-decode)
// =========================================================================

/// Decoded gain map image — cross-codec normalized type.
///
/// Produced either by:
/// - Decoding a [`GainMapSource`] through the normal codec path
/// - Codecs that decode the gain map internally (HEIC)
///
/// Stored in [`DecodeOutput`](crate::decode::DecodeOutput) extensions
/// via `output.with_extras(decoded_gain_map)`.
///
/// Gain map decode is opt-in — this is only present when the caller
/// explicitly requested gain map extraction.
#[derive(Debug)]
#[non_exhaustive]
pub struct DecodedGainMap {
    /// Gain map image pixels.
    pub pixels: zenpixels::PixelBuffer,
    /// ISO 21496-1 gain map metadata.
    pub metadata: GainMapInfo,
}

impl DecodedGainMap {
    /// Create a new decoded gain map.
    pub fn new(pixels: zenpixels::PixelBuffer, metadata: GainMapInfo) -> Self {
        Self { pixels, metadata }
    }
}

// =========================================================================
// ISO 21496-1 fractions
// =========================================================================

/// Convert a float to an unsigned fraction using continued fractions.
///
/// Ported from libultrahdr's `floatToUnsignedFractionImpl`. Produces canonical
/// compact fractions (e.g. `1/64` instead of `15625/1000000`) that match
/// what browsers and reference decoders expect.
fn float_to_unsigned_fraction(
    v: f32,
    max_numerator: u32,
    numerator: &mut u32,
    denominator: &mut u32,
) -> bool {
    let v = v as f64;
    if v.is_nan() || v < 0.0 || v > max_numerator as f64 {
        return false;
    }
    let max_d: u64 = if v <= 1.0 {
        u32::MAX as u64
    } else {
        (max_numerator as f64 / v).floor() as u64
    };
    *denominator = 1;
    let mut previous_d: u32 = 0;
    let mut current_v = v - v.floor();
    const MAX_ITER: usize = 39;
    for _ in 0..MAX_ITER {
        let num_double = (*denominator as f64) * v;
        if num_double > max_numerator as f64 {
            return false;
        }
        *numerator = num_double.round() as u32;
        if (num_double - (*numerator as f64)).abs() == 0.0 {
            return true;
        }
        current_v = 1.0 / current_v;
        let new_d = previous_d as f64 + current_v.floor() * (*denominator as f64);
        if new_d > max_d as f64 {
            return true;
        }
        previous_d = *denominator;
        if new_d > u32::MAX as f64 {
            return false;
        }
        *denominator = new_d as u32;
        current_v -= current_v.floor();
    }
    *numerator = ((*denominator as f64) * v).round() as u32;
    true
}

/// Signed rational fraction for ISO 21496-1 binary format.
///
/// Used for gain map min/max and offsets where negative values are valid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fraction {
    /// Signed numerator.
    pub numerator: i32,
    /// Unsigned denominator (0 is invalid).
    pub denominator: u32,
}

impl Fraction {
    /// Convert to f64. Returns 0.0 if denominator is zero.
    pub fn to_f64(self) -> f64 {
        if self.denominator == 0 {
            0.0
        } else {
            self.numerator as f64 / self.denominator as f64
        }
    }

    /// Create from f64 using continued fractions for canonical encoding.
    ///
    /// Finds the best rational approximation, matching the algorithm used by
    /// libultrahdr and canonical ISO 21496-1 encoders. Produces compact
    /// fractions (e.g. `1/64` instead of `15625/1000000`).
    pub fn from_f64(value: f64) -> Self {
        let mut numerator = 0u32;
        let mut denominator = 1u32;
        if !float_to_unsigned_fraction(
            value.abs() as f32,
            i32::MAX as u32,
            &mut numerator,
            &mut denominator,
        ) {
            return Self {
                numerator: 0,
                denominator: 1,
            };
        }
        Self {
            numerator: if value < 0.0 {
                -(numerator as i32)
            } else {
                numerator as i32
            },
            denominator,
        }
    }

    /// Backward-compatible alias for [`from_f64`](Self::from_f64).
    ///
    /// The `_denominator` parameter is ignored — the continued fraction
    /// algorithm finds the optimal denominator automatically.
    #[deprecated(since = "0.1.12", note = "Use `Fraction::from_f64(value)` instead")]
    pub fn from_f64_with_denom(value: f64, _denominator: u32) -> Self {
        Self::from_f64(value)
    }

    /// Whether this fraction has a valid (non-zero) denominator.
    pub fn is_valid(&self) -> bool {
        self.denominator != 0
    }
}

/// Unsigned rational fraction for ISO 21496-1 binary format.
///
/// Used for HDR headroom and gamma where values are always non-negative.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UFraction {
    /// Unsigned numerator.
    pub numerator: u32,
    /// Unsigned denominator (0 is invalid).
    pub denominator: u32,
}

impl UFraction {
    /// Convert to f64. Returns 0.0 if denominator is zero.
    pub fn to_f64(self) -> f64 {
        if self.denominator == 0 {
            0.0
        } else {
            self.numerator as f64 / self.denominator as f64
        }
    }

    /// Create from f64 using continued fractions for canonical encoding.
    ///
    /// Finds the best rational approximation, matching the algorithm used by
    /// libultrahdr and canonical ISO 21496-1 encoders. Clamps negative values to 0.
    pub fn from_f64(value: f64) -> Self {
        let value = value.max(0.0) as f32;
        let mut numerator = 0u32;
        let mut denominator = 1u32;
        if !float_to_unsigned_fraction(value, u32::MAX, &mut numerator, &mut denominator) {
            return Self {
                numerator: 0,
                denominator: 1,
            };
        }
        Self {
            numerator,
            denominator,
        }
    }

    /// Backward-compatible alias for [`from_f64`](Self::from_f64).
    ///
    /// The `_denominator` parameter is ignored — the continued fraction
    /// algorithm finds the optimal denominator automatically.
    #[deprecated(since = "0.1.12", note = "Use `UFraction::from_f64(value)` instead")]
    pub fn from_f64_with_denom(value: f64, _denominator: u32) -> Self {
        Self::from_f64(value)
    }

    /// Whether this fraction has a valid (non-zero) denominator.
    pub fn is_valid(&self) -> bool {
        self.denominator != 0
    }
}

// =========================================================================
// ISO 21496-1 parser/serializer
// =========================================================================

/// Errors from ISO 21496-1 parsing or validation.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GainMapParseError {
    /// Data is too short to contain the expected fields.
    TruncatedData { expected: usize, actual: usize },
    /// Unsupported metadata version.
    UnsupportedVersion { version: u8 },
    /// A fraction has a zero denominator.
    ZeroDenominator { field: &'static str },
    /// Gamma must be > 0 and finite.
    InvalidGamma { channel: usize, value: f64 },
    /// Channel min exceeds max.
    MinExceedsMax { channel: usize, min: f64, max: f64 },
    /// A value is NaN or infinity.
    NonFiniteValue { field: &'static str },
}

impl core::fmt::Display for GainMapParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TruncatedData { expected, actual } => {
                write!(
                    f,
                    "ISO 21496-1: data truncated (need {expected} bytes, got {actual})"
                )
            }
            Self::UnsupportedVersion { version } => {
                write!(f, "ISO 21496-1: unsupported version {version}")
            }
            Self::ZeroDenominator { field } => {
                write!(f, "ISO 21496-1: zero denominator in {field}")
            }
            Self::InvalidGamma { channel, value } => {
                write!(f, "ISO 21496-1: invalid gamma {value} on channel {channel}")
            }
            Self::MinExceedsMax { channel, min, max } => {
                write!(
                    f,
                    "ISO 21496-1: channel {channel} min ({min}) > max ({max})"
                )
            }
            Self::NonFiniteValue { field } => {
                write!(f, "ISO 21496-1: non-finite value in {field}")
            }
        }
    }
}

impl core::error::Error for GainMapParseError {}

/// Flag bit: multichannel gain map (bit 7 of flags byte).
const FLAG_MULTI_CHANNEL: u8 = 0x80;

/// Flag bit: gain map uses base image colour space (bit 6 of flags byte).
const FLAG_USE_BASE_COLOUR_SPACE: u8 = 0x40;

/// Flag bit: backward direction — base is HDR, alternate is SDR (bit 2).
const FLAG_BACKWARD_DIRECTION: u8 = 0x04;

/// Flag bit: common denominator encoding (bit 3).
/// When set, a single shared denominator precedes all numerators.
const FLAG_COMMON_DENOMINATOR: u8 = 0x08;

/// Header size for AVIF tmap: version (1) + minimum_version (2) + writer_version (2) + flags (1).
const AVIF_HEADER_SIZE: usize = 6;

/// Header size for JPEG APP2 / JXL jhgm: minimum_version (2) + writer_version (2) + flags (1).
const JPEG_HEADER_SIZE: usize = 5;

/// Size of one fraction pair (numerator + denominator = 8 bytes).
const FRACTION_SIZE: usize = 8;

/// Parse ISO 21496-1 binary metadata from AVIF `tmap` item payload.
///
/// This format includes a `version(u8)` prefix byte before the common header.
/// For JPEG APP2 or JXL `jhgm` (no version byte), use [`parse_iso21496_jpeg`].
///
/// Handles both full and common-denominator compact encodings.
pub fn parse_iso21496(data: &[u8]) -> Result<GainMapParams, GainMapParseError> {
    parse_iso21496_avif(data)
}

/// Parse ISO 21496-1 binary metadata without version byte prefix.
///
/// Used for JPEG APP2 and JXL `jhgm` formats where the containing marker
/// or box type already identifies the format.
///
/// Handles both full and common-denominator compact encodings.
pub fn parse_iso21496_jpeg(data: &[u8]) -> Result<GainMapParams, GainMapParseError> {
    parse_iso21496_no_version(data)
}

/// Serialize [`GainMapParams`] to ISO 21496-1 binary format (AVIF `tmap`).
///
/// Includes the `version(u8)` prefix byte. For JPEG APP2 or JXL `jhgm`,
/// use [`serialize_iso21496_jpeg`].
pub fn serialize_iso21496(params: &GainMapParams) -> Vec<u8> {
    serialize_iso21496_avif(params)
}

/// Serialize [`GainMapParams`] without version byte prefix (JPEG APP2 / JXL `jhgm`).
pub fn serialize_iso21496_jpeg(params: &GainMapParams) -> Vec<u8> {
    serialize_iso21496_no_version(params)
}

/// Parse ISO 21496-1 binary gain map metadata with explicit format selection.
///
/// The `format` parameter selects the wire format variant:
/// - [`Iso21496Format::AvifTmap`]: expects `version(u8)` prefix (AVIF `tmap` item payload)
/// - [`Iso21496Format::JpegApp2`]: no version prefix (JPEG APP2, JXL `jhgm`)
///
/// Both variants handle the common-denominator compact encoding (flag bit 3)
/// used by libultrahdr.
pub fn parse_iso21496_fmt(
    data: &[u8],
    format: Iso21496Format,
) -> Result<GainMapParams, GainMapParseError> {
    match format {
        Iso21496Format::AvifTmap => parse_iso21496_avif(data),
        Iso21496Format::JpegApp2 => parse_iso21496_no_version(data),
    }
}

/// Serialize [`GainMapParams`] to ISO 21496-1 binary format with explicit format selection.
///
/// The `format` parameter selects the wire format variant:
/// - [`Iso21496Format::AvifTmap`]: includes `version(u8)` prefix
/// - [`Iso21496Format::JpegApp2`]: no version prefix (also correct for JXL `jhgm`)
///
/// Always writes the full (non-compact) format for maximum compatibility.
pub fn serialize_iso21496_fmt(params: &GainMapParams, format: Iso21496Format) -> Vec<u8> {
    match format {
        Iso21496Format::AvifTmap => serialize_iso21496_avif(params),
        Iso21496Format::JpegApp2 => serialize_iso21496_no_version(params),
    }
}

// -- Internal format-specific entry points --

/// Parse ISO 21496-1 from AVIF `tmap` item payload (with version byte prefix).
fn parse_iso21496_avif(data: &[u8]) -> Result<GainMapParams, GainMapParseError> {
    if data.len() < AVIF_HEADER_SIZE {
        return Err(GainMapParseError::TruncatedData {
            expected: AVIF_HEADER_SIZE,
            actual: data.len(),
        });
    }

    let mut offset = 0;

    // version (u8) — must be 0
    let version = read_u8(data, &mut offset)?;
    if version != 0 {
        return Err(GainMapParseError::UnsupportedVersion { version });
    }

    parse_common_header_and_payload(data, &mut offset)
}

/// Parse ISO 21496-1 without version byte prefix (JPEG APP2 / JXL jhgm).
fn parse_iso21496_no_version(data: &[u8]) -> Result<GainMapParams, GainMapParseError> {
    if data.len() < JPEG_HEADER_SIZE {
        return Err(GainMapParseError::TruncatedData {
            expected: JPEG_HEADER_SIZE,
            actual: data.len(),
        });
    }

    let mut offset = 0;
    parse_common_header_and_payload(data, &mut offset)
}

/// Parse the common header (min_version + writer_version + flags) and payload.
/// Shared between AVIF and JPEG/JXL variants.
fn parse_common_header_and_payload(
    data: &[u8],
    offset: &mut usize,
) -> Result<GainMapParams, GainMapParseError> {
    // minimum_version (u16 BE) — must be 0
    let minimum_version = read_u16_be(data, offset)?;
    if minimum_version > 0 {
        return Err(GainMapParseError::UnsupportedVersion {
            version: minimum_version as u8,
        });
    }

    // writer_version (u16 BE) — informational
    let _writer_version = read_u16_be(data, offset)?;

    // flags (u8)
    let flags = read_u8(data, offset)?;
    let is_multichannel = (flags & FLAG_MULTI_CHANNEL) != 0;
    let use_base_color_space = (flags & FLAG_USE_BASE_COLOUR_SPACE) != 0;
    let backward_direction = (flags & FLAG_BACKWARD_DIRECTION) != 0;
    let common_denominator = (flags & FLAG_COMMON_DENOMINATOR) != 0;

    let num_channels: usize = if is_multichannel { 3 } else { 1 };

    if common_denominator {
        parse_payload_common_denom(
            data,
            offset,
            num_channels,
            use_base_color_space,
            backward_direction,
        )
    } else {
        parse_payload_full(
            data,
            offset,
            num_channels,
            use_base_color_space,
            backward_direction,
        )
    }
}

/// Parse the standard (full) payload: each value has its own denominator.
fn parse_payload_full(
    data: &[u8],
    offset: &mut usize,
    num_channels: usize,
    use_base_color_space: bool,
    backward_direction: bool,
) -> Result<GainMapParams, GainMapParseError> {
    let base_headroom = read_ufraction(data, offset, "base_hdr_headroom")?;
    let alt_headroom = read_ufraction(data, offset, "alternate_hdr_headroom")?;

    let mut channels = [GainMapChannel::default(); 3];
    for ch in channels.iter_mut().take(num_channels) {
        let min_frac = read_fraction(data, offset, "gain_map_min")?;
        let max_frac = read_fraction(data, offset, "gain_map_max")?;
        let gamma_frac = read_ufraction(data, offset, "gamma")?;
        let base_offset_frac = read_fraction(data, offset, "base_offset")?;
        let alt_offset_frac = read_fraction(data, offset, "alternate_offset")?;

        *ch = GainMapChannel {
            min: min_frac.to_f64(),
            max: max_frac.to_f64(),
            gamma: gamma_frac.to_f64(),
            base_offset: base_offset_frac.to_f64(),
            alternate_offset: alt_offset_frac.to_f64(),
        };
    }

    // Single-channel: replicate to all three
    if num_channels == 1 {
        channels[1] = channels[0];
        channels[2] = channels[0];
    }

    Ok(GainMapParams {
        channels,
        base_hdr_headroom: base_headroom.to_f64(),
        alternate_hdr_headroom: alt_headroom.to_f64(),
        use_base_color_space,
        backward_direction,
    })
}

/// Parse the compact payload: one shared denominator for all fractions.
///
/// Layout: `common_denom(u32)`, then numerator-only values with the shared
/// denominator applied to all of them. Used by libultrahdr when all fractions
/// happen to share the same denominator.
fn parse_payload_common_denom(
    data: &[u8],
    offset: &mut usize,
    num_channels: usize,
    use_base_color_space: bool,
    backward_direction: bool,
) -> Result<GainMapParams, GainMapParseError> {
    let common_d = read_u32_be(data, offset)?;
    if common_d == 0 {
        return Err(GainMapParseError::ZeroDenominator {
            field: "common_denominator",
        });
    }

    // Headroom (numerators only, unsigned)
    let base_headroom_n = read_u32_be(data, offset)?;
    let alt_headroom_n = read_u32_be(data, offset)?;

    let base_headroom = UFraction {
        numerator: base_headroom_n,
        denominator: common_d,
    };
    let alt_headroom = UFraction {
        numerator: alt_headroom_n,
        denominator: common_d,
    };

    let mut channels = [GainMapChannel::default(); 3];
    for ch in channels.iter_mut().take(num_channels) {
        let min_n = read_i32_be(data, offset)?;
        let max_n = read_i32_be(data, offset)?;
        let gamma_n = read_u32_be(data, offset)?;
        let base_off_n = read_i32_be(data, offset)?;
        let alt_off_n = read_i32_be(data, offset)?;

        *ch = GainMapChannel {
            min: Fraction {
                numerator: min_n,
                denominator: common_d,
            }
            .to_f64(),
            max: Fraction {
                numerator: max_n,
                denominator: common_d,
            }
            .to_f64(),
            gamma: UFraction {
                numerator: gamma_n,
                denominator: common_d,
            }
            .to_f64(),
            base_offset: Fraction {
                numerator: base_off_n,
                denominator: common_d,
            }
            .to_f64(),
            alternate_offset: Fraction {
                numerator: alt_off_n,
                denominator: common_d,
            }
            .to_f64(),
        };
    }

    if num_channels == 1 {
        channels[1] = channels[0];
        channels[2] = channels[0];
    }

    Ok(GainMapParams {
        channels,
        base_hdr_headroom: base_headroom.to_f64(),
        alternate_hdr_headroom: alt_headroom.to_f64(),
        use_base_color_space,
        backward_direction,
    })
}

// -- Serializers (always write full format, no common denominator) --

fn build_flags(params: &GainMapParams) -> u8 {
    let mut flags = 0u8;
    if !params.is_single_channel() {
        flags |= FLAG_MULTI_CHANNEL;
    }
    if params.use_base_color_space {
        flags |= FLAG_USE_BASE_COLOUR_SPACE;
    }
    if params.backward_direction {
        flags |= FLAG_BACKWARD_DIRECTION;
    }
    // Never set FLAG_COMMON_DENOMINATOR — always write full format.
    flags
}

fn write_payload(data: &mut Vec<u8>, params: &GainMapParams) {
    let num_channels: usize = if params.is_single_channel() { 1 } else { 3 };

    write_ufraction(data, UFraction::from_f64(params.base_hdr_headroom));
    write_ufraction(data, UFraction::from_f64(params.alternate_hdr_headroom));

    for ch in params.channels.iter().take(num_channels) {
        write_fraction(data, Fraction::from_f64(ch.min));
        write_fraction(data, Fraction::from_f64(ch.max));
        write_ufraction(data, UFraction::from_f64(ch.gamma));
        write_fraction(data, Fraction::from_f64(ch.base_offset));
        write_fraction(data, Fraction::from_f64(ch.alternate_offset));
    }
}

/// Serialize with AVIF `tmap` version byte prefix.
fn serialize_iso21496_avif(params: &GainMapParams) -> Vec<u8> {
    let num_channels: usize = if params.is_single_channel() { 1 } else { 3 };
    let size = AVIF_HEADER_SIZE + 2 * FRACTION_SIZE + num_channels * 5 * FRACTION_SIZE;
    let mut data = Vec::with_capacity(size);

    data.push(0u8); // version
    data.extend_from_slice(&0u16.to_be_bytes()); // minimum_version
    data.extend_from_slice(&0u16.to_be_bytes()); // writer_version
    data.push(build_flags(params));

    write_payload(&mut data, params);
    data
}

/// Serialize without version byte prefix (JPEG APP2 / JXL jhgm).
fn serialize_iso21496_no_version(params: &GainMapParams) -> Vec<u8> {
    let num_channels: usize = if params.is_single_channel() { 1 } else { 3 };
    let size = JPEG_HEADER_SIZE + 2 * FRACTION_SIZE + num_channels * 5 * FRACTION_SIZE;
    let mut data = Vec::with_capacity(size);

    // No version byte.
    data.extend_from_slice(&0u16.to_be_bytes()); // minimum_version
    data.extend_from_slice(&0u16.to_be_bytes()); // writer_version
    data.push(build_flags(params));

    write_payload(&mut data, params);
    data
}

// =========================================================================
// Internal helpers
// =========================================================================

fn read_u8(data: &[u8], offset: &mut usize) -> Result<u8, GainMapParseError> {
    if *offset >= data.len() {
        return Err(GainMapParseError::TruncatedData {
            expected: *offset + 1,
            actual: data.len(),
        });
    }
    let v = data[*offset];
    *offset += 1;
    Ok(v)
}

fn read_u16_be(data: &[u8], offset: &mut usize) -> Result<u16, GainMapParseError> {
    if *offset + 2 > data.len() {
        return Err(GainMapParseError::TruncatedData {
            expected: *offset + 2,
            actual: data.len(),
        });
    }
    let v = u16::from_be_bytes([data[*offset], data[*offset + 1]]);
    *offset += 2;
    Ok(v)
}

fn read_i32_be(data: &[u8], offset: &mut usize) -> Result<i32, GainMapParseError> {
    if *offset + 4 > data.len() {
        return Err(GainMapParseError::TruncatedData {
            expected: *offset + 4,
            actual: data.len(),
        });
    }
    let v = i32::from_be_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset += 4;
    Ok(v)
}

fn read_u32_be(data: &[u8], offset: &mut usize) -> Result<u32, GainMapParseError> {
    if *offset + 4 > data.len() {
        return Err(GainMapParseError::TruncatedData {
            expected: *offset + 4,
            actual: data.len(),
        });
    }
    let v = u32::from_be_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset += 4;
    Ok(v)
}

fn read_fraction(
    data: &[u8],
    offset: &mut usize,
    field: &'static str,
) -> Result<Fraction, GainMapParseError> {
    let n = read_i32_be(data, offset)?;
    let d = read_u32_be(data, offset)?;
    if d == 0 {
        return Err(GainMapParseError::ZeroDenominator { field });
    }
    Ok(Fraction {
        numerator: n,
        denominator: d,
    })
}

fn read_ufraction(
    data: &[u8],
    offset: &mut usize,
    field: &'static str,
) -> Result<UFraction, GainMapParseError> {
    let n = read_u32_be(data, offset)?;
    let d = read_u32_be(data, offset)?;
    if d == 0 {
        return Err(GainMapParseError::ZeroDenominator { field });
    }
    Ok(UFraction {
        numerator: n,
        denominator: d,
    })
}

fn write_fraction(data: &mut Vec<u8>, frac: Fraction) {
    data.extend_from_slice(&frac.numerator.to_be_bytes());
    data.extend_from_slice(&frac.denominator.to_be_bytes());
}

fn write_ufraction(data: &mut Vec<u8>, frac: UFraction) {
    data.extend_from_slice(&frac.numerator.to_be_bytes());
    data.extend_from_slice(&frac.denominator.to_be_bytes());
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- GainMapChannel ---

    #[test]
    fn channel_default() {
        let ch = GainMapChannel::default();
        assert_eq!(ch.min, 0.0);
        assert_eq!(ch.max, 0.0);
        assert_eq!(ch.gamma, 1.0);
        assert_eq!(ch.base_offset, 1.0 / 64.0);
        assert_eq!(ch.alternate_offset, 1.0 / 64.0);
    }

    #[test]
    fn channel_copy() {
        let ch = GainMapChannel {
            min: -1.0,
            max: 2.0,
            gamma: 1.0,
            base_offset: 0.0,
            alternate_offset: 0.0,
        };
        let ch2 = ch; // Copy
        assert_eq!(ch, ch2);
    }

    #[test]
    fn channel_linear_helpers() {
        let ch = GainMapChannel {
            min: 0.0,
            max: 2.0,
            gamma: 1.0,
            base_offset: 0.0,
            alternate_offset: 0.0,
        };
        assert!((ch.linear_min() - 1.0).abs() < 1e-10); // 2^0 = 1
        assert!((ch.linear_max() - 4.0).abs() < 1e-10); // 2^2 = 4
    }

    #[test]
    fn channel_linear_negative() {
        let ch = GainMapChannel {
            min: -1.0,
            max: 0.0,
            gamma: 1.0,
            base_offset: 0.0,
            alternate_offset: 0.0,
        };
        assert!((ch.linear_min() - 0.5).abs() < 1e-10); // 2^-1 = 0.5
        assert!((ch.linear_max() - 1.0).abs() < 1e-10); // 2^0 = 1
    }

    // --- GainMapParams ---

    #[test]
    fn params_default() {
        let p = GainMapParams::default();
        assert!(p.is_single_channel());
        assert_eq!(p.base_hdr_headroom, 0.0);
        assert_eq!(p.alternate_hdr_headroom, 0.0);
        assert!(p.use_base_color_space);
        assert_eq!(p.direction(), GainMapDirection::BaseIsSdr);
    }

    #[test]
    fn params_direction_sdr_base() {
        let p = GainMapParams {
            base_hdr_headroom: 0.0,
            alternate_hdr_headroom: 1.3,
            ..Default::default()
        };
        assert_eq!(p.direction(), GainMapDirection::BaseIsSdr);
    }

    #[test]
    fn params_direction_hdr_base() {
        let p = GainMapParams {
            base_hdr_headroom: 5.0,
            alternate_hdr_headroom: 0.0,
            ..Default::default()
        };
        assert_eq!(p.direction(), GainMapDirection::BaseIsHdr);
    }

    #[test]
    fn params_direction_equal_headroom() {
        let p = GainMapParams {
            base_hdr_headroom: 1.0,
            alternate_hdr_headroom: 1.0,
            ..Default::default()
        };
        // Equal headroom defaults to BaseIsSdr
        assert_eq!(p.direction(), GainMapDirection::BaseIsSdr);
    }

    #[test]
    fn params_is_single_channel() {
        let mut p = GainMapParams::default();
        assert!(p.is_single_channel());

        p.channels[1].max = 3.0;
        assert!(!p.is_single_channel());
    }

    #[test]
    fn params_linear_headroom() {
        let p = GainMapParams {
            base_hdr_headroom: 0.0,
            alternate_hdr_headroom: 1.3,
            ..Default::default()
        };
        assert!((p.linear_base_headroom() - 1.0).abs() < 1e-10);
        assert!((p.linear_alternate_headroom() - 2.0f64.powf(1.3)).abs() < 1e-10);
    }

    #[test]
    fn params_validate_ok() {
        let p = GainMapParams::default();
        assert!(p.validate().is_ok());
    }

    #[test]
    fn params_validate_nan_headroom() {
        let p = GainMapParams {
            base_hdr_headroom: f64::NAN,
            ..Default::default()
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn params_validate_inf_headroom() {
        let p = GainMapParams {
            alternate_hdr_headroom: f64::INFINITY,
            ..Default::default()
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn params_validate_zero_gamma() {
        let mut p = GainMapParams::default();
        p.channels[0].gamma = 0.0;
        let err = p.validate().unwrap_err();
        assert!(matches!(
            err,
            GainMapParseError::InvalidGamma { channel: 0, .. }
        ));
    }

    #[test]
    fn params_validate_negative_gamma() {
        let mut p = GainMapParams::default();
        p.channels[2].gamma = -0.5;
        let err = p.validate().unwrap_err();
        assert!(matches!(
            err,
            GainMapParseError::InvalidGamma { channel: 2, .. }
        ));
    }

    #[test]
    fn params_validate_min_exceeds_max() {
        let mut p = GainMapParams::default();
        p.channels[1].min = 3.0;
        p.channels[1].max = 1.0;
        let err = p.validate().unwrap_err();
        assert!(matches!(
            err,
            GainMapParseError::MinExceedsMax { channel: 1, .. }
        ));
    }

    #[test]
    fn params_validate_nan_channel() {
        let mut p = GainMapParams::default();
        p.channels[0].min = f64::NAN;
        assert!(p.validate().is_err());
    }

    #[test]
    fn params_validate_inf_offset() {
        let mut p = GainMapParams::default();
        p.channels[0].base_offset = f64::INFINITY;
        assert!(p.validate().is_err());
    }

    // --- GainMapPresence ---

    #[test]
    fn presence_default_is_unknown() {
        let p = GainMapPresence::default();
        assert!(p.is_unknown());
        assert!(!p.is_present());
        assert!(!p.is_absent());
        assert!(p.info().is_none());
    }

    #[test]
    fn presence_absent() {
        let p = GainMapPresence::Absent;
        assert!(p.is_absent());
        assert!(!p.is_present());
        assert!(!p.is_unknown());
        assert!(p.info().is_none());
    }

    #[test]
    fn presence_available() {
        let info = GainMapInfo::new(GainMapParams::default(), 128, 128, 1);
        let p = GainMapPresence::Available(Box::new(info));
        assert!(p.is_present());
        assert!(!p.is_absent());
        assert!(!p.is_unknown());

        let i = p.info().unwrap();
        assert_eq!(i.width, 128);
        assert_eq!(i.height, 128);
        assert_eq!(i.channels, 1);
    }

    #[test]
    fn presence_into_info() {
        let info = GainMapInfo::new(GainMapParams::default(), 64, 64, 3);
        let p = GainMapPresence::Available(Box::new(info));
        let i = p.into_info().unwrap();
        assert_eq!(i.width, 64);
        assert_eq!(i.channels, 3);
    }

    #[test]
    fn presence_into_info_none() {
        assert!(GainMapPresence::Unknown.into_info().is_none());
        assert!(GainMapPresence::Absent.into_info().is_none());
    }

    // --- GainMapInfo ---

    #[test]
    fn info_builder() {
        let info = GainMapInfo::new(GainMapParams::default(), 256, 256, 1)
            .with_alternate_cicp(Cicp::BT2100_PQ)
            .with_alternate_icc(alloc::vec![1, 2, 3]);
        assert_eq!(info.alternate_cicp, Some(Cicp::BT2100_PQ));
        assert_eq!(info.alternate_icc.as_deref(), Some([1, 2, 3].as_slice()));
    }

    // --- Fraction ---

    #[test]
    fn fraction_roundtrip() {
        let f = Fraction::from_f64(1.5);
        assert!((f.to_f64() - 1.5).abs() < 1e-6);
    }

    #[test]
    fn fraction_negative() {
        let f = Fraction::from_f64(-0.256907);
        assert!((f.to_f64() - (-0.256907)).abs() < 1e-6);
    }

    #[test]
    fn fraction_zero_denom() {
        let f = Fraction {
            numerator: 42,
            denominator: 0,
        };
        assert_eq!(f.to_f64(), 0.0);
        assert!(!f.is_valid());
    }

    #[test]
    fn fraction_default() {
        let f = Fraction::default();
        assert_eq!(f.numerator, 0);
        assert_eq!(f.denominator, 0);
        assert!(!f.is_valid());
    }

    // --- UFraction ---

    #[test]
    fn ufraction_roundtrip() {
        let f = UFraction::from_f64(1.3);
        assert!((f.to_f64() - 1.3).abs() < 1e-6);
    }

    #[test]
    fn ufraction_clamps_negative() {
        let f = UFraction::from_f64(-5.0);
        assert_eq!(f.numerator, 0);
        assert_eq!(f.to_f64(), 0.0);
    }

    #[test]
    fn ufraction_zero_denom() {
        let f = UFraction {
            numerator: 42,
            denominator: 0,
        };
        assert_eq!(f.to_f64(), 0.0);
        assert!(!f.is_valid());
    }

    // --- parse_iso21496 ---

    #[test]
    fn parse_roundtrip_single_channel() {
        let original = GainMapParams {
            channels: [GainMapChannel {
                min: -0.5,
                max: 2.0,
                gamma: 1.0,
                base_offset: 1.0 / 64.0,
                alternate_offset: 1.0 / 64.0,
            }; 3],
            base_hdr_headroom: 0.0,
            alternate_hdr_headroom: 1.3,
            use_base_color_space: true,
            backward_direction: false,
        };

        let blob = serialize_iso21496(&original);
        assert_eq!(blob.len(), 62); // 6 + 16 + 1*40

        let parsed = parse_iso21496(&blob).unwrap();
        assert!(parsed.is_single_channel());
        assert!((parsed.base_hdr_headroom - 0.0).abs() < 1e-6);
        assert!((parsed.alternate_hdr_headroom - 1.3).abs() < 1e-6);
        assert!((parsed.channels[0].min - (-0.5)).abs() < 1e-6);
        assert!((parsed.channels[0].max - 2.0).abs() < 1e-6);
        assert!((parsed.channels[0].gamma - 1.0).abs() < 1e-6);
        assert!(parsed.use_base_color_space);
    }

    #[test]
    fn parse_roundtrip_multi_channel() {
        let original = GainMapParams {
            channels: [
                GainMapChannel {
                    min: -0.3,
                    max: 2.0,
                    gamma: 1.0,
                    base_offset: 0.01,
                    alternate_offset: 0.02,
                },
                GainMapChannel {
                    min: -0.1,
                    max: 1.5,
                    gamma: 0.8,
                    base_offset: 0.01,
                    alternate_offset: 0.02,
                },
                GainMapChannel {
                    min: -0.5,
                    max: 2.5,
                    gamma: 1.2,
                    base_offset: 0.01,
                    alternate_offset: 0.02,
                },
            ],
            base_hdr_headroom: 0.0,
            alternate_hdr_headroom: 1.3,
            use_base_color_space: false,
            backward_direction: false,
        };

        let blob = serialize_iso21496(&original);
        assert_eq!(blob.len(), 142); // 6 + 16 + 3*40

        let parsed = parse_iso21496(&blob).unwrap();
        assert!(!parsed.is_single_channel());
        assert!(!parsed.use_base_color_space);

        for i in 0..3 {
            assert!(
                (parsed.channels[i].min - original.channels[i].min).abs() < 1e-6,
                "channel {i} min"
            );
            assert!(
                (parsed.channels[i].max - original.channels[i].max).abs() < 1e-6,
                "channel {i} max"
            );
            assert!(
                (parsed.channels[i].gamma - original.channels[i].gamma).abs() < 1e-6,
                "channel {i} gamma"
            );
        }
    }

    #[test]
    fn parse_known_blob() {
        // Construct a known ISO 21496-1 binary blob manually.
        // Single channel, use_base_color_space=true, base=SDR, alt headroom=13/10=1.3
        let mut blob = Vec::new();
        // Header
        blob.push(0); // version
        blob.extend_from_slice(&0u16.to_be_bytes()); // min version
        blob.extend_from_slice(&0u16.to_be_bytes()); // writer version
        blob.push(0x40); // flags: single channel, use_base_color_space
        // Headroom
        blob.extend_from_slice(&0u32.to_be_bytes()); // base_headroom_n = 0
        blob.extend_from_slice(&1u32.to_be_bytes()); // base_headroom_d = 1
        blob.extend_from_slice(&13u32.to_be_bytes()); // alt_headroom_n = 13
        blob.extend_from_slice(&10u32.to_be_bytes()); // alt_headroom_d = 10
        // Channel 0
        blob.extend_from_slice(&0i32.to_be_bytes()); // min_n = 0
        blob.extend_from_slice(&1u32.to_be_bytes()); // min_d = 1
        blob.extend_from_slice(&2i32.to_be_bytes()); // max_n = 2
        blob.extend_from_slice(&1u32.to_be_bytes()); // max_d = 1
        blob.extend_from_slice(&1u32.to_be_bytes()); // gamma_n = 1
        blob.extend_from_slice(&1u32.to_be_bytes()); // gamma_d = 1
        blob.extend_from_slice(&1i32.to_be_bytes()); // base_offset_n = 1
        blob.extend_from_slice(&64u32.to_be_bytes()); // base_offset_d = 64
        blob.extend_from_slice(&1i32.to_be_bytes()); // alt_offset_n = 1
        blob.extend_from_slice(&64u32.to_be_bytes()); // alt_offset_d = 64

        let params = parse_iso21496(&blob).unwrap();
        assert_eq!(params.base_hdr_headroom, 0.0);
        assert!((params.alternate_hdr_headroom - 1.3).abs() < 1e-10);
        assert_eq!(params.channels[0].min, 0.0);
        assert_eq!(params.channels[0].max, 2.0);
        assert_eq!(params.channels[0].gamma, 1.0);
        assert_eq!(params.channels[0].base_offset, 1.0 / 64.0);
        assert!(params.is_single_channel());
        assert!(params.use_base_color_space);
        assert_eq!(params.direction(), GainMapDirection::BaseIsSdr);

        // The alternate headroom in linear domain should be 2^1.3 ≈ 2.462
        assert!((params.linear_alternate_headroom() - 2.0f64.powf(1.3)).abs() < 1e-10);
    }

    #[test]
    fn parse_truncated() {
        assert!(parse_iso21496(&[]).is_err());
        assert!(parse_iso21496(&[0]).is_err());
        assert!(parse_iso21496(&[0; 5]).is_err());
        // 6 bytes header OK, but not enough for headroom
        assert!(parse_iso21496(&[0, 0, 0, 0, 0, 0x40]).is_err());
    }

    #[test]
    fn parse_wrong_version() {
        let mut blob = alloc::vec![0u8; 62];
        blob[0] = 1; // unsupported version
        let err = parse_iso21496(&blob).unwrap_err();
        assert!(matches!(
            err,
            GainMapParseError::UnsupportedVersion { version: 1 }
        ));
    }

    #[test]
    fn parse_wrong_min_version() {
        let mut blob = alloc::vec![0u8; 62];
        blob[0] = 0; // version OK
        blob[1] = 0;
        blob[2] = 1; // minimum_version = 1 (unsupported)
        let err = parse_iso21496(&blob).unwrap_err();
        assert!(matches!(err, GainMapParseError::UnsupportedVersion { .. }));
    }

    #[test]
    fn parse_zero_denominator() {
        // Build a blob with zero denominator in base_headroom
        let mut blob = Vec::new();
        blob.push(0); // version
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.push(0x40); // flags
        blob.extend_from_slice(&0u32.to_be_bytes()); // base_headroom_n
        blob.extend_from_slice(&0u32.to_be_bytes()); // base_headroom_d = 0 !
        // pad to avoid truncation error before we hit zero-denom
        blob.extend_from_slice(&[0; 100]);

        let err = parse_iso21496(&blob).unwrap_err();
        assert!(matches!(err, GainMapParseError::ZeroDenominator { .. }));
    }

    // --- serialize_iso21496 ---

    #[test]
    fn serialize_single_channel_size() {
        let p = GainMapParams::default();
        assert!(p.is_single_channel());
        assert_eq!(serialize_iso21496(&p).len(), 62);
    }

    #[test]
    fn serialize_multi_channel_size() {
        let mut p = GainMapParams::default();
        p.channels[1].max = 3.0; // make multichannel
        assert!(!p.is_single_channel());
        assert_eq!(serialize_iso21496(&p).len(), 142);
    }

    // --- GainMapParseError ---

    #[test]
    fn error_display() {
        let e = GainMapParseError::TruncatedData {
            expected: 62,
            actual: 10,
        };
        let s = alloc::format!("{e}");
        assert!(s.contains("truncated"));
        assert!(s.contains("62"));
    }

    #[test]
    fn error_is_error() {
        let e = GainMapParseError::UnsupportedVersion { version: 1 };
        let _: &dyn core::error::Error = &e;
    }

    // --- GainMapDirection ---

    #[test]
    fn direction_copy() {
        let d = GainMapDirection::BaseIsSdr;
        let d2 = d;
        assert_eq!(d, d2);
    }

    // --- Additional coverage tests ---

    #[test]
    fn channel_custom_values() {
        let ch = GainMapChannel {
            min: -2.5,
            max: 3.7,
            gamma: 2.2,
            base_offset: 0.05,
            alternate_offset: 0.1,
        };
        assert_eq!(ch.min, -2.5);
        assert_eq!(ch.max, 3.7);
        assert_eq!(ch.gamma, 2.2);
        assert_eq!(ch.base_offset, 0.05);
        assert_eq!(ch.alternate_offset, 0.1);
    }

    #[test]
    fn params_multi_channel_different_values() {
        let p = GainMapParams {
            channels: [
                GainMapChannel {
                    min: -1.0,
                    max: 2.0,
                    gamma: 1.0,
                    base_offset: 0.01,
                    alternate_offset: 0.02,
                },
                GainMapChannel {
                    min: -0.5,
                    max: 1.5,
                    gamma: 0.9,
                    base_offset: 0.03,
                    alternate_offset: 0.04,
                },
                GainMapChannel {
                    min: 0.0,
                    max: 3.0,
                    gamma: 1.1,
                    base_offset: 0.05,
                    alternate_offset: 0.06,
                },
            ],
            base_hdr_headroom: 0.0,
            alternate_hdr_headroom: 2.0,
            use_base_color_space: false,
            backward_direction: false,
        };
        assert!(!p.is_single_channel());
        assert_eq!(p.direction(), GainMapDirection::BaseIsSdr);

        // HDR base direction
        let p2 = GainMapParams {
            base_hdr_headroom: 3.0,
            alternate_hdr_headroom: 0.0,
            ..p.clone()
        };
        assert_eq!(p2.direction(), GainMapDirection::BaseIsHdr);
    }

    #[test]
    fn gainmap_info_clone_and_equality() {
        let info = GainMapInfo::new(GainMapParams::default(), 512, 256, 3)
            .with_alternate_cicp(Cicp::BT2100_PQ);
        let clone = info.clone();
        assert_eq!(info, clone);

        // Modify clone, verify inequality
        let mut modified = clone;
        modified.width = 1024;
        assert_ne!(info, modified);
    }

    #[test]
    fn presence_clone_available() {
        let info = GainMapInfo::new(GainMapParams::default(), 200, 100, 1);
        let presence = GainMapPresence::Available(Box::new(info));
        let cloned = presence.clone();
        assert_eq!(presence, cloned);
        assert!(cloned.is_present());
        assert_eq!(cloned.info().unwrap().width, 200);
        assert_eq!(cloned.info().unwrap().height, 100);
    }

    #[test]
    fn fraction_edge_cases() {
        // Zero — continued fractions produce 0/1
        let f = Fraction::from_f64(0.0);
        assert_eq!(f.numerator, 0);
        assert_eq!(f.denominator, 1);
        assert!((f.to_f64()).abs() < 1e-10);
        assert!(f.is_valid());

        // Negative zero
        let f_neg0 = Fraction::from_f64(-0.0);
        assert_eq!(f_neg0.numerator, 0);
        assert!((f_neg0.to_f64()).abs() < 1e-10);

        // f64::MAX — continued fractions return fallback 0/1
        let f_max = Fraction::from_f64(f64::MAX);
        let _ = f_max.to_f64();

        // Canonical denominators for common values
        let f = Fraction::from_f64(0.015625);
        assert_eq!((f.numerator, f.denominator), (1, 64));

        let f = Fraction::from_f64(4.0);
        assert_eq!((f.numerator, f.denominator), (4, 1));

        let f = Fraction::from_f64(-1.0);
        assert_eq!((f.numerator, f.denominator), (-1, 1));
    }

    #[test]
    fn ufraction_edge_cases() {
        // Zero — continued fractions produce 0/1
        let f = UFraction::from_f64(0.0);
        assert_eq!(f.numerator, 0);
        assert_eq!(f.denominator, 1);
        assert!((f.to_f64()).abs() < 1e-10);
        assert!(f.is_valid());

        // f64::MAX — continued fractions return fallback 0/1
        let f_max = UFraction::from_f64(f64::MAX);
        let _ = f_max.to_f64();

        // Canonical denominators
        let f = UFraction::from_f64(0.015625);
        assert_eq!((f.numerator, f.denominator), (1, 64));

        let f = UFraction::from_f64(4.0);
        assert_eq!((f.numerator, f.denominator), (4, 1));
    }

    #[test]
    fn parse_iso21496_default_params_roundtrip() {
        let defaults = GainMapParams::default();
        let blob = serialize_iso21496(&defaults);
        let parsed = parse_iso21496(&blob).unwrap();

        assert!(parsed.is_single_channel());
        assert!(parsed.use_base_color_space);
        assert!((parsed.base_hdr_headroom - 0.0).abs() < 1e-6);
        assert!((parsed.alternate_hdr_headroom - 0.0).abs() < 1e-6);
        for ch in &parsed.channels {
            assert!((ch.min - 0.0).abs() < 1e-6);
            assert!((ch.max - 0.0).abs() < 1e-6);
            assert!((ch.gamma - 1.0).abs() < 1e-6);
            assert!((ch.base_offset - 1.0 / 64.0).abs() < 1e-6);
            assert!((ch.alternate_offset - 1.0 / 64.0).abs() < 1e-6);
        }
    }

    #[test]
    fn serialize_iso21496_flags() {
        // Single channel with use_base_color_space=true: bit 7 clear, bit 6 set → 0x40
        let single = GainMapParams::default();
        assert!(single.is_single_channel());
        let blob_single = serialize_iso21496(&single);
        assert_eq!(
            blob_single[5] & 0x80,
            0x00,
            "single channel: bit 7 must be clear"
        );
        assert_eq!(
            blob_single[5] & 0x40,
            0x40,
            "use_base_color_space: bit 6 must be set"
        );

        // Multi channel: bit 7 set
        let mut multi = GainMapParams::default();
        multi.channels[1].max = 5.0;
        assert!(!multi.is_single_channel());
        let blob_multi = serialize_iso21496(&multi);
        assert_eq!(
            blob_multi[5] & 0x80,
            0x80,
            "multi channel: bit 7 must be set"
        );
        assert_eq!(
            blob_multi[5] & 0x40,
            0x40,
            "use_base_color_space: bit 6 must be set"
        );

        // use_base_color_space=false: bit 6 clear
        let no_base_cs = GainMapParams {
            use_base_color_space: false,
            ..Default::default()
        };
        let blob_no_base = serialize_iso21496(&no_base_cs);
        assert_eq!(
            blob_no_base[5] & 0x40,
            0x00,
            "use_base_color_space=false: bit 6 must be clear"
        );
    }

    #[test]
    fn params_validate_equal_min_max() {
        let mut p = GainMapParams::default();
        p.channels[0].min = 1.5;
        p.channels[0].max = 1.5;
        p.channels[1].min = 1.5;
        p.channels[1].max = 1.5;
        p.channels[2].min = 1.5;
        p.channels[2].max = 1.5;
        assert!(p.validate().is_ok(), "equal min and max should be valid");
    }

    #[test]
    fn linear_helpers_fractional_log2() {
        let ch = GainMapChannel {
            min: 1.5,
            max: -0.75,
            gamma: 1.0,
            base_offset: 0.0,
            alternate_offset: 0.0,
        };
        // 2^1.5 ≈ 2.828427
        assert!((ch.linear_min() - 2.0f64.powf(1.5)).abs() < 1e-10);
        // 2^-0.75 ≈ 0.594604
        assert!((ch.linear_max() - 2.0f64.powf(-0.75)).abs() < 1e-10);
    }

    #[test]
    fn gainmap_parse_error_display_all_variants() {
        let variants: alloc::vec::Vec<GainMapParseError> = alloc::vec![
            GainMapParseError::TruncatedData {
                expected: 100,
                actual: 10,
            },
            GainMapParseError::UnsupportedVersion { version: 42 },
            GainMapParseError::ZeroDenominator {
                field: "test_field",
            },
            GainMapParseError::InvalidGamma {
                channel: 1,
                value: -0.5,
            },
            GainMapParseError::MinExceedsMax {
                channel: 2,
                min: 5.0,
                max: 1.0,
            },
            GainMapParseError::NonFiniteValue { field: "headroom" },
        ];

        for err in &variants {
            let msg = alloc::format!("{err}");
            assert!(!msg.is_empty(), "Display for {err:?} should be non-empty");
        }
    }

    // --- Iso21496Format and format-aware API ---

    #[test]
    fn format_enum_debug_and_eq() {
        assert_eq!(Iso21496Format::AvifTmap, Iso21496Format::AvifTmap);
        assert_ne!(Iso21496Format::AvifTmap, Iso21496Format::JpegApp2);
        let _ = alloc::format!("{:?}", Iso21496Format::JpegApp2);
    }

    #[test]
    fn roundtrip_avif_format() {
        let p = GainMapParams::default();
        let blob = serialize_iso21496_fmt(&p, Iso21496Format::AvifTmap);
        assert_eq!(blob.len(), 62); // 6-byte header + 16 headroom + 40 channel
        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::AvifTmap).unwrap();
        assert!(parsed.is_single_channel());
        assert!(parsed.use_base_color_space);
        assert!(!parsed.backward_direction);
    }

    #[test]
    fn roundtrip_jpeg_format() {
        let p = GainMapParams::default();
        let blob = serialize_iso21496_fmt(&p, Iso21496Format::JpegApp2);
        assert_eq!(blob.len(), 61); // 5-byte header + 16 headroom + 40 channel
        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::JpegApp2).unwrap();
        assert!(parsed.is_single_channel());
        assert!(parsed.use_base_color_space);
        assert!(!parsed.backward_direction);
    }

    #[test]
    fn old_api_matches_format_api() {
        let p = GainMapParams {
            alternate_hdr_headroom: 1.3,
            ..Default::default()
        };
        // Old API should produce identical bytes to format-aware API
        assert_eq!(
            serialize_iso21496(&p),
            serialize_iso21496_fmt(&p, Iso21496Format::AvifTmap)
        );
        assert_eq!(
            serialize_iso21496_jpeg(&p),
            serialize_iso21496_fmt(&p, Iso21496Format::JpegApp2)
        );
    }

    // --- backward_direction flag ---

    #[test]
    fn backward_direction_roundtrip_avif() {
        let p = GainMapParams {
            backward_direction: true,
            ..Default::default()
        };
        let blob = serialize_iso21496(&p);
        // flags byte is at offset 5 (after version + min_ver + writer_ver)
        assert_ne!(blob[5] & 0x04, 0, "backward_direction bit must be set");

        let parsed = parse_iso21496(&blob).unwrap();
        assert!(parsed.backward_direction);
    }

    #[test]
    fn backward_direction_roundtrip_jpeg() {
        let p = GainMapParams {
            backward_direction: true,
            ..Default::default()
        };
        let blob = serialize_iso21496_jpeg(&p);
        // flags byte is at offset 4 (after min_ver + writer_ver)
        assert_ne!(blob[4] & 0x04, 0, "backward_direction bit must be set");

        let parsed = parse_iso21496_jpeg(&blob).unwrap();
        assert!(parsed.backward_direction);
    }

    #[test]
    fn backward_direction_false_by_default() {
        let p = GainMapParams::default();
        let blob = serialize_iso21496(&p);
        assert_eq!(blob[5] & 0x04, 0, "backward_direction bit must be clear");

        let parsed = parse_iso21496(&blob).unwrap();
        assert!(!parsed.backward_direction);
    }

    // --- common_denominator compact format parsing ---

    #[test]
    fn parse_common_denominator_single_channel() {
        // Build a compact-format blob (no version byte = JPEG/JXL format)
        // with FLAG_COMMON_DENOMINATOR set.
        let mut blob = Vec::new();
        blob.extend_from_slice(&0u16.to_be_bytes()); // minimum_version
        blob.extend_from_slice(&0u16.to_be_bytes()); // writer_version
        // flags: single channel, use_base_colour_space, common_denominator
        blob.push(0x40 | 0x08);

        let common_d: u32 = 64;
        blob.extend_from_slice(&common_d.to_be_bytes()); // common denominator

        // Headroom numerators only
        blob.extend_from_slice(&0u32.to_be_bytes()); // base_headroom_n = 0
        blob.extend_from_slice(&83u32.to_be_bytes()); // alt_headroom_n = 83 (83/64 ≈ 1.297)

        // Channel 0 numerators only (min, max, gamma, base_offset, alt_offset)
        blob.extend_from_slice(&(-32i32).to_be_bytes()); // min_n = -32 (-32/64 = -0.5)
        blob.extend_from_slice(&128i32.to_be_bytes()); // max_n = 128 (128/64 = 2.0)
        blob.extend_from_slice(&64u32.to_be_bytes()); // gamma_n = 64 (64/64 = 1.0)
        blob.extend_from_slice(&1i32.to_be_bytes()); // base_offset_n = 1 (1/64)
        blob.extend_from_slice(&1i32.to_be_bytes()); // alt_offset_n = 1 (1/64)

        let params = parse_iso21496_jpeg(&blob).unwrap();
        assert!((params.base_hdr_headroom - 0.0).abs() < 1e-6);
        assert!((params.alternate_hdr_headroom - 83.0 / 64.0).abs() < 1e-6);
        assert!((params.channels[0].min - (-0.5)).abs() < 1e-6);
        assert!((params.channels[0].max - 2.0).abs() < 1e-6);
        assert!((params.channels[0].gamma - 1.0).abs() < 1e-6);
        assert!((params.channels[0].base_offset - 1.0 / 64.0).abs() < 1e-10);
        assert!(params.is_single_channel());
        assert!(params.use_base_color_space);
        assert!(!params.backward_direction);
    }

    #[test]
    fn parse_common_denominator_multi_channel() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&0u16.to_be_bytes()); // minimum_version
        blob.extend_from_slice(&0u16.to_be_bytes()); // writer_version
        // flags: multichannel + common_denominator
        blob.push(0x80 | 0x40 | 0x08);

        let common_d: u32 = 100;
        blob.extend_from_slice(&common_d.to_be_bytes());

        // Headroom
        blob.extend_from_slice(&0u32.to_be_bytes()); // base = 0
        blob.extend_from_slice(&200u32.to_be_bytes()); // alt = 200/100 = 2.0

        // 3 channels × 5 numerators each
        for ch_idx in 0..3u32 {
            let offset = (ch_idx as i32) * 10;
            blob.extend_from_slice(&(-50i32 + offset).to_be_bytes()); // min
            blob.extend_from_slice(&(200i32 + offset).to_be_bytes()); // max
            blob.extend_from_slice(&(100u32 + ch_idx).to_be_bytes()); // gamma
            blob.extend_from_slice(&1i32.to_be_bytes()); // base_offset
            blob.extend_from_slice(&2i32.to_be_bytes()); // alt_offset
        }

        let params = parse_iso21496_jpeg(&blob).unwrap();
        assert!(!params.is_single_channel());
        assert!((params.alternate_hdr_headroom - 2.0).abs() < 1e-6);
        // Channel 0: min = -50/100 = -0.5
        assert!((params.channels[0].min - (-0.5)).abs() < 1e-6);
        // Channel 1: min = -40/100 = -0.4
        assert!((params.channels[1].min - (-0.4)).abs() < 1e-6);
        // Channel 2: min = -30/100 = -0.3
        assert!((params.channels[2].min - (-0.3)).abs() < 1e-6);
    }

    #[test]
    fn parse_common_denominator_zero_rejected() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.push(0x40 | 0x08); // common_denominator flag
        blob.extend_from_slice(&0u32.to_be_bytes()); // common_d = 0 (invalid!)
        blob.extend_from_slice(&[0; 100]); // pad

        let err = parse_iso21496_jpeg(&blob).unwrap_err();
        assert!(matches!(err, GainMapParseError::ZeroDenominator { .. }));
    }

    #[test]
    fn parse_common_denominator_with_backward_direction() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.extend_from_slice(&0u16.to_be_bytes());
        // common_denominator + backward_direction + use_base_colour_space
        blob.push(0x40 | 0x08 | 0x04);

        let common_d: u32 = 1;
        blob.extend_from_slice(&common_d.to_be_bytes());
        blob.extend_from_slice(&0u32.to_be_bytes()); // base headroom
        blob.extend_from_slice(&1u32.to_be_bytes()); // alt headroom

        // Single channel, 5 numerators
        blob.extend_from_slice(&0i32.to_be_bytes()); // min
        blob.extend_from_slice(&2i32.to_be_bytes()); // max
        blob.extend_from_slice(&1u32.to_be_bytes()); // gamma
        blob.extend_from_slice(&0i32.to_be_bytes()); // base_offset
        blob.extend_from_slice(&0i32.to_be_bytes()); // alt_offset

        let params = parse_iso21496_jpeg(&blob).unwrap();
        assert!(params.backward_direction);
        assert!(params.use_base_color_space);
        assert!((params.alternate_hdr_headroom - 1.0).abs() < 1e-6);
        assert!((params.channels[0].max - 2.0).abs() < 1e-6);
    }

    #[test]
    fn parse_common_denom_avif_format() {
        // AVIF tmap with version byte + common denominator
        let mut blob = Vec::new();
        blob.push(0); // version
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.extend_from_slice(&0u16.to_be_bytes());
        blob.push(0x40 | 0x08); // use_base_colour_space + common_denominator

        let common_d: u32 = 10;
        blob.extend_from_slice(&common_d.to_be_bytes());
        blob.extend_from_slice(&0u32.to_be_bytes()); // base headroom
        blob.extend_from_slice(&13u32.to_be_bytes()); // alt headroom = 13/10 = 1.3

        blob.extend_from_slice(&0i32.to_be_bytes()); // min
        blob.extend_from_slice(&20i32.to_be_bytes()); // max = 20/10 = 2.0
        blob.extend_from_slice(&10u32.to_be_bytes()); // gamma = 10/10 = 1.0
        blob.extend_from_slice(&0i32.to_be_bytes()); // base_offset
        blob.extend_from_slice(&0i32.to_be_bytes()); // alt_offset

        let params = parse_iso21496(&blob).unwrap();
        assert!((params.alternate_hdr_headroom - 1.3).abs() < 1e-6);
        assert!((params.channels[0].max - 2.0).abs() < 1e-6);
    }

    // --- Verify backward compat wrappers match ---

    #[test]
    fn jpeg_format_size_one_less_than_avif() {
        let p = GainMapParams::default();
        let avif_blob = serialize_iso21496(&p);
        let jpeg_blob = serialize_iso21496_jpeg(&p);
        assert_eq!(avif_blob.len(), jpeg_blob.len() + 1);
        // The AVIF blob starts with version=0, then matches JPEG blob
        assert_eq!(avif_blob[0], 0);
        assert_eq!(&avif_blob[1..], &jpeg_blob[..]);
    }
}
