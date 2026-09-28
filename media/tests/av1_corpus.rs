#![cfg(feature = "av1-decode")]

use std::io::{self, Read};
use zencodec_media::av1::Av1IvfDecoder;
use zencodec_media::plane::Samples;
use zenpixels::ChannelType;

#[test]
fn cancelled_decode_retains_its_error_category_and_poison_state() {
    struct Cancel;
    impl enough::Stop for Cancel {
        fn check(&self) -> Result<(), enough::StopReason> {
            Err(enough::StopReason::Cancelled)
        }
    }
    let bytes = include_bytes!("../corpus/av1/av1-10-420-full-17x13.ivf");
    let mut decoder =
        Av1IvfDecoder::new(bytes.as_slice(), 1 << 20, rav1d_safe::Settings::default()).unwrap();
    decoder.set_stop(Some(std::sync::Arc::new(Cancel)));
    match decoder.next_frame() {
        Err(zencodec_media::av1::DecodeError::Codec(error)) => {
            assert_eq!(error.error(), &rav1d_safe::Error::Cancelled)
        }
        _ => panic!("cancelled decoding must retain its category"),
    }
    assert!(matches!(
        decoder.next_frame(),
        Err(zencodec_media::av1::DecodeError::FailedStream)
    ));
}

struct Fragments<'a> {
    bytes: &'a [u8],
}
impl Read for Fragments<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = buffer.len().min(self.bytes.len()).min(7);
        buffer[..n].copy_from_slice(&self.bytes[..n]);
        self.bytes = &self.bytes[n..];
        Ok(n)
    }
}

#[test]
fn every_lossless_corpus_sample_survives_native_mapping_and_decoder_drop() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../corpus/av1-manifest.json")).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 48);
    for case in cases {
        let n = |field: &str| case[field].as_u64().unwrap() as usize;
        let bits = n("bits");
        let (width, height) = (n("width"), n("height"));
        let full = case["range"] == "full";
        let sampling = case["sampling"].as_str().unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("corpus")
            .join(case["path"].as_str().unwrap());
        let encoded = std::fs::read(path).unwrap();
        let mut decoder = Av1IvfDecoder::new(
            Fragments { bytes: &encoded },
            1 << 20,
            rav1d_safe::Settings::default(),
        )
        .unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            frames.push(frame);
        }
        assert!(decoder.next_frame().unwrap().is_none());
        drop(decoder);
        assert_eq!(frames.len(), n("frames"), "{}", case["id"]);
        let (sx, sy) = match sampling {
            "420" => (2, 2),
            "422" => (2, 1),
            _ => (1, 1),
        };
        let components = if sampling == "mono" { 1 } else { 3 };
        for (f, frame) in frames.iter().enumerate() {
            assert_eq!(
                (frame.width(), frame.height()),
                (width as u32, height as u32)
            );
            assert_eq!(frame.bit_depth(), bits as u8);
            assert_eq!(frame.timestamp().unwrap().ticks(), f as i64);
            assert_eq!(frame.color().full_range, full);
            assert_eq!(frame.color().matrix_coefficients, 1);
            let mapped = frame.map();
            for component in 0..3 {
                let plane = mapped.plane(component).unwrap();
                if component >= components {
                    assert!(plane.is_none());
                    continue;
                }
                let plane = plane.unwrap();
                let (pw, ph) = if component == 0 {
                    (width, height)
                } else {
                    (width.div_ceil(sx), height.div_ceil(sy))
                };
                assert_eq!((plane.width(), plane.height()), (pw, ph));
                assert_eq!(plane.encoding().code_bits(), bits as u8);
                assert_eq!(plane.encoding().bit_shift(), 0);
                assert_eq!(
                    plane.encoding().storage(),
                    if bits == 8 {
                        ChannelType::U8
                    } else {
                        ChannelType::U16
                    }
                );
                let scale = 1 << (bits - 8);
                let (lo, hi) = if full {
                    (0, (1 << bits) - 1)
                } else {
                    (
                        16 * scale,
                        if component == 0 {
                            235 * scale
                        } else {
                            240 * scale
                        },
                    )
                };
                for y in 0..ph {
                    assert_eq!(
                        plane.row(y).unwrap().len(),
                        pw,
                        "padding must stay outside visible rows"
                    );
                    for x in 0..pw {
                        let level = if x == 0 && y == 0 {
                            if f % 2 == 0 { 0 } else { 256 }
                        } else {
                            (17 * x + 29 * y + 43 * f + 71 * component) % 257
                        };
                        let expected = lo + ((hi - lo) * level + 128) / 256;
                        assert_eq!(
                            plane.code(x, y).unwrap(),
                            expected as u16,
                            "{} frame {f} plane {component} ({x},{y})",
                            case["id"]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn mapped_guard_can_outlive_progress_on_the_same_decoder() {
    let data = include_bytes!("../corpus/av1/av1-12-444-full-17x13.ivf");
    let mut decoder =
        Av1IvfDecoder::new(data.as_slice(), 1 << 20, rav1d_safe::Settings::default()).unwrap();
    let first = decoder.next_frame().unwrap().unwrap();
    let mapped = first.map();
    let plane = mapped.plane(0).unwrap().unwrap();
    let Samples::U16(row) = plane.row(0).unwrap() else {
        panic!("12-bit data was narrowed")
    };
    let pointer = row.as_ptr();
    let original = row.to_vec();
    let second = decoder.next_frame().unwrap().unwrap();
    assert_eq!(second.timestamp().unwrap().ticks(), 1);
    drop(decoder);
    assert_eq!(row.as_ptr(), pointer);
    assert_eq!(row, original);
    assert_eq!(plane.encoding().code_bits(), 12);
}

#[test]
fn network_truncation_is_not_clean_end_of_stream() {
    let data = include_bytes!("../corpus/av1/av1-8-420-narrow-17x13.ivf");
    let mut decoder = Av1IvfDecoder::new(
        &data[..data.len() - 1],
        1 << 20,
        rav1d_safe::Settings::default(),
    )
    .unwrap();
    for _ in 0..4 {
        match decoder.next_frame() {
            Ok(Some(_)) => {}
            Ok(None) => panic!("truncation must not produce EOS"),
            Err(_) => {
                assert!(decoder.next_frame().is_err());
                return;
            }
        }
    }
    panic!("truncated last frame was accepted");
}
