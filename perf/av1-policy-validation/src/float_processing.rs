//! Exercise the actual output plan on unclipped native RGB, including row splits.
use std::sync::Arc;
use zencodec::{
    At,
    image::{ImageOutput, OutputError, PackedImageOwner},
    storage::{SharedStorage, StorageBudget},
};
use zenpixels::{Cicp, ColorOrigin, PixelDescriptor, PixelSlice, PixelSliceMut};
use zenpixels_convert::{ConvertOptions, OutputPlan, OutputProfile};

struct Processed {
    data: SharedStorage<[f32; 3]>,
    width: u32,
    rows: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    context: Arc<zenpixels::ColorContext>,
}
impl PackedImageOwner for Processed {
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
        Ok(PixelSlice::new(
            bytemuck::cast_slice(self.data.as_slice()),
            self.width,
            self.rows,
            self.stride * 12,
            self.descriptor,
        )
        .unwrap()
        .with_color_context(self.context.clone()))
    }
}
impl Processed {
    fn active(&self) -> Vec<[f32; 3]> {
        self.data
            .as_slice()
            .chunks_exact(self.stride)
            .flat_map(|row| row[..self.width as usize].iter().copied())
            .collect()
    }
}

const LINEAR: Cicp = Cicp::new(1, 8, 0, true);
const SENTINEL: u32 = 0x7fc1_2345;
// Test-specific normalized agreement, separate from exact batch consistency.
const ABS_LIMIT: f64 = 0.00001;

pub(super) struct Measurements {
    pub max_linear_error: f64,
    pub max_roundtrip_error: f64,
    pub max_model_error: f64,
    pub clipped_samples: usize,
    pub min_linear: f32,
    pub max_linear: f32,
}

fn plan(source: &PixelSlice<'_>, linear: bool, clip: bool) -> OutputPlan {
    let target = if linear { LINEAR } else { Cicp::SRGB };
    let descriptor = if linear {
        PixelDescriptor::RGBF32_LINEAR
    } else {
        PixelDescriptor::RGBF32_LINEAR.with_transfer(zenpixels::TransferFunction::Srgb)
    };
    OutputPlan::new_with_descriptor(
        source.descriptor(),
        source.color_context().cloned(),
        &ColorOrigin::from_cicp(Cicp::SRGB),
        OutputProfile::Named(target),
        descriptor,
        &ConvertOptions::forbid_lossy().with_clip_out_of_gamut(clip),
        None,
    )
    .unwrap()
}

fn convert(source: PixelSlice<'_>, linear: bool, clip: bool, batch: u32) -> Processed {
    let mut plan = plan(&source, linear, clip);
    plan.prepare(source.width()).unwrap();
    assert!(plan.borrow_rows(source.clone()).unwrap().is_none());
    let stride = source.width() as usize + 3;
    let count = stride * source.rows() as usize;
    let pool = StorageBudget::new(count * 12, 1);
    let mut output = pool.filled(count, [f32::from_bits(SENTINEL); 3]).unwrap();
    let mut context = None;
    for first in (0..source.rows()).step_by(batch as usize) {
        let rows = batch.min(source.rows() - first);
        let begin = first as usize * stride;
        let end = begin + rows as usize * stride;
        let destination = PixelSliceMut::new(
            bytemuck::cast_slice_mut(&mut output.as_mut_slice()[begin..end]),
            source.width(),
            rows,
            stride * 12,
            plan.output_descriptor(),
        )
        .unwrap();
        let converted = plan
            .convert_rows(source.sub_rows(first, rows), destination)
            .unwrap();
        context = converted.color_context().cloned();
        assert_eq!(converted.descriptor(), plan.output_descriptor());
        assert_eq!(
            converted.color_context().unwrap().cicp,
            plan.metadata().cicp
        );
        assert_eq!(
            plan.metadata().cicp,
            Some(if linear { LINEAR } else { Cicp::SRGB })
        );
    }
    for row in output.as_slice().chunks_exact(stride) {
        assert!(
            row[source.width() as usize..]
                .iter()
                .flatten()
                .all(|v| v.to_bits() == SENTINEL)
        );
    }
    let retained = output.freeze();
    assert_eq!(
        (pool.reserved_bytes(), pool.reserved_objects()),
        (count * 12, 1)
    );
    Processed {
        data: retained,
        width: source.width(),
        rows: source.rows(),
        stride,
        descriptor: plan.output_descriptor(),
        context: context.unwrap(),
    }
}

// Independent double-precision definition of reflected extended sRGB:
// https://www.w3.org/TR/css-color-4/#color-conversion-code (lin_sRGB).
fn reference_linear(encoded: f64) -> f64 {
    let magnitude = encoded.abs();
    if magnitude <= 0.04045 {
        encoded / 12.92
    } else {
        ((magnitude + 0.055) / 1.055).powf(2.4).copysign(encoded)
    }
}

