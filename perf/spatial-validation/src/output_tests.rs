use crate::{
    image_items::Composition,
    image_output::{
        AuxiliaryKind, ColorContribution, ImageOutput, Meaning, Row, Samples, SourcePrecision,
    },
    jxl_items::{JxlItems, sequential},
    jxl_tests::fixture,
};
use zenjxl_decoder::api::{JxlDecoder, JxlDecoderOptions, ProcessingResult};
use zenpixels::{AlphaMode, ChannelType, Orientation};

pub(crate) fn expected_output(data: &[u8]) -> Vec<Vec<crate::jxl_items::Plane>> {
    let mut input = data;
    let ProcessingResult::Complete { result: header } =
        JxlDecoder::new(JxlDecoderOptions::default().with_adjust_orientation(false))
            .process(&mut input)
            .unwrap()
    else {
        panic!("incomplete fixture")
    };
    let mut frames = sequential(data, true).unwrap();
    // The native default returns separate alpha. The consumer path explicitly
    // requests interleaving; its exact expectation is native color + native
    // alpha bytes, with no arithmetic or normalization in this oracle.
    if !header.current_pixel_format().color_type.has_alpha()
        && let Some(alpha_index) = header
            .basic_info()
            .extra_channels
            .iter()
            .position(|ec| ec.ec_type == zenjxl_decoder::api::ExtraChannel::Alpha)
    {
        for frame in &mut frames {
            let color = &frame[0];
            let alpha = &frame[alpha_index + 1];
            let channels = color.channels + 1;
            let stride = color.width * channels * 4;
            let mut bytes = Vec::with_capacity(stride * color.height);
            for y in 0..color.height {
                for x in 0..color.width {
                    let start = y * color.stride + x * color.channels * 4;
                    bytes.extend_from_slice(&color.bytes[start..start + color.channels * 4]);
                    let start = y * alpha.stride + x * 4;
                    bytes.extend_from_slice(&alpha.bytes[start..start + 4]);
                }
            }
            frame[0] = crate::jxl_items::Plane {
                width: color.width,
                height: color.height,
                channels,
                stride,
                bytes,
            };
        }
    }
    frames
}

pub(crate) fn mapped_planes(image: &ImageOutput) -> Vec<crate::jxl_items::Plane> {
    let mapped = image.map().unwrap();
    (0..mapped.plane_count())
        .map(|index| {
            let plane = mapped.plane(index).unwrap().unwrap();
            let (width, height) = plane.samples.dimensions();
            let stride = plane.samples.stride_bytes();
            let (channels, bytes) = match plane.samples {
                Samples::Pixels(p) => (
                    p.descriptor().layout().channels(),
                    p.as_strided_bytes().to_vec(),
                ),
                Samples::Values(v) => (1, v.as_strided_bytes().to_vec()),
                Samples::Codes(_) => panic!("JXL f32 output became unsigned codes"),
            };
            crate::jxl_items::Plane {
                width,
                height,
                channels,
                stride,
                bytes,
            }
        })
        .collect()
}

