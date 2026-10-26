use crate::{
    collection::Collection,
    control::CheckStop,
    description::{Describe, Description},
    image_items::{Composition, ImageRequest, SelectionError},
    jxl_items::{JxlItems, Limits},
    jxl_layers::JxlLayers,
    jxl_tests::fixture,
};
use zencodec::{StopToken, Unstoppable};

fn limits(images: usize) -> Limits {
    Limits {
        images,
        frame_headers: Some(64),
    }
}

fn compare<C: Describe>(mut collection: C, orientation: Option<u8>) -> usize {
    let mut cursor = collection.images().unwrap();
    let mut descriptions = Vec::new();
    while let Some(id) = collection.next(&mut cursor).unwrap() {
        let description = collection.describe(&id).unwrap();
        if let Some(expected) = orientation {
            assert_eq!(
                description.stored_orientation,
                zenpixels::Orientation::from_exif(expected)
            );
        }
        descriptions.push((id, description));
    }
    assert!(!descriptions.is_empty());
    // Discovery destroys the native scanner at EOF. Descriptions must remain
    // available without starting a pixel decoder or changing the cursor.
    for (id, description) in descriptions.iter().rev() {
        assert_eq!(collection.describe(id).unwrap(), *description);
        let output = collection.decode(id, &ImageRequest::new()).unwrap();
        let interpretation = output.interpretation();
        assert_eq!(
            description.stored_size,
            (
                interpretation.geometry.canvas_width,
                interpretation.geometry.canvas_height
            )
        );
        assert_eq!(description.stored_orientation, interpretation.orientation);
        assert_eq!(description.placement, interpretation.placement);
        assert_eq!(collection.describe(id).unwrap(), *description);
    }
    assert!(collection.next(&mut cursor).unwrap().is_none());
    descriptions.len()
}

#[test]
fn description_jxl_presentations_and_raw_layers_survive_eof_and_match_reconstruction() {
    for (name, raw_count) in [
        ("multiple_layers_noise_spline.jxl", 5),
        ("conformance_test_images/cmyk_layers.jxl", 4),
    ] {
        let bytes = fixture(name);
        compare(
            JxlItems::open(&bytes, Composition::Composited, limits(64)).unwrap(),
            None,
        );
        assert_eq!(
            compare(
                JxlLayers::open(&bytes, limits(64), StopToken::new(Unstoppable)).unwrap(),
                None
            ),
            raw_count
        );
    }
}

#[test]
fn description_jxl_preserves_all_eight_stored_orientations_in_both_views() {
    for (index, name) in [
        "orientation1_identity.jxl",
        "orientation2_flip_horizontal.jxl",
        "orientation3_rotate_180.jxl",
        "orientation4_flip_vertical.jxl",
        "orientation5_transpose.jxl",
        "orientation6_rotate_90_cw.jxl",
        "orientation7_anti_transpose.jxl",
        "orientation8_rotate_90_ccw.jxl",
    ]
    .into_iter()
    .enumerate()
    {
        let bytes = fixture(name);
        let orientation = Some(u8::try_from(index + 1).unwrap());
        assert_eq!(
            compare(
                JxlItems::open(&bytes, Composition::Composited, limits(64)).unwrap(),
                orientation
            ),
            1
        );
        assert_eq!(
            compare(
                JxlLayers::open(&bytes, limits(64), StopToken::new(Unstoppable)).unwrap(),
                orientation
            ),
            1
        );
    }
}

fn cached_stop<C: Describe>(factory: impl Fn(StopToken) -> C) {
    let describe =
        |stop: &CheckStop,
         at: Option<usize>|
         -> Result<(Description, usize), zencodec::At<crate::collection::CollectionError>> {
            let mut collection = factory(stop.token());
            let mut cursor = collection.images()?;
            let first = collection.next(&mut cursor)?.unwrap();
            while collection.next(&mut cursor)?.is_some() {}
            let before = stop.calls();
            if let Some(at) = at {
                stop.after(at);
            }
            let result = collection.describe(&first)?;
            Ok((result, stop.calls() - before))
        };
    let (_, checks) = describe(&CheckStop::default(), None).unwrap();
    assert!(checks >= 2);
    for reason in [
        zencodec::enough::StopReason::Cancelled,
        zencodec::enough::StopReason::TimedOut,
    ] {
        for at in 0..checks {
            let error = describe(&CheckStop::with_reason(reason), Some(at))
                .expect_err("cached description ignored stop");
            assert_eq!(
                zencodec::find_cause::<zencodec::CodecError>(&error)
                    .unwrap()
                    .category(),
                zencodec::ErrorCategory::Stopped(reason)
            );
        }
    }
}

fn failed_scan<C: Describe>(mut collection: C) {
    let mut cursor = collection.images().unwrap();
    let first = collection.next(&mut cursor).unwrap().unwrap();
    collection.describe(&first).unwrap();
    let failure = collection
        .next(&mut cursor)
        .expect_err("discovery limit ignored");
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&failure),
        Some(&SelectionError::DiscoveryLimit)
    );
    let failure = collection
        .describe(&first)
        .expect_err("cached description escaped failed scan");
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&failure),
        Some(&SelectionError::DiscoveryLimit)
    );
}

#[test]
fn description_jxl_cached_returns_honor_stop_and_latched_native_failure() {
    let bytes = fixture("multiple_layers_noise_spline.jxl");
    cached_stop(|stop| {
        JxlItems::open_with_stop(&bytes, Composition::Composited, limits(64), stop).unwrap()
    });
    cached_stop(|stop| JxlLayers::open(&bytes, limits(64), stop).unwrap());
    // This file has five raw layers but only one visible presentation. Use an
    // animation to exercise failure after a retained presentation identity.
    let animation = fixture("conformance_test_images/animation_newtons_cradle.jxl");
    failed_scan(JxlItems::open(&animation, Composition::Composited, limits(1)).unwrap());
    failed_scan(JxlLayers::open(&bytes, limits(1), StopToken::new(Unstoppable)).unwrap());
}

#[test]
fn description_jxl_ids_do_not_cross_operations_or_composition_views() {
    let bytes = fixture("multiple_layers_noise_spline.jxl");
    let mut first = JxlItems::open(&bytes, Composition::Composited, limits(64)).unwrap();
    let mut cursor = first.images().unwrap();
    let id = Collection::next(&mut first, &mut cursor).unwrap().unwrap();
    let mut second = JxlItems::open(&bytes, Composition::Composited, limits(64)).unwrap();
    let mut layers = JxlLayers::open(&bytes, limits(64), StopToken::new(Unstoppable)).unwrap();
    for error in [
        second.describe(&id).unwrap_err(),
        layers.describe(&id).unwrap_err(),
    ] {
        assert_eq!(
            zencodec::find_cause::<SelectionError>(&error),
            Some(&SelectionError::ForeignImage)
        );
    }
}
