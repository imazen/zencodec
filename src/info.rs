//! Image metadata types.
//!
//! Core types for describing image properties: dimensions, format,
//! color space, HDR metadata, and embedded metadata blobs.

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::detect::SourceEncodingDetails;
use crate::gainmap::GainMapPresence;
use crate::metadata::Metadata;
use crate::{ImageFormat, Orientation};
use zenpixels::{ColorPrimaries, TransferFunction};

// Re-export color types from zenpixels — the canonical definitions.
pub use zenpixels::Cicp;

// =========================================================================
// ImageSequence
// =========================================================================

/// What kind of image sequence the file contains.
///
/// Determines which decoder trait is appropriate:
/// - `Single` → [`Decode`](crate::decode::Decode)
/// - `Animation` → [`FullFrameDecoder`](crate::decode::FullFrameDecoder)
/// - `Multi` → future `MultiPageDecoder` (or `Decode` for primary only)
///
/// # Key invariant
///
/// The variant tells you which decoder trait applies. Code that sees `Multi`
/// knows not to use `FullFrameDecoder`. Code that sees `Animation` knows
/// `MultiPageDecoder` is wrong. `Single` means only `Decode` is needed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImageSequence {
    /// Single image. `Decode` returns it.
    #[default]
    Single,

    /// Temporal animation: frames share a canvas size, have durations,
    /// and may use compositing (disposal, blending, reference slots).
    ///
    /// Use `FullFrameDecoder`.
    Animation {
        /// Number of displayed frames. `None` if unknown without full parse
        /// (e.g., GIF requires scanning all frames to count them).
        frame_count: Option<u32>,
        /// Loop count: 0 = infinite, N = play N times. `None` = unspecified.
        loop_count: Option<u32>,
        /// Whether frame N can be rendered without decoding frames 0..N-1.
        ///
        /// True when all frames are full-canvas replacements (no disposal
        /// dependencies). False for GIF/APNG with inter-frame disposal.
        /// JXL is typically true (keyframe-based).
        random_access: bool,
    },

    /// Multiple independent images in a single container.
    ///
    /// Pages may differ in dimensions, pixel format, color space, and
    /// metadata. `Decode` returns the primary image only. Other images
    /// require a `MultiPageDecoder` (future) or the codec's native API.
    ///
    /// Examples: multi-page TIFF, HEIF collections, ICO sizes, DICOM slices,
    /// GeoTIFF spectral bands.
    Multi {
        /// Number of primary-level images, excluding thumbnails, masks,
        /// and pyramid levels (those are reported via `Supplements`).
        ///
        /// `None` if unknown without full parse.
        image_count: Option<u32>,
        /// Whether image N can be decoded without decoding images 0..N-1.
        ///
        /// True for most container formats (TIFF IFDs, HEIF items, ICO
        /// entries) where each image is independently addressable.
        random_access: bool,
    },
}

impl ImageSequence {
    /// Frame/image count if known.
    ///
    /// - `Single` → `Some(1)`
    /// - `Animation` → `frame_count` (may be `None`)
    /// - `Multi` → `image_count` (may be `None`)
    pub fn count(&self) -> Option<u32> {
        match self {
            Self::Single => Some(1),
            Self::Animation { frame_count, .. } => *frame_count,
            Self::Multi { image_count, .. } => *image_count,
        }
    }

    /// Whether individual frames/images can be accessed without decoding all priors.
    pub fn random_access(&self) -> bool {
        match self {
            Self::Single => true,
            Self::Animation { random_access, .. } => *random_access,
            Self::Multi { random_access, .. } => *random_access,
        }
    }

    /// Whether this is an animation sequence.
    pub fn is_animation(&self) -> bool {
        matches!(self, Self::Animation { .. })
    }

    /// Whether this contains multiple independent images.
    pub fn is_multi(&self) -> bool {
        matches!(self, Self::Multi { .. })
    }
}

// =========================================================================
// Supplements
// =========================================================================

