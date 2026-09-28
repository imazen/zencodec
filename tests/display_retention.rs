use zencodec::{GainMapParams, Metadata, MetadataFields, MetadataPolicy, Retention};
use zenpixels::DiffuseWhite;
#[test]
fn source_xmp_is_dropped_but_render_fields_survive() {
    let meta = Metadata::none()
        .with_xmp(b"private source XMP".to_vec())
        .with_cicp(zencodec::Cicp::SRGB);
    let clean = zencodec::display_metadata::filter_for_gain_map(
        &meta,
        &GainMapParams::default(),
        &MetadataPolicy::ColorAndRotation,
    )
    .unwrap();
    assert!(clean.xmp.is_none());
    assert_eq!(clean.cicp, meta.cicp);
}
#[test]
fn cannot_drop_luminance_or_invalid_gain_parameters() {
    let meta = Metadata::none().with_diffuse_white(DiffuseWhite::new(100.0));
    let policy = MetadataPolicy::Custom(MetadataFields::KEEP_ALL.with_hdr(Retention::Discard));
    assert!(
        zencodec::display_metadata::filter_for_gain_map(&meta, &GainMapParams::default(), &policy)
            .is_err()
    );
    let mut params = GainMapParams::default();
    params.channels[0].gamma = f64::NAN;
    assert!(
        zencodec::display_metadata::filter_for_gain_map(
            &Metadata::none(),
            &params,
            &MetadataPolicy::Web
        )
        .is_err()
    );
}
