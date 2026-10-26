//! Check the actual public output against mismatched native plane contracts.
use std::sync::Arc;
use zencodec::{
    At,
    frame::{
        ChromaSiting, Frame, FrameGeometry, FrameInterpretation, FrameTiming, MapError,
        MappedPlanes, Picture, PlaneRole, Presentation, SampleEncoding, Subsampling,
    },
    image::{ImageOutput, OutputError, Row},
};
use zencodec_media::frame::{OwnedPlane, OwnedPlanes, PlaneData};
use zenpixels::ChannelType;

fn encoding(bits: u8) -> SampleEncoding {
    SampleEncoding::new(ChannelType::U16, bits, 0).unwrap()
}
fn frame(picture: Arc<dyn Picture>, width: u32, bits: u8) -> ImageOutput {
    let interpretation = FrameInterpretation::new(
        FrameGeometry::full(width, 2),
        Subsampling::Mono,
        ChromaSiting::Unknown,
        encoding(bits),
        vec![PlaneRole::Luma],
    );
    ImageOutput::from_frame(Frame::new(
        picture,
        Arc::new(interpretation),
        FrameTiming::default(),
        Presentation::default(),
    ))
}
fn words() -> Arc<dyn Picture> {
    Arc::new(
        OwnedPlanes::new(
            vec![OwnedPlane {
                data: PlaneData::U16(vec![0, 512, 1023, 65535, 1, 511, 1022]),
                width: 3,
                height: 2,
                stride_bytes: 8,
            }],
            encoding(10),
        )
        .unwrap(),
    )
}

#[test]
fn placement_is_validated_with_owner_without_clipping_hidden_samples() {
    use zencodec::image::{ImageGeometry, ImageInterpretation, Placement, PlacementError, Rect};
    use zenpixels::{Orientation, PixelBuffer, PixelDescriptor};

    let geometry = ImageGeometry {
        canvas_width: 3,
        canvas_height: 2,
        region: Rect {
            x: 0,
            y: 0,
            width: 3,
            height: 2,
        },
    };
    let open = |placement| {
        let pixels =
            PixelBuffer::from_vec(vec![1, 2, 3, 4, 5, 6], 3, 2, PixelDescriptor::GRAY8).unwrap();
        let mut interpretation = ImageInterpretation::packed(geometry, Orientation::Identity);
        interpretation.placement = Some(placement);
        ImageOutput::packed(pixels, vec![], interpretation)
    };
    let invalid = open(Placement {
        canvas: (0, 2),
        origin: (0, 0),
    })
    .unwrap_err();
    assert!(matches!(
        invalid.error(),
        OutputError::Placement(PlacementError::EmptyCanvas)
    ));
    assert!(zencodec::find_cause::<PlacementError>(&invalid).is_some());
    let overflow = open(Placement {
        canvas: (2, 2),
        origin: (i64::MAX, 0),
    })
    .unwrap_err();
    assert!(matches!(
        overflow.error(),
        OutputError::Placement(PlacementError::CoordinateOverflow)
    ));

    let placement = Placement {
        canvas: (2, 2),
        origin: (-10, 0),
    };
    let image = open(placement).unwrap();
    assert_eq!(placement.visible(geometry).unwrap(), None);
    assert_eq!(image.interpretation().placement, Some(placement));
    assert_eq!(
        zencodec::operation::ImageFacts::new(&image).placement,
        Some(placement)
    );
    image
        .with_pixels(|pixels| {
            assert_eq!(pixels.row(0), &[1, 2, 3]);
            assert_eq!(pixels.row(1), &[4, 5, 6]);
            assert_eq!(pixels.stride(), 3);
        })
        .unwrap();
    let mut detached = image.interpretation().clone();
    detached.placement = None;
    assert_eq!(image.interpretation().placement, Some(placement));
}

