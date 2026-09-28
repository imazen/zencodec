use serde_json::Value;
use zencodec_media::display::{DisplayConversion, DisplayError, DisplayTransfer, OutOfRange};
use zenpixels::{Cicp, ColorPrimaries};

fn number(v: &Value) -> f32 {
    v.as_str().unwrap().parse().unwrap()
}
fn triple(v: &Value) -> [f32; 3] {
    std::array::from_fn(|i| number(&v[i]))
}

#[test]
fn display_transfers_match_independent_decimal_luminance_and_colored_hlg() {
    let corpus: Value =
        serde_json::from_str(include_str!("../corpus/display-references.json")).unwrap();
    let mut cases = 0;
    for case in corpus["display"].as_array().unwrap() {
        let peak = number(&case["peak"]);
        let black = number(&case["black"]);
        let gamma = number(&case["gamma"]);
        let curve = case["curve"].as_str().unwrap();
        let transfer = match curve {
            "srgb" => DisplayTransfer::srgb(peak).unwrap(),
            "pq" => DisplayTransfer::pq(),
            "hlg" => DisplayTransfer::hlg(peak, black, gamma).unwrap(),
            "bt1886" => DisplayTransfer::bt1886(peak, black).unwrap(),
            "linear" => DisplayTransfer::linear(peak).unwrap(),
            _ => panic!("unknown reference operation"),
        };
        let input = triple(&case["signal"]);
        let expected = triple(&case["nits"]);
        let actual = transfer.decode(input, OutOfRange::Reject).unwrap();
        for i in 0..3 {
            // Public production kernels are f32 approximations. Budget is in
            // physical light: 2e-6 of encoding peak + 2e-5 of the component.
            let budget = 2e-6 * peak + 2e-5 * expected[i];
            assert!(
                (actual[i] - expected[i]).abs() <= budget,
                "{curve} {input:?} ch{i}: {} != {} (budget {budget})",
                actual[i],
                expected[i]
            );
        }
        // Inverse is fed the independent Decimal light, not production output.
        let encoded = transfer.encode(expected, OutOfRange::Clamp).unwrap();
        for i in 0..3 {
            assert!(
                (encoded[i] - input[i]).abs() < 1e-5,
                "{curve} inverse {input:?}: {encoded:?}"
            );
        }
        cases += 1;
    }
    assert_eq!(cases, 720);
}

#[test]
fn primary_matrices_match_independent_chromaticity_solution_without_clipping() {
    let corpus: Value =
        serde_json::from_str(include_str!("../corpus/display-references.json")).unwrap();
    let mut negative = false;
    let mut over_one = false;
    for case in corpus["primaries"].as_array().unwrap() {
        let from = ColorPrimaries::from_cicp(case["source"].as_u64().unwrap() as u8).unwrap();
        let to = ColorPrimaries::from_cicp(case["target"].as_u64().unwrap() as u8).unwrap();
        let matrix = from.gamut_matrix_to(to).unwrap();
        let input = triple(&case["rgb"]);
        let expected = triple(&case["converted"]);
        let actual = matrix.map(|row| row.iter().zip(input).map(|(a, b)| a * b).sum::<f32>());
        for i in 0..3 {
            negative |= expected[i] < -0.01;
            over_one |= expected[i] > 1.01;
            assert!(
                (actual[i] - expected[i]).abs() < 1e-6,
                "{from:?}->{to:?}, {input:?}: {actual:?} != {expected:?}"
            );
        }
    }
    assert!(negative && over_one);
    assert_eq!(corpus["primaries"].as_array().unwrap().len(), 576);
}

