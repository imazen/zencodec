//! The seekable-source testkit checks, exercised against the harness's
//! own codecs and the in-crate slice source.

use zencodec::source::{DynReadAt, ReadAtShim, SliceReadAt};
use zencodec_testkit::{
    ReferenceDecoderConfig, check_read_at_contract, check_seekable_capability_honesty,
};

#[test]
fn read_at_contract_accepts_slice_source() {
    let expected: Vec<u8> = (0..250u8).collect();
    let mut source: Box<dyn DynReadAt> = Box::new(ReadAtShim(SliceReadAt::new(&expected)));
    check_read_at_contract(&mut *source, &expected).expect("slice source honors ReadAt contract");
}

#[test]
fn seekable_honesty_false_direction_for_reference_codec() {
    // The reference codec does not declare seekable decoding, so the
    // check must see a clean SeekableDecode rejection.
    let expected: &'static [u8] = Box::leak(Box::new((0..64u8).collect::<Vec<u8>>()));
    let source: Box<dyn DynReadAt> = Box::new(ReadAtShim(SliceReadAt::new(expected)));
    check_seekable_capability_honesty(ReferenceDecoderConfig, source)
        .expect("undeclared seekable must reject cleanly");
}
