//! Qualify the scope of native limits before mapping shared requirements to them.
use super::{
    collection::Collection,
    image_items::{Composition, ImageRequest},
    jxl_items::{Limits, Plane, decode_next_details, pixel_decoder},
    jxl_layers::JxlLayers,
    jxl_tests::fixture,
};
use std::{fs::OpenOptions, path::PathBuf};
use zencodec::{StopToken, Unstoppable};
use zenjxl_decoder::api::{Error, JxlColorType, JxlDecoderOptions};

fn options() -> JxlDecoderOptions {
    JxlDecoderOptions::default()
        .with_adjust_orientation(false)
        .with_coalescing(false)
        .with_render_spot_colors(false)
        .with_dither_u8(false)
}

fn evidence(name: &str, value: serde_json::Value) {
    let root = PathBuf::from(
        std::env::var_os("SPATIAL_LAYER_ARTIFACTS").expect("set SPATIAL_LAYER_ARTIFACTS"),
    );
    std::fs::create_dir_all(&root).unwrap();
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .expect("fresh admission evidence file");
    serde_json::to_writer_pretty(output, &value).unwrap();
}

#[test]
fn native_jxl_admission_pixel_cap_includes_canvas_before_small_layer_selection() {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let mut layers = JxlLayers::open(
        &bytes,
        Limits {
            images: 64,
            frame_headers: Some(64),
        },
        StopToken::new(Unstoppable),
    )
    .unwrap();
    assert_eq!(layers.view(), Composition::Uncomposited);
    let mut cursor = layers.images().unwrap();
    let mut ids = Vec::new();
    while let Some(id) = layers.next(&mut cursor).unwrap() {
        ids.push(id);
    }
    let selected = layers.describe(ids.last().unwrap()).unwrap();
    assert_eq!(selected.stored_size, (110, 68));
    let output = layers
        .decode(ids.last().unwrap(), &ImageRequest::new())
        .unwrap();
    output
        .with_pixels(|pixels| {
            assert_eq!((pixels.width(), pixels.rows()), selected.stored_size);
        })
        .unwrap();

    let selected_pixels = u64::from(selected.stored_size.0) * u64::from(selected.stored_size.1);
    let mut limited = options();
    limited.limits.max_pixels = Some(usize::try_from(selected_pixels).unwrap());
    // The same cap is sufficient for the selected local layer but the native
    // option rejects the larger canvas, before a layer can be selected at all.
    let error = pixel_decoder(&bytes, limited)
        .err()
        .expect("canvas was admitted");
    let native = zencodec::find_cause::<Error>(error.as_ref()).unwrap();
    let (actual, limit) = match native {
        Error::LimitExceeded {
            resource: "pixels",
            actual,
            limit,
        } => (*actual, *limit),
        other => panic!("unexpected native refusal: {other:?}"),
    };
    assert_eq!(actual, 512 * 512);
    assert_eq!(limit, selected_pixels);
    let mut admitted = options();
    admitted.limits.max_pixels = Some(usize::try_from(actual).unwrap());
    assert!(pixel_decoder(&bytes, admitted).is_ok());
    evidence(
        "native-pixel-scope.json",
        serde_json::json!({
            "fixture": "conformance_test_images/cmyk_layers.jxl",
            "selected_size": selected.stored_size,
            "selected_pixels": selected_pixels,
            "native_refusal_actual_pixels": actual,
            "native_refusal_limit_pixels": limit,
            "native_scope": "canvas, preview and internal upsampled frame extents; not just selected output",
        }),
    );
}

fn first_cmyk_planes(bytes: &[u8], budget: u64) -> Result<Vec<Plane>, zencodec::collection::Cause> {
    let mut options = options();
    options.limits.max_memory_bytes = Some(budget);
    let (mut decoder, mut input) = pixel_decoder(bytes, options)?;
    let mut format = decoder.current_pixel_format().clone();
    format.color_type = JxlColorType::Cmyk;
    decoder.set_pixel_format(format);
    // These are the same caller-owned, fallibly allocated, strided output
    // planes used by the qualified native adapter. Return them after decoder drop.
    let decoded = decode_next_details(decoder, &mut input, 16)?;
    Ok(decoded.planes)
}

#[test]
fn native_jxl_admission_memory_budget_does_not_charge_retained_output_owners() {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let mut attempts = Vec::new();
    let mut admitted = None;
    for budget in [0, 1 << 20, 4 << 20, 16 << 20, 64 << 20] {
        match first_cmyk_planes(&bytes, budget) {
            Ok(planes) => {
                attempts.push(serde_json::json!({"budget": budget, "accepted": true}));
                admitted = Some((budget, planes));
                break;
            }
            Err(error) => {
                let native = zencodec::find_cause::<Error>(error.as_ref()).unwrap();
                let actual = match native {
                    Error::LimitExceeded {
                        resource: "memory_bytes",
                        actual,
                        limit,
                    } => {
                        assert_eq!(*limit, budget);
                        assert!(*actual > budget);
                        *actual
                    }
                    other => panic!("unexpected native refusal: {other:?}"),
                };
                attempts.push(
                    serde_json::json!({"budget": budget, "accepted": false, "actual": actual}),
                );
            }
        }
    }
    let (budget, first) = admitted.expect("fixture must decode within the explicit test ceiling");
    assert!(budget > 0, "negative native memory control did not refuse");
    let capacity = |planes: &[Plane]| -> u64 {
        planes
            .iter()
            .map(|p| u64::try_from(p.bytes.capacity()).unwrap())
            .sum()
    };
    let mut retained_capacity = capacity(&first);
    let mut retained = vec![first];
    while retained_capacity <= budget {
        assert!(retained.len() < 64, "bounded repetition ceiling reached");
        let planes = first_cmyk_planes(&bytes, budget).unwrap();
        assert_eq!(
            planes, retained[0],
            "native samples changed with retained owners"
        );
        retained_capacity += capacity(&planes);
        retained.push(planes);
    }
    // This is measured Vec capacity, not RSS, a peak estimate, or all allocations.
    // Each native decoder succeeded while earlier output owners remained alive.
    assert!(retained_capacity > budget);
    evidence(
        "native-memory-scope.json",
        serde_json::json!({
            "fixture": "conformance_test_images/cmyk_layers.jxl",
            "attempts": attempts,
            "per_decoder_budget": budget,
            "retained_decodes": retained.len(),
            "retained_output_vec_capacity_bytes": retained_capacity,
            "native_samples": "bit-exact across repeated independently budgeted decodes",
            "measurement_scope": "actual output Vec capacities only; not RSS or total native workspace",
        }),
    );
}