/// Supplemental content that accompanies the primary image(s).
///
/// These are not independent viewable images — they modify or augment
/// the primary content. Each supplement type implies a distinct access
/// pattern and a future accessor trait.
///
/// Populated during probe. May be incomplete from `probe()` (cheap) and
/// more complete from `probe_full()` (expensive).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Supplements {
    /// Reduced-resolution versions (image pyramid, thumbnails).
    ///
    /// TIFF pyramids, HEIF thumbnails, JPEG JFIF thumbnails.
    pub pyramid: bool,

    /// HDR gain map for SDR/HDR tone mapping.
    ///
    /// JPEG Ultra HDR (ISO 21496-1), AVIF gain map, JXL gain map,
    /// HEIF gain map.
    pub gain_map: bool,

    /// Depth map (portrait mode, 3D reconstruction).
    ///
    /// HEIF depth maps, Google Camera depth in JPEG, AVIF depth auxiliary.
    pub depth_map: bool,

    /// Segmentation mattes (portrait effects, hair, skin, teeth, glasses, sky).
    ///
    /// iPhone HEIC files with portrait mode or semantic segmentation.
    pub segmentation_mattes: bool,

    /// Other auxiliary images not covered by named fields.
    ///
    /// Alpha planes stored as separate images (HEIF), transparency masks
    /// (TIFF), vendor-specific auxiliary data.
    pub auxiliary: bool,
}

/// Physical pixel resolution (DPI or pixels-per-unit).
///
/// Sourced from JPEG JFIF density, PNG pHYs, TIFF XResolution/YResolution,
/// BMP biXPelsPerMeter/biYPelsPerMeter, etc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolution {
    /// Horizontal pixels per unit.
    pub x: f64,
    /// Vertical pixels per unit.
    pub y: f64,
    /// Unit of measurement.
    pub unit: ResolutionUnit,
}

impl Resolution {
    /// Resolution in dots per inch. Converts from centimeters if needed.
    pub fn dpi(&self) -> (f64, f64) {
        match self.unit {
            ResolutionUnit::Inch => (self.x, self.y),
            ResolutionUnit::Centimeter => (self.x * 2.54, self.y * 2.54),
            ResolutionUnit::Meter => (self.x * 0.0254, self.y * 0.0254),
            ResolutionUnit::Unknown => (self.x, self.y),
        }
    }
}

/// Unit for [`Resolution`] values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ResolutionUnit {
    /// Dots per inch (JPEG JFIF, TIFF).
    Inch,
    /// Dots per centimeter (TIFF).
    Centimeter,
    /// Pixels per meter (PNG pHYs, BMP).
    Meter,
    /// Unit unknown or not specified.
    #[default]
    Unknown,
}

pub use zenpixels::{ContentLightLevel, MasteringDisplay};

/// Source color description from the image file.
///
/// Groups color-related metadata from the original source: CICP tags,
/// ICC profile, bit depth, channel count, and HDR descriptors
/// (content light level, mastering display).
///
/// These describe the *source* color space — not the current pixel
/// data's color space (which is tracked by [`PixelDescriptor`]).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SourceColor {
    /// CICP color description (ITU-T H.273).
    ///
    /// When present, describes the color space without requiring an ICC
    /// profile. Both CICP and ICC may be present — CICP takes precedence
    /// per AVIF/HEIF specs, but callers should use ICC when CICP is absent.
    pub cicp: Option<Cicp>,
    /// Embedded ICC color profile.
    ///
    /// Stored as `Arc<[u8]>` for cheap sharing across pipeline stages
    /// and pixel slices. Accepts `Vec<u8>` via [`with_icc_profile()`](Self::with_icc_profile).
    pub icc_profile: Option<Arc<[u8]>>,
    /// Bits per channel (e.g. 8, 10, 12, 16, 32).
    ///
    /// `None` if unknown (e.g. from a header-only probe that doesn't
    /// report bit depth).
    pub bit_depth: Option<u8>,
    /// Number of channels (1=gray, 2=gray+alpha, 3=RGB, 4=RGBA).
    ///
    /// `None` if unknown.
    pub channel_count: Option<u8>,
    /// Content Light Level Info (CEA-861.3) for HDR content.
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering Display Color Volume (SMPTE ST 2086) for HDR content.
    pub mastering_display: Option<MasteringDisplay>,
}

impl SourceColor {
    /// Set the CICP color description.
    pub fn with_cicp(mut self, cicp: Cicp) -> Self {
        self.cicp = Some(cicp);
        self
    }

    /// Set the ICC color profile.
    pub fn with_icc_profile(mut self, icc: impl Into<Arc<[u8]>>) -> Self {
        self.icc_profile = Some(icc.into());
        self
    }

    /// Set the bit depth.
    pub fn with_bit_depth(mut self, bit_depth: u8) -> Self {
        self.bit_depth = Some(bit_depth);
        self
    }

