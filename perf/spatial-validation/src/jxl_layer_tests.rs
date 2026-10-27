use super::{
    checked_collection::{Checked, Stage},
    collection::Collection,
    control::CheckStop,
    image_items::{Composition, ImageRequest, Rect, SelectionError},
    jxl_items::{Limits, Plane, sequential},
    jxl_layers::JxlLayers,
    jxl_output::{JxlRawLayer, JxlSourceFacts},
    jxl_tests::fixture,
    output_tests::mapped_planes,
};
use std::{cell::Cell, convert::Infallible, path::PathBuf};
use zencodec::{
    At, SharedPulse,
    operation::{Decision, PolicySet, Question},
};
use zenjxl_decoder::api::{ExtraChannel, JxlLayerBlend, JxlLayerBlendMode};

fn limits(images: usize) -> Limits {
    Limits {
        images,
        frame_headers: Some(64),
    }
}
fn open(bytes: &[u8]) -> JxlLayers<'_> {
    JxlLayers::open(bytes, limits(64), SharedPulse::default()).unwrap()
}
fn ids(collection: &mut JxlLayers<'_>) -> Vec<super::image_items::ImageId> {
    let mut cursor = collection.images().unwrap();
    let mut ids = Vec::new();
    while let Some(id) = collection.next(&mut cursor).unwrap() {
        ids.push(id);
    }
    ids
}

fn oracle(name: &str, bytes: &[u8]) -> (PathBuf, serde_json::Value) {
    let root = PathBuf::from(
        std::env::var_os("SPATIAL_LAYER_ARTIFACTS").expect("set SPATIAL_LAYER_ARTIFACTS"),
    );
    std::fs::create_dir_all(&root).unwrap();
    let output = root.join(format!("layers-{}", name.replace('/', "_")));
    std::fs::create_dir(&output).expect("fresh layer evidence directory");
    let input = output.join("source.jxl");
    std::fs::write(&input, bytes).unwrap();
    let executable = std::env::var_os("JXL_LAYER_ORACLE").expect("set JXL_LAYER_ORACLE");
    let run = std::process::Command::new("timeout")
        .args(["--kill-after=2s", "30s"])
        .arg(executable)
        .arg(input)
        .arg(&output)
        .output()
        .unwrap();
    std::fs::write(output.join("stdout.json"), &run.stdout).unwrap();
    std::fs::write(output.join("stderr.txt"), &run.stderr).unwrap();
    assert!(
        run.status.success(),
        "independent layer decode: {:?}",
        run.status
    );
    let facts = serde_json::from_slice(&run.stdout).unwrap();
    (output, facts)
}
fn blend(value: &JxlLayerBlend) -> serde_json::Value {
    serde_json::json!({
        "mode": match value.mode {
            JxlLayerBlendMode::Replace => 0, JxlLayerBlendMode::Add => 1,
            JxlLayerBlendMode::Blend => 2, JxlLayerBlendMode::AlphaWeightedAdd => 3,
            JxlLayerBlendMode::Multiply => 4, _ => panic!("unqualified blend mode"),
        },
        "source": value.source, "alpha": value.alpha_channel, "clamp": u32::from(value.clamp),
    })
}
fn sample(plane: &Plane, x: usize, y: usize, channel: usize) -> u32 {
    let offset = y * plane.stride + (x * plane.channels + channel) * 4;
    u32::from_ne_bytes(plane.bytes[offset..offset + 4].try_into().unwrap())
}
fn compare_independent(actual: u32, expected: u32, exact: bool) -> Result<f32, &'static str> {
    let (a, b) = (f32::from_bits(actual), f32::from_bits(expected));
    if !a.is_finite() || !b.is_finite() {
        return Err("nonfinite fixture sample");
    }
    if exact && actual != expected {
        return Err("lossless sample changed");
    }
    let error = (a - b).abs();
    // Existing native F32 conformance gate, retained for independent lossy RGB.
    // Native cross-path, CMYK and alpha comparison below always requires exact bits.
    if error > 1.0 / 256.0 {
        return Err("native F32 conformance gate exceeded");
    }
    Ok(error)
}

