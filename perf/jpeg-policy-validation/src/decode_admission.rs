//! Native counterexample to treating unconfigured output prediction as admission.
use crate::fixtures;
use std::borrow::Cow;
use zencodec::{
    CodecSet,
    decode::{Decode, DecodeJob, DecoderConfig},
    operation::{DecodeRequest, OperationError},
};
use zenjpeg::JpegDecoderConfig;
use zenpixels::PixelDescriptor;

#[test]
fn negotiated_output_must_not_be_rejected_by_native_prediction() {
    for progressive in [false, true] {
        let base = fixtures::base(progressive);
        let mut bytes = base[..2].to_vec();
        bytes.extend(fixtures::descriptive());
        bytes.extend_from_slice(&base[2..]);
        let config = JpegDecoderConfig::new();
        let predicted = config.clone().job().output_info(&bytes).unwrap();
        let requested = PixelDescriptor::RGBA8_SRGB;
        assert_ne!(predicted.native_format, requested);
        let expected = config
            .clone()
            .job()
            .decoder(Cow::Borrowed(&bytes), &[requested])
            .unwrap()
            .decode()
            .unwrap();
        let codecs = CodecSet::new().with_decoder(config);
        let actual = DecodeRequest::new()
            .with_output(requested)
            .decode(&codecs, &bytes)
            .unwrap();
        assert_eq!(actual.pixels().descriptor(), requested);
        assert_eq!(actual.metadata(), expected.metadata());
        assert_eq!(actual.info().orientation, expected.info().orientation);
        assert_eq!(actual.width(), expected.width());
        assert_eq!(actual.height(), expected.height());
        for row in 0..actual.height() {
            assert_eq!(actual.pixels().row(row), expected.pixels().row(row));
        }
    }
}

#[test]
fn required_output_keeps_final_check_when_native_falls_back() {
    let bytes = fixtures::base(false);
    let codecs = CodecSet::new().with_decoder(JpegDecoderConfig::new());
    let requested = PixelDescriptor::RGB16_SRGB;
    let error = DecodeRequest::new()
        .with_output(requested)
        .decode(&codecs, &bytes)
        .unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<OperationError>(&error),
        Some(OperationError::OutputDescriptor { requested: got, actual })
            if *got == requested && *actual != requested
    ));
}

#[test]
fn required_native_layouts_preserve_samples_and_metadata_across_scan_modes() {
    for progressive in [false, true] {
        let base = fixtures::base(progressive);
        let mut bytes = base[..2].to_vec();
        bytes.extend(fixtures::descriptive());
        bytes.extend_from_slice(&base[2..]);
        for requested in [
            PixelDescriptor::RGB8_SRGB,
            PixelDescriptor::RGBA8_SRGB,
            PixelDescriptor::BGRA8_SRGB,
            PixelDescriptor::RGBX8_SRGB,
            PixelDescriptor::BGRX8_SRGB,
            PixelDescriptor::RGBF32_LINEAR,
            PixelDescriptor::RGBAF32_LINEAR,
        ] {
            let config = JpegDecoderConfig::new();
            let expected = config
                .clone()
                .job()
                .decoder(Cow::Borrowed(&bytes), &[requested])
                .unwrap()
                .decode()
                .unwrap();
            assert_eq!(
                expected.pixels().descriptor(),
                requested,
                "native output differs: progressive={progressive}, requested={requested:?}"
            );
            let codecs = CodecSet::new().with_decoder(config);
            let actual = DecodeRequest::new()
                .with_output(requested)
                .decode(&codecs, &bytes)
                .unwrap();
            assert_eq!(actual.pixels().descriptor(), requested);
            assert_eq!(actual.metadata(), expected.metadata());
            assert_eq!(actual.info().orientation, expected.info().orientation);
            assert_eq!(
                (actual.width(), actual.height()),
                (expected.width(), expected.height())
            );
            for row in 0..actual.height() {
                assert_eq!(actual.pixels().row(row), expected.pixels().row(row));
            }
        }
    }
}

#[test]
fn ranked_native_output_does_not_promote_a_later_float_preference() {
    for progressive in [false, true] {
        let bytes = fixtures::base(progressive);
        for preferred in [
            [PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBF32_LINEAR],
            [PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGBAF32_LINEAR],
            [PixelDescriptor::RGBF32_LINEAR, PixelDescriptor::RGB8_SRGB],
            [PixelDescriptor::RGBAF32_LINEAR, PixelDescriptor::RGBA8_SRGB],
        ] {
            let config = JpegDecoderConfig::new();
            let expected = config
                .clone()
                .job()
                .decoder(Cow::Borrowed(&bytes), &preferred[..1])
                .unwrap()
                .decode()
                .unwrap();
            let actual = DecodeRequest::new()
                .preferring_output(preferred.to_vec())
                .decode(&CodecSet::new().with_decoder(config), &bytes)
                .unwrap();
            assert_eq!(
                actual.pixels().descriptor(),
                expected.pixels().descriptor(),
                "progressive={progressive}, preferred={preferred:?}"
            );
            assert_eq!(actual.metadata(), expected.metadata());
            for row in 0..actual.height() {
                assert_eq!(actual.pixels().row(row), expected.pixels().row(row));
            }
        }
    }
}
