use super::*;
use crate::{control::CheckStop, jxl_tests::fixture};
use std::{io::Cursor, path::PathBuf};
use tiff::{
    encoder::{TiffEncoder, TiffKind, TiffValue, colortype},
    tags::Tag,
};
use zencodec::{
    NoPulse, SharedPulse,
    collection::{ImageRequest, OpenRequest},
    image::{AuxiliaryKind, ColorContribution, ImageInterpretation, Row, SourcePrecision, Values},
    operation::{CheckedCollection, PolicySet},
};
use zenpixels::{AlphaMode, ChannelType, PixelBuffer, PixelBufferParts};

// Independent EXIF coordinate table; do not call the implementation's D4 helpers.
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
        _ => panic!("invalid test orientation"),
    }
}
fn exact(source: &ImageOutput, output: &ImageOutput) {
    let before = source.interpretation();
    let after = output.interpretation();
    let exif = before.orientation.unwrap().to_exif();
    let g = before.geometry;
    let region = after.geometry.region;
    let canvas = if exif < 5 {
        (g.canvas_width, g.canvas_height)
    } else {
        (g.canvas_height, g.canvas_width)
    };
    assert_eq!(
        (after.geometry.canvas_width, after.geometry.canvas_height),
        canvas
    );
    assert_eq!(after.orientation, Some(Orientation::Identity));
    assert_eq!(after.composition, Some(Composition::Composited));
    assert_eq!(after.placement, None);
    assert_eq!(after.timing, before.timing);
    assert_eq!(
        *after.source_details.get::<ProjectionSource>().unwrap(),
        ProjectionSource {
            geometry: g,
            orientation: before.orientation.unwrap()
        }
    );
    let a = source.map().unwrap();
    let b = output.map().unwrap();
    assert_eq!(a.plane_count(), b.plane_count());
    for index in 0..a.plane_count() {
        let original = a.plane(index).unwrap().unwrap();
        let actual = b.plane(index).unwrap().unwrap();
        assert_eq!(original.meaning, actual.meaning);
        assert_eq!(
            original.samples.channel_type(),
            actual.samples.channel_type()
        );
        assert_eq!(
            actual.samples.dimensions(),
            (region.width as usize, region.height as usize)
        );
        let (bytes, bpp) = match (&original.samples, &actual.samples) {
            (Samples::Pixels(a), Samples::Pixels(b)) => {
                assert_eq!(a.descriptor(), b.descriptor());
                assert_eq!(a.color_context(), b.color_context());
                (b.as_strided_bytes(), b.descriptor().bytes_per_pixel())
            }
            (Samples::Values(a), Samples::Values(b)) => {
                assert_eq!(a.grid(), AuxiliaryGrid::ColorSamples);
                assert_eq!(b.grid(), AuxiliaryGrid::ColorSamples);
                (
                    b.as_strided_bytes(),
                    actual.samples.channel_type().byte_size(),
                )
            }
            _ => panic!("sample role changed"),
        };
        let mut visited = 0;
        for y in 0..g.region.height {
            let Row::Bytes(source_row) = original.samples.row(y as usize).unwrap() else {
                panic!("not byte storage")
            };
            for x in 0..g.region.width {
                let (dx, dy) = position(
                    exif,
                    x + g.region.x,
                    y + g.region.y,
                    g.canvas_width,
                    g.canvas_height,
                );
                if dx < region.x
                    || dy < region.y
                    || dx >= region.x + region.width
                    || dy >= region.y + region.height
                {
                    continue;
                }
                let offset = (dy - region.y) as usize * actual.samples.stride_bytes()
                    + (dx - region.x) as usize * bpp;
                assert_eq!(
                    &bytes[offset..offset + bpp],
                    &source_row[x as usize * bpp..(x as usize + 1) * bpp]
                );
                visited += 1;
            }
        }
        assert_eq!(visited, region.width as usize * region.height as usize);
        for y in 0..region.height as usize {
            let end = (y + 1) * actual.samples.stride_bytes();
            let active = y * actual.samples.stride_bytes() + region.width as usize * bpp;
            assert!(
                bytes[active..end].iter().all(|&b| b == 0xa5),
                "destination padding changed"
            );
        }
    }
}

