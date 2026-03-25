//! Owned metadata for encode/decode roundtrip.
//!
//! [`Metadata`] carries ICC, EXIF, XMP, CICP, HDR, and orientation data
//! using `Arc<[u8]>` for byte buffers (cheap cloning via ref-count bump).

use alloc::sync::Arc;

use crate::Orientation;
use crate::info::{Cicp, ContentLightLevel, MasteringDisplay};
use zenpixels::{ColorPrimaries, TransferFunction};

/// Owned image metadata for encode/decode roundtrip.
///
/// Byte buffers (ICC, EXIF, XMP) use `Arc<[u8]>` so cloning is a cheap
/// ref-count bump. Construct via [`Metadata::none()`] + builders,
/// or extract from decoded info via `From<&ImageInfo>`.
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
}

// Metadata contains 3× Option<Arc<[u8]>> (fat pointers), so size varies by
// pointer width. Catch unexpected growth from new fields or alignment changes.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<Metadata>() == 104);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::size_of::<Metadata>() <= 72);

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

    /// Derive the transfer function from CICP metadata.
    ///
    /// Returns the [`TransferFunction`] corresponding to the CICP
    /// `transfer_characteristics` code, or [`Unknown`](TransferFunction::Unknown)
    /// if CICP is absent or the code is not recognized.
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImageFormat;

    #[test]
    fn metadata_roundtrip() {
        let info = crate::ImageInfo::new(100, 200, ImageFormat::Jpeg)
            .with_icc_profile(alloc::vec![1, 2, 3])
            .with_exif(alloc::vec![4, 5])
            .with_cicp(Cicp::SRGB)
            .with_content_light_level(ContentLightLevel {
                max_content_light_level: 1000,
                max_frame_average_light_level: 400,
            });
        let meta = info.metadata();
        assert_eq!(meta.icc_profile.as_deref(), Some([1, 2, 3].as_slice()));
        assert_eq!(meta.exif.as_deref(), Some([4, 5].as_slice()));
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
        let meta = Metadata::none();
        assert!(meta.is_empty());
    }

    #[test]
    fn metadata_with_cicp_not_empty() {
        let meta = Metadata::none().with_cicp(Cicp::SRGB);
        assert!(!meta.is_empty());
    }

    #[test]
    fn metadata_with_hdr_not_empty() {
        let meta = Metadata::none().with_content_light_level(ContentLightLevel {
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
        let meta = Metadata::none();
        assert_eq!(meta.orientation, Orientation::Normal);
    }

    #[test]
    fn metadata_with_orientation_builder() {
        let meta = Metadata::none().with_orientation(Orientation::Rotate270);
        assert_eq!(meta.orientation, Orientation::Rotate270);
    }

    #[test]
    fn metadata_orientation_not_empty() {
        let meta = Metadata::none().with_orientation(Orientation::Rotate90);
        assert!(!meta.is_empty());
    }

    #[test]
    fn metadata_normal_orientation_is_empty() {
        let meta = Metadata::none().with_orientation(Orientation::Normal);
        assert!(meta.is_empty());
    }

    #[test]
    fn metadata_transfer_function() {
        let meta = Metadata::none().with_cicp(Cicp::SRGB);
        assert_eq!(meta.transfer_function(), TransferFunction::Srgb);

        let meta = Metadata::none();
        assert_eq!(meta.transfer_function(), TransferFunction::Unknown);
    }

    #[test]
    fn metadata_builder() {
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
    fn metadata_from_image_info() {
        let info = crate::ImageInfo::new(100, 200, ImageFormat::Jpeg)
            .with_icc_profile(alloc::vec![10, 20, 30])
            .with_exif(alloc::vec![4, 5])
            .with_cicp(Cicp::SRGB)
            .with_orientation(Orientation::Rotate270);
        let meta = Metadata::from(&info);
        assert_eq!(meta.icc_profile.as_deref(), Some([10, 20, 30].as_slice()));
        assert_eq!(meta.exif.as_deref(), Some([4, 5].as_slice()));
        assert_eq!(meta.cicp, Some(Cicp::SRGB));
        assert_eq!(meta.orientation, Orientation::Rotate270);
    }
}
