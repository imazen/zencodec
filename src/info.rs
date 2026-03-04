//! Image metadata types.

use alloc::sync::Arc;
use alloc::vec::Vec;

use zenpixels::{ColorPrimaries, PixelDescriptor, TransferFunction};
use crate::color::{ColorContext, ColorProfileSource};
use crate::gainmap::GainMapMetadata;
use crate::{ImageFormat, Orientation};

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

    /// Display primaries as CIE 1931 xy coordinates: `[[Rx, Ry], [Gx, Gy], [Bx, By]]`.
    pub fn primaries_f64(&self) -> [[f64; 2]; 3] {
        self.primaries
            .map(|[x, y]| [x as f64 * 0.00002, y as f64 * 0.00002])
    }

    /// White point as CIE 1931 xy coordinates: `[x, y]`.
    pub fn white_point_f64(&self) -> [f64; 2] {
        [
            self.white_point[0] as f64 * 0.00002,
            self.white_point[1] as f64 * 0.00002,
        ]
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

    /// Get the source color profile for CMS integration.
    pub fn color_profile_source(&self) -> Option<ColorProfileSource<'_>> {
        if let Some(cicp) = self.cicp {
            Some(ColorProfileSource::Cicp(cicp))
        } else {
            self.icc_profile.as_deref().map(ColorProfileSource::Icc)
        }
    }

    /// Build a [`ColorContext`] from the embedded ICC and CICP metadata.
    pub fn color_context(&self) -> Option<Arc<ColorContext>> {
        if self.icc_profile.is_some() || self.cicp.is_some() {
            Some(Arc::new(ColorContext {
                icc: self.icc_profile.clone(),
                cicp: self.cicp,
            }))
        } else {
            None
        }
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
    pub exif: Option<Vec<u8>>,
    /// Embedded XMP metadata.
    pub xmp: Option<Vec<u8>>,
}

impl EmbeddedMetadata {
    /// Set the EXIF metadata.
    pub fn with_exif(mut self, exif: Vec<u8>) -> Self {
        self.exif = Some(exif);
        self
    }

    /// Set the XMP metadata.
    pub fn with_xmp(mut self, xmp: Vec<u8>) -> Self {
        self.xmp = Some(xmp);
        self
    }

    /// Whether any metadata is present.
    pub fn is_empty(&self) -> bool {
        self.exif.is_none() && self.xmp.is_none()
    }
}

/// Image metadata obtained from probing or decoding.
#[derive(Clone, Debug, PartialEq)]
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
    /// Whether the image contains an HDR gain map (ISO 21496-1).
    ///
    /// When `true`, the image carries a secondary gain map image that
    /// enables continuous adaptation between SDR and HDR rendering.
    /// Detected via UltraHDR XMP metadata (JPEG), `tmap` box (AVIF/HEIF),
    /// or gain map bundle (JPEG XL).
    pub has_gain_map: bool,
    /// Gain map metadata parameters (ISO 21496-1).
    ///
    /// Present when `has_gain_map` is true and the metadata was successfully
    /// parsed. Describes the mathematical relationship between the base image
    /// and the gain map image. The gain map pixel data is a separate image
    /// accessible via [`DecodeOutput::extras`](crate::DecodeOutput).
    pub gain_map_metadata: Option<GainMapMetadata>,
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
            has_gain_map: false,
            gain_map_metadata: None,
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
    pub fn with_exif(mut self, exif: Vec<u8>) -> Self {
        self.embedded_metadata.exif = Some(exif);
        self
    }

    /// Set the XMP metadata. Convenience for `embedded_metadata.xmp`.
    pub fn with_xmp(mut self, xmp: Vec<u8>) -> Self {
        self.embedded_metadata.xmp = Some(xmp);
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

    /// Set whether the image contains an HDR gain map.
    pub fn with_gain_map(mut self, has: bool) -> Self {
        self.has_gain_map = has;
        self
    }

    /// Set the gain map metadata (ISO 21496-1 parameters).
    ///
    /// Also sets `has_gain_map` to `true`.
    pub fn with_gain_map_metadata(mut self, meta: GainMapMetadata) -> Self {
        self.gain_map_metadata = Some(meta);
        self.has_gain_map = true;
        self
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

    /// Get the source color profile for CMS integration.
    ///
    /// Delegates to [`SourceColor::color_profile_source()`].
    pub fn color_profile_source(&self) -> Option<ColorProfileSource<'_>> {
        self.source_color.color_profile_source()
    }

    /// Build a [`ColorContext`] from the embedded ICC and CICP metadata.
    ///
    /// Delegates to [`SourceColor::color_context()`].
    pub fn color_context(&self) -> Option<Arc<ColorContext>> {
        self.source_color.color_context()
    }

    /// Borrow embedded metadata for roundtrip encode.
    pub fn metadata(&self) -> MetadataView<'_> {
        MetadataView {
            icc_profile: self.source_color.icc_profile.as_deref(),
            exif: self.embedded_metadata.exif.as_deref(),
            xmp: self.embedded_metadata.xmp.as_deref(),
            cicp: self.source_color.cicp,
            content_light_level: self.source_color.content_light_level,
            mastering_display: self.source_color.mastering_display,
            orientation: self.orientation,
        }
    }
}

