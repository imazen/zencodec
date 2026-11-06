//! The resolver's gray plan, lowered through the real ICC synthesis.
//!
//! `resolve_color_emit` can't depend on `zenpixels-convert`, so it copies that
//! crate's "the sRGB default needs no profile" rule. This keeps the two in step
//! over all code points, and checks the profile is one a gray image may carry.

use zencodec::encode::EncodeCapabilities;
use zencodec::{ColorEmitPolicy, IccDisposition, SourceColor, resolve_color_emit};
use zenpixels::{Cicp, ColorModel};
use zenpixels_convert::icc_profiles::SynthesizedIcc;

/// PNG's carriers: CICP is a valid carrier but not safe alone.
fn png_caps() -> EncodeCapabilities {
    EncodeCapabilities::new()
        .with_icc(true)
        .with_cicp(true)
        .with_cicp_is_valid_carrier(true)
}

fn plan_for(cicp: Cicp, channels: u8) -> IccDisposition {
    let source = SourceColor::default()
        .with_cicp(cicp)
        .with_channel_count(channels);
    resolve_color_emit(&source, &png_caps(), ColorEmitPolicy::Balanced).icc
}

#[test]
fn gray_plan_agrees_with_gray_synthesis_for_all_code_points() {
    let mut profiles = 0_u32;
    let mut defaults = 0_u32;
    for primaries in 0..=u8::MAX {
        for transfer in 0..=u8::MAX {
            let cicp = Cicp::new(primaries, transfer, 0, true);
            let synthesized = SynthesizedIcc::for_gray_cicp(cicp);
            for channels in [1, 2] {
                match (&synthesized, plan_for(cicp, channels)) {
                    (SynthesizedIcc::NotNeeded, IccDisposition::Drop) => defaults += 1,
                    (SynthesizedIcc::Profile(_), IccDisposition::SynthesizeGrayFrom(asked)) => {
                        assert_eq!(asked, cicp);
                        profiles += 1;
                    }
                    (SynthesizedIcc::CmsUnsupported, IccDisposition::SynthesizeGrayFrom(asked)) => {
                        assert_eq!(asked, cicp);
                    }
                    (outcome, plan) => panic!("{cicp:?}: {outcome:?} but the plan is {plan:?}"),
                }
            }
        }
    }
    // Gray serves 175 assigned non-default pairs and 5 default pairs (zenpixels-convert).
    assert_eq!(profiles, 175 * 2);
    assert_eq!(defaults, 5 * 2);
}

#[test]
fn lowered_profile_is_gray_class_and_round_trips_through_the_resolver() {
    for cicp in [Cicp::new(1, 16, 0, true), Cicp::new(1, 4, 0, true)] {
        let IccDisposition::SynthesizeGrayFrom(asked) = plan_for(cicp, 1) else {
            panic!("{cicp:?} should synthesize");
        };
        let SynthesizedIcc::Profile(profile) = SynthesizedIcc::for_gray_cicp(asked) else {
            panic!("{cicp:?} should have a bundled gray profile");
        };
        assert_eq!(ColorModel::from_icc(&profile), Some(ColorModel::Gray));

        // A second encode of that output keeps the profile instead of making another.
        let reread = SourceColor::default()
            .with_icc_profile(profile.to_vec())
            .with_channel_count(1);
        let plan = resolve_color_emit(&reread, &png_caps(), ColorEmitPolicy::Balanced);
        assert_eq!(plan.icc, IccDisposition::KeepSource);
        assert_eq!(
            plan.cicp,
            zenpixels::icc::IccIdentification::from_icc(&profile).and_then(|id| id.to_cicp())
        );
    }
}
