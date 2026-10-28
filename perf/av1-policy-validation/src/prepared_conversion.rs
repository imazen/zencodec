//! Executing witness: ask policy before materializing destination/scratch.
//! Integer strip kernels remain test-only; float uses the shared pixel converter.
use crate::planar_conversion_tests::{Refusal, SitingChoice, conversion_facts, widen};
use enough::Stop;
use std::{error::Error, fmt, sync::Arc};
use zenavif::yuv_convert::*;
use zencodec::{
    At, CodecError,
    frame::{ChromaSiting, Frame, MappedPlanes, PlaneSamples, PlaneView, Subsampling},
    image::{ImageGeometry, ImageOutput, OutputError, PackedImageOwner, Rect},
    operation::{
        ImageConversionOffer, OperationReport, Outcome, OperationPolicy, PolicySet,
        resolve_image_conversion,
    },
    storage::{PreparedStorage, SharedStorage, StorageBudget, StorageError},
};
use zenpixels::{Cicp, ColorContext, Orientation, PixelDescriptor, PixelSlice};
use zenpixels::{
    PixelSliceMut,
    sample::{ChromaSiting as PixelSiting, ChromaSubsampling, CodePlane},
};
use zenpixels_convert::ycbcr::YcbcrToRgbF32;

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported native planar conversion: {self:?}")
    }
}
impl Error for Refusal {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Prepare,
    Policy,
    Allocate,
    Execute,
    Adopt,
}

struct Failure {
    stage: Stage,
    report: Box<OperationReport>,
    cause: Box<dyn Error + Send + Sync>,
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConversionFailure")
            .field("stage", &self.stage)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "image conversion failed at {:?}", self.stage)
    }
}
impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
#[track_caller]
fn failed(
    stage: Stage,
    report: &OperationReport,
    cause: impl Error + Send + Sync + 'static,
) -> At<Failure> {
    At::wrap(Failure {
        stage,
        report: Box::new(report.clone()),
        cause: Box::new(cause),
    })
    .at()
}
fn check(stop: &dyn zencodec::Pulse, stage: Stage, report: &OperationReport) -> Result<(), At<Failure>> {
    stop.check().map_err(|reason| {
        failed(
            stage,
            report,
            At::wrap(CodecError::new(None, reason.into())),
        )
    })
}

