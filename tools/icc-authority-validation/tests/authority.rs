//! Real recognized profiles distinguish identification from substitution permission.
use zencodec::encode::EncodeCapabilities;
use zencodec::{Cicp, ColorEmitPolicy, IccDisposition, SourceColor, resolve_color_emit};
use zenpixels::{
    ColorAuthority,
    icc::{IccIdentification, IdentificationUse},
};
use zenpixels_convert::icc_profiles::{DISPLAY_P3_V4, SynthesizedIcc};

fn caps(sole_safe: bool) -> EncodeCapabilities {
    EncodeCapabilities::new()
        .with_icc(true)
        .with_cicp(true)
        .with_cicp_is_valid_carrier(true)
        .with_cicp_safe_sole_carrier(sole_safe)
}

fn metadata_profile() -> Vec<u8> {
    let SynthesizedIcc::Profile(bytes) = SynthesizedIcc::for_cicp(Cicp::new(9, 16, 0, true)) else {
        panic!("qualified PQ export must exist");
    };
    assert_eq!(Cicp::from_icc(&bytes), Some(Cicp::new(9, 16, 0, true)));
    assert_eq!(
        IccIdentification::from_icc(&bytes).unwrap().valid_use,
        IdentificationUse::MetadataOnly
    );
    bytes.into_owned()
}

#[test]
fn recognized_metadata_only_icc_remains_authoritative() {
    let source = SourceColor::default()
        .with_icc_profile(metadata_profile())
        .with_channel_count(3);
    for sole_safe in [false, true] {
        for policy in [ColorEmitPolicy::Balanced, ColorEmitPolicy::Compact] {
            let plan = resolve_color_emit(&source, &caps(sole_safe), policy);
            assert_eq!(plan.icc, IccDisposition::KeepSource);
            assert_eq!(plan.cicp, None);
        }
    }
}

#[test]
fn qualified_matrix_trc_identification_can_use_a_sole_safe_carrier() {
    let id = IccIdentification::from_icc(DISPLAY_P3_V4).unwrap();
    assert_eq!(id.valid_use, IdentificationUse::MatrixTrcSubstitution);
    let source = SourceColor::default()
        .with_icc_profile(DISPLAY_P3_V4.to_vec())
        .with_channel_count(3);
    let plan = resolve_color_emit(&source, &caps(true), ColorEmitPolicy::Balanced);
    assert_eq!(plan.cicp, id.to_cicp());
    assert!(plan.cicp.is_some());
    assert_eq!(plan.icc, IccDisposition::Drop);
}

#[test]
fn explicit_cicp_authority_still_selects_its_own_transform() {
    let cicp = Cicp::new(9, 16, 0, true);
    let source = SourceColor::default()
        .with_icc_profile(metadata_profile())
        .with_cicp(cicp)
        .with_color_authority(ColorAuthority::Cicp)
        .with_channel_count(3);
    let plan = resolve_color_emit(&source, &caps(true), ColorEmitPolicy::Balanced);
    assert_eq!(plan.cicp, Some(cicp));
    assert_eq!(plan.icc, IccDisposition::Drop);
}
