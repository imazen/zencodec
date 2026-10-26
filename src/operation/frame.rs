//! Policy before publishing an owned native video frame.
use super::{
    DecisionContext, DecisionRecord, PolicyFailure, PolicyResolver, PolicySet, Question, Selection,
    policy,
};
use crate::{
    At,
    frame::{
        ChromaSiting, ColorClaim, FrameGeometry, FrameInterpretation, FrameTiming, SampleEncoding,
        Subsampling,
    },
};

/// Bounded diagnostic facts about one native presentation. Full component roles
/// remain borrowed in the question; this record contains no sample payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameFacts {
    /// Coded, visible and render geometry, without an implicit crop or resize.
    pub geometry: FrameGeometry,
    /// Native chroma layout.
    pub subsampling: Subsampling,
    /// Declared chroma siting; unknown remains unknown.
    pub siting: ChromaSiting,
    /// Storage word and significant sample-code placement.
    pub encoding: SampleEncoding,
    /// Number of native component roles.
    pub planes: usize,
    /// Bitstream, container and selected color declarations.
    pub color: ColorClaim,
    /// Configuration captured when the native input was accepted.
    pub config_epoch: u32,
    /// Presentation timing and packet provenance captured by the native adapter.
    pub timing: FrameTiming,
}
impl FrameFacts {
    /// Snapshot diagnostic fields from the actual retained frame.
    pub fn new(interpretation: &FrameInterpretation, timing: FrameTiming) -> Self {
        Self {
            geometry: interpretation.geometry,
            subsampling: interpretation.subsampling,
            siting: interpretation.siting,
            encoding: interpretation.encoding,
            planes: interpretation.roles.len(),
            color: interpretation.color,
            config_epoch: interpretation.config_epoch,
            timing,
        }
    }
}

/// Publish the same held output without further conversion, relabeling, trimming
/// or new encoding. Unknown declarations remain inspectable. This establishes
/// no metadata privacy, display rendering or native workspace bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PublicationOffer {
    /// Return these held samples, interpretation and presentation unchanged.
    /// A prior explicit conversion may already have produced this output.
    ReturnHeld,
}

/// Resolve publication of an actual retained native frame through the same
/// application resolver used by still-image operations.
///
/// Adapter API: retain the frame throughout the callback and return that exact
/// frame only on success. This does not decode, convert or admit native storage.
/// Invoke for every presentation; an epoch alone cannot justify reusing a prior
/// decision, and application policy may depend on the presentation timestamp.
pub fn resolve_frame<R: PolicyResolver + ?Sized>(
    interpretation: &FrameInterpretation,
    timing: FrameTiming,
    resolver: &mut R,
    preset: PolicySet,
    stop: &dyn enough::Stop,
) -> Result<DecisionRecord, At<PolicyFailure>> {
    let facts = FrameFacts::new(interpretation, timing);
    let (_, record) = policy::choose(
        &[PublicationOffer::ReturnHeld],
        DecisionContext::Frame(facts),
        |choices| Question::Frame {
            interpretation,
            timing,
            choices,
        },
        Selection::Frame,
        resolver,
        preset,
        stop,
    )?;
    Ok(record)
}
