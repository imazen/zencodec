//! Native frame and IVF helpers adapted from zenavif ea055f7d's
//! tests/media_session.rs and examples/media_ivf.rs. Test artifacts only.
use std::{path::Path, process::Command, sync::Arc};

use enough::{Stop, Unstoppable};
use zenavif::media::{Av1EncodeSettings, Av1VideoEncoder};
use zencodec::time::{TimeBase, Timestamp};
use zencodec_media::{
    frame::{
        Frame, FrameGeometry, FrameInterpretation, FrameTiming, OwnedPlane, OwnedPlanes, PlaneData,
        PlaneRole, PlaneSamples, Presentation, SampleEncoding, Subsampling,
    },
    packet::Packet,
    session::{CodecConfig, Receive, SendStatus, SessionError, VideoDecoder, VideoEncoder},
};
use zenpixels::{ChannelType, Cicp};

pub const TB: TimeBase = TimeBase::MILLIS;

pub fn settings(bits: u8, sub: Subsampling, identity: bool) -> Av1EncodeSettings {
    Av1EncodeSettings::new(17, 13)
        .with_bit_depth(bits)
        .with_subsampling(sub)
        .with_color(Cicp::new(1, 13, u8::from(!identity), true))
        .with_quantizer(0)
        .with_speed(10)
        .with_key_frame_interval(4)
        .with_threads(1)
}

pub fn frame(s: &Av1EncodeSettings, index: usize) -> Frame {
    let (w, h) = (s.width as usize, s.height as usize);
    let dims = match s.subsampling {
        Subsampling::Mono => vec![(w, h)],
        Subsampling::Yuv420 => vec![
            (w, h),
            (w.div_ceil(2), h.div_ceil(2)),
            (w.div_ceil(2), h.div_ceil(2)),
        ],
        Subsampling::Yuv422 => vec![(w, h), (w.div_ceil(2), h), (w.div_ceil(2), h)],
        Subsampling::Yuv444 => vec![(w, h); 3],
        other => panic!("unsupported test layout {other:?}"),
    };
    let mask = (1u32 << s.bit_depth) - 1;
    let planes = dims
        .into_iter()
        .enumerate()
        .map(|(c, (width, height))| {
            let stride = width + 3;
            let mut words = vec![mask as u16; stride * height];
            for y in 0..height {
                for x in 0..width {
                    let seed = (17 * x + 29 * y + 43 * index + 71 * c) as u32;
                    words[y * stride + x] = ((seed.wrapping_mul(997) ^ (seed >> 2)) & mask) as u16;
                }
            }
            let data = if s.bit_depth == 8 {
                PlaneData::U8(words.into_iter().map(|v| v as u8).collect())
            } else {
                PlaneData::U16(words)
            };
            OwnedPlane {
                data,
                width,
                height,
                stride_bytes: stride * if s.bit_depth == 8 { 1 } else { 2 },
            }
        })
        .collect();
    let encoding = SampleEncoding::new(
        if s.bit_depth == 8 {
            ChannelType::U8
        } else {
            ChannelType::U16
        },
        s.bit_depth,
        0,
    )
    .unwrap();
    let roles = if s.subsampling == Subsampling::Mono {
        vec![PlaneRole::Luma]
    } else if s.color.matrix_coefficients == 0 {
        vec![PlaneRole::Green, PlaneRole::Blue, PlaneRole::Red]
    } else {
        vec![PlaneRole::Luma, PlaneRole::Cb, PlaneRole::Cr]
    };
    let mut interpretation = FrameInterpretation::new(
        FrameGeometry::full(s.width, s.height),
        s.subsampling,
        s.siting,
        encoding,
        roles,
    );
    interpretation.color = zencodec::frame::ColorClaim::new(None, None, Some(s.color));
    Frame::new(
        Arc::new(OwnedPlanes::new(planes, encoding).unwrap()),
        Arc::new(interpretation),
        FrameTiming::at(Timestamp::new(index as i64 * 40, TB)).with_duration_ticks(40),
        Presentation::default(),
    )
}

pub fn encode(settings: Av1EncodeSettings, frames: &[Frame]) -> (CodecConfig, Vec<Packet>) {
    let mut encoder = Av1VideoEncoder::new(settings, TB).unwrap();
    let config = encoder.output_config().unwrap().clone();
    let mut packets = Vec::new();
    for frame in frames {
        while encoder.send(frame, &Unstoppable).unwrap() != SendStatus::Accepted {
            let Receive::Ready(packet) = encoder.receive(&Unstoppable).unwrap() else {
                panic!("encoder stalled")
            };
            packets.push(packet);
        }
        while let Receive::Ready(packet) = encoder.receive(&Unstoppable).unwrap() {
            packets.push(packet);
        }
    }
    encoder.end_input().unwrap();
    loop {
        match encoder.receive(&Unstoppable).unwrap() {
            Receive::Ready(packet) => packets.push(packet),
            Receive::EndOfStream => break,
            other => panic!("encoder did not drain: {other:?}"),
        }
    }
    (config, packets)
}

