use super::{
    collection::{Collection, first_child, first_image},
    image_items::{Composition, ImageRequest, Rect, SelectionError},
    jxl_items::{JxlItems, Plane, sequential},
};
use std::{path::PathBuf, sync::OnceLock};

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

fn fixture(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn independent_frames(name: &str, coalescing: bool) -> Vec<Vec<i64>> {
    let tool = std::env::var("JXL_SELECTION_ORACLE").expect("JXL_SELECTION_ORACLE is required");
    let output = std::process::Command::new("timeout")
        .args(["--kill-after=5s", "30s", &tool])
        .arg(fixture_path(name))
        .arg(if coalescing { "1" } else { "0" })
        .output()
        .expect("run independent libjxl oracle");
    assert!(
        output.status.success(),
        "libjxl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("version "));
    eprintln!("libjxl {name} coalescing={coalescing}: {text}");
    text.lines()
        .filter_map(|line| line.strip_prefix("frame "))
        .map(|line| {
            line.split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect()
        })
        .collect()
}

fn exact_rows(expected: &[Plane], got: &[Plane]) {
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
        let mut items = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
        let mut selected = Vec::new();
        while let Some(item) = items.next().unwrap() {
            assert_eq!(item.native.index, selected.len());
            let target = item.native.seek_target.as_ref().expect("seekable fixture");
            dependencies += usize::from(target.visible_frames_to_skip > 0);
            selected.push(item);
        }
        assert_eq!(selected.len(), expected.len(), "{name}");
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
        }
        count += selected.len();
        eprintln!("JXL {name}: {} visible presentations", selected.len());
    }
    assert!(dependencies > 0, "must exercise dependent presentations");
    eprintln!(
        "JXL selection: {count} presentations, {dependencies} require visible dependency replay"
    );
}

#[test]
fn jxl_native_coalescing_flag_is_not_a_layer_contract() {
    let mut distinct_views = 0;
    for name in [
        "conformance_test_images/blendmodes.jxl",
        "multiple_layers_noise_spline.jxl",
    ] {
        let data = fixture(name);
        // A control documenting the pinned backend's currently ignored option.
        // If the backend implements it, this assertion forces adapter review.
        assert_eq!(
            sequential(&data, true).unwrap(),
            sequential(&data, false).unwrap()
        );
        let composited = independent_frames(name, true);
        let layers = independent_frames(name, false);
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
        err.downcast_ref::<SelectionError>(),
        Some(&SelectionError::DiscoveryLimit)
    );
    assert!(
        second.next().is_err(),
        "a refused scan cannot appear exhausted"
    );
}

#[test]
fn common_consumer_selects_jxl_and_refuses_unknown_child_support() {
    let data = fixture("named_frame_test.jxl");
    let expected = sequential(&data, true).unwrap();
    let mut collection = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
    assert_eq!(Collection::view(&collection), Composition::Composited);
    exact_rows(
        &expected[0],
        &first_image(&mut collection, &Default::default())
            .unwrap()
            .unwrap(),
    );
    // A second consumer pass must yield the same operation-bound identity and
    // pixels, even though the native scanner has already visited this frame.
    exact_rows(
        &expected[0],
        &first_image(&mut collection, &Default::default())
            .unwrap()
            .unwrap(),
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
    let request = ImageRequest {
        region: Rect {
            x: 1,
            y: 1,
            width: 1,
            height: 1,
        }
        .into(),
    };
    let error = Collection::decode(&mut collection, &id, &request).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&error),
        Some(&SelectionError::UnsupportedRegion)
    );
    // The failure is local to the request; the same selected presentation still
    // decodes exactly when the caller explicitly asks for the full image.
    let output = Collection::decode(&mut collection, &id, &ImageRequest::default()).unwrap();
    exact_rows(&sequential(&data, true).unwrap()[0], &output);
    let mut other = JxlItems::open(&data, Composition::Composited, 4096).unwrap();
    let error = Collection::decode(&mut other, &id, &request).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&error),
        Some(&SelectionError::ForeignImage)
    );
}