/// Borrowed view of image metadata (ICC/EXIF/XMP/CICP/HDR/orientation).
///
/// Used when encoding to preserve metadata from the source image.
/// Borrows from [`ImageInfo`] or user-provided slices. CICP, HDR,
/// and orientation are `Copy` types, so no borrowing needed for those.
///
/// Orientation is mutable because callers frequently resolve it during
/// transcoding (apply rotation, then set to [`Normal`](Orientation::Normal)
/// before re-encoding).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MetadataView<'a> {
    /// ICC color profile.
    pub icc_profile: Option<&'a [u8]>,
    /// EXIF metadata.
    pub exif: Option<&'a [u8]>,
    /// XMP metadata.
    pub xmp: Option<&'a [u8]>,
    /// CICP color description.
    pub cicp: Option<Cicp>,
    /// Content Light Level Info for HDR content.
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering Display Color Volume for HDR content.
    pub mastering_display: Option<MasteringDisplay>,
    /// EXIF orientation.
    ///
    /// Set to [`Normal`](Orientation::Normal) after applying rotation,
    /// or preserve the original value for the encoder to embed.
    pub orientation: Orientation,
}

impl Default for MetadataView<'_> {
    fn default() -> Self {
        Self {
            icc_profile: None,
            exif: None,
            xmp: None,
            cicp: None,
            content_light_level: None,
            mastering_display: None,
            orientation: Orientation::Normal,
        }
    }
}

/// Owned image metadata for cross-boundary transfer.
///
/// Like [`MetadataView`] but owns its byte buffers. Use when metadata
/// must outlive the source (pipelines, caches, async boundaries).
///
/// Convert from borrowed views via `From<MetadataView<'_>>`, or extract
/// from decoded info via `From<&ImageInfo>`.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Metadata {
    /// ICC color profile.
    pub icc_profile: Option<Vec<u8>>,
    /// EXIF metadata.
    pub exif: Option<Vec<u8>>,
    /// XMP metadata.
    pub xmp: Option<Vec<u8>>,
    /// CICP color description.
    pub cicp: Option<Cicp>,
    /// Content Light Level Info for HDR content.
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering Display Color Volume for HDR content.
    pub mastering_display: Option<MasteringDisplay>,
    /// EXIF orientation.
    pub orientation: Orientation,
}

impl Metadata {
    /// Create empty metadata.
    pub fn none() -> Self {
        Self::default()
    }

    /// Set the ICC color profile.
    pub fn with_icc(mut self, icc: Vec<u8>) -> Self {
        self.icc_profile = Some(icc);
        self
    }

    /// Set the EXIF metadata.
    pub fn with_exif(mut self, exif: Vec<u8>) -> Self {
        self.exif = Some(exif);
        self
    }

    /// Set the XMP metadata.
    pub fn with_xmp(mut self, xmp: Vec<u8>) -> Self {
        self.xmp = Some(xmp);
        self
    }

