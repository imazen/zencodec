use zencodec_media::{
    color::{ChromaLocation, Subsampling, YuvToRgb},
    display::OutOfRange,
    encode_color::{AlphaHandling, EncodeColorError, RgbToYuv},
};
use zenpixels::{ChannelType, Cicp, PixelDescriptor, PixelSlice, SignalRange};

fn source(
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    descriptor: PixelDescriptor,
) -> PixelSlice<'_> {
    PixelSlice::new(data, width, height, stride, descriptor).unwrap()
}

#[test]
fn rgb_to_native_codes_match_independent_decimal_vectors() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../corpus/encode-color-references.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let bits = case["bits"].as_u64().unwrap() as u8;
        let matrix = case["matrix"].as_u64().unwrap() as u8;
        let full = case["full"].as_bool().unwrap();
        // References start with integer packed RGB16, avoiding float input
        // ambiguity at a quantization half-step.
        let mut storage = Vec::new();
        for v in case["rgb16"].as_array().unwrap() {
            storage.extend_from_slice(&(v.as_u64().unwrap() as u16).to_ne_bytes());
        }
        let input = source(&storage, 1, 1, 6, PixelDescriptor::RGB16_SRGB);
        let plan = RgbToYuv::new(
            bits,
            Subsampling::Yuv444,
            ChromaLocation::Unknown,
            Cicp::new(1, 13, matrix, full),
            OutOfRange::Reject,
        )
        .unwrap();
        let output = plan
            .convert(&input, AlphaHandling::RequireOpaque, 6, None)
            .unwrap();
        for c in 0..3 {
            assert_eq!(
                output.view().plane(c).unwrap().code(0, 0).unwrap(),
                case["codes"][c].as_u64().unwrap() as u16,
                "{case}"
            );
        }
        assert_eq!(output.view().bit_depth(), bits);
        assert_eq!(
            output.view().plane(0).unwrap().encoding().storage(),
            ChannelType::U16
        );
    }
    assert_eq!(cases.as_array().unwrap().len(), 352);
}

// Independent continuous triangle integration. Walk a padded coordinate field
// and evaluate the kernel, rather than choosing the production's four taps.
fn chroma_reference(
    bytes: &[u8],
    (w, h): (usize, usize),
    (cx, cy): (f64, f64),
    (sx, sy): (usize, usize),
    component: usize,
) -> u16 {
    let mut value = 0.0;
    let mut sum = 0.0;
    for y in -3..h as isize + 3 {
        let wy = if sy == 1 {
            u8::from(y as f64 == cy) as f64
        } else {
            (1.0 - (y as f64 - cy).abs() / 2.0).max(0.0)
        };
        for x in -3..w as isize + 3 {
            let wx = if sx == 1 {
                u8::from(x as f64 == cx) as f64
            } else {
                (1.0 - (x as f64 - cx).abs() / 2.0).max(0.0)
            };
            let weight = wx * wy;
            if weight == 0.0 {
                continue;
            }
            let px = x.clamp(0, w as isize - 1) as usize;
            let py = y.clamp(0, h as isize - 1) as usize;
            let p = &bytes[(py * w + px) * 3..][..3];
            let [r, g, b] = [p[0], p[1], p[2]].map(|x| f64::from(x) / 255.0);
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            value += weight
                * if component == 1 {
                    (b - luma) / (2.0 * (1.0 - 0.0722))
                } else {
                    (r - luma) / (2.0 * (1.0 - 0.2126))
                };
            sum += weight;
        }
    }
    (512.0 + value / sum * 896.0).round().clamp(0.0, 1023.0) as u16
}

