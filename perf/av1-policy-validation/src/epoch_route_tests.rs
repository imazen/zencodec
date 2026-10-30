use super::*;
use crate::fixtures::*;
use enough::StopReason;
use std::{cell::Cell, convert::Infallible, path::PathBuf, sync::atomic::AtomicBool};
use zenavif::media::Av1VideoDecoder;
use zencodec::{
    operation::{
        Decision, DecisionCode, Decline, FrameRouteFacts, PolicyFailure, Question, Selection,
    },
    storage::StorageError,
    time::Timestamp,
};
use zencodec_media::session::VideoDecoder;

fn native_stream(threads: u32) -> Vec<Frame> {
    native_stream_observed(threads, |_, _| {})
}

fn native_stream_observed(threads: u32, mut observed: impl FnMut(&Frame, u32)) -> Vec<Frame> {
    let mut source = settings(8, Subsampling::Yuv420, false);
    let mut variants = vec![(source.clone(), TB), (source.clone(), TB)];
    source.width = 19;
    source.height = 15;
    variants.push((source.clone(), TB));
    source.color = Cicp::new(9, 16, 9, true);
    variants.push((source.clone(), TB));
    source.bit_depth = 10;
    source.subsampling = Subsampling::Yuv422;
    variants.push((source.clone(), TB));
    variants.push((source, TimeBase::hz(90_000).unwrap()));
    let mut settings = rav1d_safe::Settings::default();
    settings.threads = threads;
    let mut decoder = Av1VideoDecoder::with_settings(settings).unwrap();
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    for (group, (settings, clock)) in variants.into_iter().enumerate() {
        let frames: Vec<_> = (0..3).map(|i| frame(&settings, group * 3 + i)).collect();
        expected.extend(frames.iter().map(samples));
        let (mut config, packets) = encode(settings, &frames);
        config.epoch = 10 + group as u32;
        config.time_base = clock;
        decoder.configure(&config, Some(&NoPulse)).unwrap();
        for (index, packet) in packets.into_iter().enumerate() {
            let pts = (group * 3 + index) as i64 * if clock == TB { 40 } else { 3600 };
            let mut packet = packet
                .with_pts(Timestamp::new(pts, clock))
                .with_duration_ticks(if clock == TB { 40 } else { 3600 })
                .with_config_epoch(config.epoch)
                .with_ordinal(7)
                .with_input_offset(1000 + (group * 3 + index) as u64);
            while decoder.send(&mut packet, Some(&NoPulse)).unwrap() != SendStatus::Accepted {
                let Receive::Ready(frame) = decoder.receive(Some(&NoPulse)).unwrap() else {
                    panic!("native decoder progress promise")
                };
                observed(&frame, config.epoch);
                actual.push(frame);
            }
        }
    }
    decoder.end_input().unwrap();
    loop {
        match decoder.receive(Some(&NoPulse)).unwrap() {
            Receive::Ready(frame) => {
                observed(&frame, 15);
                actual.push(frame);
            }
            Receive::EndOfStream => break,
            other => panic!("native decoder failed to drain: {other:?}"),
        }
    }
    drop(decoder);
    assert_eq!(actual.len(), expected.len());
    for (index, (frame, expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(samples(frame), expected);
        assert_eq!(frame.interpretation().config_epoch, 10 + (index / 3) as u32);
        assert_eq!(frame.timing().packet_ordinal, Some(7));
        assert_eq!(frame.timing().input_offset, Some(1000 + index as u64));
    }
    actual
}

#[test]
fn epoch_route_native_changes_prepare_once_preserve_input_context_and_drain_before_output_wait() {
    let root = PathBuf::from(
        std::env::var_os("AV1_POLICY_ARTIFACTS").expect("persistent artifact directory required"),
    );
    std::fs::create_dir_all(&root).unwrap();
    for threads in [1, 4] {
        let frames = native_stream(threads);
        let storage = StorageBudget::new(1 << 20, 64);
        let live = Arc::new(AtomicUsize::new(0));
        let mut seen = Vec::<(FrameRouteFacts, FrameRoute)>::new();
        let resolver = |question: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::FrameRoute {
                interpretation,
                facts,
                choices,
            } = question
            else {
                panic!("encoding transition question")
            };
            assert_eq!(
                facts.input,
                FrameFacts::new(interpretation, facts.input.timing)
            );
            let choice = choices.first().unwrap();
            if let Some((previous, _)) = seen.last() {
                assert_eq!(facts.output.previous, Some(previous.input));
            } else {
                assert!(facts.output.previous.is_none());
            }
            assert!(!facts.output.output_committed);
            assert_eq!(
                choice.value().admitted_encoded_bytes,
                storage.reserved_bytes()
            );
            seen.push((*facts, choice.value().route));
            Ok(Decision::choose(choice))
        };
        let mut operation = Candidate::new(resolver, true, false, storage.clone(), live.clone());
        for frame in &frames {
            operation.push(frame, Some(&NoPulse)).unwrap();
        }
        assert_eq!(operation.report().dropped_decisions(), 2);
        for record in operation.report().decisions() {
            let Some(Selection::FrameRoute(selected)) = record.selected else {
                panic!("route record lost")
            };
            assert_eq!(selected.fidelity, Some(Fidelity::Lossless));
            assert!(selected.workspace_upper_bound.is_none());
        }
        let segments = operation.finish(Some(&NoPulse)).unwrap();
        assert_eq!(segments.len(), 5);
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert_eq!(seen.len(), frames.len());
        let mut at = 0;
        for (epoch, segment) in segments.iter().enumerate() {
            let count = if epoch == 0 { 6 } else { 3 };
            assert_eq!(segment.inputs.len(), count);
            assert_eq!(segment.config.epoch, epoch as u32);
            assert_eq!(
                segment.config.time_base,
                frames[at].timing().pts.unwrap().base()
            );
            assert_eq!(
                segment.config.video.unwrap().color,
                frames[at].interpretation().color.resolved
            );
            for (packet, input) in segment.packets.iter().zip(&segment.inputs) {
                assert_eq!(packet.pts(), input.pts);
                assert_eq!(packet.duration_ticks(), input.duration_ticks);
                assert_eq!(packet.config_epoch(), epoch as u32);
            }
            assert_eq!(segment.packets.len(), count);
            oracle_on_clock(
                &root,
                &format!("epoch-{threads}-{epoch}"),
                &segment.settings,
                &segment.packets,
                &frames[at..at + count],
                segment.config.time_base,
            );
            at += count;
        }
        for (index, (_, route)) in seen.iter().enumerate() {
            assert_eq!(
                *route,
                if [0, 6, 9, 12, 15].contains(&index) {
                    FrameRoute::BeginSegment
                } else {
                    FrameRoute::Continue
                }
            );
        }
        // A stalled output consumer holds only accounted compressed payloads.
        let retained: Vec<_> = segments
            .iter()
            .flat_map(|s| s.packets.iter().cloned())
            .collect();
        let bytes: usize = retained.iter().map(Packet::len).sum();
        drop(segments);
        drop(frames);
        assert_eq!(storage.reserved_bytes(), bytes);
        assert_eq!(live.load(Ordering::SeqCst), 0);
        drop(retained);
        assert_eq!(
            (storage.reserved_bytes(), storage.reserved_objects()),
            (0, 0)
        );
    }
}