    /// Set the channel count.
    pub fn with_channel_count(mut self, channel_count: u8) -> Self {
        self.channel_count = Some(channel_count);
        self
    }

    /// Set the Content Light Level Info.
    pub fn with_content_light_level(mut self, clli: ContentLightLevel) -> Self {
        self.content_light_level = Some(clli);
        self
    }

    /// Set the Mastering Display Color Volume.
    pub fn with_mastering_display(mut self, mdcv: MasteringDisplay) -> Self {
        self.mastering_display = Some(mdcv);
        self
    }

    /// Derive the transfer function from CICP metadata.
    pub fn transfer_function(&self) -> TransferFunction {
        self.cicp
            .and_then(|c| TransferFunction::from_cicp(c.transfer_characteristics))
            .unwrap_or(TransferFunction::Unknown)
    }

    /// Derive the color primaries from CICP metadata.
    pub fn color_primaries(&self) -> ColorPrimaries {
        self.cicp
            .map(|c| c.color_primaries_enum())
            .unwrap_or(ColorPrimaries::Bt709)
    }
}

/// Embedded non-color metadata from the image file.
///
/// Groups metadata blobs (EXIF, XMP) that are carried through
/// decode/encode for roundtrip preservation but don't affect
/// pixel interpretation.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct EmbeddedMetadata {
    /// Embedded EXIF metadata.
    pub exif: Option<Arc<[u8]>>,
    /// Embedded XMP metadata.
    pub xmp: Option<Arc<[u8]>>,
}

impl EmbeddedMetadata {
    /// Set the EXIF metadata.
    ///
    /// Accepts `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
    pub fn with_exif(mut self, exif: impl Into<Arc<[u8]>>) -> Self {
        self.exif = Some(exif.into());
        self
    }

    /// Set the XMP metadata.
    ///
    /// Accepts `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
    pub fn with_xmp(mut self, xmp: impl Into<Arc<[u8]>>) -> Self {
        self.xmp = Some(xmp.into());
        self
    }

    /// Whether any metadata is present.
    pub fn is_empty(&self) -> bool {
        self.exif.is_none() && self.xmp.is_none()
    }
}

/// Image metadata obtained from probing or decoding.
///
/// # Field scope by sequence type
///
/// | Field | Single | Animation | Multi |
/// |-------|--------|-----------|-------|
/// | `width`, `height` | The image | Canvas size | Primary image only |
/// | `has_alpha` | The image | Canvas alpha | Primary image only |
/// | `orientation` | The image | Canvas orientation | Primary image only |
/// | `source_color` | The image | Overall color info | Primary image only |
/// | `embedded_metadata` | The image | Container-level | Primary image only |
///
/// For `Multi`, other images may have completely different dimensions, color
/// spaces, and metadata. Per-image information requires a future
/// `MultiPageDecoder` or the codec's native API.
#[derive(Clone)]
#[non_exhaustive]
pub struct ImageInfo {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Detected image format.
    pub format: ImageFormat,
    /// Whether the image has an alpha channel.
    pub has_alpha: bool,
    /// What kind of image sequence the file contains.
    ///
    /// For `Single`, all fields describe the one image.
    /// For `Animation`, `width`/`height` are the canvas size.
    /// For `Multi`, `width`/`height` describe the primary image only.
    pub sequence: ImageSequence,
    /// Supplemental content alongside the primary image(s).
    ///
    /// Pyramids, gain maps, depth maps, and other auxiliary data.
    pub supplements: Supplements,
    /// Gain map detection state.
    ///
    /// Three-state: `Unknown` (not yet probed), `Absent` (definitively none),
    /// or `Available` (metadata parsed). When `Available`, the gain map
    /// image pixels are NOT included — only the metadata and dimensions.
    pub gain_map: GainMapPresence,
    /// EXIF orientation (1-8).
    ///
    /// When a codec applies orientation during decode (rotating the pixel
    /// data), this is set to [`Normal`](Orientation::Normal) and `width`/`height`
    /// reflect the display dimensions.
    ///
    /// When orientation is NOT applied, `width`/`height` are the stored
    /// dimensions and this field tells the caller what transform to apply.
    /// Use [`display_width()`](ImageInfo::display_width) /
    /// [`display_height()`](ImageInfo::display_height) to get effective
    /// display dimensions regardless.
    pub orientation: Orientation,
    /// Physical pixel resolution (DPI).
    ///
    /// From JPEG JFIF density, PNG pHYs, TIFF resolution tags, BMP
    /// pels-per-meter, etc. `None` if the file doesn't specify resolution.
    pub resolution: Option<Resolution>,

