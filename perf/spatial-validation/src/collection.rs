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

pub use checked_consumer::checked_jxl_first;
mod checked_consumer {
    // EXAMPLE: checked_jxl_first
    use crate::{
        checked_collection::{Checked, Error},
        image_items::{Composition, ImageRequest},
        jxl_items::{JxlItems, Limits},
    };
    use zencodec::{
        At, StopToken,
        image::ImageOutput,
        operation::{Outcome, PolicyResolver},
    };

    // Native factory setup stays in the adapter. Limits are application input:
    // images bounds selected-item storage; frame_headers counts hidden work.
    // The consumer uses the same checked operation for later selections.
    pub fn checked_jxl_first<R: PolicyResolver>(
        bytes: &[u8],
        limits: Limits,
        resolver: R,
        stop: StopToken,
    ) -> Result<Option<Outcome<ImageOutput>>, At<Error>> {
        let mut images = Checked::open(stop, resolver, |stop| {
            JxlItems::open_with_stop(bytes, Composition::Composited, limits, stop)
        })?;
        // ANCHOR: checked_jxl_first
        let image = images.first_image(&ImageRequest::default())?;
        // END: checked_jxl_first
        // The result owns its samples after this opened collection is dropped.
        // Policy ran before returning it; no separate authorization call follows.
        Ok(image)
    }
    // END EXAMPLE: checked_jxl_first
}

pub use checked_layer_consumer::checked_jxl_layer_first;
mod checked_layer_consumer {
    // EXAMPLE: checked_jxl_layer_first
    use crate::{
        checked_collection::{Checked, Error},
        image_items::ImageRequest,
        jxl_items::Limits,
        jxl_layers::JxlLayers,
    };
    use zencodec::{
        At, StopToken,
        image::ImageOutput,
        operation::{Outcome, PolicyResolver},
    };

    // The adapter fixes the uncomposited view before IDs are discovered.
    // This qualified F32 path retains stored orientation and unrendered spots.
    pub fn checked_jxl_layer_first<R: PolicyResolver>(
        bytes: &[u8],
        limits: Limits,
        resolver: R,
        stop: StopToken,
    ) -> Result<Option<Outcome<ImageOutput>>, At<Error>> {
        let mut images =
            Checked::open(stop, resolver, |stop| JxlLayers::open(bytes, limits, stop))?;
        // ANCHOR: checked_jxl_layer_first
        let layer = images.first_image(&ImageRequest::default())?;
        // END: checked_jxl_layer_first
        // The result retains the full local samples, source blend facts, signed
        // canvas placement and explicit CMYK polarity. It is not a flat preview.
        Ok(layer)
    }
    // END EXAMPLE: checked_jxl_layer_first
}

#[test]
fn raw_collection_guide_uses_the_same_checked_consumer_call() {
    let bytes = crate::jxl_tests::fixture("conformance_test_images/cmyk_layers.jxl");
    let result = checked_jxl_layer_first(
        &bytes,
        crate::jxl_items::Limits {
            images: 64,
            frame_headers: Some(64),
        },
        zencodec::operation::PolicySet::default(),
        zencodec::StopToken::new(zencodec::Unstoppable),
    )
    .unwrap()
    .unwrap();
    result
        .output()
        .with_pixels(|p| {
            assert_eq!(
                p.descriptor().format,
                zenpixels::PixelFormat::CmykF32Inverted
            )
        })
        .unwrap();
    assert_eq!(
        result
            .output()
            .interpretation()
            .source_details
            .get::<crate::jxl_output::JxlRawLayer>()
            .unwrap()
            .native
            .origin,
        (0, 0)
    );
}