#[test]
fn epoch_route_fixed_committed_output_refuses_before_new_frame_acceptance() {
    let frames = native_stream(1);
    let live = Arc::new(AtomicUsize::new(0));
    let storage = StorageBudget::new(1 << 20, 64);
    let calls = Cell::new(0);
    let mut operation = Candidate::new(
        |question: Question<'_>| -> Result<Decision, At<Infallible>> {
            calls.set(calls.get() + 1);
            let Question::FrameRoute { facts, choices, .. } = question else {
                panic!()
            };
            assert_eq!(facts.output.output_committed, calls.get() > 1);
            if facts.input.geometry.coded_width == 19 {
                assert_eq!(choices.iter().count(), 0);
            }
            Ok(Decision::UsePreset)
        },
        false,
        false,
        storage.clone(),
        live.clone(),
    );
    operation.push(&frames[0], Some(&NoPulse)).unwrap();
    // The publisher owns these irreversible bytes, outside the operation.
    // A header is committed even though no encoded picture has been sent.
    let mut published = Vec::new();
    operation
        .publish_header(
            |config| {
                let video = config.video.unwrap();
                published.extend(ivf_header(
                    video.coded_width,
                    video.coded_height,
                    config.time_base,
                    6,
                ));
                Ok(())
            },
            Some(&NoPulse),
        )
        .unwrap();
    let header = published.clone();
    assert_eq!(header.len(), 32);
    assert_eq!(&header[12..16], &[17, 0, 13, 0]);
    for frame in &frames[1..6] {
        operation.push(frame, Some(&NoPulse)).unwrap();
    }
    let error = operation.push(&frames[6], Some(&NoPulse)).unwrap_err();
    let policy = zencodec::find_cause::<PolicyFailure>(&*error).unwrap();
    assert_eq!(policy.record().offered, 0);
    assert_eq!(policy.record().code, DecisionCode::Declined);
    assert!(operation.is_failed());
    assert_eq!(calls.get(), 7);
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(
        (storage.reserved_bytes(), storage.reserved_objects()),
        (0, 0)
    );
    assert!(operation.push(&frames[7], Some(&NoPulse)).is_err());
    assert_eq!(calls.get(), 7);
    assert_eq!(published, header);
}

