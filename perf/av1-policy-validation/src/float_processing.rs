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
    pub max_gamma_error: f64,
    pub max_gamma_roundtrip_error: f64,
    pub gamma_extended_samples: usize,
    pub gamma_input: Vec<[f32; 3]>,
    pub gamma_encoded: Vec<[f32; 3]>,
    pub gamma_restored: Vec<[f32; 3]>,
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
        &ConvertOptions::retain_channels().with_clip_out_of_gamut(clip),
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

// Qualify the existing Gamma22 model on actual decoded-and-linearized AV1
// samples. This does not publish a new color profile or claim literal gamma 2.2.
struct GammaTrace {
    max_error: f64,
    max_roundtrip: f64,
    extended: usize,
    encoded: Vec<[f32; 3]>,
    restored: Vec<[f32; 3]>,
}
fn qualify_gamma(samples: &[[f32; 3]], width: u32, rows: u32) -> GammaTrace {
    use zenpixels_convert::RowConverter;
    let linear = PixelDescriptor::RGBF32_LINEAR;
    let encoded = linear.with_transfer(zenpixels::TransferFunction::Gamma22);
    let options = ConvertOptions::retain_channels().with_clip_out_of_gamut(false);
    let mut encode = RowConverter::new_explicit(linear, encoded, &options).unwrap();
    let mut decode = RowConverter::new_explicit(encoded, linear, &options).unwrap();
    encode.prepare(width).unwrap();
    decode.prepare(width).unwrap();
    let mut gamma = vec![[0f32; 3]; samples.len()];
    let mut roundtrip = gamma.clone();
    let stride = width as usize * 12;
    encode
        .convert_rows(
            bytemuck::cast_slice(samples),
            stride,
            bytemuck::cast_slice_mut(&mut gamma),
            stride,
            width,
            rows,
        )
        .unwrap();
    decode
        .convert_rows(
            bytemuck::cast_slice(&gamma),
            stride,
            bytemuck::cast_slice_mut(&mut roundtrip),
            stride,
            width,
            rows,
        )
        .unwrap();
    let mut max_error = 0f64;
    let mut max_roundtrip = 0f64;
    let mut extended = 0;
    for ((source, output), restored) in samples
        .iter()
        .flatten()
        .zip(gamma.iter().flatten())
        .zip(roundtrip.iter().flatten())
    {
        let source = f64::from(*source);
        let expected = source.abs().powf(256.0 / 563.0).copysign(source);
        let error = (f64::from(*output) - expected).abs();
        let roundtrip_error = (f64::from(*restored) - source).abs();
        assert!(output.is_finite() && restored.is_finite());
        assert!(
            error <= 2e-5 * expected.abs().max(1.0),
            "gamma {source} -> {output}: {error}"
        );
        assert!(
            roundtrip_error <= 2e-5 * source.abs().max(1.0),
            "gamma roundtrip {source} -> {restored}: {roundtrip_error}"
        );
        if !(0.0..=1.0).contains(&source) {
            assert!(!(0.0..=1.0).contains(output));
            assert_eq!(source.is_sign_negative(), output.is_sign_negative());
            extended += 1;
        }
        max_error = max_error.max(error);
        max_roundtrip = max_roundtrip.max(roundtrip_error);
    }
    GammaTrace {
        max_error,
        max_roundtrip,
        extended,
        encoded: gamma,
        restored: roundtrip,
    }
}

pub(super) fn qualify(source: PixelSlice<'_>) -> Measurements {
    let linear = convert(source.clone(), true, false, source.rows()).active();
    assert!(ConvertOptions::retain_channels().clip_out_of_gamut);
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
    let gamma = qualify_gamma(&linear, source.width(), source.rows());
    let mut measurements = Measurements {
        max_linear_error: 0.0,
        max_roundtrip_error: 0.0,
        max_model_error: 0.0,
        max_gamma_error: gamma.max_error,
        max_gamma_roundtrip_error: gamma.max_roundtrip,
        gamma_extended_samples: gamma.extended,
        gamma_input: linear.clone(),
        gamma_encoded: gamma.encoded,
        gamma_restored: gamma.restored,
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
