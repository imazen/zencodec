//! Required density cannot silently take an unsupported default path.
use zencodec::{
    CodecSet, ImageFormat, Resolution, ResolutionError, ResolutionUnit,
    encode::{DynEncoderConfig, EncodeJob, Encoder, EncoderConfig},
    operation::{Decision, EncodeRequest, OperationError, Question},
};
use zencodec_testkit::ReferenceEncoderConfig;
use zenpixels::{PixelDescriptor, PixelSlice};

fn pixels() -> PixelSlice<'static> {
    PixelSlice::new(
        &[1, 2, 3, 99, 98, 97, 4, 5, 6, 96, 95, 94],
        1,
        2,
        6,
        PixelDescriptor::RGB8_SRGB,
    )
    .unwrap()
}
fn density(x: f64) -> Resolution {
    Resolution {
        x,
        y: 150.0,
        unit: ResolutionUnit::Inch,
    }
}
#[test]
fn unsupported_generic_and_erased_setters_refuse_without_damaging_jobs() {
    let config = ReferenceEncoderConfig::new();
    let baseline = config.clone().encode(pixels()).unwrap();
    let mut job = config.clone().job();
    assert_eq!(
        job.try_set_resolution(density(300.0)),
        Err(ResolutionError::Unsupported)
    );
    assert_eq!(
        job.try_set_resolution(density(f64::NAN)),
        Err(ResolutionError::InvalidValue)
    );
    assert_eq!(
        job.encoder().unwrap().encode(pixels()).unwrap().data(),
        baseline.data()
    );

    let mut job = config.dyn_job();
    for (x, reason) in [
        (300.0, ResolutionError::Unsupported),
        (f64::NAN, ResolutionError::InvalidValue),
    ] {
        let error = job.try_set_resolution(density(x)).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<ResolutionError>(&*error),
            Some(&reason)
        );
    }
    assert_eq!(job.encode(pixels()).unwrap().data(), baseline.data());
    assert!(matches!(
        config.job().try_with_resolution(density(300.0)),
        Err(ResolutionError::Unsupported)
    ));
}
#[test]
fn stored_resolution_rejects_before_callback_and_registered_templates_remain_reusable() {
    let codecs = CodecSet::new().with_send_encoder(ReferenceEncoderConfig::new());
    let baseline = codecs.encode(ImageFormat::Pnm, pixels()).unwrap();
    let request = EncodeRequest::new(ImageFormat::Pnm).with_resolution(density(300.0));
    let mut calls = 0;
    let error = request
        .encode_with(&codecs, pixels(), &mut |_: Question<'_>| -> Result<
            Decision,
            zencodec::At<core::convert::Infallible>,
        > {
            calls += 1;
            Ok(Decision::UsePreset)
        })
        .unwrap_err();
    assert_eq!(calls, 0);
    let OperationError::NoEncodeRoute(routes) = error.error() else {
        panic!("missing preparation refusal")
    };
    assert_eq!(routes.len(), 1);
    assert_eq!(
        zencodec::find_cause::<ResolutionError>(&routes[0].cause),
        Some(&ResolutionError::Unsupported)
    );
    let sent = request.prepare_send(&codecs).unwrap_err();
    assert!(matches!(sent.error(), OperationError::NoEncodeRoute(_)));
    assert_eq!(
        codecs.encode(ImageFormat::Pnm, pixels()).unwrap().data(),
        baseline.data()
    );

    // An invalid request fails before even trying to find a registered encoder.
    let invalid = EncodeRequest::new(ImageFormat::Png).with_resolution(density(0.0));
    for error in [
        invalid.prepare(&CodecSet::new()).unwrap_err(),
        invalid.prepare_send(&CodecSet::new()).unwrap_err(),
    ] {
        assert_eq!(
            zencodec::find_cause::<ResolutionError>(&error),
            Some(&ResolutionError::InvalidValue)
        );
    }
}
