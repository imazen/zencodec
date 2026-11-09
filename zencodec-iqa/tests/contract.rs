//! Resolved-output admission and actual pixel context controls.
use std::sync::Arc;
use zencodec_iqa::iqa::zenpixels::{self, ColorContext, PixelDescriptor};
use zencodec_iqa::iqa::*;
#[derive(Debug)]
struct ProbeResult;
impl Evaluation for ProbeResult {
    fn scores(&self) -> impl Iterator<Item = Score<'_>> {
        static SPEC: ScoreSpec = ScoreSpec::new("score", Direction::Unordered);
        core::iter::once(Score::new(&SPEC, 1.0))
    }
}
struct Probe {
    calls: usize,
}
impl Metric for Probe {
    type Output = ProbeResult;
    fn id(&self) -> MetricId {
        MetricId::new("probe", Implementation::new("adapter-control", "1", 1))
    }
    fn evaluate(
        &mut self,
        r: &zenpixels::PixelSlice<'_>,
        d: &zenpixels::PixelSlice<'_>,
        _: EvalOpts<'_>,
    ) -> MetricResult<ProbeResult> {
        self.calls += 1;
        if r.color_context().is_some() || d.color_context().is_some() {
            return fail(MetricError::new(ErrorKind::UnsupportedInput));
        }
        assert_eq!((r.width(), r.rows()), (d.width(), d.rows()));
        Ok(ProbeResult)
    }
}
#[test]
fn codec_bridge_requires_resolved_geometry_and_preserves_context() {
    use zencodec::{
        collection::Composition,
        image::{ImageGeometry, ImageInterpretation, ImageOutput, Rect},
    };
    use zenpixels::{Orientation, PixelBuffer};
    fn image(
        orientation: Orientation,
        composition: Option<Composition>,
        context: bool,
    ) -> ImageOutput {
        let mut interpretation = ImageInterpretation::packed(
            ImageGeometry {
                canvas_width: 1,
                canvas_height: 1,
                region: Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
            },
            orientation,
        );
        interpretation.composition = composition;
        let mut buffer =
            PixelBuffer::from_vec(vec![1, 2, 3], 1, 1, PixelDescriptor::RGB8_SRGB).unwrap();
        if context {
            buffer = buffer.with_color_context(Arc::new(ColorContext::from_icc(vec![1, 2, 3])));
        }
        ImageOutput::packed(buffer, vec![], interpretation).unwrap()
    }
    let good = image(Orientation::Identity, Some(Composition::Composited), false);
    let mut metric = Probe { calls: 0 };
    assert_eq!(
        zencodec_iqa::score_outputs(&mut metric, &good, &good, EvalOpts::default())
            .unwrap()
            .scores()
            .next()
            .unwrap()
            .value,
        1.0
    );
    for bad in [
        image(Orientation::Rotate90, Some(Composition::Composited), false),
        image(Orientation::Identity, None, false),
        image(
            Orientation::Identity,
            Some(Composition::Uncomposited),
            false,
        ),
    ] {
        assert_eq!(
            zencodec_iqa::score_outputs(&mut metric, &good, &bad, EvalOpts::default())
                .unwrap_err()
                .error()
                .kind(),
            ErrorKind::UnsupportedInput
        );
    }
    assert_eq!(metric.calls, 1);
    let contextual = image(Orientation::Identity, Some(Composition::Composited), true);
    assert_eq!(
        zencodec_iqa::score_outputs(&mut metric, &good, &contextual, EvalOpts::default())
            .unwrap_err()
            .error()
            .kind(),
        ErrorKind::UnsupportedInput
    );
    assert_eq!(metric.calls, 2);
}
