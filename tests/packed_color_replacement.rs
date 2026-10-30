//! A converted color owner composes with original auxiliary guards and charges.
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use zencodec::{
    At,
    frame::MapError,
    image::{
        Auxiliary, AuxiliaryImageOwner, AuxiliaryKind, ImageGeometry, ImageInterpretation,
        ImageOutput, Meaning, OutputError, PackedImageOwner, Rect, Samples, SourcePrecision,
        ValueSlice,
    },
    operation::{
        Decision, Decline, ImageFacts, PolicySet, PublicationOffer, Question, resolve_image,
    },
    storage::{SharedStorage, StorageBudget},
};
use zenpixels::sample::SampleEncoding;
use zenpixels::{ChannelType, ColorContext, Orientation, PixelDescriptor, PixelSlice};

#[derive(Debug)]
struct NativeMapFailure;
impl std::fmt::Display for NativeMapFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private native mapping details")
    }
}
impl std::error::Error for NativeMapFailure {}

struct Owner {
    pixels: SharedStorage<u8>,
    auxiliary: Option<SharedStorage<u8>>,
    mode: Arc<AtomicU8>,
}
impl PackedImageOwner for Owner {
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
        let mode = self.mode.load(Ordering::SeqCst);
        let pixels = PixelSlice::new(
            self.pixels.as_slice(),
            if mode == 1 { 1 } else { 2 },
            2,
            9,
            PixelDescriptor::RGB8_SRGB,
        )
        .unwrap();
        Ok(if mode == 3 {
            pixels.with_color_context(Arc::new(ColorContext::from_icc(vec![1, 2, 3])))
        } else {
            pixels
        })
    }
    fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        if self.mode.load(Ordering::SeqCst) == 2 {
            return Err(At::wrap(OutputError::Mapping(MapError::Backend(Box::new(
                At::wrap(NativeMapFailure).at_named("retained auxiliary mapping"),
            )))));
        }
        match (&self.auxiliary, index) {
            (Some(data), 0) => ValueSlice::new(data.as_slice(), 2, 2, 3, ChannelType::U8).map(Some),
            _ => Ok(None),
        }
    }
}
fn owner(pool: &StorageBudget, extras: bool, fill: u8) -> (Owner, Arc<AtomicU8>) {
    let mode = Arc::new(AtomicU8::new(0));
    (
        Owner {
            pixels: pool.filled(18, fill).unwrap().freeze(),
            auxiliary: extras.then(|| pool.filled(6, 71).unwrap().freeze()),
            mode: mode.clone(),
        },
        mode,
    )
}
fn image(pool: &StorageBudget, extras: bool) -> (ImageOutput, Arc<AtomicU8>) {
    let (owner, mode) = owner(pool, extras, 7);
    (image_from(owner, extras), mode)
}
fn image_from(owner: impl PackedImageOwner + 'static, extras: bool) -> ImageOutput {
    let roles = if extras {
        vec![Auxiliary {
            kind: AuxiliaryKind::Depth,
            name: "source depth".to_owned(),
            source_precision: SourcePrecision::Integer {
                bits: 7,
                signed: false,
            },
            source_dim_shift: 0,
        }]
    } else {
        vec![]
    };
    let interpretation = ImageInterpretation::packed(
        ImageGeometry {
            canvas_width: 2,
            canvas_height: 2,
            region: Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
        },
        Orientation::from_exif(1).unwrap(),
    );
    ImageOutput::from_packed_owner(owner, roles, interpretation).unwrap()
}
fn charges(pool: &StorageBudget) -> (usize, usize) {
    (pool.reserved_bytes(), pool.reserved_objects())
}

