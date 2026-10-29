//! Independent point-membership oracle for local crop/presentation coordinates.
use zencodec::image::{ImageGeometry, Placement, PlacementError, Rect};

#[test]
fn every_visible_sample_matches_independent_canvas_membership() {
    // Include cropped selections with a nonzero local origin, layers larger
    // than the canvas, negative placement and every fully off-canvas direction.
    for (width, height) in [(1, 1), (3, 2), (5, 4)] {
        for x in 0..width {
            for y in 0..height {
                let geometry = ImageGeometry {
                    canvas_width: width,
                    canvas_height: height,
                    region: Rect {
                        x,
                        y,
                        width: width - x,
                        height: height - y,
                    },
                };
                for canvas in [(1, 1), (2, 3), (4, 2)] {
                    for ox in -6..=6 {
                        for oy in -6..=6 {
                            let placement = Placement {
                                canvas,
                                origin: (ox, oy),
                            };
                            let overlap = placement.visible(geometry).unwrap();
                            let mut visible = 0;
                            for sy in 0..geometry.region.height {
                                for sx in 0..geometry.region.width {
                                    // i128 membership is independent of the production
                                    // signed rectangle intersection/offset arithmetic.
                                    let px = i128::from(ox) + i128::from(x) + i128::from(sx);
                                    let py = i128::from(oy) + i128::from(y) + i128::from(sy);
                                    let expected = (0..i128::from(canvas.0)).contains(&px)
                                        && (0..i128::from(canvas.1)).contains(&py);
                                    let actual = overlap.is_some_and(|r| {
                                        (r.samples.x..r.samples.x + r.samples.width).contains(&sx)
                                            && (r.samples.y..r.samples.y + r.samples.height)
                                                .contains(&sy)
                                    });
                                    assert_eq!(
                                        actual, expected,
                                        "{placement:?}, {geometry:?}, ({sx},{sy})"
                                    );
                                    if expected {
                                        visible += 1;
                                        let r = overlap.unwrap();
                                        assert_eq!(i128::from(r.canvas.x + sx - r.samples.x), px);
                                        assert_eq!(i128::from(r.canvas.y + sy - r.samples.y), py);
                                    }
                                }
                            }
                            assert_eq!(
                                overlap.map_or(0, |r| r.samples.width * r.samples.height),
                                visible
                            );
                            if let Some(r) = overlap {
                                assert_eq!(
                                    (r.samples.width, r.samples.height),
                                    (r.canvas.width, r.canvas.height)
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
fn coordinate_limits_validate_the_whole_layer_even_for_a_small_crop() {
    let geometry = ImageGeometry {
        canvas_width: u32::MAX,
        canvas_height: u32::MAX,
        region: Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
    };
    for origin in [(i64::MIN, 0), (0, i64::MIN), (i64::MIN, i64::MIN)] {
        assert_eq!(
            Placement {
                canvas: (1, 1),
                origin
            }
            .visible(geometry)
            .unwrap(),
            None
        );
    }
    for origin in [(i64::MAX, 0), (0, i64::MAX), (i64::MAX - 1, 0)] {
        assert!(matches!(
            Placement {
                canvas: (1, 1),
                origin
            }
            .visible(geometry),
            Err(PlacementError::CoordinateOverflow)
        ));
    }
    let edge = i64::MAX - i64::from(u32::MAX);
    assert_eq!(
        Placement {
            canvas: (1, 1),
            origin: (edge, edge)
        }
        .visible(geometry)
        .unwrap(),
        None
    );
}