struct Owner<T> {
    data: SharedStorage<[T; 3]>,
    descriptor: PixelDescriptor,
    width: u32,
    height: u32,
    stride: usize,
    color: Arc<ColorContext>,
}
impl<T: bytemuck::Pod + Send + Sync> PackedImageOwner for Owner<T> {
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
        PixelSlice::new(
            bytemuck::cast_slice(self.data.as_slice()),
            self.width,
            self.height,
            self.stride * core::mem::size_of::<[T; 3]>(),
            self.descriptor,
        )
        .map(|p| p.with_color_context(self.color.clone()))
        .map_err(|e| {
            At::wrap(OutputError::Mapping(zencodec::frame::MapError::Backend(
                Box::new(e),
            )))
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Rgb16,
    RgbF32,
}
const FLOAT_SRGB: PixelDescriptor =
    PixelDescriptor::RGBF32_LINEAR.with_transfer(zenpixels::TransferFunction::Srgb);
// Keep the retained conversion plan inline: boxing would add an allocation
// outside the prepared-storage admission exercised by this harness.
#[expect(clippy::large_enum_variant, reason = "retained plans stay allocation-free")]
enum OutputStorage<'a> {
    Integer {
        destination: PreparedStorage<[u16; 3]>,
        scratch: PreparedStorage<u64>,
        scratch_bytes: usize,
    },
    Float {
        destination: PreparedStorage<[f32; 3]>,
        plan: YcbcrToRgbF32<'a>,
    },
}

/// Holds the exact mapping, destination reservation and kernel selection.
/// No destination payload exists during policy; allocation can still fail.
struct Prepared<'a> {
    frame: &'a Frame,
    planes: [PlaneView<'a>; 3],
    output: OutputStorage<'a>,
    stride: usize,
    offer: ImageConversionOffer,
}
impl<'a> Prepared<'a> {
    fn new(
        frame: &'a Frame,
        mapped: &'a dyn MappedPlanes,
        pool: &StorageBudget,
        target: Target,
        stop: &dyn zencodec::Pulse,
    ) -> Result<Self, At<Failure>> {
        let report = OperationReport::default();
        let fail = |e| failed(Stage::Prepare, &report, e);
        check(stop, Stage::Prepare, &report)?;
        let facts = conversion_facts(frame, SitingChoice::AssumeCenterIfUnknown).map_err(fail)?;
        let info = frame.interpretation();
        let geometry = info.geometry;
        let crop = geometry.crop;
        let chroma = info
            .subsampling
            .chroma_size(geometry.coded_width, geometry.coded_height)
            .ok_or_else(|| fail(Refusal::Interpretation))?;
        if mapped.plane_count() != 3 {
            return Err(fail(Refusal::Mapping));
        }
        let mut planes = [None; 3];
        for (i, dimensions) in [
            (geometry.coded_width, geometry.coded_height),
            chroma,
            chroma,
        ]
        .into_iter()
        .enumerate()
        {
            let plane = mapped.plane(i).ok_or_else(|| fail(Refusal::Mapping))?;
            if (plane.width(), plane.height()) != (dimensions.0 as usize, dimensions.1 as usize)
                || plane.encoding() != info.encoding
            {
                return Err(fail(Refusal::Mapping));
            }
            let max = ((1u32 << info.encoding.code_bits()) - 1) as u16;
            for row in 0..plane.height() {
                check(stop, Stage::Prepare, &report)?;
                let valid = match plane.row(row).ok_or_else(|| fail(Refusal::Mapping))? {
                    PlaneSamples::U8(r) => r.iter().all(|v| u16::from(*v) <= max),
                    PlaneSamples::U16(r) => r.iter().all(|v| *v <= max),
                };
                if !valid {
                    return Err(fail(Refusal::Sample));
                }
            }
            planes[i] = Some(plane);
        }
        let planes = planes.map(Option::unwrap);
        let overflow = || failed(Stage::Prepare, &report, StorageError::SizeOverflow);
        let stride = (crop.width as usize).checked_add(5).ok_or_else(overflow)?;
        let count = stride
            .checked_mul(crop.height as usize)
            .ok_or_else(overflow)?;
        let (output, descriptor, admitted_bytes) = match target {
            Target::Rgb16 => {
                let scratch_bytes = (geometry.coded_width as usize)
                    .checked_mul(3 * info.encoding.bytes_per_sample())
                    .ok_or_else(overflow)?;
                let destination = pool
                    .prepare_filled(count, [0xdead; 3])
                    .map_err(|e| failed(Stage::Prepare, &report, e))?;
                let scratch = pool
                    .prepare_filled(scratch_bytes.div_ceil(8), 0u64)
                    .map_err(|e| failed(Stage::Prepare, &report, e))?;
                let bytes = destination
                    .reserved_bytes()
                    .checked_add(scratch.reserved_bytes())
                    .ok_or_else(overflow)?;
                (
                    OutputStorage::Integer {
                        destination,
                        scratch,
                        scratch_bytes,
                    },
                    PixelDescriptor::RGB16_SRGB,
                    bytes,
                )
            }
            Target::RgbF32 => {
                let destination = pool
                    .prepare_filled(count, [f32::from_bits(0x7fc1_2345); 3])
                    .map_err(|e| failed(Stage::Prepare, &report, e))?;
                let bytes = destination.reserved_bytes();
                let encoding = zenpixels::sample::SampleEncoding::new(
                    info.encoding.storage(),
                    info.encoding.code_bits(),
                    info.encoding.bit_shift(),
                )
                .map_err(|e| failed(Stage::Prepare, &report, e))?;
                let view = |p: PlaneView<'a>| {
                    match p.as_strided_samples() {
                        PlaneSamples::U8(s) => {
                            CodePlane::from_u8(s, p.width(), p.height(), p.stride_bytes(), encoding)
                        }
                        PlaneSamples::U16(s) => CodePlane::from_u16(
                            s,
                            p.width(),
                            p.height(),
                            p.stride_bytes(),
                            encoding,
                        ),
                    }
                    .map_err(|e| failed(Stage::Prepare, &report, e))
                };
                let sub = match info.subsampling {
                    Subsampling::Yuv444 => ChromaSubsampling::Yuv444,
                    Subsampling::Yuv422 => ChromaSubsampling::Yuv422,
                    Subsampling::Yuv420 => ChromaSubsampling::Yuv420,
                    _ => return Err(failed(Stage::Prepare, &report, Refusal::Interpretation)),
                };
                // The selected fallback remains visible in the policy offer.
                // The shared converter only receives the resolved concrete fact.
                let plan = YcbcrToRgbF32::new(
                    [view(planes[0])?, view(planes[1])?, view(planes[2])?],
                    sub,
                    PixelSiting::Center,
                    info.color.resolved.unwrap(),
                )
                .map_err(|e| failed(Stage::Prepare, &report, e))?;
                (
                    OutputStorage::Float { destination, plan },
                    FLOAT_SRGB,
                    bytes,
                )
            }
        };
        let mut offer = ImageConversionOffer::new(
            descriptor,
            ImageGeometry {
                canvas_width: geometry.coded_width,
                canvas_height: geometry.coded_height,
                region: Rect {
                    x: crop.x,
                    y: crop.y,
                    width: crop.width,
                    height: crop.height,
                },
            },
            Orientation::Identity,
        );
        offer.assumed_chroma_siting = facts.assumed_center.then_some(ChromaSiting::Center);
        // The float recipe has no saturation; it leaves transfer/primaries
        // unchanged. It does not establish HDR rendering or exact precision.
        offer.preserves_extended_range = target == Target::RgbF32;
        offer.admitted_bytes = admitted_bytes;
        Ok(Self {
            frame,
            planes,
            output,
            stride,
            offer,
        })
    }

