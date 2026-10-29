//! Native AV1 (zenavif's media encoder): what an accepted frame retains and how
//! many frames are accepted before the first packet. `VideoEncoder` declares
//! neither before the first frame, so a caller sizing frame storage or a live
//! output wait has no stated value to use.
use std::sync::Arc;
use zenavif::media::{Av1EncodeSettings, Av1VideoEncoder};
use zencodec::NoPulse;
use zencodec::time::{TimeBase, Timestamp};
use zencodec_io::storage::StorageBudget;
use zencodec_media::{
    frame::{
        Frame, FrameGeometry, FrameInterpretation, FrameTiming, OwnedPlane, OwnedPlanes, PlaneData,
        PlaneRole, Presentation, SampleEncoding, Subsampling,
    },
    session::{Receive, SendStatus, VideoEncoder},
};
use zenpixels::{ChannelType, Cicp};

const TB: TimeBase = TimeBase::MILLIS;
const W: u32 = 64;
const H: u32 = 48;

fn settings(speed: u8) -> Av1EncodeSettings {
    Av1EncodeSettings::new(W, H)
        .with_bit_depth(8)
        .with_subsampling(Subsampling::Yuv420)
        .with_color(Cicp::new(1, 13, 1, true))
        .with_quantizer(100)
        .with_speed(speed)
        .with_key_frame_interval(240)
        .with_threads(1)
}

fn plane_dims() -> [(usize, usize); 3] {
    let (w, h) = (W as usize, H as usize);
    [
        (w, h),
        (w.div_ceil(2), h.div_ceil(2)),
        (w.div_ceil(2), h.div_ceil(2)),
    ]
}

/// A 4:2:0 frame whose three planes are charged to `pool` while any handle
/// to the picture (including an encoder's clone) exists.
fn frame(pool: &StorageBudget, s: &Av1EncodeSettings, index: usize) -> Option<Frame> {
    let mut planes = Vec::new();
    for (c, (width, height)) in plane_dims().into_iter().enumerate() {
        let mut storage = pool.bytes(width * height).ok()?;
        for (i, v) in storage.as_mut_slice().iter_mut().enumerate() {
            *v = ((17 * (i % width) + 29 * (i / width) + 43 * index + 71 * c) % 251) as u8;
        }
        planes.push(OwnedPlane {
            data: PlaneData::AccountedU8(storage.freeze()),
            width,
            height,
            stride_bytes: width,
        });
    }
    let encoding = SampleEncoding::new(ChannelType::U8, 8, 0).unwrap();
    let mut interpretation = FrameInterpretation::new(
        FrameGeometry::full(W, H),
        Subsampling::Yuv420,
        s.siting,
        encoding,
        vec![PlaneRole::Luma, PlaneRole::Cb, PlaneRole::Cr],
    );
    interpretation.color = zencodec::frame::ColorClaim::new(None, None, Some(s.color));
    Some(Frame::new(
        Arc::new(OwnedPlanes::new(planes, encoding).unwrap()),
        Arc::new(interpretation),
        FrameTiming::at(Timestamp::new(index as i64 * 40, TB)).with_duration_ticks(40),
        Presentation::default(),
    ))
}

fn frame_bytes() -> usize {
    plane_dims().iter().map(|(w, h)| w * h).sum()
}

/// Sends frames until the first packet; returns how many were accepted first.
/// Every accepted frame's handle is dropped by the caller right away, and the
/// pool is checked: a retained clone would keep its charge.
fn accepted_before_first_packet(speed: u8, limit: usize) -> Option<usize> {
    let s = settings(speed);
    let pool = StorageBudget::new(frame_bytes(), 3);
    let mut encoder = Av1VideoEncoder::new(s.clone(), TB).unwrap();
    for index in 0..limit {
        let frame = frame(&pool, &s, index).expect("one caller frame admitted");
        match encoder.send(&frame, Some(&NoPulse)).unwrap() {
            SendStatus::Accepted => {}
            // Pushback only once a packet is ready: the queue held `index`.
            SendStatus::ReceivePending => {
                assert!(matches!(
                    encoder.receive(Some(&NoPulse)).unwrap(),
                    Receive::Ready(_)
                ));
                return Some(index);
            }
            other => panic!("speed {speed}: unexpected {other:?} at frame {index}"),
        }
        drop(frame);
        assert_eq!(
            (pool.reserved_bytes(), pool.reserved_objects()),
            (0, 0),
            "speed {speed}: an accepted frame left caller storage charged"
        );
        if let Receive::Ready(_) = encoder.receive(Some(&NoPulse)).unwrap() {
            return Some(index + 1);
        }
    }
    None
}

#[test]
fn accepted_frames_are_copied_and_the_first_packet_delay_is_undeclared() {
    // The pool admits exactly one caller frame: a cloning encoder would fail
    // the second allocation. zenavif copies planes into native frames.
    let mut delays = Vec::new();
    for speed in [10u8, 6] {
        let delay = accepted_before_first_packet(speed, 120);
        eprintln!(
            "zenavif media encoder, {W}x{H} 4:2:0, speed {speed}: first packet after {delay:?} accepted frames"
        );
        delays.push((speed, delay));
    }
    // Whatever the counts, they are observed only by sending frames: the
    // session offers no delay or retention value before the first frame.
    assert!(delays.iter().all(|(_, d)| d.is_some()), "{delays:?}");
}
