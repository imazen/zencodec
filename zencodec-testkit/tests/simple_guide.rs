#[path = "../../docs/examples/simple_codec.rs"]
mod guide;
use zencodec::{CodecSet, ImageFormat};
use zencodec_testkit::{ReferenceEncoderConfig, ReferenceZcrDecoderConfig, TestImage};
#[test]
fn two_line_pixel_conversion_uses_registered_codecs() {
    let codecs = CodecSet::new()
        .with_decoder(ReferenceZcrDecoderConfig)
        .with_encoder(ReferenceEncoderConfig::new());
    let original = TestImage::rgb8_gradient(7, 3);
    let bytes = codecs
        .encode(ImageFormat::Pnm, original.as_slice())
        .unwrap();
    let output = guide::convert_pixels(&codecs, bytes.data(), ImageFormat::Pnm).unwrap();
    let decoded = codecs.decode(output.data()).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (7, 3));
    for y in 0..3 {
        assert_eq!(decoded.pixels().row(y), original.as_slice().row(y));
    }
}
#[test]
fn advanced_paths_are_the_same_contract_types() {
    fn same(input: zencodec::advanced::decode::Input<'_>) -> zencodec::decode::Input<'_> {
        input
    }
    let input = same(zencodec::decode::Input::empty());
    assert_eq!(input.offset, 0);
}
