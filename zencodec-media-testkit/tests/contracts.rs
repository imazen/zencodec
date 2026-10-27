//! The toy reference implementations against every rule, and every
//! conformance check against the toy. Each test names the rule it enforces.

use std::sync::Arc;

use enough::{Stop, StopReason};
use zencodec::decode::AnimationFrameDecoder;
use zencodec::encode::AnimationFrameEncoder;
use zencodec::how_far::NoPulse;
use zencodec::time::{FrameDuration, Rounding, TimeBase, Timestamp};
use zencodec_io::{DriveFailure, SliceCursor};
use zenpixels::{PixelBuffer, PixelDescriptor};

use zencodec_media::adapt::{AdaptError, AnimationVideoEncoder, Gray8, VideoAnimationDecoder};
use zencodec_media::container::{
    DemuxEvent, DemuxStage, Demuxer, DynMuxer, Input, MuxHint, MuxTiming, Muxer, SeekMode,
    SeekScope, SeekStatus, SeekTarget, erase_muxer,
};
use zencodec_media::frame::Frame;
use zencodec_media::limits::MediaLimits;
use zencodec_media::packet::{Packet, RandomAccess};
use zencodec_media::session::{
    CodecConfig, DynVideoDecoder, DynVideoEncoder, Receive, SendStatus, SessionError, VideoDecoder,
    drain_decoder, erase_video_decoder, erase_video_encoder, send_packet_until_accepted,
};
use zencodec_media::track::{CodecId, CodecTag, TrackInfo, VideoTrackInfo};
use zencodec_media_testkit::toy::{
    ToyDecoder, ToyDemuxer, ToyEncoder, ToyError, ToyMuxer, toy_payload,
};

const W: u32 = 4;
const H: u32 = 3;
const PIX: usize = (W * H) as usize;

fn tb() -> TimeBase {
    TimeBase::new(1001, 30000).unwrap()
}

#[test]
fn exact_timing_default_refuses_without_starting_legacy_muxer() {
    use zencodec_media::session::UnsupportedMedia;
    let mut mux = ToyMuxer::new(false);
    assert!(matches!(
        Muxer::begin_exact_timing(&mut mux, &[], MuxHint::default()),
        Err(SessionError::Unsupported(UnsupportedMedia::ExactTiming))
    ));
    assert!(mux.pending_output().is_empty());
    let mut erased = erase_muxer(ToyMuxer::new(false));
    assert!(matches!(
        erased.begin_exact_timing(&[], MuxHint::forward_only()),
        Err(SessionError::Unsupported(UnsupportedMedia::ExactTiming))
    ));
    assert!(erased.pending_output().is_empty());
}

fn gray(v: u8) -> Vec<u8> {
    vec![v; PIX]
}

fn config() -> CodecConfig {
    CodecConfig::new(CodecId::Other(CodecTag::new("toy")), tb())
        .with_video(VideoTrackInfo::new(W, H))
}

fn packet(payload: Vec<u8>, pts: i64, ra: bool) -> Packet {
    Packet::from_vec(payload)
        .unwrap()
        .with_track(0)
        .with_pts(Timestamp::new(pts, tb()))
        .with_random_access(if ra {
            RandomAccess::Yes
        } else {
            RandomAccess::No
        })
}

/// Drive a demuxer over a slice to `Done`, handing each event to `on_event`.
fn demux_slice<D: Demuxer>(
    dem: &mut D,
    bytes: &[u8],
    window: usize,
    stop: &dyn zencodec::Pulse,
    mut on_event: impl FnMut(DemuxEvent) -> Result<(), SessionError<D::Error>>,
) -> Result<zencodec_io::DriveStats, DriveFailure<SessionError<D::Error>, std::convert::Infallible>>
{
    let mut cursor = SliceCursor::new(window);
    loop {
        let y = cursor.next(bytes, &mut DemuxStage(&mut *dem), stop)?;
        let done = y.is_finished();
        on_event(y.into_inner()).map_err(DriveFailure::Step)?;
        if done {
            return Ok(cursor.stats());
        }
    }
}

