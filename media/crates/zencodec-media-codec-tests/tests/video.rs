use std::io::Cursor;
use zencodec::{
    animation::FrameDuration,
    decode::{Decode, DecodeJob, DecoderConfig},
    encode::{EncodeJob, EncoderConfig as _},
};
use zencodec_media::{
    av1::Av1IvfDecoder,
    av1_encode::Av1IvfEncoder,
    av1_index::{FrameScope, FrameSelection, IndexedAv1},
    color::{ChromaLocation, Subsampling},
    display::OutOfRange,
    encode_color::{AlphaHandling, RgbToYuv},
    frame::RgbStorage,
    time::{TimeBase, Timestamp},
    video::{
        AnimationToVideo, LoopHandling, VideoRgb, animation_to_ivf, encode_extracted,
        ivf_to_animation,
    },
};
use zencodec_media_codec_tests::{Fixture, Format, fixtures};
use zenpixels::Cicp;
use zenrav1e::{
    EncoderConfig,
    color::{
        ChromaSampling, ColorDescription, ColorPrimaries, MatrixCoefficients, PixelRange,
        TransferCharacteristics,
    },
    prelude::SpeedSettings,
};

fn settings() -> rav1d_safe::Settings {
    let mut settings = rav1d_safe::Settings::default();
    settings.threads = 1;
    settings.max_frame_delay = 1;
    settings.all_layers = false;
    settings
}
fn config(f: &Fixture) -> EncoderConfig {
    let mut speed = SpeedSettings::from_preset(10);
    speed.rdo_lookahead_frames = 1;
    EncoderConfig {
        width: f.frames[0].width() as usize,
        height: f.frames[0].height() as usize,
        bit_depth: 8,
        chroma_sampling: ChromaSampling::Cs444,
        pixel_range: PixelRange::Full,
        color_description: Some(ColorDescription {
            color_primaries: ColorPrimaries::BT709,
            transfer_characteristics: TransferCharacteristics::SRGB,
            matrix_coefficients: MatrixCoefficients::Identity,
        }),
        quantizer: 0,
        min_quantizer: 0,
        low_latency: true,
        min_key_frame_interval: 2,
        max_key_frame_interval: 2,
        speed_settings: speed,
        ..Default::default()
    }
}
fn options(loops: LoopHandling) -> AnimationToVideo {
    AnimationToVideo::new(
        TimeBase::new(1, 100).unwrap(),
        RgbToYuv::new(
            8,
            Subsampling::Yuv444,
            ChromaLocation::Unknown,
            Cicp::new(1, 13, 0, true),
            OutOfRange::Reject,
        )
        .unwrap(),
        AlphaHandling::CompositeEncoded([0.0; 3]),
        10,
        1_000_000,
    )
    .with_loops(loops)
}
fn encode(
    fixture: &Fixture,
    loops: LoopHandling,
) -> Result<(Vec<u8>, zencodec_media::video::VideoReport), zencodec_media::video::VideoError> {
    let mut source = Format::Apng
        .decoder(fixture.encode(Format::Apng).unwrap())
        .unwrap();
    let encoder = Av1IvfEncoder::new(
        Vec::new(),
        config(fixture),
        TimeBase::new(1, 100).unwrap(),
        8,
        1,
    )
    .unwrap();
    animation_to_ivf(source.as_mut(), encoder, &options(loops), None)
}
fn assert_matte(actual: zenpixels::PixelSlice<'_>, expected: &zenpixels::PixelBuffer) {
    assert_eq!(actual.width(), expected.width());
    assert_eq!(actual.rows(), expected.height());
    let channels = actual.descriptor().bytes_per_pixel();
    assert!([3, 4].contains(&channels));
    for y in 0..actual.rows() {
        for (actual, source) in actual
            .row(y)
            .chunks_exact(channels)
            .zip(expected.as_slice().row(y).as_chunks::<4>().0)
        {
            let rgb = if source[3] == 0 {
                [0, 0, 0]
            } else {
                [source[0], source[1], source[2]]
            };
            assert_eq!(&actual[..3], &rgb);
            if channels == 4 {
                assert_eq!(actual[3], 255);
            }
        }
    }
}