#[test]
fn native_codes_keep_active_rows_and_refuse_conflicting_claims() {
    let native = words();
    let correct = frame(native.clone(), 3, 10);
    let mapping = correct.map().unwrap();
    let plane = mapping.plane(0).unwrap().unwrap();
    assert_eq!(plane.samples.code_encoding(), Some(encoding(10)));
    let Some(Row::Words16(row)) = plane.samples.row(1) else {
        panic!("native words were reinterpreted");
    };
    assert_eq!(row, &[1, 511, 1022]);
    assert!(mapping.plane(1).unwrap().is_none());
    for (width, bits) in [(3, 12), (2, 10), (2, 12)] {
        let incorrect = frame(native.clone(), width, bits);
        let mapped = incorrect.map().unwrap();
        let error = match mapped.plane(0) {
            Err(error) => error,
            Ok(_) => panic!("conflicting native interpretation was accepted"),
        };
        let OutputError::Component {
            index,
            expected_dimensions,
            actual_dimensions,
            expected_encoding,
            actual_encoding,
        } = error.error()
        else {
            panic!("wrong refusal: {error:?}");
        };
        assert_eq!(*index, 0);
        assert_eq!(*expected_dimensions, (width as usize, 2));
        assert_eq!(*actual_dimensions, (3, 2));
        assert_eq!(*expected_encoding, encoding(bits));
        assert_eq!(*actual_encoding, encoding(10));
    }
}

#[derive(Debug)]
struct NativeError(u32);
impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native mapping {}", self.0)
    }
}
impl std::error::Error for NativeError {}
struct Refusing;
impl Picture for Refusing {
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        Err(MapError::Backend(Box::new(At::wrap(NativeError(73)).at())))
    }
}

#[test]
fn mapping_error_retains_native_cause_through_shared_output() {
    let image = frame(Arc::new(Refusing), 3, 10);
    let error = match image.map() {
        Err(error) => error,
        Ok(_) => panic!("native mapping refusal was hidden"),
    };
    assert_eq!(zencodec::find_cause::<NativeError>(&error).unwrap().0, 73);
    assert!(zencodec::find_cause::<At<NativeError>>(&error).is_some());
}

#[test]
fn missing_planes_and_invalid_canvas_refuse_before_exposing_samples() {
    let missing = OwnedPlanes::new(vec![], encoding(10)).unwrap();
    let output = frame(Arc::new(missing), 3, 10);
    assert!(
        matches!(output.map(), Err(error) if matches!(error.error(), OutputError::MissingPlane))
    );
    let invalid = frame(Arc::new(Refusing), 0, 10);
    // Geometry is rejected before entering the deliberately refusing backend.
    assert!(matches!(invalid.map(), Err(error) if matches!(error.error(), OutputError::Region(_))));
}

mod publication {
    use super::*;
    use enough::{Stop, StopReason, Unstoppable};
    use std::{
        convert::Infallible,
        sync::atomic::{AtomicBool, Ordering},
    };
    use zencodec::operation::{
        Decision, DecisionCode, DecisionContext, Decline, MetadataTransfer, PolicySet, Question,
        Selection, resolve_image,
    };

