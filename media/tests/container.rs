//! Container contract tests: WebM mux/demux round-trips and MP4 demuxing
//! against committed ffmpeg-generated fixtures (see corpus/mp4, provenance in
//! DATA_PROVENANCE.md).

use std::io::Cursor;
use zencodec_media::mp4::Mp4Demuxer;
use zencodec_media::time::{TimeBase, Timestamp};
use zencodec_media::track::{
    AudioInfo, Codec, MediaLimits, MediaPacket, TrackKind, TrackSpec, VideoInfo,
};
use zencodec_media::webm::{TickPolicy, WebmDemuxer, WebmMuxer};

fn ms(ts: Timestamp) -> f64 {
    let tb = ts.time_base();
    ts.ticks() as f64 * tb.numerator() as f64 / tb.denominator() as f64 * 1000.0
}

fn spec_track(index: u32, kind: TrackKind, codec: Codec) -> TrackSpec {
    TrackSpec {
        index,
        kind,
        codec,
        codec_private: None,
        time_base: TimeBase::new(1, 1_000).unwrap(), // packet timestamps in ms
        video: if kind == TrackKind::Video {
            Some(VideoInfo {
                width: 160,
                height: 96,
            })
        } else {
            None
        },
        audio: if kind == TrackKind::Audio {
            Some(AudioInfo {
                sample_rate: 48_000,
                channels: 1,
            })
        } else {
            None
        },
        codec_delay_ns: 0,
        seek_preroll_ns: 0,
        config_epoch: 0,
        declared_packets: None,
        declared_duration: None,
        edit_delay_ticks: None,
    }
}

fn pkt(track: u32, ordinal: u64, ms_pts: i64, data: &[u8], key: bool) -> MediaPacket {
    MediaPacket {
        track,
        ordinal,
        config_epoch: 0,
        data: data.to_vec(),
        pts: Timestamp::new(ms_pts, TimeBase::new(1, 1_000).unwrap()),
        dts: None,
        duration_ticks: None,
        keyframe: key,
        discard_padding_ns: None,
    }
}

#[test]
fn webm_mux_demux_roundtrip() {
    let tracks = [
        spec_track(0, TrackKind::Video, Codec::Vp9),
        spec_track(1, TrackKind::Audio, Codec::Opus),
    ];
    let mut mux = WebmMuxer::new(Vec::new(), 1_000_000, &tracks, 5_000, TickPolicy::Exact).unwrap();

    // Interleaved writes; last audio packet carries Opus-style end trim.
    mux.write_packet(&pkt(0, 0, 0, b"frame0", true)).unwrap();
    mux.write_packet(&pkt(1, 0, 0, b"aud0", false)).unwrap();
    mux.write_packet(&pkt(1, 1, 20, b"aud1", false)).unwrap();
    mux.write_packet(&pkt(0, 1, 33, b"frame1", false)).unwrap();
    let mut tail = pkt(1, 2, 40, b"aud2", false);
    tail.discard_padding_ns = Some(-500_000); // negative: start trim
    mux.write_packet(&tail).unwrap();
    let (bytes, report) = mux.finish().unwrap();
    assert_eq!(report.packets_written, 5);
    assert_eq!(
        report.quantized_packets, 0,
        "ms times are exact at 1ms scale"
    );
    assert_eq!(report.discard_padding_written_ns, 500_000);
    assert_eq!(report.clusters_written, 1);

    let mut d = WebmDemuxer::new(&bytes[..], MediaLimits::default()).unwrap();
    let ts = d.tracks();
    assert_eq!(ts.len(), 2);
    assert_eq!(ts[0].codec, Codec::Vp9);
    assert_eq!(ts[0].kind, TrackKind::Video);
    assert_eq!(ts[0].video.unwrap().width, 160);
    assert_eq!(ts[1].codec, Codec::Opus);
    assert_eq!(ts[1].audio.unwrap().sample_rate, 48_000);

    let mut got = Vec::new();
    while let Some(p) = d.next_packet().unwrap() {
        got.push((
            p.track,
            ms(p.pts) as i64,
            p.data.clone(),
            p.keyframe,
            p.discard_padding_ns,
        ));
    }
    assert_eq!(got.len(), 5);
    // Emission order = cluster storage order.
    assert_eq!(&got[0].2[..], b"frame0");
    assert!(got[0].3, "first video packet is a keyframe");
    assert_eq!(&got[1].2[..], b"aud0");
    assert_eq!(got[4].1, 40);
    assert_eq!(
        got[4].4,
        Some(-500_000),
        "discard padding survives round-trip"
    );
}

