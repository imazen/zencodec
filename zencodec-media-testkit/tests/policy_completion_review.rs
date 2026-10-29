//! Admission must remain valid through staged completion.
//! Run with `cargo test -p zencodec-media-testkit --features zencodec-media/std --test policy_completion_review`.
use core::convert::Infallible;
use std::cell::Cell;
use zencodec::{
    At,
    how_far::NoPulse,
    operation::{
        BoundPolicy, Decision, Decline, MetadataTransfer, OutputSchedule, PolicySet, Question,
    },
    storage::StorageBudget,
    time::TimeBase,
};
use zencodec_media::{
    container::{MuxFinish, MuxHint, MuxTiming, Muxer},
    output::Forward,
    packet::Packet,
    session::{SessionError, UnsupportedMedia},
    staged::StagedEncode,
    track::TrackInfo,
};
use zencodec_media_testkit::toy::ToyEncoder;

// A header-only container. Refusing all packets makes exact timing vacuous;
// unlike delegating to a legacy muxer, this claims no unimplemented timing path.
struct HeaderOnlyMux(Vec<u8>, Option<bool>);
impl Muxer for HeaderOnlyMux {
    type Error = Infallible;
    fn begin(
        &mut self,
        _: &[TrackInfo],
        _: MuxHint,
    ) -> Result<MuxTiming, SessionError<Infallible>> {
        self.0.extend_from_slice(b"header");
        Ok(MuxTiming::streaming())
    }
    fn begin_exact_timing(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<Infallible>> {
        self.begin(tracks, hint)
    }
    fn write_packet(
        &mut self,
        _: Packet,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), SessionError<Infallible>> {
        if let Some(expected) = self.1 {
            assert_eq!(pulse.is_none(), expected);
        }
        Err(SessionError::Unsupported(UnsupportedMedia::ExactTiming))
    }
    fn pending_output(&self) -> &[u8] {
        &self.0
    }
    fn consume_output(&mut self, n: usize) {
        self.0.drain(..n.min(self.0.len()));
    }
    fn take_output(&mut self, into: &mut Vec<u8>) {
        into.append(&mut self.0);
    }
    fn finish(
        self,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<MuxFinish, SessionError<Infallible>> {
        if let Some(expected) = self.1 {
            assert_eq!(pulse.is_none(), expected);
        }
        Ok(MuxFinish::new(self.0, vec![]))
    }
}

fn encoder() -> ToyEncoder {
    ToyEncoder::new(1, 1, TimeBase::new(1, 1000).unwrap(), 0, 1)
}

#[test]
fn absent_control_survives_output_preparation_publication_and_finish() {
    use zencodec_media::output::{Output, PreparedOutput};
    for prepare_first in [false, true] {
        let mut bytes = Vec::new();
        let mux = HeaderOnlyMux(vec![], Some(true));
        let output = if prepare_first {
            PreparedOutput::new(mux, Forward(&mut bytes), &[], None)
                .unwrap()
                .publish(None)
                .unwrap()
        } else {
            Output::new(mux, Forward(&mut bytes), &[], None).unwrap()
        };
        let stats = output.finish(None).unwrap();
        assert_eq!(bytes, b"header");
        assert_eq!(stats.emitted, bytes.len() as u64);
    }
}

struct TimedOut;
impl enough::Stop for TimedOut {
    fn check(&self) -> Result<(), enough::StopReason> {
        Err(enough::StopReason::TimedOut)
    }
}

#[test]
fn stopped_output_publication_and_staged_input_keep_typed_causes_and_terminal_state() {
    use zencodec_media::output::PreparedOutput;
    let pulse = zencodec::how_far::StopOnly::new(TimedOut);
    let mut bytes = Vec::new();
    let prepared =
        PreparedOutput::new(HeaderOnlyMux(vec![], None), Forward(&mut bytes), &[], None).unwrap();
    let error = prepared.publish(Some(&pulse)).err().unwrap();
    assert_eq!(error.emitted, 0);
    assert!(matches!(
        zencodec::find_cause::<SessionError<zencodec::decode::ErasedError>>(&error),
        Some(SessionError::Cancelled(enough::StopReason::TimedOut))
    ));
    assert!(bytes.is_empty());
    let mut operation = StagedEncode::new(
        encoder(),
        HeaderOnlyMux(vec![], None),
        Forward(&mut bytes),
        PolicySet::new(),
        StorageBudget::new(1024, 16),
        None,
    )
    .unwrap();
    let error = operation.push(frame(), Some(&pulse)).unwrap_err();
    assert_eq!(error.error().emitted, 0);
    assert_eq!(
        zencodec::find_cause::<zencodec::CodecError>(&error)
            .unwrap()
            .category(),
        zencodec::ErrorCategory::Stopped(enough::StopReason::TimedOut)
    );
    assert!(operation.is_failed());
    assert_eq!(
        zencodec::find_cause::<zencodec_media::staged::Refusal>(
            &operation.push(frame(), None).unwrap_err()
        ),
        Some(&zencodec_media::staged::Refusal::Terminal)
    );
    assert!(operation.finish(None).is_err());
    assert!(bytes.is_empty());
}

#[test]
fn empty_staged_finish_cannot_publish_without_application_admission() {
    let calls = Cell::new(0);
    let mut bytes = Vec::new();
    let operation = StagedEncode::new(
        encoder(),
        HeaderOnlyMux(vec![], None),
        Forward(&mut bytes),
        |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            calls.set(calls.get() + 1);
            Ok(Decision::decline(Decline::ApplicationRule))
        },
        StorageBudget::new(1024, 16),
        Some(&NoPulse),
    )
    .unwrap();
    let result = operation.finish(Some(&NoPulse));
    assert!(
        bytes.is_empty(),
        "finish published without a policy decision: {bytes:?}; callbacks={}; result={result:?}",
        calls.get()
    );
    assert!(
        result.is_err(),
        "an always-decline resolver must not produce successful output"
    );
}

#[test]
fn empty_staged_finish_cannot_discharge_unproven_bound_requirements() {
    for required in [
        PolicySet::new().require_output_schedule(OutputSchedule::Live),
        PolicySet::new().with_output_wait_limit(0),
        PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
        PolicySet::new().allow_opaque_profiles(false),
        PolicySet::new().with_workspace_limit(0),
        PolicySet::new().require_extended_range(true),
    ] {
        let mut bytes = Vec::new();
        let policy = BoundPolicy::new(
            required,
            |_: Question<'_>| -> Result<Decision, At<Infallible>> { Ok(Decision::UsePreset) },
        );
        let operation = StagedEncode::new(
            encoder(),
            HeaderOnlyMux(vec![], None),
            Forward(&mut bytes),
            policy,
            StorageBudget::new(1024, 16),
            Some(&NoPulse),
        )
        .unwrap();
        let result = operation.finish(Some(&NoPulse));
        assert!(
            bytes.is_empty(),
            "unadmitted header published under {required:?}: {bytes:?}; result={result:?}"
        );
        assert!(
            result.is_err(),
            "unknown or contradictory evidence must refuse"
        );
    }
}

fn frame() -> zencodec::frame::Frame {
    use std::sync::Arc;
    use zencodec::frame::{
        ChromaSiting, Frame, FrameGeometry, FrameInterpretation, FrameTiming, PlaneRole,
        Presentation, SampleEncoding, Subsampling,
    };
    use zencodec_media::frame::{OwnedPlane, OwnedPlanes, PlaneData};
    let encoding = SampleEncoding::new(zenpixels::ChannelType::U8, 8, 0).unwrap();
    let pixels = OwnedPlanes::new(
        vec![OwnedPlane {
            data: PlaneData::U8(vec![42]),
            width: 1,
            height: 1,
            stride_bytes: 1,
        }],
        encoding,
    )
    .unwrap();
    Frame::new(
        Arc::new(pixels),
        Arc::new(FrameInterpretation::new(
            FrameGeometry::full(1, 1),
            Subsampling::Mono,
            ChromaSiting::Unknown,
            encoding,
            vec![PlaneRole::Luma],
        )),
        FrameTiming::at(zencodec::time::Timestamp::new(
            0,
            TimeBase::new(1, 1000).unwrap(),
        ))
        .with_duration_ticks(1),
        Presentation::default(),
    )
}

#[test]
fn changing_the_legacy_policy_after_input_cannot_publish_or_accept_more_input() {
    for (push_again, preset) in [false, true].into_iter().flat_map(|push_again| {
        [
            PolicySet::new(),
            PolicySet::new().require_output_schedule(OutputSchedule::Live),
        ]
        .map(|preset| (push_again, preset))
    }) {
        let mut bytes = Vec::new();
        let encoder = ToyEncoder::new(1, 1, TimeBase::new(1, 1000).unwrap(), 1, 1);
        let mut operation = StagedEncode::new(
            encoder,
            HeaderOnlyMux(vec![], None),
            Forward(&mut bytes),
            PolicySet::new(),
            StorageBudget::new(1024, 16),
            Some(&NoPulse),
        )
        .unwrap();
        operation.push(frame(), Some(&NoPulse)).unwrap();
        #[allow(
            deprecated,
            reason = "Legacy setter must not bypass an admitted operation"
        )]
        let mut operation = operation.with_policy_set(preset);
        if push_again {
            let error = operation.push(frame(), Some(&NoPulse)).unwrap_err();
            assert_eq!(error.error().stage, zencodec_media::staged::Stage::Policy);
            assert_eq!(error.error().emitted, 0);
            assert_eq!(
                zencodec::find_cause::<zencodec_media::staged::Refusal>(&error),
                Some(&zencodec_media::staged::Refusal::PolicyChangedAfterInput)
            );
            assert!(operation.is_failed());
        }
        let error = operation.finish(Some(&NoPulse)).unwrap_err();
        assert_eq!(error.error().emitted, 0);
        assert_eq!(
            zencodec::find_cause::<zencodec_media::staged::Refusal>(&error),
            Some(&if push_again {
                zencodec_media::staged::Refusal::Terminal
            } else {
                zencodec_media::staged::Refusal::PolicyChangedAfterInput
            })
        );
        assert!(bytes.is_empty());
    }
}