    fn run<R: OperationPolicy + ?Sized>(
        self,
        resolver: &mut R,
        preset: PolicySet,
        stop: &dyn zencodec::Pulse,
    ) -> Result<Outcome<ImageOutput>, At<Failure>> {
        let mut report = OperationReport::default();
        let source = ImageOutput::from_frame(self.frame.clone());
        match resolve_image_conversion(&source, &[self.offer], resolver, preset, stop) {
            Ok((index, record)) => {
                assert_eq!(index, 0);
                report.record(record);
            }
            Err(error) => {
                report.record(error.error().record());
                return Err(failed(Stage::Policy, &report, error));
            }
        }
        drop(source);
        check(stop, Stage::Allocate, &report)?;
        let image = match self.output {
            OutputStorage::Integer {
                destination,
                scratch,
                scratch_bytes,
            } => {
                let mut data = destination
                    .allocate()
                    .map_err(|e| failed(Stage::Allocate, &report, e))?;
                let mut scratch = scratch
                    .allocate()
                    .map_err(|e| failed(Stage::Allocate, &report, e))?;
                let scratch_bytes =
                    &mut bytemuck::cast_slice_mut::<u64, u8>(scratch.as_mut_slice())
                        [..scratch_bytes];
                let info = self.frame.interpretation();
                let crop = info.geometry.crop;
                let width = info.geometry.coded_width as usize;
                let height = info.geometry.coded_height as usize;
                let bits = info.encoding.code_bits();
                let range = if info.color.resolved.unwrap().full_range {
                    YuvRange::Full
                } else {
                    YuvRange::Limited
                };
                macro_rules! execute {
            ($kind:ident, $f420:ident, $f422:ident, $f444:ident $(, $depth:expr)?) => {{
                let [y, u, v] = self.planes.each_ref().map(|p| p.as_strided_samples());
                let (PlaneSamples::$kind(y), PlaneSamples::$kind(u), PlaneSamples::$kind(v)) = (y, u, v)
                    else { return Err(failed(Stage::Execute, &report, Refusal::Mapping)); };
                let [ys, us, vs] = self.planes.each_ref().map(|p| p.stride_bytes() / info.encoding.bytes_per_sample());
                let scratch = bytemuck::cast_slice_mut(scratch_bytes);
                for row in 0..crop.height as usize {
                    check(stop, Stage::Execute, &report)?;
                    let start = row + crop.y as usize;
                    match info.subsampling {
                        Subsampling::Yuv420 => $f420(y, ys, u, us, v, vs, width, height, start, 1, range, YuvMatrix::Bt709, $($depth,)? scratch),
                        Subsampling::Yuv422 => $f422(y, ys, u, us, v, vs, width, start, 1, range, YuvMatrix::Bt709, $($depth,)? scratch),
                        Subsampling::Yuv444 => $f444(y, ys, u, us, v, vs, width, start, 1, range, YuvMatrix::Bt709, $($depth,)? scratch),
                        _ => unreachable!(),
                    }
                    for (out, pixel) in data.as_mut_slice()[row * self.stride..][..crop.width as usize].iter_mut()
                        .zip(&scratch[crop.x as usize..][..crop.width as usize]) {
                        *out = [widen(pixel.r, bits), widen(pixel.g, bits), widen(pixel.b, bits)];
                    }
                }
            }};
        }
                if bits == 8 {
                    execute!(
                        U8,
                        yuv420_to_rgb8_strip,
                        yuv422_to_rgb8_strip,
                        yuv444_to_rgb8_strip
                    );
                } else {
                    execute!(
                        U16,
                        yuv420_to_rgb16_strip,
                        yuv422_to_rgb16_strip,
                        yuv444_to_rgb16_strip,
                        bits
                    );
                }
                check(stop, Stage::Adopt, &report)?;
                let owner = Owner {
                    data: data.freeze(),
                    descriptor: PixelDescriptor::RGB16_SRGB,
                    width: crop.width,
                    height: crop.height,
                    stride: self.stride,
                    color: Arc::new(ColorContext::from_cicp(Cicp::SRGB)),
                };
                ImageOutput::from_frame_pixels(self.frame.clone(), owner, self.offer.orientation)
                    .map_err(|e| failed(Stage::Adopt, &report, e))?
            }
            OutputStorage::Float { destination, plan } => {
                let mut data = destination
                    .allocate()
                    .map_err(|e| failed(Stage::Allocate, &report, e))?;
                let crop = self.frame.interpretation().geometry.crop;
                let mut color = None;
                for row in 0..crop.height as usize {
                    check(stop, Stage::Execute, &report)?;
                    let dst = PixelSliceMut::new(
                        bytemuck::cast_slice_mut(
                            &mut data.as_mut_slice()[row * self.stride..][..self.stride],
                        ),
                        crop.width,
                        1,
                        self.stride * 12,
                        plan.output_descriptor(),
                    )
                    .map_err(|e| failed(Stage::Execute, &report, e))?;
                    let converted = plan
                        .convert_rows(crop.x as usize, crop.y as usize + row, dst)
                        .map_err(|e| failed(Stage::Execute, &report, e))?;
                    color = converted.color_context().cloned();
                }
                check(stop, Stage::Adopt, &report)?;
                let owner = Owner {
                    data: data.freeze(),
                    descriptor: plan.output_descriptor(),
                    width: crop.width,
                    height: crop.height,
                    stride: self.stride,
                    color: color.unwrap(),
                };
                ImageOutput::from_frame_pixels(self.frame.clone(), owner, self.offer.orientation)
                    .map_err(|e| failed(Stage::Adopt, &report, e))?
            }
        };
        Ok(Outcome::from_parts(image, report))
    }
}

