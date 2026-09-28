use zencodec_media::color::{
    ChromaLocation as Location, ColorError, Subsampling, YuvToRgb, YuvView,
};
use zencodec_media::plane::{Plane, Samples};
use zenpixels::{ChannelType, Cicp, sample::SampleEncoding};

fn plane(data: &[u16], w: usize, h: usize, stride: usize, bits: u8, shift: u8) -> Plane<'_> {
    Plane::new(
        Samples::U16(data),
        w,
        h,
        stride * 2,
        SampleEncoding::new(ChannelType::U16, bits, shift).unwrap(),
    )
    .unwrap()
}

fn close(actual: [f32; 3], expected: [f64; 3]) {
    for (a, e) in actual.into_iter().zip(expected) {
        let tolerance = 2.0 * f64::from(f32::EPSILON) * e.abs().max(1.0);
        assert!(
            (f64::from(a) - e).abs() <= tolerance,
            "{a} != {e} (budget {tolerance})"
        );
    }
}

#[test]
fn reconstruction_matches_all_independent_decimal_ycbcr_vectors_in_both_packings() {
    let mut cases = 0;
    for line in include_str!("../corpus/reference-vectors.csv")
        .lines()
        .skip(1)
    {
        let cells: Vec<_> = line.split(',').collect();
        if cells[0] != "ycbcr" {
            continue;
        }
        let bits: u8 = cells[1].parse().unwrap();
        let color = Cicp::new(222, 223, cells[3].parse().unwrap(), cells[2] == "1");
        let codes: [u16; 3] = std::array::from_fn(|i| cells[4 + i].parse().unwrap());
        let expected = std::array::from_fn(|i| cells[7 + i].parse().unwrap());
        for shift in [0, 16 - bits] {
            // All low padding bits set proves storage padding is not signal.
            let padded = codes.map(|c| [c << shift | ((1_u32 << shift) - 1) as u16]);
            let planes: [_; 3] = std::array::from_fn(|i| plane(&padded[i], 1, 1, 1, bits, shift));
            let view = YuvView::new(
                planes[0],
                Some([planes[1], planes[2]]),
                Subsampling::Yuv444,
                Location::Unknown,
                color,
            )
            .unwrap();
            let converter = YuvToRgb::new(view).unwrap();
            assert_eq!(converter.output_color(), Cicp::new(222, 223, 0, true));
            let mut row = [[f32::NAN; 3]];
            converter.write_row(0, &mut row).unwrap();
            close(row[0], expected);
        }
        cases += 1;
    }
    assert_eq!(cases, 192);
}

// Independent spatial reference: sum triangle-kernel weights across the whole
// component grid after clamping continuous coordinates. Production computes
// integer neighbors and quarter phases; this computes neither of those.
fn interpolate(data: &[u16], w: usize, h: usize, x: f64, y: f64) -> f64 {
    let x = x.clamp(0.0, (w - 1) as f64);
    let y = y.clamp(0.0, (h - 1) as f64);
    let mut sum = 0.0;
    for row in 0..h {
        for column in 0..w {
            let weight = (1.0 - (column as f64 - x).abs()).max(0.0)
                * (1.0 - (row as f64 - y).abs()).max(0.0);
            sum += f64::from(data[row * w + column]) * weight;
        }
    }
    sum
}

