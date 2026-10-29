//! HDR metadata that first appears mid-stream. rav1d-safe reports a content
//! light level from the temporal unit that carries its metadata OBU; zenavif's
//! media decoder returns exactly the frames it returns without the OBU, so
//! a transcode through the media types drops the value without any signal,
//! error or refusal.
use std::sync::Arc;
use zenavif::media::{Av1EncodeSettings, Av1VideoDecoder, Av1VideoEncoder};
use zencodec::NoPulse;
use zencodec::time::{TimeBase, Timestamp};
use zencodec_media::{
    frame::{
        Frame, FrameGeometry, FrameInterpretation, FrameTiming, OwnedPlane, OwnedPlanes, PlaneData,
        PlaneRole, PlaneSamples, Presentation, SampleEncoding, Subsampling,
    },
    packet::Packet,
    session::{CodecConfig, Receive, SendStatus, VideoDecoder, VideoEncoder},
};
use zenpixels::{ChannelType, Cicp};

const TB: TimeBase = TimeBase::MILLIS;
const W: u32 = 64;
const H: u32 = 48;
const FRAMES: usize = 6;
const LATE: usize = 3;

fn settings() -> Av1EncodeSettings {
    Av1EncodeSettings::new(W, H)
        .with_bit_depth(8)
        .with_subsampling(Subsampling::Yuv420)
        .with_color(Cicp::new(9, 16, 9, false))
        .with_quantizer(100)
        .with_speed(10)
        .with_key_frame_interval(240)
        .with_threads(1)
}

fn frame(s: &Av1EncodeSettings, index: usize) -> Frame {
    let (w, h) = (W as usize, H as usize);
    let planes = [(w, h), (w / 2, h / 2), (w / 2, h / 2)]
        .into_iter()
        .enumerate()
        .map(|(c, (width, height))| OwnedPlane {
            data: PlaneData::U8(
                (0..width * height)
                    .map(|i| {
                        ((13 * (i % width) + 7 * (i / width) + 31 * index + 59 * c) % 251) as u8
                    })
                    .collect(),
            ),
            width,
            height,
            stride_bytes: width,
        })
        .collect();
    let encoding = SampleEncoding::new(ChannelType::U8, 8, 0).unwrap();
    let mut interpretation = FrameInterpretation::new(
        FrameGeometry::full(W, H),
        Subsampling::Yuv420,
        s.siting,
        encoding,
        vec![PlaneRole::Luma, PlaneRole::Cb, PlaneRole::Cr],
    );
    interpretation.color = zencodec::frame::ColorClaim::new(None, None, Some(s.color));
    Frame::new(
        Arc::new(OwnedPlanes::new(planes, encoding).unwrap()),
        Arc::new(interpretation),
        FrameTiming::at(Timestamp::new(index as i64 * 40, TB)).with_duration_ticks(40),
        Presentation::default(),
    )
}

fn encode(s: &Av1EncodeSettings) -> (CodecConfig, Vec<Packet>) {
    let mut encoder = Av1VideoEncoder::new(s.clone(), TB).unwrap();
    let config = encoder.output_config().unwrap().clone();
    let mut packets = Vec::new();
    for index in 0..FRAMES {
        let f = frame(s, index);
        while encoder.send(&f, Some(&NoPulse)).unwrap() != SendStatus::Accepted {
            let Receive::Ready(p) = encoder.receive(Some(&NoPulse)).unwrap() else {
                panic!("encoder stalled")
            };
            packets.push(p);
        }
        while let Receive::Ready(p) = encoder.receive(Some(&NoPulse)).unwrap() {
            packets.push(p);
        }
    }
    encoder.end_input().unwrap();
    while let Receive::Ready(p) = encoder.receive(Some(&NoPulse)).unwrap() {
        packets.push(p);
    }
    (config, packets)
}

/// AV1 metadata OBU with an HDR content light level (AV1 5.8.1, 5.8.2):
/// type 5 with a size field, metadata_type 1, max_cll, max_fall, trailing bit.
fn cll_obu(max_cll: u16, max_fall: u16) -> Vec<u8> {
    let mut payload = vec![1u8];
    payload.extend_from_slice(&max_cll.to_be_bytes());
    payload.extend_from_slice(&max_fall.to_be_bytes());
    payload.push(0x80);
    let mut obu = vec![(5 << 3) | 0b10, payload.len() as u8];
    obu.extend(payload);
    obu
}

