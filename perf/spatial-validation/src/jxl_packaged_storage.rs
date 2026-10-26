//! Shared observations for the ordinary/admitted native selection matrix.
pub(crate) use registration::factories;
use zencodec::{image::ImageOutput, storage::StorageBudget};

mod registration {
    // EXAMPLE: checked_output_storage
    use zencodec::{CodecSet, storage::StorageBudget};

    pub fn factories(images: usize, headers: u64) -> (CodecSet, StorageBudget) {
        // A finite fixture allowance, not a prediction of native/process memory.
        // Exact boundary and contention controls live in the package consumer tests.
        // ANCHOR: checked_output_storage
        let storage = StorageBudget::new(256 * 1024 * 1024, 64);
        let factories = CodecSet::new().with_collection(
            zenjxl::collection::JxlCollectionConfig::new(images, headers)
                .with_output_storage(storage.clone()),
        );
        // END: checked_output_storage
        // Images keep their charge after the registry drops; hold this handle to
        // observe it. Native workspace is outside this output payload allowance.
        (factories, storage)
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
