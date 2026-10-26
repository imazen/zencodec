use super::*;
use crate::{
    collection_factories::{Factory, FactoryError, JxlFactory},
    jxl_items::{JxlItems, Limits},
    jxl_layers::JxlLayers,
    output_tests::mapped_planes,
};
use std::{cell::Cell, convert::Infallible, sync::Arc};
use zencodec::decode::DecodePolicy;
use zencodec::{
    At, CodecSet, ImageFormat, ResourceLimits, StopToken, Unstoppable,
    collection::{Cause, DiscoveryExhausted, ErasedCollection, OpenRequest},
    operation::{CollectionStage, Decision, PolicySet, Question},
    source::{DynReadAt, ReadAtShim, SliceReadAt, SourceError},
};

fn registered() -> CodecSet {
    crate::collection::registered_factories(
        Allowances::default(),
        Limits {
            images: 64,
            frame_headers: Some(64),
        },
    )
}
fn request(view: Composition) -> OpenRequest {
    OpenRequest::new(view, 64)
}
fn stop() -> StopToken {
    StopToken::new(Unstoppable)
}

// A local source cannot be sent to another thread and cannot expose a resident
// slice. It also records length queries to detect accidental full-source scans.
struct Positional<'a> {
    bytes: SliceReadAt<'a>,
    reads: Rc<Cell<usize>>,
    lengths: Rc<Cell<usize>>,
    fail_at: Rc<Cell<usize>>,
}
impl<'a> Positional<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes: SliceReadAt::new(bytes),
            reads: Rc::default(),
            lengths: Rc::default(),
            fail_at: Rc::new(Cell::new(usize::MAX)),
        }
    }
    fn tick(&self) -> Result<(), At<Replaced>> {
        let n = self.reads.get() + 1;
        self.reads.set(n);
        if n == self.fail_at.get() {
            Err(At::wrap(Replaced(n)))
        } else {
            Ok(())
        }
    }
}
#[derive(Debug)]
struct Replaced(usize);
impl std::fmt::Display for Replaced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "private-source-payload {}", self.0)
    }
}
impl std::error::Error for Replaced {}
#[derive(Debug)]
enum InputFailure {
    Replaced(At<Replaced>),
    Read(SourceError),
}
impl std::fmt::Display for InputFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("input failed")
    }
}
impl std::error::Error for InputFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Replaced(e) => e,
            Self::Read(e) => e,
        })
    }
}
impl ReadAt for Positional<'_> {
    type Error = InputFailure;
    fn len(&mut self) -> Result<u64, Self::Error> {
        self.lengths.set(self.lengths.get() + 1);
        ReadAt::len(&mut self.bytes).map_err(InputFailure::Read)
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Self::Error> {
        self.tick().map_err(InputFailure::Replaced)?;
        ReadAt::read_at(&mut self.bytes, offset, out).map_err(InputFailure::Read)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        self.tick().map_err(InputFailure::Replaced)?;
        ReadAt::read_exact_at(&mut self.bytes, offset, out).map_err(InputFailure::Read)
    }
}

#[test]
fn collection_factory_borrowed_positional_tiff_keeps_regions_children_and_output_ownership() {
    for big in [false, true] {
        for planar in [false, true] {
            let (bytes, refs) = nested_fixture(big, planar, false);
            let mut source = ReadAtShim(Positional::new(&bytes));
            let lengths = source.0.lengths.clone();
            let decisions = Cell::new(0);
            let resolver = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                assert!(matches!(q, Question::Image { .. }));
                decisions.set(decisions.get() + 1);
                Ok(Decision::UsePreset)
            };
            let set = registered();
            let mut images = set
                .open_collection(
                    ImageFormat::Tiff,
                    &mut source,
                    request(Composition::Composited),
                    resolver,
                    stop(),
                )
                .unwrap();
            // Factory configuration is not part of the opened operation's lifetime.
            drop(set);
            let mut roots = images.images().unwrap();
            let root = images.next(&mut roots).unwrap().unwrap();
            let mut children = images.children(&root).unwrap();
            let child = images.next(&mut children).unwrap().unwrap();
            let description = images.describe(&child).unwrap();
            assert_eq!(description.stored_size, (refs[1].width, refs[1].height));
            assert_eq!(decisions.get(), 0);
            let region = Rect {
                x: 3,
                y: 2,
                width: 11,
                height: 9,
            };
            let output = images
                .decode(&child, &ImageRequest::new().with_region(region))
                .unwrap();
            assert_eq!(decisions.get(), 1);
            assert_eq!(
                lengths.get(),
                0,
                "native positional path queried the whole length"
            );
            drop(images);
            drop(source);
            drop(bytes);
            assert_eq!(
                output
                    .with_pixels(|p| p.as_strided_bytes().to_vec())
                    .unwrap(),
                crop(&refs[1], region)
            );
        }
    }
}

