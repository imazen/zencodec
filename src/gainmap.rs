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

/// Wire format variant for ISO 21496-1 binary gain map metadata.
///
/// Selects the shape of the bytes handed to / returned from
/// [`parse_iso21496_fmt`] and [`serialize_iso21496_fmt`]. Covers the
/// ISO-defined parts of the wire format only — the URN namespace prefix
/// for URN-namespaced containers, plus the payload. **Does not include
/// any codec-specific outer envelope** (JPEG `FF E2` marker + `u16 BE`
/// length, ISOBMFF box headers, the JXL `jhgm` bundle wrapper, etc.) —
/// every caller adds that framing on top.
///
/// # How each container uses the bytes
///
/// | Context | Zencodec produces/consumes | Framing the caller adds on top |
/// |---------|----------------------------|--------------------------------|
/// | JPEG APP2 (secondary) | [`JpegApp2BodyWithUrn`] bytes — URN + payload | `FF E2` marker + `u16 BE` length (length counts itself + these bytes) |
/// | AVIF `tmap` item | [`AvifTmap`] bytes — this *is* the tmap item payload | ISOBMFF item framing (`iinf` / `iloc` / `iref`) pointing at these bytes |
/// | JXL `jhgm` box | [`JpegApp2`] bytes go into the bundle's `gain_map_metadata` field only — **no URN** | A whole `JxlGainMapBundle` (`jhgm_version u8` + `gain_map_metadata_size u16 BE` + **payload** + color encoding + alt ICC + JXL codestream), then the ISOBMFF `jhgm` box around the bundle |
///
/// **Note on JXL:** `jhgm` is *not* just "an ISOBMFF box around the ISO
/// payload". It's a structured bundle with its own header and trailing
/// fields — see `libjxl/lib/include/jxl/gain_map.h:38-63` and
/// `libjxl/lib/extras/gain_map.cc:83-153`. Zencodec produces what goes
/// in the `gain_map_metadata` field only; the bundle header and
/// trailing fields are libjxl's responsibility.
///
/// # Primary JPEG "signal" marker
///
/// Canonical Ultra HDR JPEGs also carry a 4-byte version-only APP2 marker
/// on the *primary* image (`00 00 00 00` — min_ver u16 + writer_ver u16,
/// no flags, no fractions). That marker is not a payload this enum can
/// represent; it's framing. Produced by ultrahdr-core, not zencodec.
///
/// # Flag bit positions are consistent across implementations
///
/// libultrahdr and libavif both pack flag bits MSB-first, so bit 7
/// (`is_multichannel`) and bit 6 (`use_base_colour_space`) occupy the
/// same positions in JPEG APP2 and AVIF tmap payloads. Verified in
/// `libultrahdr/lib/include/ultrahdr/gainmapmetadata.h:26-27`,
/// `libavif/src/stream.c:506-508` (writeBits MSB-first), and
/// `libavif/src/read.c:2162-2166` (readBits symmetric).
///
/// libultrahdr also uses bit 2 (`backward_direction`) and bit 3
/// (`common_denominator`) in the same byte; libavif treats bits 5..0 as
/// reserved. **libavif does not enforce `reserved == 0`** on read
/// (`libavif/src/read.c:2169-2170` consumes the 6 reserved bits into a
/// discarded local). A libultrahdr-encoded payload using the compact
/// (common-denominator) encoding would therefore be **silently
/// misparsed** by libavif: libavif ignores the flag bit, then expects
/// the full-format fraction layout, which doesn't match the compact
/// bytes that follow. For that reason [`serialize_iso21496_fmt`] never
/// emits the compact encoding, for either variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Iso21496Format {
    /// **Semantically equivalent to** [`JxlJhgm`](Self::JxlJhgm) — produces and
    /// accepts identical bytes, but is a distinct variant with a different
    /// discriminant. **Deprecated**: use [`JxlJhgm`](Self::JxlJhgm) for the bare
    /// payload, or [`JpegApp2BodyWithUrn`](Self::JpegApp2BodyWithUrn) for the
    /// full JPEG APP2 body.
    ///
    /// The original name was misleading: the bytes are NOT what a JPEG APP2
    /// segment contains on the wire (they lack the required URN prefix).
    /// Kept for source compatibility with 0.1.12–0.1.19 at the original
    /// discriminant `0`.
    #[deprecated(
        since = "0.1.20",
        note = "Misleading name — these bytes lack the URN and are not a standalone JPEG APP2 body. Use `JxlJhgm` for the same bytes, or `JpegApp2BodyWithUrn` for the full APP2 body including the URN."
    )]
    JpegApp2 = 0,
    /// Payload with an AVIF `tmap` version byte prefix.
    ///
    /// Layout: `version(u8=0) + min_ver(u16 BE) + writer_ver(u16 BE) + flags(u8) + <fraction payload>`.
    ///
    /// The leading `version` byte is written at offset 0 of the `tmap`
    /// item payload. `tmap` is an item type, not a FullBox — the version
    /// byte is part of the item data, not a box header field. Verified
    /// in `libavif/src/write.c:1027-1039` (encoder) and
    /// `libavif/src/read.c:2202-2211` (decoder). Currently always 0;
    /// both libavif and zencodec reject any other value on parse.
    AvifTmap = 1,
    /// Bare ISO 21496-1 payload with no outer prefix.
    ///
    /// Layout: `min_ver(u16 BE) + writer_ver(u16 BE) + flags(u8) + <fraction payload>`.
    ///
    /// Canonical consumer: the `gain_map_metadata` field of libjxl's
    /// `JxlGainMapBundle` (the `jhgm` bundle envelope lives above this
    /// payload — see the enum-level note on JXL). The same byte shape
    /// also sits inside a JPEG APP2 segment *after* the 28-byte URN
    /// prefix has been stripped — use
    /// [`JpegApp2BodyWithUrn`](Self::JpegApp2BodyWithUrn) to round-trip the full
    /// APP2 body including the URN in one call.
    JxlJhgm = 2,
    /// Full JPEG APP2 body: URN + [`JxlJhgm`](Self::JxlJhgm) payload.
    ///
    /// Layout: `"urn:iso:std:iso:ts:21496:-1\0"` (28 bytes, see
    /// [`ISO_21496_1_URN`]) followed immediately by the
    /// [`JxlJhgm`](Self::JxlJhgm) bytes.
    ///
    /// These are the bytes that sit directly inside a JPEG APP2 segment
    /// **after** the segment's `FF E2` marker + `u16 BE` length header.
    /// **This variant does NOT include the `FF E2` marker or the length
    /// word** — the caller (zenjpeg) emits those as part of its JPEG
    /// syntax, same as for every other APP segment. Use this variant
    /// when you want zencodec to produce or parse the full APP2 body in
    /// one call without separately handling the URN.
    ///
    /// AVIF and JXL do not use this variant: their containers identify
    /// the gain-map payload via the item/box type system (`tmap`,
    /// `jhgm`), not a string URN.
    JpegApp2BodyWithUrn = 3,
}

