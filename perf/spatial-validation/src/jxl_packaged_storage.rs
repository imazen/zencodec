//! Shared observations for the ordinary/admitted native selection matrix.
pub(crate) use opening::open_admitted;
use zencodec::{image::ImageOutput, storage::StorageBudget};

pub(crate) fn factories(images: usize, headers: u64) -> (zencodec::CodecSet, StorageBudget) {
    // Finite fixture allowance, not a prediction of native/process memory.
    let storage = StorageBudget::new(256 * 1024 * 1024, 64);
    let factories = zencodec::CodecSet::new().with_collection(
        zenjxl::collection::JxlCollectionConfig::new(images, headers),
    );
    (factories, storage)
}

mod opening {
    // EXAMPLE: checked_output_storage
    use zencodec::{
        At, CodecSet, ImageFormat, SharedPulse,
        collection::{Composition, Discovery, ErasedCollection, OpenRequest},
        operation::{CheckedCollection, CollectionOperationError, PolicySet},
        source::DynReadAt,
        storage::StorageBudget,
    };

    pub fn open_admitted<'a>(
        codecs: &CodecSet,
        source: &'a mut dyn DynReadAt,
        view: Composition,
        discovery_attempts: u64,
        storage: &StorageBudget,
    ) -> Result<
        CheckedCollection<Discovery<ErasedCollection<'a>>, PolicySet>,
        At<CollectionOperationError>,
    > {
        // ANCHOR: checked_output_storage
        let request =
            OpenRequest::new(view, discovery_attempts).with_output_storage(storage.clone());
        let opened = codecs.open_collection(
            ImageFormat::Jxl,
            source,
            request,
            PolicySet::new(),
            SharedPulse::default(),
        );
        // END: checked_output_storage
        // Ordinary selection methods now retain output charges automatically.
        // Unsupported adapters return a typed opening refusal. Native workspace
        // remains outside this allowance, even when reconstruction succeeds.
        opened
    }
    // END EXAMPLE: checked_output_storage
}

/// Each argument is a distinct reconstruction, not another clone of one image.
pub(crate) fn assert_charges(storage: &StorageBudget, images: &[&ImageOutput]) {
    let mut bytes = 0;
    let mut objects = 0;
    for image in images {
        let map = image.map().unwrap();
        objects += map.plane_count();
        for index in 0..map.plane_count() {
            let plane = map.plane(index).unwrap().unwrap();
            bytes += plane.samples.stride_bytes() * plane.samples.dimensions().1;
        }
    }
    assert_eq!(
        (storage.reserved_bytes(), storage.reserved_objects()),
        (bytes, objects)
    );
}
