//! Shared offers and resolvers for the policy-composition tests.
#![allow(dead_code)]

use std::fmt;
use zencodec::{
    At, CodecSet, ImageFormat,
    decode::{DecodeBoundary, InterpretationFacts, InterpretationState},
    operation::{
        Decision, DecisionRecord, EncodeRequest, MetadataTransfer, PolicyFailure, PolicySet,
        Question, TranscodeOffer,
    },
};
use zencodec_testkit::{ReferenceDecoderConfig, ReferenceEncoderConfig};
use zenpixels::PixelDescriptor;

#[derive(Debug)]
pub struct ProbeError;
impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("probe callback error")
    }
}
impl std::error::Error for ProbeError {}

pub type Resolved = Result<(usize, DecisionRecord), At<PolicyFailure>>;

pub fn codecs() -> CodecSet {
    CodecSet::new()
        .with_decoder(ReferenceDecoderConfig)
        .with_encoder(ReferenceEncoderConfig::new())
}

/// The strict tenant preset: three mandatory requirements.
pub fn strict() -> PolicySet {
    PolicySet::new()
        .require_metadata(MetadataTransfer::DiscardDescriptive)
        .allow_opaque_profiles(false)
        .with_workspace_limit(1 << 30)
}

pub fn route(
    metadata: MetadataTransfer,
    opaque: Option<bool>,
    workspace: Option<u64>,
) -> TranscodeOffer {
    let encoder = EncodeRequest::new(ImageFormat::Pnm)
        .prepare(&codecs())
        .expect("prepared reference encoder")
        .remove(0)
        .offer();
    let mut offer = TranscodeOffer::new(encoder, PixelDescriptor::RGB8_SRGB, metadata);
    offer.retains_opaque_profile = opaque;
    offer.workspace_upper_bound = workspace;
    offer
}

pub fn compliant() -> TranscodeOffer {
    route(
        MetadataTransfer::DiscardDescriptive,
        Some(false),
        Some(1 << 20),
    )
}
pub fn conflicting_metadata() -> TranscodeOffer {
    route(
        MetadataTransfer::PreserveReported,
        Some(false),
        Some(1 << 20),
    )
}
pub fn unknown_evidence() -> TranscodeOffer {
    route(MetadataTransfer::DiscardDescriptive, None, None)
}

pub fn boundary(state: InterpretationState) -> DecodeBoundary {
    DecodeBoundary::new(InterpretationFacts::new(
        16,
        16,
        Some(PixelDescriptor::RGB8_SRGB),
        state,
    ))
    .expect("valid boundary")
}

pub fn delegate(_: Question<'_>) -> Result<Decision, At<ProbeError>> {
    Ok(Decision::UsePreset)
}

/// Selects the k-th presented transcode choice directly.
pub fn choose_transcode(k: usize) -> impl FnMut(Question<'_>) -> Result<Decision, At<ProbeError>> {
    move |question: Question<'_>| match question {
        Question::Transcode { choices, .. } => Ok(choices
            .iter()
            .nth(k)
            .map(Decision::choose)
            .unwrap_or(Decision::UsePreset)),
        _ => Ok(Decision::UsePreset),
    }
}

pub fn selected(result: &Resolved) -> Option<usize> {
    result.as_ref().ok().map(|(index, _)| *index)
}