#[test]
fn webm_mux_tick_policy() {
    let tracks = [spec_track(0, TrackKind::Video, Codec::Vp9)];
    // 1/30 s is irrational in 1ms ticks: Exact must fail, Nearest must count.
    let p = MediaPacket {
        pts: Timestamp::new(1, TimeBase::new(1, 30).unwrap()),
        ..pkt(0, 0, 0, b"x", true)
    };
    let mut strict =
        WebmMuxer::new(Vec::new(), 1_000_000, &tracks, 5_000, TickPolicy::Exact).unwrap();
    assert!(strict.write_packet(&p).is_err());

    let mut lax =
        WebmMuxer::new(Vec::new(), 1_000_000, &tracks, 5_000, TickPolicy::Nearest).unwrap();
    lax.write_packet(&p).unwrap();
    let (_w, rep) = lax.finish().unwrap();
    assert_eq!(rep.quantized_packets, 1);
    assert!(rep.max_tick_error_ns > 0);
}

#[test]
fn webm_mux_epoch_change_rejected() {
    let tracks = [spec_track(0, TrackKind::Video, Codec::Vp9)];
    let mut mux = WebmMuxer::new(Vec::new(), 1_000_000, &tracks, 5_000, TickPolicy::Exact).unwrap();
    let mut p = pkt(0, 0, 0, b"x", true);
    p.config_epoch = 1; // spec declares epoch 0
    assert!(matches!(
        mux.write_packet(&p),
        Err(zencodec_media::track::MediaError::Contract(_))
    ));
}

fn fixture(name: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus/mp4")
        .join(name);
    std::fs::read(p).expect("fixture")
}

fn demux_all(bytes: &[u8]) -> (Vec<TrackSpec>, Vec<MediaPacket>) {
    let mut d = Mp4Demuxer::new(Cursor::new(bytes), MediaLimits::default()).unwrap();
    let tracks = d.tracks();
    let mut pkts = Vec::new();
    while let Some(p) = d.next_packet().unwrap() {
        pkts.push(p);
    }
    (tracks, pkts)
}

#[test]
fn mp4_demux_h264_aac() {
    for name in ["h264_aac_faststart.mp4", "h264_aac_moovlast.mp4"] {
        let (tracks, pkts) = demux_all(&fixture(name));
        assert_eq!(tracks.len(), 2, "{name}");

        let v = tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap();
        assert_eq!(v.codec, Codec::H264, "{name}");
        assert_eq!(v.video.unwrap().width, 160);
        assert_eq!(
            v.codec_private.as_ref().unwrap()[0],
            1,
            "avcC record version"
        );
        let a = tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap();
        assert_eq!(a.codec, Codec::Aac, "{name}");
        assert_eq!(a.audio.unwrap().sample_rate, 44_100);
        assert!(a.codec_private.is_some(), "esds AudioSpecificConfig");

        let vp: Vec<_> = pkts.iter().filter(|p| p.track == v.index).collect();
        let ap: Vec<_> = pkts.iter().filter(|p| p.track == a.index).collect();
        assert_eq!(vp.len(), 15, "{name} video packets");
        assert_eq!(ap.len(), 45, "{name} audio packets");
        assert!(vp[0].keyframe);
        for w in vp.windows(2) {
            assert!(w[0].dts.unwrap().ticks() < w[1].dts.unwrap().ticks());
        }
        for w in ap.windows(2) {
            assert!(w[0].dts.unwrap().ticks() < w[1].dts.unwrap().ticks());
        }
        // Merged order is nondecreasing in physical decode time.
        for w in pkts.windows(2) {
            let a_ts = w[0].dts.unwrap_or(w[0].pts);
            let b_ts = w[1].dts.unwrap_or(w[1].pts);
            assert!(
                a_ts.compare(b_ts) != std::cmp::Ordering::Greater,
                "{name} order"
            );
        }
        // 15fps on a 15360 timescale: exactly 1024 ticks/frame in decode order.
        // (pts is reordered by b-frames; dts is the monotonic sequence.)
        assert_eq!(
            vp[1].dts.unwrap().ticks() - vp[0].dts.unwrap().ticks(),
            1024
        );
        // b-frame reorder: pts must differ from dts for at least one frame.
        assert!(
            vp.iter().any(|p| p.pts.ticks() != p.dts.unwrap().ticks()),
            "{name}: expected ctts reordering present"
        );
    }
}

