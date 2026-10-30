//! Representation-only gate: borrows real decoded AV1 planes after decoder drop.
use crate::{
    component_grid::{Axis, Grid},
    component_storage::{Order, Plane},
    fixtures::*,
};
use zenavif::media::Av1VideoDecoder;
use zencodec::frame::{PlaneSamples, Subsampling};

#[test]
fn component_storage_native_borrows_av1_without_repacking() {
    for bits in [8, 10, 12] {
        for sub in [Subsampling::Yuv420, Subsampling::Yuv444] {
            let settings = settings(bits, sub, false);
            let input = frame(&settings, 0);
            let (config, packets) = encode(settings, &[input]);
            let mut decoder = Av1VideoDecoder::new().unwrap();
            let frames = decode(&mut decoder, &config, &packets, None).unwrap();
            drop(decoder);
            assert_eq!(frames.len(), 1);
            for frame in frames {
                let mapped = frame.map().unwrap();
                assert_eq!(mapped.plane_count(), 3);
                for index in 0..mapped.plane_count() {
                    let original = mapped.plane(index).unwrap();
                    let bytes = match original.as_strided_samples() {
                        PlaneSamples::U8(values) => values,
                        PlaneSamples::U16(values) => bytemuck::cast_slice(values),
                    };
                    let encoding = original.encoding();
                    let plane = Plane::new(
                        bytes,
                        encoding.storage(),
                        Order::Native,
                        original.width(),
                        original.height(),
                        original.stride_bytes(),
                        encoding.storage().byte_size(),
                    )
                    .unwrap();
                    // An odd luma crop must not reset the chroma sampling phase.
                    // Retain all samples from the chosen component origin; this is
                    // a view identity control, not a resampling/filter-halo policy.
                    let factor = if index > 0 && sub == Subsampling::Yuv420 {
                        2
                    } else {
                        1
                    };
                    let (px, py) = (1 / factor, 3 / factor);
                    let cropped = plane
                        .window(px, py, original.width() - px, original.height() - py)
                        .unwrap();
                    let axis = Axis::new((factor - 1) as i64, (2 * factor) as u64, 2).unwrap();
                    let Grid::Axes { x: gx, y: gy } = (Grid::Axes { x: axis, y: axis })
                        .rebase((px as u64, py as u64), (1, 3))
                        .unwrap()
                    else {
                        panic!()
                    };
                    let cropped_codes = cropped.codes(encoding).unwrap();
                    for y in 0..original.height() - py {
                        for x in 0..original.width() - px {
                            assert_eq!(
                                cropped_codes.code(x, y).unwrap(),
                                original.code(x + px, y + py).unwrap()
                            );
                            let (nx, dx) = gx.center(x as u64).unwrap();
                            let (ny, dy) = gy.center(y as u64).unwrap();
                            let source_x2 = (2 * (x + px) * factor + factor - 1) as i128;
                            let source_y2 = (2 * (y + py) * factor + factor - 1) as i128;
                            assert_eq!(nx * 2, (source_x2 - 2) * i128::from(dx));
                            assert_eq!(ny * 2, (source_y2 - 6) * i128::from(dy));
                        }
                    }
                    let codes = plane.codes(encoding).unwrap();
                    for y in 0..original.height() {
                        let row = plane.row(y).unwrap();
                        match original.row(y).unwrap() {
                            PlaneSamples::U8(values) => {
                                assert_eq!(row.typed::<u8>().unwrap().as_ptr(), values.as_ptr());
                                assert_eq!(row.typed::<u8>().unwrap(), values);
                            }
                            PlaneSamples::U16(values) => {
                                assert_eq!(row.typed::<u16>().unwrap().as_ptr(), values.as_ptr());
                                assert_eq!(row.typed::<u16>().unwrap(), values);
                            }
                        }
                        for x in 0..original.width() {
                            assert_eq!(codes.code(x, y).unwrap(), original.code(x, y).unwrap());
                        }
                    }
                }
            }
        }
    }
}