#[test]
fn output_jxl_color_and_extra_planes_keep_native_bytes_roles_and_geometry() {
    for name in [
        "conformance_test_images/alpha_nonpremultiplied.jxl",
        "conformance_test_images/alpha_premultiplied.jxl",
        "conformance_test_images/spot.jxl",
        "extra_channels.jxl",
        "gray_alpha_lossless.jxl",
        "upsampled_alpha.jxl",
        "orientation6_rotate_90_cw.jxl",
        "small_grayscale_patches_modular_with_icc.jxl",
    ] {
        let data = fixture(name);
        let mut input = data.as_slice();
        let ProcessingResult::Complete { result: header } =
            JxlDecoder::new(JxlDecoderOptions::default().with_adjust_orientation(false))
                .process(&mut input)
                .unwrap()
        else {
            panic!("incomplete fixture header {name}")
        };
        let info = header.basic_info().clone();
        let independent = crate::jxl_tests::independent_metadata(name, true);
        assert!(
            independent
                .lines()
                .any(|line| line == format!("orientation {}", info.orientation as u8))
        );
        let independent_extras: Vec<Vec<u32>> = independent
            .lines()
            .filter_map(|line| line.strip_prefix("extra "))
            .map(|line| {
                line.split_whitespace()
                    .map(|v| v.parse().unwrap())
                    .collect()
            })
            .collect();
        assert_eq!(independent_extras.len(), info.extra_channels.len());
        let independent_details: Vec<Vec<u32>> = independent
            .lines()
            .filter_map(|line| line.strip_prefix("extra_details "))
            .map(|line| {
                line.split_whitespace()
                    .map(|v| v.parse().unwrap())
                    .collect()
            })
            .collect();
        assert_eq!(independent_details.len(), info.extra_channels.len());
        let expected = expected_output(&data);
        let mut items = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
        let selected = items.next().unwrap().unwrap();
        let output = items
            .decode_image(&selected.image)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        drop(items);
        drop(data);
        let interpretation_address = output.interpretation() as *const _;
        let retained = output.clone();
        drop(output);
        assert_eq!(
            retained.interpretation() as *const _,
            interpretation_address
        );
        let mut detached_facts = retained.interpretation().clone();
        detached_facts.orientation = Some(Orientation::Identity);
        detached_facts.geometry.region.width = 0;
        assert_ne!(retained.interpretation().geometry.region.width, 0);
        assert_eq!(
            retained.interpretation().orientation,
            Orientation::from_exif(info.orientation as u8)
        );
        assert_eq!(
            (
                retained.interpretation().geometry.canvas_width as usize,
                retained.interpretation().geometry.canvas_height as usize
            ),
            info.coded_size
        );
        let mapped = retained.map().unwrap();
        assert_eq!(mapped.plane_count(), expected[0].len(), "{name}");
        assert_eq!(mapped.plane_count(), 1 + info.extra_channels.len());
        let expected_channels = expected[0][0].channels;
        for (index, expected) in expected[0].iter().enumerate() {
            let plane = mapped.plane(index).unwrap().unwrap();
            assert_eq!(
                plane.samples.dimensions(),
                (expected.width, expected.height)
            );
            assert_eq!(plane.samples.channel_type(), ChannelType::F32);
            assert!(plane.samples.code_encoding().is_none());
            assert_eq!(
                plane.samples.stride_bytes(),
                expected.width * expected.channels * 4 + expected_channels * 4
            );
            for y in 0..expected.height {
                let Row::Bytes(row) = plane.samples.row(y).unwrap() else {
                    panic!("f32 output became integer words")
                };
                assert_eq!(
                    row,
                    &expected.bytes[y * expected.stride
                        ..y * expected.stride + expected.width * expected.channels * 4],
                    "{name} plane={index} row={y}"
                );
            }
            assert!(plane.samples.row(expected.height).is_none());
            if index == 0 {
                assert_eq!(plane.meaning, &Meaning::Color);
                let Samples::Pixels(pixels) = plane.samples else {
                    panic!("color plane is not a pixel view")
                };
                assert!(pixels.color_context().unwrap().icc.is_some());
                if pixels.descriptor().layout().has_alpha() {
                    let alpha = info
                        .extra_channels
                        .iter()
                        .find(|c| c.ec_type == zenjxl_decoder::api::ExtraChannel::Alpha)
                        .unwrap();
                    assert_eq!(
                        pixels.descriptor().alpha(),
                        Some(if alpha.alpha_associated {
                            AlphaMode::Premultiplied
                        } else {
                            AlphaMode::Straight
                        })
                    );
                }
            } else {
                assert!(
                    matches!(plane.samples, Samples::Values(_)),
                    "auxiliary samples must not become gray pixels"
                );
                let Meaning::Auxiliary(role) = plane.meaning else {
                    panic!("extra channel lost its role")
                };
                let native = &info.extra_channels[index - 1];
                let facts = &independent_extras[index - 1];
                let role_code = match role.kind {
                    AuxiliaryKind::Alpha(mode) => {
                        assert_eq!(mode == AlphaMode::Premultiplied, facts[4] == 1);
                        0
                    }
                    AuxiliaryKind::Depth => 1,
                    AuxiliaryKind::SpotColor {
                        linear_rgba,
                        contribution,
                    } => {
                        assert_eq!(contribution, ColorContribution::Applied);
                        assert_eq!(
                            linear_rgba.map(f32::to_bits).as_slice(),
                            &independent_details[index - 1][5..9]
                        );
                        2
                    }
                    AuxiliaryKind::SelectionMask => 3,
                    AuxiliaryKind::Black { inverted: true } => 4,
                    AuxiliaryKind::Cfa { channel } => {
                        assert_eq!(channel, independent_details[index - 1][9]);
                        5
                    }
                    AuxiliaryKind::Thermal => 6,
                    AuxiliaryKind::Unknown(code) => code,
                    _ => panic!("unqualified auxiliary role in the native fixture matrix"),
                };
                assert_eq!(
                    role_code, facts[1],
                    "{name}: common role disagrees with libjxl"
                );
                assert_eq!(
                    role.source_precision,
                    SourcePrecision::Integer {
                        bits: facts[2],
                        signed: false
                    }
                );
                assert_eq!(independent_details[index - 1][3], 0);
                assert_eq!(role.source_dim_shift, facts[3]);
                assert_eq!(role.name, native.name);
                assert_eq!(
                    role.source_precision,
                    SourcePrecision::Integer {
                        bits: native.bits_per_sample,
                        signed: false
                    }
                );
                assert_eq!(role.source_dim_shift, native.dim_shift);
            }
        }
        assert!(mapped.plane(mapped.plane_count()).unwrap().is_none());
        // The ordinary packed-color consumer uses the same shared output type
        // as TIFF. Every value and the declared source color survive this view.
        check_color(&retained, &expected[0][0]);
    }
}

