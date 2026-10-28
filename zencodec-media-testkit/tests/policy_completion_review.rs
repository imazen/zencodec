//! Review witnesses, prepared but not compiled or run.
//! Run with `cargo test -p zencodec-media-testkit --features zencodec-media/std --test policy_completion_review`.
use core::convert::Infallible;
use std::cell::Cell;
use zencodec::{
    At,
    how_far::NoPulse,
    operation::{BoundPolicy, Decision, Decline, MetadataTransfer, OutputSchedule, PolicySet, Question},
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
struct HeaderOnlyMux(Vec<u8>);
impl Muxer for HeaderOnlyMux {
    type Error = Infallible;
    fn begin(&mut self, _: &[TrackInfo], _: MuxHint) -> Result<MuxTiming, SessionError<Infallible>> {
        self.0.extend_from_slice(b"header");
        Ok(MuxTiming::streaming())
    }
    fn begin_exact_timing(&mut self, tracks: &[TrackInfo], hint: MuxHint) -> Result<MuxTiming, SessionError<Infallible>> {
        self.begin(tracks, hint)
    }
    fn write_packet(&mut self, _: Packet, _: &dyn zencodec::Pulse) -> Result<(), SessionError<Infallible>> {
        Err(SessionError::Unsupported(UnsupportedMedia::ExactTiming))
    }
    fn pending_output(&self) -> &[u8] { &self.0 }
    fn consume_output(&mut self, n: usize) { self.0.drain(..n.min(self.0.len())); }
    fn take_output(&mut self, into: &mut Vec<u8>) { into.append(&mut self.0); }
    fn finish(self, _: &dyn zencodec::Pulse) -> Result<MuxFinish, SessionError<Infallible>> {
        Ok(MuxFinish::new(self.0, vec![]))
    }
}

fn encoder() -> ToyEncoder {
    ToyEncoder::new(1, 1, TimeBase::new(1, 1000).unwrap(), 0, 1)
}

#[test]
fn empty_staged_finish_cannot_publish_without_application_admission() {
    let calls = Cell::new(0);
    let mut bytes = Vec::new();
    let operation = StagedEncode::new(
        encoder(), HeaderOnlyMux(vec![]), Forward(&mut bytes),
        |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            calls.set(calls.get() + 1);
            Ok(Decision::decline(Decline::ApplicationRule))
        },
        StorageBudget::new(1024, 16), &NoPulse,
    ).unwrap();
    let result = operation.finish(&NoPulse);
    assert!(bytes.is_empty(), "finish published without a policy decision: {bytes:?}; callbacks={}; result={result:?}", calls.get());
    assert!(result.is_err(), "an always-decline resolver must not produce successful output");
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
        let policy = BoundPolicy::new(required,
            |_: Question<'_>| -> Result<Decision, At<Infallible>> { Ok(Decision::UsePreset) });
        let operation = StagedEncode::new(
            encoder(), HeaderOnlyMux(vec![]), Forward(&mut bytes), policy,
            StorageBudget::new(1024, 16), &NoPulse,
        ).unwrap();
        let result = operation.finish(&NoPulse);
        assert!(bytes.is_empty(), "unadmitted header published under {required:?}: {bytes:?}; result={result:?}");
        assert!(result.is_err(), "unknown or contradictory evidence must refuse");
    }
}