struct Separable {
    packed: Owner,
    retained: Arc<SeparateAuxiliary>,
    calls: Arc<AtomicU8>,
}
struct SeparateAuxiliary {
    values: SharedStorage<u8>,
    mode: Arc<AtomicU8>,
}
impl AuxiliaryImageOwner for SeparateAuxiliary {
    fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        if index != 0 {
            return Ok(None);
        }
        match self.mode.load(Ordering::SeqCst) {
            1 => Ok(None),
            2 => ValueSlice::new(self.values.as_slice(), 1, 2, 3, ChannelType::U8).map(Some),
            3 => Err(At::wrap(OutputError::Mapping(MapError::Backend(Box::new(
                At::wrap(NativeMapFailure).at_named("separate auxiliary mapping"),
            ))))),
            _ => ValueSlice::new(self.values.as_slice(), 2, 2, 3, ChannelType::U8).map(Some),
        }
    }
}
impl PackedImageOwner for Separable {
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
        self.packed.pixels()
    }
    fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        self.packed.auxiliary(index)
    }
    fn retain_auxiliaries(&self) -> Result<Option<Arc<dyn AuxiliaryImageOwner>>, At<OutputError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.retained.mode.load(Ordering::SeqCst) == 4 {
            return Err(At::wrap(OutputError::Mapping(MapError::Backend(Box::new(
                At::wrap(NativeMapFailure).at_named("retain separate auxiliary"),
            )))));
        }
        if self.retained.mode.load(Ordering::SeqCst) == 5 {
            panic!("controlled retain auxiliary unwind");
        }
        Ok(Some(self.retained.clone()))
    }
}
fn separable(pool: &StorageBudget) -> (ImageOutput, Arc<AtomicU8>, Arc<AtomicU8>) {
    let (owner, mode, calls) = separable_owner(pool);
    (image_from(owner, true), mode, calls)
}
fn separable_owner(pool: &StorageBudget) -> (Separable, Arc<AtomicU8>, Arc<AtomicU8>) {
    let (packed, _) = owner(pool, true, 7);
    let mode = Arc::new(AtomicU8::new(0));
    let calls = Arc::new(AtomicU8::new(0));
    let retained = Arc::new(SeparateAuxiliary {
        values: packed.auxiliary.as_ref().unwrap().clone(),
        mode: mode.clone(),
    });
    (
        Separable {
            packed,
            retained,
            calls: calls.clone(),
        },
        mode,
        calls,
    )
}

#[test]
fn separate_auxiliary_releases_old_color_but_respects_original_clones_and_repeated_replacement() {
    let pool = StorageBudget::new(60, 4);
    let (image, _, calls) = separable(&pool);
    let address = {
        let mapped = image.map().unwrap();
        let Samples::Values(values) = mapped.plane(1).unwrap().unwrap().samples else {
            panic!()
        };
        values.as_strided_bytes().as_ptr()
    };
    let original = image.clone();
    let (color, _) = owner(&pool, false, 19);
    let first = image.replace_packed_color(color).unwrap();
    assert_eq!(charges(&pool), (42, 3));
    drop(original);
    assert_eq!(charges(&pool), (24, 2)); // only the 6 auxiliary bytes remain
    let (color, _) = owner(&pool, false, 29);
    let last = first.replace_packed_color(color).unwrap();
    assert_eq!(charges(&pool), (24, 2));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let facts = ImageFacts::new(&last);
    assert_eq!((facts.planes, facts.auxiliaries), (2, 1));
    {
        let mapped = last.map().unwrap();
        assert_eq!(mapped.plane_count(), 2);
        assert!(mapped.plane(2).unwrap().is_none());
        assert!(mapped.plane(usize::MAX).unwrap().is_none());
        let plane = mapped.plane(1).unwrap().unwrap();
        assert!(matches!(
            plane.meaning,
            Meaning::Auxiliary(Auxiliary {
                kind: AuxiliaryKind::Depth,
                ..
            })
        ));
        let Samples::Values(values) = plane.samples else {
            panic!()
        };
        assert_eq!(values.as_strided_bytes().as_ptr(), address);
        assert_eq!(values.as_strided_bytes(), &[71; 6]);
    }
    let retained = last.clone();
    drop(last);
    assert_eq!(charges(&pool), (24, 2));
    drop(retained);
    assert_eq!(charges(&pool), (0, 0));
}

