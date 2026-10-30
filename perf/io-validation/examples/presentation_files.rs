//! Native MP4 edit fixtures from an existing, independently decoded native IVF.
//! No encoding or pixel conversion occurs here.
use std::{error::Error, path::PathBuf, sync::Arc};
use zencodec::time::{FrameDuration, TimeBase, Timestamp};
use zencodec_io::SliceCursor;
use zencodec_media::{
    container::{DemuxEvent, DemuxStage, Demuxer, MuxHint, Muxer},
    limits::MediaLimits,
    timeline::{EditContent, EditExtent, EditList, EditSegment, PlaybackRate, TrackTimeline},
    track::TrackInfo,
};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [input, output] = args.as_slice() else {
        return Err("usage: presentation_files INPUT.ivf NEW_OUTPUT_DIRECTORY".into());
    };
    let bytes = std::fs::read(input)?;
    let output = PathBuf::from(output);
    std::fs::create_dir(&output)?;
    let mut decoder = zenivf::IvfDemuxer::new(MediaLimits::default());
    let mut cursor = SliceCursor::new(17);
    let mut packets = Vec::new();
    loop {
        let next = cursor.next(&bytes, &mut DemuxStage(&mut decoder), None)?;
        let done = next.is_finished();
        if let DemuxEvent::Packet(packet) = next.into_inner() {
            packets.push(packet);
        }
        if done {
            break;
        }
    }
    let [source] = decoder.tracks() else {
        return Err("expected one track".into());
    };
    if source.time_base() != TimeBase::MILLIS || packets.len() != 3 {
        return Err("fixture requires three frames on the millisecond clock".into());
    }
    for (packet, expected) in packets.iter().zip([0, 40, 80]) {
        if packet.pts() != Some(Timestamp::new(expected, TimeBase::MILLIS)) {
            return Err("unexpected fixture timestamps".into());
        }
    }
    let media = |duration, start, rate| {
        EditSegment::new(
            EditExtent::Finite(FrameDuration::new(duration, 1000).unwrap()),
            EditContent::Media {
                start: Timestamp::new(start, TimeBase::MILLIS),
                rate: PlaybackRate::new(rate, 1).unwrap(),
            },
        )
    };
    let programs = [
        ("identity", vec![media(120, 0, 1)]),
        ("clip", vec![media(40, 40, 1)]),
        ("repeat", vec![media(40, 0, 1), media(40, 0, 1)]),
        (
            "gap",
            vec![
                EditSegment::new(
                    EditExtent::Finite(FrameDuration::new(40, 1000)?),
                    EditContent::Gap,
                ),
                media(80, 0, 1),
            ],
        ),
        ("dwell", vec![media(80, 40, 0)]),
        ("double", vec![media(60, 0, 2)]),
    ];
    for (name, edits) in programs {
        for fragmented in [false, true] {
            let track = TrackInfo::video(
                0,
                source.codec().clone(),
                source.time_base(),
                *source.video_info().unwrap(),
            )
            .with_timeline(TrackTimeline::Edited(Arc::new(EditList::new(
                Timestamp::new(0, TimeBase::MILLIS),
                edits.clone(),
            )?)))
            .build()?;
            let mut mux = if fragmented {
                zenmp4::Mp4Muxer::fragmented()
            } else {
                zenmp4::Mp4Muxer::new()
            };
            mux.begin(&[track], MuxHint::default())?;
            let mut encoded = Vec::new();
            for packet in &packets {
                let packet = packet
                    .clone()
                    .with_dts(packet.pts().unwrap())
                    .with_duration_ticks(40);
                mux.write_packet(packet, None)?;
                mux.take_output(&mut encoded);
            }
            mux.finish(None)?.apply_to(&mut encoded)?;
            std::fs::write(
                output.join(format!(
                    "{name}-{}.mp4",
                    if fragmented {
                        "fragmented"
                    } else {
                        "progressive"
                    }
                )),
                encoded,
            )?;
        }
    }
    Ok(())
}
