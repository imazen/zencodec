use super::control::CheckStop;
use super::{
    collection::{Collection, first_child, first_image},
    image_items::{Composition, ImageRequest, Rect, SelectionError},
    jxl_items::{JxlItems, Plane, sequential},
};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

// Immutable corpus revision and cache key; a mutable branch is not a fixture pin.
fn fixture_path(name: &str) -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    let root = ROOT.get_or_init(|| {
        codec_corpus::Corpus::new().unwrap().tar_url(
            "zencodec-jxl-8399f17c54c8138f65304bbc46efc80c269c7402",
            "https://codeload.github.com/imazen/zenjxl-decoder/tar.gz/8399f17c54c8138f65304bbc46efc80c269c7402",
        ).expect("download pinned JXL corpus via codec-corpus")
        .join("zenjxl-decoder/resources/test")
    });
    root.join(name)
}

pub(crate) fn fixture(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn assert_cancelled(error: &(dyn std::error::Error + 'static)) {
    assert_stopped(error, zencodec::enough::StopReason::Cancelled);
}

fn assert_stopped(
    error: &(dyn std::error::Error + 'static),
    expected: zencodec::enough::StopReason,
) {
    if let Some(cause) = zencodec::find_cause::<zencodec::CodecError>(error) {
        assert_eq!(cause.category(), zencodec::ErrorCategory::Stopped(expected));
    } else {
        let native = zencodec::find_cause::<zenjxl_decoder::api::Error>(error)
            .unwrap_or_else(|| panic!("not a cancellation cause: {error:?}"));
        assert_eq!(native.stop_reason(), Some(expected));
    }
}

#[test]
fn cancellation_covers_native_jxl_open_discovery_and_selected_decode() {
    let data = fixture("conformance_test_images/blendmodes.jxl");
    let run = |stop: &CheckStop| -> Result<_, super::tiff_region::Failure> {
        let mut collection =
            JxlItems::open_with_stop(&data, Composition::Composited, 4096, stop.token())?;
        Ok(first_image(&mut collection, &ImageRequest::default())?)
    };
    let counted = CheckStop::default();
    let output = run(&counted).unwrap().unwrap();
    let expected = super::output_tests::expected_output(&data);
    let mapped = output.map().unwrap();
    assert_eq!(mapped.plane_count(), expected[0].len());
    for (index, expected) in expected[0].iter().enumerate() {
        let actual = mapped.plane(index).unwrap().unwrap();
        assert_eq!(
            actual.samples.dimensions(),
            (expected.width, expected.height)
        );
        for y in 0..expected.height {
            let super::image_output::Row::Bytes(row) = actual.samples.row(y).unwrap() else {
                panic!("native f32 output became code words")
            };
            assert_eq!(
                row,
                &expected.bytes[y * expected.stride..][..expected.width * expected.channels * 4]
            );
        }
    }
    let checkpoints = counted.calls();
    assert!(
        checkpoints > 5 && checkpoints < 2048,
        "unexpected fixture checkpoints: {checkpoints}"
    );
    for reason in [
        zencodec::enough::StopReason::Cancelled,
        zencodec::enough::StopReason::TimedOut,
    ] {
        for at in 0..checkpoints {
            let stop = CheckStop::with_reason(reason);
            stop.after(at);
            let error = run(&stop).expect_err("stopped JXL output escaped");
            assert_stopped(error.as_ref(), reason);
        }
    }
}

#[test]
fn cancellation_blocks_cached_and_exhausted_jxl_cursors() {
    let data = fixture("conformance_test_images/blendmodes.jxl");
    let stop = CheckStop::default();
    let mut collection =
        JxlItems::open_with_stop(&data, Composition::Composited, 4096, stop.token()).unwrap();
    let mut complete = collection.images().unwrap();
    while Collection::next(&mut collection, &mut complete)
        .unwrap()
        .is_some()
    {}
    let mut cached = collection.images().unwrap();
    stop.after(0);
    for cursor in [&mut complete, &mut cached] {
        let error = Collection::next(&mut collection, cursor).unwrap_err();
        assert_cancelled(&error);
    }
    assert!(collection.images().is_err());
}

#[test]
fn cancellation_during_native_jxl_scan_cannot_appear_exhausted() {
    let data = fixture("conformance_test_images/blendmodes.jxl");
    let stop = CheckStop::default();
    let mut collection =
        JxlItems::open_with_stop(&data, Composition::Composited, 4096, stop.token()).unwrap();
    stop.after(1);
    let error = collection.next().err().unwrap();
    assert_cancelled(error.as_ref());
    assert!(collection.next().is_err());
}

#[test]
fn cancellation_native_timeout_preserves_reason() {
    struct Deadline;
    impl zencodec::enough::Stop for Deadline {
        fn check(&self) -> Result<(), zencodec::enough::StopReason> {
            Err(zencodec::enough::StopReason::TimedOut)
        }
    }
    let data = fixture("conformance_test_images/blendmodes.jxl");
    let error = zenjxl_decoder::decode_with(
        &data,
        zenjxl_decoder::api::JxlDecoderOptions::default().with_stop(std::sync::Arc::new(Deadline)),
    )
    .err()
    .expect("native deadline must stop work");
    assert_eq!(
        error.error().kind(),
        zenjxl_decoder::api::ErrorClass::Cancelled
    );
    assert_eq!(
        error.error().stop_reason(),
        Some(zencodec::enough::StopReason::TimedOut)
    );
    assert!(
        error
            .to_string()
            .contains(&zencodec::enough::StopReason::TimedOut.to_string()),
        "native error lost the deadline reason: {error}"
    );
}

#[test]
fn discovery_budget_native_jxl_counts_cached_results_and_exhaustion() {
    let data = fixture("conformance_test_images/blendmodes.jxl");
    let open = || JxlItems::open(&data, Composition::Composited, 4096).unwrap();
    super::discovery::verify_repeated_discovery(open());
    super::discovery::verify_exhaustion_is_charged(open());
}

#[test]
fn discovery_budget_native_jxl_counts_refusals_before_blocking_native_work() {
    use super::discovery::{Discovery, Exhausted, Method};
    let data = fixture("conformance_test_images/blendmodes.jxl");
    let stop = CheckStop::default();
    let native =
        JxlItems::open_with_stop(&data, Composition::Composited, 4096, stop.token()).unwrap();
    let mut collection = Discovery::new(native, 4);
    let mut cursor = collection.images().unwrap();
    let id = collection.next(&mut cursor).unwrap().unwrap();
    for _ in 0..2 {
        let error = collection.children(&id).err().unwrap();
        assert_eq!(
            zencodec::find_cause::<SelectionError>(&error),
            Some(&SelectionError::UnsupportedChildren)
        );
    }
    assert_eq!(collection.usage().backend_errors, 2);
    let before = stop.calls();
    let error = collection.next(&mut cursor).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<Exhausted>(&error).unwrap().method,
        Method::Next
    );
    assert_eq!(
        stop.calls(),
        before,
        "budget refusal reached native collection"
    );
}

pub(crate) fn independent_metadata(name: &str, coalescing: bool) -> String {
    independent_metadata_at(&fixture_path(name), coalescing)
}

pub(crate) fn independent_metadata_at(path: &Path, coalescing: bool) -> String {
    independent_inspect_at(path, coalescing, false)
}

pub(crate) fn independent_inspect_at(path: &Path, coalescing: bool, full: bool) -> String {
    independent_inspect_mode(path, coalescing, full.then_some("full"))
}

pub(crate) fn independent_inspect_mode(
    path: &Path,
    coalescing: bool,
    mode: Option<&str>,
) -> String {
    let tool = std::env::var("JXL_SELECTION_ORACLE").expect("JXL_SELECTION_ORACLE is required");
    let output = std::process::Command::new("timeout")
        .args(["--kill-after=5s", "30s", &tool])
        .arg(path)
        .arg(if coalescing { "1" } else { "0" })
        .args(mode)
        .output()
        .expect("run independent libjxl oracle");
    assert!(
        output.status.success(),
        "libjxl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("version "));
    eprintln!("libjxl {} coalescing={coalescing}: {text}", path.display());
    text
}

fn independent_frames(name: &str, coalescing: bool) -> Vec<Vec<i64>> {
    independent_metadata(name, coalescing)
        .lines()
        .filter_map(|line| line.strip_prefix("frame "))
        .map(|line| {
            line.split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect()
        })
        .collect()
}

pub(crate) fn exact_rows(expected: &[Plane], got: &[Plane]) {
    assert_eq!(got.len(), expected.len());
    for (actual, expected) in got.iter().zip(expected) {
        assert_eq!(
            (actual.width, actual.height, actual.channels),
            (expected.width, expected.height, expected.channels)
        );
        let row_bytes = expected.width * expected.channels * 4;
        for y in 0..expected.height {
            let row = &actual.bytes[y * actual.stride..(y + 1) * actual.stride];
            assert_eq!(
                &row[..row_bytes],
                &expected.bytes[y * expected.stride..y * expected.stride + row_bytes]
            );
            assert!(
                row[row_bytes..].iter().all(|&b| b == 0xa5),
                "padding changed"
            );
        }
    }
}

#[test]
fn selected_jxl_presentations_match_native_sequential_with_dependencies() {
    let mut dependencies = 0;
    let mut count = 0;
    for name in [
        "conformance_test_images/blendmodes.jxl",
        "conformance_test_images/animation_spline.jxl",
        "conformance_test_images/animation_newtons_cradle.jxl",
        "conformance_test_images/animation_icos4d.jxl",
        "multiple_layers_noise_spline.jxl",
        "named_frame_test.jxl",
    ] {
        let data = fixture(name);
        let expected = sequential(&data, true).unwrap();
        let expected_owned = super::output_tests::expected_output(&data);
        let factories = zencodec::CodecSet::new()
            .with_collection(zenjxl::collection::JxlCollectionConfig::new(4096, 4096));
        let mut source = zencodec::source::ReadAtShim(zencodec::source::SliceReadAt::new(&data));
        let mut packaged = factories
            .open_collection(
                zencodec::ImageFormat::Jxl,
                &mut source,
                zencodec::collection::OpenRequest::new(Composition::Composited, 16384),
                zencodec::operation::PolicySet::new(),
                zencodec::StopToken::new(zencodec::Unstoppable),
            )
            .unwrap();
        let mut cursor = packaged.images().unwrap();
        let mut package_ids = Vec::new();
        while let Some(id) = packaged.next(&mut cursor).unwrap() {
            package_ids.push(id);
        }
        let mut items = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
        let mut selected = Vec::new();
        while let Some(item) = items.next().unwrap() {
            assert_eq!(item.native.index, selected.len());
            let target = item.native.seek_target.as_ref().expect("seekable fixture");
            dependencies += usize::from(target.visible_frames_to_skip > 0);
            selected.push(item);
        }
        assert_eq!(selected.len(), expected.len(), "{name}");
        assert_eq!(
            package_ids.len(),
            selected.len(),
            "packaged discovery: {name}"
        );
        let oracle = independent_frames(name, true);
        assert_eq!(
            selected.len(),
            oracle.len(),
            "libjxl visible presentation count: {name}"
        );
        for (item, fields) in selected.iter().zip(&oracle) {
            assert_eq!(
                i64::from(item.native.duration_ticks),
                fields[0],
                "native duration ticks"
            );
            let plane = &expected[item.native.index][0];
            assert_eq!(
                (plane.width as i64, plane.height as i64),
                (fields[1], fields[2])
            );
        }
        assert!(items.next().unwrap().is_none());
        // Reverse order proves an ID is not the current decoder position.
        for item in selected.iter().rev() {
            exact_rows(
                &expected[item.native.index],
                &items.decode(&item.image, 13).unwrap(),
            );
            let id = &package_ids[item.native.index];
            assert_eq!(
                packaged.describe(id).unwrap(),
                Collection::describe(&mut items, &item.image).unwrap(),
                "packaged description: {name}"
            );
            let output = packaged.decode(id, &ImageRequest::new()).unwrap();
            exact_rows(
                &expected_owned[item.native.index],
                &super::output_tests::mapped_planes(&output),
            );
        }
        count += selected.len();
        eprintln!("JXL {name}: {} visible presentations", selected.len());
    }
    assert!(dependencies > 0, "must exercise dependent presentations");
    eprintln!(
        "JXL selection: {count} presentations through private and packaged adapters, {dependencies} require visible dependency replay"
    );
}

#[test]
fn jxl_native_coalescing_requirement_is_honored() {
    let mut distinct_views = 0;
    for name in [
        "conformance_test_images/blendmodes.jxl",
        "multiple_layers_noise_spline.jxl",
    ] {
        let data = fixture(name);
        let native = sequential(&data, true).unwrap();
        // This session's old probe first enforced refusal of silent composition.
        // Qualified F32 layers now execute; the cardinality must match the raw
        // oracle rather than silently returning visible presentations.
        let raw = sequential(&data, false).unwrap();
        let composited = independent_frames(name, true);
        assert_eq!(native.len(), composited.len());
        let layers = independent_frames(name, false);
        assert_eq!(raw.len(), layers.len());
        distinct_views += usize::from(composited != layers);
        let err = JxlItems::open(&data, Composition::Uncomposited, 4096)
            .err()
            .unwrap();
        assert_eq!(
            err.downcast_ref::<SelectionError>(),
            Some(&SelectionError::UnsupportedComposition)
        );
    }
    assert!(distinct_views > 0, "oracle must prove the two views differ");
    // The required-view refusal precedes native parsing or any pixel work.
    let err = JxlItems::open(&[], Composition::Uncomposited, 4096)
        .err()
        .unwrap();
    assert_eq!(
        err.downcast_ref::<SelectionError>(),
        Some(&SelectionError::UnsupportedComposition)
    );
}

#[test]
fn jxl_selection_rejects_foreign_identity_and_fuses_discovery_refusal() {
    let data = fixture("conformance_test_images/blendmodes.jxl");
    let mut first = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
    let mut second = JxlItems::open(&data, Composition::Composited, 0).unwrap();
    let item = first.next().unwrap().unwrap();
    let err = second.decode(&item.image, 0).unwrap_err();
    assert_eq!(
        err.downcast_ref::<SelectionError>(),
        Some(&SelectionError::ForeignImage)
    );
    let err = second.next().err().unwrap();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(err.as_ref()),
        Some(&SelectionError::DiscoveryLimit)
    );
    assert!(
        second.next().is_err(),
        "a refused scan cannot appear exhausted"
    );
}

#[test]
fn discovery_budget_native_headers_bound_hidden_work_and_retain_terminal_cause() {
    use super::jxl_items::Limits;
    let data = fixture("multiple_layers_noise_spline.jxl");
    let assert_limit = |error: &(dyn std::error::Error + 'static), bound| {
        let cause = zencodec::find_cause::<zenjxl_decoder::api::Error>(error)
            .expect("lost native header allowance cause");
        assert!(
            matches!(cause, zenjxl_decoder::api::Error::LimitExceeded {
            resource: "frame_headers", actual, limit,
        } if *limit == bound && *actual == bound + 1),
            "{cause:?}"
        );
    };
    for bound in [0, 4] {
        let stop = CheckStop::default();
        let mut collection = JxlItems::open_with_stop(
            &data,
            Composition::Composited,
            Limits {
                images: 1,
                frame_headers: Some(bound),
            },
            stop.token(),
        )
        .unwrap();
        let mut cursor = collection.images().unwrap();
        let error = Collection::next(&mut collection, &mut cursor).unwrap_err();
        assert_limit(&error, bound);
        let before = stop.calls();
        // A terminal discovery failure cannot become EOF or a generic message,
        // and retrying it must not execute the native parser again.
        let error = Collection::next(&mut collection, &mut cursor).unwrap_err();
        assert_limit(&error, bound);
        let error = collection.images().err().unwrap();
        assert_limit(&error, bound);
        let error = collection.next().err().unwrap();
        assert_limit(error.as_ref(), bound);
        assert_eq!(stop.calls(), before, "terminal retry reached native work");
    }
    let mut collection = JxlItems::open(
        &data,
        Composition::Composited,
        Limits {
            images: 1,
            frame_headers: Some(5),
        },
    )
    .unwrap();
    let expected = super::output_tests::expected_output(&data);
    let image = first_image(&mut collection, &ImageRequest::default())
        .unwrap()
        .unwrap();
    exact_rows(&expected[0], &super::output_tests::mapped_planes(&image));
    // The documented bound is per native decoder, not a shared total across
    // independent selected decodes. Cached discovery still returns the same ID.
    let again = first_image(&mut collection, &ImageRequest::default())
        .unwrap()
        .unwrap();
    exact_rows(&expected[0], &super::output_tests::mapped_planes(&again));
}

#[test]
fn common_consumer_selects_jxl_and_refuses_unknown_child_support() {
    let data = fixture("named_frame_test.jxl");
    let expected = crate::output_tests::expected_output(&data);
    let mut collection = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
    assert_eq!(Collection::view(&collection), Composition::Composited);
    exact_rows(
        &expected[0],
        &crate::output_tests::mapped_planes(
            &first_image(&mut collection, &Default::default())
                .unwrap()
                .unwrap(),
        ),
    );
    // A second consumer pass must yield the same operation-bound identity and
    // pixels, even though the native scanner has already visited this frame.
    exact_rows(
        &expected[0],
        &crate::output_tests::mapped_planes(
            &first_image(&mut collection, &Default::default())
                .unwrap()
                .unwrap(),
        ),
    );
    let mut a = Collection::images(&mut collection).unwrap();
    let mut b = Collection::images(&mut collection).unwrap();
    let id = Collection::next(&mut collection, &mut a).unwrap().unwrap();
    assert_eq!(
        Collection::next(&mut collection, &mut b).unwrap().unwrap(),
        id
    );
    let err = first_child(&mut collection, &Default::default()).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&err),
        Some(&SelectionError::UnsupportedChildren)
    );

    let mut other = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
    let err = Collection::next(&mut other, &mut a).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&err),
        Some(&SelectionError::ForeignCursor)
    );
}

#[test]
fn common_jxl_region_requirement_refuses_without_substituting_full_output() {
    let data = fixture("named_frame_test.jxl");
    let mut collection = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
    let mut cursor = Collection::images(&mut collection).unwrap();
    let id = Collection::next(&mut collection, &mut cursor)
        .unwrap()
        .unwrap();
    let request = ImageRequest::new().with_region(Rect {
        x: 1,
        y: 1,
        width: 1,
        height: 1,
    });
    let error = Collection::decode(&mut collection, &id, &request).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&error),
        Some(&SelectionError::UnsupportedRegion)
    );
    // The failure is local to the request; the same selected presentation still
    // decodes exactly when the caller explicitly asks for the full image.
    let output = Collection::decode(&mut collection, &id, &ImageRequest::default()).unwrap();
    exact_rows(
        &crate::output_tests::expected_output(&data)[0],
        &crate::output_tests::mapped_planes(&output),
    );
    let mut other = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
    let error = Collection::decode(&mut other, &id, &request).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&error),
        Some(&SelectionError::ForeignImage)
    );
}
