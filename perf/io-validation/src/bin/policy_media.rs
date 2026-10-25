//! Native-container integration; invoke with actual MP4/WebM/IVF fixtures.
#![forbid(unsafe_code)]
#[path = "../policy/mod.rs"]
mod policy;
#[path = "../policy_media.rs"]
mod selection;
#[allow(dead_code)]
#[path = "../transport_model.rs"]
mod transport;
use policy::*;
use std::{cell::RefCell, collections::BTreeMap, convert::Infallible, rc::Rc};
use zencodec::{
    At, Unstoppable,
    source::{ReadAt, SliceReadAt},
};
use zencodec_io::{DriveOptions, FileReadAt, ReadAtCursor};
use zencodec_media::{
    container::{DemuxEvent, DemuxStage, Demuxer},
    limits::MediaLimits,
};

fn read<D: Demuxer, S: ReadAt>(
    mut demux: D,
    source: &mut S,
    read_size: usize,
    may_drop: bool,
) -> (Vec<String>, usize, usize) {
    let mut cursor = ReadAtCursor::new(DriveOptions::new(0, read_size));
    let mut engine = Engine::new(PolicySet::default(), 64);
    let mut selections = BTreeMap::new();
    let mut output = Vec::new();
    let mut callbacks = 0;
    let mut dropped = 0;
    let mut resolver = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
        callbacks += 1;
        let Question::Media { facts, choices } = q else {
            panic!("unexpected question")
        };
        if facts.audio && may_drop {
            Ok(Decision::choose(
                choices
                    .iter()
                    .find(|c| c.value.action == Action::Media(MediaAction::DropTrack))
                    .unwrap(),
            ))
        } else {
            Ok(Decision::UsePreset)
        }
    };
    loop {
        let step = cursor
            .next(source, &mut DemuxStage(&mut demux), &Unstoppable)
            .unwrap();
        let done = step.is_finished();
        match step.into_inner() {
            DemuxEvent::Packet(packet) => {
                let key = (packet.track(), packet.config_epoch());
                if let std::collections::btree_map::Entry::Vacant(v) = selections.entry(key) {
                    let track = demux
                        .tracks()
                        .iter()
                        .find(|t| t.index() == packet.track())
                        .unwrap();
                    v.insert(
                        selection::select_track(
                            &mut engine,
                            track,
                            &packet,
                            1,
                            may_drop,
                            &mut resolver,
                            &Unstoppable,
                        )
                        .unwrap(),
                    );
                }
                let selected = selections.get(&key).unwrap();
                assert!(selected.applies(1, &packet));
                assert!(!selected.applies(2, &packet));
                if selected.action == MediaAction::KeepTrack {
                    output.push(format!("{packet:?}"));
                } else {
                    dropped += 1;
                }
            }
            DemuxEvent::Headers => output.push(format!("{:?}", demux.tracks())),
            event => output.push(format!("{event:?}")),
        }
        if done {
            break;
        }
    }
    assert_eq!(
        callbacks,
        selections.len(),
        "policy is per track/epoch, not per packet"
    );
    (output, callbacks, dropped)
}
fn check<D: Demuxer>(path: &str, make: impl Fn() -> D) {
    let data = std::fs::read(path).unwrap();
    let expected = read(make(), &mut SliceReadAt::new(&data), 65536, false);
    assert!(
        expected.0.len() > expected.1 + 1,
        "fixture must contain multiple packets"
    );
    let cfg = transport::Config {
        ranges: true,
        cache_bytes: 65536,
        latency_ms: 120.,
        bytes_per_ms: 12500.,
        concurrency: 8,
        merge_gap: 4096,
        max_range: 65536,
        part_size: 0,
        shared_cache: true,
    };
    for read_size in [7, 65536] {
        let got = read(
            make(),
            &mut FileReadAt::open(path).unwrap(),
            read_size,
            false,
        );
        assert_eq!(got, expected);
        let ledger = Rc::new(RefCell::new(transport::Ledger::default()));
        let mut net = transport::Transport::new(
            SliceReadAt::new(&data),
            "immutable-fixture-revision-1".into(),
            cfg,
            ledger.clone(),
        )
        .unwrap();
        let got = read(make(), &mut net, read_size, false);
        assert_eq!(got, expected);
        assert!(ledger.borrow().transferred_bytes > 0);
        println!(
            "{} read_size={} events={} decisions={} network_bytes={} parity=true",
            std::path::Path::new(path)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap(),
            read_size,
            got.0.len(),
            got.1,
            ledger.borrow().transferred_bytes
        );
    }
    let omitted = read(make(), &mut SliceReadAt::new(&data), 65536, true);
    assert_eq!(omitted.0.len() + omitted.2, expected.0.len());
    println!(
        "{} explicitly_dropped_audio_packets={}",
        std::path::Path::new(path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap(),
        omitted.2
    );
}
fn main() {
    let paths: Vec<_> = std::env::args().skip(1).collect();
    assert!(!paths.is_empty(), "provide native media fixtures");
    for path in paths {
        match std::path::Path::new(&path)
            .extension()
            .and_then(|x| x.to_str())
        {
            Some("mp4") => check(&path, || zenmp4::Mp4Demuxer::new(MediaLimits::default())),
            Some("webm" | "mkv") => {
                check(&path, || zenwebm::WebmDemuxer::new(MediaLimits::default()))
            }
            Some("ivf") => check(&path, || zenivf::IvfDemuxer::new(MediaLimits::default())),
            _ => panic!("unsupported fixture extension"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zencodec::time::TimeBase;
    use zencodec_media::{
        packet::Packet,
        session::{CodecConfig, Receive, VideoDecoder},
        track::{CodecId, CodecTag, TrackInfo, VideoTrackInfo},
    };
    use zencodec_media_testkit::toy::{ToyDecoder, toy_payload};

    #[test]
    fn epoch_decisions_do_not_relabel_retained_frames_or_unknown_facts() {
        let time = TimeBase::new(1, 25).unwrap();
        let codec = CodecId::Other(CodecTag::new("toy"));
        let track = TrackInfo::video(0, codec.clone(), time, VideoTrackInfo::new(2, 2))
            .build()
            .unwrap();
        let mut first = Packet::from_vec(toy_payload(&[&[9; 4]], false)).unwrap();
        let mut engine = Engine::new(PolicySet::default(), 8);
        let mut callback = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Media { facts, .. } = q else {
                panic!()
            };
            assert_eq!(facts.random_access, None);
            assert!(!facts.first_pts_known);
            Ok(Decision::UsePreset)
        };
        let selected = selection::select_track(
            &mut engine,
            &track,
            &first,
            1,
            false,
            &mut callback,
            &Unstoppable,
        )
        .unwrap();
        let mut decoder = ToyDecoder::new(2);
        let config = CodecConfig::new(codec, time).with_video(VideoTrackInfo::new(2, 2));
        decoder.configure(&config, &Unstoppable).unwrap();
        decoder.send(&mut first, &Unstoppable).unwrap();
        decoder.end_input().unwrap();
        let Receive::Ready(retained) = decoder.receive(&Unstoppable).unwrap() else {
            panic!()
        };
        decoder.reset().unwrap();
        decoder
            .configure(&config.with_epoch(1), &Unstoppable)
            .unwrap();
        let next = Packet::from_vec(toy_payload(&[&[4; 4]], false))
            .unwrap()
            .with_config_epoch(1);
        assert!(!selected.applies(1, &next));
        assert!(!selected.applies(2, &next));
        let next_selection = selection::select_track(
            &mut engine,
            &track,
            &next,
            1,
            false,
            &mut callback,
            &Unstoppable,
        )
        .unwrap();
        assert!(next_selection.applies(1, &next));
        drop(decoder);
        assert_eq!(retained.interpretation().config_epoch, 0);
        assert_eq!(
            retained.map().unwrap().plane(0).unwrap().code(1, 1),
            Some(9)
        );
        assert_eq!(engine.records().count(), 2);
    }

    #[test]
    fn required_tracks_cannot_be_dropped_and_cancellation_keeps_the_packet() {
        let track = TrackInfo::video(
            0,
            CodecId::Av1,
            TimeBase::new(1, 25).unwrap(),
            VideoTrackInfo::new(2, 2),
        )
        .build()
        .unwrap();
        let packet = Packet::from_vec(vec![1, 2, 3]).unwrap();
        let mut callback = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Media { choices, .. } = q else {
                panic!()
            };
            assert_eq!(choices.iter().count(), 1);
            Ok(Decision::choose(choices.first().unwrap()))
        };
        let selected = selection::select_track(
            &mut Engine::new(PolicySet::default(), 8),
            &track,
            &packet,
            1,
            false,
            &mut callback,
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(selected.action, MediaAction::KeepTrack);
        struct Cancel;
        impl zencodec::enough::Stop for Cancel {
            fn check(&self) -> Result<(), zencodec::enough::StopReason> {
                Err(zencodec::enough::StopReason::Cancelled)
            }
        }
        let error = selection::select_track(
            &mut Engine::new(PolicySet::default(), 8),
            &track,
            &packet,
            1,
            false,
            &mut callback,
            &Cancel,
        )
        .unwrap_err();
        assert_eq!(error.error().code(), Code::Stopped);
        assert_eq!(packet.bytes(), [1, 2, 3]);
    }
}
