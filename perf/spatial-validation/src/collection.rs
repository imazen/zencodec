//! One consumer path for selected images with mapped samples and interpretation.
//! This remains a private native-codec experiment, not a public CodecSet API.
use super::{
    image_items::{Composition, ImageId, ImageRequest, SelectionError},
    image_output::ImageOutput,
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
/// The private adapters accept the existing StopToken at open and check it
/// during discovery/replay and before returning an ID, exhaustion or output.
/// This is cooperative cancellation, not a hard native-call latency bound.
pub trait Collection {
    type Cursor;
    fn view(&self) -> Composition;
    fn images(&mut self) -> Result<Self::Cursor, At<CollectionError>>;
    fn children(&mut self, image: &ImageId) -> Result<Self::Cursor, At<CollectionError>>;
    fn next(&mut self, cursor: &mut Self::Cursor) -> Result<Option<ImageId>, At<CollectionError>>;
    fn decode(
        &mut self,
        image: &ImageId,
        request: &ImageRequest,
    ) -> Result<ImageOutput, At<CollectionError>>;
}

pub use consumer::first_image;
mod consumer {
    // EXAMPLE: selected_first_image
    use super::{Collection, CollectionError, ImageOutput, ImageRequest};
    use zencodec::At;

    // This private experiment returns a common mapped image. Native layers,
    // metadata coverage and resource bounds remain public promotion gates.
    pub fn first_image<C: Collection>(
        collection: &mut C,
        request: &ImageRequest,
    ) -> Result<Option<ImageOutput>, At<CollectionError>> {
        // ANCHOR: selected_first_image
        let mut cursor = collection.images()?;
        let Some(image) = collection.next(&mut cursor)? else {
            return Ok(None);
        };
        let samples = collection.decode(&image, request)?;
        // END: selected_first_image
        Ok(Some(samples))
    }
    // END EXAMPLE: selected_first_image
}

/// A second, format-independent use case: select a child of the first image.
/// A flat-only adapter must refuse child discovery, rather than report zero.
pub fn first_child<C: Collection>(
    collection: &mut C,
    request: &ImageRequest,
) -> Result<Option<ImageOutput>, At<CollectionError>> {
    let mut roots = collection.images()?;
    let Some(parent) = collection.next(&mut roots)? else {
        return Ok(None);
    };
    let mut children = collection.children(&parent)?;
    collection
        .next(&mut children)?
        .map(|id| collection.decode(&id, request))
        .transpose()
}