// Independently evaluate the continuous curve selected by linear-srgb 0.6.12.
// This definition differs from the IEC constants above; report both rather
// than misclassifying the difference as polynomial error.
fn continuous_linear(encoded: f64) -> f64 {
    let magnitude = encoded.abs();
    if magnitude <= 12.92 * 0.003_041_282_560_127_521 {
        encoded / 12.92
    } else {
        ((magnitude + 0.055_010_718_947_586_6) / 1.055_010_718_947_586_6)
            .powf(2.4)
            .copysign(encoded)
    }
}

pub(super) fn qualify(source: PixelSlice<'_>) -> Measurements {
    let linear = convert(source.clone(), true, false, source.rows()).active();
    assert!(ConvertOptions::forbid_lossy().clip_out_of_gamut);
    let clipped = convert(source.clone(), true, true, source.rows()).active();
    assert!(clipped.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
    for batch in [1, 2, 4] {
        let split = convert(source.clone(), true, false, batch).active();
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&linear),
            bytemuck::cast_slice::<_, u8>(&split),
            "processing row partition must not alter samples"
        );
    }
    let linear_view = PixelSlice::new(
        bytemuck::cast_slice(&linear),
        source.width(),
        source.rows(),
        source.width() as usize * 12,
        PixelDescriptor::RGBF32_LINEAR,
    )
    .unwrap()
    .with_color_context(std::sync::Arc::new(zenpixels::ColorContext::from_cicp(
        LINEAR,
    )));
    let roundtrip = convert(linear_view, false, false, 2).active();
    let mut measurements = Measurements {
        max_linear_error: 0.0,
        max_roundtrip_error: 0.0,
        max_model_error: 0.0,
        clipped_samples: 0,
        min_linear: f32::INFINITY,
        max_linear: f32::NEG_INFINITY,
    };
    for y in 0..source.rows() {
        let encoded: &[[f32; 3]] = bytemuck::cast_slice(source.row(y));
        for (x, pixel) in encoded.iter().enumerate() {
            let index = y as usize * source.width() as usize + x;
            for (channel, encoded) in pixel.iter().copied().enumerate() {
                let actual = linear[index][channel];
                let expected = reference_linear(f64::from(encoded));
                let error = (f64::from(actual) - expected).abs();
                let model_error = (f64::from(actual) - continuous_linear(f64::from(encoded))).abs();
                let roundtrip_error = f64::from(roundtrip[index][channel] - encoded).abs();
                assert!(actual.is_finite());
                assert!(
                    model_error <= ABS_LIMIT,
                    "continuous model {encoded} -> {actual}: {model_error}"
                );
                assert!(
                    roundtrip_error <= ABS_LIMIT,
                    "roundtrip {encoded}: {roundtrip_error}"
                );
                if !(0.0..=1.0).contains(&encoded) {
                    assert!(!(0.0..=1.0).contains(&actual));
                    assert_ne!(actual.to_bits(), clipped[index][channel].to_bits());
                    measurements.clipped_samples += 1;
                }
                measurements.max_model_error = measurements.max_model_error.max(model_error);
                measurements.max_linear_error = measurements.max_linear_error.max(error);
                measurements.max_roundtrip_error =
                    measurements.max_roundtrip_error.max(roundtrip_error);
                measurements.min_linear = measurements.min_linear.min(actual);
                measurements.max_linear = measurements.max_linear.max(actual);
            }
        }
    }
    measurements
}

/// Adopt the actual strided processed owner, retaining source facts and timing.
pub(super) fn qualify_output(source: &ImageOutput) -> Measurements {
    let measurements = source.with_pixels(qualify).unwrap();
    let processed = source.with_pixels(|p| convert(p, true, false, 2)).unwrap();
    let descriptor = processed.descriptor;
    let address = processed.data.as_slice().as_ptr();
    let result = source.clone().replace_packed_color(processed).unwrap();
    assert_eq!(
        result.interpretation().geometry,
        source.interpretation().geometry
    );
    assert_eq!(
        result.interpretation().timing,
        source.interpretation().timing
    );
    assert_eq!(
        result.interpretation().presentation,
        source.interpretation().presentation
    );
    assert!(Arc::ptr_eq(
        result.source_frame().unwrap(),
        source.source_frame().unwrap()
    ));
    result
        .with_pixels(|p| {
            assert_eq!(p.row(0).as_ptr(), address.cast::<u8>());
            assert_eq!(p.descriptor(), descriptor);
            assert_eq!(p.color_context().unwrap().cicp, Some(LINEAR));
        })
        .unwrap();
    source
        .with_pixels(|p| assert_eq!(p.color_context().unwrap().cicp, Some(Cicp::SRGB)))
        .unwrap();
    measurements
}