#[test]
fn animation_video_animation_and_timestamp_selected_png_preserve_samples_and_vfr() {
    let render = VideoRgb::new(RgbStorage::U8, OutOfRange::Reject, 1_000_000);
    for fixture in fixtures().into_iter().filter(|f| f.frames[0].width() <= 17) {
        assert!(encode(&fixture, LoopHandling::RequireSinglePlay).is_err());
        let (bytes, report) = encode(&fixture, LoopHandling::OneIteration).unwrap();
        assert_eq!(report.frames(), 5);
        assert_eq!(report.source_plays(), Some(2));
        assert_eq!(report.first_timestamp().ticks(), 0);
        assert_eq!(report.end_timestamp().ticks(), 14);
        assert_eq!(report.last_duration(), fixture.durations[4]);
        let mut source =
            Av1IvfDecoder::new(Cursor::new(bytes.clone()), 1_000_000, settings()).unwrap();
        let (animation, report) = ivf_to_animation(
            &mut source,
            Format::Apng.encoder(Some(1)).unwrap(),
            &render,
            fixture.durations[4],
            5,
            None,
        )
        .unwrap();
        assert_eq!(report.end_timestamp().ticks(), 14);
        let mut decoded = Format::Apng.decoder(animation.into_vec()).unwrap();
        for (pixels, duration) in fixture.frames.iter().zip(&fixture.durations) {
            let frame = decoded.render_next_frame_owned(None).unwrap().unwrap();
            assert_eq!(frame.duration(), *duration);
            assert_matte(frame.pixels(), pixels);
        }
        assert!(decoded.render_next_frame_owned(None).unwrap().is_none());
        let mut index =
            IndexedAv1::build(Cursor::new(bytes), 1_000_000, 20, 4096, settings(), None).unwrap();
        for (request, selection, scope, ordinal, actual_pts) in [
            (4, FrameSelection::Nearest, FrameScope::All, 2, 3),
            (6, FrameSelection::Nearest, FrameScope::All, 3, 6),
            (6, FrameSelection::Nearest, FrameScope::Keyframes, 4, 7),
            (5, FrameSelection::Nearest, FrameScope::Keyframes, 2, 3),
            (6, FrameSelection::AtOrBefore, FrameScope::Keyframes, 2, 3),
            (6, FrameSelection::AtOrAfter, FrameScope::Keyframes, 4, 7),
        ] {
            let extracted = index
                .extract(
                    Timestamp::new(request, TimeBase::new(1, 100).unwrap()),
                    selection,
                    scope,
                )
                .unwrap()
                .unwrap();
            assert_eq!(extracted.presentation_index(), ordinal);
            assert_eq!(extracted.frame().timestamp().unwrap().ticks(), actual_pts);
            let png = encode_extracted(
                &extracted,
                zenpng::PngEncoderConfig::new().job().encoder().unwrap(),
                &render,
                None,
            )
            .unwrap();
            let image = zenpng::PngDecoderConfig::new()
                .job()
                .decoder(
                    std::borrow::Cow::Owned(png.into_vec()),
                    &[zenpixels::PixelDescriptor::RGBA8_SRGB],
                )
                .unwrap()
                .decode()
                .unwrap();
            assert_matte(image.into_buffer().as_slice(), &fixture.frames[ordinal]);
        }
    }
}

#[test]
fn duplicate_pts_and_unknown_final_duration_are_errors() {
    let fixture = fixtures().remove(0);
    let (mut bytes, _) = encode(&fixture, LoopHandling::OneIteration).unwrap();
    let first_len = u32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize;
    let second_timestamp = 32 + 12 + first_len + 4;
    bytes[second_timestamp..second_timestamp + 8].copy_from_slice(&0u64.to_le_bytes());
    let render = VideoRgb::new(RgbStorage::U8, OutOfRange::Reject, 1_000_000);
    for final_duration in [
        FrameDuration::new(0, 1).unwrap(),
        FrameDuration::new(1, 100).unwrap(),
    ] {
        let mut source =
            Av1IvfDecoder::new(Cursor::new(bytes.clone()), 1_000_000, settings()).unwrap();
        assert!(
            ivf_to_animation(
                &mut source,
                Format::Apng.encoder(Some(1)).unwrap(),
                &render,
                final_duration,
                10,
                None
            )
            .is_err()
        );
    }
}
