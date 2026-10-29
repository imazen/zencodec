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
use zenpixels::{Orientation, PixelBuffer, PixelDescriptor};

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
        PixelBuffer::from_vec(
            vec![3, 5, 7, 255, 11, 13, 17, 255],
            2,
            1,
            PixelDescriptor::RGBA8_SRGB,
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
            assert_eq!((pixels.width(), pixels.rows()), (1, 2));
            assert_eq!(pixels.descriptor(), PixelDescriptor::RGBA8_SRGB);
            assert_eq!(pixels.row(0), [3, 5, 7, 255]);
            assert_eq!(pixels.row(1), [11, 13, 17, 255]);
        })
        .unwrap();
}

#[test]
#[allow(deprecated)] // Keep the separate-preset entry point covered during migration.
fn none_renders_exact_samples_and_publishes_once() {
    for entry in 0..3 {
        let storage = StorageBudget::new(1024, 16);
        let renderer = Renderer::new(storage.clone())
            .with_request(RenderRequest::new(PixelDescriptor::RGBA8_SRGB));
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
            let output = if entry == 1 {
                renderer.render(image(), &mut resolver, None)
            } else {
                renderer.render_with_preset(image(), &mut resolver, PolicySet::new(), None)
            }
            .unwrap();
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
#[allow(deprecated)]
fn some_stopped_preserves_the_typed_reason_before_allocation_and_publication() {
    let pulse = StopOnly::new(TimedOut);
    for entry in 0..3 {
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
            let error = if entry == 1 {
                renderer.render(image(), &mut resolver, Some(&pulse))
            } else {
                renderer.render_with_preset(image(), &mut resolver, PolicySet::new(), Some(&pulse))
            }
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