    /// Set the CICP color description.
    pub fn with_cicp(mut self, cicp: Cicp) -> Self {
        self.cicp = Some(cicp);
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

    /// Set the EXIF orientation.
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Borrow as a [`MetadataView`].
    pub fn as_view(&self) -> MetadataView<'_> {
        MetadataView {
            icc_profile: self.icc_profile.as_deref(),
            exif: self.exif.as_deref(),
            xmp: self.xmp.as_deref(),
            cicp: self.cicp,
            content_light_level: self.content_light_level,
            mastering_display: self.mastering_display,
            orientation: self.orientation,
        }
    }

    /// Whether any metadata is present.
    pub fn is_empty(&self) -> bool {
        self.icc_profile.is_none()
            && self.exif.is_none()
            && self.xmp.is_none()
            && self.cicp.is_none()
            && self.content_light_level.is_none()
            && self.mastering_display.is_none()
            && self.orientation == Orientation::Normal
    }
}

impl From<MetadataView<'_>> for Metadata {
    fn from(view: MetadataView<'_>) -> Self {
        Self {
            icc_profile: view.icc_profile.map(|s| s.to_vec()),
            exif: view.exif.map(|s| s.to_vec()),
            xmp: view.xmp.map(|s| s.to_vec()),
            cicp: view.cicp,
            content_light_level: view.content_light_level,
            mastering_display: view.mastering_display,
            orientation: view.orientation,
        }
    }
}

impl From<&ImageInfo> for Metadata {
    fn from(info: &ImageInfo) -> Self {
        Self {
            icc_profile: info.source_color.icc_profile.as_deref().map(|s| s.to_vec()),
            exif: info.embedded_metadata.exif.clone(),
            xmp: info.embedded_metadata.xmp.clone(),
            cicp: info.source_color.cicp,
            content_light_level: info.source_color.content_light_level,
            mastering_display: info.source_color.mastering_display,
            orientation: info.orientation,
        }
    }
}

impl<'a> MetadataView<'a> {
    /// Create empty metadata.
    pub fn none() -> Self {
        Self::default()
    }

    /// Set the ICC color profile.
    pub fn with_icc(mut self, icc: &'a [u8]) -> Self {
        self.icc_profile = Some(icc);
        self
    }

    /// Set the EXIF metadata.
    pub fn with_exif(mut self, exif: &'a [u8]) -> Self {
        self.exif = Some(exif);
        self
    }

    /// Set the XMP metadata.
    pub fn with_xmp(mut self, xmp: &'a [u8]) -> Self {
        self.xmp = Some(xmp);
        self
    }

    /// Set the CICP color description.
    pub fn with_cicp(mut self, cicp: Cicp) -> Self {
        self.cicp = Some(cicp);
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

    /// Set the EXIF orientation.
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Derive the transfer function from CICP metadata.
    ///
    /// Returns the [`TransferFunction`](TransferFunction) corresponding
    /// to the CICP `transfer_characteristics` code, or
    /// [`Unknown`](TransferFunction::Unknown) if CICP is absent or
    /// the code is not recognized.
    pub fn transfer_function(&self) -> TransferFunction {
        self.cicp
            .and_then(|c| TransferFunction::from_cicp(c.transfer_characteristics))
            .unwrap_or(TransferFunction::Unknown)
    }

    /// Derive the color primaries from CICP metadata.
    ///
    /// Returns [`Bt709`](ColorPrimaries::Bt709) if CICP is absent.
    pub fn color_primaries(&self) -> ColorPrimaries {
        self.cicp
            .map(|c| c.color_primaries_enum())
            .unwrap_or(ColorPrimaries::Bt709)
    }

    /// Get the source color profile for CMS integration.
    ///
    /// Returns CICP if present (takes precedence per AVIF/HEIF specs),
    /// otherwise returns the ICC profile. Returns `None` if neither is
    /// available — callers should assume sRGB in that case.
    pub fn color_profile_source(&self) -> Option<ColorProfileSource<'a>> {
        if let Some(cicp) = self.cicp {
            Some(ColorProfileSource::Cicp(cicp))
        } else {
            self.icc_profile.map(ColorProfileSource::Icc)
        }
    }