/// The same packet facts around new payload bytes.
fn repacked(original: &Packet, bytes: Vec<u8>) -> Packet {
    let mut p = Packet::from_vec(bytes)
        .unwrap()
        .with_random_access(original.random_access())
        .with_config_epoch(original.config_epoch());
    if let Some(t) = original.pts() {
        p = p.with_pts(t);
    }
    if let Some(d) = original.duration_ticks() {
        p = p.with_duration_ticks(d);
    }
    if let Some(o) = original.ordinal() {
        p = p.with_ordinal(o);
    }
    p
}

/// The packet with `obu` after its opening temporal delimiter, if any.
fn with_obu(packet: &[u8], obu: &[u8]) -> Vec<u8> {
    let at = if packet.starts_with(&[0x12, 0x00]) {
        2
    } else {
        0
    };
    [&packet[..at], obu, &packet[at..]].concat()
}

/// Decodes `bytes` (each with its original packet's facts) through the media decoder.
fn media_frames(config: &CodecConfig, packets: &[Packet], bytes: &[Vec<u8>]) -> Vec<Frame> {
    let mut decoder = Av1VideoDecoder::new().unwrap();
    decoder.configure(config, Some(&NoPulse)).unwrap();
    let mut frames = Vec::new();
    for (original, b) in packets.iter().zip(bytes) {
        let mut packet = repacked(original, b.clone());
        while decoder.send(&mut packet, Some(&NoPulse)).unwrap() != SendStatus::Accepted {
            let Receive::Ready(f) = decoder.receive(Some(&NoPulse)).unwrap() else {
                panic!("decoder stalled")
            };
            frames.push(f);
        }
        while let Receive::Ready(f) = decoder.receive(Some(&NoPulse)).unwrap() {
            frames.push(f);
        }
    }
    decoder.end_input().unwrap();
    while let Receive::Ready(f) = decoder.receive(Some(&NoPulse)).unwrap() {
        frames.push(f);
    }
    frames
}

/// Every plane's samples, row by row.
fn samples(frame: &Frame) -> Vec<u8> {
    let mapping = frame.map().unwrap();
    let mut out = Vec::new();
    for c in 0..mapping.plane_count() {
        let plane = mapping.plane(c).unwrap();
        for y in 0..plane.height() {
            match plane.row(y).unwrap() {
                PlaneSamples::U8(row) => out.extend_from_slice(row),
                PlaneSamples::U16(row) => row
                    .iter()
                    .for_each(|w| out.extend_from_slice(&w.to_le_bytes())),
            }
        }
    }
    out
}

#[test]
fn late_content_light_level_is_decoded_natively_and_invisible_to_media_frames() {
    let s = settings();
    let (config, packets) = encode(&s);
    assert_eq!(packets.len(), FRAMES);
    let obu = cll_obu(1000, 400);
    let bytes: Vec<Vec<u8>> = packets
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let raw = p.bytes();
            if i == LATE {
                with_obu(raw, &obu)
            } else {
                raw.to_vec()
            }
        })
        .collect();

    // Native: the level appears from the temporal unit carrying the OBU.
    let mut native = rav1d_safe::Decoder::new().unwrap();
    let mut pictures = Vec::new();
    for b in &bytes {
        if let Some(picture) = native.decode(b).unwrap() {
            pictures.push(picture);
        }
        while let Some(picture) = native.get_frame().unwrap() {
            pictures.push(picture);
        }
    }
    pictures.extend(native.flush().unwrap());
    assert_eq!(pictures.len(), FRAMES);
    let levels: Vec<_> = pictures
        .iter()
        .map(|p| {
            p.content_light()
                .map(|l| (l.max_content_light_level, l.max_frame_average_light_level))
        })
        .collect();
    eprintln!("rav1d-safe content light levels per picture: {levels:?}");
    assert!(levels[..LATE].iter().all(Option::is_none), "{levels:?}");
    assert_eq!(levels[LATE], Some((1000, 400)), "{levels:?}");

    // Media boundary: decoding the stream with and without the metadata OBU
    // yields identical frames, so nothing downstream can see or carry it.
    let plain: Vec<Vec<u8>> = packets.iter().map(|p| p.bytes().to_vec()).collect();
    let with_level = media_frames(&config, &packets, &bytes);
    let without = media_frames(&config, &packets, &plain);
    assert_eq!(with_level.len(), FRAMES);
    assert_eq!(without.len(), FRAMES);
    for (i, (a, b)) in with_level.iter().zip(&without).enumerate() {
        assert_eq!(**a.interpretation(), **b.interpretation(), "frame {i}");
        assert_eq!(a.presentation(), b.presentation(), "frame {i}");
        assert_eq!(a.timing(), b.timing(), "frame {i}");
        assert_eq!(samples(a), samples(b), "frame {i}");
    }
}
