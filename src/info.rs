//! Image metadata types.
//!
//! Core types for describing image properties: dimensions, format,
//! color space, HDR metadata, and embedded metadata blobs.

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::detect::SourceEncodingDetails;
use crate::metadata::Metadata;
use crate::{ImageFormat, Orientation};
use zenpixels::{ColorPrimaries, TransferFunction};

/// Re-export CICP from zenpixels — the canonical definition.
pub use zenpixels::Cicp;

/// Content Light Level Info (CEA-861.3).
///
/// Describes the light level of HDR content. Used alongside [`MasteringDisplay`]
/// to guide tone mapping on displays with different capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ContentLightLevel {
    /// Maximum Content Light Level in cd/m² (nits).
    /// Peak luminance of any single pixel in the content.
    pub max_content_light_level: u16,
    /// Maximum Frame-Average Light Level in cd/m² (nits).
    /// Peak average luminance of any single frame.
    pub max_frame_average_light_level: u16,
}

impl ContentLightLevel {
    /// Create content light level info.
    pub const fn new(max_content_light_level: u16, max_frame_average_light_level: u16) -> Self {
        Self {
            max_content_light_level,
            max_frame_average_light_level,
        }
    }
}

/// Mastering Display Color Volume (SMPTE ST 2086).
///
/// Describes the color volume of the display used to master HDR content.
/// Chromaticity values are in units of 0.00002 (as per the spec).
/// Luminance values are in units of 0.0001 cd/m².
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct MasteringDisplay {
    /// Display primaries chromaticity [R, G, B], each as [x, y].
    /// Values in units of 0.00002. (50000 = 1.0)
    pub primaries: [[u16; 2]; 3],
    /// White point chromaticity [x, y].
    /// Values in units of 0.00002. (50000 = 1.0)
    pub white_point: [u16; 2],
    /// Maximum display luminance in units of 0.0001 cd/m².
    pub max_luminance: u32,
    /// Minimum display luminance in units of 0.0001 cd/m².
    pub min_luminance: u32,
}

impl MasteringDisplay {
    /// Create mastering display metadata.
    pub const fn new(
        primaries: [[u16; 2]; 3],
        white_point: [u16; 2],
        max_luminance: u32,
        min_luminance: u32,
    ) -> Self {
        Self {
            primaries,
            white_point,
            max_luminance,
            min_luminance,
        }
    }

    /// Maximum display luminance in cd/m² (nits).
    pub fn max_luminance_nits(&self) -> f64 {
        self.max_luminance as f64 * 0.0001
    }

    /// Minimum display luminance in cd/m² (nits).
    pub fn min_luminance_nits(&self) -> f64 {
        self.min_luminance as f64 * 0.0001
    }
}

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
    /// Whether the image contains animation (multiple frames).
    pub has_animation: bool,
    /// Number of frames (None if unknown without full parse).
    pub frame_count: Option<u32>,
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
    /// Other fields default to no alpha, no animation, no metadata.
    /// Use the `with_*` builder methods to set them.
    pub fn new(width: u32, height: u32, format: ImageFormat) -> Self {
        Self {
            width,
            height,
            format,
            has_alpha: false,
            has_animation: false,
            frame_count: None,
            orientation: Orientation::Normal,
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

    /// Set whether the image is animated.
    pub fn with_animation(mut self, has_animation: bool) -> Self {
        self.has_animation = has_animation;
        self
    }

    /// Set the frame count.
    pub fn with_frame_count(mut self, count: u32) -> Self {
        self.frame_count = Some(count);
        self
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
            .field("has_animation", &self.has_animation)
            .field("frame_count", &self.frame_count)
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
            && self.has_animation == other.has_animation
            && self.frame_count == other.frame_count
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
            .with_animation(true)
            .with_frame_count(5)
            .with_icc_profile(alloc::vec![1, 2])
            .with_exif(alloc::vec![3, 4])
            .with_xmp(alloc::vec![5, 6]);
        assert!(info.has_alpha);
        assert!(info.has_animation);
        assert_eq!(info.frame_count, Some(5));
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
        let clli = ContentLightLevel {
            max_content_light_level: 4000,
            max_frame_average_light_level: 1000,
        };
        let mdcv = MasteringDisplay {
            primaries: [[34000, 16000], [13250, 34500], [7500, 3000]],
            white_point: [15635, 16450],
            max_luminance: 40000000,
            min_luminance: 50,
        };
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
            40000000
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
        assert_eq!(s, "BT.709/sRGB / sRGB / BT.601 (full range)");
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