    /// Whether any metadata is present.
    ///
    /// Returns `false` if orientation is not [`Normal`](Orientation::Normal),
    /// since orientation is meaningful metadata for roundtrip encoding.
    pub fn is_empty(&self) -> bool {
        self.icc_profile.is_none()
            && self.exif.is_none()
            && self.xmp.is_none()
            && self.cicp.is_none()
            && self.content_light_level.is_none()
            && self.mastering_display.is_none()
            && self.orientation == Orientation::Normal
    }
}

/// Predicted output from a decode operation.
///
/// Returned by [`DecodeJob::output_info()`](crate::DecodeJob::output_info).
/// Describes what `decode()` or `decode_into()` will produce given the
/// current decode hints (crop, scale, orientation).
///
/// Use this to allocate destination buffers — the `width` and `height`
/// are what the decoder will actually write.
///
/// # Natural info vs output info
///
/// [`ImageInfo`] from `probe_header()` describes the file as stored:
/// original dimensions, original orientation, embedded metadata.
///
/// `OutputInfo` describes the decoder's output: post-crop, post-scale,
/// post-orientation dimensions and pixel format. This is what your
/// buffer must match.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutputInfo {
    /// Width of the decoded output in pixels.
    pub width: u32,
    /// Height of the decoded output in pixels.
    pub height: u32,
    /// Pixel format the decoder will produce natively (for `decode()`).
    ///
    /// For `decode_into()`, use any format from
    /// [`supported_descriptors()`](crate::DecoderConfig::supported_descriptors) —
    /// this field tells you what the codec would pick if you let it choose.
    pub native_format: PixelDescriptor,
    /// Whether the output has an alpha channel.
    pub has_alpha: bool,
    /// Orientation the decoder will apply internally.
    ///
    /// [`Normal`](Orientation::Normal) means the decoder will NOT handle
    /// orientation — the caller must apply it. Any other value means the
    /// decoder will rotate/flip the pixels, and the output `width`/`height`
    /// already reflect the rotated dimensions.
    ///
    /// Remaining orientation for the caller:
    /// `natural.orientation - orientation_applied` (via D4 group composition).
    pub orientation_applied: Orientation,
    /// Crop the decoder will actually apply (`[x, y, width, height]` in
    /// source coordinates).
    ///
    /// May differ from the crop hint due to block alignment (JPEG MCU
    /// boundaries, AV1 superblock alignment, etc.). `None` if no crop.
    pub crop_applied: Option<[u32; 4]>,
}

impl OutputInfo {
    /// Create an `OutputInfo` for a simple full-frame decode (no hints applied).
    pub fn full_decode(width: u32, height: u32, native_format: PixelDescriptor) -> Self {
        Self {
            width,
            height,
            native_format,
            has_alpha: native_format.has_alpha(),
            orientation_applied: Orientation::Normal,
            crop_applied: None,
        }
    }

    /// Set whether the output has alpha.
    pub fn with_alpha(mut self, has_alpha: bool) -> Self {
        self.has_alpha = has_alpha;
        self
    }

    /// Set the orientation the decoder will apply.
    pub fn with_orientation_applied(mut self, o: Orientation) -> Self {
        self.orientation_applied = o;
        self
    }

    /// Set the crop the decoder will apply.
    pub fn with_crop_applied(mut self, rect: [u32; 4]) -> Self {
        self.crop_applied = Some(rect);
        self
    }

    /// Minimum buffer size in bytes for the native format (no padding).
    ///
    /// This is `width * height * bytes_per_pixel`. For aligned/strided
    /// buffers, use [`PixelDescriptor::aligned_stride()`] instead.
    pub fn buffer_size(&self) -> u64 {
        self.width as u64 * self.height as u64 * self.native_format.bytes_per_pixel() as u64
    }

