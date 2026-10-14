//! The sans-I/O conformance checks against both in-crate codecs: the
//! reference declares `incremental` and `seekable` (true direction), the
//! minimal codec declares neither (false direction).

use zencodec::decode::DecoderConfig;
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec::source::{ReadAtShim, SliceReadAt};
use zencodec_testkit::reference::{ReferenceDecoderConfig, ReferenceEncoderConfig};
use zencodec_testkit::{
    TestImage, check_incremental_capability_honesty, check_read_at_contract,
    check_seekable_capability_honesty,
};

fn reference_bytes() -> Vec<u8> {
    let img = TestImage::rgba8_gradient(40, 24);
    ReferenceEncoderConfig::new()
        .job()
        .encoder()
        .expect("reference encoder")
        .encode(img.as_slice())
        .expect("reference encode")
        .into_vec()
}

#[test]
fn slice_source_meets_the_read_at_contract() {
    let bytes = reference_bytes();
    let mut src = ReadAtShim(SliceReadAt::new(&bytes));
    check_read_at_contract(&mut src, &bytes).unwrap();
    let mut empty = ReadAtShim(SliceReadAt::new(&[]));
    check_read_at_contract(&mut empty, &[]).unwrap();
}

#[test]
fn reference_codec_is_honest_in_the_true_direction() {
    let bytes = reference_bytes();
    assert!(ReferenceDecoderConfig::capabilities().incremental());
    assert!(ReferenceDecoderConfig::capabilities().seekable());
    check_incremental_capability_honesty(ReferenceDecoderConfig, &bytes).unwrap();
    check_seekable_capability_honesty(ReferenceDecoderConfig, &bytes).unwrap();
}

#[test]
fn check_all_covers_the_sansio_checks() {
    zencodec_testkit::check_all(ReferenceEncoderConfig::new(), ReferenceDecoderConfig).unwrap();
}