fn regions(image: &ImageOutput) -> [Option<Rect>; 3] {
    let g = image.interpretation().geometry;
    let exif = image.interpretation().orientation.unwrap().to_exif();
    let (w, h) = if exif < 5 {
        (g.canvas_width, g.canvas_height)
    } else {
        (g.canvas_height, g.canvas_width)
    };
    assert!(w > 2 && h > 2);
    [
        None,
        Some(Rect {
            x: 1,
            y: 1,
            width: w - 2,
            height: h - 2,
        }),
        Some(Rect {
            x: w - 1,
            y: h - 1,
            width: 1,
            height: 1,
        }),
    ]
}
fn qualify(source: &ImageOutput) -> usize {
    let mut checked = 0;
    for region in regions(source) {
        let pool = StorageBudget::new(256 * 1024 * 1024, 16);
        let output = project(source, region, &pool, Some(&NoPulse)).unwrap();
        exact(source, &output);
        assert!(pool.reserved_bytes() > 0);
        let retained = output.clone();
        drop(output);
        assert!(pool.reserved_bytes() > 0);
        drop(retained);
        assert_eq!(pool.reserved_bytes(), 0);
        assert_eq!(pool.reserved_objects(), 0);
        checked += 1;
    }
    checked
}

fn receipt(name: &str, value: serde_json::Value) {
    let root = PathBuf::from(
        std::env::var_os("SPATIAL_PROJECTION_ARTIFACTS").expect("set SPATIAL_PROJECTION_ARTIFACTS"),
    );
    std::fs::create_dir_all(&root).unwrap();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(format!("projection-{name}.json")))
        .expect("fresh projection receipt");
    serde_json::to_writer_pretty(file, &value).unwrap();
}

fn tiff_bytes<C: colortype::ColorType, K: TiffKind>(
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
    image.write_data(samples).unwrap();
}
fn tiff_image(bytes: Vec<u8>, region: Option<Rect>) -> ImageOutput {
    let mut images = CheckedCollection::open(SharedPulse::default(), PolicySet::new(), |stop| {
        Ok(crate::tiff_items::TiffCollection::open_with_stop(
            Cursor::new(bytes),
            Composition::Composited,
            crate::tiff_region::Allowances::default(),
            stop,
        )?)
    })
    .unwrap();
    let request = region.map_or_else(ImageRequest::new, |r| ImageRequest::new().with_region(r));
    images.first_image(&request).unwrap().unwrap().into_output()
}

