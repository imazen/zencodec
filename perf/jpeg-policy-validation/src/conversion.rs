use crate::{fixtures, pixel_guide};
use zencodec::{CodecSet, Orientation};
use zenjpeg::JpegDecoderConfig;
use zenpixels::{ColorContext, PixelDescriptor, PixelSlice, TransferFunction};

#[test]
fn stored_requests_execute_native_jpeg_templates_without_changing_bytes_or_pixels() {
    use zencodec::encode::EncoderConfig;
    use zencodec::operation::{DecodeRequest, EncodeRequest};
    for progressive in [false, true] {
        let config = fixtures::config(progressive, true);
        let raw = fixtures::raw();
        let direct = config.clone().encode(fixtures::pixels(&raw)).unwrap();
        let codecs = CodecSet::new()
            .with_encoder(config)
            .with_decoder(JpegDecoderConfig::new());
        let encoded = EncodeRequest::new(zencodec::ImageFormat::Jpeg)
            .encode(&codecs, fixtures::pixels(&raw))
            .unwrap();
        assert_eq!(encoded.data(), direct.data());
        let direct_pixels = codecs.decode(encoded.data()).unwrap();
        let decoded = DecodeRequest::new()
            .with_output(PixelDescriptor::RGB8_SRGB)
            .decode(&codecs, encoded.data())
            .unwrap();
        for y in 0..fixtures::HEIGHT {
            assert_eq!(decoded.pixels().row(y), direct_pixels.pixels().row(y));
        }
        assert_eq!(
            fixtures::oracle(encoded.data()),
            fixtures::oracle(direct.data())
        );
    }
}

#[test]
fn production_resolver_selects_native_jpeg_jobs_and_preserves_direct_output() {
    use std::convert::Infallible;
    use zencodec::{
        At, ImageFormat,
        encode::{EncoderConfig, Fidelity, FidelityTargetError},
        operation::{Decision, EncodeRequest, Question, Selection},
    };
    for progressive in [false, true] {
        let config = fixtures::config(progressive, true);
        let raw = fixtures::raw();
        let set = CodecSet::new().with_encoder(config.clone());
        let targets = [
            Fidelity::Lossless,
            Fidelity::codec_quality(35.0),
            Fidelity::codec_quality(80.0),
        ];
        let request = EncodeRequest::new(ImageFormat::Jpeg).preferring_fidelity(targets.to_vec());
        for index in [1, 2] {
            let mut calls = 0;
            let actual = request
                .encode_with(&set, fixtures::pixels(&raw), &mut |question: Question<
                    '_,
                >|
                 -> Result<
                    _,
                    At<Infallible>,
                > {
                    calls += 1;
                    let Question::Encode { facts, choices } = question else {
                        panic!()
                    };
                    assert_eq!(
                        (facts.width, facts.height),
                        (fixtures::WIDTH, fixtures::HEIGHT)
                    );
                    assert_eq!(
                        choices.len(),
                        2,
                        "JPEG cannot qualify mathematical losslessness"
                    );
                    assert!(
                        choices
                            .iter()
                            .all(|c| c.value().fidelity != Some(Fidelity::Lossless))
                    );
                    Ok(Decision::choose(
                        choices
                            .iter()
                            .find(|c| c.value().preference_index == index)
                            .unwrap(),
                    ))
                })
                .unwrap();
            let direct = config
                .clone()
                .try_with_fidelity(targets[index])
                .unwrap()
                .encode(fixtures::pixels(&raw))
                .unwrap();
            assert_eq!(actual.data(), direct.data());
            assert_eq!(
                fixtures::oracle(actual.data()),
                fixtures::oracle(direct.data())
            );
            let Selection::Encode(selected) = actual
                .report()
                .decisions()
                .next()
                .unwrap()
                .selected
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(selected.fidelity, Some(targets[index]));
            assert_eq!(selected.preference_index, index);
            assert_eq!(calls, 1);
        }
        let error = EncodeRequest::new(ImageFormat::Jpeg)
            .with_fidelity(Fidelity::Lossless)
            .encode(&set, fixtures::pixels(&raw))
            .unwrap_err();
        assert!(matches!(
            zencodec::find_cause::<FidelityTargetError>(&error),
            Some(FidelityTargetError::Substituted { .. })
        ));
    }
}

