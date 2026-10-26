//! A shared allowance for caller-driven discovery, independent of native caches.
//! This bounds calls into the adapter, not the work performed inside one call.
use super::{
    collection::{Collection, CollectionError},
    image_items::{Composition, ImageId, ImageRequest},
    image_output::ImageOutput,
};
use zencodec::At;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Images,
    Children,
    Next,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub attempts: u64,
    pub returned_ids: u64,
    pub exhausted_cursors: u64,
    pub backend_errors: u64,
}

#[derive(Debug)]
pub struct Exhausted {
    pub method: Method,
    pub limit: u64,
    pub used: Usage,
}
impl std::fmt::Display for Exhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "discovery {:?} refused after {} of {} allowed attempts",
            self.method, self.used.attempts, self.limit
        )
    }
}
impl std::error::Error for Exhausted {}

/// One immutable allowance per opened collection. Charge before delegating,
/// including cached results, exhaustion, unsupported requests and native errors.
/// Starting another cursor does not reset it; failures do not refund work.
/// Decode of an already returned identity does not consume discovery attempts.
/// Native parsing limits, cancellation and source I/O limits remain necessary.
pub struct Discovery<C> {
    collection: C,
    limit: u64,
    used: Usage,
}
impl<C> Discovery<C> {
    pub fn new(collection: C, attempts: u64) -> Self {
        Self {
            collection,
            limit: attempts,
            used: Usage::default(),
        }
    }
    pub fn usage(&self) -> Usage {
        self.used
    }
    #[track_caller]
    fn charge(&mut self, method: Method) -> Result<(), At<CollectionError>> {
        if self.used.attempts == self.limit {
            return Err(At::wrap(CollectionError::Backend(Box::new(Exhausted {
                method,
                limit: self.limit,
                used: self.used,
            }))));
        }
        self.used.attempts += 1;
        Ok(())
    }
    fn record<T>(&mut self, result: &Result<T, At<CollectionError>>) {
        if result.is_err() {
            self.used.backend_errors += 1;
        }
    }
}
impl<C: Collection> Collection for Discovery<C> {
    type Cursor = C::Cursor;
    fn view(&self) -> Composition {
        self.collection.view()
    }
    fn images(&mut self) -> Result<Self::Cursor, At<CollectionError>> {
        self.charge(Method::Images)?;
        let result = self.collection.images();
        self.record(&result);
        result
    }
    fn children(&mut self, image: &ImageId) -> Result<Self::Cursor, At<CollectionError>> {
        self.charge(Method::Children)?;
        let result = self.collection.children(image);
        self.record(&result);
        result
    }
    fn next(&mut self, cursor: &mut Self::Cursor) -> Result<Option<ImageId>, At<CollectionError>> {
        self.charge(Method::Next)?;
        let result = self.collection.next(cursor);
        match &result {
            Ok(Some(_)) => self.used.returned_ids += 1,
            Ok(None) => self.used.exhausted_cursors += 1,
            Err(_) => self.record(&result),
        }
        result
    }
    fn decode(
        &mut self,
        image: &ImageId,
        request: &ImageRequest,
    ) -> Result<ImageOutput, At<CollectionError>> {
        self.collection.decode(image, request)
    }
}

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
