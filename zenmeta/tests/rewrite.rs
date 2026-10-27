#![cfg(feature = "rewrite")]
use zencodec::metadata_rewrite::{Error, RewriteRequest};
use zenmeta::rewrite::Context;

#[test]
fn filtering_preserves_orientation_and_rejects_corrupt_pointers() {
    let mut exif = zencodec::Exif::new(zencodec::TextEncoding::Ascii);
    exif.set_orientation(zencodec::Orientation::Rotate90);
    exif.set_artist("private artist");
    let context = Context::new(&RewriteRequest::new(b"")).unwrap();
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
            _: &dyn zencodec::Pulse,
        ) -> Result<Option<Vec<u8>>, Error> {
            Ok(Some(vec![0; 17]))
        }
    }
    let request = RewriteRequest::new(b"")
        .with_metadata_limit(16)
        .with_services(&Expands)
        .with_xmp_editing(true);
    assert_eq!(
        Context::new(&request).unwrap().xmp(b"small"),
        Err(Error::Limit(zencodec::LimitKind::MetadataSize))
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
    let pulse = zencodec::how_far::StopOnly::borrowed(&Deadline);
    let request = RewriteRequest::new(b"data").with_pulse(&pulse);
    assert!(matches!(
        Context::new(&request),
        Err(Error::Stopped(zencodec::enough::StopReason::TimedOut))
    ));
}

#[test]
fn preserving_xmp_needs_no_editor_and_keeps_exact_packet_bytes() {
    let request = RewriteRequest::new(b"").with_policy(zencodec::MetadataPolicy::PreserveExact);
    let context = Context::new(&request).unwrap();
    let xml = br#"<packet unusual='retained byte for byte'/>"#;
    assert_eq!(context.xmp(xml).unwrap().as_deref(), Some(xml.as_slice()));
    let context = Context::new(&RewriteRequest::new(b"")).unwrap();
    assert_eq!(context.xmp(xml).unwrap(), None);
    let bounded = request.with_metadata_limit(1);
    assert_eq!(
        Context::new(&bounded).unwrap().xmp(xml),
        Err(Error::Limit(zencodec::LimitKind::MetadataSize))
    );
}

#[test]
fn rejected_carriers_do_not_damage_the_builder() {
    use std::borrow::Cow;
    use zencodec::metadata_rewrite::Action;
    use zenmeta::rewrite::PlanBuilder;
    let bytes = b"1234";
    let request = RewriteRequest::new(bytes).with_byte_limits(4, 4);
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
        Context::new(
            &RewriteRequest::new(b"").with_policy(zencodec::MetadataPolicy::PreserveExact)
        )
        .is_ok()
    );
    assert!(matches!(
        Context::new(&RewriteRequest::new(b"").with_xmp_editing(true)),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        Context::new(&RewriteRequest::new(b"123").with_byte_limits(2, 10)),
        Err(Error::Limit(zencodec::LimitKind::InputSize))
    ));
}

#[test]
fn failed_child_append_does_not_add_partial_reports() {
    use zenmeta::rewrite::PlanBuilder;
    let parent = Context::new(&RewriteRequest::new(b"123456")).unwrap();
    let child = Context::new(&RewriteRequest::new(b"abcdef")).unwrap();
    let mut builder = PlanBuilder::new();
    builder.keep(&parent, 0..1, "prefix").unwrap();
    let mut nested = PlanBuilder::new();
    nested.keep(&child, 0..2, "one").unwrap();
    nested.keep(&child, 2..6, "two").unwrap();
    assert!(builder.append(&parent, nested, 2).is_err());
    let plan = builder.finish().unwrap();
    assert_eq!(plan.to_vec().unwrap(), b"1");
    assert_eq!(plan.report().len(), 1);
}

#[test]
fn equal_length_inputs_cannot_be_mixed_in_one_builder() {
    use zenmeta::rewrite::PlanBuilder;
    let first = Context::new(&RewriteRequest::new(b"123456")).unwrap();
    let other = Context::new(&RewriteRequest::new(b"abcdef")).unwrap();
    let mut builder = PlanBuilder::new();
    builder.keep(&first, 0..3, "one").unwrap();
    assert!(builder.keep(&other, 3..6, "two").is_err());
    assert_eq!(builder.finish().unwrap().to_vec().unwrap(), b"123");
}

#[test]
fn replacing_context_input_cannot_bypass_input_budget() {
    let context = Context::new(
        &RewriteRequest::new(b"1")
            .with_byte_limits(1, 10)
            .with_policy(zencodec::MetadataPolicy::PreserveExact),
    )
    .unwrap()
    .with_input(b"123");
    assert_eq!(
        context.xmp(b"a"),
        Err(Error::Limit(zencodec::LimitKind::InputSize))
    );
}

#[test]
fn overlapping_exif_values_obey_expanded_metadata_budget() {
    let mut exif = b"II*\0\x08\0\0\0".to_vec();
    exif.extend(2u16.to_le_bytes());
    for (index, tag) in [0x013bu16, 0x8298].into_iter().enumerate() {
        exif.extend(tag.to_le_bytes());
        exif.extend(2u16.to_le_bytes());
        exif.extend(130u32.to_le_bytes());
        exif.extend((38u32 + index as u32).to_le_bytes());
    }
    exif.extend(0u32.to_le_bytes());
    exif.extend([b'a'; 130]);
    exif.push(0);
    assert!(exif.len() <= 190);
    let context = Context::new(
        &RewriteRequest::new(b"")
            .with_metadata_limit(190)
            .with_policy(zencodec::MetadataPolicy::PreserveExact),
    )
    .unwrap();
    assert_eq!(
        context.exif(&exif),
        Err(Error::Limit(zencodec::LimitKind::MetadataSize))
    );

    // Identical aliases are shared by the serializer, unlike overlapping windows.
    exif[30..34].copy_from_slice(&38u32.to_le_bytes());
    assert!(context.exif(&exif).unwrap().len() <= 190);
}