/// A TIVF file with `n` single-picture packets, pts = i, RAP every `keyint`.
fn toy_file(n: u32, keyint: u32, write_count: bool) -> Vec<u8> {
    let mut m = ToyMuxer::new(write_count);
    let track = TrackInfo::video(
        0,
        CodecId::Other(CodecTag::new("toy")),
        tb(),
        VideoTrackInfo::new(W, H),
    )
    .build()
    .unwrap();
    m.begin(&[track], MuxHint::default()).unwrap();
    let mut out = Vec::new();
    for i in 0..n {
        let p = packet(
            toy_payload(&[&gray(i as u8)], false),
            i64::from(i),
            i % keyint == 0,
        );
        m.write_packet(p, &NoPulse).unwrap();
        m.take_output(&mut out);
    }
    m.finish(&NoPulse).unwrap().apply_to(&mut out).unwrap();
    out
}

// ------------------------------------------------------ object safety ---

fn assert_send<T: Send>() {}
fn takes_dyn(
    _: &DynVideoDecoder,
    _: &DynVideoEncoder,
    _: &(dyn DynMuxer + Send),
    _: &(dyn Demuxer<Error = ToyError> + Send),
) {
}

#[test]
fn sessions_are_object_safe_and_send() {
    assert_send::<ToyDecoder>();
    assert_send::<ToyEncoder>();
    assert_send::<ToyMuxer>();
    assert_send::<ToyDemuxer>();
    assert_send::<Frame>();
    let dec = erase_video_decoder(ToyDecoder::new(2));
    let enc = erase_video_encoder(ToyEncoder::new(W, H, tb(), 1, 3));
    let mux: Box<dyn DynMuxer + Send> = erase_muxer(ToyMuxer::new(false));
    let dem = ToyDemuxer::new(MediaLimits::default());
    takes_dyn(&*dec, &*enc, &*mux, &dem);
    // Decoder state moves between threads (zenjxl-decoder #60's requirement).
    let handle = std::thread::spawn(move || {
        let mut dec = dec;
        dec.configure(&config(), &NoPulse).unwrap();
        dec
    });
    let _dec = handle.join().unwrap();
}

// ---------------------------------------------- backpressure and drain ---

#[test]
fn receive_pending_keeps_the_packet_and_guarantees_output() {
    let mut dec = ToyDecoder::new(2);
    dec.configure(&config(), &NoPulse).unwrap();
    let mut p0 = packet(toy_payload(&[&gray(1), &gray(2)], false), 0, true);
    let mut p1 = packet(toy_payload(&[&gray(3)], false), 1, false);
    let mut p2 = packet(toy_payload(&[&gray(4)], false), 2, false);
    assert_eq!(dec.send(&mut p0, &NoPulse).unwrap(), SendStatus::Accepted);
    assert!(p0.is_taken());
    // p1 releases p0's two pictures into the ready queue (one-packet reorder).
    assert_eq!(dec.send(&mut p1, &NoPulse).unwrap(), SendStatus::Accepted);
    assert!(!matches!(
        dec.receive(&NoPulse).unwrap(),
        Receive::NeedInput
    ));
    // Refill: now ready has 1 and hold has 1 => a third send is backpressured.
    let st = dec.send(&mut p2, &NoPulse).unwrap();
    assert_eq!(st, SendStatus::ReceivePending);
    assert!(
        !p2.is_taken(),
        "a backpressured packet is retained for retry"
    );
    assert!(
        matches!(dec.receive(&NoPulse).unwrap(), Receive::Ready(_)),
        "ReceivePending guarantees output"
    );
    assert_eq!(dec.send(&mut p2, &NoPulse).unwrap(), SendStatus::Accepted);
    // Drain: end_input is idempotent, EndOfStream is stable.
    dec.end_input().unwrap();
    dec.end_input().unwrap();
    let mut n = 0;
    loop {
        match dec.receive(&NoPulse).unwrap() {
            Receive::Ready(_) => n += 1,
            Receive::EndOfStream => break,
            _ => panic!("NeedInput after end_input"),
        }
    }
    assert_eq!(n, 2, "p1's and p2's pictures were still held");
    assert!(matches!(
        dec.receive(&NoPulse).unwrap(),
        Receive::EndOfStream
    ));
    assert!(matches!(
        dec.send(&mut packet(toy_payload(&[], false), 3, false), &NoPulse),
        Err(SessionError::Protocol(_))
    ));
}