#[test]
fn collection_factory_resident_jxl_views_keep_native_pixels_and_inspection() {
    for (name, view) in [
        ("named_frame_test.jxl", Composition::Composited),
        (
            "conformance_test_images/cmyk_layers.jxl",
            Composition::Uncomposited,
        ),
    ] {
        let bytes = crate::jxl_tests::fixture(name);
        let limits = Limits {
            images: 64,
            frame_headers: Some(64),
        };
        let mut typed = match view {
            Composition::Composited => ErasedCollection::new(
                JxlItems::open_with_stop(&bytes, view, limits, stop()).unwrap(),
            ),
            _ => ErasedCollection::new(JxlLayers::open(&bytes, limits, stop()).unwrap()),
        };
        let mut cursor = typed.images().unwrap();
        let id = typed.next(&mut cursor).unwrap().unwrap();
        let expected_description = typed.describe(&id).unwrap();
        let expected = typed.decode(&id, &ImageRequest::new()).unwrap();
        let mut source = ReadAtShim(SliceReadAt::new(&bytes));
        let mut images = registered()
            .open_collection(
                ImageFormat::Jxl,
                &mut source,
                request(view),
                PolicySet::new(),
                stop(),
            )
            .unwrap();
        let mut cursor = images.images().unwrap();
        let id = images.next(&mut cursor).unwrap().unwrap();
        assert_eq!(images.describe(&id).unwrap(), expected_description);
        let output = images.decode(&id, &ImageRequest::new()).unwrap();
        assert_eq!(mapped_planes(&output), mapped_planes(&expected));
        assert_eq!(
            output.interpretation().placement,
            expected.interpretation().placement
        );
    }
}
#[test]
fn collection_factory_refuses_resident_input_requirement_before_reading() {
    let bytes = crate::jxl_tests::fixture("named_frame_test.jxl");
    let mut source = ReadAtShim(Positional::new(&bytes));
    for format in [ImageFormat::Jxl, ImageFormat::Png] {
        let error = registered()
            .open_collection(
                format,
                &mut source,
                request(Composition::Composited),
                PolicySet::new(),
                stop(),
            )
            .err()
            .unwrap();
        let expected = if format == ImageFormat::Jxl {
            FactoryError::ResidentInputRequired
        } else {
            FactoryError::NotRegistered(format)
        };
        assert_eq!(
            *zencodec::find_cause::<FactoryError>(&error).unwrap(),
            expected
        );
        assert_eq!(error.error().stage, CollectionStage::Open);
        assert_eq!(error.error().stop_reason(), None);
        assert_eq!(source.0.reads.get(), 0);
        assert_eq!(source.0.lengths.get(), 0);
    }
}

#[test]
fn collection_factory_foreign_cursors_do_not_advance_either_operation() {
    let bytes = crate::jxl_tests::fixture("named_frame_test.jxl");
    let mut left = ReadAtShim(SliceReadAt::new(&bytes));
    let mut right = ReadAtShim(SliceReadAt::new(&bytes));
    let set = registered();
    let mut a = set
        .open_collection(
            ImageFormat::Jxl,
            &mut left,
            request(Composition::Composited),
            PolicySet::new(),
            stop(),
        )
        .unwrap();
    let mut b = set
        .open_collection(
            ImageFormat::Jxl,
            &mut right,
            request(Composition::Composited),
            PolicySet::new(),
            stop(),
        )
        .unwrap();
    let mut cursor = a.images().unwrap();
    let error = b.next(&mut cursor).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&error),
        Some(&SelectionError::ForeignCursor)
    );
    let id = a.next(&mut cursor).unwrap().unwrap();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&b.describe(&id).unwrap_err()),
        Some(&SelectionError::ForeignImage)
    );
    assert!(a.describe(&id).is_ok());
    assert!(b.first_image(&ImageRequest::new()).unwrap().is_some());
}