#[test]
fn epoch_route_fixed_header_refuses_same_size_interpretation_and_clock_changes() {
    let frames = native_stream(1);
    // These are actual decoded frames, not relabeled synthetic owners. All
    // pairs have equal geometry: dimensions alone cannot authorize Continue.
    for (before, after) in [(6, 9), (9, 12), (12, 15)] {
        let old = &frames[before];
        let incoming = &frames[after];
        assert_eq!(
            old.interpretation().geometry,
            incoming.interpretation().geometry
        );
        let old_pixels = samples(old);
        let old_interpretation = old.interpretation().clone();
        let storage = StorageBudget::new(1 << 20, 64);
        let live = Arc::new(AtomicUsize::new(0));
        let calls = Cell::new(0);
        let mut operation = Candidate::new(
            |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                calls.set(calls.get() + 1);
                let Question::FrameRoute {
                    interpretation,
                    facts,
                    choices,
                } = question
                else {
                    panic!("frame route expected")
                };
                if calls.get() == 2 {
                    assert_eq!(interpretation, incoming.interpretation().as_ref());
                    assert_eq!(
                        facts.output.previous,
                        Some(FrameFacts::new(old.interpretation(), old.timing(),))
                    );
                    assert!(facts.output.output_committed);
                    assert_eq!(choices.iter().count(), 0);
                    // No replacement encoder may be prepared for this fixed
                    // destination, even if the callback would prefer one.
                    assert_eq!(live.load(Ordering::SeqCst), 1);
                }
                Ok(Decision::UsePreset)
            },
            false,
            false,
            storage.clone(),
            live.clone(),
        );
        operation.push(old, Some(&NoPulse)).unwrap();
        let mut published = None;
        operation
            .publish_header(
                |config| {
                    published = Some(config.clone());
                    Ok(())
                },
                Some(&NoPulse),
            )
            .unwrap();
        let published = published.unwrap();
        let error = operation.push(incoming, Some(&NoPulse)).unwrap_err();
        let failure = zencodec::find_cause::<PolicyFailure>(&*error).unwrap();
        assert_eq!(failure.record().offered, 0);
        assert_eq!(failure.record().code, DecisionCode::Declined);
        assert!(operation.is_failed());
        assert!(operation.push(old, Some(&NoPulse)).is_err());
        assert_eq!(calls.get(), 2);
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert_eq!(
            (storage.reserved_bytes(), storage.reserved_objects()),
            (0, 0)
        );
        assert_eq!(
            published.video.unwrap().color,
            old.interpretation().color.resolved
        );
        assert_eq!(published.time_base, old.timing().pts.unwrap().base());
        assert_eq!(old.interpretation(), &old_interpretation);
        assert_eq!(samples(old), old_pixels);
    }
}

struct StopAt {
    stopped: AtomicBool,
    reason: StopReason,
}
impl Stop for StopAt {
    fn check(&self) -> Result<(), StopReason> {
        if self.stopped.load(Ordering::SeqCst) {
            Err(self.reason)
        } else {
            Ok(())
        }
    }
}

