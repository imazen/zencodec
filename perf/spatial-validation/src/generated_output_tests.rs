//! Native encoder fixtures qualify the selected-output boundary. libjxl checks
//! source interpretation; native sequential decode checks mapped sample bytes.
use crate::{
    collection::{Collection, first_image},
    image_items::{Composition, ImageRequest},
    image_output::{AuxiliaryKind, ColorContribution, ImageOutput, Meaning, Row, SourcePrecision},
    jxl_items::{JxlItems, Plane},
    jxl_output::JxlSourceFacts,
    jxl_tests::independent_inspect_mode,
    output_tests::expected_output,
};
use jxl_encoder::{
    AnimationFrame, AnimationParams, ImageMetadata, LosslessConfig, PixelLayout, api::ExtraChannel,
};
use std::{fs::OpenOptions, io::Write, path::PathBuf};
use zenpixels::{AlphaMode, ChannelType};

fn inspect(name: &str, encoded: &[u8]) -> String {
    inspect_mode(name, encoded, "full")
}

fn inspect_mode(name: &str, encoded: &[u8], mode: &str) -> String {
    let root = PathBuf::from(
        std::env::var_os("SPATIAL_GENERATED_ARTIFACTS")
            .expect("persistent generated artifact directory required"),
    );
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("{name}.jxl"));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap()
        .write_all(encoded)
        .unwrap();
    independent_inspect_mode(&path, true, Some(mode))
}

fn rows(text: &str, prefix: &str) -> Vec<Vec<u32>> {
    text.lines()
        .filter_map(|l| l.strip_prefix(prefix))
        .map(|l| l.split_whitespace().map(|x| x.parse().unwrap()).collect())
        .collect()
}

fn exact_samples(image: &ImageOutput, expected: &[Plane]) {
    let mapped = image.map().unwrap();
    assert_eq!(mapped.plane_count(), expected.len());
    for (index, expected) in expected.iter().enumerate() {
        let plane = mapped.plane(index).unwrap().unwrap();
        assert_eq!(
            plane.samples.dimensions(),
            (expected.width, expected.height)
        );
        assert_eq!(plane.samples.channel_type(), ChannelType::F32);
        for y in 0..expected.height {
            let Row::Bytes(actual) = plane.samples.row(y).unwrap() else {
                panic!("native f32 samples became integer codes")
            };
            assert_eq!(
                actual,
                &expected.bytes[y * expected.stride
                    ..y * expected.stride + expected.width * expected.channels * 4]
            );
        }
        assert!(plane.samples.row(expected.height).is_none());
    }
}

fn owned_first(encoded: Vec<u8>) -> (ImageOutput, Vec<Plane>) {
    let expected = expected_output(&encoded).remove(0);
    let mut collection = JxlItems::open(&encoded, Composition::Composited, 8).unwrap();
    let image = first_image(&mut collection, &ImageRequest::default())
        .unwrap()
        .unwrap();
    drop(collection);
    drop(encoded);
    let address = image.interpretation() as *const _;
    let retained = image.clone();
    drop(image);
    assert_eq!(retained.interpretation() as *const _, address);
    exact_samples(&retained, &expected);
    (retained, expected)
}

