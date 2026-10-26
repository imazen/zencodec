//! The collection's view must survive detached output and policy publication.
extern crate std;
use super::*;
use crate::{
    Unstoppable,
    collection::{CollectionError, Origin, SelectionError},
    image::{ImageGeometry, ImageInterpretation, Rect},
    operation::{Decision, ImageFacts, Question},
};
use alloc::{rc::Rc, vec};
use core::{cell::Cell, convert::Infallible};
use zenpixels::{Orientation, PixelBuffer, PixelDescriptor};

fn image(composition: Option<Composition>) -> ImageOutput {
    let mut interpretation = ImageInterpretation::packed(
        ImageGeometry {
            canvas_width: 2,
            canvas_height: 1,
            region: Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            },
        },
        Orientation::Rotate90,
    );
    interpretation.composition = composition;
    ImageOutput::packed(
        PixelBuffer::from_vec(vec![13, 37], 2, 1, PixelDescriptor::GRAY8).unwrap(),
        vec![],
        interpretation,
    )
    .unwrap()
}

struct Adapter {
    origin: Origin,
    view: Rc<Cell<Composition>>,
    change_during_decode: bool,
    calls: Rc<Cell<usize>>,
    output: ImageOutput,
}
impl Collection for Adapter {
    type Cursor = bool;
    fn view(&self) -> Composition {
        self.view.get()
    }
    fn images(&mut self) -> Result<bool, At<CollectionError>> {
        Ok(false)
    }
    fn children(&mut self, _: &ImageId) -> Result<bool, At<CollectionError>> {
        Err(At::wrap(CollectionError::Requirement(
            SelectionError::UnsupportedChildren,
        )))
    }
    fn next(&mut self, used: &mut bool) -> Result<Option<ImageId>, At<CollectionError>> {
        Ok((!core::mem::replace(used, true)).then(|| self.origin.image(0)))
    }
    fn describe(&mut self, id: &ImageId) -> Result<ImageDescription, At<CollectionError>> {
        self.origin
            .key(id)
            .map_err(|e| At::wrap(CollectionError::Requirement(e)))?;
        Ok(ImageDescription::new((2, 1)).with_orientation(Orientation::Rotate90))
    }
    fn decode(
        &mut self,
        id: &ImageId,
        _: &ImageRequest,
    ) -> Result<ImageOutput, At<CollectionError>> {
        self.origin
            .key(id)
            .map_err(|e| At::wrap(CollectionError::Requirement(e)))?;
        self.calls.set(self.calls.get() + 1);
        if self.change_during_decode {
            self.view.set(Composition::Uncomposited);
        }
        Ok(self.output.clone())
    }
}

#[test]
fn collection_view_is_bound_before_policy_without_copying_or_relabeling_clones() {
    for view in [Composition::Composited, Composition::Uncomposited] {
        for claim in [None, Some(view)] {
            let source = image(claim);
            let address = source.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap();
            let seen = Cell::new(0);
            let resolver = |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Image { image, facts, .. } = question else {
                    panic!("wrong question")
                };
                assert_eq!(facts.composition, Some(view));
                assert_eq!(image.interpretation().composition, Some(view));
                assert_eq!(
                    image.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap(),
                    address
                );
                seen.set(seen.get() + 1);
                Ok(Decision::UsePreset)
            };
            let mut collection =
                CheckedCollection::open(StopToken::new(Unstoppable), resolver, |_| {
                    Ok(Adapter {
                        origin: Origin::new(),
                        view: Rc::new(Cell::new(view)),
                        change_during_decode: false,
                        calls: Rc::new(Cell::new(0)),
                        output: source.clone(),
                    })
                })
                .unwrap();
            let output = collection
                .first_image(&ImageRequest::new())
                .unwrap()
                .unwrap()
                .into_output();
            drop(collection);
            assert_eq!(seen.get(), 1);
            assert_eq!(output.interpretation().composition, Some(view));
            assert_eq!(
                output.interpretation().orientation,
                Some(Orientation::Rotate90)
            );
            assert_eq!(source.interpretation().composition, claim);
            assert_eq!(ImageFacts::new(&source).composition, claim);
            assert_eq!(
                output.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap(),
                address
            );
        }
    }
}

#[test]
fn collection_view_conflicts_refuse_before_publication_with_typed_facts() {
    for when in 0..3 {
        let view = Rc::new(Cell::new(Composition::Composited));
        let calls = Rc::new(Cell::new(0));
        let seen = Cell::new(0);
        let resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            seen.set(seen.get() + 1);
            Ok(Decision::UsePreset)
        };
        let source = image((when == 2).then_some(Composition::Uncomposited));
        let mut collection = CheckedCollection::open(StopToken::new(Unstoppable), resolver, |_| {
            Ok(Adapter {
                origin: Origin::new(),
                view: view.clone(),
                change_during_decode: when == 1,
                calls: calls.clone(),
                output: source.clone(),
            })
        })
        .unwrap();
        let mut cursor = collection.images().unwrap();
        let id = collection.next(&mut cursor).unwrap().unwrap();
        if when == 0 {
            view.set(Composition::Uncomposited);
        }
        let error = collection.decode(&id, &ImageRequest::new()).unwrap_err();
        assert_eq!(error.error().stage, CollectionStage::Decode);
        assert!(matches!(
            crate::find_cause::<OutputError>(&error),
            Some(OutputError::Composition {
                expected: Composition::Composited,
                actual: Composition::Uncomposited,
            })
        ));
        assert_eq!(seen.get(), 0);
        assert_eq!(calls.get(), usize::from(when != 0));
        assert_eq!(collection.report().decisions().count(), 0);
        assert_eq!(
            source.interpretation().composition,
            (when == 2).then_some(Composition::Uncomposited)
        );
    }
}