#[test]
fn explicit_display_pipeline_preserves_absolute_light_or_rejects_unrepresentable_output() {
    let pq = Cicp::new(9, 16, 0, true);
    let hlg = Cicp::new(9, 18, 0, true);
    let plan = DisplayConversion::new(
        hlg,
        DisplayTransfer::hlg(1000.0, 0.0, 1.2).unwrap(),
        pq,
        DisplayTransfer::pq(),
        OutOfRange::Reject,
    )
    .unwrap();
    let signal = [0.75, 0.5, 0.25];
    let expected_light = DisplayTransfer::hlg(1000.0, 0.0, 1.2)
        .unwrap()
        .decode(signal, OutOfRange::Reject)
        .unwrap();
    let converted = plan.convert(signal).unwrap();
    let actual_light = DisplayTransfer::pq()
        .decode(converted, OutOfRange::Reject)
        .unwrap();
    for i in 0..3 {
        assert!((actual_light[i] - expected_light[i]).abs() < 0.02);
    }
    assert_eq!(plan.output_color(), pq);

    let sdr = DisplayConversion::new(
        pq,
        DisplayTransfer::pq(),
        Cicp::SRGB,
        DisplayTransfer::srgb(100.0).unwrap(),
        OutOfRange::Reject,
    )
    .unwrap();
    let mut destination = [[123.0; 3]; 2];
    assert_eq!(
        sdr.write_row(&[[0.1; 3], [1.0; 3]], &mut destination),
        Err(DisplayError::OutOfRange)
    );
    assert_eq!(destination, [[123.0; 3]; 2]);
    assert_eq!(
        sdr.write_row(&[[0.1; 3]], &mut destination),
        Err(DisplayError::OutputWidth)
    );
    assert_eq!(destination, [[123.0; 3]; 2]);
    assert_eq!(
        sdr.convert([f32::NAN, 0.0, 0.0]),
        Err(DisplayError::NonFinite)
    );
    assert_eq!(
        plan.convert([-0.01, 0.0, 0.0]),
        Err(DisplayError::OutOfRange)
    );
}

#[test]
fn mismatched_and_unspecified_interpretations_are_errors() {
    for value in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
        assert!(DisplayTransfer::srgb(value).is_err());
        assert!(DisplayTransfer::hlg(value, 0.0, 1.2).is_err());
        assert!(DisplayTransfer::hlg(1000.0, 0.0, value).is_err());
        assert!(DisplayTransfer::bt1886(value, 0.0).is_err());
        assert!(DisplayTransfer::linear(value).is_err());
    }
    for black in [-1.0, f32::NAN, 1000.0, 2000.0] {
        assert!(DisplayTransfer::hlg(1000.0, black, 1.2).is_err());
        assert!(DisplayTransfer::bt1886(1000.0, black).is_err());
    }
    let pq = DisplayTransfer::pq();
    for color in [
        Cicp::new(9, 16, 9, true),
        Cicp::new(9, 16, 0, false),
        Cicp::new(9, 2, 0, true),
    ] {
        assert!(matches!(
            DisplayConversion::new(color, pq, Cicp::new(9, 16, 0, true), pq, OutOfRange::Reject),
            Err(DisplayError::ColorMismatch)
        ));
    }
    assert!(matches!(
        DisplayConversion::new(
            Cicp::new(2, 16, 0, true),
            pq,
            Cicp::new(9, 16, 0, true),
            pq,
            OutOfRange::Reject
        ),
        Err(DisplayError::UnknownPrimaries)
    ));
    assert!(matches!(
        DisplayConversion::new(
            Cicp::new(1, 18, 0, true),
            DisplayTransfer::hlg(1000.0, 0.0, 1.2).unwrap(),
            Cicp::new(9, 16, 0, true),
            pq,
            OutOfRange::Reject
        ),
        Err(DisplayError::HlgPrimaries)
    ));
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            pq.decode([v; 3], OutOfRange::Clamp),
            Err(DisplayError::NonFinite)
        );
        assert_eq!(
            pq.encode([v; 3], OutOfRange::Clamp),
            Err(DisplayError::NonFinite)
        );
    }
}

#[test]
fn strict_display_roundtrips_accept_domain_endpoints() {
    for (name, transfer) in [
        ("sRGB", DisplayTransfer::srgb(203.0).unwrap()),
        ("PQ", DisplayTransfer::pq()),
        (
            "HLG black zero",
            DisplayTransfer::hlg(1000.0, 0.0, 1.2).unwrap(),
        ),
        (
            "HLG lifted black",
            DisplayTransfer::hlg(1000.0, 0.005, 1.2).unwrap(),
        ),
        (
            "BT1886 black zero",
            DisplayTransfer::bt1886(100.0, 0.0).unwrap(),
        ),
        (
            "BT1886 lifted black",
            DisplayTransfer::bt1886(100.0, 0.1).unwrap(),
        ),
        ("linear", DisplayTransfer::linear(203.0).unwrap()),
    ] {
        for signal in [[0.0; 3], [1.0; 3], [0.0, 0.5, 1.0], [1.0, 0.0, 0.5]] {
            let light = transfer.decode(signal, OutOfRange::Reject).unwrap();
            let encoded = transfer
                .encode(light, OutOfRange::Reject)
                .unwrap_or_else(|e| panic!("{name} {signal:?}: {light:?}: {e}"));
            for (actual, expected) in encoded.into_iter().zip(signal) {
                assert!(
                    (actual - expected).abs() < 1e-5,
                    "{name}: {encoded:?} vs {signal:?}"
                );
            }
        }
    }
}
