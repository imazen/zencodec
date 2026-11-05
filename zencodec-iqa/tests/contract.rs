//! Protocol probes, not implementations or qualification of an IQA algorithm.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec_iqa::*;
use zenpixels::{ColorContext, PixelDescriptor, PixelSlice};

fn descriptor() -> Descriptor {
    Descriptor {
        id: MetricId {
            method: "protocol-probe/profile".into(),
            reference: "synthetic contract control".into(),
            implementation: "test/1".into(),
            score_revision: 1,
            backend: "cpu".into(),
            configuration: "preparation=explicit;model=none".into(),
        },
        output: ScoreSpec {
            key: "probe".into(),
            direction: Direction::Lower,
            unit: "native".into(),
            nonfinite: NonFinite::Refuse,
        },
    }
}
struct Probe {
    descriptor: Descriptor,
    calls: usize,
    output: f64,
}
impl Probe {
    fn new(output: f64) -> Self {
        Self {
            descriptor: descriptor(),
            calls: 0,
            output,
        }
    }
}
fn sum(image: &PixelSlice<'_>) -> f64 {
    (0..image.rows())
        .flat_map(|y| image.row(y))
        .map(|v| f64::from(*v))
        .sum()
}
impl Metric for Probe {
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn evaluate(
        &mut self,
        reference: PixelSlice<'_>,
        candidate: PixelSlice<'_>,
        _: EvalOpts<'_>,
    ) -> Result<f64> {
        self.calls += 1;
        if reference.descriptor() != PixelDescriptor::RGB8_SRGB
            || candidate.descriptor() != PixelDescriptor::RGB8_SRGB
            || reference.color_context().is_some()
            || candidate.color_context().is_some()
        {
            return Err(whereat::At::wrap(Error::new(ErrorKind::UnsupportedInput)));
        }
        assert_eq!(sum(&reference), sum(&candidate));
        Ok(self.output)
    }
    fn prepare_reference(
        &mut self,
        reference: PixelSlice<'_>,
        _: EvalOpts<'_>,
    ) -> Result<Box<dyn WarmReference + '_>> {
        struct Warm {
            reference_sum: f64,
            output: f64,
        }
        impl WarmReference for Warm {
            fn evaluate(&mut self, candidate: PixelSlice<'_>, _: EvalOpts<'_>) -> Result<f64> {
                if candidate.color_context().is_some()
                    || candidate.descriptor() != PixelDescriptor::RGB8_SRGB
                {
                    return Err(whereat::At::wrap(Error::new(ErrorKind::UnsupportedInput)));
                }
                assert_eq!(self.reference_sum, sum(&candidate));
                Ok(self.output)
            }
        }
        self.calls += 1;
        Ok(Box::new(Warm {
            reference_sum: sum(&reference),
            output: self.output,
        }))
    }
}
fn pixels(data: &[u8], width: u32, rows: u32, stride: usize) -> PixelSlice<'_> {
    PixelSlice::new(data, width, rows, stride, PixelDescriptor::RGB8_SRGB).unwrap()
}

#[test]
fn strided_minimum_extent_and_warm_results_preserve_identity() {
    let tight = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    let padded = [1, 2, 3, 4, 5, 6, 255, 255, 255, 7, 8, 9, 10, 11, 12];
    let mut metric = Probe::new(3.25);
    let cold = score(
        &mut metric,
        pixels(&tight, 2, 2, 6),
        pixels(&padded, 2, 2, 9),
        EvalOpts::default(),
    )
    .unwrap();
    assert_eq!(cold.value.to_bits(), 3.25f64.to_bits());
    assert_eq!(cold.descriptor, descriptor());
    let mut warm =
        prepare_reference(&mut metric, pixels(&padded, 2, 2, 9), EvalOpts::default()).unwrap();
    let result = warm
        .score(pixels(&tight, 2, 2, 6), EvalOpts::default())
        .unwrap();
    assert_eq!(result, cold);
    assert_eq!(
        warm.score(pixels(&tight[..6], 2, 1, 6), EvalOpts::default())
            .unwrap_err()
            .error()
            .kind,
        ErrorKind::DimensionMismatch
    );
    assert_eq!(
        warm.score(pixels(&tight, 2, 2, 6), EvalOpts::default())
            .unwrap(),
        cold
    );
}

#[test]
fn dimension_and_pixel_limits_refuse_before_backend() {
    let data = [0; 12];
    let mut metric = Probe::new(0.0);
    let error = score(
        &mut metric,
        pixels(&data, 2, 2, 6),
        pixels(&data[..6], 2, 1, 6),
        EvalOpts::default(),
    )
    .unwrap_err();
    assert_eq!(error.error().kind, ErrorKind::DimensionMismatch);
    let opts = EvalOpts {
        max_pixels: Some(3),
        ..EvalOpts::default()
    };
    assert_eq!(
        score(
            &mut metric,
            pixels(&data, 2, 2, 6),
            pixels(&data, 2, 2, 6),
            opts
        )
        .unwrap_err()
        .error()
        .kind,
        ErrorKind::Resource
    );
    assert!(prepare_reference(&mut metric, pixels(&data, 2, 2, 6), opts).is_err());
    assert_eq!(metric.calls, 0);
}

