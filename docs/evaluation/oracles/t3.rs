use zencodec::{
    encode::EncoderConfig,
    source::{ReadAt, SliceReadAt},
};
use zenpixels::{PixelDescriptor, PixelSlice};
struct Source<'a> {
    bytes: &'a [u8],
    reads: usize,
    fail: bool,
}
impl ReadAt for Source<'_> {
    type Error = std::io::Error;
    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(self.bytes.len() as u64)
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        self.reads += 1;
        if self.fail {
            return Err(std::io::Error::other("fixture source failure"));
        }
        SliceReadAt::new(self.bytes)
            .read_exact_at(offset, out)
            .map_err(std::io::Error::other)
    }
}
#[test]
fn positioned_path_reads_and_preserves_rows_and_reports_source_failure() {
    let raw: Vec<_> = (0..7 * 5 * 3).map(|v| v as u8).collect();
    let view = PixelSlice::new(&raw, 7, 5, 21, PixelDescriptor::RGB8_SRGB).unwrap();
    let bytes = zenbitmaps::QoiEncoderConfig::new()
        .encode(view.clone())
        .unwrap();
    let mut source = Source {
        bytes: bytes.data(),
        reads: 0,
        fail: false,
    };
    let output = candidate::decode_positioned(&mut source).unwrap();
    assert!(source.reads > 0);
    assert!(output.finished());
    assert_eq!((output.width(), output.height()), (7, 5));
    for y in 0..5 {
        assert_eq!(output.as_pixel_slice().unwrap().row(y), view.row(y));
    }
    source.fail = true;
    assert!(candidate::decode_positioned(&mut source).is_err());
}
