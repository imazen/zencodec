#![cfg(feature = "av1-decode")]
use std::io::Cursor;
use zencodec_media::{
    av1::Av1IvfDecoder,
    av1_index::{FrameScope, FrameSelection, IndexedAv1},
    ivf::{IvfReader, IvfWriter},
    time::{TimeBase, Timestamp},
};

fn settings() -> rav1d_safe::Settings {
    let mut s = rav1d_safe::Settings::default();
    s.all_layers = false;
    s
}
fn index(bytes: Vec<u8>) -> IndexedAv1<Cursor<Vec<u8>>> {
    IndexedAv1::build(Cursor::new(bytes), 1 << 20, 100, 1 << 16, settings(), None).unwrap()
}
fn expected(
    times: &[i64],
    request: i64,
    denominator: i64,
    mode: FrameSelection,
    key: bool,
) -> Option<usize> {
    let candidates = times
        .iter()
        .enumerate()
        .filter(|&(i, _)| !key || i % 2 == 0);
    match mode {
        FrameSelection::Nearest => candidates
            .min_by_key(|&(i, &t)| ((t * denominator - request).unsigned_abs(), t, i))
            .map(|(i, _)| i),
        FrameSelection::AtOrBefore => candidates
            .filter(|&(_, &t)| t * denominator <= request)
            .min_by_key(|&(i, &t)| (-t, i))
            .map(|(i, _)| i),
        FrameSelection::AtOrAfter => candidates
            .filter(|&(_, &t)| t * denominator >= request)
            .min_by_key(|&(i, &t)| (t, i))
            .map(|(i, _)| i),
        _ => unreachable!(),
    }
}