#[test]
fn packets_are_not_frames_and_show_existing_shares_storage() {
    let mut dec = ToyDecoder::new(8);
    dec.configure(&config(), &NoPulse).unwrap();
    let mut frames: Vec<Frame> = Vec::new();
    fn feed(dec: &mut ToyDecoder, p: &mut Packet, frames: &mut Vec<Frame>) {
        send_packet_until_accepted(dec, p, &NoPulse, |f| {
            frames.push(f);
            Ok::<(), ()>(())
        })
        .unwrap();
        while let Receive::Ready(f) = dec.receive(&NoPulse).unwrap() {
            frames.push(f);
        }
    }
    feed(
        &mut dec,
        &mut packet(toy_payload(&[&gray(1), &gray(2)], false), 0, true),
        &mut frames,
    ); // 2 pictures
    assert!(frames.is_empty(), "frames come out one packet late");
    feed(
        &mut dec,
        &mut packet(toy_payload(&[], false), 1, false),
        &mut frames,
    ); // 0 pictures
    assert_eq!(frames.len(), 2);
    feed(
        &mut dec,
        &mut packet(toy_payload(&[], true), 2, false),
        &mut frames,
    ); // show existing
    assert_eq!(frames.len(), 2);
    drain_decoder(&mut dec, &NoPulse, |f| {
        frames.push(f);
        Ok::<(), ()>(())
    })
    .unwrap();
    assert_eq!(frames.len(), 3);
    let again = &frames[2];
    assert!(again.presentation().show_existing);
    assert!(
        again.shares_picture(&frames[1]),
        "the re-presentation shares the picture"
    );
    assert_ne!(
        again.timing().pts,
        frames[1].timing().pts,
        "but not the timing"
    );
    assert!(frames[0].presentation().keyframe && !frames[1].presentation().keyframe);
    assert_eq!(
        Arc::strong_count(frames[1].picture()),
        3,
        "two presentations plus the decoder's reference picture"
    );
    drop(dec);
    assert_eq!(
        Arc::strong_count(frames[1].picture()),
        2,
        "the decoder's reference is gone; both frames stay valid"
    );
    assert_eq!(frames[0].timing().packet_ordinal, None);
    assert_eq!(
        frames[2].map().unwrap().plane(0).unwrap().code(0, 0),
        Some(2)
    );
}

#[test]
fn an_old_mapping_survives_more_decoding_reset_and_decoder_drop() {
    let mut dec = ToyDecoder::new(8);
    dec.configure(&config(), &NoPulse).unwrap();
    dec.send(
        &mut packet(toy_payload(&[&gray(9)], false), 0, true),
        &NoPulse,
    )
    .unwrap();
    dec.end_input().unwrap();
    let Receive::Ready(first) = dec.receive(&NoPulse).unwrap() else {
        panic!()
    };
    let mapping = first.map().unwrap();
    let plane = mapping.plane(0).unwrap();
    // Keep the mapping while the decoder decodes more, resets and is dropped.
    dec.reset().unwrap();
    dec.send(
        &mut packet(toy_payload(&[&gray(5)], false), 0, true),
        &NoPulse,
    )
    .unwrap();
    dec.end_input().unwrap();
    let Receive::Ready(second) = dec.receive(&NoPulse).unwrap() else {
        panic!()
    };
    drop(dec);
    assert_eq!(plane.code(3, 2), Some(9));
    assert_eq!(second.map().unwrap().plane(0).unwrap().code(0, 0), Some(5));
    assert!(!first.shares_picture(&second));
}

#[test]
fn a_queued_frame_keeps_the_interpretation_it_was_decoded_under() {
    let mut dec = ToyDecoder::new(8);
    dec.configure(&config(), &NoPulse).unwrap();
    dec.send(
        &mut packet(toy_payload(&[&gray(1)], false), 0, true),
        &NoPulse,
    )
    .unwrap();
    // Epoch change before the queued frame was received.
    let cfg = config().with_epoch(1);
    dec.configure(&cfg, &NoPulse).unwrap();
    let mut late = packet(toy_payload(&[&gray(2)], false), 1, true).with_config_epoch(1);
    dec.send(&mut late, &NoPulse).unwrap();
    assert!(
        matches!(
            dec.send(
                &mut packet(toy_payload(&[&gray(3)], false), 2, true),
                &NoPulse
            ),
            Err(SessionError::Protocol(_))
        ),
        "a packet from the old epoch is refused"
    );
    dec.end_input().unwrap();
    let Receive::Ready(old) = dec.receive(&NoPulse).unwrap() else {
        panic!()
    };
    let Receive::Ready(new) = dec.receive(&NoPulse).unwrap() else {
        panic!()
    };
    assert_eq!(old.interpretation().config_epoch, 0);
    assert_eq!(new.interpretation().config_epoch, 1);
    assert!(!Arc::ptr_eq(old.interpretation(), new.interpretation()));
}