#[test]
fn canvas_projection_native_tiff_all_orientations_regions_and_float_payloads() {
    // Include exact NaN payloads, signed zero and excursions: geometry performs
    // no arithmetic. Native adoption and projection must preserve their bits.
    let floats: Vec<f32> = (0..7 * 5 * 4)
        .map(|n| {
            f32::from_bits([0x7fc01234, 0x80000000, 0xbf800000, 0x40000000, 0x3f000000][n % 5])
        })
        .collect();
    let bytes: Vec<u8> = (0..7 * 5 * 4).map(|n| (n * 31) as u8).collect();
    let (mut files, mut projections, mut partial_refusals) = (0, 0, 0);
    for big in [false, true] {
        for exif in 1..=8 {
            for float in [false, true] {
                let mut file = Cursor::new(Vec::new());
                match (big, float) {
                    (false, false) => tiff_bytes::<colortype::RGBA8, _>(
                        TiffEncoder::new(&mut file).unwrap(),
                        &bytes,
                        exif,
                    ),
                    (true, false) => tiff_bytes::<colortype::RGBA8, _>(
                        TiffEncoder::new_big(&mut file).unwrap(),
                        &bytes,
                        exif,
                    ),
                    (false, true) => tiff_bytes::<colortype::RGBA32Float, _>(
                        TiffEncoder::new(&mut file).unwrap(),
                        &floats,
                        exif,
                    ),
                    (true, true) => tiff_bytes::<colortype::RGBA32Float, _>(
                        TiffEncoder::new_big(&mut file).unwrap(),
                        &floats,
                        exif,
                    ),
                }
                let input = file.into_inner();
                let source = tiff_image(input.clone(), None);
                let original = if float {
                    bytemuck::cast_slice(&floats)
                } else {
                    &bytes
                };
                source
                    .with_pixels(|pixels| {
                        for (y, row) in original
                            .chunks(7 * pixels.descriptor().bytes_per_pixel())
                            .enumerate()
                        {
                            assert_eq!(pixels.row(y as u32), row);
                        }
                    })
                    .unwrap();
                files += 1;
                projections += qualify(&source);
                let available = Rect {
                    x: 2,
                    y: 1,
                    width: 3,
                    height: 3,
                };
                let source = tiff_image(input, Some(available));
                let (ax, ay) = position(exif, 2, 1, 7, 5);
                let (bx, by) = position(exif, 4, 3, 7, 5);
                let region = Rect {
                    x: ax.min(bx),
                    y: ay.min(by),
                    width: ax.abs_diff(bx) + 1,
                    height: ay.abs_diff(by) + 1,
                };
                let pool = StorageBudget::new(1024 * 1024, 1);
                let output = project(&source, Some(region), &pool, Some(&NoPulse)).unwrap();
                exact(&source, &output);
                projections += 1;
                drop(output);
                let error = project(&source, None, &pool, Some(&NoPulse)).unwrap_err();
                assert!(matches!(
                    zencodec::find_cause::<Refusal>(&*error),
                    Some(Refusal::MissingStoredRegion { .. })
                ));
                assert_eq!(pool.reserved_bytes(), 0);
                partial_refusals += 1;
            }
        }
    }
    receipt(
        "tiff",
        serde_json::json!({"files":files,"projections":projections,"missing_full_canvas_refusals":partial_refusals,"orientations":8,"representations":["RGBA8","RGBA32Float"],"containers":["TIFF","BigTIFF"],"exact_samples":true}),
    );
}

fn jxl_images(bytes: &[u8], view: Composition) -> Vec<ImageOutput> {
    let codecs = zencodec::CodecSet::new()
        .with_collection(zenjxl::collection::JxlCollectionConfig::new(64, 128));
    let mut source = zencodec::source::ReadAtShim(zencodec::source::SliceReadAt::new(bytes));
    let mut images = codecs
        .open_collection(
            zencodec::ImageFormat::Jxl,
            &mut source,
            OpenRequest::new(view, 128),
            PolicySet::new(),
            SharedPulse::default(),
        )
        .unwrap();
    let mut cursor = images.images().unwrap();
    let mut result = Vec::new();
    while let Some(id) = images.next(&mut cursor).unwrap() {
        result.push(
            images
                .decode(&id, &ImageRequest::new())
                .unwrap()
                .into_output(),
        );
    }
    result
}
fn independent(name: &str, bytes: &[u8]) -> (Vec<u32>, Vec<u32>) {
    let root = PathBuf::from(
        std::env::var_os("SPATIAL_PROJECTION_ARTIFACTS").expect("set SPATIAL_PROJECTION_ARTIFACTS"),
    )
    .join("canvas-projection")
    .join(name);
    std::fs::create_dir_all(root.parent().unwrap()).unwrap();
    std::fs::create_dir(&root).expect("fresh independent evidence directory");
    let input = root.join("source.jxl");
    std::fs::write(&input, bytes).unwrap();
    let run = std::process::Command::new("timeout")
        .args(["--kill-after=2s", "30s"])
        .arg(
            std::env::var_os("JXL_SELECTION_ORACLE")
                .expect("set pinned orientation-capable libjxl oracle"),
        )
        .arg(input)
        .args(["1", "samples-oriented"])
        .output()
        .unwrap();
    std::fs::write(root.join("stdout.txt"), &run.stdout).unwrap();
    std::fs::write(root.join("stderr.txt"), &run.stderr).unwrap();
    assert!(
        run.status.success(),
        "independent orientation decoder: {:?}",
        run.status
    );
    let text = String::from_utf8(run.stdout).unwrap();
    let values = |prefix| {
        text.lines()
            .find_map(|l| l.strip_prefix(prefix))
            .unwrap()
            .split_whitespace()
            .map(|s| s.parse().unwrap())
            .collect()
    };
    assert!(text.starts_with("version 11001\n"));
    (values("canvas "), values("pixels 0 "))
}

