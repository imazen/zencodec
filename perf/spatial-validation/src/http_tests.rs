use super::*;
use crate::http_source::{Failure, FailureKind, HttpSource};

fn write_fixture(name: &str, bytes: &[u8]) {
    use std::io::Write;
    let root =
        std::env::var_os("SPATIAL_HTTP_ARTIFACTS").expect("HTTP harness must supply artifacts");
    let path = std::path::Path::new(&root).join(format!("{name}.tif"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}
fn url(mode: &str, name: &str, fault_at: u64) -> String {
    let base = std::env::var("SPATIAL_HTTP_URL").expect("HTTP harness must supply its server URL");
    format!("{base}/{mode}/{name}.tif?at={fault_at}")
}
fn child<R: std::io::Read + std::io::Seek>(
    collection: &mut TiffCollection<R>,
) -> crate::image_items::ImageId {
    let mut roots = collection.images().unwrap();
    let root = collection.next(&mut roots).unwrap().unwrap();
    let mut children = collection.children(&root).unwrap();
    collection.next(&mut children).unwrap().unwrap()
}
fn assert_cause(error: &(dyn std::error::Error + 'static), kind: FailureKind) {
    assert_eq!(
        zencodec::find_cause::<Failure>(error).unwrap().kind,
        kind,
        "{error:?}"
    );
}

#[test]
fn http_snapshot_native_tiff_matches_slice_with_direct_and_buffered_io() {
    for big in [false, true] {
        for planar in [false, true] {
            let (bytes, refs) = nested_fixture(big, planar, false);
            let check = |output: crate::image_output::ImageOutput| {
                output
                    .with_pixels(|pixels| {
                        assert_eq!(pixels.as_contiguous_bytes().unwrap(), refs[1].bytes);
                    })
                    .unwrap();
            };
            for buffered in [false, true] {
                let name = format!("control-{big}-{planar}-{buffered}");
                write_fixture(&name, &bytes);
                let source = HttpSource::open(url("stable", &name, 0), 65536).unwrap();
                let input = ReadAtReader::new(source);
                if buffered {
                    let mut collection = TiffCollection::open(
                        std::io::BufReader::with_capacity(4096, input),
                        Composition::Composited,
                        Allowances::default(),
                    )
                    .unwrap();
                    check(
                        first_child(&mut collection, &ImageRequest::default())
                            .unwrap()
                            .unwrap(),
                    );
                } else {
                    let mut collection =
                        TiffCollection::open(input, Composition::Composited, Allowances::default())
                            .unwrap();
                    check(
                        first_child(&mut collection, &ImageRequest::default())
                            .unwrap()
                            .unwrap(),
                    );
                }
            }
        }
    }
}

#[test]
fn http_snapshot_change_after_native_discovery_refuses_selected_pixels_and_retry() {
    for big in [false, true] {
        let (bytes, refs) = nested_fixture(big, true, false);
        let name = format!("after-discovery-{big}");
        write_fixture(&name, &bytes);
        let source = HttpSource::open(url("stable", &name, 0), 65536).unwrap();
        let attempts = source.attempts.clone();
        let mut collection = TiffCollection::open(
            ReadAtReader::new(source),
            Composition::Composited,
            Allowances::default(),
        )
        .unwrap();
        let id = child(&mut collection);
        let fault_at = attempts.get() + 1;
        collection
            .decode(&id, &ImageRequest::default())
            .unwrap()
            .with_pixels(|p| {
                assert_eq!(p.as_contiguous_bytes().unwrap(), refs[1].bytes);
            })
            .unwrap();
        assert!(
            attempts.get() >= fault_at,
            "decode must perform another range request"
        );
        let last_read = attempts.get();
        assert!(
            last_read > fault_at,
            "fixture must distinguish first and last decode read"
        );
        for (mode, at, kind) in [
            ("change", fault_at, FailureKind::Status(412)),
            ("change", last_read, FailureKind::Status(412)),
            ("changed_tag", last_read, FailureKind::Validator),
            ("wrong_total", last_read, FailureKind::ContentRange),
            ("short_body", last_read, FailureKind::Body),
        ] {
            let source = HttpSource::open(url(mode, &name, at), 65536).unwrap();
            let attempts = source.attempts.clone();
            let mut collection = TiffCollection::open(
                ReadAtReader::new(source),
                Composition::Composited,
                Allowances::default(),
            )
            .unwrap();
            let id = child(&mut collection);
            assert_eq!(attempts.get(), fault_at - 1);
            let error = collection
                .decode(&id, &ImageRequest::default())
                .unwrap_err();
            assert_cause(&error, kind);
            assert_eq!(attempts.get(), at);
            let error = collection
                .decode(&id, &ImageRequest::default())
                .unwrap_err();
            assert_cause(&error, kind);
            assert_eq!(attempts.get(), at, "failed source silently reopened");
        }
    }
}

#[test]
fn http_snapshot_malformed_responses_keep_typed_causes_through_native_open() {
    let (bytes, _) = nested_fixture(false, false, false);
    write_fixture("faults", &bytes);
    for (mode, kind) in [
        ("change", FailureKind::Status(412)),
        ("changed_tag", FailureKind::Validator),
        ("weak", FailureKind::Validator),
        ("missing_tag", FailureKind::Validator),
        ("bad_tag", FailureKind::Validator),
        ("duplicate_tag", FailureKind::Validator),
        ("wrong_range", FailureKind::ContentRange),
        ("wrong_total", FailureKind::ContentRange),
        ("wrong_length", FailureKind::ContentLength),
        ("missing_length", FailureKind::ContentLength),
        ("encoding", FailureKind::Encoding),
        ("full_body", FailureKind::Status(200)),
        ("redirect", FailureKind::Status(302)),
        ("short_body", FailureKind::Body),
    ] {
        let source = HttpSource::open(url(mode, "faults", 2), 65536).unwrap();
        let attempts = source.attempts.clone();
        let error = TiffCollection::open(
            ReadAtReader::new(source),
            Composition::Composited,
            Allowances::default(),
        )
        .err()
        .expect("malformed response opened native collection");
        assert_cause(&error, kind);
        assert_eq!(attempts.get(), 2, "unexpected HTTP retry for {mode}");
        if kind == FailureKind::Body {
            assert!(zencodec::find_cause::<std::io::Error>(&error).is_some());
        }
    }
}

#[test]
fn http_snapshot_initial_validator_and_range_limit_are_required() {
    let (bytes, _) = nested_fixture(false, false, false);
    write_fixture("admission", &bytes);
    for mode in ["weak", "missing_tag", "bad_tag", "duplicate_tag"] {
        let error = HttpSource::open(url(mode, "admission", 1), 65536)
            .err()
            .unwrap();
        assert_eq!(error.kind, FailureKind::Validator);
    }
    // An empty strong opaque tag is legal; it is not a weak/missing validator.
    let mut source = HttpSource::open(url("empty_tag", "admission", 1), 65536).unwrap();
    let mut out = [0; 4];
    source.read_exact_at(0, &mut out).unwrap();
    assert_eq!(out, bytes[..4]);

    let mut source = HttpSource::open(url("stable", "admission", 0), 1).unwrap();
    let error = source.read_exact_at(0, &mut [0; 2]).unwrap_err();
    assert_eq!(error.kind, FailureKind::RangeLimit);
    assert_eq!(
        source.attempts.get(),
        1,
        "oversized request reached network"
    );
    assert_eq!(source.len_hint(), None);
    assert_eq!(source.len().unwrap_err().kind, error.kind);
    assert_eq!(
        source.read_exact_at(0, &mut []).unwrap_err().kind,
        error.kind
    );
}
