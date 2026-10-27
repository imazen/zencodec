use super::*;
use crate::http_source::{Failure, FailureKind, HttpSource};

#[test]
fn http_snapshot_collection_limits_keep_inspection_and_smaller_children_available() {
    use crate::collection_factories::TiffFactory;
    use zencodec::{CodecSet, ImageFormat, LimitExceeded, ResourceLimits, StopToken, Unstoppable,
        collection::OpenRequest, operation::PolicySet, source::ReadAtShim};
    let (bytes, refs) = nested_fixture(true, true, false);
    let name = "collection-limits";
    write_fixture(name, &bytes);
    let mut source = ReadAtShim(HttpSource::open(url("stable", name, 0), 65536).unwrap());
    let codecs = CodecSet::new().with_collection(TiffFactory(Allowances::default()))
        .with_limits(ResourceLimits::none().with_max_width(refs[1].width));
    let mut images = codecs.open_collection(
        ImageFormat::Tiff, &mut source, OpenRequest::new(Composition::Composited, 32),
        PolicySet::new(), StopToken::new(Unstoppable),
    ).unwrap();
    let mut roots = images.images().unwrap();
    let root = images.next(&mut roots).unwrap().unwrap();
    assert_eq!(images.describe(&root).unwrap().stored_size, (refs[0].width, refs[0].height));
    let error = images.decode(&root, &ImageRequest::new()).unwrap_err();
    assert_eq!(zencodec::find_cause::<LimitExceeded>(&error), Some(&LimitExceeded::Width {
        actual: refs[0].width, max: refs[1].width,
    }));
    assert_eq!(images.report().decisions().count(), 0);
    assert!(!images.is_finished());
    let mut children = images.children(&root).unwrap();
    let child = images.next(&mut children).unwrap().unwrap();
    let output = images.decode(&child, &ImageRequest::new()).unwrap();
    assert_eq!(output.with_pixels(|p| p.as_contiguous_bytes().unwrap().to_vec()).unwrap(), refs[1].bytes);
    assert_eq!(images.report().decisions().count(), 1);
}

#[test]
fn http_snapshot_collection_factory_preserves_positional_selection() {
    use crate::collection_factories::TiffFactory;
    use zencodec::{CodecSet, collection::OpenRequest};
    use zencodec::{ImageFormat, StopToken, Unstoppable, operation::PolicySet, source::ReadAtShim};
    let (bytes, refs) = nested_fixture(true, true, false);
    let name = "collection-factory";
    write_fixture(name, &bytes);
    let mut final_read = 0;
    for mode in ["stable", "change", "changed_tag", "short_body"] {
        let source = HttpSource::open(url(mode, name, final_read), 65536).unwrap();
        let attempts = source.attempts.clone();
        let mut source = ReadAtShim(source);
        let factories = CodecSet::new().with_collection(TiffFactory(Allowances::default()));
        let mut images = factories.open_collection(
            ImageFormat::Tiff, &mut source,
            OpenRequest::new(Composition::Composited, 32),
            PolicySet::new(), StopToken::new(Unstoppable),
        ).unwrap();
        let mut roots = images.images().unwrap();
        let root = images.next(&mut roots).unwrap().unwrap();
        let description = images.describe(&root).unwrap();
        assert_eq!(description.stored_size, (refs[0].width, refs[0].height));
        let output = images.decode(&root, &ImageRequest::new());
        if mode == "stable" {
            final_read = attempts.get();
            assert_eq!(output.unwrap().with_pixels(|p| p.as_strided_bytes().to_vec()).unwrap(), refs[0].bytes);
        } else {
            let expected = match mode {
                "change" => FailureKind::Status(412),
                "changed_tag" => FailureKind::Validator,
                _ => FailureKind::Body,
            };
            let error = output.err().unwrap();
            assert_cause(&error, expected);
            assert_eq!(error.error().stop_reason(), None);
            let before = attempts.get();
            assert_cause(&images.decode(&root, &ImageRequest::new()).err().unwrap(), expected);
            assert_eq!(attempts.get(), before, "terminal source retried transport I/O");
            assert_eq!(images.report().decisions().count(), 0, "failed native output reached publication policy");
        }
    }
}

