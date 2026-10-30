//! Representation-only gate: borrows real decoded AV1 planes after decoder drop.
use crate::{
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