    /// Source color description (CICP, ICC, bit depth, HDR metadata).
    pub source_color: SourceColor,
    /// Embedded non-color metadata (EXIF, XMP).
    pub embedded_metadata: EmbeddedMetadata,
    /// Source encoding details (quality estimate, encoder fingerprint, etc.).
    ///
    /// Populated by codecs that can detect how the image was encoded.
    /// Use [`source_encoding_details()`](ImageInfo::source_encoding_details)
    /// for the generic interface and
    /// [`codec_details::<T>()`](dyn crate::SourceEncodingDetails::codec_details)
    /// for codec-specific fields.
    ///
    /// Skipped by `PartialEq` (trait objects aren't comparable).
    pub source_encoding: Option<Arc<dyn SourceEncodingDetails>>,
    /// Non-fatal diagnostic messages from probing or decoding.
    ///
    /// Populated when the operation succeeded but encountered unusual
    /// conditions (e.g., metadata located beyond the fast-path probe cap,
    /// permissive parsing recovered from structural issues).
    pub warnings: Vec<alloc::string::String>,
}

impl ImageInfo {
    /// Create a new `ImageInfo` with the given dimensions and format.
    ///
    /// Other fields default to no alpha, single image, no metadata.
    /// Use the `with_*` builder methods to set them.
    pub fn new(width: u32, height: u32, format: ImageFormat) -> Self {
        Self {
            width,
            height,
            format,
            has_alpha: false,
            sequence: ImageSequence::Single,
            supplements: Supplements::default(),
            gain_map: GainMapPresence::default(),
            orientation: Orientation::Normal,
            resolution: None,
            source_color: SourceColor::default(),
            embedded_metadata: EmbeddedMetadata::default(),
            source_encoding: None,
            warnings: Vec::new(),
        }
    }

    /// Set whether the image has alpha.
    pub fn with_alpha(mut self, has_alpha: bool) -> Self {
        self.has_alpha = has_alpha;
        self
    }

    /// Set the image sequence type.
    pub fn with_sequence(mut self, sequence: ImageSequence) -> Self {
        self.sequence = sequence;
        self
    }

    /// Set supplemental content flags.
    pub fn with_supplements(mut self, supplements: Supplements) -> Self {
        self.supplements = supplements;
        self
    }

    /// Set gain map detection state.
    pub fn with_gain_map(mut self, gain_map: GainMapPresence) -> Self {
        self.gain_map = gain_map;
        self
    }

    /// Set physical pixel resolution.
    pub fn with_resolution(mut self, resolution: Resolution) -> Self {
        self.resolution = Some(resolution);
        self
    }

    // --- Compatibility helpers ---

    /// Whether this file contains animation.
    ///
    /// Convenience for `matches!(self.sequence, ImageSequence::Animation { .. })`.
    pub fn is_animation(&self) -> bool {
        self.sequence.is_animation()
    }

    /// Whether this file contains multiple independent images.
    pub fn is_multi_image(&self) -> bool {
        self.sequence.is_multi()
    }

    /// Whether `Decode` returns only one of multiple images in this file.
    ///
    /// True for both animation and multi-image. When true, `Decode` returns
    /// the primary image and additional images require specialized decoders.
    pub fn has_additional_images(&self) -> bool {
        !matches!(self.sequence, ImageSequence::Single)
    }

    /// Frame/image count from the sequence, if known.
    pub fn frame_count(&self) -> Option<u32> {
        self.sequence.count()
    }

    /// Set the bit depth (bits per channel). Convenience for `source_color.bit_depth`.
    pub fn with_bit_depth(mut self, bit_depth: u8) -> Self {
        self.source_color.bit_depth = Some(bit_depth);
        self
    }

    /// Set the channel count. Convenience for `source_color.channel_count`.
    pub fn with_channel_count(mut self, channel_count: u8) -> Self {
        self.source_color.channel_count = Some(channel_count);
        self
    }

    /// Set the CICP color description. Convenience for `source_color.cicp`.
    pub fn with_cicp(mut self, cicp: Cicp) -> Self {
        self.source_color.cicp = Some(cicp);
        self
    }

