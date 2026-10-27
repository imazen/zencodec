//! Executes the native patch, not a model of JPEG's output schedule.
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec::output_patch::OutputAccess;
use zenjpeg::JpegEncoderConfig;
use zenjpeg::encoder::{ChromaSubsampling, EncoderConfig as NativeConfig, XybSubsampling};
use zenpixels::{PixelDescriptor, PixelSlice};

fn configurations() -> Vec<JpegEncoderConfig> {
    let mut configs = vec![JpegEncoderConfig::new()];
    for subsampling in [
        ChromaSubsampling::None,
        ChromaSubsampling::HalfHorizontal,
        ChromaSubsampling::Quarter,
    ] {
        for progressive in [false, true] {
            for optimized in [false, true] {
                configs.push(
                    JpegEncoderConfig::ycbcr(83.0, subsampling)
                        .with_progressive(progressive)
                        .with_huffman(optimized),
                );
            }
        }
    }
    configs.push(JpegEncoderConfig::new().with_prefer_streaming(true));
    configs.push(
        JpegEncoderConfig::new()
            .with_progressive(true)
            .with_prefer_streaming(true),
    );
    for native in [
        NativeConfig::rgb(83.0),
        NativeConfig::xyb(83.0, XybSubsampling::BQuarter),
    ] {
        let mut config = JpegEncoderConfig::new();
        *config.inner_mut() = native;
        configs.push(config.with_prefer_streaming(true));
    }
    configs
}

#[test]
fn effective_plan_matches_native_startup_across_modes_and_geometry() {
    let mut cells = 0;
    for config in configurations() {
        for width in [1, 17] {
            for height in [1, 15, 16, 31, 32, 33, 65] {
                for descriptor in [PixelDescriptor::RGB8_SRGB, PixelDescriptor::GRAY8_SRGB] {
                    for access in [OutputAccess::Forward, OutputAccess::SeekBack] {
                        let job = config.clone().job();
                        let planned = job.plan_rows(width, height, descriptor, None, access);
                        let actual = job
                            .encoder()
                            .unwrap()
                            .begin_output(width, height, descriptor, access);
                        match (planned, actual) {
                            (Ok(planned), Ok(actual)) => assert_eq!(
                                planned, actual,
                                "{width}x{height} {descriptor:?} {config:?}"
                            ),
                            (Err(_), Err(_)) => {}
                            (planned, actual) => panic!(
                                "planning/startup disagreement: {planned:?} / {actual:?}; {config:?}"
                            ),
                        }
                        cells += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cells, 952);
}

#[test]
fn planning_is_repeatable_and_does_not_change_strided_encode_bytes() {
    let stride = 17 * 3 + 15;
    let mut data = vec![0xa5; stride * 33];
    for y in 0..33 {
        for x in 0..17 * 3 {
            data[y * stride + x] = ((x * 31 + y * 17) % 256) as u8;
        }
    }
    let pixels = || PixelSlice::new(&data, 17, 33, stride, PixelDescriptor::RGB8_SRGB).unwrap();
    for config in configurations() {
        let expected = config
            .clone()
            .job()
            .encoder()
            .unwrap()
            .encode(pixels())
            .unwrap();
        let job = config.clone().job();
        let first = job
            .plan_rows(
                17,
                33,
                PixelDescriptor::RGB8_SRGB,
                None,
                OutputAccess::Forward,
            )
            .unwrap();
        let second = job
            .plan_rows(
                17,
                33,
                PixelDescriptor::RGB8_SRGB,
                None,
                OutputAccess::Forward,
            )
            .unwrap();
        assert_eq!(first, second);
        let actual = job.encoder().unwrap().encode(pixels()).unwrap();
        assert_eq!(expected.data(), actual.data());
    }
}

#[test]
fn invalid_input_limits_canvas_and_metadata_refuse_during_planning() {
    let config = JpegEncoderConfig::new();
    for (width, height) in [(0, 32), (32, 0), (u32::MAX, 2)] {
        assert!(
            config
                .clone()
                .job()
                .plan_rows(
                    width,
                    height,
                    PixelDescriptor::RGB8_SRGB,
                    None,
                    OutputAccess::Forward
                )
                .is_err()
        );
    }
    for job in [
        config.clone().job().with_canvas_size(16, 16),
        config
            .clone()
            .job()
            .with_limits(zencodec::ResourceLimits::none().with_max_pixels(1)),
        config
            .clone()
            .job()
            .with_limits(zencodec::ResourceLimits::none().with_max_memory(1)),
    ] {
        assert!(
            job.plan_rows(
                32,
                32,
                PixelDescriptor::RGB8_SRGB,
                None,
                OutputAccess::Forward
            )
            .is_err()
        );
    }
    let meta = zencodec::Metadata::default().with_exif(vec![0; 70_000]);
    let job = config
        .clone()
        .job()
        .with_metadata_policy(meta, zencodec::MetadataPolicy::PreserveExact);
    assert!(
        job.plan_rows(
            32,
            32,
            PixelDescriptor::RGB8_SRGB,
            None,
            OutputAccess::Forward
        )
        .is_err()
    );
}

#[test]
fn current_color_context_is_checked_before_startup() {
    use zenpixels::{ColorContext, ColorPrimaries, TransferFunction};
    let profile = crate::fixtures::profile();
    let descriptor = PixelDescriptor::RGB8_SRGB
        .with_transfer(TransferFunction::Unknown)
        .with_primaries(ColorPrimaries::Unknown);
    let job = JpegEncoderConfig::new().job().with_metadata_policy(
        zencodec::Metadata::default().with_icc(profile.clone()),
        zencodec::MetadataPolicy::PreserveExact,
    );
    let matching = ColorContext::default().with_icc(profile);
    assert!(
        job.plan_rows(32, 32, descriptor, Some(&matching), OutputAccess::Forward)
            .is_ok()
    );
    for context in [
        ColorContext::default(),
        matching.with_cicp(zencodec::Cicp::new(1, 13, 0, true)),
    ] {
        assert!(
            job.plan_rows(32, 32, descriptor, Some(&context), OutputAccess::Forward)
                .is_err()
        );
    }
}

#[test]
fn cancelled_planning_refuses_before_begin() {
    #[derive(Clone)]
    struct Cancel;
    impl zencodec::enough::Stop for Cancel {
        fn check(&self) -> Result<(), zencodec::enough::StopReason> {
            Err(zencodec::enough::StopReason::TimedOut)
        }
    }
    let job = JpegEncoderConfig::new()
        .job()
        .with_stop(zencodec::StopToken::new(Cancel));
    assert!(
        job.plan_rows(
            32,
            32,
            PixelDescriptor::RGB8_SRGB,
            None,
            OutputAccess::Forward
        )
        .is_err()
    );
}
