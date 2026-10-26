//! Operation-wide accounting of calls into native collection discovery.
use super::{Collection, CollectionError, Composition, ImageId, ImageRequest};
use crate::{At, image::ImageOutput};
use alloc::boxed::Box;

/// Which call exhausted the operation's discovery allowance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DiscoveryMethod {
    /// Starting top-level discovery.
    Images,
    /// Starting child discovery.
    Children,
    /// Advancing a cursor, including cached or exhausted cursors.
    Next,
}

/// Counts from delegated discovery calls, not native parse work or source bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct DiscoveryUsage {
    /// Charged calls, including failures and successful exhaustion.
    pub attempts: u64,
    /// Successful identity returns; repeated identities count again.
    pub returned_ids: u64,
    /// Successful end-of-cursor returns; repeated exhaustion counts again.
    pub exhausted_cursors: u64,
    /// Errors returned by delegated discovery, including request refusals.
    pub backend_errors: u64,
}

/// A refused discovery call. The adapter was not called and no work was refunded.
#[derive(Debug)]
#[non_exhaustive]
pub struct DiscoveryExhausted {
    /// Method refused before delegation.
    pub method: DiscoveryMethod,
    /// Immutable operation allowance.
    pub limit: u64,
    /// Usage retained at refusal.
    pub used: DiscoveryUsage,
}
impl core::fmt::Display for DiscoveryExhausted {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "discovery {:?} refused after {} of {} allowed attempts",
            self.method, self.used.attempts, self.limit
        )
    }
}
impl core::error::Error for DiscoveryExhausted {}

/// One immutable allowance per opened collection. Charge before delegating,
/// including cached results, exhaustion, unsupported requests and native errors.
/// Starting another cursor does not reset it; failures do not refund work.
/// Decode of an already returned identity does not consume discovery attempts.
/// Native parsing limits, cancellation and source I/O limits remain necessary.
pub struct Discovery<C> {
    collection: C,
    limit: u64,
    used: DiscoveryUsage,
}
impl<C> Discovery<C> {
    /// Bind one allowance to the opened collection, shared by all its cursors.
    pub fn new(collection: C, attempts: u64) -> Self {
        Self {
            collection,
            limit: attempts,
            used: DiscoveryUsage::default(),
        }
    }
    /// Inspect counters without spending an additional discovery attempt.
    pub fn usage(&self) -> DiscoveryUsage {
        self.used
    }
    #[track_caller]
    fn charge(&mut self, method: DiscoveryMethod) -> Result<(), At<CollectionError>> {
        if self.used.attempts == self.limit {
            return Err(At::wrap(CollectionError::Backend(Box::new(
                DiscoveryExhausted {
                    method,
                    limit: self.limit,
                    used: self.used,
                },
            ))));
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
        self.charge(DiscoveryMethod::Images)?;
        let result = self.collection.images();
        self.record(&result);
        result
    }
    fn children(&mut self, image: &ImageId) -> Result<Self::Cursor, At<CollectionError>> {
        self.charge(DiscoveryMethod::Children)?;
        let result = self.collection.children(image);
        self.record(&result);
        result
    }
    fn next(&mut self, cursor: &mut Self::Cursor) -> Result<Option<ImageId>, At<CollectionError>> {
        self.charge(DiscoveryMethod::Next)?;
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
