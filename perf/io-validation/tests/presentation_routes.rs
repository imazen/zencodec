//! A container-native convenience route must not claim strict preservation.
use zencodec::time::{TimeBase, Timestamp};
use zencodec_io::SliceCursor;
use zencodec_media::{
    container::{DemuxEvent, DemuxStage, Demuxer, MuxHint, Muxer},
    limits::MediaLimits,
    output::{Forward, Output},
    packet::Packet,
    timeline::TrackTimeline,
    track::{CodecId, TrackInfo, VideoTrackInfo},
};
use zenivf::{IvfDemuxer, IvfMuxer};

fn read(bytes: &[u8]) -> (Vec<TrackInfo>, Vec<Packet>) {
    let mut decoder = IvfDemuxer::new(MediaLimits::default());
    let mut cursor = SliceCursor::new(7);
    let mut packets = Vec::new();
    loop {
        let next = cursor
            .next(bytes, &mut DemuxStage(&mut decoder), None)
            .unwrap();
        let done = next.is_finished();
        if let DemuxEvent::Packet(packet) = next.into_inner() {
            packets.push(packet);
        }
        if done {
            break;
        }
    }
    (decoder.tracks().to_vec(), packets)
}

#[test]
fn unedited_ivf_remux_uses_opted_in_native_timing_without_certifying_exactness() {
    let clock = TimeBase::new(1, 30).unwrap();
    let track = TrackInfo::video(0, CodecId::Av1, clock, VideoTrackInfo::new(2, 2))
        .build()
        .unwrap();
    let mut native = IvfMuxer::new();
    native.begin(&[track], MuxHint::default()).unwrap();
    let mut original = Vec::new();
    for pts in [0, 2, 5] {
        native
            .write_packet(
                Packet::from_vec(vec![0x12, 0])
                    .unwrap()
                    .with_track(0)
                    .with_pts(Timestamp::new(pts, clock)),
                None,
            )
            .unwrap();
        native.take_output(&mut original);
    }
    native
        .finish(None)
        .unwrap()
        .apply_to(&mut original)
        .unwrap();
    let (tracks, packets) = read(&original);
    assert_eq!(tracks[0].timeline(), &TrackTimeline::Unedited);
    let mut strict = Vec::new();
    assert!(
        Output::preserving_presentation(IvfMuxer::new(), Forward(&mut strict), &tracks, None)
            .is_err()
    );
    assert!(strict.is_empty());
    let mut remuxed = Vec::new();
    let mut output =
        Output::with_container_timing(IvfMuxer::new(), Forward(&mut remuxed), &tracks, None)
            .expect("explicit native timing must remain usable after truthful demux");
    for packet in packets {
        output.send(packet, None).unwrap();
    }
    output.finish(None).unwrap();
    assert_eq!(remuxed, original);
    let (_, packets) = read(&remuxed);
    assert_eq!(
        packets
            .iter()
            .map(|p| p.pts().unwrap().ticks())
            .collect::<Vec<_>>(),
        vec![0, 2, 5]
    );
}