#[allow(deprecated)]
const _: () = {
    assert!(Iso21496Format::JpegApp2 as u8 == 0);
    assert!(Iso21496Format::AvifTmap as u8 == 1);
    assert!(Iso21496Format::JxlJhgm as u8 == 2);
    assert!(Iso21496Format::JpegApp2BodyWithUrn as u8 == 3);
};

// =========================================================================
// ISO 21496-1 URN and primary-image signal payload
// =========================================================================

/// ISO 21496-1 namespace URN. 28 bytes including the trailing NUL byte:
/// `b"urn:iso:std:iso:ts:21496:-1\0"`.
///
/// Defined by ISO/IEC 21496-1. Containers that namespace the gain-map
/// payload behind a string identifier prefix these exact 28 bytes before
/// the [`Iso21496Format::JxlJhgm`] payload — most notably a JPEG APP2
/// segment. libultrahdr writes the same byte sequence
/// (`libultrahdr/lib/src/jpegr.cpp:69`, `kIsoNameSpace`).
///
/// AVIF's `tmap` item and JXL's `jhgm` bundle do **not** use this URN;
/// they identify the payload via their container's item/box type system.
///
/// Use [`Iso21496Format::JpegApp2BodyWithUrn`] with
/// [`serialize_iso21496_fmt`] / [`parse_iso21496_fmt`] to (de)serialize a
/// URN-prefixed payload in one call. This constant is exposed for callers
/// that need the raw URN bytes directly (detection, length accounting).
pub const ISO_21496_1_URN: &[u8; 28] = b"urn:iso:std:iso:ts:21496:-1\0";

/// The complete JPEG APP2 body (URN + version-only tail) placed on the
/// **primary** image of a canonical Ultra HDR JPEG to advertise ISO
/// 21496-1 awareness.
///
/// Byte layout (32 bytes total):
/// - `[0..28]`: [`ISO_21496_1_URN`]
/// - `[28..32]`: `minimum_version=0u16 BE` + `writer_version=0u16 BE` —
///   the bare `GainMapVersion` structure from ISO 21496-1 clause C.2.2,
///   with no flags byte and no per-channel data
///
/// These bytes go directly inside a JPEG APP2 segment, after the `FF E2`
/// marker + `u16 BE` length header. The caller (zenjpeg) is responsible
/// for emitting the marker/length envelope. Byte-identical to what
/// libultrahdr writes at `libultrahdr/lib/src/jpegr.cpp:1266-1268` and
/// documents at `jpegr.cpp:1073-1077`.
///
/// Full gain-map parameters live on the *secondary* (gain map) image's
/// APP2 segment and are linked from the primary via MPF (see
/// `zenjpeg::container::mpf`). The primary here is just the "I am ISO
/// 21496-1 aware" signal.
///
/// ## Version field semantics
///
/// - `minimum_version` is the spec version required to parse the
///   payload. Only `0` is defined today; libultrahdr, libavif, and
///   zencodec all reject anything `> 0` as unsupported.
/// - `writer_version` is informational (MUST be `>= minimum_version`).
///   Future writers may bump it to advertise backward-compatible
///   extension fields appended after the known layout. libultrahdr,
///   libavif, and zencodec all write `0` and ignore it on parse today.
///
/// ## Detection, not parsing
///
/// The 4-byte version-only tail is shorter than the minimum valid
/// [`Iso21496Format::JxlJhgm`] parse input (which requires at least the
/// flags byte and one channel), so feeding the full 32 bytes to
/// [`parse_iso21496_fmt`] under [`Iso21496Format::JpegApp2BodyWithUrn`]
/// returns [`GainMapParseError::TruncatedData`] — by design. Callers
/// detect the signal by **exact bytes match**, not by parsing. Typical
/// use:
///
/// ```ignore
/// if app2_segment_body == ISO_21496_1_PRIMARY_APP2_BODY {
///     // primary-image signal: full metadata lives on the MPF secondary
/// }
/// ```
pub const ISO_21496_1_PRIMARY_APP2_BODY: &[u8; 32] = b"urn:iso:std:iso:ts:21496:-1\0\0\0\0\0";

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

    /// Create from f64 with a fixed denominator (multiply-and-round).
    ///
    /// This is the simplest encoding: `numerator = round(value * denominator)`.
    /// For canonical ISO 21496-1 fractions, prefer [`Self::from_f64_cf`].
    #[deprecated(
        since = "0.1.12",
        note = "use `from_f64_cf` for canonical continued-fraction encoding"
    )]
    pub fn from_f64(value: f64, denominator: u32) -> Self {
        Self {
            numerator: (value * denominator as f64).round() as i32,
            denominator,
        }
    }

    /// Create from f64 using continued fractions for canonical encoding.
    ///
    /// Finds the best rational approximation, matching the algorithm used by
    /// libultrahdr and canonical ISO 21496-1 encoders. Produces compact
    /// fractions (e.g. `1/64` instead of `15625/1000000`).
    pub fn from_f64_cf(value: f64) -> Self {
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

    /// Create from f64 with a fixed denominator (multiply-and-round).
    ///
    /// This is the simplest encoding: `numerator = round(value * denominator)`.
    /// Clamps negative values to 0.
    /// For canonical ISO 21496-1 fractions, prefer [`Self::from_f64_cf`].
    #[deprecated(
        since = "0.1.12",
        note = "use `from_f64_cf` for canonical continued-fraction encoding"
    )]
    pub fn from_f64(value: f64, denominator: u32) -> Self {
        Self {
            numerator: (value.max(0.0) * denominator as f64).round() as u32,
            denominator,
        }
    }

    /// Create from f64 using continued fractions for canonical encoding.
    ///
    /// Finds the best rational approximation, matching the algorithm used by
    /// libultrahdr and canonical ISO 21496-1 encoders. Clamps negative values to 0.
    pub fn from_f64_cf(value: f64) -> Self {
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
    /// Input to [`parse_iso21496_with_urn`] did not begin with the ISO 21496-1 URN.
    UrnMismatch,
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
            Self::UrnMismatch => {
                write!(
                    f,
                    "ISO 21496-1: input does not begin with the ISO 21496-1 URN"
                )
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

/// Parse ISO 21496-1 binary gain map metadata (JpegApp2 format).
///
/// This is a convenience alias for
/// `parse_iso21496_fmt(data, Iso21496Format::JxlJhgm)`.
///
/// **Note:** In 0.1.11 this function used the AVIF tmap format (version byte
/// prefix). It now uses the JpegApp2 format (no version byte). Use
/// [`parse_iso21496_fmt`] with an explicit [`Iso21496Format`] to avoid
/// ambiguity.
#[deprecated(
    since = "0.1.12",
    note = "use `parse_iso21496_fmt` with an explicit `Iso21496Format`"
)]
pub fn parse_iso21496(data: &[u8]) -> Result<GainMapParams, GainMapParseError> {
    parse_iso21496_no_version(data)
}