#[test]
fn canvas_projection_native_jxl_all_orientations_match_independent_display() {
    let (mut projections, mut independent) = (0, 0);
    for name in [
        "orientation1_identity.jxl",
        "orientation2_flip_horizontal.jxl",
        "orientation3_rotate_180.jxl",
        "orientation4_flip_vertical.jxl",
        "orientation5_transpose.jxl",
        "orientation6_rotate_90_cw.jxl",
        "orientation7_anti_transpose.jxl",
        "orientation8_rotate_90_ccw.jxl",
    ] {
        let bytes = fixture(name);
        let mut images = jxl_images(&bytes, Composition::Composited);
        assert_eq!(images.len(), 1);
        let source = images.pop().unwrap();
        projections += qualify(&source);
        let pool = StorageBudget::new(256 * 1024 * 1024, 16);
        let output = project(&source, None, &pool, Some(&NoPulse)).unwrap();
        let (canvas, expected) = self::independent(name, &bytes);
        assert_eq!(
            canvas,
            vec![
                output.interpretation().geometry.canvas_width,
                output.interpretation().geometry.canvas_height
            ]
        );
        output
            .with_pixels(|pixels| {
                assert_eq!(pixels.descriptor().channel_type(), ChannelType::F32);
                assert_eq!(pixels.descriptor().layout(), ChannelLayout::Rgb);
                assert_eq!(
                    expected.len(),
                    pixels.width() as usize * pixels.rows() as usize * 3
                );
                for (row, reference) in
                    (0..pixels.rows()).zip(expected.chunks(pixels.width() as usize * 3))
                {
                    for (actual, expected) in
                        pixels.row(row).as_chunks::<4>().0.iter().zip(reference)
                    {
                        let actual = f32::from_ne_bytes(*actual);
                        let expected = f32::from_bits(*expected);
                        // Existing independent native F32 conformance tolerance.
                        // Projection itself is bit-exact against the source above.
                        assert!(actual.is_finite() && expected.is_finite());
                        assert!(
                            (actual - expected).abs() <= 1.0 / 256.0,
                            "{name}: {actual} != {expected}"
                        );
                    }
                }
            })
            .unwrap();
        projections += 1;
        independent += 1;
    }
    receipt(
        "jxl-orientation",
        serde_json::json!({"projections":projections,"independent_libjxl_presentations":independent,"libjxl_version":11001,"exact_source_projection":true,"independent_max_allowed_absolute_error":1.0/256.0}),
    );
}

#[test]
fn canvas_projection_preserves_native_canvas_and_refuses_oversized_raw_layers() {
    let (mut projections, mut native_presentations, mut raw_refusals) = (0, 0, 0);
    for name in [
        "multiple_layers_noise_spline.jxl",
        "conformance_test_images/cmyk_layers.jxl",
    ] {
        let bytes = fixture(name);
        let presentations = jxl_images(&bytes, Composition::Composited);
        assert!(!presentations.is_empty());
        for source in &presentations {
            projections += qualify(source);
            native_presentations += 1;
        }
        let layers = jxl_images(&bytes, Composition::Uncomposited);
        assert_eq!(
            layers.len(),
            if name.starts_with("multiple") { 5 } else { 4 }
        );
        for layer in &layers {
            let pool = StorageBudget::new(0, 0);
            let error = project(layer, None, &pool, Some(&NoPulse)).unwrap_err();
            assert_eq!(
                zencodec::find_cause::<Refusal>(&*error),
                Some(&Refusal::Composition(Some(Composition::Uncomposited)))
            );
            assert_eq!(pool.reserved_bytes(), 0);
            raw_refusals += 1;
        }
        if name.starts_with("multiple") {
            let layer = &layers[4];
            let facts = layer.interpretation();
            assert_eq!(
                (facts.geometry.region.width, facts.geometry.region.height),
                (1024, 576)
            );
            let visible = facts
                .placement
                .unwrap()
                .visible(facts.geometry)
                .unwrap()
                .unwrap();
            assert_eq!((visible.samples.width, visible.samples.height), (188, 202));
        }
    }
    receipt(
        "jxl-layers",
        serde_json::json!({"projections":projections,"native_presentations":native_presentations,"raw_layer_refusals":raw_refusals,"oversized_layer":[1024,576],"visible_intersection":[188,202]}),
    );
}