fn convert<R: OperationPolicy + ?Sized>(
    frame: Frame,
    pool: &StorageBudget,
    resolver: &mut R,
    preset: PolicySet,
    stop: &dyn zencodec::Pulse,
) -> Result<Outcome<ImageOutput>, At<Failure>> {
    convert_target(frame, pool, resolver, preset, Target::Rgb16, stop)
}

fn convert_target<R: OperationPolicy + ?Sized>(
    frame: Frame,
    pool: &StorageBudget,
    resolver: &mut R,
    preset: PolicySet,
    target: Target,
    stop: &dyn zencodec::Pulse,
) -> Result<Outcome<ImageOutput>, At<Failure>> {
    check(stop, Stage::Prepare, &OperationReport::default())?;
    let map = frame
        .map()
        .map_err(|e| failed(Stage::Prepare, &OperationReport::default(), e))?;
    Prepared::new(&frame, map.as_ref(), pool, target, stop)?.run(resolver, preset, stop)
}

/// Called for every native matrix case, comparing actual selected output to
/// whole-picture conversion after native sample parity with the independent decoder.
pub(super) fn qualify(native: &Frame, expected: &[[u16; 3]]) {
    use std::{cell::Cell, convert::Infallible};
    use zencodec::operation::{Decision, Question, Selection};
    let crop = native.interpretation().geometry.crop;
    let bytes = (crop.width as usize + 5) * crop.height as usize * 6;
    let pool = StorageBudget::new(
        bytes + native.interpretation().geometry.coded_width as usize * 6 + 7,
        2,
    );
    let calls = Cell::new(0);
    let result = convert(
        native.clone(),
        &pool,
        &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::ImageConversion {
                image,
                facts,
                choices,
            } = q
            else {
                panic!()
            };
            assert!(Arc::ptr_eq(
                image.interpretation().signal.as_ref().unwrap(),
                native.interpretation()
            ));
            assert_eq!(
                facts.frame.unwrap().encoding,
                native.interpretation().encoding
            );
            let offer = choices.first().unwrap().value();
            assert_eq!(offer.admitted_bytes, pool.reserved_bytes());
            assert_eq!(pool.reserved_objects(), 2);
            assert!(!offer.preserves_extended_range);
            assert_eq!(offer.workspace_upper_bound, None);
            calls.set(calls.get() + 1);
            Ok(Decision::UsePreset)
        },
        PolicySet::new().allow_assumed_chroma_siting(true),
        &zencodec::how_far::NoPulse,
    )
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert!(matches!(
        result.report().decisions().next().unwrap().selected,
        Some(Selection::ImageConversion(_))
    ));
    assert_eq!(result.report().decisions().count(), 1);
    assert_eq!(pool.reserved_bytes(), bytes);
    assert_eq!(pool.reserved_objects(), 1);
    assert!(Arc::ptr_eq(
        result.source_frame().unwrap(),
        native.interpretation()
    ));
    result
        .with_pixels(|p| {
            assert_eq!(p.descriptor(), PixelDescriptor::RGB16_SRGB);
            for row in 0..crop.height as usize {
                assert_eq!(
                    p.row(row as u32),
                    bytemuck::cast_slice(
                        &expected[(row + crop.y as usize)
                            * native.interpretation().geometry.coded_width as usize
                            + crop.x as usize..][..crop.width as usize]
                    )
                );
            }
        })
        .unwrap();
    let retained = result.output().clone();
    drop(result);
    assert_eq!(pool.reserved_bytes(), bytes);
    drop(retained);
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
}

