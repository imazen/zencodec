//! Draft source-only controls for the paired zenpixels verifier. No native proof.
use zencodec::encode::{EncodeDeclaration, EncodePlan, EncodePlanError, OutputTiming};
use zenpixels::{
    ChannelType, PixelDescriptor, SignalRange,
    sample::{ExactDepth, ExactDepthError},
};

fn eight_in_sixteen() -> ExactDepth {
    ExactDepth::new(ChannelType::U16, SignalRange::Full, 8, 16).unwrap()
}

#[test]
fn declaration_defaults_builder_debug_and_equality_include_the_claim() {
    let plain = EncodeDeclaration::new(2, 3, PixelDescriptor::RGB16);
    assert_eq!(plain.exact_depth, None);
    let claimed = plain.with_exact_depth(eight_in_sixteen());
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
            PixelDescriptor::RGB8,
            full,
            ExactDepthError::DescriptorMismatch,
        ),
        (
            PixelDescriptor::RGB16.with_signal_range(SignalRange::Narrow),
            full,
            ExactDepthError::DescriptorMismatch,
        ),
        (
            PixelDescriptor::CMYK8,
            ExactDepth::new(ChannelType::U8, SignalRange::Full, 8, 8).unwrap(),
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
        plan.validate(&EncodeDeclaration::new(2, 3, PixelDescriptor::CMYK8))
            .is_ok()
    );
}

#[test]
fn structural_validation_is_deliberately_not_a_content_certificate() {
    let declaration =
        EncodeDeclaration::new(1, 1, PixelDescriptor::GRAY16).with_exact_depth(eight_in_sixteen());
    assert!(declaration.validate_exact_depth().is_ok());
    // A legal assertion can still be false for actual samples.
    let mut samples = zenpixels::PixelBuffer::new(1, 1, PixelDescriptor::GRAY16);
    samples
        .as_slice_mut()
        .row_mut(0)
        .copy_from_slice(&1u16.to_ne_bytes());
    assert_eq!(eight_in_sixteen().holds_for(&samples.as_slice()), Ok(false));
}