fn synthetic(exif: u8, unaligned: bool, composition: Option<Composition>) -> ImageOutput {
    synthetic_with_grid(exif, unaligned, composition, AuxiliaryGrid::ColorSamples)
}

fn synthetic_with_grid(
    exif: u8,
    unaligned: bool,
    composition: Option<Composition>,
    grid: AuxiliaryGrid,
) -> ImageOutput {
    let width = 7;
    let height = 5;
    let geometry = ImageGeometry {
        canvas_width: width,
        canvas_height: height,
        region: Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    };
    let mut interpretation =
        ImageInterpretation::packed(geometry, Orientation::from_exif(exif).unwrap());
    interpretation.composition = composition;
    interpretation.timing = Some(zencodec::frame::FrameTiming::default());
    let descriptor = PixelDescriptor::new(
        ChannelType::F32,
        ChannelLayout::Rgba,
        Some(AlphaMode::Premultiplied),
        TransferFunction::Linear,
    );
    let stride = width as usize * 16 + 16;
    let bytes = (0..stride * height as usize).map(|x| x as u8).collect();
    let color = PixelBuffer::try_from_parts(PixelBufferParts::new(
        bytes, width, height, stride, descriptor,
    ))
    .unwrap();
    let mut extras = Vec::new();
    for (index, (channel, kind)) in [
        (ChannelType::U8, AuxiliaryKind::SelectionMask),
        (
            ChannelType::F32,
            AuxiliaryKind::SpotColor {
                linear_rgba: [0.25, 0.5, 0.75, 1.0],
                contribution: ColorContribution::Applied,
            },
        ),
        (ChannelType::F64, AuxiliaryKind::Depth),
    ]
    .into_iter()
    .enumerate()
    {
        let w = if unaligned && index == 2 {
            width - 1
        } else {
            width
        };
        let stride = w as usize * channel.byte_size() + 16;
        let bytes = (0..stride * height as usize)
            .map(|n| (n * 37 + index) as u8)
            .collect();
        extras.push((
            Auxiliary {
                kind,
                name: format!("source {index}"),
                source_precision: SourcePrecision::Integer {
                    bits: 11,
                    signed: false,
                },
                source_dim_shift: 0,
            },
            Values::new(bytes, w, height, stride, channel)
                .unwrap()
                .with_grid(if w == width {
                    grid
                } else {
                    AuxiliaryGrid::Unknown
                }),
        ));
    }
    ImageOutput::packed(color, extras, interpretation).unwrap()
}

#[test]
fn canvas_projection_same_size_unknown_auxiliary_registration_refuses_before_admission() {
    let source = synthetic_with_grid(
        6,
        false,
        Some(Composition::Composited),
        AuxiliaryGrid::Unknown,
    );
    let pool = StorageBudget::new(1 << 20, 16);
    let error = project(&source, None, &pool, Some(&zencodec::NoPulse)).unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<Refusal>(error.as_ref()),
        Some(Refusal::AuxiliaryGrid {
            index: 1,
            registration: AuxiliaryGrid::Unknown,
            ..
        })
    ));
    assert_eq!(pool.reserved_bytes(), 0);
    assert_eq!(pool.reserved_objects(), 0);
}