#[test]
fn subsampling_filter_preserves_phase_and_odd_edges() {
    for w in [1_usize, 2, 3, 7] {
        for h in [1_usize, 2, 3, 5] {
            let bytes: Vec<_> = (0..w * h * 3)
                .map(|i| ((i * 113 + i / 5 * 29) % 256) as u8)
                .collect();
            let input = source(
                &bytes,
                w as u32,
                h as u32,
                w * 3,
                PixelDescriptor::RGB8_SRGB,
            );
            for (sampling, sx, sy) in [
                (Subsampling::Yuv420, 2, 2),
                (Subsampling::Yuv422, 2, 1),
                (Subsampling::Yuv444, 1, 1),
            ] {
                for location in [
                    ChromaLocation::Center,
                    ChromaLocation::Left,
                    ChromaLocation::TopLeft,
                ] {
                    let plan = RgbToYuv::new(
                        10,
                        sampling,
                        location,
                        Cicp::new(1, 13, 1, false),
                        OutOfRange::Reject,
                    )
                    .unwrap();
                    let output = plan
                        .convert(&input, AlphaHandling::RequireOpaque, 100000, None)
                        .unwrap();
                    for c in 1..3 {
                        let p = output.view().plane(c).unwrap();
                        assert_eq!((p.width(), p.height()), (w.div_ceil(sx), h.div_ceil(sy)));
                        for y in 0..p.height() {
                            for x in 0..p.width() {
                                let cx = (x * sx) as f64
                                    + if sx == 2 && location == ChromaLocation::Center {
                                        0.5
                                    } else {
                                        0.0
                                    };
                                let cy = (y * sy) as f64
                                    + if sy == 2 && location != ChromaLocation::TopLeft {
                                        0.5
                                    } else {
                                        0.0
                                    };
                                assert_eq!(
                                    p.code(x, y).unwrap(),
                                    chroma_reference(&bytes, (w, h), (cx, cy), (sx, sy), c),
                                    "{w}x{h} {sampling:?} {location:?} c{c} at{x},{y}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn identity_preserves_rgb_and_explicit_alpha_matte_without_reading_row_padding() {
    let bytes = [
        255, 0, 128, 255, 0, 255, 40, 0, 91, 92, 93, 94, 10, 20, 30, 128, 255, 255, 255, 255,
    ];
    let input = source(&bytes, 2, 2, 12, PixelDescriptor::RGBA8_SRGB);
    let plan = RgbToYuv::new(
        8,
        Subsampling::Yuv444,
        ChromaLocation::Unknown,
        Cicp::new(1, 13, 0, true),
        OutOfRange::Reject,
    )
    .unwrap();
    assert!(matches!(
        plan.convert(&input, AlphaHandling::RequireOpaque, 100, None),
        Err(EncodeColorError::NonopaquePixel)
    ));
    assert!(matches!(
        plan.convert(&input, AlphaHandling::CompositeEncoded([0.0; 3]), 23, None),
        Err(EncodeColorError::AllocationLimit)
    ));
    let output = plan
        .convert(
            &input,
            AlphaHandling::CompositeEncoded([1.0, 0.0, 0.0]),
            24,
            None,
        )
        .unwrap();
    let converter = YuvToRgb::new(output.view()).unwrap();
    let mut row = [[0.0; 3]; 2];
    converter.write_row(0, &mut row).unwrap();
    assert_eq!(row, [[1.0, 0.0, 128.0 / 255.0], [1.0, 0.0, 0.0]]);
    converter.write_row(1, &mut row).unwrap();
    assert_eq!(row[1], [1.0; 3]);
    let expected = [132.0 / 255.0, 10.0 / 255.0, 15.0 / 255.0];
    assert_eq!(row[0], expected);
    let narrow = source(
        &bytes,
        2,
        2,
        12,
        PixelDescriptor::RGBA8_SRGB.with_signal_range(SignalRange::Narrow),
    );
    assert!(matches!(
        plan.convert(&narrow, AlphaHandling::RequireOpaque, 100, None),
        Err(EncodeColorError::UnsupportedPixels)
    ));
    assert!(matches!(
        plan.convert(
            &input,
            AlphaHandling::CompositeEncoded([f32::NAN; 3]),
            100,
            None
        ),
        Err(EncodeColorError::InvalidMatte)
    ));
}
