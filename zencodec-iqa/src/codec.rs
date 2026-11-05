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
//!         EvalOpts::new().with_max_pixels(16_000_000))
//! }
//! ```

use crate::{At, ResultAtExt};
use crate::{Error, ErrorKind, EvalOpts, Evaluation, Metric, Result};
use zencodec::{
    CodecErrorExt, ErrorCategory,
    collection::Composition,
    image::{ImageOutput, OutputError},
};
use zenpixels::Orientation;

#[track_caller]
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
        return crate::fail(ErrorKind::UnsupportedInput);
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
#[track_caller]
pub fn score_outputs(
    metric: &mut dyn Metric,
    reference: &ImageOutput,
    candidate: &ImageOutput,
    opts: EvalOpts<'_>,
) -> Result<Evaluation> {
    crate::operation(opts, "score decoded outputs", |opts| {
        opts.checkpoint().at()?;
        admit(reference).at()?;
        admit(candidate).at()?;
        reference
            .with_pixels(|reference| {
                candidate
                    .with_pixels(|candidate| crate::score(metric, reference, candidate, opts))
                    .map_err(mapping_error)
                    .at()?
            })
            .map_err(mapping_error)
            .at()?
    })
}

fn mapping_error(cause: At<OutputError>) -> At<Error> {
    let kind = match cause.error_category() {
        Some(ErrorCategory::Stopped(reason)) => ErrorKind::Stopped(reason),
        _ if matches!(cause.error(), OutputError::NotPackedColor) => ErrorKind::UnsupportedInput,
        _ => ErrorKind::Backend,
    };
    Error::from_located(kind, cause)
}