#[test]
fn mp4_to_webm_copy() {
    let bytes = fixture("h264_aac_faststart.mp4");
    let mut d = Mp4Demuxer::new(Cursor::new(&bytes), MediaLimits::default()).unwrap();
    let tracks = d.tracks();
    // Out specs: same codec/private; WebM track time_base is informational
    // (packets carry their own tb) — normalize it to the segment scale.
    let out_tracks: Vec<TrackSpec> = tracks
        .iter()
        .map(|t| TrackSpec {
            time_base: TimeBase::new(1_000_000, 1_000_000_000).unwrap(),
            ..t.clone()
        })
        .collect();

    let mut mux = WebmMuxer::new(
        Vec::new(),
        1_000_000,
        &out_tracks,
        5_000,
        TickPolicy::Nearest,
    )
    .unwrap();
    let mut count = 0u64;
    while let Some(p) = d.next_packet().unwrap() {
        mux.write_packet(&p).unwrap();
        count += 1;
    }
    let (webm, rep) = mux.finish().unwrap();
    assert_eq!(count, 60);
    assert_eq!(rep.packets_written, 60);

    let mut dd = WebmDemuxer::new(&webm[..], MediaLimits::default()).unwrap();
    let t2 = dd.tracks();
    assert_eq!(t2.len(), 2);
    assert_eq!(t2[0].codec, Codec::H264);
    assert_eq!(t2[1].codec, Codec::Aac);
    let mut n = 0;
    while let Some(_p) = dd.next_packet().unwrap() {
        n += 1;
    }
    assert_eq!(n, 60, "packet count survives mp4→webm copy");
}

// --- hand-built WebM bodies (lacing / malformed coverage) ------------------

use zencodec_media::ebml as e;

fn ebml_head() -> Vec<u8> {
    let mut h = Vec::new();
    e::write_uint(&mut h, e::id::EBML_VERSION, 1).unwrap();
    e::write_uint(&mut h, e::id::EBML_READ_VERSION, 1).unwrap();
    e::write_uint(&mut h, e::id::EBML_MAX_ID_LENGTH, 4).unwrap();
    e::write_uint(&mut h, e::id::EBML_MAX_SIZE_LENGTH, 8).unwrap();
    e::write_utf8(&mut h, e::id::DOC_TYPE, "webm").unwrap();
    e::write_uint(&mut h, e::id::DOC_TYPE_VERSION, 4).unwrap();
    e::write_uint(&mut h, e::id::DOC_TYPE_READ_VERSION, 2).unwrap();
    let mut out = Vec::new();
    e::write_element(&mut out, e::id::EBML, &h).unwrap();
    out
}

/// One Opus audio track entry (track number 1).
fn opus_track_entry() -> Vec<u8> {
    let mut te = Vec::new();
    e::write_uint(&mut te, e::id::TRACK_NUMBER, 1).unwrap();
    e::write_uint(&mut te, e::id::TRACK_UID, 1).unwrap();
    e::write_uint(&mut te, e::id::TRACK_TYPE, 2).unwrap(); // audio
    e::write_uint(&mut te, e::id::FLAG_LACING, 1).unwrap();
    e::write_utf8(&mut te, e::id::CODEC_ID, "A_OPUS").unwrap();
    let mut ae = Vec::new();
    e::write_id(&mut ae, e::id::SAMPLING_FREQUENCY).unwrap();
    e::write_size(&mut ae, Some(8)).unwrap();
    ae.extend_from_slice(&48000f64.to_be_bytes());
    e::write_uint(&mut ae, e::id::CHANNELS, 1).unwrap();
    e::write_element(&mut te, e::id::AUDIO, &ae).unwrap();
    let mut tracks = Vec::new();
    e::write_element(&mut tracks, e::id::TRACK_ENTRY, &te).unwrap();
    tracks
}

/// Wrap Info+Tracks+Cluster bodies into a complete segment-level stream.
fn wrap_segment(children: &[u8]) -> Vec<u8> {
    let mut out = ebml_head();
    e::write_id(&mut out, e::id::SEGMENT).unwrap();
    e::write_size(&mut out, Some(children.len() as u64)).unwrap();
    out.extend_from_slice(children);
    out
}

