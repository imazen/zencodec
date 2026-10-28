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
fn requirement_refusal_keeps_its_cause_when_default_preferences_also_decline() {
    use zencodec::operation::{
        Decline, ImageConversionOffer, MetadataTransfer, PolicyRequirement, PolicySet,
        RequirementEvidence, resolve_image_conversion,
    };
    use zenpixels::{Orientation, PixelDescriptor};
    let image = frame(words(), 3, 10);
    let mut offer = ImageConversionOffer::new(
        PixelDescriptor::RGB16_SRGB,
        image.interpretation().geometry,
        Orientation::Identity,
    );
    offer.assumed_chroma_siting = Some(ChromaSiting::Unknown);
    let required = PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive);
    for (mut policy, legacy) in [(required, PolicySet::new()), (PolicySet::new(), required)] {
        let error =
            resolve_image_conversion(&image, &[offer], &mut policy, legacy, &zencodec::NoPulse)
                .unwrap_err();
        assert_eq!(
            zencodec::find_cause::<Decline>(&error),
            Some(&Decline::UnsupportedQuestion)
        );
        assert_eq!(
            error
                .error()
                .record()
                .unmet_requirements
                .iter()
                .collect::<Vec<_>>(),
            [(PolicyRequirement::Metadata, RequirementEvidence::Unknown)]
        );
    }
}

#[test]
fn conversion_policy_selects_only_qualified_choices_and_keeps_source_facts() {
    use zencodec::operation::{
        Decision, DecisionContext, ImageConversionOffer, PolicySet, Question, Selection,
        resolve_image, resolve_image_conversion,
    };
    use zenpixels::{Orientation, PixelDescriptor};
    let source = frame(words(), 3, 10);
    let mut assumed = ImageConversionOffer::new(
        PixelDescriptor::RGB16_SRGB,
        source.interpretation().geometry,
        Orientation::Identity,
    );
    assumed.assumed_chroma_siting = Some(ChromaSiting::Center);
    let mut declared = assumed;
    declared.assumed_chroma_siting = None;
    let mut extended = declared;
    // Policy-selection control only: this test does not execute/qualify F32 math.
    extended.output = PixelDescriptor::RGBF32_LINEAR;
    extended.preserves_extended_range = true;
    extended.workspace_upper_bound = Some(4096);
    let offers = [assumed, declared, extended];
    let mut use_preset = |_: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
        Ok(Decision::UsePreset)
    };
    for (preset, expected) in [
        (PolicySet::new(), 1),
        (PolicySet::new().allow_assumed_chroma_siting(true), 0),
        (PolicySet::new().require_extended_range(true), 2),
        (PolicySet::new().with_workspace_limit(4096), 2),
    ] {
        let (index, record) = resolve_image_conversion(
            &source,
            &offers,
            &mut use_preset,
            preset,
            &zencodec::how_far::NoPulse,
        )
        .unwrap();
        assert_eq!(index, expected);
        let DecisionContext::ImageConversion(facts) = record.context else {
            panic!()
        };
        assert_eq!(facts.frame.unwrap().encoding, encoding(10));
        let Some(Selection::ImageConversion(selected)) = record.selected else {
            panic!()
        };
        assert_eq!(selected.output, offers[index].output);
    }
    assumed.assumed_chroma_siting = Some(ChromaSiting::Unknown);
    assert!(
        resolve_image_conversion(
            &source,
            &[assumed],
            &mut use_preset,
            PolicySet::new().allow_assumed_chroma_siting(true),
            &zencodec::how_far::NoPulse
        )
        .is_err()
    );
    assert!(
        resolve_image_conversion(
            &source,
            &[extended],
            &mut use_preset,
            PolicySet::new().with_workspace_limit(4095),
            &zencodec::how_far::NoPulse
        )
        .is_err()
    );
    // A different output question cannot silently ignore the new requirement.
    assert!(
        resolve_image(
            source,
            &mut use_preset,
            PolicySet::new().require_extended_range(true),
            &zencodec::how_far::NoPulse
        )
        .is_err()
    );
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
    use enough::{Stop, StopReason};
    use std::{
        convert::Infallible,
        sync::atomic::{AtomicBool, Ordering},
    };
    use zencodec::how_far::NoPulse;
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
            &NoPulse,
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
            &NoPulse,
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
                    &NoPulse,
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
                &zencodec::how_far::StopOnly::borrowed(&stop),
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
                    panic!("mandatory admission must refuse before callback")
                },
                preset,
                &NoPulse,
            )
            .unwrap_err();
            assert!(owner.upgrade().is_none());
            assert_eq!(
                zencodec::find_cause::<Decline>(&error),
                Some(&Decline::UnsupportedQuestion)
            );
            let record = error.error().record();
            assert!(!record.used_preset);
            assert_eq!(record.admitted, 0);
            assert!(!record.unmet_requirements.is_empty());
        }
    }
}