fn project_packaged_subsampled(encoded: &[u8], expected: &[Plane], shift: u32) {
    use zencodec::{
        image::{AuxiliaryGrid, Rect, Samples},
        source::{ReadAtShim, SliceReadAt},
        storage::StorageBudget,
    };
    let (codecs, source_pool) = crate::jxl_packaged_storage::factories(8, 8);
    let mut source = ReadAtShim(SliceReadAt::new(encoded));
    let mut images = crate::jxl_packaged_storage::open_admitted(
        &codecs,
        &mut source,
        Composition::Composited,
        8,
        &source_pool,
    )
    .unwrap();
    let image = images
        .first_image(&ImageRequest::new())
        .unwrap()
        .unwrap()
        .into_output();
    exact_samples(&image, expected);
    let region = Rect {
        x: 1,
        y: 3,
        width: 13,
        height: 9,
    };
    crate::canvas_render::tests::qualify_auxiliaries(&image, region);
    let destination_pool = StorageBudget::new(1 << 20, 8);
    let output = crate::canvas_projection::project(
        &image,
        Some(region),
        &destination_pool,
        Some(&zencodec::NoPulse),
    )
    .unwrap();
    let mapped = output.map().unwrap();
    assert_eq!(mapped.plane_count(), expected.len());
    for (index, original) in expected.iter().enumerate() {
        let plane = mapped.plane(index).unwrap().unwrap();
        assert_eq!(plane.samples.dimensions(), (13, 9));
        if let Meaning::Auxiliary(role) = plane.meaning {
            assert_eq!(role.source_dim_shift, shift);
            let Samples::Values(values) = &plane.samples else {
                panic!("auxiliary values lost")
            };
            assert_eq!(values.grid(), AuxiliaryGrid::ColorSamples);
        }
        for y in 0..9 {
            let Row::Bytes(actual) = plane.samples.row(y).unwrap() else {
                panic!("native samples lost")
            };
            let start = (y + 3) * original.stride + original.channels * 4;
            assert_eq!(
                actual,
                &original.bytes[start..start + 13 * original.channels * 4]
            );
        }
    }
    drop(mapped);
    drop(output);
    drop(image);
    assert_eq!(
        (source_pool.reserved_bytes(), source_pool.reserved_objects()),
        (0, 0)
    );
    assert_eq!(
        (
            destination_pool.reserved_bytes(),
            destination_pool.reserved_objects()
        ),
        (0, 0)
    );
}

#[test]
fn output_generated_all_seven_auxiliary_roles_preserve_parameters_and_source_hdr() {
    let color: Vec<u8> = (0..8 * 6 * 3).map(|v| (v * 37 + 11) as u8).collect();
    let samples: Vec<u8> = (0..8 * 6).map(|v| (v * 29 + 7) as u8).collect();
    let spot = [0.25, 0.5, 0.75, 1.0];
    for role in 0..7 {
        let extras = [match role {
            0 => ExtraChannel::from_alpha_buf(&samples, false),
            1 => ExtraChannel::depth(&samples),
            2 => ExtraChannel::spot_color(&samples, spot),
            3 => ExtraChannel::selection_mask(&samples),
            4 => ExtraChannel::black(&samples),
            5 => ExtraChannel::cfa(&samples, 2),
            6 => ExtraChannel::thermal(&samples),
            _ => unreachable!(),
        }];
        let metadata = ImageMetadata::new()
            .with_intrinsic_size(13, 9)
            .with_intensity_target(1000.0)
            .with_min_nits(0.125)
            .with_relative_to_max_display(true)
            .with_linear_below(0.25);
        let encoded = LosslessConfig::new()
            .with_effort(3)
            .encode_request(8, 6, PixelLayout::Rgb8)
            .with_extra_channels(&extras)
            .with_metadata(&metadata)
            .encode(&color)
            .unwrap();
        let independent = inspect(&format!("role-{role}"), &encoded);
        let (image, _) = owned_first(encoded);
        let mapped = image.map().unwrap();
        let auxiliary = mapped.plane(1).unwrap().unwrap();
        let Meaning::Auxiliary(aux) = auxiliary.meaning else {
            panic!("auxiliary role lost")
        };
        assert_eq!(
            aux.source_precision,
            SourcePrecision::Integer {
                bits: 8,
                signed: false
            }
        );
        assert_eq!(aux.source_dim_shift, 0);
        assert_eq!(
            aux.kind,
            match role {
                0 => AuxiliaryKind::Alpha(AlphaMode::Straight),
                1 => AuxiliaryKind::Depth,
                2 => AuxiliaryKind::SpotColor {
                    linear_rgba: spot,
                    contribution: ColorContribution::Applied
                },
                3 => AuxiliaryKind::SelectionMask,
                4 => AuxiliaryKind::Black { inverted: true },
                5 => AuxiliaryKind::Cfa { channel: 2 },
                6 => AuxiliaryKind::Thermal,
                _ => unreachable!(),
            }
        );
        let details = rows(&independent, "extra_details ");
        assert_eq!(&details[0][..5], &[0, role, 8, 0, 0]);
        if role == 2 {
            assert_eq!(&details[0][5..9], &spot.map(f32::to_bits));
        }
        if role == 5 {
            assert_eq!(details[0][9], 2);
        }
        let source = image
            .interpretation()
            .source_details
            .get::<JxlSourceFacts>()
            .unwrap();
        assert_eq!(
            source.extra_details[0].spot_color,
            (role == 2).then_some(spot)
        );
        assert_eq!(
            source.extra_details[0].cfa_channel,
            (role == 5).then_some(2)
        );
        assert_eq!(source.frame_timecode, None);
        assert_eq!(source.header.intrinsic_size, Some((13, 9)));
        assert_eq!(rows(&independent, "source_display "), vec![vec![13, 9]]);
        let tone = &source.header.tone_mapping;
        let expected = vec![
            1000.0f32.to_bits(),
            0.125f32.to_bits(),
            1,
            0.25f32.to_bits(),
        ];
        assert_eq!(
            vec![
                tone.intensity_target.to_bits(),
                tone.min_nits.to_bits(),
                u32::from(tone.relative_to_max_display),
                tone.linear_below.to_bits()
            ],
            expected
        );
        assert_eq!(rows(&independent, "source_tone "), vec![expected]);
        assert_eq!(
            (
                image.interpretation().geometry.canvas_width,
                image.interpretation().geometry.canvas_height
            ),
            (8, 6)
        );
    }
}