#[test]
fn http_snapshot_render_first_matches_slice_and_rejects_late_source_faults() {
    use crate::collection_factories::TiffFactory;
    use zencodec::{CodecSet, ImageFormat, StopToken, Unstoppable,
        collection::OpenRequest, operation::PolicySet,
        source::{ReadAtShim, SliceReadAt}, storage::StorageBudget};
    use zencodec_render::Renderer;
    use zenpixels_convert::cms_moxcms::MoxCms;

    let bytes = crate::canvas_render::tests::native_tiff_bytes(6, true, 8);
    let name = "render-first";
    write_fixture(name, &bytes);
    let codecs = CodecSet::new().with_collection(TiffFactory(Allowances::default()));
    let budget = StorageBudget::new(1024 * 1024, 32);
    let renderer = Renderer::new(budget.clone()).with_cms(&MoxCms);
    // The returned owner must survive both the collection and its source.
    let reference = {
        let mut source = ReadAtShim(SliceReadAt::new(&bytes));
        let mut images = codecs.open_collection(
            ImageFormat::Tiff, &mut source, OpenRequest::new(Composition::Composited, 32),
            PolicySet::new(), StopToken::new(Unstoppable),
        ).unwrap();
        let image = renderer.render_first(&mut images).unwrap().unwrap();
        assert_eq!(images.report().decisions().count(), 1);
        image
    };
    let expected = reference.with_pixels(|p| {
        assert_eq!((p.width(), p.rows()), (5, 7));
        assert_eq!(p.descriptor(), zenpixels::PixelDescriptor::RGBA8_SRGB);
        p.as_strided_bytes().to_vec()
    }).unwrap();
    drop(reference);
    assert_eq!(budget.reserved_bytes(), 0);
    let mut final_read = 0;
    for mode in ["stable", "change", "changed_tag", "short_body"] {
        let source = HttpSource::open(url(mode, name, final_read), 65536).unwrap();
        let attempts = source.attempts.clone();
        let mut source = ReadAtShim(source);
        let mut images = codecs.open_collection(
            ImageFormat::Tiff, &mut source, OpenRequest::new(Composition::Composited, 32),
            PolicySet::new(), StopToken::new(Unstoppable),
        ).unwrap();
        let output = renderer.render_first(&mut images);
        if mode == "stable" {
            final_read = attempts.get();
            let output = output.unwrap().unwrap();
            assert_eq!(images.report().decisions().count(), 1);
            drop(images);
            drop(source);
            assert_eq!(output.with_pixels(|p| p.as_strided_bytes().to_vec()).unwrap(), expected);
            drop(output);
        } else {
            let kind = match mode {
                "change" => FailureKind::Status(412),
                "changed_tag" => FailureKind::Validator,
                _ => FailureKind::Body,
            };
            let error = output.expect_err("fault must not become an empty collection or an image");
            assert_cause(&error, kind);
            assert_eq!(error.error().stop_reason(), None);
            assert_eq!(images.report().decisions().count(), 0);
            let before = attempts.get();
            assert_cause(&renderer.render_first(&mut images).err().unwrap(), kind);
            assert_eq!(attempts.get(), before, "terminal source retried transport I/O");
        }
        assert_eq!(budget.reserved_bytes(), 0);
        assert_eq!(budget.reserved_objects(), 0);
    }
}

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
fn http_snapshot_description_and_later_decode_keep_the_source_revision() {
        for big in [false, true] {
        let (bytes, refs) = nested_fixture(big, true, false);
        let name = format!("selected-description-{big}");
        write_fixture(&name, &bytes);
        let mut final_description_read = 0;
        for (mode, expected) in [
            ("stable", None),
            ("change", Some(FailureKind::Status(412))),
            ("changed_tag", Some(FailureKind::Validator)),
            ("short_body", Some(FailureKind::Body)),
        ] {
            let source = HttpSource::open(url(mode, &name, final_description_read), 65536).unwrap();
            let attempts = source.attempts.clone();
            let mut collection = TiffCollection::open(ReadAtReader::new(source), Composition::Composited, Allowances::default()).unwrap();
            let id = child(&mut collection);
            let discovered = attempts.get();
            match expected {
                None => {
                    let description = collection.describe(&id).unwrap();
                    assert_eq!(description.stored_size, (refs[1].width, refs[1].height));
                    final_description_read = attempts.get();
                    assert!(final_description_read > discovered);
                }
                Some(kind) => {
                    let error = collection.describe(&id).expect_err("partial or replaced description escaped");
                    assert_cause(&error, kind);
                    assert_eq!(attempts.get(), final_description_read);
                    let error = collection.describe(&id).expect_err("failed source reopened");
                    assert_cause(&error, kind);
                    assert_eq!(attempts.get(), final_description_read);
                }
            }
        }
        // A successful description is a fact about the retained representation,
        // not permission to decode a different revision under the same ID.
        let source = HttpSource::open(url("change", &name, final_description_read + 1), 65536).unwrap();
        let attempts = source.attempts.clone();
        let mut collection = TiffCollection::open(ReadAtReader::new(source), Composition::Composited, Allowances::default()).unwrap();
        let id = child(&mut collection);
        collection.describe(&id).unwrap();
        assert_eq!(attempts.get(), final_description_read);
        let error = collection.decode(&id, &ImageRequest::new()).expect_err("new revision decoded after old description");
        assert_cause(&error, FailureKind::Status(412));
        assert_eq!(attempts.get(), final_description_read + 1);
    }
}

