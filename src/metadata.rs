//! Metadata transfer types for roundtrip encode/decode.
//!
//! [`MetadataView`] borrows from [`ImageInfo`] or user-provided slices.
//! [`Metadata`] owns its byte buffers for cross-boundary transfer.

use alloc::sync::Arc;

use crate::Orientation;
use crate::info::{Cicp, ContentLightLevel, MasteringDisplay, Resolution};
use zenpixels::{ColorPrimaries, ColorProfileSource, TransferFunction};

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
    /// Physical resolution (DPI / pixels-per-cm / pixels-per-meter).
    ///
    /// Extracted from format-specific containers (JFIF, pHYs, TIFF IFD)
    /// and written back by encoders that support resolution metadata.
    pub resolution: Option<Resolution>,
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
            resolution: None,
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
    pub icc_profile: Option<Arc<[u8]>>,
    /// EXIF metadata.
    pub exif: Option<Arc<[u8]>>,
    /// XMP metadata.
    pub xmp: Option<Arc<[u8]>>,
    /// CICP color description.
    pub cicp: Option<Cicp>,
    /// Content Light Level Info for HDR content.
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering Display Color Volume for HDR content.
    pub mastering_display: Option<MasteringDisplay>,
    /// EXIF orientation.
    pub orientation: Orientation,
    /// Physical resolution (DPI / pixels-per-cm / pixels-per-meter).
    pub resolution: Option<Resolution>,
}

impl Metadata {
    /// Create empty metadata.
    pub fn none() -> Self {
        Self::default()
    }

    /// Set the ICC color profile.
    ///
    /// Accepts `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
    pub fn with_icc(mut self, icc: impl Into<Arc<[u8]>>) -> Self {
        self.icc_profile = Some(icc.into());
        self
    }

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

    /// Set the physical resolution.
    pub fn with_resolution(mut self, resolution: Resolution) -> Self {
        self.resolution = Some(resolution);
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
            resolution: self.resolution,
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
            && self.resolution.is_none()
    }
}

impl From<MetadataView<'_>> for Metadata {
    fn from(view: MetadataView<'_>) -> Self {
        Self {
            icc_profile: view.icc_profile.map(Arc::from),
            exif: view.exif.map(Arc::from),
            xmp: view.xmp.map(Arc::from),
            cicp: view.cicp,
            content_light_level: view.content_light_level,
            mastering_display: view.mastering_display,
            orientation: view.orientation,
            resolution: view.resolution,
        }
    }
}

impl From<&crate::ImageInfo> for Metadata {
    fn from(info: &crate::ImageInfo) -> Self {
        Self {
            icc_profile: info.source_color.icc_profile.clone(),
            exif: info.embedded_metadata.exif.clone(),
            xmp: info.embedded_metadata.xmp.clone(),
            cicp: info.source_color.cicp,
            content_light_level: info.source_color.content_light_level,
            mastering_display: info.source_color.mastering_display,
            orientation: info.orientation,
            resolution: None,
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

    /// Set the physical resolution.
    pub fn with_resolution(mut self, resolution: Resolution) -> Self {
        self.resolution = Some(resolution);
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

    /// ICC color profile, if present.
    pub fn icc_profile(&self) -> Option<&'a [u8]> {
        self.icc_profile
    }

    /// EXIF metadata, if present.
    pub fn exif(&self) -> Option<&'a [u8]> {
        self.exif
    }

    /// XMP metadata, if present.
    pub fn xmp(&self) -> Option<&'a [u8]> {
        self.xmp
    }

    /// CICP color description, if present.
    pub fn cicp(&self) -> Option<Cicp> {
        self.cicp
    }

    /// Content Light Level Info, if present.
    pub fn content_light_level(&self) -> Option<ContentLightLevel> {
        self.content_light_level
    }

    /// Mastering Display Color Volume, if present.
    pub fn mastering_display(&self) -> Option<MasteringDisplay> {
        self.mastering_display
    }

    /// EXIF orientation.
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    /// Physical resolution, if present.
    pub fn resolution(&self) -> Option<Resolution> {
        self.resolution
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
            && self.resolution.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImageFormat;

    #[test]
    fn metadata_roundtrip() {
        let info = crate::ImageInfo::new(100, 200, ImageFormat::Jpeg)
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
    fn metadata_orientation_roundtrip() {
        let info = crate::ImageInfo::new(100, 200, ImageFormat::Jpeg)
            .with_orientation(Orientation::Rotate90);
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
    fn metadata_transfer_function() {
        use TransferFunction;

        let meta = MetadataView::none().with_cicp(Cicp::SRGB);
        assert_eq!(meta.transfer_function(), TransferFunction::Srgb);

        let meta = MetadataView::none();
        assert_eq!(meta.transfer_function(), TransferFunction::Unknown);
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
        let info = crate::ImageInfo::new(100, 200, ImageFormat::Jpeg)
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
}