#[test]
fn all_native_layouts_extract_exact_nearest_directional_and_key_pictures() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../corpus/av1-manifest.json")).unwrap();
    let clock = TimeBase::new(1001, 30000).unwrap();
    let quarter = TimeBase::new(1001, 120000).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("corpus")
                .join(case["path"].as_str().unwrap()),
        )
        .unwrap();
        let mut source = index(bytes);
        assert_eq!(source.presentation_count(), 4);
        assert_eq!(source.keyframe_count(), 2);
        for scope in [FrameScope::All, FrameScope::Keyframes] {
            for mode in [
                FrameSelection::Nearest,
                FrameSelection::AtOrBefore,
                FrameSelection::AtOrAfter,
            ] {
                for ticks in -2..=14 {
                    let expected = expected(
                        &[0, 1, 2, 3],
                        ticks,
                        4,
                        mode,
                        scope == FrameScope::Keyframes,
                    );
                    let got = source
                        .extract(Timestamp::new(ticks, quarter), mode, scope)
                        .unwrap();
                    assert_eq!(got.as_ref().map(|f| f.presentation_index()), expected);
                    if let Some(got) = got {
                        let n = expected.unwrap();
                        assert_eq!(
                            got.frame().timestamp(),
                            Some(Timestamp::new(n as i64, clock))
                        );
                        assert_eq!(got.frame().is_keyframe(), n.is_multiple_of(2));
                        // With one frame context, a GOP starts without reading
                        // its prefix. A retained sequence header initializes it.
                        assert!(got.packets_read() <= n % 2 + 1);
                        let mapped = got.frame().map();
                        for p in 0..if case["sampling"] == "mono" { 1 } else { 3 } {
                            let plane = mapped.plane(p).unwrap().unwrap();
                            for y in 0..plane.height() {
                                for x in 0..plane.width() {
                                    assert_eq!(
                                        plane.code(x, y).unwrap(),
                                        zencodec_media_testkit::synthetic::code(
                                            n,
                                            p,
                                            x,
                                            y,
                                            case["bits"].as_u64().unwrap() as u8,
                                            case["range"] == "full"
                                        )
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn retime(bytes: &[u8], times: &[i64], clock: TimeBase) -> Vec<u8> {
    let mut read = IvfReader::new(bytes, 1 << 20).unwrap();
    let mut write =
        IvfWriter::new(Vec::new(), read.info().width(), read.info().height(), clock).unwrap();
    for &t in times {
        let p = read.next_packet().unwrap().unwrap();
        write
            .write_packet(&p.data, Timestamp::new(t, clock))
            .unwrap();
    }
    assert!(read.next_packet().unwrap().is_none());
    write.finish().unwrap()
}

#[test]
fn nonmonotonic_duplicate_and_extreme_times_are_selected_without_float_rounding() {
    let fixture = include_bytes!("../corpus/av1/av1-10-420-full-17x13.ivf");
    let clock = TimeBase::new(1, 30).unwrap();
    let times = [100, -5, -5, 50];
    let mut source = index(retime(fixture, &times, clock));
    for key in [false, true] {
        for mode in [
            FrameSelection::Nearest,
            FrameSelection::AtOrBefore,
            FrameSelection::AtOrAfter,
        ] {
            for tick in -12..=205 {
                let expected = expected(&times, tick, 2, mode, key);
                let got = source
                    .extract(
                        Timestamp::new(tick, TimeBase::new(1, 60).unwrap()),
                        mode,
                        if key {
                            FrameScope::Keyframes
                        } else {
                            FrameScope::All
                        },
                    )
                    .unwrap();
                assert_eq!(got.map(|f| f.presentation_index()), expected);
            }
        }
    }
    let clock = TimeBase::new(u32::MAX, u32::MAX - 1).unwrap();
    let times = [i64::MIN + 1, -1, 0, i64::MAX];
    let mut source = index(retime(fixture, &times, clock));
    for (target, expected) in [(i64::MIN, 0), (i64::MAX, 3)] {
        // Cross products straddle the complete signed i128 domain: distance
        // subtraction needs u128 even though each product itself fits i128.
        let found = source
            .extract(
                Timestamp::new(target, TimeBase::new(u32::MAX - 1, u32::MAX).unwrap()),
                FrameSelection::Nearest,
                FrameScope::All,
            )
            .unwrap()
            .unwrap();
        assert_eq!(found.presentation_index(), expected);
    }
}

#[test]
fn reordered_show_existing_frames_are_real_presentations_but_not_keyframes() {
    let bytes = include_bytes!("../corpus/native-reordered-17x17-444.obu");
    let offsets = [0, 922, 2325, 2330, 2642, 2647];
    let clock = TimeBase::new(1001, 30000).unwrap();
    let mut writer = IvfWriter::new(Vec::new(), 17, 17, clock).unwrap();
    for (i, pair) in offsets.windows(2).enumerate() {
        writer
            .write_packet(&bytes[pair[0]..pair[1]], Timestamp::new(i as i64, clock))
            .unwrap();
    }
    let encoded = writer.finish().unwrap();
    let mut sequential = Av1IvfDecoder::new(encoded.as_slice(), 1 << 20, settings()).unwrap();
    let mut originals = Vec::new();
    while let Some(frame) = sequential.next_frame().unwrap() {
        originals.push(frame);
    }
    for threads in [1, 4] {
        let mut settings = settings();
        settings.threads = threads;
        settings.max_frame_delay = 4;
        let mut source = IndexedAv1::build(
            Cursor::new(encoded.clone()),
            1 << 20,
            10,
            4096,
            settings,
            None,
        )
        .unwrap();
        assert_eq!(source.keyframe_count(), 1);
        for n in [4, 0, 2, 3, 1, 4] {
            let frame = source
                .extract(
                    Timestamp::new(n as i64, clock),
                    FrameSelection::Nearest,
                    FrameScope::All,
                )
                .unwrap()
                .unwrap()
                .into_frame();
            assert_eq!(frame.timestamp(), originals[n].timestamp());
            assert_eq!(frame.is_show_existing(), matches!(n, 2 | 4));
            let mapped = frame.map();
            let original = originals[n].map();
            for p in 0..3 {
                let a = mapped.plane(p).unwrap().unwrap();
                let b = original.plane(p).unwrap().unwrap();
                for y in 0..a.height() {
                    for x in 0..a.width() {
                        assert_eq!(a.code(x, y), b.code(x, y));
                    }
                }
            }
        }
    }
}

#[test]
fn index_limits_cancellation_and_unsupported_output_modes_fail_explicitly() {
    let bytes = include_bytes!("../corpus/av1/av1-10-420-full-17x13.ivf");
    assert!(IndexedAv1::build(Cursor::new(bytes), 1 << 20, 3, 4096, settings(), None).is_err());
    assert!(IndexedAv1::build(Cursor::new(bytes), 1 << 20, 10, 1, settings(), None).is_err());
    let mut invalid = settings();
    invalid.output_invisible_frames = true;
    assert!(IndexedAv1::build(Cursor::new(bytes), 1 << 20, 10, 4096, invalid, None).is_err());
    struct Cancel;
    impl enough::Stop for Cancel {
        fn check(&self) -> Result<(), enough::StopReason> {
            Err(enough::StopReason::Cancelled)
        }
    }
    let error = IndexedAv1::build(
        Cursor::new(bytes),
        1 << 20,
        10,
        4096,
        settings(),
        Some(std::sync::Arc::new(Cancel)),
    );
    assert!(
        matches!(error,Err(zencodec_media::av1::DecodeError::Codec(ref e)) if e.error()==&rav1d_safe::Error::Cancelled)
    );
    let clock = TimeBase::new(1, 1).unwrap();
    let mut empty = index(
        IvfWriter::new(Vec::new(), 1, 1, clock)
            .unwrap()
            .finish()
            .unwrap(),
    );
    assert!(
        empty
            .extract(
                Timestamp::new(0, clock),
                FrameSelection::Nearest,
                FrameScope::All
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn seeking_supplies_sequence_initialization_when_a_later_key_packet_omits_it() {
    let fixture = include_bytes!("../corpus/av1/av1-10-420-full-17x13.ivf");
    let mut reader = IvfReader::new(fixture.as_slice(), 1 << 20).unwrap();
    let clock = reader.info().time_base();
    let mut writer = IvfWriter::new(Vec::new(), 17, 13, clock).unwrap();
    for n in 0..4 {
        let mut packet = reader.next_packet().unwrap().unwrap();
        if n == 2 {
            assert_eq!(
                &packet.data[..3],
                &[0x12, 0, 0x0a],
                "fixture temporal delimiter and sequence header"
            );
            let size = usize::from(packet.data[3]);
            assert!(size < 128);
            packet.data.drain(2..4 + size);
        }
        writer.write_packet(&packet.data, packet.timestamp).unwrap();
    }
    let mut source = index(writer.finish().unwrap());
    let found = source
        .extract(
            Timestamp::new(3, clock),
            FrameSelection::Nearest,
            FrameScope::All,
        )
        .unwrap()
        .unwrap();
    assert_eq!(found.presentation_index(), 3);
    assert_eq!(
        found.packets_read(),
        2,
        "seek should start from the later key packet"
    );
    let mapped = found.frame().map();
    let y = mapped.plane(0).unwrap().unwrap();
    assert_eq!(y.code(0, 0).unwrap(), 1023);
}
