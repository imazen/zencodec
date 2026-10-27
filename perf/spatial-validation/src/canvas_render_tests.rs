use super::*;
use crate::{control::CheckStop, jxl_tests::fixture};
use std::{convert::Infallible, io::Cursor};
use tiff::{
    encoder::{TiffEncoder, TiffKind, TiffValue, colortype},
    tags::Tag,
};
use zencodec::{
    Unstoppable,
    collection::{Composition, ImageRequest},
    image::{Row, Samples},
    operation::{Decision, Decline, Question},
};
use zenpixels::{ColorPrimaries, Orientation, TransferFunction};
use zenpixels_convert::{cms_moxcms::MoxCms, icc_profiles::display_p3_icc};

#[cfg(feature = "cms_oracle")]
#[path = "canvas_render_oracle_tests.rs"]
mod oracle;

fn tiff<C: colortype::ColorType, K: TiffKind>(
    mut encoder: TiffEncoder<&mut Cursor<Vec<u8>>, K>,
    samples: &[C::Inner],
    exif: u8,
) where
    [C::Inner]: TiffValue,
{
    let mut image = encoder.new_image::<C>(7, 5).unwrap();
    image.rows_per_strip(2).unwrap();
    image
        .encoder()
        .write_tag(Tag::Orientation, u16::from(exif))
        .unwrap();
    image
        .encoder()
        .write_tag(Tag::ExtraSamples, &[2u16][..])
        .unwrap();
    image
        .encoder()
        .write_tag(Tag::IccProfile, display_p3_icc(false))
        .unwrap();
    image.write_data(samples).unwrap();
}
fn native_tiff(exif: u8, big: bool, depth: u8) -> ImageOutput {
    let mut file = Cursor::new(Vec::new());
    macro_rules! encode {
        ($color:ty, $samples:expr) => {
            if big {
                tiff::<$color, _>(TiffEncoder::new_big(&mut file).unwrap(), &$samples, exif);
            } else {
                tiff::<$color, _>(TiffEncoder::new(&mut file).unwrap(), &$samples, exif);
            }
        };
    }
    match depth {
        8 => encode!(
            colortype::RGBA8,
            (0..7 * 5 * 4).map(|n| (n * 37) as u8).collect::<Vec<_>>()
        ),
        16 => encode!(
            colortype::RGBA16,
            (0..7 * 5 * 4).map(|n| (n * 733) as u16).collect::<Vec<_>>()
        ),
        32 => encode!(
            colortype::RGBA32Float,
            (0..7 * 5 * 4)
                .map(|n| (n % 11) as f32 / 10.0)
                .collect::<Vec<_>>()
        ),
        _ => panic!("invalid depth"),
    }
    let mut images = zencodec::operation::CheckedCollection::open(
        zencodec::StopToken::new(Unstoppable),
        PolicySet::new(),
        |stop| {
            Ok(crate::tiff_items::TiffCollection::open_with_stop(
                Cursor::new(file.into_inner()),
                Composition::Composited,
                crate::tiff_region::Allowances::default(),
                stop,
            )?)
        },
    )
    .unwrap();
    images
        .first_image(&ImageRequest::new())
        .unwrap()
        .unwrap()
        .into_output()
}
fn native_jxl(name: &str, view: Composition) -> ImageOutput {
    let bytes = fixture(name);
    let (codecs, pool) = crate::jxl_packaged_storage::factories(64, 64);
    let mut source = zencodec::source::ReadAtShim(zencodec::source::SliceReadAt::new(&bytes));
    let mut images =
        crate::jxl_packaged_storage::open_admitted(&codecs, &mut source, view, 64, &pool).unwrap();
    images
        .first_image(&ImageRequest::new())
        .unwrap()
        .unwrap()
        .into_output()
}
fn p3_descriptor(source: &ImageOutput) -> PixelDescriptor {
    source
        .with_pixels(|p| {
            p.descriptor()
                .with_transfer(TransferFunction::Srgb)
                .with_primaries(ColorPrimaries::DisplayP3)
        })
        .unwrap()
}
fn renderer(source: &ImageOutput, region: Option<Rect>) -> Renderer<'static> {
    Renderer {
        request: Request {
            target: Target::Icc(Arc::from(display_p3_icc(false))),
            required: p3_descriptor(source),
            region,
            conversion: ConvertOptions::forbid_lossy().with_clip_out_of_gamut(false),
        },
        storage: StorageBudget::new(256 * 1024 * 1024, 32),
        cms: Some(&MoxCms),
    }
}
// Independent orientation table: this never calls the projection's D4 helpers.
fn position(exif: u8, x: u32, y: u32, w: u32, h: u32) -> (u32, u32) {
    match exif {
        1 => (x, y),
        2 => (w - 1 - x, y),
        3 => (w - 1 - x, h - 1 - y),
        4 => (x, h - 1 - y),
        5 => (y, x),
        6 => (h - 1 - y, x),
        7 => (h - 1 - y, w - 1 - x),
        8 => (y, w - 1 - x),
        _ => panic!(),
    }
}
fn exact(source: &ImageOutput, output: &ImageOutput, color: bool) {
    let before = source.interpretation();
    let after = output.interpretation();
    assert_eq!(after.orientation, Some(Orientation::Identity));
    assert_eq!(after.composition, Some(Composition::Composited));
    assert_eq!(after.placement, None);
    assert_eq!(after.timing, before.timing);
    let region = after.geometry.region;
    let a = source.map().unwrap();
    let b = output.map().unwrap();
    assert_eq!(a.plane_count(), b.plane_count());
    for index in 0..a.plane_count() {
        let original = a.plane(index).unwrap().unwrap();
        let actual = b.plane(index).unwrap().unwrap();
        assert_eq!(original.meaning, actual.meaning);
        if !color && index == 0 {
            continue;
        }
        let bpp = match &original.samples {
            Samples::Pixels(p) => p.descriptor().bytes_per_pixel(),
            Samples::Values(_) => original.samples.channel_type().byte_size(),
            _ => panic!("expected scalar storage"),
        };
        let mut visited = 0;
        for y in 0..before.geometry.region.height {
            let Row::Bytes(row) = original.samples.row(y as usize).unwrap() else {
                panic!()
            };
            for x in 0..before.geometry.region.width {
                let (dx, dy) = position(
                    before.orientation.unwrap().to_exif(),
                    x + before.geometry.region.x,
                    y + before.geometry.region.y,
                    before.geometry.canvas_width,
                    before.geometry.canvas_height,
                );
                if dx < region.x
                    || dy < region.y
                    || dx >= region.x + region.width
                    || dy >= region.y + region.height
                {
                    continue;
                }
                let Row::Bytes(dst) = actual.samples.row((dy - region.y) as usize).unwrap() else {
                    panic!()
                };
                let from = x as usize * bpp;
                let to = (dx - region.x) as usize * bpp;
                assert_eq!(&row[from..from + bpp], &dst[to..to + bpp]);
                visited += 1;
            }
        }
        assert_eq!(visited, region.width * region.height);
    }
}

