//! Ownership and publication at the real frame-policy adapter boundary.
use enough::{Stop, StopReason};
use std::{
    collections::VecDeque,
    convert::Infallible,
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use zencodec::how_far::NoPulse;
use zencodec::{
    At,
    frame::{
        ChromaSiting, FrameGeometry, FrameInterpretation, FrameTiming, PlaneRole, Subsampling,
    },
    operation::{
        Decision, DecisionCode, DecisionContext, Decline, MetadataTransfer, PolicyFailure,
        PolicySet, Question,
    },
    storage::StorageBudget,
    time::{TimeBase, Timestamp},
};
use zencodec_media::{
    frame::{Frame, OwnedPlane, OwnedPlanes, PlaneData, PlaneSamples, Presentation},
    packet::Packet,
    policy::{PolicyDecoder, Stage},
    session::{CodecConfig, Receive, SendStatus, SessionError, VideoDecoder, VideoStreamInfo},
    track::CodecId,
};
use zenpixels::sample::SampleEncoding;

fn frame(budget: &StorageBudget, i: u16) -> Frame {
    let mut words = budget.words(8).unwrap();
    words
        .as_mut_slice()
        .copy_from_slice(&[i, 512, 1023, 60000, i + 1, 511, 1022, 60001]);
    let encoding = SampleEncoding::new(zenpixels::ChannelType::U16, 10, 0).unwrap();
    let picture = OwnedPlanes::new(
        vec![OwnedPlane {
            data: PlaneData::AccountedU16(words.freeze()),
            width: 3,
            height: 2,
            stride_bytes: 8,
        }],
        encoding,
    )
    .unwrap();
    let interpretation = FrameInterpretation::new(
        FrameGeometry::full(3, 2),
        Subsampling::Mono,
        ChromaSiting::Unknown,
        encoding,
        vec![PlaneRole::Luma],
    )
    .with_config_epoch(u32::from(i % 2));
    Frame::new(
        Arc::new(picture),
        Arc::new(interpretation),
        FrameTiming::at(Timestamp::new(i.into(), TimeBase::new(1, 1000).unwrap()))
            .with_packet(Some(7), Some(u64::from(i))),
        Presentation::default(),
    )
}

#[test]
fn frame_route_preset_does_not_trade_required_workspace_for_preferred_fidelity() {
    use zencodec::{
        encode::Fidelity,
        operation::{FrameRoute, FrameRouteOffer, FrameRouteState, resolve_frame_route},
    };
    let budget = StorageBudget::new(16, 1);
    let held = frame(&budget, 0);
    let mut unknown = FrameRouteOffer::new(FrameRoute::Continue);
    unknown.fidelity = Some(Fidelity::Lossless);
    let mut bounded = FrameRouteOffer::new(FrameRoute::Continue);
    bounded.workspace_upper_bound = Some(64);
    let mut larger = unknown;
    larger.workspace_upper_bound = Some(128);
    let offers = [unknown, bounded, larger];
    for (limit, expected) in [(64, 1), (128, 2)] {
        let (selected, record) = resolve_frame_route(
            held.interpretation(),
            held.timing(),
            FrameRouteState::default(),
            &offers,
            &mut PolicySet::new()
                .prefer_lossless(true)
                .with_workspace_limit(limit),
            PolicySet::new(),
            Some(&NoPulse),
        )
        .unwrap();
        assert_eq!(selected, expected);
        assert_eq!(record.offered, 3);
    }
    for mut preset in [
        PolicySet::new().with_workspace_limit(63),
        PolicySet::new().allow_opaque_profiles(false),
        PolicySet::new().require_metadata(MetadataTransfer::PreserveReported),
    ] {
        let error = resolve_frame_route(
            held.interpretation(),
            held.timing(),
            FrameRouteState::default(),
            &offers,
            &mut preset,
            PolicySet::new(),
            Some(&NoPulse),
        )
        .unwrap_err();
        assert_eq!(error.error().record().code, DecisionCode::Declined);
    }
}

#[derive(Debug)]
struct Native(u8);
impl std::fmt::Display for Native {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native {}", self.0)
    }
}
impl Error for Native {}
struct Queued {
    frames: VecDeque<Frame>,
    calls: Arc<AtomicUsize>,
    fail_at: Option<u8>,
    expected_none: Option<bool>,
}
impl Queued {
    fn new(frames: impl IntoIterator<Item = Frame>) -> Self {
        Self {
            frames: frames.into_iter().collect(),
            calls: Arc::new(AtomicUsize::new(0)),
            fail_at: None,
            expected_none: None,
        }
    }
    fn check_pulse(&self, pulse: Option<&dyn zencodec::Pulse>) {
        if let Some(expected) = self.expected_none {
            assert_eq!(
                pulse.is_none(),
                expected,
                "policy changed per-call control presence"
            );
        }
    }
    fn call(&self, at: u8) -> Result<(), SessionError<Native>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_at == Some(at) {
            Err(SessionError::Codec(Native(at)))
        } else {
            Ok(())
        }
    }
}
impl VideoDecoder for Queued {
    type Error = Native;
    fn configure(
        &mut self,
        _: &CodecConfig,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<Native>> {
        self.check_pulse(pulse);
        self.call(0)
    }
    fn send(
        &mut self,
        _: &mut Packet,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<SendStatus, SessionError<Native>> {
        self.check_pulse(pulse);
        self.call(1)?;
        Ok(SendStatus::Accepted)
    }
    fn receive(
        &mut self,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Receive<Frame>, SessionError<Native>> {
        self.check_pulse(pulse);
        self.call(2)?;
        Ok(self
            .frames
            .pop_front()
            .map_or(Receive::EndOfStream, Receive::Ready))
    }
    fn end_input(&mut self) -> Result<(), SessionError<Native>> {
        self.call(3)
    }
    fn reset(&mut self) -> Result<(), SessionError<Native>> {
        self.call(4)?;
        self.frames.clear();
        Ok(())
    }
    fn stream_info(&self) -> Option<&VideoStreamInfo> {
        None
    }
}
fn config() -> CodecConfig {
    CodecConfig::new(CodecId::Av1, TimeBase::new(1, 1000).unwrap())
}

#[test]
fn absent_control_reaches_native_policy_calls_and_preserves_required_metadata_gate() {
    for refuse in [false, true] {
        let budget = StorageBudget::new(16, 1);
        let input = frame(&budget, 0);
        let owner = Arc::downgrade(input.picture());
        let mut native = Queued::new([input]);
        native.expected_none = Some(true);
        let policy = if refuse {
            PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive)
        } else {
            PolicySet::new()
        };
        let mut decoder = PolicyDecoder::new(native, policy);
        decoder.configure(&config(), None).unwrap();
        decoder
            .send(&mut Packet::from_vec(vec![1]).unwrap(), None)
            .unwrap();
        if refuse {
            assert!(decoder.receive(None).is_err());
            assert!(decoder.is_failed());
            assert_eq!(budget.reserved_bytes(), 0);
            assert!(owner.upgrade().is_none());
            assert_eq!(
                decoder.report().decisions().next().unwrap().code,
                DecisionCode::Declined
            );
            assert!(matches!(decoder.receive(None), Err(SessionError::Finished)));
        } else {
            let Receive::Ready(output) = decoder.receive(None).unwrap() else {
                panic!("missing output")
            };
            assert!(Arc::ptr_eq(output.picture(), &owner.upgrade().unwrap()));
            assert_eq!(decoder.report().decisions().count(), 1);
            assert_eq!(output.interpretation().encoding.code_bits(), 10);
        }
    }
}

#[test]
fn every_native_presentation_keeps_samples_and_facts_and_the_report_is_bounded() {
    let budget = StorageBudget::new(24 * 16, 24);
    let frames: Vec<_> = (0..24).map(|i| frame(&budget, i)).collect();
    // This annotation is also an old-path/new-path type compatibility check.
    let _: &zencodec_media::frame::FrameInterpretation = frames[0].interpretation();
    let source_owners: Vec<_> = frames.iter().map(|f| Arc::downgrade(f.picture())).collect();
    let mut seen = 0;
    let mut decoder = PolicyDecoder::new(
        Queued::new(frames),
        |question: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Frame {
                interpretation,
                timing,
                choices,
            } = question
            else {
                panic!("wrong domain")
            };
            assert_eq!(interpretation.encoding.code_bits(), 10);
            assert_eq!(interpretation.roles, [PlaneRole::Luma]);
            assert_eq!(interpretation.config_epoch, seen % 2);
            assert_eq!(timing.pts.unwrap().ticks(), i64::from(seen));
            assert_eq!(timing.packet_ordinal, Some(7));
            seen += 1;
            Ok(Decision::choose(choices.first().unwrap()))
        },
    );
    for (i, owner) in source_owners.iter().enumerate() {
        let Receive::Ready(output) = decoder.receive(Some(&NoPulse)).unwrap() else {
            panic!("missing native output")
        };
        assert!(Arc::ptr_eq(output.picture(), &owner.upgrade().unwrap()));
        let mapping = output.map().unwrap();
        let plane = mapping.plane(0).unwrap();
        assert!(
            matches!(plane.row(0).unwrap(), PlaneSamples::U16(row) if row == [i as u16, 512, 1023])
        );
        assert!(
            matches!(plane.row(1).unwrap(), PlaneSamples::U16(row) if row == [i as u16 + 1, 511, 1022])
        );
    }
    assert_eq!(decoder.report().decisions().count(), 16);
    assert_eq!(decoder.report().dropped_decisions(), 8);
    let first = decoder.report().decisions().next().unwrap();
    let DecisionContext::Frame(facts) = first.context else {
        panic!("wrong context")
    };
    assert_eq!(facts.timing.pts.unwrap().ticks(), 8);
    assert!(matches!(
        decoder.receive(Some(&NoPulse)).unwrap(),
        Receive::EndOfStream
    ));
    assert_eq!(budget.reserved_bytes(), 0);
    drop(decoder);
    assert_eq!(seen, 24);
}

fn assert_finished<R: zencodec::operation::OperationPolicy>(
    decoder: &mut PolicyDecoder<Queued, R>,
) {
    let mut packet = Packet::from_vec(vec![1]).unwrap();
    assert!(matches!(
        decoder.configure(&config(), Some(&NoPulse)),
        Err(SessionError::Finished)
    ));
    assert!(matches!(
        decoder.send(&mut packet, Some(&NoPulse)),
        Err(SessionError::Finished)
    ));
    assert!(matches!(
        decoder.receive(Some(&NoPulse)),
        Err(SessionError::Finished)
    ));
    assert!(matches!(decoder.end_input(), Err(SessionError::Finished)));
    assert!(matches!(decoder.reset(), Err(SessionError::Finished)));
    assert_eq!(packet.bytes(), &[1]);
}

#[test]
fn stale_policy_choice_publishes_no_second_frame_and_releases_all_native_owners() {
    let budget = StorageBudget::new(48, 3);
    let mut token = None;
    let mut decoder = PolicyDecoder::new(
        Queued::new((0..3).map(|i| frame(&budget, i))),
        |question: Question<'_>| -> Result<Decision, At<Infallible>> {
            let Question::Frame { choices, .. } = question else {
                panic!("wrong domain")
            };
            if let Some(token) = token {
                return Ok(Decision::Choose(token));
            }
            token = Some(choices.first().unwrap().id());
            Ok(Decision::UsePreset)
        },
    );
    let Receive::Ready(old) = decoder.receive(Some(&NoPulse)).unwrap() else {
        panic!("missing first frame")
    };
    let error = decoder.receive(Some(&NoPulse)).unwrap_err();
    let SessionError::Codec(error) = error else {
        panic!("expected policy failure")
    };
    assert_eq!(error.stage(), Stage::Policy);
    assert_eq!(
        error.report().decisions().last().unwrap().code,
        DecisionCode::InvalidChoice
    );
    assert_eq!(
        budget.reserved_bytes(),
        16,
        "only previously published frame survives"
    );
    assert_finished(&mut decoder);
    assert_eq!(old.map().unwrap().plane(0).unwrap().code(2, 0), Some(1023));
    drop(old);
    assert_eq!(budget.reserved_bytes(), 0);
}