    /// Pixel count (`width * height`).
    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// Estimated resource cost of a decode operation.
///
/// Returned by `DecodeJob::estimated_cost()`.
/// Use this for resource management: reject oversized images, limit
/// concurrency, enforce memory budgets, or choose processing strategies
/// before committing to a decode.
///
/// `output_bytes` and `pixel_count` are always accurate (derived from
/// [`OutputInfo`]). `peak_memory` is a codec-specific estimate and may
/// be `None` if the codec can't predict it.
///
/// Use [`ResourceLimits::check_decode_cost()`](crate::ResourceLimits::check_decode_cost)
/// to validate against limits.
///
/// # Typical working memory multipliers (over output buffer size)
///
/// | Codec | Multiplier | Notes |
/// |-------|-----------|-------|
/// | JPEG | ~1-2x | DCT blocks + Huffman state |
/// | PNG | ~1-2x | Filter + zlib state |
/// | GIF | ~1-2x | LZW + frame compositing canvas |
/// | WebP lossy | ~2x | VP8 reference frames |
/// | AV1/AVIF | ~2-3x | Tile buffers + CDEF + loop restoration + reference frames |
/// | JPEG XL to u8 | ~1-2x | Native format output |
/// | JPEG XL to f32 | ~4x | Float conversion overhead |
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeCost {
    /// Output buffer size in bytes (width × height × bytes_per_pixel).
    pub output_bytes: u64,
    /// Total pixels in the output (width × height).
    pub pixel_count: u64,
    /// Estimated peak memory during decode, in bytes.
    ///
    /// Includes working buffers (YUV planes, entropy decode state, etc.)
    /// plus the output buffer. `None` if the codec can't estimate this.
    ///
    /// When `None`, callers should fall back to `output_bytes` as a
    /// lower-bound estimate for limit checks.
    pub peak_memory: Option<u64>,
}

impl DecodeCost {
    /// Create a decode cost estimate.
    pub const fn new(output_bytes: u64, pixel_count: u64, peak_memory: Option<u64>) -> Self {
        Self {
            output_bytes,
            pixel_count,
            peak_memory,
        }
    }
}

/// Estimated resource cost of an encode operation.
///
/// Returned by `EncodeJob::estimated_cost()`.
/// Use this for resource management before committing to an encode.
///
/// The caller already knows the input dimensions and pixel format, so
/// `input_bytes` and `pixel_count` are provided for convenience (the
/// caller could compute these). `peak_memory` is the useful codec-specific
/// estimate.
///
/// Use [`ResourceLimits::check_encode_cost()`](crate::ResourceLimits::check_encode_cost)
/// to validate against limits.
///
/// # Typical working memory multipliers (over input buffer size)
///
/// | Codec | Multiplier | Notes |
/// |-------|-----------|-------|
/// | JPEG | ~2-3x | DCT blocks + Huffman coding |
/// | PNG | ~2x | Filter selection + zlib |
/// | GIF | ~1-2x | LZW + quantization palette |
/// | WebP lossy | ~3-4x | VP8 RDO + reference frames |
/// | AV1/AVIF | ~4-8x | Transform + RDO + reference frames |
/// | JPEG XL lossless | ~12x | Float buffers + ANS tokens |
/// | JPEG XL lossy | ~6-22x | Highly variable with effort/quality |
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeCost {
    /// Input buffer size in bytes (width × height × bytes_per_pixel).
    pub input_bytes: u64,
    /// Total pixels in the input (width × height).
    pub pixel_count: u64,
    /// Estimated peak memory during encode, in bytes.
    ///
    /// Includes input pixel data, working buffers (transform coefficients,
    /// entropy coding state, rate-distortion buffers), and output buffer.
    /// `None` if the codec can't estimate this.
    ///
    /// When `None`, callers should fall back to `input_bytes` as a
    /// lower-bound estimate for limit checks.
    pub peak_memory: Option<u64>,
}

impl EncodeCost {
    /// Create an encode cost estimate.
    pub const fn new(input_bytes: u64, pixel_count: u64, peak_memory: Option<u64>) -> Self {
        Self {
            input_bytes,
            pixel_count,
            peak_memory,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_roundtrip() {
        let info = ImageInfo::new(100, 200, ImageFormat::Jpeg)
            .with_frame_count(1)
            .with_icc_profile(alloc::vec![1, 2, 3])
            .with_exif(alloc::vec![4, 5])
            .with_cicp(Cicp::SRGB)
            .with_content_light_level(ContentLightLevel {
                max_content_light_level: 1000,
                max_frame_average_light_level: 400,
            });
        let meta = info.metadata();
        assert_eq!(meta.icc_profile, Some([1, 2, 3].as_slice()));
        assert_eq!(meta.exif, Some([4, 5].as_slice()));
        assert!(meta.xmp.is_none());
        assert_eq!(meta.cicp, Some(Cicp::SRGB));
        assert_eq!(
            meta.content_light_level.unwrap().max_content_light_level,
            1000
        );
        assert!(meta.mastering_display.is_none());
        assert!(!meta.is_empty());
    }

    #[test]
    fn metadata_empty() {
        let meta = MetadataView::none();
        assert!(meta.is_empty());
    }

    #[test]
    fn metadata_equality() {
        let a = MetadataView::none().with_icc(&[1, 2, 3]);
        let b = MetadataView::none().with_icc(&[1, 2, 3]);
        assert_eq!(a, b);

        let c = MetadataView::none().with_icc(&[4, 5]);
        assert_ne!(a, c);
    }

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
        assert!(Cicp::SRGB.full_range);
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
    fn metadata_with_cicp_not_empty() {
        let meta = MetadataView::none().with_cicp(Cicp::SRGB);
        assert!(!meta.is_empty());
    }

    #[test]
    fn metadata_with_hdr_not_empty() {
        let meta = MetadataView::none().with_content_light_level(ContentLightLevel {
            max_content_light_level: 1000,
            max_frame_average_light_level: 400,
        });
        assert!(!meta.is_empty());
    }

    #[test]
    fn mastering_display_helpers() {
        // BT.2020 primaries from the spec
        let mdcv = MasteringDisplay {
            primaries: [[34000, 16000], [13250, 34500], [7500, 3000]],
            white_point: [15635, 16450],
            max_luminance: 10_000_000, // 1000 nits
            min_luminance: 500,        // 0.05 nits
        };
        let p = mdcv.primaries_f64();
        assert!((p[0][0] - 0.680).abs() < 0.001); // Rx
        assert!((p[0][1] - 0.320).abs() < 0.001); // Ry
        assert!((p[1][0] - 0.265).abs() < 0.001); // Gx
        assert!((p[1][1] - 0.690).abs() < 0.001); // Gy
        assert!((p[2][0] - 0.150).abs() < 0.001); // Bx
        assert!((p[2][1] - 0.060).abs() < 0.001); // By

        let wp = mdcv.white_point_f64();
        assert!((wp[0] - 0.3127).abs() < 0.001); // D65 x
        assert!((wp[1] - 0.3290).abs() < 0.001); // D65 y

        assert!((mdcv.max_luminance_nits() - 1000.0).abs() < 0.01);
        assert!((mdcv.min_luminance_nits() - 0.05).abs() < 0.001);
    }

    #[test]
    fn metadata_orientation_roundtrip() {
        let info =
            ImageInfo::new(100, 200, ImageFormat::Jpeg).with_orientation(Orientation::Rotate90);
        let meta = info.metadata();
        assert_eq!(meta.orientation, Orientation::Rotate90);
    }

    #[test]
    fn metadata_orientation_default_is_normal() {
        let meta = MetadataView::none();
        assert_eq!(meta.orientation, Orientation::Normal);
    }

    #[test]
    fn metadata_with_orientation_builder() {
        let meta = MetadataView::none().with_orientation(Orientation::Rotate270);
        assert_eq!(meta.orientation, Orientation::Rotate270);
    }

    #[test]
    fn metadata_orientation_not_empty() {
        let meta = MetadataView::none().with_orientation(Orientation::Rotate90);
        assert!(!meta.is_empty());
    }

    #[test]
    fn metadata_normal_orientation_is_empty() {
        let meta = MetadataView::none().with_orientation(Orientation::Normal);
        assert!(meta.is_empty());
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
    fn metadata_transfer_function() {
        use TransferFunction;

        let meta = MetadataView::none().with_cicp(Cicp::SRGB);
        assert_eq!(meta.transfer_function(), TransferFunction::Srgb);

        let meta = MetadataView::none();
        assert_eq!(meta.transfer_function(), TransferFunction::Unknown);
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
    fn gain_map_metadata_builder() {
        let meta = crate::GainMapMetadata {
            gain_map_max: [2.0, 2.0, 2.0],
            hdr_capacity_max: 2.0,
            ..crate::GainMapMetadata::default()
        };
        let info = ImageInfo::new(100, 100, ImageFormat::Jpeg).with_gain_map_metadata(meta);
        assert!(info.has_gain_map);
        assert!(info.gain_map_metadata.is_some());
        let gm = info.gain_map_metadata.unwrap();
        assert_eq!(gm.gain_map_max, [2.0, 2.0, 2.0]);
        assert_eq!(gm.hdr_capacity_max, 2.0);
    }

    #[test]
    fn owned_metadata_none() {
        let meta = Metadata::none();
        assert!(meta.is_empty());
    }

    #[test]
    fn owned_metadata_builder() {
        let meta = Metadata::none()
            .with_icc(alloc::vec![1, 2, 3])
            .with_exif(alloc::vec![4, 5])
            .with_cicp(Cicp::SRGB)
            .with_orientation(Orientation::Rotate90);
        assert!(!meta.is_empty());
        assert_eq!(meta.icc_profile.as_deref(), Some([1, 2, 3].as_slice()));
        assert_eq!(meta.exif.as_deref(), Some([4, 5].as_slice()));
        assert!(meta.xmp.is_none());
        assert_eq!(meta.cicp, Some(Cicp::SRGB));
        assert_eq!(meta.orientation, Orientation::Rotate90);
    }

    #[test]
    fn owned_metadata_as_view_roundtrip() {
        let meta = Metadata::none()
            .with_icc(alloc::vec![10, 20])
            .with_cicp(Cicp::BT2100_PQ)
            .with_content_light_level(ContentLightLevel::new(4000, 1000));
        let view = meta.as_view();
        assert_eq!(view.icc_profile, Some([10, 20].as_slice()));
        assert_eq!(view.cicp, Some(Cicp::BT2100_PQ));
        assert_eq!(
            view.content_light_level.unwrap().max_content_light_level,
            4000
        );
    }

    #[test]
    fn owned_metadata_from_view() {
        let view = MetadataView::none()
            .with_icc(&[1, 2, 3])
            .with_exif(&[4, 5])
            .with_cicp(Cicp::SRGB);
        let owned = Metadata::from(view.clone());
        assert_eq!(owned.icc_profile.as_deref(), Some([1, 2, 3].as_slice()));
        assert_eq!(owned.exif.as_deref(), Some([4, 5].as_slice()));
        assert_eq!(owned.cicp, Some(Cicp::SRGB));
        // Round-trip back to view
        let view2 = owned.as_view();
        assert_eq!(view, view2);
    }

    #[test]
    fn owned_metadata_from_image_info() {
        let info = ImageInfo::new(100, 200, ImageFormat::Jpeg)
            .with_icc_profile(alloc::vec![10, 20, 30])
            .with_exif(alloc::vec![4, 5])
            .with_cicp(Cicp::SRGB)
            .with_orientation(Orientation::Rotate270);
        let owned = Metadata::from(&info);
        assert_eq!(owned.icc_profile.as_deref(), Some([10, 20, 30].as_slice()));
        assert_eq!(owned.exif.as_deref(), Some([4, 5].as_slice()));
        assert_eq!(owned.cicp, Some(Cicp::SRGB));
        assert_eq!(owned.orientation, Orientation::Rotate270);
    }

    #[test]
    fn gain_map_metadata_default_none() {
        let info = ImageInfo::new(100, 100, ImageFormat::Jpeg);
        assert!(!info.has_gain_map);
        assert!(info.gain_map_metadata.is_none());
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
