use zencodec::encode::EncoderConfig;
use zenpixels::{PixelDescriptor, PixelSlice};
#[test]
fn exact_limit_passes_and_one_less_refuses_without_changing_pixels() {
    let raw: Vec<_> = (0..7 * 5 * 3).map(|v| v as u8).collect();
    let view = PixelSlice::new(&raw, 7, 5, 21, PixelDescriptor::RGB8_SRGB).unwrap();
    let bytes = zenbitmaps::QoiEncoderConfig::new()
        .encode(view.clone())
        .unwrap();
    assert!(candidate::decode_limited(bytes.data(), 34).is_err());
    let output = candidate::decode_limited(bytes.data(), 35).unwrap();
    assert_eq!((output.width(), output.height()), (7, 5));
    for y in 0..5 {
        assert_eq!(output.pixels().row(y), view.row(y));
    }
}