fn info_ms() -> Vec<u8> {
    let mut i = Vec::new();
    e::write_uint(&mut i, e::id::TIMESTAMP_SCALE, 1_000_000).unwrap();
    let mut out = Vec::new();
    e::write_element(&mut out, e::id::INFO, &i).unwrap();
    out
}

fn simple_block(track: u8, rel_ts: i16, flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    e::write_id(&mut b, track as u64 | 0x80).unwrap(); // track no as vint
    b.extend_from_slice(&rel_ts.to_be_bytes());
    b.push(flags);
    b.extend_from_slice(payload);
    b
}

fn cluster(children: Vec<Vec<u8>>) -> Vec<u8> {
    let mut c = Vec::new();
    e::write_uint(&mut c, e::id::TIMESTAMP, 0).unwrap();
    for ch in children {
        e::write_element(&mut c, e::id::SIMPLE_BLOCK, &ch).unwrap();
    }
    let mut out = Vec::new();
    e::write_element(&mut out, e::id::CLUSTER, &c).unwrap();
    out
}

fn collect(file: &[u8]) -> Result<Vec<MediaPacket>, zencodec_media::track::MediaError> {
    let mut d = WebmDemuxer::new(file, MediaLimits::default())?;
    let mut v = Vec::new();
    while let Some(p) = d.next_packet()? {
        v.push(p);
    }
    Ok(v)
}

#[test]
fn webm_demux_xiph_lacing() {
    // Xiph lacing (flags 0b0010): count-1 byte, then count-1 size bytes;
    // the last frame is the remainder.
    let lace: &[u8] = &[
        2, 3, 4, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i',
    ];
    //          count-1=2, sizes 3,4 → frames: "abc" "defg" "hi"
    let blk = simple_block(1, 5, 0x02, lace);
    let mut kids = info_ms();
    let mut t = Vec::new();
    e::write_element(&mut t, e::id::TRACKS, &opus_track_entry()).unwrap();
    kids.extend_from_slice(&t);
    kids.extend_from_slice(&cluster(vec![blk]));
    let file = wrap_segment(&kids);

    let pkts = collect(&file).unwrap();
    assert_eq!(pkts.len(), 3);
    assert_eq!(pkts[0].data, b"abc");
    assert_eq!(pkts[1].data, b"defg");
    assert_eq!(pkts[2].data, b"hi");
    // All laced frames share the block timestamp: 5 ticks @1ms.
    for p in &pkts {
        assert_eq!(p.pts.ticks(), 5);
    }
}

#[test]
fn webm_demux_fixed_lacing() {
    // Fixed lacing (flags 0b0100): count-1 byte, then equal-size frames.
    let lace: &[u8] = &[2, b'1', b'2', b'3', b'4', b'5', b'6'];
    // 3 frames of 2 bytes each.
    let blk = simple_block(1, 0, 0x04, lace);
    let mut kids = info_ms();
    let mut t = Vec::new();
    e::write_element(&mut t, e::id::TRACKS, &opus_track_entry()).unwrap();
    kids.extend_from_slice(&t);
    kids.extend_from_slice(&cluster(vec![blk]));
    let file = wrap_segment(&kids);

    let pkts = collect(&file).unwrap();
    assert_eq!(pkts.len(), 3);
    assert_eq!(pkts[0].data, b"12");
    assert_eq!(pkts[1].data, b"34");
    assert_eq!(pkts[2].data, b"56");
}

#[test]
fn webm_demux_ebml_lacing() {
    // EBML lacing (flags 0b0110): count-1 byte, first size as unsigned size
    // vint, then count-2 signed vint size diffs (bias = 2^(7n-1)-1; n=1 → 63).
    // Frames: 3, 5, 2 bytes → diffs +2, -3.
    let mut lace = vec![2u8];
    lace.push(0x83); // first size 3 → 1-byte size vint 0x80|3
    lace.push(0xC1); // diff +2 → 63+2 = 65 → 0x80|65
    // frame 3 is the remainder — no explicit size.
    lace.extend_from_slice(b"aaabbbbbcc");
    let blk = simple_block(1, 0, 0x06, &lace);
    let mut kids = info_ms();
    let mut t = Vec::new();
    e::write_element(&mut t, e::id::TRACKS, &opus_track_entry()).unwrap();
    kids.extend_from_slice(&t);
    kids.extend_from_slice(&cluster(vec![blk]));
    let file = wrap_segment(&kids);

    let pkts = collect(&file).unwrap();
    assert_eq!(pkts.len(), 3);
    assert_eq!(pkts[0].data, b"aaa");
    assert_eq!(pkts[1].data, b"bbbbb");
    assert_eq!(pkts[2].data, b"cc");
}

