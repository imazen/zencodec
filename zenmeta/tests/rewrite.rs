#![cfg(feature = "rewrite")]
use zencodec::metadata_rewrite::{Error, Request};
use zenmeta::rewrite::Context;

#[test]
fn filtering_preserves_orientation_and_rejects_corrupt_pointers() {
    let mut exif = zencodec::Exif::new(zencodec::TextEncoding::Ascii);
    exif.set_orientation(zencodec::Orientation::Rotate90);
    exif.set_artist("private artist");
    let context = Context::new(&Request::new(b"")).unwrap();
    let output = context.exif(&zenmeta::exif::serialize(&exif)).unwrap();
    let parsed = zencodec::Exif::parse(&output).unwrap();
    assert_eq!(parsed.orientation(), Some(zencodec::Orientation::Rotate90));
    assert!(!output.windows(14).any(|w| w == b"private artist"));
    assert!(context.exif(b"II*\0\xff\xff\xff\xff").is_err());
}

#[test]
fn metadata_services_cannot_return_over_budget_packets() {
    struct Expands;
    impl zencodec::metadata_rewrite::Services for Expands {
        fn rewrite_xmp(
            &self,
            _: &[u8],
            _: usize,
            _: &dyn zencodec::enough::Stop,
        ) -> Result<Option<Vec<u8>>, Error> {
            Ok(Some(vec![0; 17]))
        }
    }
    let request = Request::new(b"")
        .with_metadata_limit(16)
        .with_services(&Expands)
        .with_xmp_editing(true);
    assert_eq!(
        Context::new(&request).unwrap().xmp(b"small"),
        Err(Error::Limit("metadata bytes"))
    );
}

#[test]
fn cancellation_retains_deadline_reason() {
    struct Deadline;
    impl zencodec::enough::Stop for Deadline {
        fn check(&self) -> Result<(), zencodec::enough::StopReason> {
            Err(zencodec::enough::StopReason::TimedOut)
        }
    }
    let request = Request::new(b"data").with_stop(&Deadline);
    assert!(matches!(
        Context::new(&request),
        Err(Error::Stopped(zencodec::enough::StopReason::TimedOut))
    ));
}

#[test]
fn preserving_xmp_needs_no_editor_and_keeps_exact_packet_bytes() {
    let request = Request::new(b"").with_policy(zencodec::MetadataPolicy::PreserveExact);
    let context = Context::new(&request).unwrap();
    let xml = br#"<packet unusual='retained byte for byte'/>"#;
    assert_eq!(context.xmp(xml).unwrap().as_deref(), Some(xml.as_slice()));
    let context = Context::new(&Request::new(b"")).unwrap();
    assert_eq!(context.xmp(xml).unwrap(), None);
    let bounded = request.with_metadata_limit(1);
    assert_eq!(
        Context::new(&bounded).unwrap().xmp(xml),
        Err(Error::Limit("metadata bytes"))
    );
}

#[test]
fn rejected_carriers_do_not_damage_the_builder() {
    use std::borrow::Cow;
    use zencodec::metadata_rewrite::Action;
    use zenmeta::rewrite::PlanBuilder;
    let bytes = b"1234";
    let request = Request::new(bytes).with_byte_limits(4, 4);
    let context = Context::new(&request).unwrap();
    let mut builder = PlanBuilder::new();
    assert!(
        builder
            .record(&context, 0..5, "bad", Action::Remove, None)
            .is_err()
    );
    assert!(
        builder
            .record(
                &context,
                0..4,
                "large",
                Action::Rewrite,
                Some(Cow::Owned(vec![0; 5]))
            )
            .is_err()
    );
    builder.keep(&context, 0..4, "image").unwrap();
    let plan = builder.finish().unwrap();
    assert_eq!(plan.report().len(), 1);
    assert_eq!(plan.to_vec().unwrap(), bytes);
}

#[test]
fn empty_and_service_requests_are_validated_by_zenmeta() {
    assert!(
        Context::new(&Request::new(b"").with_policy(zencodec::MetadataPolicy::PreserveExact))
            .is_ok()
    );
    assert!(matches!(
        Context::new(&Request::new(b"").with_xmp_editing(true)),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        Context::new(&Request::new(b"123").with_byte_limits(2, 10)),
        Err(Error::Limit("input bytes"))
    ));
}