#[test]
fn collection_factory_source_failures_keep_trace_and_safe_default_formatting() {
    let (bytes, _) = nested_fixture(false, false, false);
    let mut source = ReadAtShim(Positional::new(&bytes));
    let reads = source.0.reads.clone();
    let fail_at = source.0.fail_at.clone();
    let mut images = registered()
        .open_collection(
            ImageFormat::Tiff,
            &mut source,
            request(Composition::Composited),
            PolicySet::new(),
            stop(),
        )
        .unwrap();
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    fail_at.set(reads.get() + 1);
    let error = images.describe(&id).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<Replaced>(&error).unwrap().0,
        fail_at.get()
    );
    assert!(zencodec::find_cause::<At<Replaced>>(&error).is_some());
    assert!(!format!("{error:?} {error}").contains("private-source-payload"));
    assert_eq!(error.error().stage, CollectionStage::Describe);
    assert_eq!(error.error().stop_reason(), None);
}

#[test]
fn collection_factory_shares_discovery_allowance_without_charging_decode() {
    let bytes = crate::jxl_tests::fixture("named_frame_test.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let mut request = request(Composition::Composited);
    request.discovery_attempts = 3;
    let mut images = registered()
        .open_collection(
            ImageFormat::Jxl,
            &mut source,
            request,
            PolicySet::new(),
            stop(),
        )
        .unwrap();
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    images.describe(&id).unwrap();
    let error = images.describe(&id).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<DiscoveryExhausted>(&error)
            .unwrap()
            .used
            .attempts,
        3
    );
    images.decode(&id, &ImageRequest::new()).unwrap();
}

