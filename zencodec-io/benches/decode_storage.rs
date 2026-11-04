//! The pixel paths a decode takes through zencodec: rows into `CollectSink`, reading
//! the result, and `to_owned_frame`. Run against two zenpixels storages to compare them.
//! A decoder is simulated by filling each strip; no codec runs here.

use zenbench::prelude::*;
use zencodec::decode::DecodeRowSink;
use zencodec::AnimationFrame;
use zencodec_io::CollectSink;
use zenpixels::{PixelBuffer, PixelDescriptor};

const STRIP: u32 = 16;

fn collect(w: u32, h: u32, d: PixelDescriptor, fill: u8) -> PixelBuffer {
    let mut sink = CollectSink::new();
    sink.begin(w, h, d).unwrap();
    let mut y = 0;
    while y < h {
        let n = STRIP.min(h - y);
        let mut strip = sink.provide_next_buffer(y, n, w, d).unwrap();
        for row in 0..n {
            strip.row_mut(row).fill(fill);
        }
        y += n;
    }
    sink.finish().unwrap();
    sink.into_buffer().unwrap()
}

fn main() {
    zenbench::run(|suite| {
        let formats = [
            ("u8 RGB8", PixelDescriptor::RGB8_SRGB),
            ("u16 RGB16", PixelDescriptor::RGB16_SRGB),
            ("f32 RGBA", PixelDescriptor::RGBAF32_LINEAR),
        ];
        for (name, d) in formats {
            for (w, h) in [(64u32, 64u32), (1024, 1024), (4096, 4096)] {
                let bytes = u64::from(w) * u64::from(h) * d.bytes_per_pixel() as u64;
                suite.group(format!("{name} {w}x{h}"), |g| {
                    g.throughput(Throughput::Bytes(bytes));
                    g.config().auto_rounds(false).min_rounds(20).max_rounds(20);
                    g.bench("collect", move |bench| {
                        bench.iter(|| black_box(collect(w, h, d, 3)));
                    });
                    let source = collect(w, h, d, 5);
                    g.bench("read rows", move |bench| {
                        bench.iter(|| {
                            let slice = source.as_slice();
                            let mut sum = 0u64;
                            for y in 0..slice.rows() {
                                sum += u64::from(slice.row(y)[0]);
                            }
                            black_box(sum)
                        });
                    });
                    let source = collect(w, h, d, 7);
                    g.bench("to_owned_frame", move |bench| {
                        bench.iter(|| {
                            let frame = AnimationFrame::new(source.as_slice(), 10, 0);
                            black_box(frame.to_owned_frame())
                        });
                    });
                });
            }
        }
    });
}
