//! Retention checks for a pipeline retaining a gain-map rendition.
//! Extract vendor/XMP parameters into [`GainMapParams`] before calling this.
//! Container writers must regenerate gain-map discovery/parameters separately
//! from source XMP, and filter metadata inside encoded auxiliary images too.
use zencodec::{GainMapParams, Metadata, MetadataPolicy};

/// A rendering-critical signal that a metadata policy attempted to remove.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RenderSignal {
    /// EXIF/container pixel orientation.
    Orientation,
    /// Coding-independent code points (primaries, transfer, matrix, range).
    Cicp,
    /// HDR luminance metadata (content light level, mastering display, diffuse white).
    HdrLuminance,
    /// Non-sRGB ICC color profile.
    IccProfile,
}

impl core::fmt::Display for RenderSignal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Orientation => "orientation",
            Self::Cicp => "CICP",
            Self::HdrLuminance => "HDR luminance",
            Self::IccProfile => "ICC profile",
        })
    }
}

/// Retention would change rendering, or the gain-map parameters are invalid.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Invalid typed parameters; preserve the diagnostic from the gain-map parser.
    GainMap(zencodec::gainmap::GainMapParseError),
    /// A rendering field was removed without an accompanying pixel conversion.
    RemovedSignal(RenderSignal),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::GainMap(e) => write!(f, "invalid gain map: {e}"),
            Self::RemovedSignal(signal) => {
                write!(f, "retention would remove display information: {signal}")
            }
        }
    }
}
impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::GainMap(e) => Some(e),
            _ => None,
        }
    }
}
/// Filter source metadata while retaining a gain map with unchanged base pixels.
///
/// This does not parse vendor data, scan pixels, copy gain-map storage, certify
/// arbitrary ICC contents, or sanitize containers. It couples an explicit policy
/// with already extracted parameters and refuses removal of known render signals.
/// An SDR fallback or orientation bake is a separate, explicit pixel operation.
/// The target codec must serialize `params` anew, even when source XMP is dropped.
pub fn filter_for_gain_map(
    meta: &Metadata,
    params: &GainMapParams,
    policy: &MetadataPolicy,
) -> Result<Metadata, Error> {
    params.validate().map_err(Error::GainMap)?;
    let out = crate::metadata::filter(meta, policy);
    if out.orientation != meta.orientation {
        return Err(Error::RemovedSignal(RenderSignal::Orientation));
    }
    if out.cicp != meta.cicp {
        return Err(Error::RemovedSignal(RenderSignal::Cicp));
    }
    if out.content_light_level != meta.content_light_level
        || out.mastering_display != meta.mastering_display
        || out.diffuse_white != meta.diffuse_white
    {
        return Err(Error::RemovedSignal(RenderSignal::HdrLuminance));
    }
    if meta.icc_profile.is_some()
        && out.icc_profile.is_none()
        && !meta
            .icc_profile
            .as_ref()
            .is_some_and(|icc| zenpixels::icc::IccIdentification::is_common_srgb(icc))
    {
        return Err(Error::RemovedSignal(RenderSignal::IccProfile));
    }
    Ok(out)
}