/// Serialize [`GainMapParams`] to ISO 21496-1 binary format (JpegApp2 format).
///
/// This is a convenience alias for
/// `serialize_iso21496_fmt(params, Iso21496Format::JxlJhgm)`.
///
/// **Note:** In 0.1.11 this function used the AVIF tmap format (version byte
/// prefix). It now uses the JpegApp2 format (no version byte). Use
/// [`serialize_iso21496_fmt`] with an explicit [`Iso21496Format`] to avoid
/// ambiguity.
#[deprecated(
    since = "0.1.12",
    note = "use `serialize_iso21496_fmt` with an explicit `Iso21496Format`"
)]
pub fn serialize_iso21496(params: &GainMapParams) -> Vec<u8> {
    serialize_iso21496_fmt(params, Iso21496Format::JxlJhgm)
}

/// Parse ISO 21496-1 binary gain map metadata with explicit format selection.
///
/// The `format` parameter selects the wire format variant:
/// - [`Iso21496Format::JxlJhgm`]: no prefix (bare payload — JXL `jhgm`
///   `gain_map_metadata` field, or a JPEG APP2 body after the URN is
///   stripped by the caller)
/// - [`Iso21496Format::JpegApp2BodyWithUrn`]: URN + bare payload (full JPEG APP2
///   body after the `FF E2 + u16 BE length` header is stripped by the caller)
/// - [`Iso21496Format::AvifTmap`]: `version(u8=0)` prefix + payload (AVIF
///   `tmap` item payload)
///
/// All variants handle the common-denominator compact encoding (flag bit 3)
/// used by libultrahdr.
///
/// # Input expectations
///
/// - **JPEG APP2 callers:** strip the JPEG segment header (`FF E2` + `u16 BE`
///   length) and pass the rest as [`Iso21496Format::JpegApp2BodyWithUrn`]. If you
///   have already stripped the URN yourself, pass [`Iso21496Format::JxlJhgm`].
/// - **AVIF callers:** pass the raw `tmap` item payload — the version byte
///   is the first byte and is consumed here.
/// - **JXL callers:** pass the bytes inside the `jhgm` bundle's
///   `gain_map_metadata` field as [`Iso21496Format::JxlJhgm`] (no URN, no
///   version byte).
///
/// For `JpegApp2BodyWithUrn`, returns [`GainMapParseError::UrnMismatch`] if the
/// input does not begin with [`ISO_21496_1_URN`]. Does **not** include the
/// JPEG `FF E2` marker or `u16 BE` length — those are JPEG syntax that the
/// caller emits/parses as part of its APP segment handling.
pub fn parse_iso21496_fmt(
    data: &[u8],
    format: Iso21496Format,
) -> Result<GainMapParams, GainMapParseError> {
    #[allow(deprecated)]
    match format {
        Iso21496Format::AvifTmap => parse_iso21496_avif(data),
        Iso21496Format::JxlJhgm | Iso21496Format::JpegApp2 => parse_iso21496_no_version(data),
        Iso21496Format::JpegApp2BodyWithUrn => {
            if data.len() < ISO_21496_1_URN.len()
                || &data[..ISO_21496_1_URN.len()] != ISO_21496_1_URN
            {
                return Err(GainMapParseError::UrnMismatch);
            }
            parse_iso21496_no_version(&data[ISO_21496_1_URN.len()..])
        }
    }
}

