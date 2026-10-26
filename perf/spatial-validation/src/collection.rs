//! One consumer path for the candidate; metadata/pixel unification is not yet
//! qualified, so the decoded output remains an associated native sample type.
use super::{
    image_items::{Composition, ImageId, SelectionError},
    tiff_region::Failure,
};
use std::{error::Error, fmt};
use zencodec::At;

#[derive(Debug)]
pub enum CollectionError {
    Requirement(SelectionError),
    Backend(Failure),
}
impl fmt::Display for CollectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Requirement(e) => e.fmt(f),
            Self::Backend(e) => e.fmt(f),
        }
    }
}
impl Error for CollectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Requirement(e) => e,
            Self::Backend(e) => e.as_ref(),
        })
    }
}
#[track_caller]
pub fn located(error: Failure) -> At<CollectionError> {
    At::wrap(match error.downcast::<SelectionError>() {
        Ok(error) => CollectionError::Requirement(*error),
        Err(error) => CollectionError::Backend(error),
    })
}

/// Opening the concrete adapter fixes its view. Discovery can then be lazy and
/// selection IDs remain stable while the source operation lives.
pub trait Collection {
    type Cursor;
    type Samples;
    fn view(&self) -> Composition;
    fn images(&mut self) -> Result<Self::Cursor, At<CollectionError>>;
    fn children(&mut self, image: &ImageId) -> Result<Self::Cursor, At<CollectionError>>;
    fn next(&mut self, cursor: &mut Self::Cursor) -> Result<Option<ImageId>, At<CollectionError>>;
    fn decode(&mut self, image: &ImageId) -> Result<Self::Samples, At<CollectionError>>;
}

pub use consumer::first_image;
mod consumer {
    // EXAMPLE: selected_first_image
    use super::{Collection, CollectionError};
    use zencodec::At;

    // This private experiment keeps native Samples until interpretation and
    // zenpixels output are qualified. It is not a public CodecSet entry point.
    pub fn first_image<C: Collection>(
        collection: &mut C,
    ) -> Result<Option<C::Samples>, At<CollectionError>> {
        // ANCHOR: selected_first_image
        let mut cursor = collection.images()?;
        let Some(image) = collection.next(&mut cursor)? else {
            return Ok(None);
        };
        let samples = collection.decode(&image)?;
        // END: selected_first_image
        Ok(Some(samples))
    }
    // END EXAMPLE: selected_first_image
}

/// A second, format-independent use case: select a child of the first image.
/// A flat-only adapter must refuse child discovery, rather than report zero.
pub fn first_child<C: Collection>(
    collection: &mut C,
) -> Result<Option<C::Samples>, At<CollectionError>> {
    let mut roots = collection.images()?;
    let Some(parent) = collection.next(&mut roots)? else {
        return Ok(None);
    };
    let mut children = collection.children(&parent)?;
    collection
        .next(&mut children)?
        .map(|id| collection.decode(&id))
        .transpose()
}
