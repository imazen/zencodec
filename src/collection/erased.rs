//! Erase once at opening and once per cursor; preserve native image identities.
//! Native collections may borrow input; their cursor state must be owned.
use crate::{
    At,
    collection::{
        Collection, CollectionError, Composition, ImageDescription, ImageId, ImageRequest, Origin,
        SelectionError,
    },
    image::ImageOutput,
};
use alloc::boxed::Box;
use core::any::Any;

/// Opaque discovery state belonging to one erased collection.
/// Passing it to another opened collection fails before native work.
pub struct ErasedCursor {
    owner: ImageId,
    state: Box<dyn Any>,
}
/// A collection that may borrow its source without borrowing its registry.
///
/// Erasure allocates a small box at opening and per cursor, never per image ID
/// or pixel. These allocations are infallible and are not workspace admission.
/// This form does not promise thread transfer; use a concrete collection when
/// its native type provides that capability.
pub struct ErasedCollection<'a>(Box<dyn Collection<Cursor = ErasedCursor> + 'a>);

impl<'a> ErasedCollection<'a> {
    /// Retain the native collection and its source lifetime. Cursor state must
    /// be owned; native image identities and output owners are unchanged.
    pub fn new<C: Collection + 'a>(collection: C) -> Self
    where
        C::Cursor: 'static,
    {
        Self(Box::new(Erased {
            collection,
            origin: Origin::new(),
        }))
    }
}
impl Collection for ErasedCollection<'_> {
    type Cursor = ErasedCursor;
    fn view(&self) -> Composition {
        self.0.view()
    }
    fn images(&mut self) -> Result<ErasedCursor, At<CollectionError>> {
        self.0.images()
    }
    fn children(&mut self, id: &ImageId) -> Result<ErasedCursor, At<CollectionError>> {
        self.0.children(id)
    }
    fn next(&mut self, cursor: &mut ErasedCursor) -> Result<Option<ImageId>, At<CollectionError>> {
        self.0.next(cursor)
    }
    fn describe(&mut self, id: &ImageId) -> Result<ImageDescription, At<CollectionError>> {
        self.0.describe(id)
    }
    fn decode(
        &mut self,
        id: &ImageId,
        request: &ImageRequest,
    ) -> Result<ImageOutput, At<CollectionError>> {
        self.0.decode(id, request)
    }
}

struct Erased<C> {
    collection: C,
    origin: Origin,
}
impl<C: Collection> Erased<C>
where
    C::Cursor: 'static,
{
    fn cursor(&self, state: C::Cursor) -> ErasedCursor {
        ErasedCursor {
            owner: self.origin.image(0),
            state: Box::new(state),
        }
    }
}
impl<C: Collection> Collection for Erased<C>
where
    C::Cursor: 'static,
{
    type Cursor = ErasedCursor;
    fn view(&self) -> Composition {
        self.collection.view()
    }
    fn images(&mut self) -> Result<ErasedCursor, At<CollectionError>> {
        let cursor = self.collection.images()?;
        Ok(self.cursor(cursor))
    }
    fn children(&mut self, id: &ImageId) -> Result<ErasedCursor, At<CollectionError>> {
        let cursor = self.collection.children(id)?;
        Ok(self.cursor(cursor))
    }
    fn next(&mut self, cursor: &mut ErasedCursor) -> Result<Option<ImageId>, At<CollectionError>> {
        self.origin
            .key(&cursor.owner)
            .map_err(|_| At::wrap(CollectionError::Requirement(SelectionError::ForeignCursor)))?;
        let state = cursor
            .state
            .downcast_mut::<C::Cursor>()
            .ok_or_else(|| At::wrap(CollectionError::Requirement(SelectionError::ForeignCursor)))?;
        self.collection.next(state)
    }
    fn describe(&mut self, id: &ImageId) -> Result<ImageDescription, At<CollectionError>> {
        self.collection.describe(id)
    }
    fn decode(
        &mut self,
        id: &ImageId,
        request: &ImageRequest,
    ) -> Result<ImageOutput, At<CollectionError>> {
        self.collection.decode(id, request)
    }
}
