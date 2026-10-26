use crate::{
    image_items::Composition,
    image_output::{AuxiliaryKind, ImageOutput, Meaning, Row, Samples},
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
        let expected = expected_output(&data);
        let mut items = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
        let selected = items.next().unwrap().unwrap();
        let output = items
            .decode_image(&selected.image)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        drop(items);
        drop(data);
        let retained = output.clone();
        drop(output);
        assert_eq!(
            retained.orientation,
            Orientation::from_exif(info.orientation as u8)
        );
        assert_eq!(
            (
                retained.geometry.canvas_width as usize,
                retained.geometry.canvas_height as usize
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
                    AuxiliaryKind::SpotColor => 2,
                    AuxiliaryKind::SelectionMask => 3,
                    AuxiliaryKind::Black => 4,
                    AuxiliaryKind::Cfa => 5,
                    AuxiliaryKind::Thermal => 6,
                    AuxiliaryKind::Unknown(code) => code,
                };
                assert_eq!(
                    role_code, facts[1],
                    "{name}: common role disagrees with libjxl"
                );
                assert_eq!(role.source_bits, facts[2]);
                assert_eq!(role.source_dim_shift, facts[3]);
                assert_eq!(role.name, native.name);
                assert_eq!(role.source_bits, native.bits_per_sample);
                assert_eq!(role.source_dim_shift, native.dim_shift);
                assert_eq!(
                    role.unexposed_native_parameters,
                    matches!(
                        native.ec_type,
                        zenjxl_decoder::api::ExtraChannel::SpotColor
                            | zenjxl_decoder::api::ExtraChannel::CFA
                    )
                );
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
        let timing = output.timing.unwrap();
        assert_eq!(timing.pts, Some(expected_pts));
        assert_eq!(timing.duration_ticks, Some(durations[index]));
        assert_eq!(timing.packet_ordinal, None);
        assert_eq!(timing.input_offset, None);
    }
}
