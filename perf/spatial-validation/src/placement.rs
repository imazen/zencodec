//! Native and independent-oracle qualification of the shared placement.
use zencodec::image::{ImageGeometry, Rect};
pub use zencodec::image::{Placement, PlacementError as Error, PlacementOverlap as Overlap};

#[test]
fn placement_matches_independent_point_membership() {
    // An independent per-sample membership oracle checks clipping on every
    // edge, subregion offsets and valid fully off-canvas placements.
    for x in -5..=5 {
        for y in -5..=5 {
            for width in 1..=4 {
                for height in 1..=4 {
                    for start_x in 0..width {
                        for start_y in 0..height {
                            let g = ImageGeometry {
                                canvas_width: width,
                                canvas_height: height,
                                region: Rect {
                                    x: start_x,
                                    y: start_y,
                                    width: width - start_x,
                                    height: height - start_y,
                                },
                            };
                            let placement = Placement {
                                origin: (x, y),
                                canvas: (3, 2),
                            };
                            let result = placement.visible(g).unwrap();
                            let mut expected = Vec::new();
                            for sy in 0..g.region.height {
                                for sx in 0..g.region.width {
                                    let cx = i64::from(sx + start_x) + x;
                                    let cy = i64::from(sy + start_y) + y;
                                    if (0..3).contains(&cx) && (0..2).contains(&cy) {
                                        expected.push((sx, sy, cx as u32, cy as u32));
                                    }
                                }
                            }
                            let mut actual = Vec::new();
                            if let Some(Overlap { samples, canvas }) = result {
                                assert_eq!(
                                    (samples.width, samples.height),
                                    (canvas.width, canvas.height)
                                );
                                for dy in 0..samples.height {
                                    for dx in 0..samples.width {
                                        actual.push((
                                            samples.x + dx,
                                            samples.y + dy,
                                            canvas.x + dx,
                                            canvas.y + dy,
                                        ));
                                    }
                                }
                            }
                            assert_eq!(actual, expected, "{placement:?} {g:?}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn placement_extremes_refuse_overflow_without_clamping_the_source() {
    let g = ImageGeometry {
        canvas_width: u32::MAX,
        canvas_height: u32::MAX,
        region: Rect {
            x: 0,
            y: 0,
            width: u32::MAX,
            height: u32::MAX,
        },
    };
    assert!(
        Placement {
            canvas: (1, 1),
            origin: (i64::MIN, i64::MIN)
        }
        .visible(g)
        .unwrap()
        .is_none()
    );
    assert!(matches!(
        Placement {
            canvas: (1, 1),
            origin: (i64::MAX, 0)
        }
        .visible(g),
        Err(Error::CoordinateOverflow)
    ));
    assert!(matches!(
        Placement {
            canvas: (1, 1),
            origin: (0, i64::MAX)
        }
        .visible(g),
        Err(Error::CoordinateOverflow)
    ));
    assert!(matches!(
        Placement {
            canvas: (0, 1),
            origin: (0, 0)
        }
        .visible(g),
        Err(Error::EmptyCanvas)
    ));
    let invalid = ImageGeometry {
        region: Rect {
            x: u32::MAX,
            width: 1,
            ..g.region
        },
        ..g
    };
    assert!(matches!(
        Placement {
            canvas: (1, 1),
            origin: (0, 0)
        }
        .visible(invalid),
        Err(Error::Region(_))
    ));
}

#[test]
fn placement_intersection_copies_strided_samples_without_relabeling_or_clipping_owner() {
    use zencodec::image::{ImageInterpretation, ImageOutput};
    use zenpixels::{Orientation, PixelBuffer, PixelBufferParts, PixelDescriptor};
    let geometry = ImageGeometry {
        canvas_width: 6,
        canvas_height: 5,
        region: Rect {
            x: 1,
            y: 1,
            width: 4,
            height: 3,
        },
    };
    let mut bytes = vec![0xa5; 3 * 18];
    for y in 0..3 {
        for x in 0..4 {
            bytes[y * 18 + x * 3..][..3].fill((y * 4 + x) as u8);
        }
    }
    let expected_storage = bytes.clone();
    let pixels = PixelBuffer::try_from_parts(PixelBufferParts::new(
        bytes,
        4,
        3,
        18,
        PixelDescriptor::RGB8,
    ))
    .unwrap();
    let image = ImageOutput::packed(
        pixels,
        vec![],
        ImageInterpretation::packed(geometry, Orientation::Identity),
    )
    .unwrap();
    let placement = Placement {
        canvas: (2, 2),
        origin: (-2, -2),
    };
    let visible = placement
        .visible(image.interpretation().geometry)
        .unwrap()
        .unwrap();
    assert_eq!(
        visible.samples,
        Rect {
            x: 1,
            y: 1,
            width: 2,
            height: 2
        }
    );
    assert_eq!(
        visible.canvas,
        Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 2
        }
    );
    let retained = image.clone();
    drop(image);
    let mut destination = [0xcc; 16];
    retained
        .with_pixels(|pixels| {
            for y in 0..visible.samples.height {
                let row = pixels.row(y + visible.samples.y);
                let start = visible.samples.x as usize * 3;
                destination[y as usize * 8..][..6].copy_from_slice(&row[start..start + 6]);
            }
            for y in 0..3 {
                assert_eq!(pixels.row(y), &expected_storage[y as usize * 18..][..12]);
            }
        })
        .unwrap();
    assert_eq!(
        destination,
        [
            5, 5, 5, 6, 6, 6, 0xcc, 0xcc, 9, 9, 9, 10, 10, 10, 0xcc, 0xcc
        ]
    );
    assert_eq!(retained.interpretation().geometry, geometry);
}

#[test]
fn placement_retains_real_jxl_layer_extents_beyond_the_canvas() {
    use crate::jxl_tests::independent_metadata;
    let text = independent_metadata("multiple_layers_noise_spline.jxl", false);
    let canvas: Vec<i64> = text
        .lines()
        .find_map(|line| line.strip_prefix("canvas "))
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    let mut layers = 0;
    let mut outside = 0;
    for line in text.lines().filter_map(|line| line.strip_prefix("frame ")) {
        let f: Vec<i64> = line
            .split_whitespace()
            .map(|s| s.parse().unwrap())
            .collect();
        let geometry = ImageGeometry {
            canvas_width: f[1].try_into().unwrap(),
            canvas_height: f[2].try_into().unwrap(),
            region: Rect {
                x: 0,
                y: 0,
                width: f[1].try_into().unwrap(),
                height: f[2].try_into().unwrap(),
            },
        };
        let placement = Placement {
            canvas: (canvas[0].try_into().unwrap(), canvas[1].try_into().unwrap()),
            origin: (f[3], f[4]),
        };
        let overlap = placement.visible(geometry).unwrap().unwrap();
        if overlap.samples.width != geometry.region.width
            || overlap.samples.height != geometry.region.height
        {
            outside += 1;
        }
        if placement.origin == (1860, 950) {
            assert_eq!(
                overlap.canvas,
                Rect {
                    x: 1860,
                    y: 950,
                    width: 188,
                    height: 202
                }
            );
            assert_eq!(
                geometry.region,
                Rect {
                    x: 0,
                    y: 0,
                    width: 1024,
                    height: 576
                }
            );
        }
        layers += 1;
    }
    assert_eq!(layers, 5);
    assert_eq!(outside, 2); // y=640 also extends below the 1152-row canvas.
}
