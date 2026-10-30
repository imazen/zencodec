//! Private bounded-fixture execution witness, not a streaming production driver.
//! Decode native AV1, apply presentation intervals, re-encode losslessly, then
//! write files with explicitly unedited tracks. Gap fill is explicitly black.
#[allow(dead_code)]
#[path = "../src/fixtures.rs"]
mod fixtures;
use fixtures::*;
use presentation_map_experiment::{
    Acquisition, Content, Cursor, Edit, Extent, Interval, Plan, Time,
};
use std::{path::PathBuf, sync::Arc};
use zencodec::time::{FrameDuration, Timestamp};
use zencodec_media::{
    container::{MuxHint, Muxer},
    frame::{Frame, FrameTiming, OwnedPlane, OwnedPlanes, PlaneData, Presentation, Subsampling},
    timeline::TrackTimeline,
    track::TrackInfo,
};

fn encode(
    settings: zenavif::media::Av1EncodeSettings,
    frames: &[Frame],
) -> (
    zencodec_media::session::CodecConfig,
    Vec<zencodec_media::packet::Packet>,
) {
    use zencodec_media::session::{VideoEncoder, drain_encoder, send_frame_until_accepted};
    let mut encoder = zenavif::media::Av1VideoEncoder::new(settings, TB).unwrap();
    let config = encoder.output_config().unwrap().clone();
    let mut packets = Vec::new();
    for frame in frames {
        send_frame_until_accepted(&mut encoder, frame, None, |packet| {
            packets.push(packet);
            Ok::<(), core::convert::Infallible>(())
        })
        .unwrap();
    }
    drain_encoder(&mut encoder, None, |packet| {
        packets.push(packet);
        Ok::<(), core::convert::Infallible>(())
    })
    .unwrap();
    (config, packets)
}

