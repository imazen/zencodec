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
#[derive(Clone, Debug, Default, PartialEq)]
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
    ///
    /// As a convenience, the Orientation tag (0x0112) is parsed from the
    /// blob and stored in `self.orientation` — but only if `self.orientation`
    /// is currently `Identity` (the default). Callers who set orientation
    /// explicitly via [`with_orientation`](Self::with_orientation) before
    /// `with_exif` keep their explicit value; callers who set it after
    /// also override the parsed one.
    pub fn with_exif(mut self, exif: impl Into<Arc<[u8]>>) -> Self {
        let bytes: Arc<[u8]> = exif.into();
        if self.orientation == Orientation::Identity
            && let Some(o) = parse_exif_orientation(&bytes)
        {
            self.orientation = o;
        }
        self.exif = Some(bytes);
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
            && self.orientation == Orientation::Identity
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

/// Parse the EXIF Orientation tag (0x0112) from a TIFF/EXIF blob.
///
/// Handles both little-endian (`II*\0`) and big-endian (`MM\0*`) byte
/// orders. Walks IFD0 and returns the first Orientation entry found.
/// Returns `None` if the blob is malformed or no Orientation tag exists.
fn parse_exif_orientation(blob: &[u8]) -> Option<Orientation> {
    if blob.len() < 8 {
        return None;
    }
    let little_endian = match &blob[0..4] {
        [b'I', b'I', 0x2a, 0x00] => true,
        [b'M', b'M', 0x00, 0x2a] => false,
        _ => return None,
    };
    let read_u16 = |offset: usize| -> Option<u16> {
        let bytes = blob.get(offset..offset + 2)?;
        Some(if little_endian {
            u16::from_le_bytes([bytes[0], bytes[1]])
        } else {
            u16::from_be_bytes([bytes[0], bytes[1]])
        })
    };
    let read_u32 = |offset: usize| -> Option<u32> {
        let bytes = blob.get(offset..offset + 4)?;
        Some(if little_endian {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        })
    };

    let ifd0_offset = read_u32(4)? as usize;
    let entry_count = read_u16(ifd0_offset)? as usize;
    let entries_start = ifd0_offset.checked_add(2)?;

    for i in 0..entry_count {
        let entry_offset = entries_start.checked_add(i.checked_mul(12)?)?;
        let tag = read_u16(entry_offset)?;
        if tag == 0x0112 {
            // Orientation tag: type SHORT (3), count 1, value inline at +8.
            let value = read_u16(entry_offset + 8)?;
            if value > 0 && value <= 8 {
                return Orientation::from_exif(value as u8);
            }
        }
    }
    None
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
        assert_eq!(meta.orientation, Orientation::Identity);
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
    fn metadata_identity_orientation_is_empty() {
        let meta = Metadata::none().with_orientation(Orientation::Identity);
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

    fn build_minimal_exif_with_orientation(value: u16, big_endian: bool) -> alloc::vec::Vec<u8> {
        let mut v = alloc::vec::Vec::new();
        if big_endian {
            v.extend_from_slice(b"MM\x00\x2a");
            v.extend_from_slice(&8u32.to_be_bytes());
            v.extend_from_slice(&1u16.to_be_bytes());
            v.extend_from_slice(&0x0112u16.to_be_bytes());
            v.extend_from_slice(&3u16.to_be_bytes());
            v.extend_from_slice(&1u32.to_be_bytes());
            // SHORT value is padded right within 4-byte value field; for BE
            // the value sits in the FIRST 2 bytes.
            v.extend_from_slice(&value.to_be_bytes());
            v.extend_from_slice(&[0u8, 0]);
            v.extend_from_slice(&0u32.to_be_bytes());
        } else {
            v.extend_from_slice(b"II\x2a\x00");
            v.extend_from_slice(&8u32.to_le_bytes());
            v.extend_from_slice(&1u16.to_le_bytes());
            v.extend_from_slice(&0x0112u16.to_le_bytes());
            v.extend_from_slice(&3u16.to_le_bytes());
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&(value as u32).to_le_bytes());
            v.extend_from_slice(&0u32.to_le_bytes());
        }
        v
    }

    #[test]
    fn parse_exif_orientation_le_returns_correct_variant() {
        let blob = build_minimal_exif_with_orientation(6, false);
        assert_eq!(parse_exif_orientation(&blob), Some(Orientation::Rotate90));
    }

    #[test]
    fn parse_exif_orientation_be_returns_correct_variant() {
        let blob = build_minimal_exif_with_orientation(6, true);
        assert_eq!(parse_exif_orientation(&blob), Some(Orientation::Rotate90));
    }

    #[test]
    fn parse_exif_orientation_garbage_returns_none() {
        assert_eq!(parse_exif_orientation(b"garbage"), None);
        assert_eq!(parse_exif_orientation(&[]), None);
        assert_eq!(parse_exif_orientation(&[0u8; 7]), None);
    }

    #[test]
    fn with_exif_auto_parses_orientation_from_blob() {
        let blob = build_minimal_exif_with_orientation(8, false);
        let meta = Metadata::none().with_exif(blob);
        assert_eq!(meta.orientation, Orientation::Rotate270);
    }

    #[test]
    fn with_exif_does_not_override_explicit_orientation() {
        let blob = build_minimal_exif_with_orientation(6, false);
        let meta = Metadata::none()
            .with_orientation(Orientation::FlipH)
            .with_exif(blob);
        // Explicit FlipH must win over the EXIF blob's Rotate90.
        assert_eq!(meta.orientation, Orientation::FlipH);
    }
}
