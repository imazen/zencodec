#![cfg(all(feature = "av1-encode", feature = "av1-decode"))]

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};
use zencodec_media::{
    av1::Av1IvfDecoder,
    av1_encode::{Av1Encoder, Av1IvfEncoder, EncodeError, EncodeReceive, SubmitStatus},
    color::{ChromaLocation, Subsampling, YuvView},
    plane::{Plane, Samples},
    time::{TimeBase, Timestamp},
};
use zenpixels::{ChannelType, Cicp, sample::SampleEncoding};
use zenrav1e::{
    EncoderConfig,
    prelude::{Rational, SpeedSettings},
};

fn config(bits: u8, sampling: Subsampling, full: bool, reorder: bool) -> EncoderConfig {
    use zenrav1e::color::*;
    let mut speed = SpeedSettings::from_preset(10);
    speed.rdo_lookahead_frames = 1;
    EncoderConfig {
        width: 17,
        height: 19,
        bit_depth: bits as usize,
        chroma_sampling: match sampling {
            Subsampling::Monochrome => ChromaSampling::Cs400,
            Subsampling::Yuv420 => ChromaSampling::Cs420,
            Subsampling::Yuv422 => ChromaSampling::Cs422,
            _ => ChromaSampling::Cs444,
        },
        pixel_range: if full {
            PixelRange::Full
        } else {
            PixelRange::Limited
        },
        color_description: Some(ColorDescription {
            color_primaries: ColorPrimaries::BT709,
            transfer_characteristics: TransferCharacteristics::BT709,
            matrix_coefficients: MatrixCoefficients::BT709,
        }),
        quantizer: 0,
        min_quantizer: 0,
        low_latency: !reorder,
        min_key_frame_interval: 2,
        max_key_frame_interval: 2,
        time_base: Rational::new(1001, 30000),
        speed_settings: speed,
        ..EncoderConfig::default()
    }
}

fn dimensions(sampling: Subsampling) -> [(usize, usize); 3] {
    let chroma = match sampling {
        Subsampling::Yuv420 => (9, 10),
        Subsampling::Yuv422 => (9, 19),
        _ => (17, 19),
    };
    [(17, 19), chroma, chroma]
}
fn pixels(frame: usize, bits: u8, sampling: Subsampling, full: bool, shift: u8) -> [Vec<u16>; 3] {
    let sizes = dimensions(sampling);
    std::array::from_fn(|p| {
        let (w, h) = sizes[p];
        (0..w * h)
            .map(|i| {
                zencodec_media_testkit::synthetic::code(frame, p, i % w, i / w, bits, full) << shift
            })
            .collect()
    })
}
fn view<'a>(
    data: &'a [Vec<u16>; 3],
    bits: u8,
    sampling: Subsampling,
    full: bool,
    shift: u8,
) -> YuvView<'a> {
    let sizes = dimensions(sampling);
    let planes: [_; 3] = std::array::from_fn(|p| {
        let (w, h) = sizes[p];
        Plane::new(
            Samples::U16(&data[p]),
            w,
            h,
            w * 2,
            SampleEncoding::new(ChannelType::U16, bits, shift).unwrap(),
        )
        .unwrap()
    });
    YuvView::new(
        planes[0],
        (sampling != Subsampling::Monochrome).then_some([planes[1], planes[2]]),
        sampling,
        ChromaLocation::Unknown,
        Cicp::new(1, 1, 1, full),
    )
    .unwrap()
}