#[test]
fn output_generated_float_alpha_source_precision_differs_from_f32_output_storage() {
    for (layout, bits, exponent, samples) in [
        (
            PixelLayout::RgbaLinearF16,
            16,
            5,
            [0x3400u16, 0x3800, 0x3a00, 0x3c00]
                .repeat(8 * 6)
                .into_iter()
                .flat_map(u16::to_ne_bytes)
                .collect::<Vec<_>>(),
        ),
        (
            PixelLayout::RgbaLinearF32,
            32,
            8,
            [0.25f32, 0.5, 0.75, 1.0]
                .repeat(8 * 6)
                .into_iter()
                .flat_map(f32::to_ne_bytes)
                .collect::<Vec<_>>(),
        ),
    ] {
        let encoded = LosslessConfig::new()
            .with_effort(3)
            .encode(&samples, 8, 6, layout)
            .unwrap();
        let independent = inspect(&format!("float-alpha-{bits}"), &encoded);
        let (image, _) = owned_first(encoded);
        let mapped = image.map().unwrap();
        let plane = mapped.plane(1).unwrap().unwrap();
        let Meaning::Auxiliary(aux) = plane.meaning else {
            panic!("alpha role lost")
        };
        assert_eq!(aux.kind, AuxiliaryKind::Alpha(AlphaMode::Straight));
        assert_eq!(
            aux.source_precision,
            SourcePrecision::Float {
                bits,
                exponent_bits: exponent
            }
        );
        assert_eq!(plane.samples.channel_type(), ChannelType::F32);
        assert_eq!(
            &rows(&independent, "extra_details ")[0][..4],
            &[0, 0, bits, exponent]
        );
        // These exactly representable source values give a second pixel oracle,
        // independent of native sequential selection and its f32 normalization.
        image
            .with_pixels(|pixels| {
                for y in 0..6 {
                    let expected: Vec<u8> = [0.25f32, 0.5, 0.75, 1.0]
                        .repeat(8)
                        .into_iter()
                        .flat_map(f32::to_ne_bytes)
                        .collect();
                    assert_eq!(pixels.row(y), expected.as_slice());
                }
            })
            .unwrap();
    }
}