pub fn decode<D: VideoDecoder>(
    decoder: &mut D,
    config: &CodecConfig,
    packets: &[Packet],
    stop: &dyn Stop,
) -> Result<Vec<Frame>, SessionError<D::Error>> {
    decoder.configure(config, stop)?;
    let mut outputs = Vec::new();
    for packet in packets {
        let mut packet = packet.clone();
        while decoder.send(&mut packet, stop)? != SendStatus::Accepted {
            let Receive::Ready(frame) = decoder.receive(stop)? else {
                panic!("decoder stalled")
            };
            outputs.push(frame);
        }
        while let Receive::Ready(frame) = decoder.receive(stop)? {
            outputs.push(frame);
        }
    }
    decoder.end_input()?;
    loop {
        match decoder.receive(stop)? {
            Receive::Ready(frame) => outputs.push(frame),
            Receive::EndOfStream => break,
            other => panic!("decoder did not drain: {other:?}"),
        }
    }
    Ok(outputs)
}

pub fn samples(frame: &Frame) -> Vec<u8> {
    let mapping = frame.map().unwrap();
    let mut bytes = Vec::new();
    for c in 0..mapping.plane_count() {
        let plane = mapping.plane(c).unwrap();
        for y in 0..plane.height() {
            match plane.row(y).unwrap() {
                PlaneSamples::U8(row) => bytes.extend_from_slice(row),
                PlaneSamples::U16(row) => row
                    .iter()
                    .for_each(|word| bytes.extend_from_slice(&word.to_le_bytes())),
            }
        }
    }
    bytes
}

/// The independent decoder inspects actual packets emitted after policy.
/// No third-party image generation or conversion supplies expected samples.
pub fn oracle(
    root: &Path,
    name: &str,
    s: &Av1EncodeSettings,
    packets: &[Packet],
    frames: &[Frame],
) {
    let dir = root.join(name);
    std::fs::create_dir(&dir)
        .expect("use a fresh artifact directory; previous evidence is retained");
    let mut ivf = Vec::new();
    ivf.extend_from_slice(b"DKIF\0\0\x20\0AV01");
    ivf.extend_from_slice(&(s.width as u16).to_le_bytes());
    ivf.extend_from_slice(&(s.height as u16).to_le_bytes());
    ivf.extend_from_slice(&TB.denominator().to_le_bytes());
    ivf.extend_from_slice(&TB.numerator().to_le_bytes());
    ivf.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    ivf.extend_from_slice(&0u32.to_le_bytes());
    for packet in packets {
        ivf.extend_from_slice(&(packet.bytes().len() as u32).to_le_bytes());
        ivf.extend_from_slice(&(packet.pts().unwrap().ticks() as u64).to_le_bytes());
        ivf.extend_from_slice(packet.bytes());
    }
    let encoded = dir.join("approved.ivf");
    let decoded = dir.join("dav1d.raw");
    std::fs::write(&encoded, ivf).unwrap();
    let base = match (s.subsampling, s.color.matrix_coefficients) {
        (Subsampling::Mono, _) => "gray",
        (Subsampling::Yuv420, _) => "yuv420p",
        (Subsampling::Yuv422, _) => "yuv422p",
        (Subsampling::Yuv444, 0) => "gbrp",
        (Subsampling::Yuv444, _) => "yuv444p",
        _ => panic!("unsupported oracle layout"),
    };
    let fmt = if s.bit_depth == 8 {
        base.to_owned()
    } else {
        format!("{base}{}le", s.bit_depth)
    };
    let result = Command::new("timeout")
        .args([
            "--kill-after=5s",
            "30s",
            "ffmpeg",
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-threads",
            "1",
            "-c:v",
            "libdav1d",
            "-i",
        ])
        .arg(&encoded)
        .args([
            "-f",
            "rawvideo",
            "-pix_fmt",
            &fmt,
            "-fps_mode",
            "passthrough",
        ])
        .arg(&decoded)
        .output()
        .expect("FFmpeg is a required oracle");
    std::fs::write(dir.join("ffmpeg.stderr"), &result.stderr).unwrap();
    assert!(
        result.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected: Vec<u8> = frames.iter().flat_map(samples).collect();
    std::fs::write(dir.join("expected.raw"), &expected).unwrap();
    assert_eq!(
        std::fs::read(decoded).unwrap(),
        expected,
        "{name}: independent decoded samples"
    );
}
