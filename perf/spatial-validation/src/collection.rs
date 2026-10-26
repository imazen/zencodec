//! Native adapter factories and executable consumer examples.
use crate::image_items::ImageRequest;
pub use zencodec::collection::{Collection, CollectionError};
use zencodec::{At, image::ImageOutput};

#[track_caller]
pub fn located(error: crate::tiff_region::Failure) -> At<CollectionError> {
    CollectionError::located(error)
}

pub use consumer::first_image;
mod consumer {
    // EXAMPLE: selected_first_image
    use zencodec::{
        At,
        collection::{Collection, CollectionError, ImageRequest},
        image::ImageOutput,
    };

    // Adapter-level selection returns the shared owner. Consumer publication
    // policy belongs in CheckedCollection, shown below.
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
    use crate::jxl_items::{JxlItems, Limits};
    use zencodec::{
        At, StopToken,
        collection::{Composition, ImageRequest},
        image::ImageOutput,
        operation::{CheckedCollection, CollectionOperationError, Outcome, PolicyResolver},
    };

    // Native factory setup stays in the adapter. Limits are application input:
    // images bounds selected-item storage; frame_headers counts hidden work.
    // The consumer uses the same checked operation for later selections.
    pub fn checked_jxl_first<R: PolicyResolver>(
        bytes: &[u8],
        limits: Limits,
        resolver: R,
        stop: StopToken,
    ) -> Result<Option<Outcome<ImageOutput>>, At<CollectionOperationError>> {
        let mut images = CheckedCollection::open(stop, resolver, |stop| {
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

pub use checked_inspection::checked_first_description;
mod checked_inspection {
    // EXAMPLE: checked_first_description
    use zencodec::{
        At,
        collection::{Collection, ImageDescription, ImageId},
        operation::{CheckedCollection, CollectionOperationError, PolicyResolver},
    };

    // The caller keeps this opened operation, so the returned ID can be used
    // for later region requests. No pixel-format capability query is needed.
    pub fn checked_first_description<C: Collection, R: PolicyResolver>(
        images: &mut CheckedCollection<C, R>,
    ) -> Result<Option<(ImageId, ImageDescription)>, At<CollectionOperationError>> {
        let mut roots = images.images()?;
        let Some(id) = images.next(&mut roots)? else {
            return Ok(None);
        };
        // ANCHOR: checked_first_description
        let description = images.describe(&id)?;
        // END: checked_first_description
        // Stored geometry can guide layout/region choices. A later decode still
        // checks source consistency and must pass publication policy.
        Ok(Some((id, description)))
    }
    // END EXAMPLE: checked_first_description
}

pub use registered_consumer::{checked_registered_first, registered_factories};
mod registered_consumer {
    // EXAMPLE: checked_registered_first
    use crate::{collection_factories::TiffFactory, tiff_region::Allowances};
    use zencodec::{
        At, CodecSet, ImageFormat, StopToken,
        collection::{ImageRequest, OpenRequest},
        image::ImageOutput,
        operation::{CollectionOperationError, Outcome, PolicyResolver},
        source::DynReadAt,
    };

    use zenjxl::collection::JxlCollectionConfig;

    // For example: JxlCollectionConfig::new(64, 256) bounds image identities
    // and processed frame headers separately. Configure these once.
    // JXL uses its real codec package; TIFF
    // remains a private adapter candidate. Both use the shared checked operation.
    pub fn registered_factories(tiff: Allowances, jxl: JxlCollectionConfig) -> CodecSet {
        CodecSet::new()
            .with_collection(TiffFactory(tiff))
            .with_collection(jxl)
    }

    // The caller supplies a stable source: immutable bytes, a conditional HTTP
    // object, or a file held unchanged. Choose one view and discovery allowance.
    pub fn checked_registered_first<R: PolicyResolver>(
        factories: &CodecSet,
        format: ImageFormat,
        source: &mut dyn DynReadAt,
        request: OpenRequest,
        resolver: R,
        stop: StopToken,
    ) -> Result<Option<Outcome<ImageOutput>>, At<CollectionOperationError>> {
        // ANCHOR: checked_registered_first
        let mut images = factories.open_collection(format, source, request, resolver, stop)?;
        let image = images.first_image(&ImageRequest::new())?;
        // END: checked_registered_first
        // The image owns its samples after this operation ends. Applications
        // that retain IDs instead keep the opened operation for later requests.
        Ok(image)
    }
    // END EXAMPLE: checked_registered_first
}

pub use checked_layer_consumer::checked_jxl_layer_first;
mod checked_layer_consumer {
    // EXAMPLE: checked_jxl_layer_first
    use crate::{jxl_items::Limits, jxl_layers::JxlLayers};
    use zencodec::{
        At, StopToken,
        collection::ImageRequest,
        image::ImageOutput,
        operation::{CheckedCollection, CollectionOperationError, Outcome, PolicyResolver},
    };

    // The adapter fixes the uncomposited view before IDs are discovered.
    // This qualified F32 path retains stored orientation and unrendered spots.
    pub fn checked_jxl_layer_first<R: PolicyResolver>(
        bytes: &[u8],
        limits: Limits,
        resolver: R,
        stop: StopToken,
    ) -> Result<Option<Outcome<ImageOutput>>, At<CollectionOperationError>> {
        let mut images =
            CheckedCollection::open(stop, resolver, |stop| JxlLayers::open(bytes, limits, stop))?;
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
