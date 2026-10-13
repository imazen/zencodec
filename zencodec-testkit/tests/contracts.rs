use zencodec::animation::FrameDuration as Duration;
use zencodec::decode::{DecodePolicy, DecoderConfig};
use zencodec::encode::{EncodeJob, EncodePolicy, EncoderConfig};
use zencodec::{ColorEmitPolicy, Metadata, MetadataPolicy};
use zencodec_testkit::{
    self as tk, ReferenceDecoderConfig as D, ReferenceEncoderConfig as E, TestImage,
};

fn duration(n: u64, d: u32) -> Duration {
    Duration::new(n, d).unwrap()
}

#[test]
fn exact_timing_checks_boundaries_and_rejected_frame_atomicity() {
    let ms = Duration::from_millis;
    let cases = [
        (duration(1, 30000), None),
        (ms(40), Some(ms(40))),
        (duration(u64::MAX, 1), None),
        (ms(0), Some(ms(0))),
        (ms(u32::MAX), Some(ms(u32::MAX))),
        (duration(1001, 30000), None),
        (ms(60), Some(ms(60))),
    ];
    tk::check_animation_timing(E::new(), D, &TestImage::rgb8_gradient(4, 3), true, &cases).unwrap();
}

#[test]
fn ordinary_timing_preserves_phase_across_rejection() {
    // Hand-derived 1/30-second endpoints rounded to milliseconds: 33,67,100.
    let source = duration(1, 30);
    let ms = Duration::from_millis;
    let cases = [
        (source, Some(ms(33))),
        (duration(u64::MAX, 1), None),
        (source, Some(ms(34))),
        (ms(0), Some(ms(0))),
        (source, Some(ms(33))),
    ];
    tk::check_animation_timing(E::new(), D, &TestImage::rgb8_gradient(4, 3), false, &cases)
        .unwrap();
    // Independent per-frame rounding would report 33 instead of 34: must fail.
    let mut wrong = cases;
    wrong[2].1 = Some(ms(33));
    assert!(
        tk::check_animation_timing(E::new(), D, &TestImage::rgb8_gradient(4, 3), false, &wrong)
            .is_err()
    );
}

#[test]
fn explicit_duration_oracle_catches_lost_timing_and_extra_frames() {
    let image = TestImage::rgb8_gradient(4, 3);
    let bytes = E::new().job().encode(image.as_slice()).unwrap();
    tk::check_animation_durations(D, bytes.data(), &[Duration::from_millis(0)]).unwrap();
    assert!(tk::check_animation_durations(D, bytes.data(), &[duration(1, 30000)]).is_err());
    assert!(tk::check_animation_durations(D, bytes.data(), &[]).is_err());
    assert!(
        tk::check_animation_durations(D, bytes.data(), &[duration(0, 1), duration(0, 1)]).is_err()
    );
}

#[test]
fn checked_encode_policy_verifies_delivery_and_atomic_rejection() {
    let image = TestImage::rgb8_gradient(4, 3);
    let meta = Metadata::none().with_xmp(b"private".as_slice());
    let strip = EncodePolicy::none().with_embed_xmp(false);
    for request in [
        EncodePolicy::none(),
        EncodePolicy::none().with_embed_xmp(true),
    ] {
        tk::check_encode_policy_admission(
            E::new(),
            D,
            &image,
            meta.clone(),
            EncodePolicy::none(),
            request,
            true,
            &meta,
        )
        .unwrap();
    }
    tk::check_encode_policy_admission(
        E::new(),
        D,
        &image,
        meta.clone(),
        EncodePolicy::none(),
        strip,
        true,
        &Metadata::none(),
    )
    .unwrap();
    let mixed = EncodePolicy::none()
        .with_embed_xmp(true)
        .with_color(ColorEmitPolicy::Compact);
    tk::check_encode_policy_admission(
        E::new(),
        D,
        &image,
        meta.clone(),
        strip,
        mixed,
        false,
        &Metadata::none(),
    )
    .unwrap();
    assert!(
        tk::check_encode_policy_admission(
            E::new(),
            D,
            &image,
            meta.clone(),
            strip,
            mixed,
            false,
            &meta
        )
        .is_err()
    );
    assert!(
        tk::check_encode_policy_admission(
            E::new(),
            D,
            &image,
            meta.clone(),
            strip,
            mixed,
            true,
            &meta
        )
        .is_err()
    );
}