#[test]
fn output_generated_reverse_selected_timecodes_remain_separate_from_exact_pts() {
    let pixels: Vec<Vec<u8>> = (0..3)
        .map(|f| (0..8 * 6 * 3).map(|i| (i * 31 + f * 43) as u8).collect())
        .collect();
    let durations = [3, 7, 2];
    let codes = [0x01020304, 0, 0x09080706];
    for present in [false, true] {
        let frames: Vec<_> = pixels
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let frame = AnimationFrame::new(p, durations[i]).with_name(format!("frame-{i}"));
                if present {
                    frame.with_timecode(codes[i])
                } else {
                    frame
                }
            })
            .collect();
        let params = AnimationParams {
            tps_numerator: 30000,
            tps_denominator: 1001,
            num_loops: 4,
            ..Default::default()
        };
        let encoded = LosslessConfig::new()
            .with_effort(3)
            .with_auto_delta_frames(false)
            .encode_animation(8, 6, PixelLayout::Rgb8, &params, &frames)
            .unwrap();
        let independent = inspect(&format!("timecodes-{present}"), &encoded);
        let expected = expected_output(&encoded);
        assert_eq!(
            rows(&independent, "timecode ")
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            if present { codes.to_vec() } else { vec![0; 3] }
        );
        assert_eq!(
            rows(&independent, "animation "),
            vec![vec![30000, 1001, 4, u32::from(present)]]
        );
        let mut collection = JxlItems::open(&encoded, Composition::Composited, 8).unwrap();
        let mut cursor = collection.images().unwrap();
        let mut ids = Vec::new();
        while let Some(id) = Collection::next(&mut collection, &mut cursor).unwrap() {
            ids.push(id);
        }
        assert_eq!(ids.len(), 3);
        let selected: Vec<_> = [2, 0, 1, 2]
            .into_iter()
            .map(|index| {
                (
                    index,
                    Collection::decode(&mut collection, &ids[index], &ImageRequest::default())
                        .unwrap(),
                )
            })
            .collect();
        drop(collection);
        drop(encoded);
        for (index, image) in selected {
            exact_samples(&image, &expected[index]);
            let facts = image.interpretation();
            let source = facts.source_details.get::<JxlSourceFacts>().unwrap();
            assert_eq!(source.frame_timecode, present.then_some(codes[index]));
            assert_eq!(source.frame_index, index);
            assert_eq!(source.frame_name, format!("frame-{index}"));
            let timing = facts.timing.unwrap();
            assert_eq!(timing.duration_ticks, Some(u64::from(durations[index])));
            let ticks = durations[..index].iter().map(|&n| i64::from(n)).sum();
            assert_eq!(
                timing.pts,
                Some(zencodec::time::Timestamp::new(
                    ticks,
                    zencodec::time::TimeBase::new(1001, 30000).unwrap()
                ))
            );
        }
    }
}

#[test]
fn output_generated_subsampled_auxiliary_geometry_is_distinct_from_source_shift() {
    let (width, height) = (17usize, 13usize);
    let color: Vec<u8> = (0..width * height * 3)
        .map(|i| (i * 19 + 7) as u8)
        .collect();
    for shift in [1, 2, 3] {
        let source_width = width.div_ceil(1 << shift);
        let source_height = height.div_ceil(1 << shift);
        let values: Vec<u8> = (0..source_width * source_height)
            .map(|i| (i * 37 + 11) as u8)
            .collect();
        for role in 0..7 {
            let extra = match role {
                0 => ExtraChannel::from_alpha_buf(&values, false),
                1 => ExtraChannel::depth(&values),
                2 => ExtraChannel::spot_color(&values, [0.25, 0.5, 0.75, 1.0]),
                3 => ExtraChannel::selection_mask(&values),
                4 => ExtraChannel::black(&values),
                5 => ExtraChannel::cfa(&values, 2),
                6 => ExtraChannel::thermal(&values),
                _ => unreachable!(),
            }
            .with_dim_shift(shift);
            let extras = [extra];
            let encoded = LosslessConfig::new()
                .with_effort(3)
                .encode_request(
                    width.try_into().unwrap(),
                    height.try_into().unwrap(),
                    PixelLayout::Rgb8,
                )
                .with_extra_channels(&extras)
                .encode(&color)
                .unwrap();
            let independent = inspect(&format!("subsampled-{role}-{shift}"), &encoded);
            assert_eq!(
                &rows(&independent, "extra_details ")[0][..5],
                &[0, role, 8, 0, shift]
            );
            let (output, expected) = owned_first(encoded.clone());
            project_packaged_subsampled(&encoded, &expected, shift);
            let mapped = output.map().unwrap();
            let plane = mapped.plane(1).unwrap().unwrap();
            let Meaning::Auxiliary(aux) = plane.meaning else {
                panic!("auxiliary role lost")
            };
            assert_eq!(aux.source_dim_shift, shift);
            // The native API emits an upsampled canvas. The source shift does
            // not redefine its emitted row length or permit reading fewer rows.
            assert_eq!(plane.samples.dimensions(), (width, height));
            assert_ne!(plane.samples.dimensions(), (source_width, source_height));
            for row in 0..height {
                let Row::Bytes(bytes) = plane.samples.row(row).unwrap() else {
                    panic!("native F32 output lost")
                };
                assert_eq!(bytes.len(), width * 4);
            }
        }
    }
}