#[test]
fn streaming_lossless_roundtrip_preserves_native_precision_color_and_variable_pts() {
    let clock = TimeBase::new(1, 90000).unwrap();
    let times = [-9009, -3003, 0, 3003, 12012, 12013, 30030, 45045];
    for bits in [8, 10, 12] {
        for sampling in [
            Subsampling::Monochrome,
            Subsampling::Yuv420,
            Subsampling::Yuv422,
            Subsampling::Yuv444,
        ] {
            for full in [false, true] {
                let mut encoder = Av1IvfEncoder::new(
                    Vec::new(),
                    config(bits, sampling, full, false),
                    clock,
                    32,
                    1,
                )
                .unwrap();
                for (n, ticks) in times.into_iter().enumerate() {
                    let shift = if n.is_multiple_of(2) { 0 } else { 16 - bits };
                    let source = pixels(n, bits, sampling, full, shift);
                    encoder
                        .push(
                            view(&source, bits, sampling, full, shift),
                            Timestamp::new(ticks, clock),
                        )
                        .unwrap();
                    assert!(encoder.queued_frames() <= 32);
                }
                assert!(encoder.packets_written() > 0, "must emit before finish");
                let encoded = encoder.finish().unwrap();
                let mut decoder = Av1IvfDecoder::new(
                    encoded.as_slice(),
                    1 << 20,
                    rav1d_safe::Settings::default(),
                )
                .unwrap();
                for (n, ticks) in times.into_iter().enumerate() {
                    let decoded = decoder.next_frame().unwrap().expect("presentation");
                    assert_eq!(decoded.timestamp(), Some(Timestamp::new(ticks, clock)));
                    assert_eq!(decoded.color(), Cicp::new(1, 1, 1, full));
                    let mapped = decoded.map();
                    let native = mapped.yuv_view().unwrap();
                    assert_eq!(native.subsampling(), sampling);
                    assert_eq!(native.bit_depth(), bits);
                    for p in 0..if sampling == Subsampling::Monochrome {
                        1
                    } else {
                        3
                    } {
                        let plane = mapped.plane(p).unwrap().unwrap();
                        for y in 0..plane.height() {
                            for x in 0..plane.width() {
                                assert_eq!(
                                    plane.code(x, y).unwrap(),
                                    zencodec_media_testkit::synthetic::code(n, p, x, y, bits, full),
                                    "{bits} {sampling:?} full={full}, frame{n}, p{p}, {x},{y}"
                                );
                            }
                        }
                    }
                }
                assert!(decoder.next_frame().unwrap().is_none());
            }
        }
    }
}

#[test]
fn delayed_packets_keep_the_input_identity_and_drain_exactly_once() {
    let clock = TimeBase::new(1001, 30000).unwrap();
    let sampling = Subsampling::Yuv420;
    let mut cfg = config(10, sampling, true, true);
    cfg.min_key_frame_interval = 16;
    cfg.max_key_frame_interval = 16;
    let mut encoder = Av1Encoder::new(cfg, clock, 32, 1).unwrap();
    let mut out = Vec::new();
    for n in 0..21 {
        let source = pixels(n, 10, sampling, true, 0);
        assert_eq!(
            encoder
                .submit(
                    view(&source, 10, sampling, true, 0),
                    Timestamp::new((n * n) as i64 - 30, clock)
                )
                .unwrap(),
            SubmitStatus::Accepted
        );
        loop {
            match encoder.receive().unwrap() {
                EncodeReceive::Packet(p) => out.push(p),
                EncodeReceive::NeedInput => break,
                _ => panic!("premature EOS"),
            }
        }
    }
    encoder.end_input().unwrap();
    encoder.end_input().unwrap();
    loop {
        match encoder.receive().unwrap() {
            EncodeReceive::Packet(p) => out.push(p),
            EncodeReceive::EndOfStream => break,
            _ => panic!("drain needs input"),
        }
    }
    assert_eq!(out.len(), 21);
    assert_eq!(encoder.queued_frames(), 0);
    assert!(matches!(
        encoder.receive().unwrap(),
        EncodeReceive::EndOfStream
    ));
    for (i, p) in out.iter().enumerate() {
        assert_eq!(p.input_index(), i as u64);
        assert_eq!(p.timestamp(), Timestamp::new((i * i) as i64 - 30, clock));
    }
    let source = pixels(0, 10, sampling, true, 0);
    assert!(matches!(
        encoder.submit(
            view(&source, 10, sampling, true, 0),
            Timestamp::new(500, clock)
        ),
        Err(EncodeError::InputEnded)
    ));
}