struct Tripped;
impl Stop for Tripped {
    fn check(&self) -> Result<(), StopReason> {
        Err(StopReason::Cancelled)
    }
}

#[test]
fn cancellation_is_terminal_until_reset() {
    let mut dec = erase_video_decoder(ToyDecoder::new(2));
    dec.configure(&config(), &NoPulse).unwrap();
    assert!(matches!(
        dec.receive(&zencodec::how_far::StopOnly::borrowed(&Tripped)),
        Err(SessionError::Cancelled(_))
    ));
    assert!(matches!(dec.receive(&NoPulse), Err(SessionError::Finished)));
    dec.reset().unwrap();
    assert!(matches!(dec.receive(&NoPulse), Ok(Receive::NeedInput)));
    // A codec error is terminal the same way.
    let mut bad = packet(vec![9, 0], 0, true);
    assert!(matches!(
        dec.send(&mut bad, &NoPulse),
        Err(SessionError::Codec(_))
    ));
    assert!(matches!(
        dec.send(&mut bad, &NoPulse),
        Err(SessionError::Finished)
    ));
}

// ------------------------------------------------------------ container ---

#[test]
fn demuxer_output_is_identical_for_every_window_and_truncation_is_typed() {
    let file = toy_file(7, 3, true);
    let mut baseline: Option<Vec<(i64, RandomAccess, Vec<u8>)>> = None;
    for window in [1usize, 7, 64, file.len()] {
        let mut dem = ToyDemuxer::new(MediaLimits::default());
        let mut got = Vec::new();
        let mut headers = 0;
        let stats = demux_slice(&mut dem, &file, window, &NoPulse, |ev| {
            match ev {
                DemuxEvent::Headers => headers += 1,
                DemuxEvent::Packet(p) => got.push((
                    p.pts().unwrap().ticks(),
                    p.random_access(),
                    p.bytes().to_vec(),
                )),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(headers, 1);
        assert_eq!(got.len(), 7);
        assert!(stats.steps >= 8);
        match &baseline {
            None => baseline = Some(got),
            Some(b) => assert_eq!(b, &got, "window {window}"),
        }
        assert_eq!(dem.tracks()[0].declared_packets(), Some(7));
    }
    // Truncated inside a packet: a codec error from the stage, not a driver violation.
    let mut dem = ToyDemuxer::new(MediaLimits::default());
    let cut = &file[..file.len() - 3];
    let err = demux_slice(&mut dem, cut, 5, &NoPulse, |_| Ok(())).unwrap_err();
    assert!(
        matches!(
            err,
            DriveFailure::Step(SessionError::Codec(ToyError::Truncated))
        ),
        "{err:?}"
    );
    // A packet above the limit is a typed limit error, checked before allocation.
    let mut dem = ToyDemuxer::new(MediaLimits::default().with_max_packet_bytes(4));
    let err = demux_slice(&mut dem, &file, 64, &NoPulse, |_| Ok(())).unwrap_err();
    assert!(
        matches!(err, DriveFailure::Step(SessionError::Limit(_))),
        "{err:?}"
    );
    // Unrequested input (wrong offset) is ignored: nothing consumed, a request named.
    let mut dem = ToyDemuxer::new(MediaLimits::default());
    let p = dem.step(Input::new(5, &file[5..]), &NoPulse).unwrap();
    assert_eq!(p.consumed, 0);
    assert!(matches!(p.event, DemuxEvent::Need));
    assert!(!dem.requested().is_empty());
}

#[test]
fn muxer_bytes_do_not_depend_on_how_they_are_drained() {
    let track = TrackInfo::video(
        0,
        CodecId::Other(CodecTag::new("toy")),
        tb(),
        VideoTrackInfo::new(W, H),
    )
    .build()
    .unwrap();
    let run = |drain: u8| -> Vec<u8> {
        let mut m = ToyMuxer::new(true);
        assert_eq!(
            m.begin(std::slice::from_ref(&track), MuxHint::default())
                .unwrap(),
            MuxTiming::streaming_with_patches()
        );
        let mut out = Vec::new();
        let mut rng = 12345u32;
        for i in 0..9u32 {
            m.write_packet(
                packet(
                    toy_payload(&[&gray(i as u8)], false),
                    i64::from(i),
                    i % 4 == 0,
                ),
                &NoPulse,
            )
            .unwrap();
            match drain {
                0 => {}
                1 => {
                    let n = m.pending_output().len();
                    out.extend_from_slice(m.pending_output());
                    m.consume_output(n);
                }
                2 => {
                    rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
                    let n = (rng >> 8) as usize % (m.pending_output().len() + 2);
                    out.extend_from_slice(&m.pending_output()[..n.min(m.pending_output().len())]);
                    m.consume_output(n); // may exceed pending: clamps
                }
                _ => m.take_output(&mut out),
            }
        }
        m.finish(&NoPulse).unwrap().apply_to(&mut out).unwrap();
        out
    };
    let undrained = run(0);
    assert_eq!(run(1), undrained);
    assert_eq!(run(2), undrained);
    assert_eq!(run(3), undrained);
    assert_eq!(
        u32::from_le_bytes(undrained[16..20].try_into().unwrap()),
        9,
        "the patch landed"
    );
    // A patching layout is refused for a forward-only destination at begin.
    let mut m = ToyMuxer::new(true);
    assert!(matches!(
        m.begin(std::slice::from_ref(&track), MuxHint::forward_only()),
        Err(SessionError::Unsupported(_))
    ));
    let mut m = ToyMuxer::new(false);
    assert_eq!(
        m.begin(std::slice::from_ref(&track), MuxHint::forward_only())
            .unwrap(),
        MuxTiming::streaming()
    );
    assert!(m.finish(&NoPulse).unwrap().patches.is_empty());
    // Begin twice is a protocol violation; a packet on another clock is refused.
    let mut m = ToyMuxer::new(false);
    m.begin(std::slice::from_ref(&track), MuxHint::default())
        .unwrap();
    assert!(matches!(
        m.begin(std::slice::from_ref(&track), MuxHint::default()),
        Err(SessionError::Protocol(_))
    ));
    let wrong = Packet::from_vec(vec![1])
        .unwrap()
        .with_track(0)
        .with_pts(Timestamp::new(0, TimeBase::MILLIS));
    assert!(matches!(
        m.write_packet(wrong, &NoPulse),
        Err(SessionError::Protocol(_))
    ));
}

#[test]
fn malformed_finish_does_not_partially_modify_the_output() {
    use zencodec_media::container::{MuxFinish, Patch, PatchError};
    let original = b"prefix".to_vec();
    for bad in [
        Patch {
            offset: 99,
            bytes: vec![1],
        },
        Patch {
            offset: u64::MAX,
            bytes: vec![1, 2],
        },
    ] {
        let mut output = original.clone();
        let finish = MuxFinish::new(
            b"tail".to_vec(),
            vec![
                Patch {
                    offset: 0,
                    bytes: b"BAD".to_vec(),
                },
                bad,
            ],
        );
        assert_eq!(finish.apply_to(&mut output), Err(PatchError::OutOfRange));
        assert_eq!(
            output, original,
            "validate every patch before applying any effect"
        );
    }
}

// ----------------------------------------------------------------- seek ---

#[test]
fn seek_by_time_reports_the_actual_pts_and_random_access_evidence() {
    let file = toy_file(10, 4, false); // RAPs at 0, 4, 8
    let mut dem = ToyDemuxer::new(MediaLimits::default());
    let mut cursor = SliceCursor::new(16);
    assert!(matches!(
        cursor
            .next(&file, &mut DemuxStage(&mut dem), &NoPulse)
            .unwrap()
            .into_inner(),
        DemuxEvent::Headers
    ));
    // Target 6.5 frames in, asked in milliseconds: the demuxer compares exactly.
    let target_ms = Timestamp::new(6, tb())
        .rescale(TimeBase::MILLIS, Rounding::Nearest)
        .unwrap();
    let seek = |dem: &mut ToyDemuxer, cursor: &mut SliceCursor, mode, scope| {
        let t = SeekTarget::new(0, target_ms, mode, scope);
        match dem.seek(&t).unwrap() {
            SeekStatus::Positioned(r) => r,
            SeekStatus::Scanning => loop {
                if let DemuxEvent::Seeked(r) = cursor
                    .next(&file, &mut DemuxStage(&mut *dem), &NoPulse)
                    .unwrap()
                    .into_inner()
                {
                    break r;
                }
            },
            _ => unreachable!(),
        }
    };
    let r = seek(&mut dem, &mut cursor, SeekMode::Nearest, SeekScope::Any);
    assert_eq!(r.pts.ticks(), 6);
    assert_eq!(r.random_access, RandomAccess::No);
    let r = seek(
        &mut dem,
        &mut cursor,
        SeekMode::AtOrBefore,
        SeekScope::RandomAccessPoints,
    );
    assert_eq!(
        (r.pts.ticks(), r.ordinal, r.random_access),
        (4, 4, RandomAccess::Yes)
    );
    let r = seek(
        &mut dem,
        &mut cursor,
        SeekMode::AtOrAfter,
        SeekScope::RandomAccessPoints,
    );
    assert_eq!((r.pts.ticks(), r.ordinal), (8, 8));
    // After a seek the packets resume at the position, and the decoder is
    // reset + reconfigured by the driver; decode work is reported.
    let r = seek(
        &mut dem,
        &mut cursor,
        SeekMode::AtOrBefore,
        SeekScope::RandomAccessPoints,
    );
    assert_eq!(r.ordinal, 4);
    let mut dec = ToyDecoder::new(8);
    dec.reset().unwrap();
    dec.configure(&CodecConfig::from(&dem.tracks()[0]), &NoPulse)
        .unwrap();
    let (mut packets_read, mut discarded, mut shown) = (0u64, 0u64, Vec::new());
    loop {
        match cursor
            .next(&file, &mut DemuxStage(&mut dem), &NoPulse)
            .unwrap()
            .into_inner()
        {
            DemuxEvent::Packet(mut p) => {
                packets_read += 1;
                send_packet_until_accepted(&mut dec, &mut p, &NoPulse, |f| {
                    if f.timing().pts.unwrap().compare(target_ms) == std::cmp::Ordering::Less {
                        discarded += 1;
                    } else {
                        shown.push(f.timing().pts.unwrap().ticks());
                    }
                    Ok::<(), ()>(())
                })
                .unwrap();
            }
            DemuxEvent::Done => break,
            _ => {}
        }
    }
    drain_decoder(&mut dec, &NoPulse, |f| {
        shown.push(f.timing().pts.unwrap().ticks());
        Ok::<(), ()>(())
    })
    .unwrap();
    assert_eq!(packets_read, 6);
    assert_eq!(
        discarded, 2,
        "frames 4 and 5 decoded and discarded to reach 6"
    );
    assert_eq!(shown, vec![6, 7, 8, 9]);
}

// ------------------------------------------------- animation over video ---

#[test]
fn animation_round_trips_through_the_video_model_with_exact_timing() {
    let frames: Vec<PixelBuffer> = (0..5u8)
        .map(|i| {
            let mut b = PixelBuffer::new(W, H, PixelDescriptor::GRAY8_SRGB);
            for y in 0..H {
                for (x, px) in b.as_slice_mut().row_mut(y).iter_mut().enumerate() {
                    *px = i * 40 + y as u8 * 4 + x as u8;
                }
            }
            b
        })
        .collect();
    let durations: Vec<FrameDuration> = (1..=5u64)
        .map(|n| FrameDuration::new(n * 1001, 30000).unwrap())
        .collect();

    // Encode: the animation encoder is a layer over VideoEncoder + Muxer, and
    // PR 149's begin/pending_output are the muxer's pull buffer.
    let encode = |drain: bool| -> (zencodec::encode::AnimationOutputTiming, Vec<u8>, Vec<usize>) {
        let enc = ToyEncoder::new(W, H, tb(), 1, 3);
        let mut a =
            AnimationVideoEncoder::new(enc, ToyMuxer::new(false), Gray8::default(), 1).unwrap();
        let timing = a.begin(W, H, PixelDescriptor::GRAY8_SRGB, Some(5)).unwrap();
        let mut out = Vec::new();
        let mut pending_after_push = Vec::new();
        if drain {
            a.take_output(&mut out);
        }
        for (f, d) in frames.iter().zip(&durations) {
            a.push_frame_exact(f.as_slice(), *d, None).unwrap();
            pending_after_push.push(out.len() + a.pending_output().len());
            if drain {
                a.take_output(&mut out);
            }
        }
        out.extend_from_slice(a.finish(None).unwrap().data());
        (timing, out, pending_after_push)
    };
    let (timing, drained, points) = encode(true);
    let (_, undrained, _) = encode(false);
    assert_eq!(timing.lag_frames(), Some(1));
    assert_eq!(drained, undrained, "draining never changes the bytes");
    // With a lag of one frame, bytes grow between any two points two frames apart.
    for w in points.windows(2) {
        assert!(
            w[1] > w[0],
            "no bytes became pending across the lag: {points:?}"
        );
    }
    // A fractional duration on this clock is refused as AnimationTiming.
    let enc = ToyEncoder::new(W, H, tb(), 1, 3);
    let mut a = AnimationVideoEncoder::new(enc, ToyMuxer::new(false), Gray8::default(), 1).unwrap();
    let err = a.push_frame(frames[0].as_slice(), 40, None).unwrap_err();
    assert!(matches!(
        err,
        AdaptError::Unsupported(zencodec::UnsupportedOperation::AnimationTiming)
    ));

    // Decode through the real AnimationFrameDecoder trait.
    let dem = ToyDemuxer::new(MediaLimits::default());
    let dec = ToyDecoder::new(4);
    let mut anim = VideoAnimationDecoder::new(
        drained.clone().into(),
        dem,
        dec,
        Gray8::default(),
        durations[4],
        5,
    );
    let mut got = Vec::new();
    while let Some(f) = anim.render_next_frame(None).unwrap() {
        got.push((f.duration(), f.frame_index(), f.pixels().row(1).to_vec()));
    }
    assert_eq!(got.len(), 5);
    assert_eq!(anim.info().width, W);
    for (i, (d, idx, row)) in got.iter().enumerate() {
        assert_eq!(*d, durations[i], "frame {i} duration from adjacent pts");
        assert_eq!(*idx, i as u32);
        assert_eq!(row, &frames[i].as_slice().row(1).to_vec());
    }
    assert_eq!(anim.frames_decoded(), 5);
}

#[test]
fn declared_frame_count_is_binding() {
    let enc = ToyEncoder::new(W, H, tb(), 1, 3);
    let mut a = AnimationVideoEncoder::new(enc, ToyMuxer::new(false), Gray8::default(), 1).unwrap();
    a.begin(W, H, PixelDescriptor::GRAY8_SRGB, Some(2)).unwrap();
    let b = PixelBuffer::new(W, H, PixelDescriptor::GRAY8_SRGB);
    a.push_frame_exact(b.as_slice(), FrameDuration::new(1001, 30000).unwrap(), None)
        .unwrap();
    assert!(matches!(
        a.finish(None),
        Err(AdaptError::FrameCountMismatch)
    ));
}

#[test]
fn finish_preflight_supports_tail_patches_and_ordered_overlaps_and_rejects_overflow() {
    use zencodec_media::container::{MuxFinish, Patch, PatchError};
    let finish = MuxFinish::new(
        b"tail".to_vec(),
        vec![
            Patch {
                offset: 3,
                bytes: b"XYZ".to_vec(),
            },
            Patch {
                offset: 4,
                bytes: b"!".to_vec(),
            },
            Patch {
                offset: 7,
                bytes: vec![],
            },
        ],
    );
    assert_eq!(finish.validate(3), Ok(7));
    let mut output = b"pre".to_vec();
    finish.apply_to(&mut output).unwrap();
    assert_eq!(&output, b"preX!Zl");
    assert_eq!(
        MuxFinish::new(vec![1], vec![]).validate(u64::MAX),
        Err(PatchError::OutOfRange)
    );
    let finish = MuxFinish::new(
        vec![],
        vec![Patch {
            offset: 4,
            bytes: vec![],
        }],
    );
    assert_eq!(finish.validate(3), Err(PatchError::OutOfRange));
}

#[test]
#[allow(deprecated)]
fn deprecated_finish_helper_uses_the_same_transactional_validation() {
    use zencodec_media::container::{MuxFinish, Patch, PatchError, apply_finish};
    let mut output = b"keep".to_vec();
    let bad = MuxFinish::new(
        b"tail".to_vec(),
        vec![Patch {
            offset: 9,
            bytes: vec![0],
        }],
    );
    assert_eq!(apply_finish(&mut output, bad), Err(PatchError::OutOfRange));
    assert_eq!(&output, b"keep");
    apply_finish(&mut output, MuxFinish::new(b"tail".to_vec(), vec![])).unwrap();
    assert_eq!(&output, b"keeptail");
}
