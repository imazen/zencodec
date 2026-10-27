//! Decide on an actual prepared frame-encoding route before accepting its input.
use super::{
    DecisionContext, DecisionRecord, FrameFacts, PolicyFailure, PolicyResolver, PolicySet,
    Question, Selection, policy,
};
use crate::{
    At,
    encode::Fidelity,
    frame::{FrameInterpretation, FrameTiming},
};

/// Output continuity supplied by a prepared adapter route. This does not grant
/// permission to change an already published representation or container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameRoute {
    /// Keep the existing output sequence and its interpretation unchanged.
    Continue,
    /// Start an independently configured segment. The request and destination
    /// must explicitly permit this; it is not in-band container reconfiguration.
    BeginSegment,
}

/// Bounded facts at a frame encoding decision. Source epochs and presentation
/// clocks remain input provenance, independent of output segment numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameRouteFacts {
    /// Facts of the exact held incoming frame.
    pub input: FrameFacts,
    /// Output operation state before accepting the incoming frame.
    pub output: FrameRouteState,
}

/// Output progress supplied by the adapter. Initial state has no accepted
/// frame, completed segment or externally committed representation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameRouteState {
    /// Last frame accepted by this output operation, if any.
    pub previous: Option<FrameFacts>,
    /// Segments already completed by the adapter, whether staged or published.
    pub completed_segments: u64,
    /// Whether any output representation/header/body has escaped private
    /// staging. Zero body bytes alone must not establish that this is false.
    pub output_committed: bool,
}

/// A prepared route for the destination encoder registration fixed by the
/// request. The adapter retains its encoder/configuration and held input while
/// policy runs. This question selects continuity, not an arbitrary output codec.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct FrameRouteOffer {
    /// Qualified output continuity. Unsupported destination transitions must
    /// not appear in the choices, even when policy would select one.
    pub route: FrameRoute,
    /// Fidelity established by the prepared encoder configuration.
    pub fidelity: Option<Fidelity>,
    /// Currently admitted encoded staging held by the operation. This excludes
    /// native input/reference/work buffers and is not a whole-operation bound.
    pub admitted_encoded_bytes: usize,
    /// Qualified additional codec/conversion workspace bound; unknown is not zero.
    pub workspace_upper_bound: Option<u64>,
}
impl FrameRouteOffer {
    /// A prepared continuity mechanism with unknown fidelity/workspace evidence.
    pub const fn new(route: FrameRoute) -> Self {
        Self {
            route,
            fidelity: None,
            admitted_encoded_bytes: 0,
            workspace_upper_bound: None,
        }
    }
}

/// Resolve before passing this exact frame to a prepared encoder or committing
/// an output transition. Every offer must satisfy the request and destination;
/// a callback cannot invent a fallback or authorize an unsupported transition.
///
/// The adapter must retain prepared objects, execute only the selected route,
/// preserve prior-frame ordering while draining, and keep old frames immutable.
/// A successful decision does not certify privacy or make native allocation
/// infallible. An admission/runtime failure must not retry a different route.
pub fn resolve_frame_route<R: PolicyResolver + ?Sized>(
    interpretation: &FrameInterpretation,
    timing: FrameTiming,
    output: FrameRouteState,
    offers: &[FrameRouteOffer],
    resolver: &mut R,
    preset: PolicySet,
    stop: &dyn enough::Stop,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    let facts = FrameRouteFacts {
        input: FrameFacts::new(interpretation, timing),
        output,
    };
    policy::choose(
        offers,
        DecisionContext::FrameRoute(facts),
        |choices| Question::FrameRoute {
            interpretation,
            facts: &facts,
            choices,
        },
        Selection::FrameRoute,
        resolver,
        preset,
        stop,
    )
}
