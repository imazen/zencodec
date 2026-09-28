//! Retention checks for a pipeline retaining a gain-map rendition.
//! Extract vendor/XMP parameters into [`GainMapParams`] before calling this.
//! Container writers must regenerate gain-map discovery/parameters separately
//! from source XMP, and filter metadata inside encoded auxiliary images too.
use crate::{GainMapParams, Metadata, MetadataPolicy};

/// Retention would change rendering, or the gain-map parameters are invalid.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Invalid typed parameters; preserve the diagnostic from the gain-map parser.
    GainMap(crate::gainmap::GainMapParseError),
    /// A rendering field was removed without an accompanying pixel conversion.
    Removed(&'static str),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::GainMap(e) => write!(f, "invalid gain map: {e}"),
            Self::Removed(field) => {
                write!(f, "retention would remove display information: {field}")
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
    let out = meta.filtered(policy);
    if out.orientation != meta.orientation {
        return Err(Error::Removed("orientation"));
    }
    if out.cicp != meta.cicp {
        return Err(Error::Removed("CICP"));
    }
    if out.content_light_level != meta.content_light_level
        || out.mastering_display != meta.mastering_display
        || out.diffuse_white != meta.diffuse_white
    {
        return Err(Error::Removed("HDR luminance"));
    }
    if meta.icc_profile.is_some()
        && out.icc_profile.is_none()
        && !meta
            .icc_profile
            .as_ref()
            .is_some_and(|icc| zenpixels::icc::is_common_srgb(icc))
    {
        return Err(Error::Removed("ICC profile"));
    }
    Ok(out)
}
