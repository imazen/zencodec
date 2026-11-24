//! The pixel paths a decode takes through zencodec: rows into `CollectSink`, reading
//! the result, and `to_owned_frame`. Run against two zenpixels storages to compare them.
//! A decoder is simulated by filling each strip; no codec runs here.

use zenbench::prelude::*;
use zencodec::AnimationFrame;
use zencodec::decode::DecodeRowSink;
use zencodec_io::CollectSink;
use zenpixels::{ImageSpec, PixelBuf};

const STRIP: u32 = 16;

fn collect(w: u32, h: u32, d: ImageSpec, fill: u8) -> PixelBuf {
    let mut sink = CollectSink::new();
    sink.begin(w, h, d.clone()).unwrap();
    let mut y = 0;
    while y < h {
        let n = STRIP.min(h - y);
        let mut strip = sink.provide_next_buffer(y, n, w, d.clone()).unwrap();
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
            ("u8 RGB8", ImageSpec::rgb8_srgb()),
            ("u16 RGB16", ImageSpec::rgb16_srgb()),
            ("f32 RGBA", ImageSpec::rgbaf32_linear()),
        ];
        for (name, d) in formats {
            for (w, h) in [(64u32, 64u32), (1024, 1024), (4096, 4096)] {
                let bytes = u64::from(w) * u64::from(h) * d.layout.bytes_per_pixel() as u64;
                suite.group(format!("{name} {w}x{h}"), |g| {
                    g.throughput(Throughput::Bytes(bytes));
                    g.config().auto_rounds(false).min_rounds(20).max_rounds(20);
                    let spec = d.clone();
                    g.bench("collect", move |bench| {
                        bench.iter(|| black_box(collect(w, h, spec.clone(), 3)));
                    });
                    let source = collect(w, h, d.clone(), 5);
                    g.bench("read rows", move |bench| {
                        bench.iter(|| {
                            let slice = source.as_view();
                            let mut sum = 0u64;
                            for y in 0..slice.rows() {
                                sum += u64::from(slice.row(y)[0]);
                            }
                            black_box(sum)
                        });
                    });
                    let source = collect(w, h, d.clone(), 7);
                    g.bench("to_owned_frame", move |bench| {
                        bench.iter(|| {
                            let frame = AnimationFrame::new(source.as_view(), 10, 0);
                            black_box(frame.try_to_owned_frame().unwrap())
                        });
                    });
                });
            }
        }
    });
}
