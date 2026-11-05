//! Optional bridge from resolved, packed zencodec outputs.
//!
//! Prepare display composition, orientation, crop and color explicitly before
//! calling this bridge (for example with zencodec-render). Native components
//! require a separate preparation route; no RGB interpretation is invented.
//!
//! ```
//! use zencodec::image::ImageOutput;
//! use zencodec_iqa::{Metric, EvalOpts, Evaluation, Result};
//! fn compare_decoded(
//!     metric: &mut dyn Metric,
//!     reference: &ImageOutput,
//!     candidate: &ImageOutput,
//! ) -> Result<Evaluation> {
//!     zencodec_iqa::codec::score_outputs(metric, reference, candidate,
//!         EvalOpts { max_pixels: Some(16_000_000), ..EvalOpts::default() })
//! }
//! ```

use crate::{Error, ErrorKind, EvalOpts, Evaluation, Metric, Result};
use whereat::At;
use zencodec::{collection::Composition, image::ImageOutput};
use zenpixels::Orientation;

fn admit(image: &ImageOutput) -> Result<()> {
    let interpretation = image.interpretation();
    let geometry = interpretation.geometry;
    let region = geometry.region;
    if interpretation.composition != Some(Composition::Composited)
        || interpretation.orientation != Some(Orientation::Identity)
        || interpretation.placement.is_some()
        || interpretation.signal.is_some()
        || region.x != 0
        || region.y != 0
        || region.width != geometry.canvas_width
        || region.height != geometry.canvas_height
    {
        return Err(At::wrap(Error::new(ErrorKind::UnsupportedInput)));
    }
    Ok(())
}

/// Score full-canvas, composited, already-oriented packed output pairs.
///
/// Both mapping guards remain alive for the complete evaluation. Actual
/// descriptors, color contexts and strides pass unchanged to the scorer, which
/// must admit their color/alpha/precision/luminance interpretation. This bridge
/// does not decode, transform pixels, certify a renderer, or budget native
/// mapping allocations. Mapping failures retain their original cause.
pub fn score_outputs(
    metric: &mut dyn Metric,
    reference: &ImageOutput,
    candidate: &ImageOutput,
    opts: EvalOpts<'_>,
) -> Result<Evaluation> {
    opts.checkpoint()?;
    admit(reference)?;
    admit(candidate)?;
    reference
        .with_pixels(|reference| {
            candidate
                .with_pixels(|candidate| crate::score(metric, reference, candidate, opts))
                .map_err(|cause| At::wrap(Error::new(ErrorKind::Backend).with_cause(cause)))?
        })
        .map_err(|cause| At::wrap(Error::new(ErrorKind::Backend).with_cause(cause)))?
}