fn check_color(image: &ImageOutput, expected: &crate::jxl_items::Plane) {
    image
        .with_pixels(|pixels| {
            let mut converter =
                zenpixels_convert::RowConverter::new(pixels.descriptor(), pixels.descriptor())
                    .unwrap();
            let bytes = pixels.width() as usize * pixels.descriptor().bytes_per_pixel();
            let mut row = vec![0xa5; bytes + 13];
            for y in 0..pixels.rows() {
                converter
                    .try_convert_row(pixels.row(y), &mut row, pixels.width())
                    .unwrap();
                assert_eq!(
                    &row[..bytes],
                    &expected.bytes
                        [y as usize * expected.stride..y as usize * expected.stride + bytes]
                );
                assert_eq!(&row[bytes..], &[0xa5; 13]);
            }
        })
        .unwrap();
}

#[test]
fn output_jxl_selected_timing_keeps_exact_independent_ticks() {
    let name = "conformance_test_images/animation_newtons_cradle.jxl";
    let data = fixture(name);
    let independent = crate::jxl_tests::independent_metadata(name, true);
    let animation: Vec<u32> = independent
        .lines()
        .find_map(|line| line.strip_prefix("animation "))
        .unwrap()
        .split_whitespace()
        .map(|v| v.parse().unwrap())
        .collect();
    let durations: Vec<u64> = independent
        .lines()
        .filter_map(|line| line.strip_prefix("frame "))
        .map(|line| line.split_whitespace().next().unwrap().parse().unwrap())
        .collect();
    let base = zencodec::time::TimeBase::new(animation[1], animation[0]).unwrap();
    let mut items = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
    let mut ids = Vec::new();
    while let Some(item) = items.next().unwrap() {
        ids.push(item.image);
    }
    assert_eq!(ids.len(), durations.len());
    for index in [ids.len() - 1, 0, 3] {
        let output = items.decode_image(&ids[index]).unwrap();
        let expected_pts = zencodec::time::Timestamp::new(
            i64::try_from(durations[..index].iter().sum::<u64>()).unwrap(),
            base,
        );
        let timing = output.interpretation().timing.unwrap();
        assert_eq!(timing.pts, Some(expected_pts));
        assert_eq!(timing.duration_ticks, Some(durations[index]));
        assert_eq!(timing.packet_ordinal, None);
        assert_eq!(timing.input_offset, None);
        let source = output
            .interpretation()
            .source_details
            .get::<crate::jxl_output::JxlSourceFacts>()
            .unwrap();
        let actual = source.header.animation.as_ref().unwrap();
        assert_eq!(
            [
                actual.tps_numerator,
                actual.tps_denominator,
                actual.num_loops,
                u32::from(actual.have_timecodes),
            ]
            .as_slice(),
            animation.as_slice()
        );
        assert_eq!(source.frame_index, index);
        assert_eq!(source.is_last, index + 1 == ids.len());
    }
}

