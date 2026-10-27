// Keep the legacy writer protocol/fallback regression coverage executable.
#![allow(deprecated)]
//! Observations that must not become expectations preserving known defects.
#![forbid(unsafe_code)]
use zenbitmaps::BmpEncoderConfig;
use zencodec::decode::ByteRange;
use zencodec::encode::{EncodeJob, EncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};

fn main() {
    let data = [7u8; 16];
    let pixels = PixelSlice::new(&data, 2, 2, 8, PixelDescriptor::RGBA8_SRGB).unwrap();
    let direct = BmpEncoderConfig::new().encode(pixels.clone()).unwrap();
    let mut output = Vec::new();
    let result = zencodec_io::encode_to_writer(
        BmpEncoderConfig::new().job().encoder().unwrap(),
        pixels,
        &mut output,
    );
    println!(
        "bmp direct_bytes={} writer={result:?} written={}",
        direct.len(),
        output.len()
    );
    if result.is_ok() {
        assert_eq!(output, direct.data());
    }

    let ranges = [0, 64, 8192, 16384].map(|offset| ByteRange::new(offset, 32));
    for gap in [0, 32, 4096, 8192] {
        let merged = zencodec_io::coalesce_byte_ranges(&ranges, gap);
        let fetched: u64 = merged.iter().map(|r| r.length).sum();
        println!(
            "coalescing gap={gap} useful_bytes=128 fetched_bytes={fetched} ranges={}",
            merged.len()
        );
    }
}
