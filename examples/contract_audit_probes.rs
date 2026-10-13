//! Observations for the published-contract audit, not regression expectations.
//! Re-run after contract changes; output is allowed to change as gaps are fixed.
use std::borrow::Cow;
use zencodec::encode::EncodeCapabilities;
use zencodec::metadata_rewrite::{Action, Entry, Plan, Request};
use zencodec::{
    ColorEmitPolicy, IccDisposition, MetadataPolicy, SourceColor, UnsupportedOperation,
    resolve_color_emit,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = [1_u8];
    let plan = Plan::from_parts(
        vec![Cow::Borrowed(&bytes)],
        vec![Entry::new(100..101, "example", Action::Keep)],
    );
    println!("out_of_bounds_report_accepted={}", plan.is_ok());
    println!(
        "preserve_exact_empty_request={:?}",
        Request::new(&[])
            .with_policy(MetadataPolicy::PreserveExact)
            .validate()
    );

    let no_carriers = EncodeCapabilities::new();
    let source = SourceColor::default().with_cicp(zencodec::Cicp::DISPLAY_P3);
    let plan = resolve_color_emit(&source, &no_carriers, ColorEmitPolicy::Balanced);
    println!(
        "icc_required_without_target_icc={}",
        matches!(plan.icc, IccDisposition::SynthesizeFrom(_)) && !no_carriers.icc()
    );
    println!(
        "animation_capability_reports_exact_timing={}",
        EncodeCapabilities::new()
            .with_animation(true)
            .supports(UnsupportedOperation::AnimationTiming)
    );
    Ok(())
}
