//! Resolved-output admission and actual pixel context controls.
use std::sync::Arc;
use zencodec_iqa::iqa::zenpixels::{self, ColorContext, PixelDescriptor};
use zencodec_iqa::iqa::*;
struct Probe {
    calls: usize,
}
impl Metric for Probe {
    fn id(&self) -> MetricId {
        MetricId::new("probe", Implementation::new("adapter-control", "1", 1))
    }
    fn outputs(&self) -> std::borrow::Cow<'static, [ScoreSpec]> {
        std::borrow::Cow::Owned(vec![ScoreSpec::new("score", ScoreKind::Feature)])
    }
    fn accepts(&self, f: &InputFormat) -> bool {
        is_rgb8_srgb(f)
    }
    fn evaluate(
        &mut self,
        r: Image<'_>,
        d: Image<'_>,
        _: EvalOpts<'_>,
    ) -> MetricResult<Box<dyn Evaluation>> {
        self.calls += 1;
        let (r, d) = rgb8_pair(&r, &d)?;
        assert_eq!((r.width, r.height), (d.width, d.height));
        Ok(Box::new(ScalarEval::single("score", 1.0)))
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
            .result
            .score(),
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
