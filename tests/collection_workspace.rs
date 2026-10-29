//! Consumer-level deadlines: registry factories and retained decode owners.
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zencodec::{
    At, CodecSet, ImageFormat, SharedPulse,
    collection::{
        Cause, Collection, CollectionError, CollectionFactory, CollectionOpenContext, Composition,
        ErasedCollection, ImageDescription, ImageId, ImageRequest, OpenRequest, Origin,
        SelectionError,
    },
    image::ImageOutput,
    operation::{
        BoundPolicy, CheckedCollection, Decision, PolicyRequirement, PolicySet, Question,
        RequirementEvidence, UnmetRequirements,
    },
    source::{DynReadAt, ReadAtShim, SliceReadAt},
};

#[derive(Debug)]
struct NativeFailure;
impl std::fmt::Display for NativeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native factory reached")
    }
}
impl std::error::Error for NativeFailure {}
struct Factory(Arc<AtomicUsize>);
impl CollectionFactory for Factory {
    fn formats(&self) -> &[ImageFormat] {
        &[ImageFormat::Tiff]
    }
    fn open_with<'a>(
        &self,
        _: &'a mut dyn DynReadAt,
        _: CollectionOpenContext,
    ) -> Result<ErasedCollection<'a>, Cause> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Box::new(NativeFailure))
    }
}
fn assert_workspace(error: &(dyn std::error::Error + 'static)) {
    let unmet = zencodec::find_cause::<UnmetRequirements>(error).expect("typed admission cause");
    assert_eq!(
        unmet.iter().collect::<Vec<_>>(),
        vec![(PolicyRequirement::Workspace, RequirementEvidence::Unknown)]
    );
}
#[test]
fn registry_admits_before_registered_factory_and_preserves_default_native_error() {
    let calls = Arc::new(AtomicUsize::new(0));
    let codecs = CodecSet::new().with_collection(Factory(calls.clone()));
    for limit in [0, 1, u64::MAX] {
        let mut source = ReadAtShim(SliceReadAt::new(&[]));
        let policy = BoundPolicy::new(
            PolicySet::new().with_workspace_limit(limit),
            |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                panic!("no callback before admission")
            },
        );
        let result = codecs.open_collection(
            ImageFormat::Tiff,
            &mut source,
            OpenRequest::new(Composition::Composited, 4),
            policy,
            SharedPulse::default(),
        );
        let error = result
            .err()
            .expect("unknown bound must refuse every explicit cap");
        assert_workspace(&error);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    let mut source = ReadAtShim(SliceReadAt::new(&[]));
    let error = codecs
        .open_collection(
            ImageFormat::Tiff,
            &mut source,
            OpenRequest::new(Composition::Composited, 4),
            PolicySet::new(),
            SharedPulse::default(),
        )
        .err()
        .expect("native factory error");
    assert!(zencodec::find_cause::<NativeFailure>(&error).is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct Selected {
    calls: Arc<AtomicUsize>,
}
impl Collection for Selected {
    type Cursor = ();
    fn view(&self) -> Composition {
        Composition::Composited
    }
    fn images(&mut self) -> Result<(), At<CollectionError>> {
        Ok(())
    }
    fn next(&mut self, _: &mut ()) -> Result<Option<ImageId>, At<CollectionError>> {
        Ok(None)
    }
    fn children(&mut self, _: &ImageId) -> Result<(), At<CollectionError>> {
        Ok(())
    }
    fn describe(&mut self, _: &ImageId) -> Result<ImageDescription, At<CollectionError>> {
        Ok(ImageDescription::new((1, 1)))
    }
    fn decode(
        &mut self,
        _: &ImageId,
        _: &ImageRequest,
    ) -> Result<ImageOutput, At<CollectionError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(At::wrap(CollectionError::Requirement(
            SelectionError::UnsupportedComposition,
        )))
    }
}
#[test]
#[allow(deprecated)] // Both requirement sources must protect the retained owner.
fn late_requirement_blocks_decode_and_transform_and_owner_remains_usable() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut images = CheckedCollection::open(SharedPulse::default(), PolicySet::new(), |_| {
        Ok(Selected {
            calls: calls.clone(),
        })
    })
    .unwrap()
    .with_policy_set(PolicySet::new().with_workspace_limit(u64::MAX));
    let id = Origin::new().image(0);
    let error = images
        .decode_with(
            &id,
            &ImageRequest::new(),
            |_, _| -> Result<ImageOutput, Cause> {
                panic!("transform must not execute after admission refusal")
            },
        )
        .unwrap_err();
    assert_workspace(&error);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!images.is_finished());
    assert_eq!(images.report().decisions().count(), 0);
    // Explicit caller reconfiguration proves the native owner survived refusal.
    images = images.with_policy_set(PolicySet::new());
    let error = images.decode(&id, &ImageRequest::new()).unwrap_err();
    assert!(zencodec::find_cause::<SelectionError>(&error).is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
