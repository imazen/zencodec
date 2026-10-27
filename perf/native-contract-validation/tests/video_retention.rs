//! Witness: `VideoEncoder` lets an accepted frame be cloned (the caller's
//! accounted picture stays charged inside the encoder) or copied (released
//! when the caller drops its handle), and it declares no delay before the
//! first packet. Two conforming encoders producing the same packets therefore
//! need different caller frame storage, and the caller learns which only when
//! a later frame allocation fails.
use std::{collections::VecDeque, error::Error, fmt, sync::Arc};
use zencodec::how_far::NoPulse;
use zencodec::time::{TimeBase, Timestamp};
use zencodec_io::storage::StorageBudget;
use zencodec_media::{
    frame::{
        ChromaSiting, Frame, FrameGeometry, FrameInterpretation, FrameTiming, OwnedPlane,
        OwnedPlanes, PlaneData, PlaneRole, PlaneSamples, Presentation, SampleEncoding, Subsampling,
    },
    packet::Packet,
    session::{CodecConfig, Receive, SendStatus, SessionError, VideoEncoder},
};

#[derive(Debug)]
struct ToyError;
impl fmt::Display for ToyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("toy encoder")
    }
}
impl Error for ToyError {}

/// One packet per frame once more than `delay` frames are held. With `clone`
/// it keeps the caller's frame handle; otherwise it copies the samples.
struct Toy {
    delay: usize,
    clone: bool,
    held: VecDeque<(Option<Frame>, Vec<u8>, Timestamp)>,
    ended: bool,
}
impl Toy {
    fn new(delay: usize, clone: bool) -> Self {
        Self {
            delay,
            clone,
            held: VecDeque::new(),
            ended: false,
        }
    }
}
impl VideoEncoder for Toy {
    type Error = ToyError;
    fn send(
        &mut self,
        frame: &Frame,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<SendStatus, SessionError<ToyError>> {
        pulse.check()?;
        if self.held.len() > self.delay {
            return Ok(SendStatus::ReceivePending);
        }
        let pts = frame.timing().pts.ok_or(SessionError::Codec(ToyError))?;
        let mut copy = Vec::new();
        if !self.clone {
            let mapped = frame.map().map_err(|_| SessionError::Codec(ToyError))?;
            for i in 0..mapped.plane_count() {
                let plane = mapped.plane(i).ok_or(SessionError::Codec(ToyError))?;
                for y in 0..plane.height() {
                    if let Some(PlaneSamples::U8(row)) = plane.row(y) {
                        copy.extend_from_slice(row);
                    }
                }
            }
        }
        self.held
            .push_back((self.clone.then(|| frame.clone()), copy, pts));
        Ok(SendStatus::Accepted)
    }
    fn receive(
        &mut self,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<Receive<Packet>, SessionError<ToyError>> {
        pulse.check()?;
        if self.held.len() > self.delay || (self.ended && !self.held.is_empty()) {
            let (_, _, pts) = self.held.pop_front().unwrap();
            let packet = Packet::from_vec(vec![1])
                .map_err(|_| SessionError::Codec(ToyError))?
                .with_pts(pts);
            return Ok(Receive::Ready(packet));
        }
        Ok(if self.ended {
            Receive::EndOfStream
        } else {
            Receive::NeedInput
        })
    }
    fn end_input(&mut self) -> Result<(), SessionError<ToyError>> {
        self.ended = true;
        Ok(())
    }
    fn output_config(&self) -> Option<&CodecConfig> {
        None
    }
}

/// A 4x2 gray frame whose samples are charged to `pool` until every handle
/// (including an encoder's clone) is dropped.
fn frame(pool: &StorageBudget, i: i64) -> Option<Frame> {
    let mut storage = pool.bytes(8).ok()?;
    storage.as_mut_slice().fill(i as u8);
    let picture = OwnedPlanes::new(
        vec![OwnedPlane {
            data: PlaneData::AccountedU8(storage.freeze()),
            width: 4,
            height: 2,
            stride_bytes: 4,
        }],
        SampleEncoding::U8,
    )
    .unwrap();
    let interpretation = FrameInterpretation::new(
        FrameGeometry::full(4, 2),
        Subsampling::Mono,
        ChromaSiting::Unknown,
        SampleEncoding::U8,
        vec![PlaneRole::Luma],
    );
    let base = TimeBase::new(1, 30).unwrap();
    Some(Frame::new(
        Arc::new(picture),
        Arc::new(interpretation),
        FrameTiming::at(Timestamp::new(i, base)),
        Presentation::default(),
    ))
}

#[derive(Debug, PartialEq)]
enum Outcome {
    /// Every frame encoded; packets received.
    Done(usize),
    /// Allocating this frame index failed; packets received before it.
    PoolExhausted { frame: usize, packets: usize },
}

/// Feeds `frames` frames from a pool admitting `capacity` frames at once. The
/// caller drops each handle after `Accepted` and drains packets as they come.
fn transcode(enc: &mut Toy, capacity: usize, frames: usize) -> Outcome {
    let pool = StorageBudget::new(8 * capacity, capacity);
    let mut packets = 0;
    for i in 0..frames {
        let Some(frame) = frame(&pool, i as i64) else {
            return Outcome::PoolExhausted { frame: i, packets };
        };
        loop {
            match enc.send(&frame, &NoPulse).unwrap() {
                SendStatus::Accepted => break,
                SendStatus::ReceivePending => {
                    if let Receive::Ready(_) = enc.receive(&NoPulse).unwrap() {
                        packets += 1;
                    }
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        drop(frame);
        while let Receive::Ready(_) = enc.receive(&NoPulse).unwrap() {
            packets += 1;
        }
    }
    enc.end_input().unwrap();
    while let Receive::Ready(_) = enc.receive(&NoPulse).unwrap() {
        packets += 1;
    }
    Outcome::Done(packets)
}

#[test]
fn caller_frame_storage_depends_on_undeclared_encoder_retention() {
    // Nothing on the trait distinguishes these before the first frame.
    for enc in [Toy::new(3, false), Toy::new(3, true), Toy::new(0, true)] {
        assert!(enc.output_config().is_none() && enc.output_fidelity().is_none());
    }
    // Copying with a three-frame delay: one caller frame at a time suffices.
    assert_eq!(transcode(&mut Toy::new(3, false), 1, 6), Outcome::Done(6));
    // Cloning without delay: also one.
    assert_eq!(transcode(&mut Toy::new(0, true), 1, 6), Outcome::Done(6));
    // Cloning with the same three-frame delay needs four caller frames; with
    // two, the third allocation fails before any packet exists.
    assert_eq!(
        transcode(&mut Toy::new(3, true), 2, 6),
        Outcome::PoolExhausted {
            frame: 2,
            packets: 0
        }
    );
    assert_eq!(transcode(&mut Toy::new(3, true), 4, 6), Outcome::Done(6));
}