#[test]
fn output_jxl_source_facts_survive_with_independent_hdr_precision_and_display() {
    use zenjxl_decoder::api::JxlBitDepth;
    for name in [
        "hdr_hlg_test.jxl",
        "hdr_pq_test.jxl",
        "pq_gradient.jxl",
        "conformance_test_images/lossless_pfm.jxl",
        "with_preview.jxl",
        "small_grayscale_patches_modular_with_icc.jxl",
    ] {
        let data = fixture(name);
        let independent = crate::jxl_tests::independent_metadata(name, true);
        let fields = |prefix: &str| -> Vec<u32> {
            independent
                .lines()
                .find_map(|line| line.strip_prefix(prefix))
                .unwrap()
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect()
        };
        let mut input = data.as_slice();
        let ProcessingResult::Complete {
            result: native_header,
        } = JxlDecoder::new(JxlDecoderOptions::default().with_adjust_orientation(false))
            .process(&mut input)
            .unwrap()
        else {
            panic!("incomplete fixture header {name}")
        };
        let mut images = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
        let selected = images.next().unwrap().unwrap();
        let image = images.decode_image(&selected.image).unwrap();
        drop(images);
        drop(data);
        let source = image
            .interpretation()
            .source_details
            .get::<crate::jxl_output::JxlSourceFacts>()
            .unwrap();
        let info = &source.header;
        let (bits, exponent) = match info.bit_depth {
            JxlBitDepth::Int { bits_per_sample } => (bits_per_sample, 0),
            JxlBitDepth::Float {
                bits_per_sample,
                exponent_bits_per_sample,
            } => (bits_per_sample, exponent_bits_per_sample),
        };
        assert_eq!(vec![bits, exponent], fields("source_samples "), "{name}");
        assert_eq!(info.bit_depth, native_header.basic_info().bit_depth);
        let tone = &info.tone_mapping;
        assert_eq!(
            vec![
                tone.intensity_target.to_bits(),
                tone.min_nits.to_bits(),
                u32::from(tone.relative_to_max_display),
                tone.linear_below.to_bits(),
            ],
            fields("source_tone "),
            "{name}"
        );
        let display = info.intrinsic_size.unwrap_or(info.coded_size);
        assert_eq!(
            vec![display.0 as u32, display.1 as u32],
            fields("source_display "),
            "{name}"
        );
        assert_eq!(
            vec![u32::from(info.uses_original_profile)],
            fields("original_profile ")
        );
        let preview = fields("preview ");
        assert_eq!(
            info.preview_size,
            (preview[0] != 0).then_some((preview[1] as usize, preview[2] as usize))
        );
        assert_eq!(source.frame_name, selected.native.name);
        assert_eq!(source.frame_index, selected.native.index);
        assert_eq!(source.is_last, selected.native.is_last);
        assert_eq!(
            source.embedded_color,
            *native_header.embedded_color_profile()
        );
        image
            .with_pixels(|pixels| {
                assert_eq!(pixels.descriptor().channel_type(), ChannelType::F32);
                assert!(pixels.color_context().unwrap().icc.is_some());
            })
            .unwrap();
        // A consumer's mutable copy cannot strip facts from the owned output.
        let mut detached = image.interpretation().clone();
        assert!(
            detached
                .source_details
                .remove::<crate::jxl_output::JxlSourceFacts>()
                .is_none()
        );
        assert!(detached.source_details.is_empty());
        assert!(
            image
                .interpretation()
                .source_details
                .contains::<crate::jxl_output::JxlSourceFacts>()
        );
    }
}