struct Refusing(Arc<std::sync::atomic::AtomicUsize>);
impl Factory for Refusing {
    fn formats(&self) -> &[ImageFormat] {
        &[ImageFormat::Jxl]
    }
    fn open<'a>(
        &self,
        _source: &'a mut dyn DynReadAt,
        _format: ImageFormat,
        _view: Composition,
        _limits: Option<ResourceLimits>,
        _policy: Option<DecodePolicy>,
        _stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(Box::new(At::wrap(Replaced(99))))
    }
}
#[test]
fn collection_factory_first_registration_failure_never_falls_back() {
    let calls = Arc::default();
    let set = CodecSet::new()
        .with_collection(Refusing(Arc::clone(&calls)))
        .with_collection(JxlFactory(64.into()));
    let bytes = crate::jxl_tests::fixture("named_frame_test.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let error = set
        .clone()
        .open_collection(
            ImageFormat::Jxl,
            &mut source,
            request(Composition::Composited),
            PolicySet::new(),
            stop(),
        )
        .err()
        .unwrap();
    assert_eq!(zencodec::find_cause::<Replaced>(&error).unwrap().0, 99);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn collection_factory_native_disk_reads_match_the_same_stored_samples() {
    for big in [false, true] {
        let (bytes, refs) = nested_fixture(big, true, false);
        let path = artifacts().join(format!("factory-disk-{big}.tif"));
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(&bytes).unwrap();
        file.seek(SeekFrom::Start(7)).unwrap();
        // This test owns the file and keeps its bytes unchanged. FileReadAt
        // itself does not provide replacement detection or snapshot isolation.
        let mut source =
            ReadAtShim(zencodec_io::FileReadAt::new(file.try_clone().unwrap()).unwrap());
        let mut images = registered()
            .open_collection(
                ImageFormat::Tiff,
                &mut source,
                request(Composition::Composited),
                PolicySet::new(),
                stop(),
            )
            .unwrap();
        let output = images.first_image(&ImageRequest::new()).unwrap().unwrap();
        assert_eq!(
            output
                .with_pixels(|p| p.as_strided_bytes().to_vec())
                .unwrap(),
            refs[0].bytes
        );
        #[cfg(unix)]
        assert_eq!(file.stream_position().unwrap(), 7);
    }
}

#[test]
fn collection_factory_guide_returns_native_output_after_operation_and_source_drop() {
    let (bytes, refs) = nested_fixture(false, true, false);
    let mut source = ReadAtShim(Positional::new(&bytes));
    let output = crate::collection::checked_registered_first(
        &registered(),
        ImageFormat::Tiff,
        &mut source,
        request(Composition::Composited),
        PolicySet::new(),
        stop(),
    )
    .unwrap()
    .unwrap();
    drop(source);
    drop(bytes);
    assert_eq!(
        output
            .with_pixels(|p| p.as_strided_bytes().to_vec())
            .unwrap(),
        refs[0].bytes
    );
}

#[test]
fn collection_factory_cancellation_survives_opening_and_erasure_at_every_observed_seam() {
    let (bytes, _) = nested_fixture(false, false, false);
    let run =
        |control: &CheckStop| -> Result<(), At<zencodec::operation::CollectionOperationError>> {
            let mut source = ReadAtShim(Positional::new(&bytes));
            let mut images = registered().open_collection(
                ImageFormat::Tiff,
                &mut source,
                request(Composition::Composited),
                PolicySet::new(),
                control.token(),
            )?;
            let mut roots = images.images()?;
            let id = images.next(&mut roots)?.unwrap();
            images.describe(&id)?;
            images.decode(&id, &ImageRequest::new())?;
            Ok(())
        };
    let baseline = CheckStop::default();
    run(&baseline).unwrap();
    assert!(baseline.calls() > 4);
    for reason in [
        zencodec::enough::StopReason::Cancelled,
        zencodec::enough::StopReason::TimedOut,
    ] {
        for at in 0..baseline.calls() {
            let control = CheckStop::with_reason(reason);
            control.after(at);
            let error = run(&control).expect_err("stopped factory operation returned pixels");
            assert_eq!(error.error().stop_reason(), Some(reason), "poll {at}");
            if let Some(policy) = zencodec::find_cause::<zencodec::operation::PolicyFailure>(&error)
            {
                assert_eq!(policy.stop_reason(), Some(reason));
                assert_eq!(
                    policy.record().code,
                    zencodec::operation::DecisionCode::Stopped
                );
            } else {
                assert_eq!(
                    zencodec::find_cause::<zencodec::CodecError>(&error)
                        .unwrap_or_else(|| panic!("missing stop cause at poll {at}: {error:?}"))
                        .category(),
                    zencodec::ErrorCategory::Stopped(reason)
                );
            }
        }
    }
}

struct StopDuringOpen(CheckStop);
impl Factory for StopDuringOpen {
    fn formats(&self) -> &[ImageFormat] {
        &[ImageFormat::Jxl]
    }
    fn open<'a>(
        &self,
        _source: &'a mut dyn DynReadAt,
        _format: ImageFormat,
        _view: Composition,
        _limits: Option<ResourceLimits>,
        _policy: Option<DecodePolicy>,
        _stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause> {
        self.0.after(0);
        Err(Box::new(At::wrap(Replaced(123))))
    }
}

#[test]
fn collection_factory_stop_accessor_retains_coincident_native_and_application_causes() {
    for reason in [
        zencodec::enough::StopReason::Cancelled,
        zencodec::enough::StopReason::TimedOut,
    ] {
        let bytes = crate::jxl_tests::fixture("named_frame_test.jxl");
        let mut source = ReadAtShim(SliceReadAt::new(&bytes));
        let control = CheckStop::with_reason(reason);
        let set = CodecSet::new().with_collection(StopDuringOpen(control.clone()));
        let error = set
            .open_collection(
                ImageFormat::Jxl,
                &mut source,
                request(Composition::Composited),
                PolicySet::new(),
                control.token(),
            )
            .err()
            .unwrap();
        assert_eq!(error.error().stage, CollectionStage::Open);
        assert_eq!(error.error().stop_reason(), Some(reason));
        assert_eq!(zencodec::find_cause::<Replaced>(&error).unwrap().0, 123);
        assert!(zencodec::find_cause::<At<Replaced>>(&error).is_some());

        let control = CheckStop::with_reason(reason);
        let resolver = |_: Question<'_>| -> Result<Decision, At<Replaced>> {
            control.after(0);
            Err(At::wrap(Replaced(456)))
        };
        let mut images = registered()
            .open_collection(
                ImageFormat::Jxl,
                &mut source,
                request(Composition::Composited),
                resolver,
                control.token(),
            )
            .unwrap();
        let error = images.first_image(&ImageRequest::new()).err().unwrap();
        assert_eq!(error.error().stage, CollectionStage::Policy);
        assert_eq!(error.error().stop_reason(), Some(reason));
        assert_eq!(zencodec::find_cause::<Replaced>(&error).unwrap().0, 456);
        assert!(zencodec::find_cause::<At<Replaced>>(&error).is_some());
        assert!(images.is_finished());
        let before = control.calls();
        let finished = images.images().err().unwrap();
        assert_eq!(finished.error().stage, CollectionStage::Finished);
        assert_eq!(finished.error().stop_reason(), Some(reason));
        assert_eq!(
            control.calls(),
            before,
            "finished operation entered the stop callback"
        );
    }
}

#[test]
fn collection_factory_inherited_requirements_refuse_before_any_source_work() {
    for format in [ImageFormat::Tiff, ImageFormat::Jxl] {
        for (set, expected) in [
            (
                registered().with_limits(ResourceLimits::none().with_max_memory(1)),
                FactoryError::UnsupportedLimits {
                    unsupported: ResourceLimits::none().with_max_memory(1),
                },
            ),
            (
                registered().with_decode_policy(DecodePolicy::strict()),
                FactoryError::UnsupportedDecodePolicy {
                    unsupported: DecodePolicy::strict(),
                },
            ),
        ] {
            let mut source = ReadAtShim(Positional::new(&[]));
            let error = set
                .clone()
                .open_collection(
                    format,
                    &mut source,
                    request(Composition::Composited),
                    PolicySet::new(),
                    stop(),
                )
                .err()
                .unwrap();
            assert_eq!(
                *zencodec::find_cause::<FactoryError>(&error).unwrap(),
                expected
            );
            assert_eq!(error.error().stage, CollectionStage::Open);
            assert_eq!(error.error().stop_reason(), None);
            assert!(zencodec::find_cause::<At<FactoryError>>(&error).is_some());
            assert_eq!(source.0.reads.get(), 0);
            assert_eq!(source.0.lengths.get(), 0);
        }
    }
}

#[test]
fn collection_factory_all_inherited_stops_survive_registry_drop_and_policy_calls() {
    use zencodec::{
        enough::StopReason,
        how_far::{SharedPulse, StopOnly},
    };
    let (bytes, _) = nested_fixture(true, true, false);
    for reason in [StopReason::Cancelled, StopReason::TimedOut] {
        for stopped in 0..3 {
            // Before opening, after opening, and inside the publication callback.
            for phase in 0..3 {
                let controls = std::array::from_fn::<_, 3, _>(|_| CheckStop::with_reason(reason));
                let set = registered()
                    .with_pulse(SharedPulse::new(StopOnly::new(controls[0].clone())))
                    .with_stop(controls[1].token());
                let resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                    if phase == 2 {
                        controls[stopped].after(0);
                    }
                    Ok(Decision::UsePreset)
                };
                let mut source = ReadAtShim(Positional::new(&bytes));
                if phase == 0 {
                    controls[stopped].after(0);
                }
                let opened = set.clone().open_collection(
                    ImageFormat::Tiff,
                    &mut source,
                    request(Composition::Composited),
                    resolver,
                    controls[2].token(),
                );
                drop(set);
                if phase == 0 {
                    let error = opened.err().unwrap();
                    assert_eq!(error.error().stage, CollectionStage::Stopped);
                    assert_eq!(error.error().stop_reason(), Some(reason));
                    assert_eq!(source.0.reads.get(), 0);
                } else {
                    let mut images = opened.unwrap();
                    if phase == 1 {
                        controls[stopped].after(0);
                    }
                    let error = images.first_image(&ImageRequest::new()).err().unwrap();
                    assert_eq!(error.error().stop_reason(), Some(reason));
                    assert!(images.is_finished());
                    let counts = controls.each_ref().map(CheckStop::calls);
                    let finished = images.images().err().unwrap();
                    assert_eq!(finished.error().stop_reason(), Some(reason));
                    assert_eq!(counts, controls.each_ref().map(CheckStop::calls));
                }
            }
        }
    }
}