#[test]
fn epoch_route_stop_at_preparation_transition_and_finish_keeps_reason_and_releases_owners() {
    let frames = native_stream(1);
    for reason in [StopReason::Cancelled, StopReason::TimedOut] {
        for point in 0..4 {
            let storage = StorageBudget::new(1 << 20, 64);
            let live = Arc::new(AtomicUsize::new(0));
            let stop = StopAt {
                stopped: AtomicBool::new(point == 0),
                reason,
            };
            let calls = Cell::new(0);
            let mut operation = Candidate::new(
                |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                    calls.set(calls.get() + 1);
                    let Question::FrameRoute { facts, .. } = question else {
                        panic!()
                    };
                    if point == 1 || point == 2 && facts.input.geometry.coded_width == 19 {
                        // The real replacement is retained beside the old encoder.
                        assert_eq!(live.load(Ordering::SeqCst), if point == 1 { 1 } else { 2 });
                        stop.stopped.store(true, Ordering::SeqCst);
                    }
                    Ok(Decision::UsePreset)
                },
                true,
                false,
                storage.clone(),
                live.clone(),
            );
            let error = if point == 3 {
                for frame in &frames[..6] {
                    operation
                        .push(frame, Some(&zencodec::how_far::StopOnly::borrowed(&stop)))
                        .unwrap();
                }
                stop.stopped.store(true, Ordering::SeqCst);
                operation
                    .finish(Some(&zencodec::how_far::StopOnly::borrowed(&stop)))
                    .err()
                    .expect("finish must stop")
            } else {
                let index = if point == 2 {
                    for frame in &frames[..6] {
                        operation
                            .push(frame, Some(&zencodec::how_far::StopOnly::borrowed(&stop)))
                            .unwrap();
                    }
                    6
                } else {
                    0
                };
                let error = operation
                    .push(
                        &frames[index],
                        Some(&zencodec::how_far::StopOnly::borrowed(&stop)),
                    )
                    .unwrap_err();
                assert!(operation.is_failed());
                let before = calls.get();
                assert!(operation.push(&frames[index], Some(&NoPulse)).is_err());
                assert_eq!(calls.get(), before);
                error
            };
            if point == 1 || point == 2 {
                let failure = zencodec::find_cause::<PolicyFailure>(&*error).unwrap();
                assert_eq!(failure.stop_reason(), Some(reason));
                assert_eq!(failure.record().code, DecisionCode::Stopped);
            } else {
                let failure = zencodec::find_cause::<CodecError>(&*error).unwrap();
                assert_eq!(failure.category(), zencodec::ErrorCategory::Stopped(reason));
            }
            assert_eq!(calls.get(), [0, 1, 7, 6][point]);
            assert_eq!(live.load(Ordering::SeqCst), 0);
            assert_eq!(
                (storage.reserved_bytes(), storage.reserved_objects()),
                (0, 0)
            );
        }
    }
}

#[test]
fn epoch_route_partial_header_failure_and_unwind_cannot_resume_or_undo_publication() {
    let frames = native_stream(1);
    for unwind in [false, true] {
        let storage = StorageBudget::new(1 << 20, 64);
        let live = Arc::new(AtomicUsize::new(0));
        let mut operation = Candidate::new(
            PolicySet::new(),
            false,
            false,
            storage.clone(),
            live.clone(),
        );
        operation.push(&frames[0], Some(&NoPulse)).unwrap();
        let mut published = Vec::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            operation.publish_header(
                |config| {
                    let video = config.video.unwrap();
                    let bytes =
                        ivf_header(video.coded_width, video.coded_height, config.time_base, 6);
                    published.extend_from_slice(&bytes[..13]);
                    if unwind {
                        panic!("controlled partial header unwind")
                    }
                    Err(
                        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "partial header")
                            .into(),
                    )
                },
                Some(&NoPulse),
            )
        }));
        if unwind {
            assert!(result.is_err());
        } else {
            let error = result.unwrap().unwrap_err();
            assert_eq!(
                zencodec::find_cause::<std::io::Error>(&*error)
                    .unwrap()
                    .kind(),
                std::io::ErrorKind::BrokenPipe
            );
        }
        assert_eq!(published.len(), 13);
        assert_eq!(&published[..4], b"DKIF");
        assert!(operation.is_failed());
        assert!(operation.push(&frames[1], Some(&NoPulse)).is_err());
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert_eq!(
            (storage.reserved_bytes(), storage.reserved_objects()),
            (0, 0)
        );
    }
}

#[derive(Debug)]
struct AppFailure;
impl std::fmt::Display for AppFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("application route failure")
    }
}
impl Error for AppFailure {}

