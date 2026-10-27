use crate::{control::CheckStop, jxl_items::JxlItems, jxl_tests::fixture};
use std::{cell::Cell, convert::Infallible, rc::Rc};
use zencodec::{
    At, SharedPulse, NoPulse,
    collection::{
        Collection, CollectionError, Composition, Discovery, DiscoveryExhausted, DiscoveryMethod,
        ImageDescription, ImageId, ImageRequest, Origin, SelectionError,
    },
    image::{GeometryError, ImageOutput},
    operation::{CheckedCollection, CollectionStage, Decision, PolicySet, Question},
};

/// Retain the real native source and decode path, but break one returned fact.
struct Tampered<C> {
    native: C,
    corrupt: Rc<Cell<bool>>,
}
impl<C: Collection> Collection for Tampered<C> {
    type Cursor = C::Cursor;
    fn view(&self) -> Composition {
        self.native.view()
    }
    fn images(&mut self) -> Result<Self::Cursor, At<CollectionError>> {
        self.native.images()
    }
    fn children(&mut self, image: &ImageId) -> Result<Self::Cursor, At<CollectionError>> {
        self.native.children(image)
    }
    fn next(&mut self, cursor: &mut Self::Cursor) -> Result<Option<ImageId>, At<CollectionError>> {
        self.native.next(cursor)
    }
    fn decode(
        &mut self,
        image: &ImageId,
        request: &ImageRequest,
    ) -> Result<ImageOutput, At<CollectionError>> {
        self.native.decode(image, request)
    }
    fn describe(&mut self, image: &ImageId) -> Result<ImageDescription, At<CollectionError>> {
        let mut description = self.native.describe(image)?;
        if self.corrupt.get() {
            description.stored_size.0 = 0;
        }
        Ok(description)
    }
}

#[test]
fn description_checked_validates_native_facts_without_claiming_pixel_publication() {
    let bytes = fixture("named_frame_test.jxl");
    let corrupt = Rc::new(Cell::new(true));
    let visits = Cell::new(0);
    let resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        visits.set(visits.get() + 1);
        Ok(Decision::UsePreset)
    };
    let mut images = CheckedCollection::open(SharedPulse::default(), resolver, |stop| {
        Ok(Tampered {
            native: JxlItems::open_with_stop(&bytes, Composition::Composited, 64, stop)?,
            corrupt: corrupt.clone(),
        })
    })
    .unwrap();
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    let error = images
        .describe(&id)
        .expect_err("invalid native geometry escaped");
    assert_eq!(error.error().stage, CollectionStage::Describe);
    assert_eq!(
        zencodec::find_cause::<GeometryError>(&error)
            .unwrap()
            .geometry
            .canvas_width,
        0
    );
    assert!(!images.is_finished());
    assert_eq!(visits.get(), 0);
    assert_eq!(images.report().decisions().count(), 0);
    corrupt.set(false);
    let (repeated_id, description) = crate::collection::checked_first_description(&mut images)
        .unwrap()
        .unwrap();
    assert_eq!(id, repeated_id);
    assert_eq!(visits.get(), 0);
    let output = images.decode(&id, &ImageRequest::new()).unwrap();
    assert_eq!(visits.get(), 1);
    assert_eq!(
        description.stored_size,
        (
            output.interpretation().geometry.canvas_width,
            output.interpretation().geometry.canvas_height
        )
    );
    assert_eq!(
        description.stored_orientation,
        output.interpretation().orientation
    );
}

#[test]
fn description_checked_cancellation_includes_the_final_return_seam() {
    let bytes = fixture("named_frame_test.jxl");
    let run = |stop: &CheckStop, at: Option<usize>| {
        let mut images = CheckedCollection::open(stop.token(), PolicySet::new(), |token| {
            JxlItems::open_with_stop(&bytes, Composition::Composited, 64, token)
        })
        .unwrap();
        let mut cursor = images.images().unwrap();
        let id = images.next(&mut cursor).unwrap().unwrap();
        let before = stop.calls();
        if let Some(at) = at {
            stop.after(at);
        }
        let result = images.describe(&id);
        let checks = stop.calls() - before;
        if result.is_err() {
            assert!(images.is_finished());
            assert_eq!(
                images.describe(&id).unwrap_err().error().stage,
                CollectionStage::Finished
            );
        }
        (result, checks)
    };
    let (result, checks) = run(&CheckStop::default(), None);
    result.unwrap();
    assert!(checks >= 4);
    for reason in [
        zencodec::enough::StopReason::Cancelled,
        zencodec::enough::StopReason::TimedOut,
    ] {
        for at in 0..checks {
            let (result, _) = run(&CheckStop::with_reason(reason), Some(at));
            let error = result.expect_err("description escaped a stopped operation");
            assert_eq!(error.error().stop_reason(), Some(reason));
            let wrapped =
                zencodec::find_cause::<zencodec::CodecError>(&error).map(|e| e.category());
            assert_eq!(
                wrapped,
                Some(zencodec::ErrorCategory::Stopped(reason)),
                "missing original stop reason: {error:?}"
            );
        }
    }
}

#[test]
fn description_discovery_charges_failures_cached_returns_and_refuses_before_native_work() {
    let bytes = fixture("named_frame_test.jxl");
    let stop = CheckStop::default();
    let native =
        JxlItems::open_with_stop(&bytes, Composition::Composited, 64, stop.token()).unwrap();
    let mut images = Discovery::new(native, 5);
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    let foreign = Origin::new().image(0);
    let error = images.describe(&foreign).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&error),
        Some(&SelectionError::ForeignImage)
    );
    assert_eq!(images.describe(&id).unwrap(), images.describe(&id).unwrap());
    let used = images.usage();
    assert_eq!(used.attempts, 5);
    assert_eq!(used.backend_errors, 1);
    assert_eq!(used.returned_descriptions, 2);
    let before = stop.calls();
    for _ in 0..2 {
        let error = images.describe(&id).unwrap_err();
        let exhausted = zencodec::find_cause::<DiscoveryExhausted>(&error).unwrap();
        assert_eq!(exhausted.method, DiscoveryMethod::Describe);
        assert_eq!(exhausted.used, used);
        assert_eq!(
            stop.calls(),
            before,
            "denied description entered the adapter"
        );
    }
    images.decode(&id, &ImageRequest::new()).unwrap();
    assert_eq!(images.usage(), used);
}
