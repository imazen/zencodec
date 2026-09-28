use crate::reference;

#[test]
fn scalar_references_match_independent_decimal_vectors() {
    let vectors = include_str!("../../../corpus/reference-vectors.csv");
    let mut count = 0;
    for line in vectors.lines().skip(1) {
        let cells: Vec<_> = line.split(',').collect();
        let x: f64 = cells[4].parse().unwrap();
        let expected: f64 = cells[7].parse().unwrap();
        let actual = match cells[0] {
            "ycbcr" => {
                let (kr, kb) = match cells[3] {
                    "1" => (0.2126, 0.0722),
                    "6" => (0.299, 0.114),
                    "9" => (0.2627, 0.0593),
                    value => panic!("matrix {value}"),
                };
                let rgb = reference::ycbcr_to_rgb(
                    [x, cells[5].parse().unwrap(), cells[6].parse().unwrap()],
                    cells[1].parse().unwrap(),
                    cells[2] == "1",
                    kr,
                    kb,
                );
                for (actual, expected) in rgb.into_iter().zip(&cells[7..10]) {
                    let expected: f64 = expected.parse().unwrap();
                    assert!((actual - expected).abs() < 2e-14, "{line}: {actual}");
                }
                rgb[0]
            }
            "srgb" => reference::srgb_to_linear(x),
            "pq_nits" => reference::pq_to_nits(x),
            "hlg_scene" => reference::hlg_to_scene(x),
            operation => panic!("reference {operation}"),
        };
        let tolerance = 1e-12 * expected.abs().max(1.0);
        assert!((actual - expected).abs() < tolerance, "{line}: {actual}");
        count += 1;
    }
    assert_eq!(count, 219, "oracle corpus must not silently shrink");
}