#[test]
fn raw_collection_native_samples_and_owner_bound_policy_match_independent_layers() {
    use super::jxl_packaged_storage::{
        assert_charges, factories as admitted_factories, open_admitted,
    };
    for (name, count) in [
        ("multiple_layers_noise_spline.jxl", 5),
        ("conformance_test_images/cmyk_layers.jxl", 4),
    ] {
        let bytes = fixture(name);
        let (directory, independent) = oracle(name, &bytes);
        let oracle_version = independent["libjxl"].as_u64().unwrap();
        let expected = sequential(&bytes, false).unwrap();
        let factories = zencodec::CodecSet::new()
            .with_collection(zenjxl::collection::JxlCollectionConfig::new(64, 64));
        let mut source = zencodec::source::ReadAtShim(zencodec::source::SliceReadAt::new(&bytes));
        let mut packaged = factories
            .open_collection(
                zencodec::ImageFormat::Jxl,
                &mut source,
                zencodec::collection::OpenRequest::new(Composition::Uncomposited, 128),
                PolicySet::new(),
                SharedPulse::default(),
            )
            .unwrap();
        let mut package_cursor = packaged.images().unwrap();
        let mut package_ids = Vec::new();
        while let Some(id) = packaged.next(&mut package_cursor).unwrap() {
            package_ids.push(id);
        }
        assert_eq!(package_ids.len(), count);
        let (factories, storage) = admitted_factories(64, 64);
        let mut admitted_source =
            zencodec::source::ReadAtShim(zencodec::source::SliceReadAt::new(&bytes));
        let mut admitted = open_admitted(
            &factories,
            &mut admitted_source,
            Composition::Uncomposited,
            128,
            &storage,
        )
        .unwrap();
        let mut cursor = admitted.images().unwrap();
        let mut admitted_ids = Vec::new();
        while let Some(id) = admitted.next(&mut cursor).unwrap() {
            admitted_ids.push(id);
        }
        assert_eq!(admitted_ids.len(), count);
        assert_charges(&storage, &[]);
        let visits = Cell::new(0);
        let address = Cell::new(0usize);
        let resolver = |question: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Image { image, facts, .. } = question else {
                panic!("wrong question")
            };
            let layer = image
                .interpretation()
                .source_details
                .get::<JxlRawLayer>()
                .unwrap();
            assert_eq!(facts.geometry.canvas_width as usize, layer.native.size.0);
            assert_eq!(facts.placement, image.interpretation().placement);
            assert_eq!(facts.composition, Some(Composition::Uncomposited));
            assert_eq!(
                facts.placement.unwrap().origin,
                (
                    i64::from(layer.native.origin.0),
                    i64::from(layer.native.origin.1)
                )
            );
            assert_eq!(facts.source_details, 2);
            address.set(image.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap());
            visits.set(visits.get() + 1);
            Ok(Decision::UsePreset)
        };
        let mut collection = Checked::open(SharedPulse::default(), resolver, |stop| {
            JxlLayers::open(&bytes, limits(64), stop)
        })
        .unwrap();
        let mut cursor = collection.images().unwrap();
        let mut images = Vec::new();
        while let Some(id) = collection.next(&mut cursor).unwrap() {
            images.push(id);
        }
        assert_eq!(images.len(), count);
        assert_eq!(
            images.len(),
            independent["layers"].as_array().unwrap().len()
        );
        let mut reports = Vec::new();
        let mut held = None;
        let mut package_held = None;
        let mut admitted_held: Option<zencodec::image::ImageOutput> = None;
        for index in (0..images.len()).rev().chain([0]) {
            let image = collection
                .decode(&images[index], &ImageRequest::default())
                .unwrap();
            let package_image = packaged
                .decode(&package_ids[index], &ImageRequest::new())
                .unwrap();
            let admitted_image = admitted
                .decode(&admitted_ids[index], &ImageRequest::new())
                .unwrap()
                .into_output();
            for output in [&*image, &*package_image, &admitted_image] {
                assert_eq!(
                    output.interpretation().composition,
                    Some(Composition::Uncomposited)
                );
            }
            assert_eq!(
                mapped_planes(&admitted_image),
                mapped_planes(&package_image)
            );
            assert_eq!(
                admitted_image.interpretation().placement,
                package_image.interpretation().placement
            );
            if let Some(previous) = admitted_held.as_ref() {
                assert_charges(&storage, &[previous, &admitted_image]);
            } else {
                assert_charges(&storage, &[&admitted_image]);
            }
            assert_eq!(mapped_planes(&package_image), mapped_planes(&image));
            assert_eq!(
                package_image.interpretation().placement,
                image.interpretation().placement
            );
            assert_eq!(
                package_image.interpretation().geometry,
                image.interpretation().geometry
            );
            let package_layer = package_image
                .interpretation()
                .source_details
                .get::<zenjxl::collection::JxlRawLayer>()
                .unwrap();
            assert_eq!(package_layer.image, package_ids[index]);
            assert_eq!(
                image.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap(),
                address.get()
            );
            let interpretation = image.interpretation();
            let layer = interpretation.source_details.get::<JxlRawLayer>().unwrap();
            assert_eq!(package_layer.native, layer.native);
            let source = interpretation
                .source_details
                .get::<JxlSourceFacts>()
                .unwrap();
            assert_eq!(layer.image, images[index]);
            let oracle = &independent["layers"][index];
            assert_eq!(
                serde_json::json!(package_layer.native.origin),
                oracle["origin"]
            );
            assert_eq!(serde_json::json!(package_layer.native.size), oracle["size"]);
            assert_eq!(blend(&package_layer.native.color_blend), oracle["blend"]);
            assert_eq!(
                package_layer.native.duration_ticks,
                oracle["duration"].as_u64().unwrap() as u32
            );
            assert_eq!(serde_json::json!(layer.native.origin), oracle["origin"]);
            assert_eq!(serde_json::json!(layer.native.size), oracle["size"]);
            assert_eq!(blend(&layer.native.color_blend), oracle["blend"]);
            assert_eq!(
                serde_json::json!(
                    layer
                        .native
                        .extra_channel_blends
                        .iter()
                        .map(blend)
                        .collect::<Vec<_>>()
                ),
                oracle["extra_blends"]
            );
            assert_eq!(
                layer.native.saved_reference.unwrap_or(0),
                oracle["save_as_reference"].as_u64().unwrap() as u32
            );
            assert_eq!(
                layer.native.duration_ticks,
                oracle["duration"].as_u64().unwrap() as u32
            );
            assert_eq!(
                u32::from(layer.native.is_last),
                oracle["is_last"].as_u64().unwrap() as u32
            );
            let placement = interpretation.placement.unwrap();
            assert_eq!(serde_json::json!(placement.origin), oracle["origin"]);
            let overlap = placement.visible(interpretation.geometry).unwrap().unwrap();
            if name == "multiple_layers_noise_spline.jxl" && index == 4 {
                assert_eq!((overlap.canvas.width, overlap.canvas.height), (188, 202));
                assert_eq!(layer.native.size, (1024, 576));
            }
            let black = source
                .header
                .extra_channels
                .iter()
                .position(|e| e.ec_type == ExtraChannel::Black);
            let alpha = source
                .header
                .extra_channels
                .iter()
                .position(|e| e.ec_type == ExtraChannel::Alpha);
            image
                .with_pixels(|pixels| {
                    assert_eq!(
                        pixels.descriptor().layout(),
                        if black.is_some() {
                            zenpixels::ChannelLayout::Cmyk
                        } else {
                            zenpixels::ChannelLayout::Rgba
                        }
                    );
                    assert!(pixels.color_context().is_some());
                    if black.is_some() {
                        assert_eq!(
                            pixels.descriptor().format,
                            zenpixels::PixelFormat::CmykF32Inverted
                        );
                        assert!(
                            zenpixels_convert::RowConverter::new(
                                pixels.descriptor(),
                                zenpixels::PixelFormat::CmykF32.descriptor()
                            )
                            .is_err(),
                            "no silent polarity conversion"
                        );
                    }
                })
                .unwrap();
            if let Some(black) = black {
                let mapping = image.map().unwrap();
                let plane = mapping.plane(black + 1).unwrap().unwrap();
                assert!(matches!(
                    plane.meaning,
                    zencodec::image::Meaning::Auxiliary(zencodec::image::Auxiliary {
                        kind: zencodec::image::AuxiliaryKind::Black { inverted: true },
                        ..
                    })
                ));
            }
            // The independent sample checks exercise the packaged output.
            // Above, the private adapter remains a bit-exact cross-path check.
            let mapped = mapped_planes(&package_image);
            let mut samples = 0u64;
            let mut different = 0u64;
            let mut max_error = 0f32;
            for (plane_index, plane) in mapped.iter().enumerate() {
                assert_eq!((plane.width, plane.height), layer.native.size);
                for channel in 0..plane.channels {
                    let (native_plane, native_channel, oracle_channel) = if plane_index != 0 {
                        (plane_index, 0, plane_index + 2)
                    } else if channel < 3 {
                        (0, channel, channel)
                    } else {
                        let extra = black.or(alpha).unwrap();
                        (extra + 1, 0, extra + 3)
                    };
                    let file = oracle["planes"][oracle_channel].as_str().unwrap();
                    let reference = std::fs::read(directory.join(file)).unwrap();
                    assert_eq!(reference.len(), plane.width * plane.height * 4);
                    let exact = black.is_some() || alpha == oracle_channel.checked_sub(3);
                    for y in 0..plane.height {
                        for x in 0..plane.width {
                            let actual = sample(plane, x, y, channel);
                            assert_eq!(
                                actual,
                                sample(&expected[index][native_plane], x, y, native_channel),
                                "native replay changed a sample in {name} layer {index}"
                            );
                            let offset = (y * plane.width + x) * 4;
                            let independent = u32::from_le_bytes(
                                reference[offset..offset + 4].try_into().unwrap(),
                            );
                            let error = compare_independent(actual, independent, exact)
                                .unwrap_or_else(|error| {
                                    panic!(
                                        "{error}: {name} layer {index} plane {plane_index} channel {channel} at ({x},{y}); native {} ({actual:08x}), libjxl {oracle_version} {} ({independent:08x})",
                                        f32::from_bits(actual),
                                        f32::from_bits(independent),
                                    )
                                });
                            max_error = max_error.max(error);
                            different += u64::from(actual != independent);
                            samples += 1;
                        }
                    }
                }
                for y in 0..plane.height {
                    let active = plane.width * plane.channels * 4;
                    let end = ((y + 1) * plane.stride).min(plane.bytes.len());
                    assert!(
                        plane.bytes[y * plane.stride + active..end]
                            .iter()
                            .all(|b| *b == 0xa5)
                    );
                }
            }
            let mut detached = interpretation.clone();
            detached.placement = None;
            assert_eq!(image.interpretation().placement, Some(placement));
            detached.source_details.remove::<JxlRawLayer>();
            assert!(
                image
                    .interpretation()
                    .source_details
                    .contains::<JxlRawLayer>()
            );
            reports.push(serde_json::json!({"layer":index,"samples":samples,"different_float_bits":different,"max_absolute_error":max_error}));
            held = Some(image);
            package_held = Some(package_image);
            admitted_held = Some(admitted_image);
        }
        assert_eq!(visits.get(), count + 1);
        drop(collection);
        drop(packaged);
        drop(admitted);
        drop(bytes);
        let admitted_held = admitted_held.unwrap();
        assert_charges(&storage, &[&admitted_held]);
        assert_eq!(
            mapped_planes(&admitted_held),
            mapped_planes(package_held.as_ref().unwrap())
        );
        drop(admitted_held);
        assert_charges(&storage, &[]);
        let package_held = package_held.unwrap();
        assert!(package_held.map().unwrap().plane_count() >= 2);
        assert!(
            package_held
                .interpretation()
                .source_details
                .contains::<zenjxl::collection::JxlRawLayer>()
        );
        let held = held.unwrap();
        assert!(held.map().unwrap().plane_count() >= 2);
        assert!(
            held.interpretation()
                .source_details
                .contains::<JxlRawLayer>()
        );
        std::fs::write(
            directory.join("comparison.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
    }
    assert!(compare_independent(1.0f32.to_bits(), 1.0f32.to_bits() + 1, true).is_err());
}

#[test]
fn raw_collection_identity_limits_and_refusals_do_not_become_exhaustion() {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let mut collection = open(&bytes);
    assert_eq!(collection.view(), Composition::Uncomposited);
    let images = ids(&mut collection);
    assert_eq!(images.len(), 4);
    let mut other = open(&bytes);
    let err = other
        .decode(&images[0], &ImageRequest::default())
        .unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&err),
        Some(&SelectionError::ForeignImage)
    );
    let mut cursor = collection.images().unwrap();
    let err = other.next(&mut cursor).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&err),
        Some(&SelectionError::ForeignCursor)
    );
    assert!(collection.children(&images[0]).is_err());
    let request = ImageRequest::new().with_region(Rect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    });
    assert!(collection.decode(&images[0], &request).is_err());
    collection
        .decode(&images[0], &ImageRequest::default())
        .unwrap();
    let mut bounded = JxlLayers::open(&bytes, limits(1), SharedPulse::default()).unwrap();
    let mut cursor = bounded.images().unwrap();
    bounded.next(&mut cursor).unwrap().unwrap();
    for _ in 0..2 {
        let error = bounded.next(&mut cursor).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<SelectionError>(&error),
            Some(&SelectionError::DiscoveryLimit)
        );
    }
    let mut native_bounded = JxlLayers::open(
        &bytes,
        Limits {
            images: 64,
            frame_headers: Some(1),
        },
        SharedPulse::default(),
    )
    .unwrap();
    let mut cursor = native_bounded.images().unwrap();
    native_bounded.next(&mut cursor).unwrap().unwrap();
    for _ in 0..2 {
        let error = native_bounded.next(&mut cursor).unwrap_err();
        assert!(matches!(
            zencodec::find_cause::<zenjxl_decoder::api::Error>(&error),
            Some(zenjxl_decoder::api::Error::LimitExceeded {
                resource: "frame_headers",
                ..
            })
        ));
    }
}

#[test]
fn raw_collection_policy_and_stop_prevent_unapproved_output() {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let mut checked = Checked::open(
        SharedPulse::default(),
        PolicySet::new().allow_opaque_profiles(false),
        |stop| JxlLayers::open(&bytes, limits(64), stop),
    )
    .unwrap();
    let error = checked.first_image(&ImageRequest::default()).unwrap_err();
    assert_eq!(error.error().stage, Stage::Policy);
    for reason in [
        zencodec::enough::StopReason::TimedOut,
        zencodec::enough::StopReason::Cancelled,
    ] {
        let stop = CheckStop::with_reason(reason);
        let mut collection = JxlLayers::open(&bytes, limits(64), stop.token()).unwrap();
        let ids = ids(&mut collection);
        stop.after(0);
        let error = collection
            .decode(&ids[3], &ImageRequest::default())
            .unwrap_err();
        let cause = zencodec::find_cause::<zencodec::CodecError>(&error).unwrap();
        assert_eq!(cause.category(), zencodec::ErrorCategory::Stopped(reason));
        assert!(collection.images().is_err());
    }
}
