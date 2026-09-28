//! The caller chooses conversion; timing and composited canvases stay intact.
use std::sync::Arc;
use zencodec::{
    animation::FrameDuration,
    encode::{EncodeJob, EncoderConfig},
};
use zencodec_media::animation::{AnimationLimits, TimingPolicy, transcode_dyn_with};
use zencodec_media_codec_tests::Format;
use zenpixels::{
    Cicp, ColorContext, ColorOrigin, ColorPrimaries, ColorProfileSource, PixelBuffer,
    PixelDescriptor, PixelFormat, TransferFunction,
};
use zenpixels_convert::{
    cms::PluggableCms, cms_moxcms::MoxCms, finalize_for_output_with, icc_profiles::ADOBE_RGB,
    output::OutputProfile, policy::ConvertOptions,
};

#[test]
fn icc_animation_is_converted_before_four_native_encoders() {
    let desc = PixelDescriptor::RGBA8_SRGB
        .with_primaries(ColorPrimaries::Unknown)
        .with_transfer(TransferFunction::Unknown);
    let input = [191, 97, 53, 1, 47, 177, 121, 128, 157, 77, 193, 255];
    let buffer = PixelBuffer::from_vec(input.to_vec(), 3, 1, desc)
        .unwrap()
        .with_color_context(Arc::new(ColorContext::from_icc(ADOBE_RGB)));
    let mut transform = MoxCms
        .build_source_transform(
            ColorProfileSource::Icc(ADOBE_RGB),
            ColorProfileSource::Cicp(Cicp::SRGB),
            PixelFormat::Rgba8,
            PixelFormat::Rgba8,
            &ConvertOptions::permissive(),
        )
        .unwrap()
        .unwrap();
    let mut expected = [0; 12];
    transform.transform_row(&input, &mut expected, 3);
    assert_ne!(input, expected);
    let mut encoder = zenpng::PngEncoderConfig::new()
        .job()
        .with_loop_count(Some(2))
        .dyn_animation_frame_encoder()
        .unwrap();
    for _ in 0..3 {
        encoder
            .push_frame_timed(buffer.as_slice(), FrameDuration::new(1, 100).unwrap(), None)
            .unwrap();
    }
    let bytes = encoder.finish(None).unwrap().into_vec();
    for format in [Format::Apng, Format::Webp, Format::Jxl, Format::Avif] {
        let mut decoder = Format::Apng
            .decoder_with_preference(bytes.clone(), &[])
            .unwrap();
        let mut count = 0;
        let (encoded, report) = transcode_dyn_with(
            decoder.as_mut(),
            |_, plays| format.encoder(plays),
            |pixels| {
                count += 1;
                let ready = finalize_for_output_with(
                    &pixels,
                    &ColorOrigin::assumed(),
                    OutputProfile::Named(Cicp::SRGB),
                    PixelFormat::Rgba8,
                    Some(&MoxCms),
                )?;
                Ok(ready.into_parts().0)
            },
            TimingPolicy::Exact,
            AnimationLimits::new(3, 36).unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(count, 3);
        assert_eq!(report.changed_durations(), 0);
        let mut decoder = format.decoder(encoded.into_vec()).unwrap();
        assert_eq!(decoder.loop_count(), Some(2));
        for _ in 0..3 {
            let frame = decoder.render_next_frame_owned(None).unwrap().unwrap();
            assert_eq!(frame.duration(), FrameDuration::new(1, 100).unwrap());
            assert_eq!(frame.pixels().descriptor(), PixelDescriptor::RGBA8_SRGB);
            assert_eq!(frame.pixels().row(0), expected, "{format:?}");
        }
        assert!(decoder.render_next_frame_owned(None).unwrap().is_none());
    }
}