    /// Set the Content Light Level Info. Convenience for `source_color.content_light_level`.
    pub fn with_content_light_level(mut self, clli: ContentLightLevel) -> Self {
        self.source_color.content_light_level = Some(clli);
        self
    }

    /// Set the Mastering Display Color Volume. Convenience for `source_color.mastering_display`.
    pub fn with_mastering_display(mut self, mdcv: MasteringDisplay) -> Self {
        self.source_color.mastering_display = Some(mdcv);
        self
    }

    /// Set the ICC color profile. Convenience for `source_color.icc_profile`.
    ///
    /// Accepts `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
    pub fn with_icc_profile(mut self, icc: impl Into<Arc<[u8]>>) -> Self {
        self.source_color.icc_profile = Some(icc.into());
        self
    }

    /// Set the EXIF metadata. Convenience for `embedded_metadata.exif`.
    ///
    /// Accepts `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
    pub fn with_exif(mut self, exif: impl Into<Arc<[u8]>>) -> Self {
        self.embedded_metadata.exif = Some(exif.into());
        self
    }

    /// Set the XMP metadata. Convenience for `embedded_metadata.xmp`.
    ///
    /// Accepts `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
    pub fn with_xmp(mut self, xmp: impl Into<Arc<[u8]>>) -> Self {
        self.embedded_metadata.xmp = Some(xmp.into());
        self
    }

    /// Set the EXIF orientation.
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Set the source color description.
    pub fn with_source_color(mut self, source_color: SourceColor) -> Self {
        self.source_color = source_color;
        self
    }

    /// Set the embedded metadata.
    pub fn with_embedded_metadata(mut self, embedded_metadata: EmbeddedMetadata) -> Self {
        self.embedded_metadata = embedded_metadata;
        self
    }

    /// Attach source encoding details (quality estimate, codec-specific probe data).
    ///
    /// The concrete type must implement [`SourceEncodingDetails`] — typically
    /// a codec's probe type (e.g. `WebPProbe`, `JpegProbe`).
    pub fn with_source_encoding_details<T: SourceEncodingDetails + 'static>(
        mut self,
        details: T,
    ) -> Self {
        self.source_encoding = Some(Arc::new(details));
        self
    }

    /// Source encoding details, if available.
    ///
    /// Returns the generic interface for querying source quality and losslessness.
    /// Downcast to the codec-specific type via
    /// [`codec_details::<T>()`](dyn SourceEncodingDetails::codec_details).
    pub fn source_encoding_details(&self) -> Option<&dyn SourceEncodingDetails> {
        self.source_encoding.as_deref()
    }

    /// Add a single warning message.
    pub fn with_warning(mut self, msg: alloc::string::String) -> Self {
        self.warnings.push(msg);
        self
    }

    /// Replace warnings with the given list.
    pub fn with_warnings(mut self, msgs: Vec<alloc::string::String>) -> Self {
        self.warnings = msgs;
        self
    }

    /// Non-fatal diagnostic messages.
    pub fn warnings(&self) -> &[alloc::string::String] {
        &self.warnings
    }

    /// Whether any warnings were recorded.
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Display width after applying EXIF orientation.
    ///
    /// For orientations 5-8 (90/270 rotation), this returns `height`.
    /// For orientations 1-4, this returns `width`.
    pub fn display_width(&self) -> u32 {
        if self.orientation.swaps_dimensions() {
            self.height
        } else {
            self.width
        }
    }

    /// Display height after applying EXIF orientation.
    ///
    /// For orientations 5-8 (90/270 rotation), this returns `width`.
    /// For orientations 1-4, this returns `height`.
    pub fn display_height(&self) -> u32 {
        if self.orientation.swaps_dimensions() {
            self.width
        } else {
            self.height
        }
    }

    /// Derive the transfer function from CICP metadata.
    ///
    /// Delegates to [`SourceColor::transfer_function()`].
    ///
    /// Use this to resolve a [`PixelDescriptor`]'s unknown transfer function:
    ///
    /// ```ignore
    /// let desc = pixels.descriptor().with_transfer(info.transfer_function());
    /// ```
    pub fn transfer_function(&self) -> TransferFunction {
        self.source_color.transfer_function()
    }

    /// Derive the color primaries from CICP metadata.
    ///
    /// Delegates to [`SourceColor::color_primaries()`].
    pub fn color_primaries(&self) -> ColorPrimaries {
        self.source_color.color_primaries()
    }

    /// Get embedded metadata for roundtrip encode.
    ///
    /// Clones Arc-backed byte buffers (cheap ref-count bump).
    pub fn metadata(&self) -> Metadata {
        Metadata::from(self)
    }
}