/// Serialize [`GainMapParams`] to ISO 21496-1 binary format with explicit format selection.
///
/// The `format` parameter selects the wire format variant:
/// - [`Iso21496Format::JxlJhgm`]: bare payload (no prefix) — suitable for JXL
///   `jhgm` `gain_map_metadata` and for callers writing the URN themselves
/// - [`Iso21496Format::JpegApp2BodyWithUrn`]: full JPEG APP2 body — URN + payload
/// - [`Iso21496Format::AvifTmap`]: `version(u8=0)` prefix + payload
///
/// Always writes the full (non-compact) format for maximum compatibility —
/// i.e. the `FLAG_COMMON_DENOMINATOR` bit is never set even when all
/// denominators happen to match. Output is slightly larger than a
/// libultrahdr-encoded file in that case (5 × u32 extra per channel) but
/// byte-for-byte compatible on parse.
///
/// # Output
///
/// - **JPEG APP2 callers:** use [`Iso21496Format::JpegApp2BodyWithUrn`] and wrap
///   the returned bytes as `FF E2 + length(u16 BE) + <bytes>`. Length counts
///   itself + the returned bytes (matches libultrahdr `lib/src/jpegr.cpp:1256`).
///   The URN is already included in the returned bytes.
/// - **AVIF callers:** the [`Iso21496Format::AvifTmap`] bytes *are* the `tmap`
///   item payload; surround with the ISOBMFF item envelope.
/// - **JXL callers:** the [`Iso21496Format::JxlJhgm`] bytes *are* the `jhgm`
///   bundle's `gain_map_metadata` field; surround with the bundle envelope
///   and ISOBMFF box.
///
/// The JPEG `FF E2` marker and `u16 BE` length are **not** emitted — those
/// are JPEG syntax that the caller writes as part of its APP segment handling.
pub fn serialize_iso21496_fmt(params: &GainMapParams, format: Iso21496Format) -> Vec<u8> {
    let mut out = Vec::with_capacity(serialized_size(params, format));
    serialize_iso21496_fmt_into(params, format, &mut out);
    out
}