#[test]
fn odd_crops_preserve_all_siting_phases_and_original_edge_neighbors() {
    let (w, h) = (7_usize, 5_usize);
    let ys: Vec<_> = (0..w * h).map(|i| (64 + i * 11) as u16).collect();
    for sampling in [
        Subsampling::Yuv420,
        Subsampling::Yuv422,
        Subsampling::Yuv444,
    ] {
        let (sx, sy) = match sampling {
            Subsampling::Yuv420 => (2, 2),
            Subsampling::Yuv422 => (2, 1),
            _ => (1, 1),
        };
        let (cw, ch) = (w.div_ceil(sx), h.div_ceil(sy));
        let cb: Vec<_> = (0..cw * ch).map(|i| (64 + (i * 29) % 897) as u16).collect();
        let cr: Vec<_> = (0..cw * ch).map(|i| (960 - i * 17) as u16).collect();
        for location in [Location::Center, Location::Left, Location::TopLeft] {
            let (ox, oy) = match location {
                Location::Center => (0.5, 0.5),
                Location::Left => (0.0, 0.5),
                _ => (0.0, 0.0),
            };
            let (ox, oy) = (
                if sx == 1 { 0.0 } else { ox },
                if sy == 1 { 0.0 } else { oy },
            );
            let view = YuvView::new(
                plane(&ys, w, h, w, 10, 0),
                Some([plane(&cb, cw, ch, cw, 10, 0), plane(&cr, cw, ch, cw, 10, 0)]),
                sampling,
                location,
                Cicp::new(1, 1, 1, false),
            )
            .unwrap();
            let full = YuvToRgb::new(view).unwrap();
            // Every nonempty crop, including nested crops and right/bottom edges.
            for top in 0..h {
                for left in 0..w {
                    let cropped = view.crop(left, top, w - left, h - top).unwrap();
                    let direct = YuvToRgb::new(cropped).unwrap();
                    let nested = YuvToRgb::new(
                        view.crop(0, top, w, h - top)
                            .unwrap()
                            .crop(left, 0, w - left, h - top)
                            .unwrap(),
                    )
                    .unwrap();
                    for row in 0..h - top {
                        let mut all = vec![[0.0; 3]; w];
                        let mut part = vec![[0.0; 3]; w - left];
                        let mut second = part.clone();
                        full.write_row(top + row, &mut all).unwrap();
                        direct.write_row(row, &mut part).unwrap();
                        nested.write_row(row, &mut second).unwrap();
                        assert_eq!(part, all[left..]);
                        assert_eq!(part, second);
                        for (col, rgb) in part.into_iter().enumerate() {
                            let (x, y) = (left + col, top + row);
                            let codes = [
                                f64::from(ys[y * w + x]),
                                interpolate(
                                    &cb,
                                    cw,
                                    ch,
                                    (x as f64 - ox) / sx as f64,
                                    (y as f64 - oy) / sy as f64,
                                ),
                                interpolate(
                                    &cr,
                                    cw,
                                    ch,
                                    (x as f64 - ox) / sx as f64,
                                    (y as f64 - oy) / sy as f64,
                                ),
                            ];
                            close(
                                rgb,
                                zencodec_media_testkit::reference::ycbcr_to_rgb(
                                    codes, 10, false, 0.2126, 0.0722,
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn identity_and_monochrome_use_luma_scaling_without_chroma_offsets() {
    let y = [64, 940, 1023];
    let b = [940, 64, 0];
    let r = [502, 64, 940];
    for full in [false, true] {
        let color = Cicp::new(12, 13, 0, full);
        let view = YuvView::new(
            plane(&y, 3, 1, 3, 10, 0),
            Some([plane(&b, 3, 1, 3, 10, 0), plane(&r, 3, 1, 3, 10, 0)]),
            Subsampling::Yuv444,
            Location::Unknown,
            color,
        )
        .unwrap();
        let mut row = [[0.0; 3]; 3];
        YuvToRgb::new(view).unwrap().write_row(0, &mut row).unwrap();
        for i in 0..3 {
            let expected = [r[i], y[i], b[i]].map(|v| {
                if full {
                    f64::from(v) / 1023.0
                } else {
                    (f64::from(v) - 64.0) / 876.0
                }
            });
            close(row[i], expected);
        }
        let mono = YuvView::new(
            plane(&y, 3, 1, 3, 10, 0),
            None,
            Subsampling::Monochrome,
            Location::Unknown,
            Cicp::new(222, 223, 255, full),
        )
        .unwrap();
        YuvToRgb::new(mono).unwrap().write_row(0, &mut row).unwrap();
        for i in 0..3 {
            close(
                row[i],
                [if full {
                    f64::from(y[i]) / 1023.0
                } else {
                    (f64::from(y[i]) - 64.0) / 876.0
                }; 3],
            );
        }
    }
}

#[test]
fn ambiguous_or_invalid_inputs_fail_before_touching_output() {
    let y = [0; 15];
    let c = [128; 6];
    let view = YuvView::new(
        plane(&y, 5, 3, 5, 8, 0),
        Some([plane(&c, 3, 2, 3, 8, 0); 2]),
        Subsampling::Yuv420,
        Location::Unknown,
        Cicp::new(1, 1, 1, true),
    )
    .unwrap();
    assert!(matches!(
        YuvToRgb::new(view),
        Err(ColorError::UnknownChromaLocation)
    ));
    let converter = YuvToRgb::new(view.with_chroma_location(Location::Left)).unwrap();
    let mut output = [[42.0; 3]; 5];
    assert!(matches!(
        converter.write_row(3, &mut output),
        Err(ColorError::OutsideFrame)
    ));
    assert_eq!(output, [[42.0; 3]; 5]);
    assert!(matches!(
        converter.write_row(0, &mut output[..4]),
        Err(ColorError::OutputWidth)
    ));
    assert_eq!(output, [[42.0; 3]; 5]);
    assert!(view.crop(usize::MAX, 0, 1, 1).is_err());
    assert!(view.crop(0, 0, usize::MAX, 1).is_err());
    let data = [0];
    for matrix in [2, 3, 8, 10, 11, 12, 13, 14, 255] {
        let p = plane(&data, 1, 1, 1, 8, 0);
        let v = YuvView::new(
            p,
            Some([p; 2]),
            Subsampling::Yuv444,
            Location::Unknown,
            Cicp::new(1, 13, matrix, true),
        )
        .unwrap();
        assert!(
            matches!(YuvToRgb::new(v),Err(ColorError::UnsupportedMatrix(code)) if code==matrix)
        );
    }
}
