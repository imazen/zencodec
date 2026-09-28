use zencodec::{
    animation::FrameDuration,
    encode::{EncodeJob, EncoderConfig},
};
use zencodec_media::{
    animation::{AnimationLimits, TimingPolicy, transcode_dyn},
    time::Rounding,
};
use zencodec_media_codec_tests::{Fixture, Format};
use zenpixels::{PixelBuffer, PixelDescriptor};

#[test]
fn fractional_animation_requires_explicit_quantization_for_gif_and_webp() {
    let durations = vec![FrameDuration::new(1, 60).unwrap(); 60];
    let frames = (0..60)
        .map(|i| {
            PixelBuffer::from_vec(
                vec![(i % 2) * 255, 0, 0, 255],
                1,
                1,
                PixelDescriptor::RGBA8_SRGB,
            )
            .unwrap()
        })
        .collect();
    let source = Fixture {
        name: "one-second-sixty-frames".into(),
        frames,
        durations,
        plays: 1,
    }
    .encode(Format::Apng)
    .unwrap();
    for (format, rate) in [(Format::Gif, 100), (Format::Webp, 1000)] {
        let limits = AnimationLimits::new(60, 240).unwrap();
        let mut decoder = Format::Apng.decoder(source.clone()).unwrap();
        assert!(
            transcode_dyn(
                decoder.as_mut(),
                |_, plays| format.encoder(plays),
                TimingPolicy::Exact,
                limits,
                None
            )
            .is_err()
        );
        let mut decoder = Format::Apng.decoder(source.clone()).unwrap();
        let (output, report) = transcode_dyn(
            decoder.as_mut(),
            |_, plays| format.encoder(plays),
            TimingPolicy::Quantize {
                ticks_per_second: rate,
                rounding: Rounding::Nearest,
            },
            limits,
            None,
        )
        .unwrap();
        assert_eq!(report.frames(), 60);
        assert_eq!(report.changed_durations(), 60);
        let mut decoder = format.decoder(output.into_vec()).unwrap();
        let mut total = 0;
        for n in 1..=60_u64 {
            let frame = decoder.render_next_frame_owned(None).unwrap().unwrap();
            total += frame.duration().ticks_at(rate).unwrap();
            // Independent integer endpoint oracle; ties round earlier.
            assert_eq!(total, (n * u64::from(rate) + 29) / 60);
        }
        assert_eq!(total, u64::from(rate));
        assert!(decoder.render_next_frame_owned(None).unwrap().is_none());
    }
}

#[test]
fn total_plays_outside_destination_field_are_not_silently_wrapped() {
    // APNG supports a full u32 total-play count; WebP's ANIM field is u16.
    let mut encoder = zenpng::PngEncoderConfig::new()
        .job()
        .with_loop_count(Some(65536))
        .dyn_animation_frame_encoder()
        .unwrap();
    let pixels =
        PixelBuffer::from_vec(vec![0, 0, 0, 255], 1, 1, PixelDescriptor::RGBA8_SRGB).unwrap();
    for _ in 0..2 {
        encoder
            .push_frame_timed(pixels.as_slice(), FrameDuration::new(1, 100).unwrap(), None)
            .unwrap();
    }
    let mut decoder = Format::Apng
        .decoder(encoder.finish(None).unwrap().into_vec())
        .unwrap();
    assert_eq!(decoder.loop_count(), Some(65536));
    assert!(
        transcode_dyn(
            decoder.as_mut(),
            |_, plays| Format::Webp.encoder(plays),
            TimingPolicy::Exact,
            AnimationLimits::new(2, 8).unwrap(),
            None
        )
        .is_err()
    );
}
