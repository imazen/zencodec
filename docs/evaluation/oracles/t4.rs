#[test]
fn preserves_scan_data_and_refuses_opaque_icc_before_writing() {
    for progressive in [false, true] {
        let clean = fixtures::base(progressive);
        let mut output = Vec::new();
        candidate::rewrite_jpeg(&clean, &mut output).unwrap();
        assert_eq!(fixtures::scans(&clean), fixtures::scans(&output));
        assert_eq!(fixtures::oracle(&clean), fixtures::oracle(&output));
        for placement in [fixtures::Placement::Header, fixtures::Placement::BeforeEoi] {
            let source = fixtures::fixture(progressive, placement);
            let mut output = Vec::new();
            assert!(candidate::rewrite_jpeg(&source, &mut output).is_err());
            assert!(output.is_empty());
        }
        let descriptive = [&clean[..2], fixtures::descriptive().as_slice(), &clean[2..]].concat();
        let mut output = Vec::new();
        candidate::rewrite_jpeg(&descriptive, &mut output).unwrap();
        assert!(
            !output
                .windows(fixtures::PRIVATE.len())
                .any(|v| v == fixtures::PRIVATE)
        );
        assert_eq!(fixtures::scans(&descriptive), fixtures::scans(&output));
    }
}
