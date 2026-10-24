//! Actual QOI codec: strided input, input boundaries, output drains and truncation.
#![forbid(unsafe_code)]
use std::io::{self, Read};
use zenbitmaps::{QoiDecoderConfig, QoiEncoderConfig};
use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
use zencodec::encode::{EncodeJob, EncoderConfig};
use zencodec_io::{CollectSink, drive_from_slice, pump_read_to_sink};
use zenpixels::{PixelDescriptor, PixelSlice};

struct Chunks<'a> {
    data: &'a [u8],
    size: usize,
}
impl Read for Chunks<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = out.len().min(self.data.len()).min(self.size);
        out[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}
fn fixture() -> (Vec<u8>, Vec<u8>) {
    let (w, h, stride) = (37, 29, 37 * 4 + 16);
    let mut raw = vec![0xED; stride * h];
    let mut packed = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let px = [
                (x * 7) as u8,
                (y * 11) as u8,
                (x ^ y) as u8,
                (x + y + 128) as u8,
            ];
            raw[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&px);
            packed.extend_from_slice(&px);
        }
    }
    let pixels = PixelSlice::new(
        &raw,
        w as u32,
        h as u32,
        stride,
        PixelDescriptor::RGBA8_SRGB,
    )
    .unwrap();
    let encoded = QoiEncoderConfig::new().encode(pixels.clone()).unwrap();
    let mut streamed = Vec::new();
    zencodec_io::encode_to_writer(
        QoiEncoderConfig::new().job().encoder().unwrap(),
        pixels,
        &mut streamed,
    )
    .unwrap();
    assert_eq!(encoded.data(), streamed, "draining changed QOI bytes");
    (streamed, packed)
}
#[test]
fn qoi_strided_encode_and_fragmented_decode_are_exact() {
    let (file, packed) = fixture();
    let decoded = QoiDecoderConfig::new()
        .job()
        .decoder(std::borrow::Cow::Borrowed(&file), &[])
        .unwrap()
        .decode()
        .unwrap();
    for y in 0..29 {
        assert_eq!(
            decoded.pixels().row(y),
            &packed[y as usize * 148..(y as usize + 1) * 148]
        );
    }
    for size in [1, 2, 3, 7, 31, 64, 4096, 65536] {
        let mut dec = QoiDecoderConfig::new()
            .job()
            .into_incremental_decoder(&[])
            .unwrap();
        let mut sink = CollectSink::new();
        drive_from_slice(&mut dec, &file, &mut sink, None, size).unwrap();
        assert_eq!(sink.bytes(), packed, "slice size {size}");
        let mut dec = QoiDecoderConfig::new()
            .job()
            .into_incremental_decoder(&[])
            .unwrap();
        let mut sink = CollectSink::new();
        pump_read_to_sink(
            Chunks { data: &file, size },
            &mut dec,
            &mut sink,
            None,
            size,
            1 << 20,
        )
        .unwrap();
        assert_eq!(sink.bytes(), packed, "pump size {size}");
    }
}
#[tokio::test]
async fn qoi_async_short_reads_are_exact() {
    use zencodec_io::tokio_io::{AsyncReadSource, Inline, decode_async};
    let (file, packed) = fixture();
    for size in [1, 7, 64, 65536] {
        let mut source = AsyncReadSource::new(file.as_slice());
        let (_, sink) = decode_async(
            QoiDecoderConfig::new()
                .job()
                .into_incremental_decoder(&[])
                .unwrap(),
            CollectSink::new(),
            &mut source,
            Inline,
            zencodec_io::DriveOptions::new(0, size),
            None,
        )
        .await
        .unwrap();
        assert_eq!(sink.bytes(), packed);
    }
}
#[test]
fn qoi_all_prefixes_match_one_shot_acceptance_and_pixels() {
    let (file, packed) = fixture();
    for cut in 0..file.len() {
        let mut dec = QoiDecoderConfig::new()
            .job()
            .into_incremental_decoder(&[])
            .unwrap();
        let mut sink = CollectSink::new();
        let incremental = drive_from_slice(&mut dec, &file[..cut], &mut sink, None, 7);
        let whole = QoiDecoderConfig::new().decode(&file[..cut]);
        assert_eq!(
            incremental.is_ok(),
            whole.is_ok(),
            "acceptance differs at cut {cut}/{}",
            file.len()
        );
        if let Ok(image) = whole {
            assert_eq!(
                sink.bytes(),
                packed,
                "accepted prefix changed pixels at {cut}"
            );
            for y in 0..29 {
                assert_eq!(
                    image.pixels().row(y),
                    &packed[y as usize * 148..(y as usize + 1) * 148]
                );
            }
        }
    }
}