#[test]
fn epoch_route_policy_error_decline_stale_choice_unwind_and_storage_failure_are_terminal() {
    let frames = native_stream(1);
    for mode in 0..6 {
        let storage = StorageBudget::new(if mode == 5 { 0 } else { 1 << 20 }, 64);
        let live = Arc::new(AtomicUsize::new(0));
        let mut stale = None;
        let mut operation = Candidate::new(
            |question: Question<'_>| -> Result<Decision, At<AppFailure>> {
                let Question::FrameRoute { facts, choices, .. } = question else {
                    panic!()
                };
                let choice = choices.first().unwrap();
                if stale.is_none() {
                    stale = Some(choice.id());
                }
                if facts.input.geometry.coded_width == 19 {
                    return match mode {
                        0 => Err(At::wrap(AppFailure).at()),
                        1 => Ok(Decision::decline(Decline::ApplicationRule)),
                        2 => Ok(Decision::Choose(stale.unwrap())),
                        3 => panic!("controlled transition policy unwind"),
                        _ => Ok(Decision::UsePreset),
                    };
                }
                Ok(Decision::UsePreset)
            },
            true,
            false,
            storage.clone(),
            live.clone(),
        );
        if mode == 4 {
            operation = operation.with_policy_set(PolicySet::new().with_workspace_limit(0));
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for frame in &frames[..7] {
                operation.push(frame, Some(&NoPulse))?;
            }
            Ok::<(), Cause>(())
        }));
        match mode {
            3 => assert!(result.is_err()),
            0 => assert!(
                zencodec::find_cause::<AppFailure>(&*result.unwrap().unwrap_err()).is_some()
            ),
            5 => assert!(
                zencodec::find_cause::<StorageError>(&*result.unwrap().unwrap_err()).is_some()
            ),
            _ => {
                let error = result.unwrap().unwrap_err();
                let failure = zencodec::find_cause::<PolicyFailure>(&*error).unwrap();
                assert_eq!(
                    failure.record().code,
                    if mode == 2 {
                        DecisionCode::InvalidChoice
                    } else {
                        DecisionCode::Declined
                    }
                );
            }
        }
        assert!(operation.is_failed());
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert_eq!(
            (storage.reserved_bytes(), storage.reserved_objects()),
            (0, 0)
        );
    }
}

#[test]
fn presentation_maps_follow_delayed_native_frame_epochs_not_latest_configuration() {
    use presentation_map_experiment::{Content, Edit, Extent, Interval, Plan, Time};
    use zencodec::time::FrameDuration;
    for threads in [1, 4] {
        let mut delayed = 0;
        let frames = native_stream_observed(threads, |frame, configured_epoch| {
            if frame.interpretation().config_epoch != configured_epoch {
                delayed += 1;
            }
        });
        assert!(
            delayed > 0,
            "the control must actually deliver old-epoch frames after configuration advances"
        );
        // Each immutable map is retained through native draining. Identical
        // codec settings in epochs 10 and 11 still have distinct presentation.
        let mut pool = presentation_map_experiment::retained::PlanPool::new(6);
        let plans: Vec<_> = (0..6)
            .map(|group| {
                let clock = if group == 5 {
                    TimeBase::hz(90_000).unwrap()
                } else {
                    TB
                };
                let ticks_per_frame = if group == 5 { 3600 } else { 40 };
                pool.prepare(|| {
                    Plan::new(
                        Timestamp::new(group * 1000, TB),
                        &[Edit {
                            extent: Extent::Finite(FrameDuration::new(120, 1000).unwrap()),
                            content: Content::Media {
                                start: Timestamp::new(group * 3 * ticks_per_frame, clock),
                                rate_num: 1,
                                rate_den: 1,
                            },
                        }],
                    )
                })
                .unwrap()
            })
            .collect();
        let mut cursors: Vec<_> = plans.iter().map(|plan| plan.cursor(0).unwrap()).collect();
        for (index, frame) in frames.iter().enumerate() {
            let epoch = frame.interpretation().config_epoch;
            let group = usize::try_from(epoch.checked_sub(10).unwrap()).unwrap();
            let timing = frame.timing();
            let start = timing.pts.unwrap();
            let duration = if group == 5 { 3600 } else { 40 };
            let interval = Interval::new(
                Time::timestamp(start),
                Time::timestamp(Timestamp::new(start.ticks() + duration, start.base())),
            )
            .unwrap();
            let output = cursors
                .get_mut(group)
                .expect("unknown epoch must refuse")
                .push(interval)
                .unwrap()
                .unwrap();
            let expected = group as i64 * 1000 + (index % 3) as i64 * 40;
            assert_eq!(output.start().on_clock(TB).unwrap().ticks(), expected);
            assert_eq!(output.end().on_clock(TB).unwrap().ticks(), expected + 40);
            if group != 5 {
                assert_eq!(
                    plans[5].plan().segments()[0].project(interval).unwrap(),
                    None,
                    "using only the latest plan loses an old-epoch presentation"
                );
            }
        }
        let replay_owner = plans[0].clone();
        assert!(replay_owner.same_owner(cursors[0].owner()));
        drop(plans);
        assert_eq!(pool.live(), 6, "native cursors retain their maps");
        assert!(matches!(
            pool.prepare(|| panic!("full pool built a map")),
            Err(presentation_map_experiment::retained::AdmissionError::Limit)
        ));
        for (group, cursor) in cursors.iter_mut().enumerate() {
            let completed = cursor.finish().unwrap();
            assert_eq!(
                completed.start().on_clock(TB).unwrap().ticks(),
                group as i64 * 1000
            );
            assert_eq!(
                completed.end().on_clock(TB).unwrap().ticks(),
                group as i64 * 1000 + 120
            );
        }
        drop(cursors);
        assert_eq!(pool.live(), 1, "draining does not retire replay ownership");
        drop(replay_owner);
        assert_eq!(pool.live(), 0);
    }
}