#[test]
fn http_snapshot_checked_collection_publishes_only_after_last_range_and_policy() {
    use crate::checked_collection::{Checked, Stage};
    use std::{cell::Cell, convert::Infallible};
    use zencodec::{
        At, StopToken, Unstoppable,
        operation::{Decision, Question},
    };
    for big in [false, true] {
        let (bytes, refs) = nested_fixture(big, true, false);
        let name = format!("checked-publication-{big}");
        write_fixture(&name, &bytes);
        let mut last_read = 0;
        for (mode, expected_error) in [
            ("stable", None),
            ("change", Some(FailureKind::Status(412))),
            ("changed_tag", Some(FailureKind::Validator)),
            ("short_body", Some(FailureKind::Body)),
        ] {
            let seen = Cell::new(0);
            let resolver = |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Image { image, .. } = question else {
                    panic!("wrong domain")
                };
                image
                    .with_pixels(|p| assert_eq!(p.as_contiguous_bytes().unwrap(), refs[1].bytes))
                    .unwrap();
                seen.set(seen.get() + 1);
                Ok(Decision::UsePreset)
            };
            let mut attempts = None;
            let mut collection = Checked::open(StopToken::new(Unstoppable), resolver, |stop| {
                let source = HttpSource::open(url(mode, &name, last_read), 65536)?;
                attempts = Some(source.attempts.clone());
                TiffCollection::open_with_stop(
                    ReadAtReader::new(source),
                    Composition::Composited,
                    Allowances::default(),
                    stop,
                )
                .map_err(Into::into)
            })
            .unwrap();
            let attempts = attempts.unwrap();
            let mut roots = collection.images().unwrap();
            let parent = collection.next(&mut roots).unwrap().unwrap();
            let mut children = collection.children(&parent).unwrap();
            let id = collection.next(&mut children).unwrap().unwrap();
            let discovered = attempts.get();
            if let Some(kind) = expected_error {
                assert!(last_read > discovered);
                let error = collection
                    .decode(&id, &ImageRequest::default())
                    .unwrap_err();
                assert_eq!(error.error().stage, Stage::Decode);
                assert_cause(&error, kind);
                assert_eq!(attempts.get(), last_read);
                assert_eq!(seen.get(), 0, "policy saw a partial or replaced image");
                assert_eq!(error.error().report.decisions().count(), 0);
                let retry = collection
                    .decode(&id, &ImageRequest::default())
                    .unwrap_err();
                assert_cause(&retry, kind);
                assert_eq!(attempts.get(), last_read, "retry reopened a failed source");
                assert_eq!(seen.get(), 0);
            } else {
                let image = collection.decode(&id, &ImageRequest::default()).unwrap();
                last_read = attempts.get();
                assert!(last_read > discovered);
                assert_eq!(seen.get(), 1);
                assert_eq!(image.report().decisions().count(), 1);
                drop(collection);
                image
                    .with_pixels(|p| assert_eq!(p.as_contiguous_bytes().unwrap(), refs[1].bytes))
                    .unwrap();
            }
        }
    }
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