impl core::fmt::Debug for ImageInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = f.debug_struct("ImageInfo");
        s.field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("has_alpha", &self.has_alpha)
            .field("sequence", &self.sequence)
            .field("supplements", &self.supplements)
            .field("gain_map", &self.gain_map)
            .field("orientation", &self.orientation)
            .field("source_color", &self.source_color)
            .field("embedded_metadata", &self.embedded_metadata);
        if self.source_encoding.is_some() {
            s.field("source_encoding", &"Some(...)");
        }
        if !self.warnings.is_empty() {
            s.field("warnings", &self.warnings);
        }
        s.finish()
    }
}

/// Manual `PartialEq` — skips `source_encoding` (trait objects aren't comparable).
impl PartialEq for ImageInfo {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.format == other.format
            && self.has_alpha == other.has_alpha
            && self.sequence == other.sequence
            && self.supplements == other.supplements
            && self.gain_map == other.gain_map
            && self.orientation == other.orientation
            && self.source_color == other.source_color
            && self.embedded_metadata == other.embedded_metadata
            && self.warnings == other.warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_dimensions_normal() {
        let info = ImageInfo::new(100, 200, ImageFormat::Jpeg);
        assert_eq!(info.display_width(), 100);
        assert_eq!(info.display_height(), 200);
    }

    #[test]
    fn display_dimensions_rotated() {
        let info =
            ImageInfo::new(100, 200, ImageFormat::Jpeg).with_orientation(Orientation::Rotate90);
        assert_eq!(info.display_width(), 200);
        assert_eq!(info.display_height(), 100);
    }

    #[test]
    fn display_dimensions_rotate180() {
        let info =
            ImageInfo::new(100, 200, ImageFormat::Jpeg).with_orientation(Orientation::Rotate180);
        // 180 does not swap dimensions
        assert_eq!(info.display_width(), 100);
        assert_eq!(info.display_height(), 200);
    }

    #[test]
    fn display_dimensions_all_orientations() {
        let info = ImageInfo::new(100, 200, ImageFormat::Jpeg);
        for orient in [
            Orientation::Normal,
            Orientation::FlipHorizontal,
            Orientation::Rotate180,
            Orientation::FlipVertical,
        ] {
            let i = info.clone().with_orientation(orient);
            assert_eq!((i.display_width(), i.display_height()), (100, 200));
        }
        for orient in [
            Orientation::Transpose,
            Orientation::Rotate90,
            Orientation::Transverse,
            Orientation::Rotate270,
        ] {
            let i = info.clone().with_orientation(orient);
            assert_eq!((i.display_width(), i.display_height()), (200, 100));
        }
    }

    #[test]
    fn image_info_builder() {
        let info = ImageInfo::new(10, 20, ImageFormat::Png)
            .with_alpha(true)
            .with_sequence(ImageSequence::Animation {
                frame_count: Some(5),
                loop_count: None,
                random_access: false,
            })
            .with_icc_profile(alloc::vec![1, 2])
            .with_exif(alloc::vec![3, 4])
            .with_xmp(alloc::vec![5, 6]);
        assert!(info.has_alpha);
        assert!(info.is_animation());
        assert_eq!(info.frame_count(), Some(5));
        assert_eq!(
            info.source_color.icc_profile.as_deref(),
            Some([1, 2].as_slice())
        );
        assert_eq!(
            info.embedded_metadata.exif.as_deref(),
            Some([3, 4].as_slice())
        );
        assert_eq!(
            info.embedded_metadata.xmp.as_deref(),
            Some([5, 6].as_slice())
        );
    }

    #[test]
    fn image_info_eq() {
        let a = ImageInfo::new(10, 20, ImageFormat::Png).with_alpha(true);
        let b = ImageInfo::new(10, 20, ImageFormat::Png).with_alpha(true);
        assert_eq!(a, b);

        let c = ImageInfo::new(10, 20, ImageFormat::Jpeg).with_alpha(true);
        assert_ne!(a, c);
    }