#[test]
fn packed_image_to_native_video_preserves_converted_codes_and_identity_rgb() {
    use zencodec_media::{
        color::YuvToRgb,
        display::OutOfRange,
        encode_color::{AlphaHandling, RgbToYuv},
    };
    use zenpixels::{PixelDescriptor, PixelSlice};
    use zenrav1e::color::{ChromaSamplePosition, MatrixCoefficients, TransferCharacteristics};
    let source: Vec<_> = (0..17 * 19 * 3)
        .map(|i| ((i * 37 + i / 7 * 83) % 256) as u8)
        .collect();
    let input = PixelSlice::new(&source, 17, 19, 17 * 3, PixelDescriptor::RGB8_SRGB).unwrap();
    let clock = TimeBase::new(1, 30).unwrap();
    for bits in [8, 10, 12] {
        for full in [false, true] {
            for (sampling, location, matrix) in [
                (Subsampling::Yuv444, ChromaLocation::Unknown, 0),
                (Subsampling::Yuv420, ChromaLocation::Left, 1),
                (Subsampling::Yuv420, ChromaLocation::TopLeft, 1),
            ] {
                // AV1 mandates full range for sRGB/BT.709/identity signaling.
                if matrix == 0 && !full {
                    continue;
                }
                let color = Cicp::new(1, 13, matrix, full);
                let conversion =
                    RgbToYuv::new(bits, sampling, location, color, OutOfRange::Reject).unwrap();
                let native = conversion
                    .convert(&input, AlphaHandling::RequireOpaque, 10000, None)
                    .unwrap();
                let mut cfg = config(bits, sampling, full, false);
                let description = cfg.color_description.as_mut().unwrap();
                description.transfer_characteristics = TransferCharacteristics::SRGB;
                description.matrix_coefficients = if matrix == 0 {
                    MatrixCoefficients::Identity
                } else {
                    MatrixCoefficients::BT709
                };
                cfg.chroma_sample_position = match location {
                    ChromaLocation::Left => ChromaSamplePosition::Vertical,
                    ChromaLocation::TopLeft => ChromaSamplePosition::Colocated,
                    _ => ChromaSamplePosition::Unknown,
                };
                let mut encoder = Av1IvfEncoder::new(Vec::new(), cfg, clock, 32, 1).unwrap();
                encoder
                    .push(native.view(), Timestamp::new(17, clock))
                    .unwrap();
                let bytes = encoder.finish().unwrap();
                let mut decoder =
                    Av1IvfDecoder::new(bytes.as_slice(), 1 << 20, rav1d_safe::Settings::default())
                        .unwrap();
                let frame = decoder.next_frame().unwrap().unwrap();
                assert_eq!(frame.timestamp(), Some(Timestamp::new(17, clock)));
                assert_eq!(frame.color(), color);
                let mapped = frame.map();
                let decoded = mapped.yuv_view().unwrap();
                for c in 0..3 {
                    let expected = native.view().plane(c).unwrap();
                    let actual = decoded.plane(c).unwrap();
                    for y in 0..actual.height() {
                        for x in 0..actual.width() {
                            assert_eq!(
                                actual.code(x, y).unwrap(),
                                expected.code(x, y).unwrap(),
                                "bits{bits} full{full} {sampling:?} {location:?}"
                            );
                        }
                    }
                }
                if bits == 8 && matrix == 0 {
                    let rgb = YuvToRgb::new(decoded).unwrap();
                    let mut row = [[0.0; 3]; 17];
                    for y in 0..19 {
                        rgb.write_row(y, &mut row).unwrap();
                        for x in 0..17 {
                            for c in 0..3 {
                                assert_eq!(
                                    (row[x][c] * 255.0).round() as u8,
                                    source[(y * 17 + x) * 3 + c]
                                );
                            }
                        }
                    }
                }
                assert!(decoder.next_frame().unwrap().is_none());
            }
        }
    }
}

