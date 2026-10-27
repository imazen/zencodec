use super::*;
use crate::{control::CheckStop, jxl_tests::fixture};
use std::{convert::Infallible, io::Cursor};
use tiff::{
    encoder::{TiffEncoder, TiffKind, TiffValue, colortype},
    tags::Tag,
};
use zencodec::{
    NoPulse,
    collection::{Composition, ImageRequest},
    image::{Row, Samples},
    operation::{CollectionStage, Decision, Decline, Question},
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
pub(crate) fn native_tiff_bytes(exif: u8, big: bool, depth: u8) -> Vec<u8> {
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
    file.into_inner()
}
fn native_tiff(exif: u8, big: bool, depth: u8) -> ImageOutput {
    let mut images = zencodec::operation::CheckedCollection::open(
        zencodec::SharedPulse::default(),
        PolicySet::new(),
        |stop| {
            Ok(crate::tiff_items::TiffCollection::open_with_stop(
                Cursor::new(native_tiff_bytes(exif, big, depth)),
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

fn qualify_selected<C: Collection>(
    expected: &ImageOutput,
    open: impl FnOnce(zencodec::SharedPulse) -> Result<C, Cause>,
) {
    use std::cell::Cell;
    let region = Rect {
        x: 1,
        y: 1,
        width: 3,
        height: 2,
    };
    let calls = Cell::new(0);
    let mut renderer = renderer(expected, Some(region));
    let required = renderer.request.required;
    let mut images = CheckedCollection::open(
        zencodec::SharedPulse::default(),
        |question: Question<'_>| -> Result<Decision, At<Infallible>> {
            calls.set(calls.get() + 1);
            let Question::Image { image, facts, .. } = question else {
                panic!()
            };
            assert_eq!(facts.orientation, Some(Orientation::Identity));
            assert_eq!(facts.geometry.region, region);
            assert_eq!(facts.packed, Some(required));
            exact(expected, image, false);
            Ok(Decision::UsePreset)
        },
        open,
    )
    .unwrap();
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    for i in 1..=2 {
        let output = renderer.render_selected(&mut images, &id).unwrap();
        assert_eq!(calls.get(), i);
        assert_eq!(output.report().decisions().count(), 1);
        assert_eq!(images.report().decisions().count(), i);
        drop(output);
        assert_eq!(renderer.storage.reserved_bytes(), 0);
    }
    renderer.request.required = required.with_primaries(ColorPrimaries::Bt2020);
    let failure = renderer.render_selected(&mut images, &id).unwrap_err();
    assert_eq!(failure.error().stage, CollectionStage::Transform);
    assert_eq!(
        zencodec::find_cause::<Failure>(&failure).unwrap().stage,
        Stage::ColorPlan
    );
    assert_eq!(calls.get(), 2);
    assert!(!images.is_finished());
    renderer.request.required = required;
    drop(renderer.render_selected(&mut images, &id).unwrap());
    assert_eq!(calls.get(), 3);
    assert_eq!(renderer.storage.reserved_bytes(), 0);
}

#[test]
fn canvas_render_selected_native_tiff_and_jxl_publish_only_the_final_owner() {
    for big in [false, true] {
        let expected = native_tiff(6, big, 32);
        qualify_selected(&expected, |stop| {
            Ok(crate::tiff_items::TiffCollection::open_with_stop(
                Cursor::new(native_tiff_bytes(6, big, 32)),
                Composition::Composited,
                crate::tiff_region::Allowances::default(),
                stop,
            )?)
        });
    }
    for name in [
        "orientation6_rotate_90_cw.jxl",
        "conformance_test_images/animation_spline.jxl",
    ] {
        let expected = native_jxl(name, Composition::Composited);
        let bytes = fixture(name);
        qualify_selected(&expected, |stop| {
            crate::jxl_items::JxlItems::open_with_stop(&bytes, Composition::Composited, 64, stop)
        });
    }
}

#[test]
fn canvas_render_selected_subifd_keeps_child_color_orientation_and_identity() {
    fn directory<K: TiffKind>(
        mut dir: tiff::encoder::DirectoryEncoder<'_, &mut Cursor<Vec<u8>>, K>,
        orientation: u16,
        seed: usize,
        child: Option<u32>,
    ) -> u32 {
        let samples: Vec<f32> = (0..7 * 5 * 4)
            .map(|i| ((i + seed) % 16) as f32 / 16.0)
            .collect();
        let offset = u32::try_from(dir.write_data(samples.as_slice()).unwrap()).unwrap();
        dir.write_tag(Tag::ImageWidth, 7u32).unwrap();
        dir.write_tag(Tag::ImageLength, 5u32).unwrap();
        dir.write_tag(Tag::SamplesPerPixel, 4u16).unwrap();
        dir.write_tag(Tag::BitsPerSample, &[32u16; 4][..]).unwrap();
        dir.write_tag(Tag::SampleFormat, &[3u16; 4][..]).unwrap();
        dir.write_tag(Tag::ExtraSamples, &[2u16][..]).unwrap();
        dir.write_tag(Tag::PhotometricInterpretation, 2u16).unwrap();
        dir.write_tag(Tag::Compression, 1u16).unwrap();
        dir.write_tag(Tag::RowsPerStrip, 5u32).unwrap();
        dir.write_tag(Tag::StripOffsets, &[offset][..]).unwrap();
        dir.write_tag(Tag::StripByteCounts, &[7 * 5 * 4 * 4u32][..])
            .unwrap();
        dir.write_tag(Tag::Orientation, orientation).unwrap();
        dir.write_tag(Tag::IccProfile, display_p3_icc(false))
            .unwrap();
        if let Some(child) = child {
            dir.write_tag(Tag::SubIfd, &[child][..]).unwrap();
        }
        u32::try_from(dir.finish_with_offsets().unwrap().pointer.0).unwrap()
    }
    fn tree<K: TiffKind>(mut encoder: TiffEncoder<&mut Cursor<Vec<u8>>, K>) {
        let child = directory(encoder.extra_directory().unwrap(), 6, 7, None);
        directory(encoder.image_directory().unwrap(), 1, 0, Some(child));
    }
    for big in [false, true] {
        let mut file = Cursor::new(Vec::new());
        if big {
            tree(TiffEncoder::new_big(&mut file).unwrap());
        } else {
            tree(TiffEncoder::new(&mut file).unwrap());
        }
        let seen = std::cell::Cell::new(0);
        let mut images = CheckedCollection::open(
            zencodec::SharedPulse::default(),
            |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Image { image, facts, .. } = q else {
                    panic!()
                };
                assert_eq!(
                    (facts.geometry.canvas_width, facts.geometry.canvas_height),
                    (5, 7)
                );
                assert_eq!(facts.orientation, Some(Orientation::Identity));
                image
                    .with_pixels(|pixels| {
                        for y in 0..2 {
                            for x in 0..3 {
                                for c in 0..4 {
                                    // Child orientation 6: stored (x,y) -> (4-y,x).
                                    let sx = y + 1;
                                    let sy = 4 - (x + 1);
                                    let expected = (((sy * 7 + sx) * 4 + c + 7) % 16) as f32 / 16.0;
                                    let at = (x * 4 + c) as usize * 4;
                                    assert_eq!(&pixels.row(y)[at..at + 4], &expected.to_ne_bytes());
                                }
                            }
                        }
                    })
                    .unwrap();
                seen.set(seen.get() + 1);
                Ok(Decision::UsePreset)
            },
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
        let mut roots = images.images().unwrap();
        let root = images.next(&mut roots).unwrap().unwrap();
        let mut children = images.children(&root).unwrap();
        let child = images.next(&mut children).unwrap().unwrap();
        assert_ne!(root, child);
        assert!(images.next(&mut children).unwrap().is_none());
        let renderer = Renderer {
            request: Request {
                target: Target::Icc(Arc::from(display_p3_icc(false))),
                required: PixelDescriptor::RGBAF32
                    .with_transfer(TransferFunction::Srgb)
                    .with_primaries(ColorPrimaries::DisplayP3),
                region: Some(Rect {
                    x: 1,
                    y: 1,
                    width: 3,
                    height: 2,
                }),
                conversion: ConvertOptions::retain_channels(),
            },
            storage: StorageBudget::new(1 << 20, 2),
            cms: Some(&MoxCms),
        };
        drop(renderer.render_selected(&mut images, &child).unwrap());
        assert_eq!(seen.get(), 1);
        assert_eq!(renderer.storage.reserved_bytes(), 0);
    }
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
            conversion: ConvertOptions::retain_channels().with_clip_out_of_gamut(false),
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
            &NoPulse,
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
            &NoPulse,
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
            &NoPulse,
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
                            &NoPulse,
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
                &NoPulse,
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
            &NoPulse,
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
            &NoPulse,
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
            &NoPulse
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
        .render(source, &mut policy, PolicySet::new(), &NoPulse)
        .unwrap_err();
    assert_eq!(error.error().stage, Stage::ColorPlan);
    assert_eq!(renderer.storage.reserved_bytes(), 0);
    let raw = native_jxl(
        "multiple_layers_noise_spline.jxl",
        Composition::Uncomposited,
    );
    renderer.request.required = p3_descriptor(&raw);
    let error = renderer
        .render(raw, &mut policy, PolicySet::new(), &NoPulse)
        .unwrap_err();
    assert_eq!(error.error().stage, Stage::Geometry);
    assert!(zencodec::find_cause::<crate::canvas_projection::Refusal>(&error).is_some());
    assert_eq!(renderer.storage.reserved_bytes(), 0);
    assert_eq!(calls, 0);
}

#[test]
fn canvas_render_public_default_delivers_oriented_srgb_rgba_and_retains_alpha() {
    let renderer = Renderer::new(StorageBudget::new(1 << 20, 32)).with_cms(&MoxCms);
    for exif in 1..=8 {
        let source = native_tiff(exif, exif % 2 == 0, 8);
        let before = source.clone();
        let output = renderer
            .render(
                source,
                &mut PolicySet::new(),
                PolicySet::new(),
                &NoPulse,
            )
            .unwrap();
        let dimensions = if exif <= 4 { (7, 5) } else { (5, 7) };
        assert_eq!(
            output.interpretation().geometry.region,
            Rect {
                x: 0,
                y: 0,
                width: dimensions.0,
                height: dimensions.1
            }
        );
        assert_eq!(
            output.interpretation().orientation,
            Some(Orientation::Identity)
        );
        output
            .with_pixels(|pixels| {
                assert_eq!(pixels.descriptor(), PixelDescriptor::RGBA8_SRGB);
                before
                    .with_pixels(|original| {
                        for y in 0..5 {
                            for x in 0..7 {
                                let (dx, dy) = position(exif, x, y, 7, 5);
                                assert_eq!(
                                    pixels.row(dy)[dx as usize * 4 + 3],
                                    original.row(y)[x as usize * 4 + 3]
                                );
                            }
                        }
                    })
                    .unwrap();
            })
            .unwrap();
        assert_eq!(output.report().decisions().count(), 1);
        drop(output);
        assert_eq!(renderer.storage.reserved_bytes(), 0);
    }
}