struct WrongView;
impl Factory for WrongView {
    fn formats(&self) -> &[ImageFormat] {
        &[ImageFormat::Tiff]
    }
    fn open<'a>(
        &self,
        source: &'a mut dyn DynReadAt,
        format: ImageFormat,
        _view: Composition,
        limits: Option<ResourceLimits>,
        policy: Option<DecodePolicy>,
        stop: StopToken,
    ) -> Result<ErasedCollection<'a>, Cause> {
        crate::collection_factories::TiffFactory(Allowances::default()).open(
            source,
            format,
            Composition::Composited,
            limits,
            policy,
            stop,
        )
    }
}

#[test]
fn collection_factory_cannot_silently_replace_the_required_view() {
    let (bytes, _) = nested_fixture(false, false, false);
    let mut source = ReadAtShim(Positional::new(&bytes));
    let error = CodecSet::new()
        .with_collection(WrongView)
        .open_collection(
            ImageFormat::Tiff,
            &mut source,
            request(Composition::Uncomposited),
            PolicySet::new(),
            stop(),
        )
        .err()
        .unwrap();
    assert_eq!(error.error().stage, CollectionStage::Open);
    assert_eq!(
        *zencodec::find_cause::<SelectionError>(&error).unwrap(),
        SelectionError::UnsupportedComposition,
    );
}