#[derive(Clone)]
struct Sink {
    bytes: Arc<Mutex<Vec<u8>>>,
    fail_after: usize,
    fail_flush: bool,
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut output = self.bytes.lock().unwrap();
        if output.len() >= self.fail_after {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "test disconnect"));
        }
        let n = bytes.len().min(7).min(self.fail_after - output.len());
        output.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            Err(io::Error::other("test flush"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn nonseekable_sink_reports_partial_write_and_finalization_errors() {
    let clock = TimeBase::new(1, 30).unwrap();
    let sampling = Subsampling::Yuv420;
    for (fail_after, fail_flush) in [(usize::MAX, false), (70, false), (usize::MAX, true)] {
        let sink = Sink {
            bytes: Arc::default(),
            fail_after,
            fail_flush,
        };
        let observed = sink.bytes.clone();
        let mut enc =
            Av1IvfEncoder::new(sink, config(8, sampling, true, false), clock, 32, 1).unwrap();
        let mut failed = false;
        for n in 0..8 {
            let source = pixels(n, 8, sampling, true, 0);
            let view = view(&source, 8, sampling, true, 0);
            match enc.push(view, Timestamp::new(n as i64, clock)) {
                Ok(()) => {}
                Err(EncodeError::Io(e)) => {
                    assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
                    assert!(matches!(
                        enc.push(view, Timestamp::new(100, clock)),
                        Err(EncodeError::FailedStream)
                    ));
                    failed = true;
                    break;
                }
                Err(e) => panic!("{e}"),
            }
        }
        let result = enc.finish();
        if failed {
            assert!(matches!(result, Err(EncodeError::FailedStream)));
        } else if fail_flush {
            assert!(matches!(result, Err(EncodeError::Io(_))));
        } else {
            assert!(result.is_ok());
            assert!(observed.lock().unwrap().len() > 32);
        }
        if fail_after == 70 {
            assert!(failed);
            assert_eq!(observed.lock().unwrap().len(), 70);
        }
    }
}

#[test]
fn validation_is_retryable_and_queue_budget_cannot_deadlock() {
    let sampling = Subsampling::Yuv420;
    let clock = TimeBase::new(1, 30).unwrap();
    let mut enc = Av1Encoder::new(config(8, sampling, true, false), clock, 1, 1).unwrap();
    let source = pixels(0, 8, sampling, true, 0);
    let frame = view(&source, 8, sampling, true, 0);
    assert!(matches!(
        enc.submit(frame, Timestamp::new(0, TimeBase::new(1, 60).unwrap())),
        Err(EncodeError::ClockMismatch)
    ));
    assert_eq!(enc.queued_frames(), 0);
    assert!(matches!(
        enc.submit(frame, Timestamp::new(i64::MIN, clock)),
        Err(EncodeError::ReservedTimestamp)
    ));
    assert_eq!(
        enc.submit(frame, Timestamp::new(-1, clock)).unwrap(),
        SubmitStatus::Accepted
    );
    assert!(matches!(
        enc.submit(frame, Timestamp::new(-2, clock)),
        Err(EncodeError::NonmonotonicTimestamp)
    ));
    assert_eq!(
        enc.submit(frame, Timestamp::new(0, clock)).unwrap(),
        SubmitStatus::ReceivePending
    );
    assert!(matches!(
        enc.receive(),
        Err(EncodeError::QueueLimitInsufficient)
    ));
    assert!(matches!(enc.receive(), Err(EncodeError::FailedStream)));

    let mut muxed =
        Av1IvfEncoder::new(Vec::new(), config(8, sampling, true, false), clock, 32, 1).unwrap();
    assert!(matches!(
        muxed.push(frame, Timestamp::new(0, TimeBase::new(1, 60).unwrap())),
        Err(EncodeError::ClockMismatch)
    ));
    muxed.push(frame, Timestamp::new(-1, clock)).unwrap();
    let bytes = muxed.finish().unwrap();
    let mut decoded =
        Av1IvfDecoder::new(bytes.as_slice(), 1 << 20, rav1d_safe::Settings::default()).unwrap();
    assert_eq!(
        decoded.next_frame().unwrap().unwrap().timestamp(),
        Some(Timestamp::new(-1, clock))
    );
    assert!(decoded.next_frame().unwrap().is_none());
}

#[test]
fn cancellation_is_typed_and_empty_stream_drains() {
    let clock = TimeBase::new(1, 30).unwrap();
    let sampling = Subsampling::Yuv420;
    let mut empty = Av1Encoder::new(config(8, sampling, true, false), clock, 32, 1).unwrap();
    empty.end_input().unwrap();
    assert!(matches!(
        empty.receive().unwrap(),
        EncodeReceive::EndOfStream
    ));
    let mut encoder = Av1Encoder::new(config(8, sampling, true, false), clock, 32, 1).unwrap();
    struct Cancel(std::sync::atomic::AtomicBool);
    impl zenrav1e::Stop for Cancel {
        fn check(&self) -> Result<(), zenrav1e::StopReason> {
            if self.0.load(std::sync::atomic::Ordering::Relaxed) {
                Err(zenrav1e::StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }
    let cancel = Arc::new(Cancel(std::sync::atomic::AtomicBool::new(false)));
    encoder.set_stop(cancel.clone());
    let source = pixels(0, 8, sampling, true, 0);
    encoder
        .submit(
            view(&source, 8, sampling, true, 0),
            Timestamp::new(0, clock),
        )
        .unwrap();
    encoder.end_input().unwrap();
    cancel.0.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(matches!(
        encoder.receive(),
        Err(EncodeError::Backend(zenrav1e::EncoderStatus::Cancelled))
    ));
    assert!(matches!(encoder.receive(), Err(EncodeError::FailedStream)));
}
