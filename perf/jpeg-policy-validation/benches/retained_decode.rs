//! Compare actual native preparation plus decode against the same bound decode.
//! No file I/O, fixture generation or checksum loop is inside the timed region.
use std::{borrow::Cow, sync::Arc};
use zenbench::prelude::*;
use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
use zencodec::encode::EncoderConfig;
use zenjpeg::{JpegDecoderConfig, JpegEncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};

fn decode(bytes: &[u8], prepare: bool) -> zencodec::decode::DecodeOutput {
    let mut decoder = JpegDecoderConfig::new()
        .job()
        .decoder(Cow::Borrowed(bytes), &[PixelDescriptor::RGB8_SRGB])
        .unwrap();
    if prepare {
        assert_eq!(
            decoder.prepare_output().unwrap(),
            Some(PixelDescriptor::RGB8_SRGB)
        );
    }
    decoder.decode().unwrap()
}

fn bench_preparation(suite: &mut Suite) {
    for (width, height) in [(97u32, 65u32), (1024, 768)] {
        let mut state = 0x12345678u32;
        let pixels: Vec<u8> = (0..width as usize * height as usize * 3)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                if (i / 3 / 32) % 2 == 0 {
                    state as u8
                } else {
                    83
                }
            })
            .collect();
        for progressive in [false, true] {
            let encoded = JpegEncoderConfig::new()
                .with_progressive(progressive)
                .with_calibrated_quality(87.0)
                .encode(
                    PixelSlice::new(
                        &pixels,
                        width,
                        height,
                        width as usize * 3,
                        PixelDescriptor::RGB8_SRGB,
                    )
                    .unwrap(),
                )
                .unwrap();
            let bytes: Arc<[u8]> = Arc::from(encoded.data());
            let direct = decode(&bytes, false);
            let prepared = decode(&bytes, true);
            assert_eq!(direct.metadata(), prepared.metadata());
            assert_eq!(direct.pixels().descriptor(), prepared.pixels().descriptor());
            for row in 0..height {
                assert_eq!(direct.pixels().row(row), prepared.pixels().row(row));
            }
            suite.group(
                format!("retained/{width}x{height}/progressive={progressive}"),
                |g| {
                    for (name, prepare) in [("decode", false), ("prepare_then_decode", true)] {
                        let bytes = bytes.clone();
                        g.bench(name, move |b| b.iter(|| decode(black_box(&bytes), prepare)));
                    }
                },
            );
        }
    }
}
zenbench::main!(bench_preparation);