#[test]
fn checked_decode_policy_and_inventory_boundaries() {
    let image = TestImage::rgb8_gradient(4, 3);
    let meta = Metadata::none().with_xmp(b"retained".as_slice());
    let bytes = E::new()
        .job()
        .with_metadata_policy(meta.clone(), MetadataPolicy::PreserveExact)
        .encode(image.as_slice())
        .unwrap();
    tk::check_decode_policy_admission(
        D,
        bytes.data(),
        DecodePolicy::none(),
        DecodePolicy::strict(),
        false,
        &meta,
    )
    .unwrap();
    tk::check_decode_policy_admission(
        D,
        bytes.data(),
        DecodePolicy::none(),
        DecodePolicy::none(),
        true,
        &meta,
    )
    .unwrap();
    assert!(
        tk::check_decode_policy_admission(
            D,
            bytes.data(),
            DecodePolicy::none(),
            DecodePolicy::strict(),
            true,
            &meta
        )
        .is_err()
    );
    tk::check_inventory_input_limit(D, bytes.data()).unwrap();
    tk::check_inventory_dispatch(D, bytes.data()).unwrap();
    assert!(D::capabilities().inventory());
}

fn rewrite_identity<'a>(
    request: &zencodec::metadata_rewrite::Request<'a>,
) -> Result<zencodec::metadata_rewrite::Plan<'a>, zencodec::metadata_rewrite::Error> {
    let context = zenmeta::rewrite::Context::new(request)?;
    let mut builder = zenmeta::rewrite::PlanBuilder::new();
    builder.keep(&context, 0..request.input().len(), "payload")?;
    builder.finish()
}

#[test]
fn rewrite_harness_catches_ignored_limits_cancellation_and_changed_output() {
    use std::borrow::Cow;
    use zencodec::metadata_rewrite::{Plan, Request};
    let bytes = b"fixture payload";
    let observe = |actual: &[u8]| {
        if actual == bytes {
            Ok(())
        } else {
            Err(tk::Failure {
                check: "payload",
                detail: "changed".into(),
            })
        }
    };
    tk::check_metadata_rewrite(Request::new(bytes), rewrite_identity, observe).unwrap();
    // A planner that silently bypasses request admission must fail the harness.
    assert!(
        tk::check_metadata_rewrite(
            Request::new(bytes),
            |r| Plan::from_parts(r.input().len(), vec![Cow::Borrowed(r.input())], vec![]),
            observe
        )
        .is_err()
    );
    // Test each failure independently so one failing check cannot mask another.
    for mutation in 0..4 {
        let broken = |r: &Request<'static>| {
            let changed = match mutation {
                0 => r.clone().with_byte_limits(usize::MAX, r.max_output()),
                1 => r.clone().with_byte_limits(r.max_input(), usize::MAX),
                2 => r.clone().with_carrier_limit(usize::MAX),
                _ => r.clone().with_stop(&zencodec::enough::Unstoppable),
            };
            rewrite_identity(&changed)
        };
        assert!(
            tk::check_metadata_rewrite(Request::new(bytes), broken, observe).is_err(),
            "mutation {mutation}"
        );
    }
    assert!(
        tk::check_metadata_rewrite(Request::new(bytes), rewrite_identity, |_| Err(
            tk::Failure {
                check: "oracle",
                detail: "pixels changed".into()
            }
        ))
        .is_err()
    );
}

#[test]
fn undeclared_timing_check_rejects_a_timing_enabled_adapter() {
    assert!(
        tk::check_animation_timing_unsupported(E::new(), D, &TestImage::rgb8_gradient(4, 3))
            .is_err()
    );
}
