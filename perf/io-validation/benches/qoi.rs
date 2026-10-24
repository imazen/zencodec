//! QOI decode -> QOI encode: high-compression and changing-pixel controls.
#![forbid(unsafe_code)]
use std::sync::Arc;
use zenbench::prelude::*;
use zenbitmaps::{QoiDecoderConfig, QoiEncoderConfig};
use zencodec::decode::DecoderConfig;
use zencodec::encode::{EncodeJob, EncoderConfig};
use zencodec_io::DriveOptions;
use zencodec_io::tokio_io::{BytesSource, Inline, transcode_async};
use zenpixels::{PixelDescriptor, PixelSlice};
fn qoi(suite: &mut Suite) {
    for smooth in [true, false] {
        let mut pixels = vec![0u8; 512 * 512 * 4];
        for (i, p) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if smooth {
                p.copy_from_slice(&[32, 64, 128, 255]);
            } else {
                p.copy_from_slice(&[
                    (i * 71) as u8,
                    (i / 512 * 17) as u8,
                    (i ^ (i >> 8)) as u8,
                    255,
                ]);
            }
        }
        let encoded = QoiEncoderConfig::new()
            .encode(PixelSlice::new(&pixels, 512, 512, 2048, PixelDescriptor::RGBA8_SRGB).unwrap())
            .unwrap();
        let bytes: Arc<[u8]> = encoded.data().into();
        let shape = if smooth { "solid" } else { "changing" };
        suite.group(
            format!("qoi-to-qoi/512x512/{shape}/{}-input-bytes", bytes.len()),
            |g| {
                g.config()
                    .max_time(std::time::Duration::from_secs(2))
                    .max_rounds(30);
                for pooled in [false, true] {
                    let bytes = bytes.clone();
                    g.bench(if pooled { "spawn_blocking" } else { "inline" }, move |b| {
                        let rt = tokio::runtime::Builder::new_multi_thread()
                            .worker_threads(2)
                            .build()
                            .unwrap();
                        let bytes = bytes.clone();
                        b.iter(move || {
                            rt.block_on(async {
                                let dec = QoiDecoderConfig::new()
                                    .job()
                                    .into_incremental_decoder(&[])
                                    .unwrap();
                                let enc = QoiEncoderConfig::new().job().encoder().unwrap();
                                let mut source = BytesSource(bytes.clone());
                                let mut out = Vec::new();
                                if pooled {
                                    transcode_async(
                                        dec,
                                        enc,
                                        &mut source,
                                        &mut out,
                                        |job| {
                                            tokio::task::spawn_blocking(job);
                                        },
                                        DriveOptions::default(),
                                        None,
                                    )
                                    .await
                                    .unwrap();
                                } else {
                                    transcode_async(
                                        dec,
                                        enc,
                                        &mut source,
                                        &mut out,
                                        Inline,
                                        DriveOptions::default(),
                                        None,
                                    )
                                    .await
                                    .unwrap();
                                }
                                // QOI's output must be independent of drain schedule.
                                assert_eq!(out.as_slice(), bytes.as_ref());
                                std::hint::black_box(out)
                            })
                        })
                    });
                }
            },
        );
    }
}
zenbench::main!(qoi);