/// Append the serialized ISO 21496-1 payload for `format` to `out`.
///
/// Same wire format as [`serialize_iso21496_fmt`] but writes into a
/// caller-provided buffer instead of returning a new `Vec`. Useful when
/// embedding the payload inside a larger output buffer (e.g., building
/// a JPEG APP2 segment alongside its `FF E2` marker and `u16 BE` length
/// header in one allocation).
///
/// Existing contents of `out` are preserved; the payload is appended.
/// Callers that want to reserve capacity up-front can size the buffer
/// to match the return of [`serialize_iso21496_fmt`]`(..).len()` for
/// the same inputs, or just let `Vec` grow — the implementation never
/// truncates or overwrites bytes already in `out`.
pub fn serialize_iso21496_fmt_into(
    params: &GainMapParams,
    format: Iso21496Format,
    out: &mut Vec<u8>,
) {
    #[allow(deprecated)]
    match format {
        Iso21496Format::AvifTmap => serialize_iso21496_avif_into(params, out),
        Iso21496Format::JxlJhgm | Iso21496Format::JpegApp2 => {
            serialize_iso21496_no_version_into(params, out)
        }
        Iso21496Format::JpegApp2BodyWithUrn => {
            out.extend_from_slice(ISO_21496_1_URN);
            serialize_iso21496_no_version_into(params, out);
        }
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

    write_ufraction(data, UFraction::from_f64_cf(params.base_hdr_headroom));
    write_ufraction(data, UFraction::from_f64_cf(params.alternate_hdr_headroom));

    for ch in params.channels.iter().take(num_channels) {
        write_fraction(data, Fraction::from_f64_cf(ch.min));
        write_fraction(data, Fraction::from_f64_cf(ch.max));
        write_ufraction(data, UFraction::from_f64_cf(ch.gamma));
        write_fraction(data, Fraction::from_f64_cf(ch.base_offset));
        write_fraction(data, Fraction::from_f64_cf(ch.alternate_offset));
    }
}

/// Exact serialized size of `params` under `format`. Used by the public
/// entry points to allocate once.
fn serialized_size(params: &GainMapParams, format: Iso21496Format) -> usize {
    let num_channels: usize = if params.is_single_channel() { 1 } else { 3 };
    #[allow(deprecated)]
    let header = match format {
        Iso21496Format::AvifTmap => AVIF_HEADER_SIZE,
        Iso21496Format::JxlJhgm | Iso21496Format::JpegApp2 => JPEG_HEADER_SIZE,
        Iso21496Format::JpegApp2BodyWithUrn => ISO_21496_1_URN.len() + JPEG_HEADER_SIZE,
    };
    header + 2 * FRACTION_SIZE + num_channels * 5 * FRACTION_SIZE
}

/// Append AVIF `tmap` serialization (with version byte prefix) to `out`.
fn serialize_iso21496_avif_into(params: &GainMapParams, out: &mut Vec<u8>) {
    out.push(0u8); // version
    out.extend_from_slice(&0u16.to_be_bytes()); // minimum_version
    out.extend_from_slice(&0u16.to_be_bytes()); // writer_version
    out.push(build_flags(params));
    write_payload(out, params);
}

/// Append JPEG APP2 / JXL jhgm serialization (no version byte prefix) to `out`.
fn serialize_iso21496_no_version_into(params: &GainMapParams, out: &mut Vec<u8>) {
    out.extend_from_slice(&0u16.to_be_bytes()); // minimum_version
    out.extend_from_slice(&0u16.to_be_bytes()); // writer_version
    out.push(build_flags(params));
    write_payload(out, params);
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
        let f = Fraction::from_f64_cf(1.5);
        assert!((f.to_f64() - 1.5).abs() < 1e-6);
    }

    #[test]
    fn fraction_negative() {
        let f = Fraction::from_f64_cf(-0.256907);
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
        let f = UFraction::from_f64_cf(1.3);
        assert!((f.to_f64() - 1.3).abs() < 1e-6);
    }

    #[test]
    fn ufraction_clamps_negative() {
        let f = UFraction::from_f64_cf(-5.0);
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

    // --- parse_iso21496_fmt / serialize_iso21496_fmt ---

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

        let blob = serialize_iso21496_fmt(&original, Iso21496Format::JxlJhgm);
        assert_eq!(blob.len(), 61); // 5 + 16 + 1*40

        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();
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

        let blob = serialize_iso21496_fmt(&original, Iso21496Format::JxlJhgm);
        assert_eq!(blob.len(), 141); // 5 + 16 + 3*40

        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();
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
        // Construct a known ISO 21496-1 binary blob manually (no version byte).
        // Single channel, use_base_color_space=true, base=SDR, alt headroom=13/10=1.3
        let mut blob = Vec::new();
        // Header (no version byte — standard format)
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

        let params = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();
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
        assert!(parse_iso21496_fmt(&[], Iso21496Format::JxlJhgm).is_err());
        assert!(parse_iso21496_fmt(&[0], Iso21496Format::JxlJhgm).is_err());
        assert!(parse_iso21496_fmt(&[0; 4], Iso21496Format::JxlJhgm).is_err());
        // 5 bytes header OK, but not enough for headroom
        assert!(parse_iso21496_fmt(&[0, 0, 0, 0, 0x40], Iso21496Format::JxlJhgm).is_err());
    }

    #[test]
    fn parse_wrong_min_version() {
        let mut blob = alloc::vec![0u8; 61];
        blob[0] = 0;
        blob[1] = 1; // minimum_version = 1 (unsupported)
        let err = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap_err();
        assert!(matches!(err, GainMapParseError::UnsupportedVersion { .. }));
    }

    #[test]
    fn parse_zero_denominator() {
        // Build a blob with zero denominator in base_headroom (no version byte)
        let mut blob = Vec::new();
        blob.extend_from_slice(&0u16.to_be_bytes()); // min_version
        blob.extend_from_slice(&0u16.to_be_bytes()); // writer_version
        blob.push(0x40); // flags
        blob.extend_from_slice(&0u32.to_be_bytes()); // base_headroom_n
        blob.extend_from_slice(&0u32.to_be_bytes()); // base_headroom_d = 0 !
        // pad to avoid truncation error before we hit zero-denom
        blob.extend_from_slice(&[0; 100]);

        let err = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap_err();
        assert!(matches!(err, GainMapParseError::ZeroDenominator { .. }));
    }

    #[test]
    fn parse_avif_tmap_roundtrip() {
        let p = GainMapParams {
            alternate_hdr_headroom: 1.3,
            backward_direction: true,
            ..Default::default()
        };
        let blob = serialize_iso21496_fmt(&p, Iso21496Format::AvifTmap);
        assert_eq!(blob[0], 0, "version byte must be 0");
        assert_eq!(blob.len(), 62); // 6-byte header + 16 headroom + 40 channel

        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::AvifTmap).unwrap();
        assert!(parsed.use_base_color_space);
        assert!(parsed.backward_direction);
        assert!((parsed.alternate_hdr_headroom - 1.3).abs() < 1e-6);
    }

    #[test]
    fn parse_avif_wrong_version() {
        let mut blob = alloc::vec![0u8; 62];
        blob[0] = 1; // unsupported version
        let err = parse_iso21496_fmt(&blob, Iso21496Format::AvifTmap).unwrap_err();
        assert!(matches!(
            err,
            GainMapParseError::UnsupportedVersion { version: 1 }
        ));
    }

    // --- serialize_iso21496_fmt ---

    #[test]
    fn serialize_single_channel_size() {
        let p = GainMapParams::default();
        assert!(p.is_single_channel());
        assert_eq!(
            serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm).len(),
            61
        ); // 5 + 16 + 40
    }

    #[test]
    fn serialize_multi_channel_size() {
        let mut p = GainMapParams::default();
        p.channels[1].max = 3.0; // make multichannel
        assert!(!p.is_single_channel());
        assert_eq!(
            serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm).len(),
            141
        ); // 5 + 16 + 3*40
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
        let f = Fraction::from_f64_cf(0.0);
        assert_eq!(f.numerator, 0);
        assert_eq!(f.denominator, 1);
        assert!((f.to_f64()).abs() < 1e-10);
        assert!(f.is_valid());

        // Negative zero
        let f_neg0 = Fraction::from_f64_cf(-0.0);
        assert_eq!(f_neg0.numerator, 0);
        assert!((f_neg0.to_f64()).abs() < 1e-10);

        // f64::MAX — continued fractions return fallback 0/1
        let f_max = Fraction::from_f64_cf(f64::MAX);
        let _ = f_max.to_f64();

        // Canonical denominators for common values
        let f = Fraction::from_f64_cf(0.015625);
        assert_eq!((f.numerator, f.denominator), (1, 64));

        let f = Fraction::from_f64_cf(4.0);
        assert_eq!((f.numerator, f.denominator), (4, 1));

        let f = Fraction::from_f64_cf(-1.0);
        assert_eq!((f.numerator, f.denominator), (-1, 1));
    }

    #[test]
    fn ufraction_edge_cases() {
        // Zero — continued fractions produce 0/1
        let f = UFraction::from_f64_cf(0.0);
        assert_eq!(f.numerator, 0);
        assert_eq!(f.denominator, 1);
        assert!((f.to_f64()).abs() < 1e-10);
        assert!(f.is_valid());

        // f64::MAX — continued fractions return fallback 0/1
        let f_max = UFraction::from_f64_cf(f64::MAX);
        let _ = f_max.to_f64();

        // Canonical denominators
        let f = UFraction::from_f64_cf(0.015625);
        assert_eq!((f.numerator, f.denominator), (1, 64));

        let f = UFraction::from_f64_cf(4.0);
        assert_eq!((f.numerator, f.denominator), (4, 1));
    }

    #[test]
    fn parse_iso21496_default_params_roundtrip() {
        let defaults = GainMapParams::default();
        let blob = serialize_iso21496_fmt(&defaults, Iso21496Format::JxlJhgm);
        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();

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
        let blob_single = serialize_iso21496_fmt(&single, Iso21496Format::JxlJhgm);
        assert_eq!(
            blob_single[4] & 0x80,
            0x00,
            "single channel: bit 7 must be clear"
        );
        assert_eq!(
            blob_single[4] & 0x40,
            0x40,
            "use_base_color_space: bit 6 must be set"
        );

        // Multi channel: bit 7 set
        let mut multi = GainMapParams::default();
        multi.channels[1].max = 5.0;
        assert!(!multi.is_single_channel());
        let blob_multi = serialize_iso21496_fmt(&multi, Iso21496Format::JxlJhgm);
        assert_eq!(
            blob_multi[4] & 0x80,
            0x80,
            "multi channel: bit 7 must be set"
        );
        assert_eq!(
            blob_multi[4] & 0x40,
            0x40,
            "use_base_color_space: bit 6 must be set"
        );

        // use_base_color_space=false: bit 6 clear
        let no_base_cs = GainMapParams {
            use_base_color_space: false,
            ..Default::default()
        };
        let blob_no_base = serialize_iso21496_fmt(&no_base_cs, Iso21496Format::JxlJhgm);
        assert_eq!(
            blob_no_base[4] & 0x40,
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
            GainMapParseError::UrnMismatch,
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
        assert_ne!(Iso21496Format::AvifTmap, Iso21496Format::JxlJhgm);
        let _ = alloc::format!("{:?}", Iso21496Format::JxlJhgm);
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
        let blob = serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm);
        assert_eq!(blob.len(), 61); // 5-byte header + 16 headroom + 40 channel
        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();
        assert!(parsed.is_single_channel());
        assert!(parsed.use_base_color_space);
        assert!(!parsed.backward_direction);
    }

    #[test]
    fn unsuffixed_matches_jpeg_format() {
        let p = GainMapParams {
            alternate_hdr_headroom: 1.3,
            ..Default::default()
        };
        // Unsuffixed API produces JpegApp2 format (no version byte)
        assert_eq!(
            serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm),
            serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm)
        );
        // AVIF format is one byte longer (version byte prefix)
        let avif = serialize_iso21496_fmt(&p, Iso21496Format::AvifTmap);
        assert_eq!(
            avif.len(),
            serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm).len() + 1
        );
    }

    // --- backward_direction flag ---

    #[test]
    fn backward_direction_roundtrip() {
        let p = GainMapParams {
            backward_direction: true,
            ..Default::default()
        };
        let blob = serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm);
        // flags byte at offset 4 (after min_ver + writer_ver)
        assert_ne!(blob[4] & 0x04, 0, "backward_direction bit must be set");

        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();
        assert!(parsed.backward_direction);
    }

    #[test]
    fn backward_direction_roundtrip_avif() {
        let p = GainMapParams {
            backward_direction: true,
            ..Default::default()
        };
        let blob = serialize_iso21496_fmt(&p, Iso21496Format::AvifTmap);
        // flags byte at offset 5 (after version + min_ver + writer_ver)
        assert_ne!(blob[5] & 0x04, 0, "backward_direction bit must be set");

        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::AvifTmap).unwrap();
        assert!(parsed.backward_direction);
    }

    #[test]
    fn backward_direction_false_by_default() {
        let p = GainMapParams::default();
        let blob = serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm);
        assert_eq!(blob[4] & 0x04, 0, "backward_direction bit must be clear");

        let parsed = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();
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

        let params = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();
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

        let params = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();
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

        let err = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap_err();
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

        let params = parse_iso21496_fmt(&blob, Iso21496Format::JxlJhgm).unwrap();
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

        let params = parse_iso21496_fmt(&blob, Iso21496Format::AvifTmap).unwrap();
        assert!((params.alternate_hdr_headroom - 1.3).abs() < 1e-6);
        assert!((params.channels[0].max - 2.0).abs() < 1e-6);
    }

    // --- Format variant relationship ---

    #[test]
    fn avif_format_has_version_byte_prefix() {
        let p = GainMapParams::default();
        let avif_blob = serialize_iso21496_fmt(&p, Iso21496Format::AvifTmap);
        let jpeg_blob = serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm);
        assert_eq!(avif_blob.len(), jpeg_blob.len() + 1);
        // The AVIF blob starts with version=0, then matches the standard format
        assert_eq!(avif_blob[0], 0);
        assert_eq!(&avif_blob[1..], &jpeg_blob[..]);
    }

    // --- Framing boundary claims (asserted by Iso21496Format doc comments) ---
    //
    // These tests lock in *what zencodec does NOT put in its output* — i.e.
    // the framing bytes callers must supply themselves.

    fn framing_test_params() -> GainMapParams {
        GainMapParams {
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
        }
    }

    #[test]
    fn jpegapp2_output_starts_with_min_version_not_a_jpeg_marker() {
        // Asserts: JpegApp2 output does NOT include the FF E2 APP2 marker
        // prefix. Caller is responsible for adding it when splicing into a
        // JPEG bitstream.
        let blob = serialize_iso21496_fmt(&framing_test_params(), Iso21496Format::JxlJhgm);
        assert!(blob.len() >= 5, "blob too short to have a header");
        // First 4 bytes = min_version(u16=0) + writer_version(u16=0).
        assert_eq!(
            &blob[0..4],
            &[0x00, 0x00, 0x00, 0x00],
            "JpegApp2 output must begin with min_ver(u16 BE=0) + writer_ver(u16 BE=0)"
        );
        assert_ne!(
            blob[0], 0xFF,
            "JpegApp2 output must not begin with a JPEG marker byte (0xFF)"
        );
    }

    #[test]
    fn jpegapp2_output_contains_no_iso_urn() {
        // Asserts: JpegApp2 output does NOT include the
        // "urn:iso:std:iso:ts:21496:-1" namespace. Callers writing a JPEG
        // APP2 marker must prepend the URN (libultrahdr writes
        // `isoNameSpaceLength = 28` bytes including null terminator; see
        // libultrahdr/lib/src/jpegr.cpp:1129, 1264).
        let blob = serialize_iso21496_fmt(&framing_test_params(), Iso21496Format::JxlJhgm);
        let urn = b"urn:iso:std:iso:ts:21496:-1";
        assert!(
            !blob.windows(urn.len()).any(|w| w == urn),
            "JpegApp2 output must not contain the ISO 21496-1 URN anywhere"
        );
    }

    #[test]
    fn aviftmap_output_is_single_version_byte_then_jpegapp2_bytes() {
        // Redundant with avif_format_has_version_byte_prefix, but asserted
        // here against a realistic (non-Default) params value so the byte
        // count reflects actual fractions, not zero-value shortcuts.
        let p = framing_test_params();
        let avif = serialize_iso21496_fmt(&p, Iso21496Format::AvifTmap);
        let jpeg = serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm);
        assert_eq!(avif.len(), jpeg.len() + 1, "avif = one extra leading byte");
        assert_eq!(avif[0], 0, "avif leading byte is version = 0");
        assert_eq!(
            &avif[1..],
            &jpeg[..],
            "rest of avif is byte-identical to jpegapp2"
        );
    }

    #[test]
    fn aviftmap_output_contains_no_isobmff_tmap_box_header() {
        // Asserts: AvifTmap output is the `tmap` *item payload*, not an
        // ISOBMFF box. Callers writing an AVIF file must wrap with the
        // item envelope themselves.
        let blob = serialize_iso21496_fmt(&framing_test_params(), Iso21496Format::AvifTmap);
        let tmap_type = b"tmap";
        assert!(
            !blob.windows(tmap_type.len()).any(|w| w == tmap_type),
            "AvifTmap output must not contain the `tmap` box type — it is the box payload, not a box"
        );
    }

    #[test]
    fn parse_jpegapp2_rejects_urn_prefixed_input() {
        // Asserts: the parser's input contract excludes the URN. A caller
        // who forgets to strip "urn:iso:std:iso:ts:21496:-1\0" before
        // calling parse gets a clear UnsupportedVersion error (the 'u' byte
        // of the URN is 0x75 → min_version u16 reads as 0x7572, which is
        // rejected by our minimum_version > 0 check).
        let payload = serialize_iso21496_fmt(&framing_test_params(), Iso21496Format::JxlJhgm);
        let mut with_urn = Vec::with_capacity(28 + payload.len());
        with_urn.extend_from_slice(b"urn:iso:std:iso:ts:21496:-1\0");
        with_urn.extend_from_slice(&payload);
        let err = parse_iso21496_fmt(&with_urn, Iso21496Format::JxlJhgm)
            .expect_err("URN-prefixed input must not parse as JpegApp2");
        assert!(
            matches!(err, GainMapParseError::UnsupportedVersion { .. }),
            "expected UnsupportedVersion, got {err:?}"
        );
    }

    #[test]
    fn parse_jpegapp2_rejects_jpeg_app2_marker_prefixed_input() {
        // Asserts: the parser's input contract excludes the FF E2 + length
        // JPEG APP2 segment header. (With those 4 bytes in front, the first
        // byte 0xFF becomes min_version MSB → min_version u16 = 0xFF??,
        // rejected.)
        let payload = serialize_iso21496_fmt(&framing_test_params(), Iso21496Format::JxlJhgm);
        let mut with_marker = Vec::with_capacity(4 + payload.len());
        with_marker.push(0xFF);
        with_marker.push(0xE2);
        let len = (2 + payload.len()) as u16;
        with_marker.extend_from_slice(&len.to_be_bytes());
        with_marker.extend_from_slice(&payload);
        let err = parse_iso21496_fmt(&with_marker, Iso21496Format::JxlJhgm)
            .expect_err("APP2-marker-prefixed input must not parse as JpegApp2");
        assert!(
            matches!(err, GainMapParseError::UnsupportedVersion { .. }),
            "expected UnsupportedVersion, got {err:?}"
        );
    }

    #[test]
    fn parse_aviftmap_rejects_nonzero_version_byte() {
        // Asserts: the parser rejects AVIF `tmap` payloads with a
        // version byte other than 0. This pins the documented behaviour of
        // the version prefix: a single byte, fixed at 0.
        let mut blob = serialize_iso21496_fmt(&framing_test_params(), Iso21496Format::AvifTmap);
        blob[0] = 1; // rewrite version byte
        let err = parse_iso21496_fmt(&blob, Iso21496Format::AvifTmap)
            .expect_err("non-zero version must be rejected");
        assert!(
            matches!(err, GainMapParseError::UnsupportedVersion { version: 1 }),
            "expected UnsupportedVersion {{ version: 1 }}, got {err:?}"
        );
    }

    #[test]
    fn aviftmap_and_jpegapp2_are_the_only_framing_layers_we_own() {
        // Asserts: no other AVIF/JXL/JPEG outer-framing bytes leak into
        // zencodec's output. Combined with the two "no URN" / "no tmap"
        // tests above, this is a sanity check against accidentally growing
        // the output contract.
        let p = framing_test_params();
        let jpeg = serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm);
        let avif = serialize_iso21496_fmt(&p, Iso21496Format::AvifTmap);

        // No JPEG SOI / APP2 / EOI markers.
        for needle in [&[0xFFu8, 0xD8][..], &[0xFF, 0xE2][..], &[0xFF, 0xD9][..]] {
            assert!(
                !jpeg.windows(needle.len()).any(|w| w == needle),
                "JpegApp2 must not contain JPEG marker {needle:x?}"
            );
            assert!(
                !avif.windows(needle.len()).any(|w| w == needle),
                "AvifTmap must not contain JPEG marker {needle:x?}"
            );
        }

        // No ISOBMFF box type names zencodec's scope doesn't cover.
        for needle in [&b"jhgm"[..], &b"tmap"[..], &b"meta"[..], &b"iloc"[..]] {
            assert!(
                !jpeg.windows(needle.len()).any(|w| w == needle),
                "JpegApp2 must not contain ISOBMFF box type {:?}",
                core::str::from_utf8(needle).unwrap()
            );
            assert!(
                !avif.windows(needle.len()).any(|w| w == needle),
                "AvifTmap must not contain ISOBMFF box type {:?}",
                core::str::from_utf8(needle).unwrap()
            );
        }
    }

    // --- URN + version signal ---

    #[test]
    fn urn_constant_shape() {
        assert_eq!(ISO_21496_1_URN.len(), 28);
        assert_eq!(ISO_21496_1_URN[27], 0, "URN must end with NUL");
        assert_eq!(
            &ISO_21496_1_URN[..27],
            b"urn:iso:std:iso:ts:21496:-1",
            "URN must be the ISO 21496-1 namespace string"
        );
    }

    #[test]
    fn primary_app2_body_shape() {
        assert_eq!(ISO_21496_1_PRIMARY_APP2_BODY.len(), 32);
        assert_eq!(
            &ISO_21496_1_PRIMARY_APP2_BODY[..ISO_21496_1_URN.len()],
            ISO_21496_1_URN
        );
        assert_eq!(
            &ISO_21496_1_PRIMARY_APP2_BODY[ISO_21496_1_URN.len()..],
            &[0u8, 0, 0, 0],
            "primary-image version tail must be min_ver=0, writer_ver=0 (4 zero bytes)"
        );
    }

    #[test]
    fn primary_app2_body_is_not_parseable_as_full_metadata() {
        // The 4-byte tail after the URN is the `GainMapVersion` structure
        // alone — no flags byte, no channel data. Parsing under
        // JpegApp2BodyWithUrn must fail with TruncatedData, which is how
        // callers distinguish the primary-image signal from a full
        // parameter payload.
        let err = parse_iso21496_fmt(
            ISO_21496_1_PRIMARY_APP2_BODY,
            Iso21496Format::JpegApp2BodyWithUrn,
        )
        .expect_err("primary signal must not parse as full metadata");
        assert!(
            matches!(err, GainMapParseError::TruncatedData { .. }),
            "expected TruncatedData, got {err:?}"
        );
    }

    #[test]
    fn serialize_iso21496_fmt_into_appends_without_truncating_existing_content() {
        let p = GainMapParams {
            alternate_hdr_headroom: 1.3,
            ..Default::default()
        };
        let prefix = [0xDE, 0xAD, 0xBE, 0xEF];
        let mut buf = prefix.to_vec();
        serialize_iso21496_fmt_into(&p, Iso21496Format::JxlJhgm, &mut buf);

        assert_eq!(&buf[..prefix.len()], &prefix);
        let standalone = serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm);
        assert_eq!(&buf[prefix.len()..], &standalone[..]);
    }

    #[test]
    fn jpeg_app2_with_urn_serialize_starts_with_urn_then_jxl_jhgm_bytes() {
        let p = GainMapParams {
            alternate_hdr_headroom: 1.3,
            ..Default::default()
        };
        let with_urn = serialize_iso21496_fmt(&p, Iso21496Format::JpegApp2BodyWithUrn);
        let bare = serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm);

        assert_eq!(with_urn.len(), ISO_21496_1_URN.len() + bare.len());
        assert_eq!(&with_urn[..ISO_21496_1_URN.len()], ISO_21496_1_URN);
        assert_eq!(&with_urn[ISO_21496_1_URN.len()..], &bare[..]);
    }

    #[test]
    fn roundtrip_jpeg_app2_with_urn() {
        let original = GainMapParams {
            alternate_hdr_headroom: 2.0,
            ..Default::default()
        };
        let bytes = serialize_iso21496_fmt(&original, Iso21496Format::JpegApp2BodyWithUrn);
        let parsed = parse_iso21496_fmt(&bytes, Iso21496Format::JpegApp2BodyWithUrn).unwrap();
        assert!((parsed.alternate_hdr_headroom - original.alternate_hdr_headroom).abs() < 1e-5);
    }

    #[test]
    fn parse_jpeg_app2_with_urn_rejects_missing_urn() {
        // Valid bare JxlJhgm bytes without the URN prefix — must not parse
        // under JpegApp2BodyWithUrn.
        let bare = serialize_iso21496_fmt(&GainMapParams::default(), Iso21496Format::JxlJhgm);
        let err = parse_iso21496_fmt(&bare, Iso21496Format::JpegApp2BodyWithUrn).unwrap_err();
        assert!(matches!(err, GainMapParseError::UrnMismatch));
    }

    #[test]
    fn parse_jpeg_app2_with_urn_rejects_wrong_urn() {
        // ICC_PROFILE-style identifier that happens to be 28 bytes — same
        // length as the ISO URN but different content.
        let fake = b"NOT:iso:std:iso:ts:21496:-1\0";
        assert_eq!(fake.len(), ISO_21496_1_URN.len());
        let mut data = fake.to_vec();
        data.extend_from_slice(&serialize_iso21496_fmt(
            &GainMapParams::default(),
            Iso21496Format::JxlJhgm,
        ));
        let err = parse_iso21496_fmt(&data, Iso21496Format::JpegApp2BodyWithUrn).unwrap_err();
        assert!(matches!(err, GainMapParseError::UrnMismatch));
    }

    #[test]
    fn parse_jpeg_app2_with_urn_rejects_short_input() {
        assert!(matches!(
            parse_iso21496_fmt(&[], Iso21496Format::JpegApp2BodyWithUrn),
            Err(GainMapParseError::UrnMismatch)
        ));
        assert!(matches!(
            parse_iso21496_fmt(&ISO_21496_1_URN[..10], Iso21496Format::JpegApp2BodyWithUrn),
            Err(GainMapParseError::UrnMismatch)
        ));
    }

    #[test]
    #[allow(deprecated)]
    fn deprecated_jpeg_app2_produces_same_bytes_as_jxl_jhgm() {
        let p = GainMapParams {
            alternate_hdr_headroom: 1.5,
            ..Default::default()
        };
        let jxl_bytes = serialize_iso21496_fmt(&p, Iso21496Format::JxlJhgm);
        let old_bytes = serialize_iso21496_fmt(&p, Iso21496Format::JpegApp2);
        assert_eq!(jxl_bytes, old_bytes);

        let parsed_jxl = parse_iso21496_fmt(&jxl_bytes, Iso21496Format::JxlJhgm).unwrap();
        let parsed_old = parse_iso21496_fmt(&jxl_bytes, Iso21496Format::JpegApp2).unwrap();
        assert_eq!(parsed_jxl, parsed_old);
    }

    #[test]
    #[allow(deprecated)]
    fn iso21496_format_discriminants_are_pinned() {
        assert_eq!(Iso21496Format::JpegApp2 as u8, 0);
        assert_eq!(Iso21496Format::AvifTmap as u8, 1);
        assert_eq!(Iso21496Format::JxlJhgm as u8, 2);
        assert_eq!(Iso21496Format::JpegApp2BodyWithUrn as u8, 3);
        assert_ne!(Iso21496Format::JpegApp2, Iso21496Format::JxlJhgm);
    }
}
