#![cfg(feature = "rewrite")]
use zencodec::metadata_rewrite::{Error, Request};
use zenmeta::rewrite::Context;

#[test]
fn filtering_preserves_orientation_and_rejects_corrupt_pointers() {
    let mut exif = zencodec::Exif::new(zencodec::TextEncoding::Ascii);
    exif.set_orientation(zencodec::Orientation::Rotate90);
    exif.set_artist("private artist");
    let context = Context::new(&Request::new(b"")).unwrap();
    let output = context.exif(&exif.to_bytes()).unwrap();
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