#[test]
fn webm_demux_malformed() {
    let mut kids = info_ms();
    let mut t = Vec::new();
    e::write_element(&mut t, e::id::TRACKS, &opus_track_entry()).unwrap();
    kids.extend_from_slice(&t);

    // Block references track 2 — only track 1 exists.
    let bad_track = simple_block(2, 0, 0x00, b"x");
    let file1 = wrap_segment(&{
        let mut k = kids.clone();
        k.extend_from_slice(&cluster(vec![bad_track]));
        k
    });
    let mut d = WebmDemuxer::new(&file1[..], MediaLimits::default()).unwrap();
    assert!(d.next_packet().is_err(), "undeclared track must error");

    // Truncated block payload.
    let trunc = simple_block(1, 0, 0x02, &[5, 3, 4]); // xiph wants frames but payload ends
    let file2 = wrap_segment(&{
        let mut k = kids.clone();
        k.extend_from_slice(&cluster(vec![trunc]));
        k
    });
    let mut d = WebmDemuxer::new(&file2[..], MediaLimits::default()).unwrap();
    assert!(d.next_packet().is_err(), "overrun lacing must error");

    // File truncated inside cluster payload.
    let mut file3 = wrap_segment(&{
        let mut k = kids.clone();
        k.extend_from_slice(&cluster(vec![simple_block(1, 0, 0, b"hello")]));
        k
    });
    file3.truncate(file3.len() - 2);
    let mut d = WebmDemuxer::new(&file3[..], MediaLimits::default()).unwrap();
    assert!(d.next_packet().is_err() || d.next_packet().unwrap().is_none());

    // Unknown-size leaf element (Info body has a child with unknown size).
    let mut bad_info = Vec::new();
    e::write_id(&mut bad_info, e::id::TIMESTAMP_SCALE).unwrap();
    e::write_size(&mut bad_info, None).unwrap(); // unknown size on a leaf
    let mut k2 = Vec::new();
    e::write_element(&mut k2, e::id::INFO, &bad_info).unwrap();
    let mut tk = Vec::new();
    e::write_element(&mut tk, e::id::TRACKS, &opus_track_entry()).unwrap();
    k2.extend_from_slice(&tk);
    k2.extend_from_slice(&cluster(vec![simple_block(1, 0, 0, b"x")]));
    let file4 = wrap_segment(&k2);
    // Demuxer reads Info children via walk_children → unknown-size child errors.
    assert!(
        WebmDemuxer::new(&file4[..], MediaLimits::default()).is_err()
            || WebmDemuxer::new(&file4[..], MediaLimits::default())
                .and_then(|mut d| d.next_packet().map(|_| ()))
                .is_err()
    );
}

#[test]
fn mp4_malformed() {
    // Empty file.
    assert!(Mp4Demuxer::new(Cursor::new(Vec::<u8>::new()), MediaLimits::default()).is_err());
    // ftyp only, no moov.
    let ftyp_only = {
        let mut v = b"....ftypisom".to_vec(); // 12-byte ftyp
        v[0..4].copy_from_slice(&12u32.to_be_bytes());
        v
    };
    assert!(Mp4Demuxer::new(Cursor::new(ftyp_only), MediaLimits::default()).is_err());
    // Truncated moov mid-trak.
    let good = fixture("h264_aac_faststart.mp4");
    for cut in [40usize, 300, 1000, 1900] {
        let t = &good[..good.len().min(cut + 40)];
        assert!(
            Mp4Demuxer::new(Cursor::new(t.to_vec()), MediaLimits::default()).is_err() || {
                // Some cuts may parse a truncated-but-valid prefix; a demuxer that
                // built must then fail or produce fewer than the full 60 packets.
                let mut d =
                    Mp4Demuxer::new(Cursor::new(t.to_vec()), MediaLimits::default()).unwrap();
                let mut n = 0;
                let mut ok = true;
                loop {
                    match d.next_packet() {
                        Ok(Some(_)) => n += 1,
                        Ok(None) => break,
                        Err(_) => {
                            ok = false;
                            break;
                        }
                    }
                }
                !(ok && n == 60)
            },
            "cut {cut} must not yield the full packet stream"
        );
    }
}