    #[test]
    fn selected_image_policy_returns_the_same_owner_and_bounded_facts() {
        let native = words();
        let owner = Arc::downgrade(&native);
        let image = frame(native, 3, 10);
        let facts_address = image.interpretation() as *const _;
        let outcome = resolve_image(
            image,
            &mut |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Image {
                    image,
                    facts,
                    choices,
                } = question
                else {
                    panic!("wrong domain")
                };
                assert_eq!(image.interpretation() as *const _, facts_address);
                assert_eq!(facts.planes, 1);
                assert_eq!(facts.auxiliaries, 0);
                assert_eq!(facts.source_details, 0);
                assert_eq!(facts.packed, None);
                assert_eq!(facts.frame.unwrap().encoding, encoding(10));
                let mapped = image.map().unwrap();
                assert_eq!(
                    mapped.plane(0).unwrap().unwrap().samples.code_encoding(),
                    Some(encoding(10))
                );
                Ok(Decision::choose(choices.first().unwrap()))
            },
            PolicySet::new(),
            &Unstoppable,
        )
        .unwrap();
        assert!(owner.upgrade().is_some());
        assert_eq!(outcome.interpretation() as *const _, facts_address);
        let records: Vec<_> = outcome.report().decisions().collect();
        assert_eq!(records.len(), 1);
        assert!(matches!(records[0].selected, Some(Selection::Image(_))));
        drop(outcome);
        assert!(owner.upgrade().is_none());
    }

    #[test]
    fn all_unsuccessful_decisions_drop_the_unpublished_image() {
        let mut stale = None;
        let first = resolve_image(
            frame(words(), 3, 10),
            &mut |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Image { choices, .. } = question else {
                    panic!("wrong domain")
                };
                stale = Some(choices.first().unwrap().id());
                Ok(Decision::UsePreset)
            },
            PolicySet::new(),
            &Unstoppable,
        )
        .unwrap();
        drop(first);
        for mode in 0..4 {
            let native = words();
            let owner = Arc::downgrade(&native);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                resolve_image(
                    frame(native, 3, 10),
                    &mut |_: Question<'_>| -> Result<Decision, At<NativeError>> {
                        match mode {
                            0 => Ok(Decision::decline(Decline::ApplicationRule)),
                            1 => Err(At::wrap(NativeError(99)).at_named("private-source-name")),
                            2 => Ok(Decision::Choose(stale.unwrap())),
                            _ => panic!("authored callback panic"),
                        }
                    },
                    PolicySet::new(),
                    &Unstoppable,
                )
            }));
            assert!(owner.upgrade().is_none());
            if mode == 3 {
                assert!(result.is_err());
            } else {
                let error = result.unwrap().unwrap_err();
                assert_eq!(
                    error.error().record().code,
                    [
                        DecisionCode::Declined,
                        DecisionCode::CallbackFailed,
                        DecisionCode::InvalidChoice
                    ][mode]
                );
                assert!(matches!(
                    error.error().record().context,
                    DecisionContext::Image(_)
                ));
                assert!(!format!("{:?}", error.error()).contains("private-source-name"));
                if mode == 1 {
                    assert_eq!(zencodec::find_cause::<NativeError>(&error).unwrap().0, 99);
                    assert!(zencodec::find_cause::<At<NativeError>>(&error).is_some());
                }
            }
        }
    }

    struct Cancel(AtomicBool);
    impl Stop for Cancel {
        fn check(&self) -> Result<(), StopReason> {
            if self.0.load(Ordering::SeqCst) {
                Err(StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn cancellation_before_and_after_callback_prevents_publication() {
        for pre_cancelled in [false, true] {
            let stop = Cancel(AtomicBool::new(pre_cancelled));
            let native = words();
            let owner = Arc::downgrade(&native);
            let mut called = false;
            let error = resolve_image(
                frame(native, 3, 10),
                &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                    called = true;
                    stop.0.store(true, Ordering::SeqCst);
                    Ok(Decision::UsePreset)
                },
                PolicySet::new(),
                &stop,
            )
            .unwrap_err();
            assert_eq!(called, !pre_cancelled);
            assert!(owner.upgrade().is_none());
            assert_eq!(error.error().record().code, DecisionCode::Stopped);
            assert_eq!(error.error().stop_reason(), Some(StopReason::Cancelled));
        }
    }

    #[test]
    fn presets_refuse_unproven_metadata_and_workspace_requirements() {
        for preset in [
            PolicySet::new().allow_opaque_profiles(false),
            PolicySet::new().with_workspace_limit(u64::MAX),
            PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
        ] {
            let native = words();
            let owner = Arc::downgrade(&native);
            let error = resolve_image(
                frame(native, 3, 10),
                &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                    Ok(Decision::UsePreset)
                },
                preset,
                &Unstoppable,
            )
            .unwrap_err();
            assert!(owner.upgrade().is_none());
            assert_eq!(
                zencodec::find_cause::<Decline>(&error),
                Some(&Decline::UnsupportedQuestion)
            );
            assert!(error.error().record().used_preset);
        }
    }
}