struct RgbOwner {
    storage: zencodec::storage::SharedStorage<u8>,
    width: u32,
    height: u32,
    stride: usize,
}
impl zencodec::image::PackedImageOwner for RgbOwner {
    fn pixels(&self) -> Result<zenpixels::PixelSlice<'_>, At<OutputError>> {
        zenpixels::PixelSlice::new(
            self.storage.as_slice(),
            self.width,
            self.height,
            self.stride,
            zenpixels::PixelDescriptor::RGB8_SRGB,
        )
        .map_err(|e| At::wrap(OutputError::Mapping(MapError::Backend(Box::new(e)))))
    }
}
fn rgb_owner(pool: &zencodec::storage::StorageBudget, width: u32) -> RgbOwner {
    RgbOwner {
        storage: pool.bytes(24).unwrap().freeze(),
        width,
        height: 2,
        stride: 12,
    }
}
fn cropped_frame(picture: Arc<dyn Picture>, crop_width: u32) -> Frame {
    let mut interpretation = FrameInterpretation::new(
        FrameGeometry::full(3, 2),
        Subsampling::Mono,
        ChromaSiting::Unknown,
        encoding(10),
        vec![PlaneRole::Luma],
    );
    interpretation.geometry.crop.x = 1;
    interpretation.geometry.crop.width = crop_width;
    interpretation.geometry.render = Some((9, 6));
    Frame::new(
        picture,
        Arc::new(interpretation),
        FrameTiming::at(zencodec::time::Timestamp::new(
            37,
            zencodec::time::TimeBase::MILLIS,
        ))
        .with_duration_ticks(41)
        .with_packet(Some(5), Some(100)),
        Presentation::SHOW_EXISTING,
    )
}

