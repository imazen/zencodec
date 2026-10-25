//! Typed offers for adapters that prepare conversion and encode together.
use super::{DecisionRecord, EncodeOffer, PolicyFailure, PolicyResolver, PolicySet};
use crate::{
    At,
    decode::{BoundaryAction, DecodeBoundary},
    operation::policy,
};
use zenpixels::PixelDescriptor;

/// Treatment of source descriptive metadata in a pixel transcode.
/// Color interpretation is a separate obligation; this is not a certificate
/// about sensitive content in pixels, opaque profiles, or nested images.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetadataTransfer {
    /// Retain the metadata fields reported by the source decoder. This does
    /// not claim preservation of unreported or unknown container carriers.
    PreserveReported,
    /// Do not copy source EXIF, XMP or descriptive packets to the destination.
    /// Required orientation and output color must still be handled explicitly.
    DiscardDescriptive,
}

/// One coherent, prepared pixel-transcode route. The adapter owns its checked
/// encoder job, conversion, metadata obligations and admitted storage throughout
/// the callback. These are facts for choosing that route, not instructions to
/// reconstruct it after selection. Execution may still fail; it never retries
/// another route after selection.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct TranscodeOffer {
    /// Confirmed configuration of the retained encoder job.
    pub encoder: EncodeOffer,
    /// Samples and interpretation delivered to that encoder after conversion.
    pub output: PixelDescriptor,
    /// Descriptive source metadata treatment.
    pub metadata: MetadataTransfer,
    /// Whether the output retains an opaque source color profile. Preserving
    /// such a profile does not certify its descriptive contents as PII-free.
    pub retains_opaque_profile: bool,
    /// Managed row/conversion storage retained by this route after selection.
    /// Alternatives may share these same allocations; do not sum their offers.
    /// Excludes common input storage and unqualified native workspace.
    pub admitted_bytes: usize,
    /// Qualified upper bound for additional codec/conversion workspace outside
    /// managed storage, if known. `None` means unknown, never zero. Estimates
    /// are not upper bounds. Encoded staging and caller-owned source storage
    /// must also be included before claiming a whole-operation bound.
    pub workspace_upper_bound: Option<u64>,
}
impl TranscodeOffer {
    /// Describe a route whose resource scope and opaque-profile retention must
    /// be filled in by its executing adapter. Unknown storage stays unknown.
    pub const fn new(
        encoder: EncodeOffer,
        output: PixelDescriptor,
        metadata: MetadataTransfer,
    ) -> Self {
        Self {
            encoder,
            output,
            metadata,
            retains_opaque_profile: false,
            admitted_bytes: 0,
            workspace_upper_bound: None,
        }
    }
}

/// Resolve a complete route at a final interpretation suspension.
///
/// Adapter API: keep every offer paired with its prepared objects, execute only
/// the returned index, and adopt the same held boundary before decoding pixels.
/// No alternative is offered for provisional/unavailable interpretation. This
/// function neither resumes a decoder nor validates an adapter's factual claims.
/// It applies the shared callback, cancellation, token and bounded-report rules.
pub fn resolve_transcode<R: PolicyResolver + ?Sized>(
    boundary: DecodeBoundary,
    offers: &[TranscodeOffer],
    resolver: &mut R,
    preset: PolicySet,
    stop: &dyn enough::Stop,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    let facts = boundary.facts();
    let offers = if BoundaryAction::AdoptInterpretation.permits(facts) {
        offers
    } else {
        &[]
    };
    policy::choose(
        offers,
        super::DecisionContext::Transcode(facts),
        |choices| super::Question::Transcode { facts, choices },
        super::Selection::Transcode,
        resolver,
        preset,
        stop,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{Decision, DecisionCode, Question};
    use crate::{
        ImageFormat,
        decode::{InterpretationFacts, InterpretationState},
        encode::Fidelity,
    };
    use core::convert::Infallible;
    fn boundary(state: InterpretationState) -> DecodeBoundary {
        DecodeBoundary::new(InterpretationFacts::new(
            97,
            65,
            Some(PixelDescriptor::RGB8_SRGB),
            state,
        ))
        .unwrap()
    }
    fn offer() -> TranscodeOffer {
        TranscodeOffer::new(
            super::super::EncodeOffer {
                format: ImageFormat::Png,
                fidelity: Some(Fidelity::Lossless),
                preference_index: 0,
            },
            PixelDescriptor::RGB8_SRGB,
            MetadataTransfer::DiscardDescriptive,
        )
    }
    #[test]
    fn constraints_filter_complete_routes_and_never_treat_unknown_workspace_as_zero() {
        let mut preserved = offer();
        preserved.metadata = MetadataTransfer::PreserveReported;
        preserved.workspace_upper_bound = Some(12);
        let mut opaque = offer();
        opaque.retains_opaque_profile = true;
        opaque.workspace_upper_bound = Some(12);
        let unknown = offer();
        let mut over = offer();
        over.workspace_upper_bound = Some(13);
        let mut accepted = offer();
        accepted.workspace_upper_bound = Some(12);
        accepted.encoder.preference_index = 4;
        let offers = [preserved, opaque, unknown, over, accepted];
        let preset = PolicySet::new()
            .require_metadata(MetadataTransfer::DiscardDescriptive)
            .allow_opaque_profiles(false)
            .with_workspace_limit(12)
            .prefer_lossless(true);
        let mut delegate =
            |_: Question<'_>| -> Result<Decision, At<Infallible>> { Ok(Decision::UsePreset) };
        let (index, record) = resolve_transcode(
            boundary(InterpretationState::Final),
            &offers,
            &mut delegate,
            preset,
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(index, 4);
        assert!(record.used_preset);
        assert_eq!(record.code, DecisionCode::Selected);
        let error = resolve_transcode(
            boundary(InterpretationState::Final),
            &offers[..4],
            &mut delegate,
            preset,
            &enough::Unstoppable,
        )
        .unwrap_err();
        assert_eq!(error.error().record().code, DecisionCode::Declined);
        assert_eq!(error.error().record().offered, 4);
    }
    #[test]
    fn provisional_interpretation_and_stale_route_tokens_never_select_work() {
        let offers = [offer()];
        let mut token = None;
        resolve_transcode(
            boundary(InterpretationState::Final),
            &offers,
            &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                let Question::Transcode { choices, .. } = q else {
                    panic!("wrong domain")
                };
                token = Some(choices.first().unwrap().id());
                Ok(Decision::UsePreset)
            },
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap();
        for state in [InterpretationState::Final, InterpretationState::Provisional] {
            let error = resolve_transcode(
                boundary(state),
                &offers,
                &mut |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                    let Question::Transcode { choices, .. } = q else {
                        panic!("wrong domain")
                    };
                    assert_eq!(choices.is_empty(), state != InterpretationState::Final);
                    Ok(Decision::Choose(token.unwrap()))
                },
                PolicySet::new(),
                &enough::Unstoppable,
            )
            .unwrap_err();
            assert_eq!(error.error().record().code, DecisionCode::InvalidChoice);
        }
        let mut called = false;
        let error = resolve_transcode(
            boundary(InterpretationState::Final),
            &[offer(); 65],
            &mut |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                called = true;
                Ok(Decision::UsePreset)
            },
            PolicySet::new(),
            &enough::Unstoppable,
        )
        .unwrap_err();
        assert!(!called);
        assert_eq!(error.error().record().code, DecisionCode::Exhausted);
    }
}