#[test]
fn collection_factory_tiff_dimensions_admit_actual_stored_extent_before_payloads() {
    use super::description_checks::NoPixels;
    use zencodec::{LimitExceeded, ThreadingPolicy};
    for big in [false, true] {
        for planar in [false, true] {
            let (bytes, refs) = nested_fixture(big, planar, false);
            let (root, child) = (&refs[0], &refs[1]);
            let pixels = u64::from(child.width) * u64::from(child.height);
            for (limits, expected) in [
                (
                    ResourceLimits::none().with_max_width(child.width),
                    LimitExceeded::Width {
                        actual: root.width,
                        max: child.width,
                    },
                ),
                (
                    ResourceLimits::none().with_max_height(child.height),
                    LimitExceeded::Height {
                        actual: root.height,
                        max: child.height,
                    },
                ),
                (
                    ResourceLimits::none().with_max_pixels(pixels),
                    LimitExceeded::Pixels {
                        actual: u64::from(root.width) * u64::from(root.height),
                        max: pixels,
                    },
                ),
            ] {
                let source = NoPixels::new(bytes.clone());
                let denied = source.denied.clone();
                let enabled = source.enabled.clone();
                let mut source = ReadAtShim(source);
                let set = registered()
                    .with_limits(
                        limits
                            .with_threading(ThreadingPolicy::Sequential)
                            .with_max_output(1),
                    )
                    .with_decode_policy(DecodePolicy::none());
                let mut images = set
                    .open_collection(
                        ImageFormat::Tiff,
                        &mut source,
                        request(Composition::Composited),
                        PolicySet::new(),
                        stop(),
                    )
                    .unwrap();
                let mut roots = images.images().unwrap();
                let root_id = images.next(&mut roots).unwrap().unwrap();
                assert_eq!(
                    images.describe(&root_id).unwrap().stored_size,
                    (root.width, root.height)
                );
                // A tiny ROI must not redefine the stored-image limit's scope.
                for requested in [
                    ImageRequest::new(),
                    ImageRequest::new().with_region(Rect {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    }),
                ] {
                    let error = images.decode(&root_id, &requested).unwrap_err();
                    assert_eq!(
                        zencodec::find_cause::<LimitExceeded>(&error),
                        Some(&expected)
                    );
                    assert_eq!(error.error().stage, CollectionStage::Decode);
                    assert_eq!(error.error().stop_reason(), None);
                    assert!(!images.is_finished());
                    assert_eq!(
                        denied.get(),
                        0,
                        "over-limit image tried to read compressed pixels"
                    );
                }
                let mut children = images.children(&root_id).unwrap();
                let child_id = images.next(&mut children).unwrap().unwrap();
                // Positive control: the admitted child really reaches pixel data.
                let error = images.decode(&child_id, &ImageRequest::new()).unwrap_err();
                assert!(zencodec::find_cause::<LimitExceeded>(&error).is_none());
                assert!(denied.get() > 0);
                enabled.set(false);
                let output = images.decode(&child_id, &ImageRequest::new()).unwrap();
                output
                    .with_pixels(|p| assert_eq!(p.as_contiguous_bytes().unwrap(), child.bytes))
                    .unwrap();
            }
        }
    }
}

