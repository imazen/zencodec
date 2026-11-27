//! Optional calls preserve sample, publication, storage and stop contracts.
use std::{cell::Cell, convert::Infallible};
use zencodec::{
    At, CodecError, ErrorCategory,
    collection::Composition,
    enough::{Stop, StopReason},
    how_far::StopOnly,
    image::{ImageGeometry, ImageInterpretation, ImageOutput, Rect},
    operation::{Decision, PolicySet, Question},
    storage::{StorageBudget, StorageError},
};
use zencodec_render::{RenderRequest, RenderStage, Renderer, geometry};
use zenpixels::{Orientation, PixelBuf, PixelFormat};

fn image() -> ImageOutput {
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
    interpretation.composition = Some(Composition::Composited);
    ImageOutput::packed(
        PixelBuf::from_bytes(
            vec![3, 5, 7, 255, 11, 13, 17, 255],
            2,
            1,
            PixelFormat::RGBA8_SRGB,
        )
        .unwrap(),
        vec![],
        interpretation,
    )
    .unwrap()
}

fn assert_pixels(output: &ImageOutput) {
    output
        .with_pixels(|pixels| {
            assert_eq!((pixels.width(), pixels.height()), (1, 2));
            assert_eq!(pixels.format(), &PixelFormat::RGBA8_SRGB);
            assert_eq!(pixels.row(0), [3, 5, 7, 255]);
            assert_eq!(pixels.row(1), [11, 13, 17, 255]);
        })
        .unwrap();
}

#[test]
fn none_renders_exact_samples_and_publishes_once() {
    for entry in 0..2 {
        let storage = StorageBudget::new(1024, 16);
        let renderer = Renderer::new(storage.clone())
            .with_request(RenderRequest::new(PixelFormat::RGBA8_SRGB));
        let publications = Cell::new(0);
        let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            publications.set(publications.get() + 1);
            Ok(Decision::UsePreset)
        };
        if entry == 0 {
            let output = geometry::project(&image(), None, &storage, None).unwrap();
            assert_pixels(&output);
            assert_eq!(publications.get(), 0);
        } else {
            let output = renderer.render(image(), &mut resolver, None).unwrap();
            assert_pixels(&output.into_output());
            assert_eq!(publications.get(), 1);
        }
        assert_eq!(storage.reserved_bytes(), 0);
        assert_eq!(storage.reserved_objects(), 0);
    }
}

struct TimedOut;
impl Stop for TimedOut {
    fn check(&self) -> Result<(), StopReason> {
        Err(StopReason::TimedOut)
    }
}

#[test]
fn some_stopped_preserves_the_typed_reason_before_allocation_and_publication() {
    let pulse = StopOnly::new(TimedOut);
    for entry in 0..2 {
        let storage = StorageBudget::new(1024, 16);
        let renderer = Renderer::new(storage.clone());
        let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            panic!("stopped render must not reach publication")
        };
        if entry == 0 {
            let error = geometry::project(&image(), None, &storage, Some(&pulse)).unwrap_err();
            assert_eq!(
                zencodec::find_cause::<CodecError>(error.as_ref())
                    .unwrap()
                    .category(),
                ErrorCategory::Stopped(StopReason::TimedOut),
            );
        } else {
            let error = renderer
                .render(image(), &mut resolver, Some(&pulse))
                .unwrap_err();
            assert_eq!(error.error().stage, RenderStage::ColorPlan);
            assert_eq!(
                zencodec::find_cause::<CodecError>(&error)
                    .unwrap()
                    .category(),
                ErrorCategory::Stopped(StopReason::TimedOut),
            );
        }
        assert_eq!(storage.reserved_bytes(), 0);
        assert_eq!(storage.reserved_objects(), 0);
    }
}

#[test]
fn none_preserves_storage_admission_and_blocks_publication_on_failure() {
    let storage = StorageBudget::new(0, 0);
    let renderer = Renderer::new(storage.clone());
    let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        panic!("unadmitted render must not reach publication")
    };
    let error = renderer.render(image(), &mut resolver, None).unwrap_err();
    assert_eq!(error.error().stage, RenderStage::Geometry);
    assert!(zencodec::find_cause::<StorageError>(&error).is_some());
    assert_eq!(storage.reserved_bytes(), 0);
    assert_eq!(storage.reserved_objects(), 0);
}

#[test]
fn required_workspace_refuses_before_projection_for_bound_policies() {
    use zencodec::operation::{
        BoundPolicy, PolicyRequirement, RequirementEvidence, UnmetRequirements,
    };
    // Zero output allowance makes premature projection observably fail with
    // StorageError instead of the required workspace admission cause.
    let storage = StorageBudget::new(0, 0);
    let renderer = Renderer::new(storage.clone());
    let strict = PolicySet::new().with_workspace_limit(u64::MAX);
    let callback = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        panic!("unknown workspace must refuse before callback")
    };
    let mut bound = BoundPolicy::new(strict, callback);
    let error = renderer.render(image(), &mut bound, None).unwrap_err();
    let unmet = zencodec::find_cause::<UnmetRequirements>(&error).unwrap();
    assert_eq!(
        unmet.iter().collect::<Vec<_>>(),
        vec![(PolicyRequirement::Workspace, RequirementEvidence::Unknown)]
    );
    assert!(zencodec::find_cause::<StorageError>(&error).is_none());
    assert_eq!(storage.reserved_bytes(), 0);
}