#[test]
fn separate_auxiliary_checks_retained_layout_and_preserves_hook_mapping_failures_and_unwind() {
    for failure in 1..=5 {
        let pool = StorageBudget::new(42, 3);
        let (image, mode, _) = separable(&pool);
        let original = image.clone();
        let (color, _) = owner(&pool, false, 19);
        mode.store(failure, Ordering::SeqCst);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            image.replace_packed_color(color)
        }));
        if failure == 5 {
            assert!(result.is_err());
        } else {
            let error = result
                .unwrap()
                .expect_err("invalid retained auxiliary was accepted");
            match failure {
                1 => assert!(matches!(error.error(), OutputError::MissingPlane)),
                2 => assert!(matches!(error.error(), OutputError::Geometry)),
                3 | 4 => {
                    let original =
                        zencodec::find_cause::<At<NativeMapFailure>>(error.as_ref()).unwrap();
                    assert!(original.full_trace().to_string().contains(if failure == 3 {
                        "separate auxiliary mapping"
                    } else {
                        "retain separate auxiliary"
                    }));
                    assert!(
                        !format!("{error:?} {error}").contains("private native mapping details")
                    );
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(charges(&pool), (24, 2)); // independent original survives
        assert!(original.map().is_ok());
        drop(original);
        assert_eq!(charges(&pool), (0, 0));
    }
    let pool = StorageBudget::new(42, 3);
    let (image, mode, _) = separable(&pool);
    let (color, _) = owner(&pool, false, 19);
    let output = image.replace_packed_color(color).unwrap();
    for failure in 1..=3 {
        mode.store(failure, Ordering::SeqCst);
        let error = output
            .map()
            .err()
            .expect("changed retained mapping accepted");
        match failure {
            1 => assert!(matches!(error.error(), OutputError::MissingPlane)),
            2 => assert!(matches!(error.error(), OutputError::Geometry)),
            3 => assert!(zencodec::find_cause::<NativeMapFailure>(error.as_ref()).is_some()),
            _ => unreachable!(),
        }
    }
    drop(output);
    assert_eq!(charges(&pool), (0, 0));
}

#[test]
fn separate_auxiliary_hook_needs_declared_planes_and_valid_replacement_color() {
    for declares_auxiliaries in [false, true] {
        let pool = StorageBudget::new(42, 3);
        let (source, mode, calls) = separable_owner(&pool);
        let image = image_from(source, declares_auxiliaries);
        mode.store(5, Ordering::SeqCst); // any hook call would unwind
        let (color, color_mode) = owner(&pool, false, 19);
        if declares_auxiliaries {
            color_mode.store(1, Ordering::SeqCst);
            assert!(matches!(
                image.replace_packed_color(color).unwrap_err().error(),
                OutputError::Geometry
            ));
        } else {
            let output = image.replace_packed_color(color).unwrap();
            assert_eq!(output.map().unwrap().plane_count(), 1);
            assert_eq!(charges(&pool), (18, 1));
            drop(output);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(charges(&pool), (0, 0));
    }
}

#[test]
fn packed_color_replacement_retains_auxiliary_addresses_and_flattens_repeated_replacements() {
    let pool = StorageBudget::new(60, 4);
    let (source, _) = image(&pool, true);
    let (address, role) = {
        let map = source.map().unwrap();
        let plane = map.plane(1).unwrap().unwrap();
        let Samples::Values(values) = plane.samples else {
            panic!()
        };
        (values.as_strided_bytes().as_ptr(), plane.meaning.clone())
    };
    let (color, _) = owner(&pool, false, 19);
    let first = source.replace_packed_color(color).unwrap();
    assert_eq!(charges(&pool), (42, 3)); // original packed owner remains intact
    let (color, _) = owner(&pool, false, 29);
    assert_eq!(charges(&pool), (60, 4));
    let last = first.replace_packed_color(color).unwrap();
    assert_eq!(charges(&pool), (42, 3)); // intermediate color is gone
    let facts = ImageFacts::new(&last);
    assert_eq!((facts.planes, facts.auxiliaries), (2, 1));
    {
        let map = last.map().unwrap();
        assert_eq!(map.plane_count(), 2);
        let plane = map.plane(1).unwrap().unwrap();
        assert_eq!(*plane.meaning, role);
        let Samples::Values(values) = plane.samples else {
            panic!()
        };
        assert_eq!(values.as_strided_bytes().as_ptr(), address);
        assert_eq!(values.as_strided_bytes(), &[71; 6]);
        assert!(map.plane(2).unwrap().is_none());
        let plane = map.plane(0).unwrap().unwrap();
        assert_eq!(*plane.meaning, Meaning::Color);
        let Samples::Pixels(pixels) = plane.samples else {
            panic!()
        };
        assert_eq!(pixels.row(0), &[29; 6]);
        assert_eq!(pixels.row(1), &[29; 6]);
    }
    let retained = last.clone();
    drop(last);
    assert_eq!(charges(&pool), (42, 3));
    drop(retained);
    assert_eq!(charges(&pool), (0, 0));
}

#[test]
fn packed_color_replacement_without_auxiliaries_releases_source_payload() {
    let source_pool = StorageBudget::new(18, 1);
    let color_pool = StorageBudget::new(18, 1);
    let (source, _) = image(&source_pool, false);
    let (color, _) = owner(&color_pool, false, 19);
    let result = source.replace_packed_color(color).unwrap();
    assert_eq!(charges(&source_pool), (0, 0));
    assert_eq!(charges(&color_pool), (18, 1));
    assert_eq!(result.map().unwrap().plane_count(), 1);
    drop(result);
    assert_eq!(charges(&color_pool), (0, 0));
}

#[test]
fn packed_color_replacement_refuses_component_codes_without_mapping_them() {
    use zencodec::frame::{
        ChromaSiting, Frame, FrameGeometry, FrameInterpretation, FrameTiming, MappedPlanes,
        Picture, PlaneRole, Presentation, Subsampling,
    };
    struct Unmapped;
    impl Picture for Unmapped {
        fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
            panic!("component-code replacement must refuse before native mapping")
        }
    }
    let interpretation = FrameInterpretation::new(
        FrameGeometry::full(2, 2),
        Subsampling::Mono,
        ChromaSiting::Unknown,
        SampleEncoding::new(ChannelType::U8, 8, 0).unwrap(),
        vec![PlaneRole::Luma],
    );
    let source = ImageOutput::from_frame(Frame::new(
        Arc::new(Unmapped),
        Arc::new(interpretation),
        FrameTiming::default(),
        Presentation::default(),
    ));
    let pool = StorageBudget::new(18, 1);
    let (color, _) = owner(&pool, false, 19);
    let error = source.replace_packed_color(color).unwrap_err();
    assert!(matches!(error.error(), OutputError::NotPackedColor));
    assert_eq!(charges(&pool), (0, 0));
}

#[test]
fn packed_color_replacement_checks_both_owners_and_preserves_late_native_causes() {
    let source_pool = StorageBudget::new(24, 2);
    let color_pool = StorageBudget::new(18, 1);
    let (source, source_mode) = image(&source_pool, true);
    let source_clone = source.clone();
    let (color, mode) = owner(&color_pool, false, 19);
    mode.store(1, Ordering::SeqCst);
    let error = source.replace_packed_color(color).unwrap_err();
    assert!(matches!(error.error(), OutputError::Geometry));
    assert_eq!(charges(&color_pool), (0, 0));
    assert_eq!(charges(&source_pool), (24, 2));

    source_mode.store(2, Ordering::SeqCst);
    let (color, _) = owner(&color_pool, false, 19);
    let error = source_clone
        .clone()
        .replace_packed_color(color)
        .unwrap_err();
    assert!(zencodec::find_cause::<NativeMapFailure>(&error).is_some());
    assert_eq!(charges(&color_pool), (0, 0));
    source_mode.store(0, Ordering::SeqCst);
    let (color, mode) = owner(&color_pool, false, 19);
    let result = source_clone.replace_packed_color(color).unwrap();
    for (changed, geometry) in [(1, true), (3, false)] {
        mode.store(changed, Ordering::SeqCst);
        let error = result.map().err().unwrap();
        assert_eq!(matches!(error.error(), OutputError::Geometry), geometry);
        assert_eq!(
            matches!(error.error(), OutputError::Interpretation),
            !geometry
        );
    }
    mode.store(0, Ordering::SeqCst);
    source_mode.store(2, Ordering::SeqCst);
    let error = result.map().err().unwrap();
    let original = zencodec::find_cause::<At<NativeMapFailure>>(&error).unwrap();
    assert!(
        original
            .full_trace()
            .to_string()
            .contains("retained auxiliary mapping")
    );
    assert!(!format!("{error:?} {error}").contains("private native mapping details"));
    source_mode.store(0, Ordering::SeqCst);
    assert_eq!(result.map().unwrap().plane_count(), 2);
    drop(result);
    assert_eq!(charges(&source_pool), (0, 0));
    assert_eq!(charges(&color_pool), (0, 0));
}

#[test]
fn packed_color_replacement_remains_subject_to_publication_refusal_and_unwind() {
    let pool = StorageBudget::new(42, 3);
    for panic in [false, true] {
        let (source, _) = image(&pool, true);
        let (color, _) = owner(&pool, false, 19);
        let converted = source.replace_packed_color(color).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            resolve_image(
                converted,
                &mut |question: Question<'_>| -> Result<Decision, At<NativeMapFailure>> {
                    let Question::Image {
                        image,
                        facts,
                        choices,
                    } = question
                    else {
                        panic!()
                    };
                    assert_eq!(facts.auxiliaries, 1);
                    assert_eq!(
                        *choices.first().unwrap().value(),
                        PublicationOffer::ReturnHeld
                    );
                    image
                        .with_pixels(|pixels| assert_eq!(pixels.row(0), &[19; 6]))
                        .unwrap();
                    assert!(!panic, "publication unwind control");
                    Ok(Decision::decline(Decline::ApplicationRule))
                },
                PolicySet::new(),
                Some(&zencodec::NoPulse),
            )
        }));
        if panic {
            assert!(result.is_err());
        } else {
            let error = result.unwrap().unwrap_err();
            assert_eq!(
                zencodec::find_cause::<Decline>(&error),
                Some(&Decline::ApplicationRule)
            );
        }
        assert_eq!(charges(&pool), (0, 0));
    }
}