#[test]
fn output_generated_spot_contribution_discloses_applied_color_without_losing_auxiliary() {
    let (width, height) = (9usize, 7usize);
    let color = vec![0; width * height * 3];
    let masks: Vec<u8> = (0..width * height)
        .map(|i| if i % 3 == 1 { 0 } else { 255 })
        .collect();
    let rgba = [0.25, 0.5, 0.75, 1.0];
    let extras = [ExtraChannel::spot_color(&masks, rgba)];
    let encoded = LosslessConfig::new()
        .with_effort(3)
        .encode_request(
            width.try_into().unwrap(),
            height.try_into().unwrap(),
            PixelLayout::Rgb8,
        )
        .with_extra_channels(&extras)
        .encode(&color)
        .unwrap();
    let independent: Vec<_> = [false, true]
        .into_iter()
        .map(|render| {
            inspect_mode(
                &format!("spot-render-{render}"),
                &encoded,
                if render {
                    "samples"
                } else {
                    "samples-unrendered"
                },
            )
        })
        .collect();
    let mut items = JxlItems::open(&encoded, Composition::Composited, 8).unwrap();
    let id = items.next().unwrap().unwrap().image;
    let outputs: Vec<_> = [false, true, false]
        .into_iter()
        .map(|render| (render, items.decode_with_spot_colors(&id, render).unwrap()))
        .collect();
    drop(items);
    drop(encoded);
    for (render, image) in &outputs {
        let mapped = image.map().unwrap();
        let extra = mapped.plane(1).unwrap().unwrap();
        let Meaning::Auxiliary(auxiliary) = extra.meaning else {
            panic!("missing auxiliary role")
        };
        assert_eq!(
            auxiliary.kind,
            AuxiliaryKind::SpotColor {
                linear_rgba: rgba,
                contribution: if *render {
                    ColorContribution::Applied
                } else {
                    ColorContribution::Separate
                }
            }
        );
        let expected = rows(&independent[usize::from(*render)], "pixels ");
        assert_eq!(expected.len(), 2);
        for (index, codes) in expected.iter().enumerate() {
            assert_eq!(codes[0] as usize, index);
            let plane = mapped.plane(index).unwrap().unwrap();
            let channels = if index == 0 { 3 } else { 1 };
            let expected_bytes: Vec<u8> = codes[1..].iter().flat_map(|v| v.to_ne_bytes()).collect();
            assert_eq!(expected_bytes.len(), width * height * channels * 4);
            for y in 0..height {
                let Row::Bytes(bytes) = plane.samples.row(y).unwrap() else {
                    panic!("F32 storage lost")
                };
                assert_eq!(
                    bytes,
                    &expected_bytes[y * width * channels * 4..(y + 1) * width * channels * 4]
                );
            }
        }
        image
            .with_pixels(|pixels| {
                for y in 0..height {
                    let expected: Vec<u8> = (0..width)
                        .flat_map(|x| {
                            let values = if *render && masks[y * width + x] != 0 {
                                [0.25f32, 0.5, 0.75]
                            } else {
                                [0.0; 3]
                            };
                            values.into_iter().flat_map(f32::to_ne_bytes)
                        })
                        .collect();
                    assert_eq!(pixels.row(y.try_into().unwrap()), expected.as_slice());
                }
            })
            .unwrap();
    }
    let first = crate::output_tests::mapped_planes(&outputs[0].1);
    let rendered = crate::output_tests::mapped_planes(&outputs[1].1);
    let repeated = crate::output_tests::mapped_planes(&outputs[2].1);
    assert_eq!(first, repeated);
    assert_eq!(first[1], rendered[1]);
    assert_ne!(first[0], rendered[0]);
}
