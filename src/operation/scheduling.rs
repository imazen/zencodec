//! Resolve an actual source/encoder/output route before publishing bytes.
use super::{
    DecisionContext, DecisionRecord, PolicyFailure, PolicyResolver, PolicySet, Question, Selection,
    policy,
};
use crate::{At, encode::OutputTiming};

/// When the route permits final-destination I/O to wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputSchedule {
    /// Complete and release the encoder before final-destination I/O.
    /// Source ownership and compressed staging remain separate obligations.
    FinishFirst,
    /// Permit final-destination I/O while the encoder remains alive.
    /// Byte availability alone supplies no retention bound.
    Live,
}
/// Source ownership retained by the actual route, not the format's capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputSource {
    /// The caller retains a borrowed complete raster throughout the operation.
    BorrowedRaster,
    /// The route requests rows on demand and stops requests during output waits.
    /// The source itself may still own a full raster or other unbounded state.
    PullRows,
    /// Source ownership has not been qualified.
    Unknown,
}
/// Facts of the retained route before its first final-destination effect.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct OutputScheduleFacts {
    /// Actual configured encoder's byte availability after begin.
    pub timing: OutputTiming,
    /// Actual source acquisition/ownership mechanism.
    pub source: OutputSource,
}
impl OutputScheduleFacts {
    /// Bind observed byte availability and source mechanism.
    pub const fn new(timing: OutputTiming, source: OutputSource) -> Self {
        Self { timing, source }
    }
}
/// A prepared executable schedule, held unchanged across the callback.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct OutputScheduleOffer {
    /// Mechanism executed if this route is selected.
    pub schedule: OutputSchedule,
    /// Actual compressed-output store; None means no staging evidence.
    pub staging: Option<super::StagingFacts>,
    /// Qualified upper bound on all route-retained memory during destination
    /// waits: source, codec, conversion, metadata, scratch and compressed data.
    /// Include caller input retained by the route's borrows. Excludes unrelated
    /// caller state and the destination's own allocations. None is unknown.
    /// This is not peak-operation memory, disk use, or a measured estimate.
    pub wait_retained_upper_bound: Option<u64>,
}
impl OutputScheduleOffer {
    /// Unknown retention by default, including for finish-first routes.
    pub const fn new(schedule: OutputSchedule) -> Self {
        Self {
            schedule,
            staging: None,
            wait_retained_upper_bound: None,
        }
    }
}
/// Adapter boundary before final output effects. Retain the source, configured
/// encoder, storage and destination; execute only the selected route. Never
/// reconstruct or fall back after selection. Return the record on later errors.
///
/// Live output is not offered when begin reported no streaming. This resolver
/// validates selection tokens, not the adapter's retention claims.
pub fn resolve_output_schedule<R: PolicyResolver + ?Sized>(
    facts: OutputScheduleFacts,
    offers: &[OutputScheduleOffer],
    resolver: &mut R,
    preset: PolicySet,
    stop: &dyn enough::Stop,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    // A contradictory offer set is refused as a whole; index identity is kept.
    let offers = if offers
        .iter()
        .any(|o| o.schedule == OutputSchedule::Live && !facts.timing.streams())
    {
        &[]
    } else {
        offers
    };
    policy::choose(
        offers,
        DecisionContext::OutputSchedule(facts),
        |choices| Question::OutputSchedule { facts, choices },
        Selection::OutputSchedule,
        resolver,
        preset,
        stop,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{Decision, DecisionCode};
    use core::convert::Infallible;

    fn facts() -> OutputScheduleFacts {
        OutputScheduleFacts::new(OutputTiming::streaming(0), OutputSource::PullRows)
    }
    #[test]
    fn unknown_retention_is_not_zero_even_after_codec_release_or_with_pull_source() {
        for schedule in [OutputSchedule::FinishFirst, OutputSchedule::Live] {
            let mut offer = OutputScheduleOffer::new(schedule);
            let required = PolicySet::new().with_output_wait_limit(0);
            let error = resolve_output_schedule(
                facts(),
                &[offer],
                &mut { required },
                required,
                &enough::Unstoppable,
            )
            .unwrap_err();
            assert_eq!(error.error().record().code, DecisionCode::Declined);
            offer.wait_retained_upper_bound = Some(0);
            let (index, _) = resolve_output_schedule(
                facts(),
                &[offer],
                &mut { required },
                required,
                &enough::Unstoppable,
            )
            .unwrap();
            assert_eq!(index, 0);
            offer.wait_retained_upper_bound = Some(1);
            assert!(
                resolve_output_schedule(
                    facts(),
                    &[offer],
                    &mut { required },
                    required,
                    &enough::Unstoppable
                )
                .is_err()
            );
        }
    }
    #[test]
    fn schedule_requirement_never_falls_back_and_retention_limit_is_independent() {
        let offers = [
            OutputScheduleOffer::new(OutputSchedule::FinishFirst),
            OutputScheduleOffer::new(OutputSchedule::Live),
        ];
        let required = PolicySet::new().require_output_schedule(OutputSchedule::Live);
        let (index, record) = resolve_output_schedule(
            facts(),
            &offers,
            &mut { required },
            required,
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(index, 1);
        assert!(
            matches!(record.selected, Some(Selection::OutputSchedule(o)) if o.schedule == OutputSchedule::Live)
        );
        assert!(
            resolve_output_schedule(
                facts(),
                &offers[..1],
                &mut { required },
                required,
                &enough::Unstoppable
            )
            .is_err()
        );
    }
    #[test]
    fn nonstreaming_live_offer_is_hidden_even_from_custom_selector() {
        let mut called = false;
        let error = resolve_output_schedule(
            OutputScheduleFacts::new(OutputTiming::AT_FINISH, OutputSource::Unknown),
            &[OutputScheduleOffer::new(OutputSchedule::Live)],
            &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                called = true;
                let Question::OutputSchedule { choices, .. } = q else {
                    panic!()
                };
                assert!(choices.is_empty());
                Ok(Decision::UsePreset)
            },
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap_err();
        assert!(called);
        assert_eq!(error.error().record().offered, 0);
    }
    #[test]
    fn replayed_selection_cannot_authorize_another_retained_route() {
        let offer = [OutputScheduleOffer::new(OutputSchedule::FinishFirst)];
        let mut token = None;
        resolve_output_schedule(
            facts(),
            &offer,
            &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::OutputSchedule { choices, .. } = q else {
                    panic!()
                };
                token = Some(choices.first().unwrap().id());
                Ok(Decision::UsePreset)
            },
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap();
        let error = resolve_output_schedule(
            facts(),
            &offer,
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                Ok(Decision::Choose(token.unwrap()))
            },
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap_err();
        assert_eq!(error.error().record().code, DecisionCode::InvalidChoice);
    }
}
