//! Private registry experiment: erase once at opening and once per cursor.
//! Native collections may borrow input; their cursor state must be owned.
use std::any::Any;
use zencodec::{
    At,
    collection::{
        Collection, CollectionError, Composition, ImageDescription, ImageId, ImageRequest, Origin,
        SelectionError,
    },
    image::ImageOutput,
};

pub struct Cursor {
    owner: ImageId,
    state: Box<dyn Any>,
}
pub struct ErasedCollection<'a>(Box<dyn Collection<Cursor = Cursor> + 'a>);

impl<'a> ErasedCollection<'a> {
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
    type Cursor = Cursor;
    fn view(&self) -> Composition {
        self.0.view()
    }
    fn images(&mut self) -> Result<Cursor, At<CollectionError>> {
        self.0.images()
    }
    fn children(&mut self, id: &ImageId) -> Result<Cursor, At<CollectionError>> {
        self.0.children(id)
    }
    fn next(&mut self, cursor: &mut Cursor) -> Result<Option<ImageId>, At<CollectionError>> {
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
    fn cursor(&self, state: C::Cursor) -> Cursor {
        Cursor {
            owner: self.origin.image(0),
            state: Box::new(state),
        }
    }
}
impl<C: Collection> Collection for Erased<C>
where
    C::Cursor: 'static,
{
    type Cursor = Cursor;
    fn view(&self) -> Composition {
        self.collection.view()
    }
    fn images(&mut self) -> Result<Cursor, At<CollectionError>> {
        let cursor = self.collection.images()?;
        Ok(self.cursor(cursor))
    }
    fn children(&mut self, id: &ImageId) -> Result<Cursor, At<CollectionError>> {
        let cursor = self.collection.children(id)?;
        Ok(self.cursor(cursor))
    }
    fn next(&mut self, cursor: &mut Cursor) -> Result<Option<ImageId>, At<CollectionError>> {
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
