//! Stored requirements are enforced through the ordinary execution boundary.
use zencodec::{
    CodecSet, ImageFormat, UnsupportedOperation,
    decode::DecodePolicy,
    encode::{EncodePolicy, Fidelity},
    operation::{DecodeRequest, EncodeRequest, OperationError},
};
use zencodec_testkit::{ReferenceEncoderConfig, ReferenceZcrDecoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};

const STRIDED: [u8; 18] = [
    1, 2, 3, 4, 5, 6, 99, 98, 97, 7, 8, 9, 10, 11, 12, 96, 95, 94,
];
fn pixels() -> PixelSlice<'static> {
    PixelSlice::new(&STRIDED, 2, 2, 9, PixelDescriptor::RGB8_SRGB).unwrap()
}
fn codecs() -> CodecSet {
    CodecSet::new()
        .with_encoder(ReferenceEncoderConfig::new())
        .with_decoder(ReferenceZcrDecoderConfig)
}

#[test]
fn stored_requests_roundtrip_strided_pixels_and_leave_templates_reusable() {
    let codecs = codecs();
    let encode = EncodeRequest::new(ImageFormat::Pnm).with_fidelity(Fidelity::Lossless);
    let decode = DecodeRequest::new().with_output(PixelDescriptor::RGB8_SRGB);
    let baseline = codecs.encode(ImageFormat::Pnm, pixels()).unwrap();
    for _ in 0..3 {
        let output = encode.encode(&codecs, pixels()).unwrap();
        assert_eq!(output.data(), baseline.data());
        let decoded = decode.decode(&codecs, output.data()).unwrap();
        assert_eq!(decoded.pixels().row(0), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(decoded.pixels().row(1), &[7, 8, 9, 10, 11, 12]);
    }
}

#[test]
fn required_output_rejects_a_native_fallback_but_preferences_allow_it() {
    let codecs = codecs();
    let encoded = codecs.encode(ImageFormat::Pnm, pixels()).unwrap();
    let error = DecodeRequest::new()
        .with_output(PixelDescriptor::RGBA8_SRGB)
        .decode(&codecs, encoded.data())
        .unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<OperationError>(&error),
        Some(OperationError::OutputDescriptor {
            requested: PixelDescriptor::RGBA8_SRGB,
            actual: PixelDescriptor::RGB8_SRGB
        })
    ));
    let decoded = DecodeRequest::new()
        .preferring_output(vec![PixelDescriptor::RGBA8_SRGB])
        .decode(&codecs, encoded.data())
        .unwrap();
    assert_eq!(decoded.pixels().descriptor(), PixelDescriptor::RGB8_SRGB);
    assert_eq!(decoded.pixels().row(1), &[7, 8, 9, 10, 11, 12]);
}

#[test]
fn inherited_policies_are_checked_and_rejection_precedes_codec_input() {
    let codecs = codecs().with_decode_policy(DecodePolicy::strict());
    // Detection succeeds; parsing this deliberately truncated body would fail.
    let error = DecodeRequest::new().decode(&codecs, b"ZCR1").unwrap_err();
    assert_eq!(
        zencodec::find_cause::<UnsupportedOperation>(&error),
        Some(&UnsupportedOperation::DecodePolicy)
    );
    let encoded = codecs.encode(ImageFormat::Pnm, pixels()).unwrap();
    // Explicit replacement clears a default; the failed request did not mutate it.
    DecodeRequest::new()
        .with_policy(DecodePolicy::none())
        .decode(&codecs, encoded.data())
        .unwrap();
    assert_eq!(
        zencodec::find_cause::<UnsupportedOperation>(
            &DecodeRequest::new()
                .decode(&codecs, encoded.data())
                .unwrap_err()
        ),
        Some(&UnsupportedOperation::DecodePolicy)
    );
}

#[test]
fn required_encode_policy_never_uses_the_legacy_best_effort_setter() {
    let codecs = codecs().with_encode_policy(EncodePolicy::none().with_strict(true));
    let error = EncodeRequest::new(ImageFormat::Pnm)
        .encode(&codecs, pixels())
        .unwrap_err();
    assert_eq!(
        zencodec::find_cause::<UnsupportedOperation>(&error),
        Some(&UnsupportedOperation::EncodePolicy)
    );
    let request = EncodeRequest::new(ImageFormat::Pnm).with_policy(EncodePolicy::none());
    let first = request.encode(&codecs, pixels()).unwrap();
    let second = request.encode(&codecs, pixels()).unwrap();
    assert_eq!(first.data(), second.data());
}

#[test]
fn required_fidelity_reports_substitution_without_mutating_the_template() {
    use zencodec::encode::FidelityTargetError;
    let codecs = codecs();
    let before = codecs.encode(ImageFormat::Pnm, pixels()).unwrap();
    let error = EncodeRequest::new(ImageFormat::Pnm)
        .with_fidelity(Fidelity::codec_quality(80.0))
        .encode(&codecs, pixels())
        .unwrap_err();
    assert_eq!(
        zencodec::find_cause::<FidelityTargetError>(&error),
        Some(&FidelityTargetError::Substituted {
            resolved: Fidelity::Lossless
        })
    );
    let after = EncodeRequest::new(ImageFormat::Pnm)
        .with_fidelity(Fidelity::Lossless)
        .encode(&codecs, pixels())
        .unwrap();
    assert_eq!(before.data(), after.data());
}

#[test]
fn controlled_execution_requires_a_qualified_implementation() {
    let set = codecs();
    let format = ReferenceZcrDecoderConfig::formats()[0];
    use zencodec::decode::{ControlError, DecoderConfig};
    let error = match DecodeRequest::new()
        .with_format(format)
        .start_controlled(&set, zencodec::storage::StorageBudget::new(4096, 4))
    {
        Err(error) => error,
        Ok(_) => panic!("ordinary decode is not controlled decode"),
    };
    assert_eq!(
        zencodec::find_cause::<ControlError>(&error),
        Some(&ControlError::NotControlled)
    );
}
