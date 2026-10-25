use zencodec::encode::EncoderConfig;
use zenpixels::{PixelDescriptor, PixelSlice};
#[test]
fn detects_actual_formats_and_rejects_unknown() {
    let pixels = [23u8; 7 * 5 * 3];
    let view = PixelSlice::new(&pixels, 7, 5, 21, PixelDescriptor::RGB8_SRGB).unwrap();
    for bytes in [
        zenbitmaps::QoiEncoderConfig::new()
            .encode(view.clone())
            .unwrap(),
        zenbitmaps::BmpEncoderConfig::new().encode(view).unwrap(),
    ] {
        assert_eq!(candidate::decode_upload(bytes.data()).unwrap(), (7, 5));
    }
    assert!(candidate::decode_upload(b"not an image").is_err());
}
