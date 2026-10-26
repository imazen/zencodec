use crate::{
    fixtures::*,
    image_output::{ImageOutput, Meaning, OutputError, Row},
};
use enough::Unstoppable;
use std::sync::Arc;
use zenavif::media::Av1VideoDecoder;
use zencodec_media::{
    frame::{PlaneSamples, Subsampling},
    session::VideoDecoder,
};

#[test]
fn output_native_av1_keeps_mapping_ownership_codes_roles_and_timing() {
    let artifacts = std::env::var_os("AV1_POLICY_ARTIFACTS")
        .expect("AV1_POLICY_ARTIFACTS must name persistent output");
    let root = std::path::Path::new(&artifacts);
    std::fs::create_dir_all(root).unwrap();
    for bits in [8, 10, 12] {
        for (sub, identity) in [
            (Subsampling::Mono, false),
            (Subsampling::Yuv420, false),
            (Subsampling::Yuv422, false),
            (Subsampling::Yuv444, false),
            (Subsampling::Yuv444, true),
        ] {
            let settings = settings(bits, sub, identity);
            let inputs: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
            let (config, packets) = encode(settings.clone(), &inputs);
            let mut native_settings = rav1d_safe::Settings::default();
            native_settings.threads = 1;
            let mut decoder = Av1VideoDecoder::with_settings(native_settings).unwrap();
            let frames = decode(&mut decoder, &config, &packets, &Unstoppable).unwrap();
            assert_eq!(frames.len(), inputs.len());
            oracle(
                root,
                &format!("output-{bits}-{sub:?}-{identity}"),
                &settings,
                &packets,
                &frames,
            );
            decoder.reset().unwrap();
            drop(decoder);
            for (native, input) in frames.into_iter().zip(inputs) {
                let expected = samples(&native);
                assert_eq!(expected, samples(&input));
                let interpretation = native.interpretation().clone();
                let timing = native.timing();
                let presentation = native.presentation();
                let owner = Arc::downgrade(native.picture());
                let addresses: Vec<_> = {
                    let mapped = native.map().unwrap();
                    (0..mapped.plane_count())
                        .map(|p| match mapped.plane(p).unwrap().row(0).unwrap() {
                            PlaneSamples::U8(row) => row.as_ptr() as usize,
                            PlaneSamples::U16(row) => row.as_ptr() as usize,
                        })
                        .collect()
                };
                let image = ImageOutput::from_frame(native);
                let retained = image.clone();
                drop(image);
                assert!(owner.upgrade().is_some());
                assert!(Arc::ptr_eq(
                    retained.interpretation().signal.as_ref().unwrap(),
                    &interpretation
                ));
                assert_eq!(retained.interpretation().timing, Some(timing));
                assert_eq!(retained.interpretation().presentation, Some(presentation));
                assert_eq!(retained.interpretation().orientation, None);
                assert_eq!(
                    (
                        retained.interpretation().geometry.canvas_width,
                        retained.interpretation().geometry.canvas_height
                    ),
                    (settings.width, settings.height)
                );
                let error = retained.with_pixels(|_| ()).unwrap_err();
                assert_eq!(
                    error.downcast_ref::<OutputError>(),
                    Some(&OutputError::NotPackedColor)
                );
                let mapped = retained.map().unwrap();
                assert_eq!(mapped.plane_count(), interpretation.roles.len());
                let mut actual = Vec::new();
                for (p, address) in addresses.iter().enumerate() {
                    let plane = mapped.plane(p).unwrap().unwrap();
                    assert_eq!(plane.meaning, &Meaning::Component(interpretation.roles[p]));
                    assert_eq!(plane.samples.code_encoding(), Some(interpretation.encoding));
                    assert_eq!(plane.samples.code_encoding().unwrap().code_bits(), bits);
                    let pointer = match plane.samples.row(0).unwrap() {
                        Row::Bytes(row) => row.as_ptr() as usize,
                        Row::Words16(row) => row.as_ptr() as usize,
                    };
                    assert_eq!(pointer, *address, "mapping must not copy the native plane");
                    for y in 0..plane.samples.dimensions().1 {
                        match plane.samples.row(y).unwrap() {
                            Row::Bytes(row) => actual.extend_from_slice(row),
                            Row::Words16(row) => {
                                actual.extend(row.iter().flat_map(|word| word.to_le_bytes()))
                            }
                        }
                    }
                    assert!(plane.samples.row(plane.samples.dimensions().1).is_none());
                }
                assert_eq!(actual, expected);
                assert!(mapped.plane(mapped.plane_count()).unwrap().is_none());
                drop(mapped);
                drop(retained);
                assert!(
                    owner.upgrade().is_none(),
                    "final image release must release the native picture"
                );
            }
        }
    }
}
