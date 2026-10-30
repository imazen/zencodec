//! Diagnostic for issue 187: no pixels are decoded and no timing benchmark runs.
//! An exit failure means distinct presentation programs collapsed to one scalar.
use std::sync::Arc;
use zencodec::time::TimeBase;
use zencodec_io::SliceCursor;
use zencodec_media::{
    container::{DemuxEvent, DemuxStage, Demuxer, MuxHint, Muxer},
    limits::MediaLimits,
    track::{CodecId, TrackInfo, VideoTrackInfo},
};

fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(payload.len() + 8)
        .unwrap()
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

// Fixture authoring only: the native muxer produces ordinary 32-bit init boxes.
fn inject(input: &[u8], edits: &[u8], in_movie: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < input.len() {
        let size = u32::from_be_bytes(input[pos..pos + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && pos + size <= input.len());
        let kind: &[u8; 4] = input[pos + 4..pos + 8].try_into().unwrap();
        let payload = &input[pos + 8..pos + size];
        if kind == b"moov" {
            out.extend(boxed(kind, &inject(payload, edits, true)));
        } else if in_movie && kind == b"trak" {
            let mut changed = payload.to_vec();
            changed.extend_from_slice(edits);
            out.extend(boxed(kind, &changed));
        } else {
            out.extend_from_slice(&input[pos..pos + size]);
        }
        pos += size;
    }
    out
}

fn edits(entries: &[(u32, i32, i16)]) -> Vec<u8> {
    let mut payload = vec![0; 4]; // elst version 0, flags 0
    payload.extend_from_slice(&u32::try_from(entries.len()).unwrap().to_be_bytes());
    for &(duration, media_time, rate) in entries {
        payload.extend_from_slice(&duration.to_be_bytes());
        payload.extend_from_slice(&media_time.to_be_bytes());
        payload.extend_from_slice(&rate.to_be_bytes());
        payload.extend_from_slice(&0_u16.to_be_bytes());
    }
    boxed(b"edts", &boxed(b"elst", &payload))
}

#[allow(deprecated)] // Diagnostic specifically measures the lossy legacy scalar.
fn inspect(bytes: &[u8]) -> Result<Option<i64>, String> {
    let mut demux = zenmp4::Mp4Demuxer::new(MediaLimits::default());
    let mut cursor = SliceCursor::new(65536);
    for _ in 0..64 {
        let event = cursor
            .next(bytes, &mut DemuxStage(&mut demux), None)
            .map_err(|e| format!("{e:?}"))?
            .into_inner();
        match event {
            DemuxEvent::Headers => {
                assert_eq!(demux.tracks().len(), 1);
                return Ok(demux.tracks()[0].edit_delay_ticks());
            }
            DemuxEvent::Done => return Err("EOF before headers".into()),
            _ => {}
        }
    }
    panic!("bounded header probe did not terminate");
}

fn main() {
    let track = TrackInfo::video(
        0,
        CodecId::Av1,
        TimeBase::new(1, 1000).unwrap(),
        VideoTrackInfo::new(64, 48),
    )
    .with_codec_private(Arc::from([0x81, 0, 0x0c, 0]))
    .build()
    .unwrap();
    let mut mux = zenmp4::Mp4Muxer::new();
    mux.begin(&[track], MuxHint::forward_only()).unwrap();
    let mut init = Vec::new();
    mux.take_output(&mut init);
    assert_eq!(inspect(&init).unwrap(), None, "unedited control");
    let first = inspect(&inject(&init, &edits(&[(1000, 17, 1)]), false)).unwrap();
    assert_eq!(first, Some(17), "single-edit control");
    let cases = [
        ("finite duration changed", vec![(2000, 17, 1)]),
        ("second media segment", vec![(1000, 17, 1), (1000, 51, 1)]),
        ("double playback rate", vec![(1000, 17, 2)]),
    ];
    let mut lost = Vec::new();
    for (name, program) in cases {
        let result = inspect(&inject(&init, &edits(&program), false));
        println!("{name}: {result:?}");
        if result == Ok(first) {
            lost.push(name);
        }
    }
    assert!(lost.is_empty(), "presentation information lost: {lost:?}");
}