#[test]
fn migration_display_conversion_keeps_pixels_and_residual_orientation() {
    for progressive in [false, true] {
        let mut source = fixtures::base(progressive);
        source.splice(2..2, fixtures::descriptive());
        let native = CodecSet::new()
            .with_decoder(JpegDecoderConfig::new())
            .decode(&source)
            .unwrap();
        let (converted, orientation) = pixel_guide::decode_for_display(&source).unwrap();
        assert_eq!(orientation, Orientation::Rotate90);
        assert_eq!(orientation, native.info().orientation);
        assert_eq!(converted.descriptor(), PixelDescriptor::BGRA8_SRGB);
        assert_eq!(native.pixels().descriptor(), PixelDescriptor::RGB8_SRGB);
        assert_eq!(
            (converted.width(), converted.height()),
            (fixtures::WIDTH, fixtures::HEIGHT)
        );
        for y in 0..converted.height() {
            for (rgb, bgra) in native
                .pixels()
                .row(y)
                .as_chunks::<3>()
                .0
                .iter()
                .zip(converted.as_slice().row(y).as_chunks::<4>().0.iter())
            {
                assert_eq!(bgra, &[rgb[2], rgb[1], rgb[0], 255]);
            }
        }
        assert!(!fixtures::oracle(&source).is_empty());
    }
}

fn framebuffer(stride: usize, trim_last: bool) -> Vec<u8> {
    let mut data = vec![0x35; stride * 5];
    for y in 0..5 {
        for x in 0..7 {
            data[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&[
                (x * 31) as u8,
                (y * 43) as u8,
                ((x + y) * 17) as u8,
                255,
            ]);
        }
    }
    if trim_last {
        data.truncate(stride * 4 + 28);
    }
    data
}

#[test]
fn migration_framebuffer_conversion_supports_stride_and_trimmed_final_padding() {
    let dense = framebuffer(28, false);
    let dense_view = PixelSlice::new(&dense, 7, 5, 28, PixelDescriptor::BGRA8_SRGB).unwrap();
    let expected = pixel_guide::encode_framebuffer(dense_view).unwrap();
    for (stride, trim) in [(28, false), (40, false), (40, true)] {
        let data = framebuffer(stride, trim);
        let view = PixelSlice::new(&data, 7, 5, stride, PixelDescriptor::BGRA8_SRGB).unwrap();
        let rgb = pixel_guide::rgb_for_jpeg(view.clone()).unwrap();
        for y in 0..5 {
            for (input, output) in view
                .row(y)
                .as_chunks::<4>()
                .0
                .iter()
                .zip(rgb.as_slice().row(y).as_chunks::<3>().0.iter())
            {
                assert_eq!(output, &[input[2], input[1], input[0]]);
            }
        }
        let jpeg = pixel_guide::encode_framebuffer(view).unwrap();
        assert_eq!(
            jpeg, expected,
            "stride/padding must not alter encoded output"
        );
        assert_eq!(fixtures::oracle(&jpeg), fixtures::oracle(&expected));
    }
}

#[test]
fn migration_framebuffer_refuses_transparency_and_unresolved_color() {
    let mut data = framebuffer(40, true);
    data[4 * 40 + 6 * 4 + 3] = 254;
    let view = PixelSlice::new(&data, 7, 5, 40, PixelDescriptor::BGRA8_SRGB).unwrap();
    let error = pixel_guide::encode_framebuffer(view).unwrap_err();
    assert!(
        error
            .downcast_ref::<zenpixels_convert::At<zenpixels_convert::ConvertError>>()
            .is_some()
    );
    data[4 * 40 + 6 * 4 + 3] = 255;
    let unknown = PixelDescriptor::BGRA8_SRGB.with_transfer(TransferFunction::Unknown);
    let view = PixelSlice::new(&data, 7, 5, 40, unknown).unwrap();
    assert!(
        pixel_guide::encode_framebuffer(view)
            .unwrap_err()
            .downcast_ref::<pixel_guide::ExpectedSrgbFramebuffer>()
            .is_some()
    );
    let view = PixelSlice::new(&data, 7, 5, 40, PixelDescriptor::BGRA8_SRGB)
        .unwrap()
        .with_color_context(std::sync::Arc::new(ColorContext::default()));
    assert!(
        pixel_guide::encode_framebuffer(view)
            .unwrap_err()
            .downcast_ref::<pixel_guide::ExpectedSrgbFramebuffer>()
            .is_some()
    );
}