#[test]
fn converted_frame_adoption_separates_source_codes_and_releases_native_storage() {
    use zencodec::{
        image::PackedImageOwner,
        operation::{Decision, ImageFacts, PolicySet, Question, resolve_image},
        storage::StorageBudget,
    };
    use zenpixels::{Orientation, PixelDescriptor};
    let pool = StorageBudget::new(48, 2);
    let frame = cropped_frame(words(), 2);
    let weak = Arc::downgrade(frame.picture());
    let source = frame.interpretation().clone();
    let timing = frame.timing();
    let presentation = frame.presentation();
    let owner = rgb_owner(&pool, 2);
    let address = owner.pixels().unwrap().row(0).as_ptr() as usize;
    let image = ImageOutput::from_frame_pixels(frame, owner, Orientation::Identity).unwrap();
    assert!(weak.upgrade().is_none());
    assert!(Arc::ptr_eq(image.source_frame().unwrap(), &source));
    assert!(image.interpretation().signal.is_none());
    assert_eq!(image.interpretation().geometry.region.x, 1);
    assert_eq!(image.interpretation().geometry.region.width, 2);
    assert_eq!(image.interpretation().geometry.canvas_width, 3);
    assert_eq!(image.source_frame().unwrap().geometry.render, Some((9, 6)));
    assert_eq!(image.interpretation().timing, Some(timing));
    assert_eq!(image.interpretation().presentation, Some(presentation));
    assert_eq!(
        image.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap(),
        address
    );
    let facts = ImageFacts::new(&image);
    assert!(facts.frame.is_none());
    assert_eq!(facts.source_frame.unwrap().encoding.code_bits(), 10);
    assert_eq!(facts.packed, Some(PixelDescriptor::RGB8_SRGB));
    let clone = image.clone();
    let replacement = rgb_owner(&pool, 2);
    let image = image.replace_packed_color(replacement).unwrap();
    assert!(Arc::ptr_eq(image.source_frame().unwrap(), &source));
    assert_eq!(image.interpretation().timing, Some(timing));
    assert_eq!(pool.reserved_bytes(), 48);
    drop(clone);
    assert_eq!(pool.reserved_bytes(), 24);
    // A later transform can clone/update current geometry while keeping source
    // provenance; re-adoption must not lose the temporal identity.
    let mut interpretation = image.interpretation().clone();
    interpretation.geometry.region.width = 1;
    let transformed =
        ImageOutput::from_packed_owner(rgb_owner(&pool, 1), vec![], interpretation).unwrap();
    assert!(Arc::ptr_eq(transformed.source_frame().unwrap(), &source));
    assert_eq!(transformed.interpretation().timing, Some(timing));
    drop(transformed);
    let calls = std::cell::Cell::new(0);
    let image = resolve_image(
        image,
        &mut |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
            let Question::Image { image, facts, .. } = q else {
                panic!()
            };
            assert!(facts.frame.is_none());
            assert_eq!(facts.source_frame.unwrap().encoding.code_bits(), 10);
            assert_eq!(facts.packed, Some(PixelDescriptor::RGB8_SRGB));
            assert!(image.with_pixels(|p| p.width() == 2).unwrap());
            assert_eq!(facts.timing, Some(timing));
            calls.set(calls.get() + 1);
            Ok(Decision::UsePreset)
        },
        PolicySet::new(),
        &zencodec::how_far::NoPulse,
    )
    .unwrap()
    .into_output();
    assert_eq!(calls.get(), 1);
    drop(image);
    assert_eq!(pool.reserved_bytes(), 0);
    assert_eq!(pool.reserved_objects(), 0);
}

#[test]
fn converted_frame_bad_geometry_and_owner_failure_release_unpublished_payloads() {
    use zencodec::storage::StorageBudget;
    use zenpixels::Orientation;
    let pool = StorageBudget::new(24, 1);
    for (crop_width, owner_width) in [(3, 3), (2, 1)] {
        let frame = cropped_frame(words(), crop_width);
        let weak = Arc::downgrade(frame.picture());
        let error = ImageOutput::from_frame_pixels(
            frame,
            rgb_owner(&pool, owner_width),
            Orientation::Identity,
        )
        .unwrap_err();
        if crop_width == 3 {
            assert!(matches!(error.error(), OutputError::Region(_)));
        } else {
            assert!(matches!(error.error(), OutputError::Geometry));
        }
        assert!(weak.upgrade().is_none());
        assert_eq!(pool.reserved_bytes(), 0);
    }
    let mut interpretation = zencodec::image::ImageInterpretation::packed(
        zencodec::image::ImageGeometry {
            canvas_width: 2,
            canvas_height: 2,
            region: zencodec::image::Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
        },
        Orientation::Identity,
    );
    interpretation.presentation = Some(Presentation::SHOW_EXISTING);
    assert!(matches!(
        ImageOutput::from_packed_owner(rgb_owner(&pool, 2), vec![], interpretation)
            .unwrap_err()
            .error(),
        OutputError::Interpretation
    ));
    assert_eq!(pool.reserved_bytes(), 0);
    struct BadOwner;
    impl zencodec::image::PackedImageOwner for BadOwner {
        fn pixels(&self) -> Result<zenpixels::PixelSlice<'_>, At<OutputError>> {
            Err(At::wrap(OutputError::Mapping(MapError::Backend(Box::new(
                At::wrap(NativeError(91)),
            )))))
        }
    }
    let failure =
        ImageOutput::from_frame_pixels(cropped_frame(words(), 2), BadOwner, Orientation::Identity)
            .unwrap_err();
    assert_eq!(zencodec::find_cause::<NativeError>(&failure).unwrap().0, 91);
    assert!(!format!("{failure:?} {failure}").contains("91"));
}