#[test]
fn output_rejects_geometry_and_interpretation_that_disagree_with_packed_samples() {
    use crate::{
        image_items::{ImageGeometry, Rect},
        image_output::{GeometryError, ImageInterpretation, OutputError},
    };
    use zenpixels::{PixelBuffer, PixelDescriptor};
    let pixels = || PixelBuffer::from_vec(vec![17; 6], 2, 1, PixelDescriptor::RGB8).unwrap();
    let geometry = ImageGeometry {
        canvas_width: 10,
        canvas_height: 8,
        region: Rect {
            x: 3,
            y: 4,
            width: 2,
            height: 1,
        },
    };
    let facts = ImageInterpretation::packed(geometry, Orientation::Identity);
    let mut mismatch = facts.clone();
    mismatch.geometry.region.width = 3;
    let error = ImageOutput::packed(pixels(), vec![], mismatch).unwrap_err();
    assert!(matches!(error.error(), OutputError::Geometry));
    for region in [
        Rect {
            x: 9,
            ..geometry.region
        },
        Rect {
            width: 0,
            ..geometry.region
        },
        Rect {
            x: u32::MAX,
            ..geometry.region
        },
    ] {
        let mut outside = facts.clone();
        outside.geometry.region = region;
        let rejected = outside.geometry;
        let error = ImageOutput::packed(pixels(), vec![], outside).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<GeometryError>(&error)
                .unwrap()
                .geometry,
            rejected,
        );
    }
    let mut unspecified = facts.clone();
    unspecified.orientation = None;
    assert!(matches!(
        ImageOutput::packed(pixels(), vec![], unspecified)
            .unwrap_err()
            .error(),
        OutputError::Interpretation,
    ));
    let accepted = ImageOutput::packed(pixels(), vec![], facts).unwrap();
    assert_eq!(accepted.interpretation().geometry, geometry);
    assert_eq!(
        accepted.with_pixels(|p| p.row(0).to_vec()).unwrap(),
        vec![17; 6]
    );
}

#[test]
fn output_policy_native_spot_decision_uses_actual_applied_effects() {
    use zencodec::operation::{
        Decision, DecisionCode, DecisionContext, Decline, PolicySet, Question, resolve_image,
    };
    let data = fixture("conformance_test_images/spot.jxl");
    let mut collection = JxlItems::open(&data, Composition::Composited, 8).unwrap();
    let id = collection.next().unwrap().unwrap().image;
    let mut seen = 0;
    for render in [true, false, true, false] {
        let image = collection.decode_with_spot_colors(&id, render).unwrap();
        let expected = mapped_planes(&image);
        let address = image.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap();
        let result = resolve_image(image,
            &mut |question: Question<'_>| -> Result<Decision, zencodec::At<std::convert::Infallible>> {
                let Question::Image { image, facts, choices } = question else { panic!("wrong domain") };
                seen += 1;
                assert_eq!(facts.source_details, 1);
                assert_eq!(facts.planes, expected.len());
                assert_eq!(facts.auxiliaries, expected.len() - 1);
                assert_eq!(facts.applied_spot_colors > 0, render);
                assert_eq!(mapped_planes(image), expected);
                if facts.applied_spot_colors > 0 {
                    Ok(Decision::decline(Decline::ApplicationRule))
                } else {
                    Ok(Decision::choose(choices.first().unwrap()))
                }
            }, PolicySet::new(), &zencodec::NoPulse);
        if render {
            let error = result.unwrap_err();
            assert_eq!(error.error().record().code, DecisionCode::Declined);
            let DecisionContext::Image(facts) = error.error().record().context else {
                panic!("lost refusal facts")
            };
            assert!(facts.applied_spot_colors > 0);
            assert!(zencodec::find_cause::<Decline>(&error).is_some());
        } else {
            let output = result.unwrap();
            assert_eq!(mapped_planes(&output), expected);
            assert_eq!(
                output.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap(),
                address
            );
            assert_eq!(output.report().decisions().count(), 1);
        }
    }
    assert_eq!(seen, 4);
}
