//! Replay of the policy-composition probes against ideal-api head e8f16683
//! (2026-10-27), which enforces mandatory requirements before any callback.
//!
//! Default tests for the current Pulse core (`cargo test`); `head-replay.sh`
//! replays them against an exported revision. `policy_present.rs` stays the
//! record of base 6b0caf34 (feature `baseline-6b0caf34`).
//!
//! Group 1 repeats baseline assertions that still apply, unchanged. Group 2
//! states head semantics for the baseline rows that issue 173 superseded; each
//! is at least as strict as the row it replaces and names that row.
mod common;
use common::*;
use zencodec::{
    ImageFormat,
    decode::{BoundaryAction, InterpretationState},
    encode::OutputTiming,
    operation::{
        Decision, DecisionCode, DecisionRecord, EncodeRequest, MetadataTransfer, OperationError,
        OutputSchedule, OutputScheduleFacts, OutputScheduleOffer, OutputSource, PolicyRequirement,
        PolicySet, Question, RequirementEvidence, resolve_boundary, resolve_output_schedule,
        resolve_transcode,
    },
};
use zenpixels::{PixelDescriptor, PixelSlice};

static STOP: Option<&dyn zencodec::Pulse> = Some(&zencodec::NoPulse);

fn transcode(offers: &[zencodec::operation::TranscodeOffer], direct: bool) -> Resolved {
    let at = boundary(InterpretationState::Final);
    if direct {
        resolve_transcode(at, offers, &mut choose_transcode(0), strict(), STOP)
    } else {
        resolve_transcode(at, offers, &mut delegate, strict(), STOP)
    }
}

fn unmet(record: &DecisionRecord) -> Vec<(PolicyRequirement, RequirementEvidence)> {
    record.unmet_requirements.iter().collect()
}

fn never_called(_: Question<'_>) -> Result<Decision, zencodec::At<ProbeError>> {
    panic!("mandatory admission must refuse before the callback")
}

// Group 1: baseline assertions that still apply at head, unchanged.

/// Baseline `use_preset_enforces_mandatory_requirements`, selection rows.
#[test]
fn use_preset_selects_compliant_routes() {
    assert_eq!(selected(&transcode(&[compliant()], false)), Some(0));
    assert_eq!(
        selected(&transcode(&[conflicting_metadata(), compliant()], false)),
        Some(1)
    );
}

/// Baseline `use_preset_enforces_mandatory_requirements`, refusal code rows.
#[test]
fn use_preset_refusals_stay_declined() {
    for offer in [conflicting_metadata(), unknown_evidence()] {
        let refused = transcode(&[offer], false).unwrap_err();
        assert_eq!(refused.error().record().code, DecisionCode::Declined);
    }
}

#[test]
fn deferred_discovery_continues_under_requirements() {
    for state in [
        InterpretationState::Pending,
        InterpretationState::Provisional,
    ] {
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

// Group 2: head semantics replacing superseded baseline rows.

/// Replaces baseline `used_preset == true` on UsePreset refusals and the
/// single-offer rows of `direct_choose_bypasses_mandatory_requirements_today`:
/// excluded routes refuse before any callback, delegating or choosing.
#[test]
fn excluded_routes_refuse_before_any_callback() {
    let cases = [
        (
            conflicting_metadata(),
            vec![(
                PolicyRequirement::Metadata,
                RequirementEvidence::Contradicted,
            )],
        ),
        (
            unknown_evidence(),
            vec![
                (
                    PolicyRequirement::OpaqueProfiles,
                    RequirementEvidence::Unknown,
                ),
                (PolicyRequirement::Workspace, RequirementEvidence::Unknown),
            ],
        ),
    ];
    for (offer, expected) in cases {
        let at = boundary(InterpretationState::Final);
        let refused =
            resolve_transcode(at, &[offer], &mut never_called, strict(), STOP).unwrap_err();
        let record = refused.error().record();
        assert_eq!(record.code, DecisionCode::Declined);
        assert!(!record.used_preset);
        assert!(record.selected.is_none());
        assert_eq!((record.offered, record.admitted), (1, 0));
        assert_eq!(unmet(&record), expected);
    }
}

/// Replaces the `[conflicting, compliant]` row of
/// `direct_choose_bypasses_mandatory_requirements_today`: the first presented
/// choice is the original index 1, and the record names the withheld route.
#[test]
fn direct_choose_sees_only_admitted_routes() {
    let (index, record) = transcode(&[conflicting_metadata(), compliant()], true).unwrap();
    assert_eq!(index, 1);
    assert!(!record.used_preset);
    assert_eq!(record.code, DecisionCode::Selected);
    assert_eq!((record.offered, record.admitted), (2, 1));
    assert_eq!(
        unmet(&record),
        [(
            PolicyRequirement::Metadata,
            RequirementEvidence::Contradicted
        )]
    );
}

/// Replaces `encode_without_evidence_succeeds_when_callback_chooses`. A plain
/// encode carries no metadata/profile/workspace evidence, so the transcode-shaped
/// tenant preset refuses it with or without a callback (review A2: scoped evidence).
#[test]
#[allow(deprecated)] // Regression control for legacy request-bound policy.
fn strict_encode_refuses_with_and_without_callback() {
    let data = vec![0u8; 16 * 16 * 3];
    let pixels = PixelSlice::new(&data, 16, 16, 48, PixelDescriptor::RGB8_SRGB).unwrap();
    let request = EncodeRequest::new(ImageFormat::Pnm).with_policy_set(strict());
    let expected = [
        (PolicyRequirement::Metadata, RequirementEvidence::Unknown),
        (
            PolicyRequirement::OpaqueProfiles,
            RequirementEvidence::Unknown,
        ),
        (PolicyRequirement::Workspace, RequirementEvidence::Unknown),
    ];
    let errors = [
        request.encode(&codecs(), pixels.clone()).unwrap_err(),
        request
            .encode_with(&codecs(), pixels, &mut never_called)
            .unwrap_err(),
    ];
    for error in errors {
        let record = match error.error() {
            OperationError::Policy(failure) => failure.record(),
            other => panic!("expected a policy refusal, got {other:?}"),
        };
        assert_eq!(record.code, DecisionCode::Declined);
        assert!(!record.used_preset);
        assert_eq!(record.admitted, 0);
        assert_eq!(unmet(&record), expected);
    }
}

/// Replaces `refusal_display_names_no_requirement_today`.
#[test]
fn refusal_display_names_unmet_requirements() {
    let refused = transcode(&[unknown_evidence()], false).unwrap_err();
    let text = refused.error().to_string();
    assert!(text.starts_with("policy Declined at question"), "{text}");
    assert!(text.contains("OpaqueProfiles(Unknown)"), "{text}");
    assert!(text.contains("Workspace(Unknown)"), "{text}");
}

/// Adds evidence to the unchanged unrelated-domain row: the refusal names the
/// metadata requirement that a schedule question cannot prove.
#[test]
fn unrelated_domain_refusal_names_metadata_unknown() {
    let facts = OutputScheduleFacts::new(OutputTiming::AT_FINISH, OutputSource::PullRows);
    let mut proven = OutputScheduleOffer::new(OutputSchedule::FinishFirst);
    proven.wait_retained_upper_bound = Some(1 << 20);
    let metadata = PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive);
    let refused =
        resolve_output_schedule(facts, &[proven], &mut never_called, metadata, STOP).unwrap_err();
    let record = refused.error().record();
    assert_eq!((record.offered, record.admitted), (1, 0));
    assert_eq!(
        unmet(&record),
        [(PolicyRequirement::Metadata, RequirementEvidence::Unknown)]
    );
}