#[test]
fn canvas_projection_strided_auxiliaries_cancel_and_admission_release_without_fallback() {
    struct PanicAt(CheckStop);
    impl zencodec::enough::Stop for PanicAt {
        fn check(&self) -> Result<(), zencodec::enough::StopReason> {
            self.0
                .check()
                .expect("controlled projection callback unwind");
            Ok(())
        }
    }
    let (mut projections, mut stops, mut unwinds, mut storage_refusals) = (0, 0, 0, 0);
    for exif in 1..=8 {
        let source = synthetic(exif, false, Some(Composition::Composited));
        projections += qualify(&source);
        let counted = CheckStop::default();
        let pool = StorageBudget::new(1024 * 1024, 4);
        let output = project(
            &source,
            None,
            &pool,
            Some(&zencodec::how_far::StopOnly::borrowed(&counted)),
        )
        .unwrap();
        let bytes = pool.reserved_bytes();
        drop(output);
        projections += 1;
        for reason in [
            zencodec::enough::StopReason::Cancelled,
            zencodec::enough::StopReason::TimedOut,
        ] {
            for at in 0..counted.calls() {
                let stop = CheckStop::with_reason(reason);
                stop.after(at);
                let error = project(
                    &source,
                    None,
                    &pool,
                    Some(&zencodec::how_far::StopOnly::borrowed(&stop)),
                )
                .unwrap_err();
                assert_eq!(
                    zencodec::find_cause::<CodecError>(&*error)
                        .unwrap()
                        .category(),
                    zencodec::ErrorCategory::Stopped(reason)
                );
                assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
                stops += 1;
            }
        }
        for at in [2, counted.calls() - 1] {
            let stop = CheckStop::default();
            stop.after(at);
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| project(
                    &source,
                    None,
                    &pool,
                    Some(&zencodec::how_far::StopOnly::borrowed(&PanicAt(stop)))
                )))
                .is_err()
            );
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
            unwinds += 1;
        }
        for objects in 0..4 {
            let pool = StorageBudget::new(1024 * 1024, objects);
            let error = project(&source, None, &pool, Some(&NoPulse)).unwrap_err();
            assert!(zencodec::find_cause::<StorageError>(&*error).is_some());
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
            storage_refusals += 1;
        }
        let limited = StorageBudget::new(bytes - 1, 4);
        let error = project(&source, None, &limited, Some(&NoPulse)).unwrap_err();
        assert!(matches!(
            zencodec::find_cause::<StorageError>(&*error),
            Some(StorageError::Bytes { .. })
        ));
        assert_eq!(
            (limited.reserved_bytes(), limited.reserved_objects()),
            (0, 0)
        );
        storage_refusals += 1;
    }
    let pool = StorageBudget::new(0, 0);
    let mut interpretation_refusals = 0;
    for composition in [None, Some(Composition::Uncomposited)] {
        let source = synthetic(1, false, composition);
        let error = project(&source, None, &pool, Some(&NoPulse)).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<Refusal>(&*error),
            Some(&Refusal::Composition(composition))
        );
        interpretation_refusals += 1;
    }
    let source = synthetic(6, true, Some(Composition::Composited));
    let error = project(&source, None, &pool, Some(&NoPulse)).unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<Refusal>(&*error),
        Some(Refusal::AuxiliaryGrid { index: 3, .. })
    ));
    interpretation_refusals += 1;
    let mut placed =
        ImageInterpretation::packed(source.interpretation().geometry, Orientation::Identity);
    placed.composition = Some(Composition::Composited);
    placed.placement = Some(zencodec::image::Placement {
        canvas: (7, 5),
        origin: (-1, 0),
    });
    let placed = ImageOutput::packed(
        PixelBuffer::new(7, 5, PixelDescriptor::GRAY8),
        vec![],
        placed,
    )
    .unwrap();
    let error = project(&placed, None, &pool, Some(&NoPulse)).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<Refusal>(&*error),
        Some(&Refusal::Placement)
    );
    interpretation_refusals += 1;
    let source = synthetic(1, false, Some(Composition::Composited));
    for region in [
        Rect {
            x: u32::MAX,
            y: 0,
            width: 1,
            height: 1,
        },
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
        },
        Rect {
            x: 6,
            y: 0,
            width: 2,
            height: 1,
        },
    ] {
        let error = project(&source, Some(region), &pool, Some(&NoPulse)).unwrap_err();
        assert!(zencodec::find_cause::<zencodec::image::GeometryError>(&*error).is_some());
        interpretation_refusals += 1;
    }
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    receipt(
        "boundaries",
        serde_json::json!({"projections":projections,"stop_refusals":stops,"unwinds":unwinds,"storage_refusals":storage_refusals,"interpretation_or_geometry_refusals":interpretation_refusals,"roles":["premultiplied color","selection mask","applied spot","depth"],"storage_channels":["U8","F32","F64"],"padded_rows_unchanged":true,"unpublished_charges_released":true}),
    );
}