#[test]
fn actual_color_context_reaches_the_scorer() {
    let data = [0; 3];
    let mut metric = Probe::new(0.0);
    let contextual =
        pixels(&data, 1, 1, 3).with_color_context(Arc::new(ColorContext::from_icc(vec![1, 2, 3])));
    let error = score(
        &mut metric,
        pixels(&data, 1, 1, 3),
        contextual,
        EvalOpts::default(),
    )
    .unwrap_err();
    assert_eq!(error.error().kind, ErrorKind::UnsupportedInput);
    assert_eq!(metric.calls, 1);
}

#[test]
fn successful_nonfinite_policy_is_explicit() {
    let data = [0; 3];
    for output in [f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
        let mut metric = Probe::new(output);
        assert_eq!(
            score(
                &mut metric,
                pixels(&data, 1, 1, 3),
                pixels(&data, 1, 1, 3),
                EvalOpts::default()
            )
            .unwrap_err()
            .error()
            .kind,
            ErrorKind::Backend
        );
        metric.descriptor.output.nonfinite = NonFinite::PositiveInfinity;
        let result = score(
            &mut metric,
            pixels(&data, 1, 1, 3),
            pixels(&data, 1, 1, 3),
            EvalOpts::default(),
        );
        assert_eq!(result.is_ok(), output == f64::INFINITY);
    }
}

struct StopAfter(AtomicUsize);
impl enough::Stop for StopAfter {
    fn check(&self) -> core::result::Result<(), enough::StopReason> {
        let previous = self.0.load(Ordering::Relaxed);
        self.0.store(previous.saturating_sub(1), Ordering::Relaxed);
        if previous == 0 {
            Err(enough::StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}
#[test]
fn cancellation_before_and_after_work_preserves_reason() {
    let data = [0; 3];
    for (checks, expected_calls) in [(0, 0), (2, 1)] {
        let control = StopAfter(AtomicUsize::new(checks));
        let stop = how_far::StopOnly::borrowed(&control);
        let mut metric = Probe::new(0.0);
        let opts = EvalOpts {
            pulse: &stop,
            max_pixels: None,
        };
        let error = score(
            &mut metric,
            pixels(&data, 1, 1, 3),
            pixels(&data, 1, 1, 3),
            opts,
        )
        .unwrap_err();
        assert_eq!(
            error.error().kind,
            ErrorKind::Stopped(enough::StopReason::TimedOut)
        );
        assert_eq!(metric.calls, expected_calls);
    }
}

#[test]
fn no_reference_uses_one_image_and_default_warm_refuses() {
    struct Single {
        descriptor: Descriptor,
        calls: usize,
    }
    impl NoReferenceMetric for Single {
        fn descriptor(&self) -> &Descriptor {
            &self.descriptor
        }
        fn evaluate(&mut self, image: PixelSlice<'_>, _: EvalOpts<'_>) -> Result<f64> {
            self.calls += 1;
            Ok(sum(&image))
        }
    }
    impl Metric for Single {
        fn descriptor(&self) -> &Descriptor {
            &self.descriptor
        }
        fn evaluate(
            &mut self,
            _: PixelSlice<'_>,
            _: PixelSlice<'_>,
            _: EvalOpts<'_>,
        ) -> Result<f64> {
            unreachable!()
        }
    }
    let mut metric = Single {
        descriptor: descriptor(),
        calls: 0,
    };
    let data = [1, 2, 3];
    assert_eq!(
        score_no_reference(&mut metric, pixels(&data, 1, 1, 3), EvalOpts::default())
            .unwrap()
            .value,
        6.0
    );
    assert_eq!(metric.calls, 1);
    assert!(
        matches!(prepare_reference(&mut metric, pixels(&data, 1, 1, 3), EvalOpts::default()), Err(error) if error.error().kind == ErrorKind::UnsupportedOperation)
    );
}

#[cfg(feature = "codec")]
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
    let mut metric = Probe::new(1.0);
    assert_eq!(
        codec::score_outputs(&mut metric, &good, &good, EvalOpts::default())
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
            codec::score_outputs(&mut metric, &good, &bad, EvalOpts::default())
                .unwrap_err()
                .error()
                .kind,
            ErrorKind::UnsupportedInput
        );
    }
    assert_eq!(metric.calls, 1);
    let contextual = image(Orientation::Identity, Some(Composition::Composited), true);
    assert_eq!(
        codec::score_outputs(&mut metric, &good, &contextual, EvalOpts::default())
            .unwrap_err()
            .error()
            .kind,
        ErrorKind::UnsupportedInput
    );
    assert_eq!(metric.calls, 2);
}