// The fixture admits replay from its already-owned compressed packet snapshot.
// Each segment creates a fresh native decoder; decoded pictures are handed
// immediately to the encoder, without a driver-owned raster cache.
fn replay_encode(
    settings: zenavif::media::Av1EncodeSettings,
    source_config: &zencodec_media::session::CodecConfig,
    source_packets: &[zencodec_media::packet::Packet],
    plan: &Plan,
    black: &Frame,
) -> (
    zencodec_media::session::CodecConfig,
    Vec<zencodec_media::packet::Packet>,
) {
    use zencodec_media::session::{
        VideoDecoder, VideoEncoder, drain_decoder, drain_encoder, send_frame_until_accepted,
        send_packet_until_accepted,
    };
    let mut encoder = zenavif::media::Av1VideoEncoder::new(settings, TB).unwrap();
    let output_config = encoder.output_config().unwrap().clone();
    let mut packets = Vec::new();
    let mut timings = Vec::new();
    let mut pictures = Vec::new(); // Weak references only, for the cleanup control.
    for segment in plan.segments() {
        let mut emit = |frame: &Frame, span: Interval| {
            let start = span.start().on_clock(TB).unwrap();
            let end = span.end().on_clock(TB).unwrap();
            let timing = FrameTiming::at(start)
                .with_duration_ticks((end.ticks() - start.ticks()).try_into().unwrap());
            let projected = frame.represent(timing, Presentation::default());
            assert!(projected.shares_picture(frame));
            timings.push(timing);
            send_frame_until_accepted(&mut encoder, &projected, None, |packet| {
                packets.push(packet);
                Ok::<(), core::convert::Infallible>(())
            })
            .unwrap();
        };
        if segment.acquisition() == Acquisition::Gap {
            emit(
                black,
                Interval::new(segment.output.start, segment.output.end.unwrap()).unwrap(),
            );
            continue;
        }
        let mut settings = rav1d_safe::Settings::default();
        settings.threads = 1;
        let mut decoder = zenavif::media::Av1VideoDecoder::with_settings(settings).unwrap();
        decoder.configure(source_config, None).unwrap();
        let mut cursor = Cursor::new(*segment).unwrap();
        let complete = std::cell::Cell::new(false);
        let mut on_frame = |frame: Frame| {
            pictures.push(Arc::downgrade(frame.picture()));
            if complete.get() {
                return Ok::<(), core::convert::Infallible>(());
            }
            let start = frame.timing().pts.unwrap();
            // The source fixture, not PTS alone, guarantees each duration.
            let end = Timestamp::new(start.ticks() + 40, TB);
            if let Some(span) = cursor
                .push(Interval::new(Time::timestamp(start), Time::timestamp(end)).unwrap())
                .unwrap()
            {
                emit(&frame, span);
            }
            complete.set(cursor.coverage_complete());
            Ok::<(), core::convert::Infallible>(())
        };
        for packet in source_packets {
            send_packet_until_accepted(&mut decoder, &mut packet.clone(), None, &mut on_frame)
                .unwrap();
            if complete.get() {
                break;
            }
        }
        // This generated fixture has no outstanding source metadata obligation.
        // Full coverage is not a general permission to skip container inspection.
        if !complete.get() {
            drain_decoder(&mut decoder, None, &mut on_frame).unwrap();
        }
        cursor.finish().unwrap();
        // No decoded frame owner crosses into the next acquisition in this driver.
    }
    drain_encoder(&mut encoder, None, |packet| {
        packets.push(packet);
        Ok::<(), core::convert::Infallible>(())
    })
    .unwrap();
    drop(encoder);
    assert!(pictures.iter().all(|picture| picture.upgrade().is_none()));
    assert_eq!(packets.len(), timings.len());
    for (packet, timing) in packets.iter().zip(timings) {
        assert_eq!(packet.pts(), timing.pts);
        assert_eq!(packet.duration_ticks(), timing.duration_ticks);
    }
    (output_config, packets)
}

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("NEW_OUTPUT_DIRECTORY required"),
    );
    let replay = std::env::args().skip(2).any(|arg| arg == "--replay-source");
    std::fs::create_dir(&output).unwrap();
    let settings = settings(8, Subsampling::Yuv444, true);
    let original: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
    let (config, packets) = encode(settings.clone(), &original);
    let mut native_settings = rav1d_safe::Settings::default();
    native_settings.threads = 1;
    let mut decoder = zenavif::media::Av1VideoDecoder::with_settings(native_settings).unwrap();
    let decoded = decode(&mut decoder, &config, &packets, None).unwrap();
    assert_eq!(decoded.len(), 3);
    for (actual, expected) in decoded.iter().zip(&original) {
        assert_eq!(samples(actual), samples(expected));
    }
    let black = Frame::new(
        Arc::new(
            OwnedPlanes::new(
                (0..3)
                    .map(|_| OwnedPlane {
                        data: PlaneData::U8(vec![0; 17 * 13]),
                        width: 17,
                        height: 13,
                        stride_bytes: 17,
                    })
                    .collect(),
                decoded[0].interpretation().encoding,
            )
            .unwrap(),
        ),
        decoded[0].interpretation().clone(),
        FrameTiming::at(Timestamp::new(0, TB)),
        Presentation::default(),
    );
    drop(original);
    drop(decoder);
    let decoded = if replay {
        drop(decoded);
        None
    } else {
        Some(decoded)
    };
    let media = |duration, start, rate| Edit {
        extent: Extent::Finite(FrameDuration::new(duration, 1000).unwrap()),
        content: Content::Media {
            start: Timestamp::new(start, TB),
            rate_num: rate,
            rate_den: 1,
        },
    };
    let programs = [
        ("identity", vec![media(120, 0, 1)]),
        ("clip", vec![media(40, 40, 1)]),
        ("repeat", vec![media(40, 0, 1), media(40, 0, 1)]),
        (
            "gap",
            vec![
                Edit {
                    extent: Extent::Finite(FrameDuration::new(40, 1000).unwrap()),
                    content: Content::Gap,
                },
                media(80, 0, 1),
            ],
        ),
        ("dwell", vec![media(80, 40, 0)]),
        ("double", vec![media(60, 0, 2)]),
    ];
    for (name, edits) in programs {
        let plan = Plan::new(Timestamp::new(0, TB), &edits).unwrap();
        let (encoded_config, encoded) = if replay {
            replay_encode(settings.clone(), &config, &packets, &plan, &black)
        } else {
            let decoded = decoded.as_ref().unwrap();
            let mut rendered = Vec::new();
            for segment in plan.segments() {
                let mut emit = |frame: &Frame, span: Interval| {
                    let start = span.start().on_clock(TB).unwrap();
                    let end = span.end().on_clock(TB).unwrap();
                    let timing = FrameTiming::at(start)
                        .with_duration_ticks((end.ticks() - start.ticks()).try_into().unwrap());
                    let projected = frame.represent(timing, Presentation::default());
                    assert!(projected.shares_picture(frame));
                    rendered.push(projected);
                };
                if segment.acquisition() == Acquisition::Gap {
                    emit(
                        &black,
                        Interval::new(segment.output.start, segment.output.end.unwrap()).unwrap(),
                    );
                } else {
                    let mut cursor = Cursor::new(*segment).unwrap();
                    for frame in decoded {
                        let start = frame.timing().pts.unwrap();
                        let end = Timestamp::new(start.ticks() + 40, TB);
                        if let Some(span) = cursor
                            .push(
                                Interval::new(Time::timestamp(start), Time::timestamp(end))
                                    .unwrap(),
                            )
                            .unwrap()
                        {
                            emit(frame, span);
                        }
                    }
                    cursor.finish().unwrap();
                }
            }
            let (encoded_config, encoded) = encode(settings.clone(), &rendered);
            assert_eq!(encoded.len(), rendered.len());
            for (packet, frame) in encoded.iter().zip(&rendered) {
                assert_eq!(packet.pts(), frame.timing().pts);
                assert_eq!(packet.duration_ticks(), frame.timing().duration_ticks);
            }
            (encoded_config, encoded)
        };
        for fragmented in [false, true] {
            let mut track = TrackInfo::video(
                0,
                encoded_config.codec.clone(),
                encoded_config.time_base,
                encoded_config.video.unwrap(),
            )
            .with_timeline(TrackTimeline::Unedited);
            if let Some(private) = &encoded_config.private {
                track = track.with_codec_private(private.clone());
            }
            let mut mux = if fragmented {
                zenmp4::Mp4Muxer::fragmented()
            } else {
                zenmp4::Mp4Muxer::new()
            };
            let tracks = [track.build().unwrap()];
            if fragmented {
                mux.begin_exact_presentation(&tracks, MuxHint::forward_only())
                    .unwrap();
            } else {
                mux.begin(&tracks, MuxHint::default()).unwrap();
            }
            let mut bytes = Vec::new();
            for packet in &encoded {
                mux.write_packet(packet.clone(), None).unwrap();
                mux.take_output(&mut bytes);
            }
            mux.finish(None).unwrap().apply_to(&mut bytes).unwrap();
            std::fs::write(
                output.join(format!(
                    "{name}-{}.mp4",
                    if fragmented {
                        "fragmented"
                    } else {
                        "progressive"
                    }
                )),
                bytes,
            )
            .unwrap();
        }
    }
}