#[test]
fn policy_refusal_callback_error_and_unwind_drop_unpublished_frames() {
    for mode in 0..3 {
        let budget = StorageBudget::new(32, 2);
        let mut decoder = PolicyDecoder::new(
            Queued::new((0..2).map(|i| frame(&budget, i))),
            move |_: Question<'_>| -> Result<Decision, At<Native>> {
                match mode {
                    0 => Ok(Decision::decline(Decline::ApplicationRule)),
                    1 => Err(At::wrap(Native(77)).at_named("private-tenant-policy")),
                    _ => panic!("authored policy panic"),
                }
            },
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            decoder.receive(Some(&NoPulse))
        }));
        if mode == 2 {
            assert!(result.is_err());
        } else {
            let error = result.unwrap().unwrap_err();
            assert!(!format!("{error:?}").contains("private-tenant-policy"));
            let SessionError::Codec(error) = error else {
                panic!("missing error")
            };
            assert_eq!(
                error.report().decisions().next().unwrap().code,
                if mode == 0 {
                    DecisionCode::Declined
                } else {
                    DecisionCode::CallbackFailed
                }
            );
            if mode == 1 {
                let mut cause: &(dyn Error + 'static) = &error;
                loop {
                    if let Some(trace) = cause.downcast_ref::<At<Native>>() {
                        assert_eq!(trace.error().0, 77);
                        assert!(trace.frame_count() > 0);
                        break;
                    }
                    cause = cause.source().expect("callback cause was lost");
                }
            }
        }
        assert_eq!(budget.reserved_bytes(), 0);
        assert_finished(&mut decoder);
    }
}