#[test]
fn presentation_maps_follow_owned_sessions_when_native_provenance_is_identical() {
    use presentation_map_experiment::{
        Content, Edit, Extent, Interval, Plan, Time,
        retained::{PlanPool, RetainedCursor},
    };
    use zencodec::time::FrameDuration;
    struct Pass {
        decoder: Av1VideoDecoder,
        cursor: RetainedCursor,
        output: Vec<Frame>,
    }
    fn project(pass: &mut Pass, frame: Frame) {
        let mut timing = frame.timing();
        assert_eq!(frame.interpretation().config_epoch, 0);
        assert_eq!(timing.packet_ordinal, Some(7));
        assert_eq!(timing.input_offset, Some(1000));
        let start = timing.pts.unwrap();
        let end = Timestamp::new(
            start.ticks() + timing.duration_ticks.unwrap() as i64,
            start.base(),
        );
        let interval = Interval::new(Time::timestamp(start), Time::timestamp(end)).unwrap();
        let mapped = pass.cursor.push(interval).unwrap().unwrap();
        let mapped_start = mapped.start().on_clock(TB).unwrap();
        let mapped_end = mapped.end().on_clock(TB).unwrap();
        timing.pts = Some(mapped_start);
        timing.duration_ticks = Some((mapped_end.ticks() - mapped_start.ticks()) as u64);
        let projected = frame.represent(timing, zencodec_media::frame::Presentation::default());
        assert!(projected.shares_picture(&frame));
        pass.output.push(projected);
    }
    for threads in [1, 4] {
        let settings = settings(8, Subsampling::Yuv444, true);
        let original: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
        let (config, packets) = encode(settings, &original);
        assert_eq!(config.epoch, 0);
        let mut pool = PlanPool::new(2);
        let mut passes: Vec<_> = [0, 1000]
            .into_iter()
            .map(|origin| {
                let owner = pool
                    .prepare(|| {
                        Plan::new(
                            Timestamp::new(origin, TB),
                            &[Edit {
                                extent: Extent::Finite(FrameDuration::new(120, 1000).unwrap()),
                                content: Content::Media {
                                    start: Timestamp::new(0, TB),
                                    rate_num: 1,
                                    rate_den: 1,
                                },
                            }],
                        )
                    })
                    .unwrap();
                let mut settings = rav1d_safe::Settings::default();
                settings.threads = threads;
                let mut decoder = Av1VideoDecoder::with_settings(settings).unwrap();
                decoder.configure(&config, None).unwrap();
                Pass {
                    decoder,
                    cursor: owner.cursor(0).unwrap(),
                    output: Vec::new(),
                }
            })
            .collect();
        assert!(
            !passes[0]
                .cursor
                .owner()
                .same_owner(passes[1].cursor.owner())
        );
        assert_eq!(pool.live(), 2);
        // Each session sees exactly the same source bytes, epoch, ordinal and offset.
        // The owning session, rather than any of those values, selects its map.
        for packet in &packets {
            for pass in &mut passes {
                let mut packet = packet.clone().with_ordinal(7).with_input_offset(1000);
                while pass.decoder.send(&mut packet, None).unwrap() != SendStatus::Accepted {
                    let Receive::Ready(frame) = pass.decoder.receive(None).unwrap() else {
                        panic!("native decoder stalled")
                    };
                    project(pass, frame);
                }
            }
        }
        for pass in &mut passes {
            pass.decoder.end_input().unwrap();
        }
        // Reverse draining order deliberately differs from map creation order.
        for pass in passes.iter_mut().rev() {
            loop {
                match pass.decoder.receive(None).unwrap() {
                    Receive::Ready(frame) => project(pass, frame),
                    Receive::EndOfStream => break,
                    other => panic!("native decoder did not drain: {other:?}"),
                }
            }
            pass.cursor.finish().unwrap();
        }
        for (index, pass) in passes.iter().enumerate() {
            assert_eq!(pass.output.len(), original.len());
            for (i, (actual, expected)) in pass.output.iter().zip(&original).enumerate() {
                assert_eq!(samples(actual), samples(expected));
                assert_eq!(
                    actual.timing().pts.unwrap().ticks(),
                    index as i64 * 1000 + i as i64 * 40
                );
                assert_eq!(actual.timing().duration_ticks, Some(40));
            }
        }
        drop(passes);
        assert_eq!(pool.live(), 0);
    }
}