/// Actual unclipped float output through the same retained-route callback.
pub(super) fn float_pixels(native: &Frame) -> Vec<[f32; 3]> {
    float_pixels_with_processing(native).0
}

pub(super) fn float_pixels_with_processing(
    native: &Frame,
) -> (Vec<[f32; 3]>, crate::float_processing::Measurements) {
    use std::{cell::Cell, convert::Infallible};
    use zencodec::operation::{Decision, Question, Selection};
    let crop = native.interpretation().geometry.crop;
    let bytes = (crop.width as usize + 5) * crop.height as usize * 12;
    let pool = StorageBudget::new(bytes, 1);
    let calls = Cell::new(0);
    let result = convert_target(
        native.clone(),
        &pool,
        &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::ImageConversion { choices, .. } = q else {
                panic!()
            };
            let offer = choices.first().unwrap().value();
            assert_eq!(offer.output, FLOAT_SRGB);
            assert!(offer.preserves_extended_range);
            assert_eq!(offer.admitted_bytes, bytes);
            assert_eq!(pool.reserved_bytes(), bytes);
            assert_eq!(pool.reserved_objects(), 1);
            calls.set(calls.get() + 1);
            Ok(Decision::UsePreset)
        },
        PolicySet::new()
            .allow_assumed_chroma_siting(true)
            .require_extended_range(true),
        Target::RgbF32,
        &zencodec::how_far::NoPulse,
    )
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(result.report().decisions().count(), 1);
    let Some(Selection::ImageConversion(offer)) =
        result.report().decisions().next().unwrap().selected
    else {
        panic!()
    };
    assert!(offer.preserves_extended_range);
    assert!(Arc::ptr_eq(
        result.source_frame().unwrap(),
        native.interpretation()
    ));
    assert_eq!(result.interpretation().timing, Some(native.timing()));
    let processing = crate::float_processing::qualify_output(result.output());
    let output = result
        .with_pixels(|p| {
            assert_eq!(p.descriptor(), FLOAT_SRGB);
            assert_eq!(p.color_context().unwrap().cicp, Some(Cicp::SRGB));
            let samples = (0..crop.height)
                .flat_map(|row| {
                    bytemuck::cast_slice::<u8, [f32; 3]>(p.row(row))
                        .iter()
                        .copied()
                })
                .collect::<Vec<_>>();
            (samples, processing)
        })
        .unwrap();
    let retained = result.output().clone();
    drop(result);
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (bytes, 1));
    drop(retained);
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    output
}