// Called by the native generated/subsampled fixture matrix as well as the held
// render tests. Color conversion never reinterprets auxiliary scalar values.
pub(crate) fn qualify_auxiliaries(source: &ImageOutput, region: Rect) {
    let renderer = renderer(source, Some(region));
    let output = renderer
        .render(
            source.clone(),
            &mut PolicySet::new(),
            PolicySet::new(),
            &Unstoppable,
        )
        .unwrap();
    exact(source, output.output(), false);
    drop(output);
    assert_eq!(renderer.storage.reserved_bytes(), 0);
    assert_eq!(renderer.storage.reserved_objects(), 0);
}

#[derive(Debug)]
struct ApplicationFailure;
impl std::fmt::Display for ApplicationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private tenant identifier")
    }
}
impl std::error::Error for ApplicationFailure {}

#[test]
fn canvas_render_traced_callback_failure_and_color_admission_are_explicit() {
    let source = native_tiff(6, false, 32);
    let mut renderer = renderer(&source, None);
    let failure = renderer
        .render(
            source.clone(),
            &mut |_: Question<'_>| -> Result<Decision, At<ApplicationFailure>> {
                Err(At::wrap(ApplicationFailure))
            },
            PolicySet::new(),
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(failure.error().stage, Stage::Publication);
    assert!(zencodec::find_cause::<ApplicationFailure>(&failure).is_some());
    assert!(zencodec::find_cause::<At<ApplicationFailure>>(&failure).is_some());
    assert!(!format!("{failure:?} {failure}").contains("private tenant"));
    assert_eq!(renderer.storage.reserved_bytes(), 0);
    renderer.storage = StorageBudget::new(1 << 20, 1);
    // The projected color occupies the first object. Replacing its unknown
    // descriptor with the ICC-resolved descriptor needs a second owner in
    // this candidate; that cost cannot be hidden by exceeding the pool.
    let failure = renderer
        .render(
            source,
            &mut PolicySet::new(),
            PolicySet::new(),
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(failure.error().stage, Stage::Color);
    assert!(zencodec::find_cause::<StorageError>(&failure).is_some());
    assert_eq!(renderer.storage.reserved_bytes(), 0);
    assert_eq!(renderer.storage.reserved_objects(), 0);
}

#[test]
fn canvas_render_native_tiff_all_orientations_depths_and_containers() {
    for big in [false, true] {
        for depth in [8, 16, 32] {
            for exif in 1..=8 {
                let source = native_tiff(exif, big, depth);
                for region in [
                    None,
                    Some(Rect {
                        x: 1,
                        y: 1,
                        width: 3,
                        height: 2,
                    }),
                ] {
                    let mut renderer = renderer(&source, region);
                    // Both current and explicit ICC targets must preserve same-profile samples.
                    if region.is_none() {
                        renderer.request.target = Target::Current;
                    }
                    let mut calls = 0;
                    let output = renderer
                        .render(
                            source.clone(),
                            &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                                calls += 1;
                                let Question::Image { image, facts, .. } = q else {
                                    panic!()
                                };
                                assert_eq!(facts.packed, Some(renderer.request.required));
                                exact(&source, image, true);
                                Ok(Decision::UsePreset)
                            },
                            PolicySet::new(),
                            &Unstoppable,
                        )
                        .unwrap();
                    assert_eq!(calls, 1);
                    exact(&source, output.output(), true);
                    drop(output);
                    assert_eq!(renderer.storage.reserved_bytes(), 0);
                    assert_eq!(renderer.storage.reserved_objects(), 0);
                }
            }
        }
    }
}