#[test]
fn presentation_maps_follow_pump_progress_through_clipping_and_terminal_failure() {
    use crate::presentation_driver::{Pass, ProjectionError};
    use presentation_map_experiment::{
        Content, Edit, Error as MapError, Extent, Plan, retained::PlanPool,
    };
    use zencodec::time::FrameDuration;
    use zencodec_media::session::{PumpError, SessionError};
    let settings = settings(8, Subsampling::Yuv444, true);
    let original: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
    let (config, packets) = encode(settings, &original);
    for case in 0..5 {
        let mut pool = PlanPool::new(1);
        let owner = pool
            .prepare(|| {
                Plan::new(
                    Timestamp::new(1000, TB),
                    &[Edit {
                        extent: Extent::Finite(
                            FrameDuration::new(if case == 2 { 120 } else { 80 }, 1000).unwrap(),
                        ),
                        content: Content::Media {
                            start: Timestamp::new(40, TB),
                            rate_num: 1,
                            rate_den: 1,
                        },
                    }],
                )
            })
            .unwrap();
        let mut settings = rav1d_safe::Settings::default();
        settings.threads = 1;
        let mut decoder = Av1VideoDecoder::with_settings(settings).unwrap();
        decoder.configure(&config, None).unwrap();
        let mut pass = Pass::new(decoder, owner.cursor(0).unwrap(), TB);
        drop(owner);
        let stop = StopAt {
            stopped: AtomicBool::new(case == 3),
            reason: StopReason::TimedOut,
        };
        let control = zencodec::how_far::StopOnly::borrowed(&stop);
        let pulse = if case == 0 || case == 2 {
            None
        } else {
            Some(&control as &dyn zencodec::Pulse)
        };
        let mut actual = Vec::new();
        let mut output = |frame| -> Result<(), std::io::Error> {
            if case == 1 {
                stop.stopped.store(true, Ordering::SeqCst);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "output witness",
                ));
            }
            actual.push(frame);
            if case == 4 {
                stop.stopped.store(true, Ordering::SeqCst);
            }
            Ok(())
        };
        let mut result = Ok(());
        for packet in &packets {
            result = pass.send(&mut packet.clone(), pulse, &mut output);
            if result.is_err() {
                break;
            }
        }
        let result = result.and_then(|()| pass.finish(pulse, &mut output).map(|_| ()));
        match case {
            0 => {
                result.unwrap();
                assert_eq!(actual.len(), 2);
                for (i, frame) in actual.iter().enumerate() {
                    assert_eq!(samples(frame), samples(&original[i + 1]));
                    assert_eq!(frame.timing().pts.unwrap().ticks(), 1000 + i as i64 * 40);
                    assert_eq!(frame.timing().duration_ticks, Some(40));
                }
            }
            1 => {
                let error = result.unwrap_err();
                assert_eq!(
                    zencodec::find_cause::<std::io::Error>(&error)
                        .unwrap()
                        .kind(),
                    std::io::ErrorKind::BrokenPipe
                );
                assert!(actual.is_empty());
            }
            2 => assert!(matches!(
                result,
                Err(PumpError::Downstream(ProjectionError::Mapping(
                    MapError::MissingCoverage
                )))
            )),
            _ => {
                assert!(matches!(
                    result,
                    Err(PumpError::Upstream(SessionError::Cancelled(
                        StopReason::TimedOut
                    )))
                ));
                assert_eq!(actual.len(), usize::from(case == 4));
            }
        }
        assert_eq!(
            pool.live(),
            0,
            "driver must release its owner on completion or failure"
        );
        assert!(matches!(
            pass.finish(None, |_| Ok::<_, std::io::Error>(())),
            Err(PumpError::Upstream(SessionError::Finished))
        ));
    }
}