#[test]
fn valid_nested_plan_keeps_offsets_and_bytes() {
    use zenmeta::rewrite::PlanBuilder;
    let bytes = b"abcdef";
    let parent = Context::new(&RewriteRequest::new(bytes)).unwrap();
    let child = parent.clone().with_input(&bytes[2..]);
    let mut nested = PlanBuilder::new();
    nested.keep(&child, 0..2, "one").unwrap();
    nested.keep(&child, 2..4, "two").unwrap();
    let mut builder = PlanBuilder::new();
    builder.keep(&parent, 0..2, "prefix").unwrap();
    builder.append(&parent, nested, 2).unwrap();
    let plan = builder.finish().unwrap();
    assert_eq!(plan.to_vec().unwrap(), bytes);
    assert_eq!(plan.report()[1].source_range(), 2..4);
    assert_eq!(plan.report()[2].source_range(), 4..6);
}

#[test]
fn cancellation_prevents_calling_a_packet_service() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct Stop(AtomicBool);
    impl zencodec::enough::Stop for Stop {
        fn check(&self) -> Result<(), zencodec::enough::StopReason> {
            if self.0.load(Ordering::Relaxed) {
                Err(zencodec::enough::StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }
    struct Service(AtomicUsize);
    impl zencodec::metadata_rewrite::Services for Service {
        fn decompress(
            &self,
            _: zencodec::metadata_rewrite::Compression,
            _: &[u8],
            _: usize,
            _: &dyn zencodec::Pulse,
        ) -> Result<Vec<u8>, Error> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(vec![])
        }
    }
    let stop = Stop(AtomicBool::new(false));
    let service = Service(AtomicUsize::new(0));
    let pulse = zencodec::how_far::StopOnly::borrowed(&stop);
    let context = Context::new(
        &RewriteRequest::new(b"")
            .with_pulse(&pulse)
            .with_services(&service),
    )
    .unwrap();
    stop.0.store(true, Ordering::Relaxed);
    assert_eq!(
        context.decompress(zencodec::metadata_rewrite::Compression::Zlib, b"packet"),
        Err(Error::Cancelled)
    );
    assert_eq!(service.0.load(Ordering::Relaxed), 0);
}

#[test]
fn typed_plan_limits_distinguish_carriers_output_and_regenerated_metadata() {
    use std::borrow::Cow;
    use zencodec::{LimitKind, metadata_rewrite::Action};
    use zenmeta::rewrite::PlanBuilder;
    let input = b"abcd";
    let request = RewriteRequest::new(input)
        .with_metadata_limit(3)
        .with_byte_limits(4, 4);
    let context = Context::new(&request).unwrap();
    let mut builder = PlanBuilder::new();
    builder
        .record(
            &context,
            0..2,
            "first",
            Action::Rewrite,
            Some(Cow::Owned(vec![1, 2])),
        )
        .unwrap();
    assert_eq!(
        builder.record(
            &context,
            2..4,
            "second",
            Action::Rewrite,
            Some(Cow::Owned(vec![3, 4]))
        ),
        Err(Error::Limit(LimitKind::MetadataRewriteSize))
    );
    builder.keep(&context, 2..4, "borrowed").unwrap();
    assert_eq!(
        builder.finish().unwrap().to_vec().unwrap(),
        [1, 2, b'c', b'd']
    );
    let context = Context::new(&request.with_byte_limits(4, 1)).unwrap();
    assert_eq!(
        PlanBuilder::new().keep(&context, 0..2, "output"),
        Err(Error::Limit(LimitKind::OutputSize))
    );
    let context = Context::new(&RewriteRequest::new(input).with_carrier_limit(0)).unwrap();
    assert_eq!(
        PlanBuilder::new().keep(&context, 0..1, "carrier"),
        Err(Error::Limit(LimitKind::MetadataCarriers))
    );
}

#[test]
fn nested_plan_rejects_unrelated_equal_bytes_without_mutating_parent() {
    use zenmeta::rewrite::PlanBuilder;
    let bytes = b"abcdef";
    let copied_child = bytes[2..].to_vec();
    let parent = Context::new(&RewriteRequest::new(bytes)).unwrap();
    let child = parent.clone().with_input(&copied_child);
    let mut nested = PlanBuilder::new();
    nested.keep(&child, 0..4, "child").unwrap();
    let mut builder = PlanBuilder::new();
    builder.keep(&parent, 0..2, "prefix").unwrap();
    assert_eq!(
        builder.append(&parent, nested, 2),
        Err(Error::Malformed("contained input identity"))
    );
    builder.keep(&parent, 2..6, "recovery").unwrap();
    let plan = builder.finish().unwrap();
    assert_eq!(plan.to_vec().unwrap(), bytes);
    assert_eq!(plan.report().len(), 2);
}