#[test]
fn collection_factory_configuration_fields_are_enforced_or_refused_individually() {
    use zencodec::AllocPreference;
    let unsupported = [
        ResourceLimits::none().with_max_memory(1),
        ResourceLimits::none().with_max_input_bytes(1),
        ResourceLimits::none().with_max_frames(1),
        ResourceLimits::none().with_max_animation_ms(1),
        ResourceLimits::none().with_max_total_pixels(1),
        ResourceLimits::none().with_prefer_fallible_allocations(AllocPreference::Fallible),
        ResourceLimits::none().with_prefer_fallible_allocations(AllocPreference::Infallible),
    ];
    for format in [ImageFormat::Tiff, ImageFormat::Jxl] {
        let unqualified_dimensions = [
            ResourceLimits::none().with_max_width(1),
            ResourceLimits::none().with_max_height(1),
            ResourceLimits::none().with_max_pixels(1),
        ];
        let mixed = ResourceLimits::none().with_max_width(1).with_max_memory(1);
        let dimensions = if format == ImageFormat::Jxl {
            &unqualified_dimensions[..]
        } else {
            &[]
        };
        for limits in unsupported
            .into_iter()
            .chain(dimensions.iter().copied())
            .chain([mixed])
        {
            let mut source = ReadAtShim(Positional::new(&[]));
            let error = registered()
                .with_limits(limits)
                .open_collection(
                    format,
                    &mut source,
                    request(Composition::Composited),
                    PolicySet::new(),
                    stop(),
                )
                .err()
                .unwrap();
            assert_eq!(
                zencodec::find_cause::<FactoryError>(&error),
                Some(&FactoryError::UnsupportedLimits {
                    unsupported: {
                        // Only the TIFF width field in the mixed request is qualified.
                        let mut expected = limits;
                        if format == ImageFormat::Tiff {
                            expected.max_width = None;
                        }
                        expected
                    }
                }),
                "{format:?}: {limits:?}"
            );
            assert_eq!((source.0.reads.get(), source.0.lengths.get()), (0, 0));
        }
        for allow in [false, true] {
            for policy in [
                DecodePolicy::none().with_allow_icc(allow),
                DecodePolicy::none().with_allow_exif(allow),
                DecodePolicy::none().with_allow_xmp(allow),
                DecodePolicy::none().with_allow_progressive(allow),
                DecodePolicy::none().with_allow_animation(allow),
                DecodePolicy::none().with_allow_truncated(allow),
                DecodePolicy::none().with_strict(allow),
            ] {
                let mut source = ReadAtShim(Positional::new(&[]));
                let error = registered()
                    .with_decode_policy(policy)
                    .open_collection(
                        format,
                        &mut source,
                        request(Composition::Composited),
                        PolicySet::new(),
                        stop(),
                    )
                    .err()
                    .unwrap();
                assert_eq!(
                    zencodec::find_cause::<FactoryError>(&error),
                    Some(&FactoryError::UnsupportedDecodePolicy {
                        unsupported: policy
                    }),
                    "{format:?}: {policy:?}"
                );
                assert_eq!((source.0.reads.get(), source.0.lengths.get()), (0, 0));
            }
        }
    }
    let bytes = crate::jxl_tests::fixture("named_frame_test.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let mut images = registered()
        .with_limits(ResourceLimits::none().with_max_output(1))
        .with_decode_policy(DecodePolicy::none())
        .open_collection(
            ImageFormat::Jxl,
            &mut source,
            request(Composition::Composited),
            PolicySet::new(),
            stop(),
        )
        .unwrap();
    assert!(images.first_image(&ImageRequest::new()).unwrap().is_some());
}
