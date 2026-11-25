//! Draft source-only controls for the paired zenpixels verifier. No native proof.
use zencodec::encode::{EncodeDeclaration, EncodePlan, EncodePlanError, OutputTiming};
use zencodec::helpers::codec_spec;
use zencodec::sample::{ExactDepth, ExactDepthError};
use zenpixels::{SampleType, SignalRange};

fn eight_in_sixteen() -> ExactDepth {
    ExactDepth::new(SampleType::U16, SignalRange::Full, 8, 16).unwrap()
}

#[test]
fn declaration_defaults_builder_debug_and_equality_include_the_claim() {
    let plain = EncodeDeclaration::new(2, 3, codec_spec(zenpixels::SampleLayout::RGB16));
    assert_eq!(plain.exact_depth, None);
    let claimed = plain.clone().with_exact_depth(eight_in_sixteen());
    assert_eq!(claimed.exact_depth, Some(eight_in_sixteen()));
    assert_ne!(plain, claimed);
    assert_eq!(claimed, plain.with_exact_depth(eight_in_sixteen()));
    assert!(format!("{claimed:?}").contains("exact_depth: Some"));
    assert!(claimed.validate_exact_depth().is_ok());
    assert!(
        EncodePlan::rows(OutputTiming::AT_FINISH)
            .validate(&claimed)
            .is_ok()
    );
}

#[test]
fn declaration_uses_paired_descriptor_rules_without_reading_samples() {
    let plan = EncodePlan::rows(OutputTiming::AT_FINISH);
    let full = eight_in_sixteen();
    let cases = [
        (
            codec_spec(zenpixels::SampleLayout::RGB8),
            full,
            ExactDepthError::SpecMismatch,
        ),
        (
            codec_spec(zenpixels::SampleLayout::RGB16).with_range(SignalRange::Narrow),
            full,
            ExactDepthError::SpecMismatch,
        ),
        (
            codec_spec(zenpixels::SampleLayout::CMYK8),
            ExactDepth::new(SampleType::U8, SignalRange::Full, 8, 8).unwrap(),
            ExactDepthError::UnsupportedLayout,
        ),
    ];
    for (descriptor, exact, expected) in cases {
        let declaration = EncodeDeclaration::new(2, 3, descriptor).with_exact_depth(exact);
        assert_eq!(declaration.validate_exact_depth(), Err(expected));
        assert_eq!(
            plan.validate(&declaration),
            Err(EncodePlanError::ExactDepth)
        );
    }
    // Unknown evidence is not a lower-depth/fidelity refusal.
    assert!(
        plan.validate(&EncodeDeclaration::new(
            2,
            3,
            codec_spec(zenpixels::SampleLayout::CMYK8)
        ))
        .is_ok()
    );
}

#[test]
fn structural_validation_is_deliberately_not_a_content_certificate() {
    let declaration = EncodeDeclaration::new(1, 1, codec_spec(zenpixels::SampleLayout::GRAY16))
        .with_exact_depth(eight_in_sixteen());
    assert!(declaration.validate_exact_depth().is_ok());
    // A legal assertion can still be false for actual samples.
    let mut samples =
        zenpixels::PixelBuf::new(1, 1, codec_spec(zenpixels::SampleLayout::GRAY16)).unwrap();
    samples
        .as_view_mut()
        .row_mut(0)
        .copy_from_slice(&1u16.to_ne_bytes());
    assert_eq!(eight_in_sixteen().holds_for(&samples.as_view()), Ok(false));
}

#[test]
fn independent_canonical_grids_classify_split_color_and_alpha() {
    use zenpixels::{ImageSpec, PixelBuf, SampleLayout, SampleType, SignalRange};
    let descriptor = |range| zencodec::helpers::codec_spec(SampleLayout::GRAYA16).with_range(range);
    let quantize = |code: u64, bits: u8| -> u16 {
        let steps = (1u64 << bits) - 1;
        ((code * 65535 + steps / 2) / steps) as u16
    };
    for bits in 1..=16u8 {
        let steps = (1u64 << bits) - 1;
        let midpoint = steps.div_ceil(2);
        let canonical = quantize(midpoint, bits);
        let pixels = PixelBuf::from_typed(
            vec![canonical, 9362u16],
            1,
            1,
            descriptor(SignalRange::Full),
        )
        .unwrap();
        let exact = ExactDepth::new(SampleType::U16, SignalRange::Full, bits, 3).unwrap();
        assert!(exact.holds_for(&pixels.as_view()).unwrap(), "depth {bits}");
        if bits < 15 {
            let off_grid = PixelBuf::from_typed(
                vec![canonical - 1, 9362u16],
                1,
                1,
                descriptor(SignalRange::Full),
            )
            .unwrap();
            assert!(
                !exact.holds_for(&off_grid.as_view()).unwrap(),
                "depth {bits}"
            );
        }
    }
    for bits in 8..=16u8 {
        let narrow = 161u16 << (16 - bits);
        let pixels =
            PixelBuf::from_typed(vec![narrow, 9362u16], 1, 1, descriptor(SignalRange::Narrow))
                .unwrap();
        let exact = ExactDepth::new(SampleType::U16, SignalRange::Narrow, bits, 3).unwrap();
        assert!(
            exact.holds_for(&pixels.as_view()).unwrap(),
            "narrow depth {bits}"
        );
        if bits < 16 {
            let other = PixelBuf::from_typed(
                vec![narrow - 1, 9362u16],
                1,
                1,
                descriptor(SignalRange::Narrow),
            )
            .unwrap();
            assert!(!exact.holds_for(&other.as_view()).unwrap());
        }
    }
    for value in 0..=u8::MAX {
        let spec = ImageSpec::GRAY8_SRGB;
        let pixel = PixelBuf::from_typed(vec![value], 1, 1, spec).unwrap();
        for bits in 1..=8 {
            let steps = (1u32 << bits) - 1;
            let oracle =
                (0..=steps).any(|index| ((index * 255 + steps / 2) / steps) as u8 == value);
            let exact = ExactDepth::new(SampleType::U8, SignalRange::Full, bits, 8).unwrap();
            assert_eq!(
                exact.holds_for(&pixel.as_view()).unwrap(),
                oracle,
                "{value} at {bits}"
            );
        }
    }
}