    #[test]
    fn cicp_constants() {
        assert_eq!(Cicp::SRGB.color_primaries, 1);
        assert_eq!(Cicp::SRGB.transfer_characteristics, 13);
        assert_eq!(Cicp::BT2100_PQ.transfer_characteristics, 16);
        assert_eq!(Cicp::BT2100_HLG.transfer_characteristics, 18);
        const { assert!(Cicp::SRGB.full_range) };
    }

    #[test]
    fn image_info_bit_depth_channels() {
        let info = ImageInfo::new(100, 100, ImageFormat::Avif)
            .with_bit_depth(10)
            .with_channel_count(4)
            .with_alpha(true);
        assert_eq!(info.source_color.bit_depth, Some(10));
        assert_eq!(info.source_color.channel_count, Some(4));
    }

    #[test]
    fn image_info_hdr_metadata() {
        let clli = ContentLightLevel::new(4000, 1000);
        let mdcv = MasteringDisplay::new(
            [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]],
            [0.3127, 0.3290],
            4000.0,
            0.005,
        );
        let info = ImageInfo::new(3840, 2160, ImageFormat::Avif)
            .with_cicp(Cicp::BT2100_PQ)
            .with_content_light_level(clli)
            .with_mastering_display(mdcv);
        assert_eq!(info.source_color.cicp, Some(Cicp::BT2100_PQ));
        assert_eq!(
            info.source_color
                .content_light_level
                .unwrap()
                .max_content_light_level,
            4000
        );
        assert_eq!(
            info.source_color.mastering_display.unwrap().max_luminance,
            4000.0
        );
    }

    #[test]
    fn transfer_function_from_cicp() {
        use TransferFunction;

        let info = ImageInfo::new(100, 100, ImageFormat::Avif).with_cicp(Cicp::SRGB);
        assert_eq!(info.transfer_function(), TransferFunction::Srgb);

        let info = ImageInfo::new(100, 100, ImageFormat::Avif).with_cicp(Cicp::BT2100_PQ);
        assert_eq!(info.transfer_function(), TransferFunction::Pq);

        let info = ImageInfo::new(100, 100, ImageFormat::Avif).with_cicp(Cicp::BT2100_HLG);
        assert_eq!(info.transfer_function(), TransferFunction::Hlg);
    }

    #[test]
    fn transfer_function_without_cicp() {
        use TransferFunction;

        let info = ImageInfo::new(100, 100, ImageFormat::Jpeg);
        assert_eq!(info.transfer_function(), TransferFunction::Unknown);
    }

    #[test]
    fn transfer_function_unrecognized_cicp() {
        use TransferFunction;

        // CICP with unrecognized transfer characteristics code
        let info = ImageInfo::new(100, 100, ImageFormat::Avif).with_cicp(Cicp::new(1, 99, 0, true));
        assert_eq!(info.transfer_function(), TransferFunction::Unknown);
    }

    #[test]
    fn cicp_display_srgb() {
        let s = alloc::format!("{}", Cicp::SRGB);
        assert_eq!(s, "BT.709/sRGB / sRGB / Identity/RGB (full range)");
    }

    #[test]
    fn cicp_display_bt2100_pq() {
        let s = alloc::format!("{}", Cicp::BT2100_PQ);
        assert_eq!(s, "BT.2020 / PQ (HDR) / BT.2020 NCL (full range)");
    }

    #[test]
    fn cicp_display_limited_range() {
        let cicp = Cicp::new(1, 1, 1, false);
        let s = alloc::format!("{}", cicp);
        assert_eq!(s, "BT.709/sRGB / BT.709 / BT.709 (limited range)");
    }

    #[test]
    fn cicp_name_helpers() {
        assert_eq!(Cicp::color_primaries_name(1), "BT.709/sRGB");
        assert_eq!(Cicp::color_primaries_name(12), "Display P3");
        assert_eq!(Cicp::color_primaries_name(255), "Unknown");

        assert_eq!(Cicp::transfer_characteristics_name(13), "sRGB");
        assert_eq!(Cicp::transfer_characteristics_name(16), "PQ (HDR)");
        assert_eq!(Cicp::transfer_characteristics_name(18), "HLG (HDR)");

        assert_eq!(Cicp::matrix_coefficients_name(0), "Identity/RGB");
        assert_eq!(Cicp::matrix_coefficients_name(6), "BT.601");
        assert_eq!(Cicp::matrix_coefficients_name(9), "BT.2020 NCL");
    }
}
