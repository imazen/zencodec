//! Optional bridge from resolved, packed zencodec outputs.
//!
//! Prepare display composition, orientation, crop and color explicitly before
//! calling this bridge (for example with zencodec-render). Native components
//! require a separate preparation route; no RGB interpretation is invented.
//!
//! ```
//! use zencodec::image::ImageOutput;
//! use zencodec_iqa::iqa::{Metric, EvalOpts, MetricResult};
//! fn compare_decoded<M: Metric>(
//!     metric: &mut M,
//!     reference: &ImageOutput,
//!     candidate: &ImageOutput,
//! ) -> MetricResult<M::Output> {
//!     zencodec_iqa::score_outputs(metric, reference, candidate,
//!         EvalOpts::new().with_max_pixels(16_000_000))
//! }
//! ```

use crate::iqa::{self, ErrorKind, EvalOpts, Metric, MetricError, MetricResult};
use iqa::how_far::{Complete, PhaseSpec, Stages, Total};
use iqa::whereat::{At, ResultAtExt};
use iqa::zenpixels::Orientation;
use zencodec::{
    CodecErrorExt, ErrorCategory,
    collection::Composition,
    image::{ImageOutput, OutputError},
};

#[track_caller]
fn admit(image: &ImageOutput) -> MetricResult<()> {
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
        return iqa::fail(MetricError::new(ErrorKind::UnsupportedInput));
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
pub fn score_outputs<M: Metric>(
    metric: &mut M,
    reference: &ImageOutput,
    candidate: &ImageOutput,
    opts: EvalOpts<'_>,
) -> MetricResult<M::Output> {
    operation(opts, |opts| {
        opts.checkpoint().at()?;
        admit(reference).at()?;
        admit(candidate).at()?;
        reference
            .with_pixels(|reference| {
                candidate
                    .with_pixels(|candidate| iqa::score(metric, &reference, &candidate, opts))
                    .map_err(mapping_error)
                    .at()?
            })
            .map_err(mapping_error)
            .at()?
    })
}

fn mapping_error(cause: At<OutputError>) -> At<MetricError> {
    let kind = match cause.error_category() {
        Some(ErrorCategory::Stopped(reason)) => ErrorKind::Stopped(reason),
        _ if matches!(cause.error(), OutputError::NotPackedColor) => ErrorKind::UnsupportedInput,
        _ => ErrorKind::Backend,
    };
    MetricError::from_located(kind, cause)
}

// The adapter owns mapping and admission as one operation. Core scoring owns
// its nested phase; neither can complete the caller's borrowed parent.
#[track_caller]
fn operation<T>(
    opts: EvalOpts<'_>,
    work: impl FnOnce(EvalOpts<'_>) -> MetricResult<T>,
) -> MetricResult<T> {
    let mut stages = Stages::new(
        opts.pulse(),
        &[PhaseSpec::new("score decoded outputs", 1, Total::Unknown)],
    );
    let result = stages.run_classified(iqa::is_stopped, |pulse| work(opts.with_pulse(pulse)));
    stages.complete_classified(result, iqa::is_stopped).at()
}
