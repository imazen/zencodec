//! The issue 173 expectations from `policy_proposed.rs`, replayed against
//! ideal-api head e8f16683. Default test for the current Pulse core.
//!
//! Below `mod common;` this file must equal `policy_proposed.rs` except for the
//! `STOP` line: head takes a `Pulse`, which `enough::Unstoppable` is not.
//! `head-replay.sh` checks that before running anything.
mod common;
use common::*;
use zencodec::{
    decode::InterpretationState,
    operation::{DecisionCode, PolicySet, Question, resolve_transcode},
};

static STOP: &zencodec::how_far::Inert = &zencodec::NoPulse;

/// Mandatory requirements survive a direct Choose: the callback never sees,
/// and cannot select, a route that violates them.
#[test]
fn mandatory_requirements_survive_direct_choose() {
    for offers in [vec![conflicting_metadata()], vec![unknown_evidence()]] {
        let result = resolve_transcode(
            boundary(InterpretationState::Final),
            &offers,
            &mut choose_transcode(0),
            strict(),
            STOP,
        );
        let refused = result.expect_err("no admitted route");
        assert_ne!(refused.error().record().code, DecisionCode::Selected);
    }
}

/// Masking keeps original indices: the only admitted route is index 1, and it
/// is the first (and only) choice presented.
#[test]
fn admission_mask_preserves_original_index() {
    let mut seen = Vec::new();
    let mut observe = |question: Question<'_>| match question {
        Question::Transcode { choices, .. } => {
            seen.push(choices.len());
            Ok(choices
                .first()
                .map(zencodec::operation::Decision::choose)
                .unwrap_or(zencodec::operation::Decision::UsePreset))
        }
        _ => Ok::<_, zencodec::At<ProbeError>>(zencodec::operation::Decision::UsePreset),
    };
    let result = resolve_transcode(
        boundary(InterpretationState::Final),
        &[conflicting_metadata(), compliant()],
        &mut observe,
        strict(),
        STOP,
    );
    assert_eq!(selected(&result), Some(1));
    assert_eq!(seen, [1], "only admitted choices are presented");
}

/// Preferences and situational permissions stay overridable: with no mandatory
/// requirement, a callback may select any offered route.
#[test]
fn preferences_do_not_become_requirements() {
    let result = resolve_transcode(
        boundary(InterpretationState::Final),
        &[conflicting_metadata(), compliant()],
        &mut choose_transcode(0),
        PolicySet::new().prefer_lossless(true),
        STOP,
    );
    assert_eq!(selected(&result), Some(0));
}

/// A refusal names the unmet requirement in its display text.
#[test]
fn refusal_names_the_requirement() {
    let refused = resolve_transcode(
        boundary(InterpretationState::Final),
        &[unknown_evidence()],
        &mut delegate,
        strict(),
        STOP,
    )
    .unwrap_err();
    let text = refused.error().to_string().to_lowercase();
    assert!(
        text.contains("workspace") || text.contains("opaque"),
        "refusal should name an unmet requirement: {text}"
    );
}