#[test]
fn canvas_render_native_jxl_color_auxiliaries_and_named_profile() {
    for name in [
        "conformance_test_images/animation_spline.jxl",
        "conformance_test_images/cmyk_layers.jxl",
        "orientation6_rotate_90_cw.jxl",
    ] {
        let source = native_jxl(name, Composition::Composited);
        let mut renderer = renderer(
            &source,
            Some(Rect {
                x: 1,
                y: 1,
                width: 3,
                height: 2,
            }),
        );
        renderer.request.target = Target::Named(Cicp::DISPLAY_P3);
        let output = renderer
            .render(
                source.clone(),
                &mut PolicySet::new(),
                PolicySet::new(),
                &Unstoppable,
            )
            .unwrap();
        exact(&source, output.output(), false);
        output
            .with_pixels(|pixels| {
                assert_eq!(pixels.descriptor(), renderer.request.required);
                assert_eq!(pixels.color_context().unwrap().cicp, Some(Cicp::DISPLAY_P3));
                assert_eq!(pixels.color_context().unwrap().icc, None);
            })
            .unwrap();
        drop(output);
        assert_eq!(renderer.storage.reserved_bytes(), 0);
    }
}

#[test]
fn canvas_render_every_stop_seam_and_refusal_drops_unpublished_storage() {
    let source = native_tiff(6, false, 32);
    let renderer = renderer(&source, None);
    let counted = CheckStop::default();
    drop(
        renderer
            .render(
                source.clone(),
                &mut PolicySet::new(),
                PolicySet::new(),
                &counted,
            )
            .unwrap(),
    );
    assert!(counted.calls() > 8);
    for reason in [
        zencodec::enough::StopReason::Cancelled,
        zencodec::enough::StopReason::TimedOut,
    ] {
        for at in 0..counted.calls() {
            let stop = CheckStop::with_reason(reason);
            stop.after(at);
            let failure = renderer
                .render(
                    source.clone(),
                    &mut PolicySet::new(),
                    PolicySet::new(),
                    &stop,
                )
                .unwrap_err();
            let codec = zencodec::find_cause::<CodecError>(&failure);
            let policy = zencodec::find_cause::<zencodec::operation::PolicyFailure>(&failure);
            assert!(
                codec.is_some_and(|e| e.category() == zencodec::ErrorCategory::Stopped(reason))
                    || policy.is_some_and(|e| e.stop_reason() == Some(reason))
            );
            assert_eq!(renderer.storage.reserved_bytes(), 0);
        }
    }
    let refused = renderer
        .render(
            source.clone(),
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                Ok(Decision::decline(Decline::ApplicationRule))
            },
            PolicySet::new(),
            &Unstoppable,
        )
        .unwrap_err();
    assert_eq!(refused.error().stage, Stage::Publication);
    assert!(zencodec::find_cause::<Decline>(&refused).is_some());
    assert_eq!(renderer.storage.reserved_bytes(), 0);
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.render(
            source.clone(),
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                panic!("injected publication panic")
            },
            PolicySet::new(),
            &Unstoppable,
        )
    }));
    assert!(unwind.is_err());
    assert_eq!(renderer.storage.reserved_bytes(), 0);
    let mut tiny = renderer;
    tiny.storage = StorageBudget::new(1, 1);
    assert_eq!(
        tiny.render(
            source,
            &mut PolicySet::new(),
            PolicySet::new(),
            &Unstoppable
        )
        .unwrap_err()
        .error()
        .stage,
        Stage::Geometry
    );
    assert_eq!(tiny.storage.reserved_bytes(), 0);
}

#[test]
fn canvas_render_refuses_wrong_color_before_projection_and_raw_layers_before_publication() {
    let source = native_tiff(1, false, 32);
    let mut renderer = renderer(&source, None);
    renderer.request.required = PixelDescriptor::RGBA8_SRGB;
    let mut calls = 0;
    let mut policy = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        calls += 1;
        Ok(Decision::UsePreset)
    };
    let error = renderer
        .render(source, &mut policy, PolicySet::new(), &Unstoppable)
        .unwrap_err();
    assert_eq!(error.error().stage, Stage::ColorPlan);
    assert_eq!(renderer.storage.reserved_bytes(), 0);
    let raw = native_jxl(
        "multiple_layers_noise_spline.jxl",
        Composition::Uncomposited,
    );
    renderer.request.required = p3_descriptor(&raw);
    let error = renderer
        .render(raw, &mut policy, PolicySet::new(), &Unstoppable)
        .unwrap_err();
    assert_eq!(error.error().stage, Stage::Geometry);
    assert!(zencodec::find_cause::<crate::canvas_projection::Refusal>(&error).is_some());
    assert_eq!(renderer.storage.reserved_bytes(), 0);
    assert_eq!(calls, 0);
}