#[test]
fn presentation_maps_follow_projected_policy_before_encoder_publication() {
    use crate::presentation_driver::{Pass, Publication, PublicationError};
    use presentation_map_experiment::{Content, Edit, Extent, Plan, retained::PlanPool};
    use zencodec::{
        At,
        operation::{BoundPolicy, MetadataTransfer, PolicySet},
        time::FrameDuration,
    };
    let settings = settings(8, Subsampling::Yuv444, true);
    let original: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
    let (config, packets) = encode(settings, &original);
    for case in 0..4 {
        let decisions = Cell::new(0);
        let requirements = if case == 2 {
            PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive)
        } else {
            PolicySet::new()
        };
        let policy = BoundPolicy::new(
            requirements,
            |question: Question<'_>| -> Result<Decision, At<std::io::Error>> {
                let Question::Frame {
                    timing, choices, ..
                } = question
                else {
                    panic!("unexpected question")
                };
                assert_eq!(
                    timing.pts.unwrap().ticks(),
                    1000,
                    "policy must inspect projected time"
                );
                decisions.set(decisions.get() + 1);
                match case {
                    1 => Ok(Decision::decline(Decline::ApplicationRule)),
                    3 => Err(At::wrap(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "tenant witness",
                    ))
                    .at()),
                    _ => Ok(Decision::choose(choices.first().unwrap())),
                }
            },
        );
        let mut publication = Publication::new(policy).unwrap();
        let mut pool = PlanPool::new(1);
        let map = pool
            .prepare(|| {
                Plan::new(
                    Timestamp::new(1000, TB),
                    &[Edit {
                        extent: Extent::Finite(FrameDuration::new(40, 1000).unwrap()),
                        content: Content::Media {
                            start: Timestamp::new(40, TB),
                            rate_num: 1,
                            rate_den: 1,
                        },
                    }],
                )
            })
            .unwrap();
        let mut settings = rav1d_safe::Settings::default();
        settings.threads = 1;
        let mut decoder = Av1VideoDecoder::with_settings(settings).unwrap();
        decoder.configure(&config, None).unwrap();
        let mut pass = Pass::new(decoder, map.cursor(0).unwrap(), TB);
        drop(map);
        let mut output = Vec::new();
        let mut emit = |frame| {
            publication.publish(frame, None, |frame| {
                output.push(frame);
                Ok::<_, Infallible>(())
            })
        };
        let mut result = Ok(());
        for packet in &packets {
            result = pass.send(&mut packet.clone(), None, &mut emit);
            if result.is_err() {
                break;
            }
        }
        let result = result.and_then(|()| pass.finish(None, &mut emit).map(|_| ()));
        if case == 0 {
            result.unwrap();
            assert_eq!(output.len(), 1);
            assert_eq!(samples(&output[0]), samples(&original[1]));
        } else {
            let error = result.unwrap_err();
            assert!(output.is_empty());
            if case == 3 {
                assert_eq!(
                    zencodec::find_cause::<std::io::Error>(&error)
                        .unwrap()
                        .kind(),
                    std::io::ErrorKind::PermissionDenied
                );
            }
            assert!(matches!(
                publication.publish(original[0].clone(), None, |_| Ok::<_, Infallible>(())),
                Err(PublicationError::Finished)
            ));
        }
        assert_eq!(decisions.get(), usize::from(case != 2));
        assert_eq!(pool.live(), 0);
        let _report = publication.report();
    }
    for cap in [0, u64::MAX] {
        assert!(Publication::new(PolicySet::new().with_workspace_limit(cap)).is_err());
    }
}
