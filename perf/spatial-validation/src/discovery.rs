//! Native tests exercise the shared discovery allowance.
use super::{collection::Collection, image_items::ImageRequest};
pub use zencodec::collection::{
    Discovery, DiscoveryExhausted as Exhausted, DiscoveryMethod as Method,
};

/// Same consumer contract executed by native TIFF and JXL tests below.
pub(crate) fn verify_repeated_discovery(collection: impl Collection) {
    let mut collection = Discovery::new(collection, 4);
    let mut first = collection.images().unwrap();
    let id = collection.next(&mut first).unwrap().unwrap();
    let mut repeated = collection.images().unwrap();
    assert_eq!(collection.next(&mut repeated).unwrap(), Some(id.clone()));
    let before = collection.usage();
    assert_eq!(before.attempts, 4);
    assert_eq!(before.returned_ids, 2);
    // Retries and starting another cursor cannot evade the operation allowance.
    for _ in 0..2 {
        let error = collection.next(&mut repeated).unwrap_err();
        let cause = zencodec::find_cause::<Exhausted>(&error).unwrap();
        assert_eq!(cause.method, Method::Next);
        assert_eq!(cause.limit, 4);
        assert_eq!(cause.used, before);
        assert!(collection.images().is_err());
    }
    // Discovery exhaustion does not revoke an identity already returned.
    let a = collection.decode(&id, &ImageRequest::default()).unwrap();
    let b = collection.decode(&id, &ImageRequest::default()).unwrap();
    assert_eq!(
        super::output_tests::mapped_planes(&a),
        super::output_tests::mapped_planes(&b)
    );
    assert_eq!(collection.usage(), before);
}

pub(crate) fn verify_exhaustion_is_charged(collection: impl Collection) {
    let mut collection = Discovery::new(collection, 128);
    let mut cursor = collection.images().unwrap();
    let mut returned = 0;
    while collection.next(&mut cursor).unwrap().is_some() {
        returned += 1;
    }
    assert_eq!(collection.usage().attempts, returned + 2);
    assert_eq!(collection.usage().returned_ids, returned);
    assert_eq!(collection.usage().exhausted_cursors, 1);
    while collection.usage().attempts < 128 {
        assert_eq!(collection.next(&mut cursor).unwrap(), None);
    }
    let before = collection.usage();
    assert_eq!(before.backend_errors, 0);
    let error = collection.next(&mut cursor).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<Exhausted>(&error).unwrap().used,
        before
    );
    assert_eq!(collection.usage(), before);
}
