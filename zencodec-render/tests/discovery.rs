//! First-image discovery must distinguish exhaustion from failure.
use zencodec::{
    At, SharedPulse, NoPulse,
    collection::{
        Collection, CollectionError, Composition, ImageDescription, ImageId, ImageRequest,
        SelectionError,
    },
    image::ImageOutput,
    operation::{CheckedCollection, CollectionStage, PolicySet},
    storage::StorageBudget,
};
use zencodec_render::Renderer;

struct Empty {
    fail: bool,
}
impl Collection for Empty {
    type Cursor = ();
    fn view(&self) -> Composition {
        Composition::Composited
    }
    fn images(&mut self) -> Result<(), At<CollectionError>> {
        Ok(())
    }
    fn next(&mut self, _: &mut ()) -> Result<Option<ImageId>, At<CollectionError>> {
        if self.fail {
            Err(At::wrap(CollectionError::Requirement(
                SelectionError::DiscoveryLimit,
            )))
        } else {
            Ok(None)
        }
    }
    fn children(&mut self, _: &ImageId) -> Result<(), At<CollectionError>> {
        panic!("must not select children")
    }
    fn describe(&mut self, _: &ImageId) -> Result<ImageDescription, At<CollectionError>> {
        panic!("no selected image")
    }
    fn decode(
        &mut self,
        _: &ImageId,
        _: &ImageRequest,
    ) -> Result<ImageOutput, At<CollectionError>> {
        panic!("no selected image")
    }
}

#[test]
fn empty_discovery_is_distinct_from_failed_discovery() {
    let storage = StorageBudget::new(0, 0);
    let renderer = Renderer::new(storage.clone());
    for fail in [false, true] {
        let mut images =
            CheckedCollection::open(SharedPulse::default(), PolicySet::new(), |_| {
                Ok(Empty { fail })
            })
            .unwrap();
        let result = renderer.render_first(&mut images);
        if fail {
            let error = result.unwrap_err();
            assert_eq!(error.error().stage, CollectionStage::Discovery);
            assert_eq!(
                zencodec::find_cause::<SelectionError>(&error),
                Some(&SelectionError::DiscoveryLimit)
            );
        } else {
            assert!(result.unwrap().is_none());
        }
        assert_eq!(images.report().decisions().count(), 0);
        assert_eq!(storage.reserved_bytes(), 0);
        assert_eq!(storage.reserved_objects(), 0);
    }
}