struct Cancel(AtomicBool);
impl Stop for Cancel {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.load(Ordering::SeqCst) {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}
#[test]
fn cancellation_after_callback_prevents_publication_and_keeps_its_decision() {
    let budget = StorageBudget::new(16, 1);
    let stop = Cancel(AtomicBool::new(false));
    let mut decoder = PolicyDecoder::new(
        Queued::new([frame(&budget, 0)]),
        |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            stop.0.store(true, Ordering::SeqCst);
            Ok(Decision::UsePreset)
        },
    );
    let SessionError::Codec(error) = decoder
        .receive(Some(&zencodec::how_far::StopOnly::borrowed(&stop)))
        .unwrap_err()
    else {
        panic!("missing failure")
    };
    let trace = error
        .source()
        .unwrap()
        .downcast_ref::<At<PolicyFailure>>()
        .unwrap();
    assert_eq!(trace.error().stop_reason(), Some(StopReason::Cancelled));
    assert_eq!(
        error.report().decisions().next().unwrap().code,
        DecisionCode::Stopped
    );
    assert_eq!(budget.reserved_bytes(), 0);
    assert_finished(&mut decoder);
}

#[test]
fn native_failure_in_every_work_method_is_terminal_with_original_cause() {
    for method in 0..5 {
        let budget = StorageBudget::new(16, 1);
        let mut native = Queued::new([frame(&budget, 0)]);
        native.fail_at = Some(method);
        let calls = native.calls.clone();
        let mut decoder = PolicyDecoder::new(native, PolicySet::new());
        let mut packet = Packet::from_vec(vec![1]).unwrap();
        let error = match method {
            0 => decoder.configure(&config(), Some(&NoPulse)),
            1 => decoder.send(&mut packet, Some(&NoPulse)).map(|_| ()),
            2 => decoder.receive(Some(&NoPulse)).map(|_| ()),
            3 => decoder.end_input(),
            _ => decoder.reset(),
        }
        .unwrap_err();
        assert_eq!(zencodec::find_cause::<Native>(&error).unwrap().0, method);
        assert_eq!(budget.reserved_bytes(), 0);
        assert_finished(&mut decoder);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn native_publication_cannot_claim_transcode_metadata_or_workspace_contracts() {
    for preset in [
        PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
        PolicySet::new().allow_opaque_profiles(false),
        PolicySet::new().with_workspace_limit(0),
    ] {
        let budget = StorageBudget::new(16, 1);
        let mut decoder = PolicyDecoder::new(Queued::new([frame(&budget, 0)]), preset);
        assert!(decoder.receive(Some(&NoPulse)).is_err());
        assert_eq!(budget.reserved_bytes(), 0);
    }
}

#[test]
fn frame_route_conversion_evidence_controls_range_and_siting_without_confusing_encoder_fidelity() {
    use zencodec::{
        encode::Fidelity,
        operation::{
            FrameConversionOffer, FrameFacts, FrameRoute, FrameRouteOffer, FrameRouteState,
            Selection, resolve_frame_route,
        },
    };
    let budget = StorageBudget::new(16, 1);
    let held = frame(&budget, 0);
    let mut unknown = FrameRouteOffer::new(FrameRoute::Continue);
    unknown.fidelity = Some(Fidelity::Lossless);
    let mut clipping = unknown;
    clipping.conversion = Some(FrameConversionOffer::new(FrameFacts::new(
        held.interpretation(),
        held.timing(),
    )));
    let mut preserving = clipping;
    preserving
        .conversion
        .as_mut()
        .unwrap()
        .preserves_extended_range = true;
    let mut unqualified_encoder = preserving;
    unqualified_encoder.fidelity = None;
    let mut assumed = preserving;
    assumed.conversion.as_mut().unwrap().assumed_chroma_siting = Some(ChromaSiting::Center);
    let mut impossible = preserving;
    impossible
        .conversion
        .as_mut()
        .unwrap()
        .assumed_chroma_siting = Some(ChromaSiting::Unknown);
    for (offers, allow_assumption, expected) in [
        (vec![unknown, clipping, preserving], false, Some(2)),
        (vec![unknown, clipping], true, None),
        (vec![unqualified_encoder], true, None),
        (vec![assumed], false, None),
        (vec![assumed], true, Some(0)),
        (vec![impossible], true, None),
    ] {
        let mut preset = PolicySet::new()
            .require_extended_range(true)
            .allow_assumed_chroma_siting(allow_assumption);
        let result = resolve_frame_route(
            held.interpretation(),
            held.timing(),
            FrameRouteState::default(),
            &offers,
            &mut preset,
            PolicySet::new(),
            Some(&NoPulse),
        );
        match expected {
            Some(expected) => {
                let (selected, record) = result.unwrap();
                assert_eq!(selected, expected);
                let Some(Selection::FrameRoute(selected)) = record.selected else {
                    panic!()
                };
                let conversion = selected.conversion.unwrap();
                assert!(conversion.preserves_extended_range);
                assert!(!conversion.preserves_samples);
                assert_eq!(selected.fidelity, Some(Fidelity::Lossless));
            }
            None => assert_eq!(
                result.unwrap_err().error().record().code,
                DecisionCode::Declined
            ),
        }
    }
}

#[test]
fn frame_route_lossless_preference_first_uses_complete_conversion_and_encoder_evidence() {
    use zencodec::{
        encode::Fidelity,
        operation::{
            FrameConversionOffer, FrameFacts, FrameRoute, FrameRouteOffer, FrameRouteState,
            resolve_frame_route,
        },
    };
    let budget = StorageBudget::new(16, 1);
    let held = frame(&budget, 0);
    let mut encoder_only = FrameRouteOffer::new(FrameRoute::Continue);
    encoder_only.fidelity = Some(Fidelity::Lossless);
    encoder_only.workspace_upper_bound = Some(16);
    let mut changes_samples = encoder_only;
    changes_samples.conversion = Some(FrameConversionOffer::new(FrameFacts::new(
        held.interpretation(),
        held.timing(),
    )));
    let mut preserves_samples = changes_samples;
    preserves_samples
        .conversion
        .as_mut()
        .unwrap()
        .preserves_samples = true;
    preserves_samples.workspace_upper_bound = Some(32);
    let offers = [changes_samples, encoder_only, preserves_samples];
    for (prefer, limit, expected) in [(true, 32, 2), (true, 16, 0), (false, 32, 0)] {
        let (selected, _) = resolve_frame_route(
            held.interpretation(),
            held.timing(),
            FrameRouteState::default(),
            &offers,
            &mut PolicySet::new()
                .prefer_lossless(prefer)
                .with_workspace_limit(limit),
            PolicySet::new(),
            Some(&NoPulse),
        )
        .unwrap();
        assert_eq!(selected, expected);
    }
    // A preserving converter with unknown encoder fidelity is not a complete
    // preserving route. Preserve the existing encoder preference in that case.
    preserves_samples.fidelity = None;
    let (selected, _) = resolve_frame_route(
        held.interpretation(),
        held.timing(),
        FrameRouteState::default(),
        &[preserves_samples, encoder_only],
        &mut PolicySet::new().prefer_lossless(true),
        PolicySet::new(),
        Some(&NoPulse),
    )
    .unwrap();
    assert_eq!(selected, 1);
}

#[test]
#[allow(deprecated)] // Verify legacy requirements remain conjunctive with bound policy.
fn required_unknown_workspace_refuses_every_native_entry_before_effects() {
    use zencodec::operation::{
        BoundPolicy, PolicyRequirement, RequirementEvidence, UnmetRequirements,
    };
    for bound in [false, true] {
        for entry in 0..5 {
            let budget = StorageBudget::new(16, 1);
            let held = frame(&budget, 0);
            let owner = Arc::downgrade(held.picture());
            let native = Queued::new([held]);
            let calls = native.calls.clone();
            let decisions = Arc::new(AtomicUsize::new(0));
            let observed = decisions.clone();
            let required = PolicySet::new().with_workspace_limit(u64::MAX);
            let policy = BoundPolicy::new(
                if bound { required } else { PolicySet::new() },
                move |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                    observed.fetch_add(1, Ordering::SeqCst);
                    Ok(Decision::UsePreset)
                },
            );
            let mut decoder = PolicyDecoder::new(native, policy).with_policy_set(if bound {
                PolicySet::new()
            } else {
                required
            });
            let error = match entry {
                0 => decoder.configure(&config(), None),
                1 => decoder
                    .send(&mut Packet::from_vec(vec![1]).unwrap(), None)
                    .map(|_| ()),
                2 => decoder.receive(None).map(|_| ()),
                3 => decoder.end_input(),
                _ => decoder.reset(),
            }
            .unwrap_err();
            assert_eq!(
                calls.load(Ordering::SeqCst),
                0,
                "entry {entry}, bound {bound}"
            );
            assert_eq!(decisions.load(Ordering::SeqCst), 0);
            let SessionError::Codec(error) = error else {
                panic!("missing policy error")
            };
            assert_eq!(error.stage(), Stage::Policy);
            let unmet = error
                .source()
                .unwrap()
                .downcast_ref::<UnmetRequirements>()
                .unwrap();
            assert_eq!(
                unmet.iter().collect::<Vec<_>>(),
                vec![(PolicyRequirement::Workspace, RequirementEvidence::Unknown)]
            );
            assert!(decoder.is_failed());
            assert!(owner.upgrade().is_none());
            assert_eq!(budget.reserved_bytes(), 0);
            assert!(matches!(decoder.receive(None), Err(SessionError::Finished)));
        }
    }
}

#[test]
fn factory_admission_precedes_native_creation_and_preserves_causes() {
    for required in [false, true] {
        let calls = AtomicUsize::new(0);
        let result = PolicyDecoder::try_new(
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Native>(Queued::new([]))
            },
            if required {
                PolicySet::new().with_workspace_limit(u64::MAX)
            } else {
                PolicySet::new()
            },
            None,
        );
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(!required));
        assert_eq!(result.is_err(), required);
        if let Err(error) = result {
            assert_eq!(error.stage(), Stage::Policy);
            assert!(
                error
                    .source()
                    .unwrap()
                    .is::<zencodec::operation::UnmetRequirements>()
            );
        }
    }
    let result = PolicyDecoder::<Queued, _>::try_new(|| Err(Native(99)), PolicySet::new(), None);
    let Err(error) = result else {
        panic!("factory error lost")
    };
    assert_eq!(error.stage(), Stage::Decode);
    assert_eq!(
        error.source().unwrap().downcast_ref::<Native>().unwrap().0,
        99
    );
    struct Cancelled;
    impl Stop for Cancelled {
        fn check(&self) -> Result<(), StopReason> {
            Err(StopReason::TimedOut)
        }
    }
    let pulse = zencodec::how_far::StopOnly::new(Cancelled);
    let result = PolicyDecoder::<Queued, _>::try_new(
        || -> Result<Queued, Native> { panic!("cancelled factory ran") },
        PolicySet::new(),
        Some(&pulse),
    );
    let Err(error) = result else {
        panic!("cancelled factory admitted")
    };
    assert!(matches!(
        error
            .source()
            .unwrap()
            .downcast_ref::<SessionError<Infallible>>(),
        Some(SessionError::Cancelled(StopReason::TimedOut))
    ));
}

#[test]
fn cancellation_during_factory_drops_the_unpublished_native_owner() {
    struct During<'a>(&'a AtomicBool);
    impl Stop for During<'_> {
        fn check(&self) -> Result<(), StopReason> {
            if self.0.load(Ordering::SeqCst) {
                Err(StopReason::TimedOut)
            } else {
                Ok(())
            }
        }
    }
    let stopped = AtomicBool::new(false);
    let during = During(&stopped);
    let pulse = zencodec::how_far::StopOnly::borrowed(&during);
    let budget = StorageBudget::new(16, 1);
    let result = PolicyDecoder::try_new(
        || {
            let decoder = Queued::new([frame(&budget, 0)]);
            stopped.store(true, Ordering::SeqCst);
            Ok::<_, Native>(decoder)
        },
        PolicySet::new(),
        Some(&pulse),
    );
    let Err(error) = result else {
        panic!("published a decoder after construction cancelled")
    };
    assert_eq!(budget.reserved_bytes(), 0);
    assert!(matches!(
        error
            .source()
            .unwrap()
            .downcast_ref::<SessionError<Infallible>>(),
        Some(SessionError::Cancelled(StopReason::TimedOut))
    ));
}
