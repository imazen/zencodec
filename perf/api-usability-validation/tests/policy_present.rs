//! Observed behaviour at base 6b0caf34 (2026-10-27), recorded as regressions.
//!
//! These document the present delegation semantics: PolicySet requirements
//! apply only when the callback returns UsePreset. When core issue 173 lands,
//! the direct-Choose assertions here are expected to change; update them in the
//! same commit as the semantic change and keep the UsePreset rows unchanged.
//! Proposed behaviour lives separately in `policy_proposed.rs`.
mod common;
use common::*;
use zencodec::{
    ImageFormat,
    decode::{BoundaryAction, InterpretationState},
    encode::OutputTiming,
    operation::{
        DecisionCode, EncodeRequest, MetadataTransfer, OutputSchedule, OutputScheduleFacts,
        OutputScheduleOffer, OutputSource, PolicySet, Question, resolve_boundary,
        resolve_output_schedule, resolve_transcode,
    },
};
use zenpixels::{PixelDescriptor, PixelSlice};

const STOP: &enough::Unstoppable = &enough::Unstoppable;

fn transcode(offers: &[zencodec::operation::TranscodeOffer], direct: bool) -> Resolved {
    let at = boundary(InterpretationState::Final);
    if direct {
        resolve_transcode(at, offers, &mut choose_transcode(0), strict(), STOP)
    } else {
        resolve_transcode(at, offers, &mut delegate, strict(), STOP)
    }
}

#[test]
fn use_preset_enforces_mandatory_requirements() {
    assert_eq!(selected(&transcode(&[compliant()], false)), Some(0));
    for offer in [conflicting_metadata(), unknown_evidence()] {
        let refused = transcode(&[offer], false).unwrap_err();
        assert_eq!(refused.error().record().code, DecisionCode::Declined);
        assert!(refused.error().record().used_preset);
    }
    assert_eq!(
        selected(&transcode(&[conflicting_metadata(), compliant()], false)),
        Some(1)
    );
}

#[test]
fn direct_choose_bypasses_mandatory_requirements_today() {
    for offers in [
        vec![conflicting_metadata()],
        vec![unknown_evidence()],
        vec![conflicting_metadata(), compliant()],
    ] {
        let result = transcode(&offers, true);
        assert_eq!(selected(&result), Some(0), "present delegation semantics");
        assert!(!result.unwrap().1.used_preset);
    }
}

#[test]
fn encode_without_evidence_succeeds_when_callback_chooses() {
    let data = vec![0u8; 16 * 16 * 3];
    let pixels = PixelSlice::new(&data, 16, 16, 48, PixelDescriptor::RGB8_SRGB).unwrap();
    let request = EncodeRequest::new(ImageFormat::Pnm).with_policy_set(strict());
    assert!(request.encode(&codecs(), pixels.clone()).is_err());
    let mut choose = |question: Question<'_>| match question {
        Question::Encode { choices, .. } => Ok(choices
            .first()
            .map(zencodec::operation::Decision::choose)
            .unwrap_or(zencodec::operation::Decision::UsePreset)),
        _ => Ok::<_, zencodec::At<ProbeError>>(zencodec::operation::Decision::UsePreset),
    };
    assert!(request.encode_with(&codecs(), pixels, &mut choose).is_ok());
}

#[test]
fn deferred_discovery_continues_under_requirements() {
    for state in [InterpretationState::Pending, InterpretationState::Provisional] {
        let (action, _) = resolve_boundary(boundary(state), &mut delegate, strict(), STOP)
            .expect("discovery continues");
        assert_eq!(action, BoundaryAction::ContinueReading);
    }
}

#[test]
fn unrelated_domain_refuses_today_and_wait_limit_needs_proof() {
    let facts = OutputScheduleFacts::new(OutputTiming::AT_FINISH, OutputSource::PullRows);
    let mut proven = OutputScheduleOffer::new(OutputSchedule::FinishFirst);
    proven.wait_retained_upper_bound = Some(1 << 20);
    let unknown = OutputScheduleOffer::new(OutputSchedule::FinishFirst);
    let metadata = PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive);
    let wait = PolicySet::new().with_output_wait_limit(10 << 30);
    assert!(resolve_output_schedule(facts, &[proven], &mut delegate, metadata, STOP).is_err());
    assert!(resolve_output_schedule(facts, &[proven], &mut delegate, wait, STOP).is_ok());
    assert!(resolve_output_schedule(facts, &[unknown], &mut delegate, wait, STOP).is_err());
}

#[test]
fn refusal_display_names_no_requirement_today() {
    let refused = transcode(&[unknown_evidence()], false).unwrap_err();
    let text = refused.error().to_string();
    assert!(text.starts_with("policy Declined at question"), "{text}");
}