pub(super) fn qualify_float(native: &Frame, expected: &[[f32; 3]]) {
    let actual = float_pixels(native);
    let crop = native.interpretation().geometry.crop;
    for y in 0..crop.height as usize {
        let wanted = &expected[(y + crop.y as usize)
            * native.interpretation().geometry.coded_width as usize
            + crop.x as usize..][..crop.width as usize];
        let actual = &actual[y * crop.width as usize..][..crop.width as usize];
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(actual),
            bytemuck::cast_slice::<_, u8>(wanted),
            "float crop/row partition must be bit-identical to the full frame"
        );
    }
}

pub(super) fn reject_declared_siting(native: &Frame) {
    for target in [Target::Rgb16, Target::RgbF32] {
        let pool = StorageBudget::new(32_768, 2);
        let error = convert_target(
            native.clone(),
            &pool,
            &mut |_: zencodec::operation::Question<'_>| -> Result<_, At<std::convert::Infallible>> {
                panic!("unsupported signaled siting must not reach policy")
            },
            PolicySet::new().allow_assumed_chroma_siting(true),
            target,
            &zencodec::how_far::NoPulse,
        )
        .unwrap_err();
        assert_eq!(error.error().stage, Stage::Prepare);
        assert_eq!(
            zencodec::find_cause::<Refusal>(&error),
            Some(&Refusal::Siting)
        );
        assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use enough::{StopReason, };
use zencodec::how_far::NoPulse;
    use std::{
        cell::Cell,
        convert::Infallible,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use zencodec::operation::{ChoiceId, Decision, DecisionCode, MetadataTransfer, Question};
    use zencodec_media::session::VideoDecoder;

    fn native() -> Frame {
        let settings = fixtures::settings(10, Subsampling::Yuv420, false)
            .with_color(Cicp::new(1, 13, 1, false));
        let input = fixtures::frame(&settings, 0);
        let (config, packets) = fixtures::encode(settings, &[input]);
        let mut settings = rav1d_safe::Settings::default();
        settings.threads = 1;
        let mut decoder = zenavif::media::Av1VideoDecoder::with_settings(settings).unwrap();
        let mut frames = fixtures::decode(&mut decoder, &config, &packets, &NoPulse).unwrap();
        decoder.reset().unwrap();
        frames.remove(0)
    }
    fn preset(_: Question<'_>) -> Result<Decision, At<Infallible>> {
        Ok(Decision::UsePreset)
    }

    #[test]
    fn planar_prepared_policy_refuses_unproven_requirements_and_releases_reservations() {
        let frame = native();
        for policy in [
            PolicySet::new(),
            PolicySet::new()
                .allow_assumed_chroma_siting(true)
                .require_extended_range(true),
            PolicySet::new()
                .allow_assumed_chroma_siting(true)
                .with_workspace_limit(u64::MAX),
            PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
            PolicySet::new().allow_opaque_profiles(false),
        ] {
            let pool = StorageBudget::new(32_768, 2);
            let error =
                convert(frame.clone(), &pool, &mut preset, policy, &NoPulse).unwrap_err();
            assert_eq!(error.error().stage, Stage::Policy);
            let record = error.error().report.decisions().next().unwrap();
            assert_eq!(record.code, DecisionCode::Declined);
            // A disallowed siting assumption is situational; it still reaches
            // the preset. Mandatory guarantees refuse before delegation.
            assert_eq!(record.used_preset, record.unmet_requirements.is_empty());
            assert_eq!(record.admitted, u8::from(record.used_preset));
            assert!(record.selected.is_none());
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
        }
        // Partial preparation releases the already-admitted destination.
        let pool = StorageBudget::new(32_768, 1);
        let error = convert(frame, &pool, &mut preset, PolicySet::new(), &NoPulse).unwrap_err();
        assert_eq!(error.error().stage, Stage::Prepare);
        assert!(matches!(
            zencodec::find_cause::<StorageError>(&error),
            Some(StorageError::Objects { .. })
        ));
        assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    }

    #[derive(Debug)]
    struct Secret;
    impl fmt::Display for Secret {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("private-callback-data")
        }
    }
    impl Error for Secret {}

    #[test]
    fn planar_prepared_callback_error_stale_choice_and_unwind_do_not_publish() {
        let frame = native();
        for target in [Target::Rgb16, Target::RgbF32] {
            let pool = StorageBudget::new(32_768, 2);
            let error = convert_target(
                frame.clone(),
                &pool,
                &mut |_: Question<'_>| -> Result<Decision, At<Secret>> {
                    Err(At::wrap(Secret).at_named("conversion user rule"))
                },
                PolicySet::new(),
                target,
                &NoPulse,
            )
            .unwrap_err();
            assert_eq!(error.error().stage, Stage::Policy);
            assert_eq!(
                error.error().report.decisions().next().unwrap().code,
                DecisionCode::CallbackFailed
            );
            assert!(zencodec::find_cause::<At<Secret>>(&error).is_some());
            assert!(!format!("{error} {error:?}").contains("private-callback-data"));
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));

            let mut old: Option<ChoiceId> = None;
            let image = convert_target(
                frame.clone(),
                &pool,
                &mut |q: Question<'_>| -> Result<_, At<Infallible>> {
                    let Question::ImageConversion { choices, .. } = q else {
                        panic!()
                    };
                    old = Some(choices.first().unwrap().id());
                    Ok(Decision::Choose(old.unwrap()))
                },
                PolicySet::new(),
                target,
                &NoPulse,
            )
            .unwrap();
            drop(image);
            let error = convert_target(
                frame.clone(),
                &pool,
                &mut |_: Question<'_>| -> Result<_, At<Infallible>> {
                    Ok(Decision::Choose(old.unwrap()))
                },
                PolicySet::new(),
                target,
                &NoPulse,
            )
            .unwrap_err();
            assert_eq!(
                error.error().report.decisions().next().unwrap().code,
                DecisionCode::InvalidChoice
            );
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));

            let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = convert_target(
                    frame.clone(),
                    &pool,
                    &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                        if target == Target::Rgb16 {
                            assert_eq!(pool.reserved_objects(), 2);
                        } else {
                            assert_eq!(pool.reserved_objects(), 1);
                        }
                        panic!("application unwound while holding reservations")
                    },
                    PolicySet::new(),
                    target,
                    &NoPulse,
                );
            }));
            assert!(unwound.is_err());
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
        }
    }

    struct Countdown(AtomicUsize);
    impl Stop for Countdown {
        fn check(&self) -> Result<(), StopReason> {
            let mut n = self.0.load(Ordering::SeqCst);
            loop {
                let next = n.checked_sub(1).ok_or(StopReason::TimedOut)?;
                match self
                    .0
                    .compare_exchange_weak(n, next, Ordering::SeqCst, Ordering::SeqCst)
                {
                    Ok(_) => return Ok(()),
                    Err(actual) => n = actual,
                }
            }
        }
    }
    #[test]
    fn planar_prepared_cancellation_retains_selected_record_and_drops_partial_output() {
        let frame = native();
        for target in [Target::Rgb16, Target::RgbF32] {
            let pool = StorageBudget::new(32_768, 2);
            let stop = Countdown(AtomicUsize::new(0));
            let called = Cell::new(false);
            let error = convert_target(
                frame.clone(),
                &pool,
                &mut |_: Question<'_>| -> Result<_, At<Infallible>> {
                    called.set(true);
                    Ok(Decision::UsePreset)
                },
                PolicySet::new(),
                target,
                &zencodec::how_far::StopOnly::borrowed(&stop),
            )
            .unwrap_err();
            assert_eq!(error.error().stage, Stage::Prepare);
            assert!(!called.get());
            for (checks, stage) in [
                (0, Stage::Policy),
                (1, Stage::Allocate),
                (3, Stage::Execute),
            ] {
                stop.0.store(usize::MAX, Ordering::SeqCst);
                let error = convert_target(
                    frame.clone(),
                    &pool,
                    &mut |q: Question<'_>| -> Result<_, At<Infallible>> {
                        let Question::ImageConversion { choices, .. } = q else {
                            panic!()
                        };
                        stop.0.store(checks, Ordering::SeqCst);
                        Ok(Decision::choose(choices.first().unwrap()))
                    },
                    PolicySet::new(),
                    target,
                    &zencodec::how_far::StopOnly::borrowed(&stop),
                )
                .unwrap_err();
                assert_eq!(error.error().stage, stage);
                let record = error.error().report.decisions().next().unwrap();
                assert_eq!(record.selected.is_some(), stage != Stage::Policy);
                if stage == Stage::Policy {
                    assert_eq!(
                        zencodec::find_cause::<zencodec::operation::PolicyFailure>(&error)
                            .unwrap()
                            .stop_reason(),
                        Some(StopReason::TimedOut)
                    );
                } else {
                    use zencodec::CodecErrorExt;
                    assert_eq!(
                        error.error_category(),
                        Some(zencodec::ErrorCategory::Stopped(StopReason::TimedOut))
                    );
                }
                assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
            }
        }
    }

    #[test]
    fn planar_prepared_mapping_is_held_once_and_source_owner_is_released() {
        use zencodec::frame::{MapError, Picture};
        struct Tracked {
            inner: Arc<dyn Picture>,
            calls: Arc<AtomicUsize>,
            fail: bool,
        }
        impl Picture for Tracked {
            fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if self.fail {
                    Err(MapError::Backend(Box::new(At::wrap(Secret))))
                } else {
                    self.inner.map()
                }
            }
        }
        let frame = native();
        for target in [Target::Rgb16, Target::RgbF32] {
            let pool = StorageBudget::new(32_768, 2);
            for fail in [false, true] {
                let calls = Arc::new(AtomicUsize::new(0));
                let owner = Arc::new(Tracked {
                    inner: frame.picture().clone(),
                    calls: calls.clone(),
                    fail,
                });
                let weak = Arc::downgrade(&owner);
                let tracked = Frame::new(
                    owner,
                    frame.interpretation().clone(),
                    frame.timing(),
                    frame.presentation(),
                );
                let result = convert_target(
                    tracked,
                    &pool,
                    &mut preset,
                    PolicySet::new().allow_assumed_chroma_siting(true),
                    target,
                    &NoPulse,
                );
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                assert!(weak.upgrade().is_none());
                if fail {
                    let error = result.unwrap_err();
                    assert_eq!(error.error().stage, Stage::Prepare);
                    assert!(zencodec::find_cause::<At<Secret>>(&error).is_some());
                    assert!(!format!("{error} {error:?}").contains("private-callback-data"));
                } else {
                    drop(result.unwrap());
                }
                assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
            }
        }
    }
}
