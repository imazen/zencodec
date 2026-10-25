//! Execute the exact Rust source displayed by the documentation site.
#![forbid(unsafe_code)]
#[path = "../../../docs/examples/journeys.rs"]
mod guide;
use std::sync::Arc;
use zenpixels::{PixelDescriptor, PixelSlice};

#[tokio::test]
async fn guide_preserves_strided_pixels_across_real_codec_paths() {
    let (w, h, stride) = (37u32, 29u32, 37 * 4 + 16);
    let mut data = vec![0xa5; stride * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let p = y * stride + x * 4;
            data[p..p + 4].copy_from_slice(&[x as u8, y as u8, (x + y) as u8, 127]);
        }
    }
    let pixels = PixelSlice::new(&data, w, h, stride, PixelDescriptor::RGBA8_SRGB).unwrap();
    let encoded = guide::encode(pixels.clone()).unwrap();
    let bytes: Arc<[u8]> = Arc::from(encoded.data());
    let image = guide::preview(&bytes).unwrap();
    assert!(guide::limited(&bytes, u64::from(w * h) - 1).is_err());
    let limited = guide::limited(&bytes, u64::from(w * h)).unwrap();
    let preferred = guide::preference(&bytes).unwrap();
    let retained = guide::retained(&image).unwrap();
    let decoded = guide::preview(retained.data()).unwrap();
    let finish_first = guide::finish_before_write(bytes.clone()).await.unwrap();
    let finished = guide::preview(finish_first.data()).unwrap();
    for out in [&image, &limited, &preferred, &decoded, &finished] {
        // Native QOI preserves samples but leaves descriptor transfer unknown.
        assert_eq!(out.descriptor(), PixelDescriptor::RGBA8);
        for y in 0..h {
            assert_eq!(out.pixels().row(y), pixels.row(y));
        }
    }
    let mut source = zencodec::source::SliceReadAt::new(&bytes);
    for sink in [
        guide::from_slice(&bytes).unwrap(),
        guide::positioned(&mut source).unwrap(),
        guide::asynchronous(bytes.clone()).await.unwrap(),
        guide::bridge(&bytes).unwrap(),
    ] {
        assert!(sink.finished());
        let view = sink.as_pixel_slice().unwrap();
        for y in 0..h {
            assert_eq!(view.row(y), pixels.row(y));
        }
    }
    guide::bounded_batch_example().unwrap();
}

#[test]
fn registry_conveniences_detect_both_codecs_and_preserve_samples() {
    use zenbitmaps::{BmpEncoderConfig, QoiEncoderConfig};
    use zencodec::encode::EncoderConfig;
    use zencodec::{CodecSetError, ImageFormat, TranscodeBarrier};

    let (w, h, stride) = (5u32, 3u32, 24);
    let mut data = vec![0xa5; stride * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            data[y * stride + x * 3..y * stride + x * 3 + 3].copy_from_slice(&[
                x as u8,
                y as u8,
                (x + y) as u8,
            ]);
        }
    }
    let pixels = PixelSlice::new(&data, w, h, stride, PixelDescriptor::RGB8_SRGB).unwrap();
    let codecs = guide::gallery_codecs();
    for (format, encoded) in [
        (
            ImageFormat::Qoi,
            QoiEncoderConfig::new().encode(pixels.clone()).unwrap(),
        ),
        (
            ImageFormat::Bmp,
            BmpEncoderConfig::new().encode(pixels.clone()).unwrap(),
        ),
    ] {
        assert_eq!(codecs.detect(encoded.data()), Some(format));
        let info = codecs.probe(encoded.data()).unwrap();
        assert_eq!((info.width, info.height), (w, h));
        let image = guide::registered_preview(&codecs, encoded.data()).unwrap();
        for y in 0..h {
            assert_eq!(image.pixels().row(y), pixels.row(y));
        }
        let derivative = guide::registered_derivative(&codecs, &image).unwrap();
        let again = guide::registered_preview(&codecs, derivative.data()).unwrap();
        for y in 0..h {
            assert_eq!(again.pixels().row(y), pixels.row(y));
        }
        let sink = guide::registered_incremental(&codecs, encoded.data()).unwrap();
        for y in 0..h {
            assert_eq!(sink.as_pixel_slice().unwrap().row(y), pixels.row(y));
        }
        match guide::registered_transcode(&codecs, encoded.data()) {
            Ok(output) => {
                let decoded = guide::registered_preview(&codecs, output.data()).unwrap();
                for y in 0..h {
                    assert_eq!(decoded.pixels().row(y), pixels.row(y));
                }
            }
            Err(CodecSetError::TranscodeRequiresExplicitHandling(
                TranscodeBarrier::PixelDescriptor,
            )) => {
                // Correct guard behavior for this pinned adapter's unknown-transfer output.
                // Accept a future adapter fix through the successful exact-pixel branch above.
                assert!(!QoiEncoderConfig::supported_descriptors().contains(&image.descriptor()));
                eprintln!(
                    "observed {format:?} to QOI transcode descriptor barrier: {:?}",
                    image.descriptor()
                );
            }
            Err(error) => panic!("unexpected registry transcode failure: {error}"),
        }
    }
    let encoded = guide::registered_encode(&codecs, pixels.clone()).unwrap();
    let probe = guide::probe(encoded.data()).unwrap();
    assert_eq!((probe.width, probe.height), (w, h));
    let decoded = guide::registered_preview(&codecs, encoded.data()).unwrap();
    for y in 0..h {
        assert_eq!(decoded.pixels().row(y), pixels.row(y));
    }
    assert!(guide::registered_preview(&codecs, b"unrecognized input").is_err());
    assert!(
        guide::gallery_codecs()
            .with_limits(zencodec::ResourceLimits::none().with_max_pixels(14))
            .decode(encoded.data())
            .is_err()
    );
}
