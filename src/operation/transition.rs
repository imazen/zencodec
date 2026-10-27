//! Decide on an actual prepared frame-encoding route before accepting its input.
use super::{
    DecisionContext, DecisionRecord, FrameFacts, PolicyFailure, PolicyResolver, PolicySet,
    Question, Selection, policy,
};
use crate::{
    At,
    encode::Fidelity,
    frame::{ChromaSiting, FrameInterpretation, FrameTiming},
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

/// Facts derived from the actual retained container and writer preparation.
/// This describes byte delivery, not encoder raw-buffer retention or a bound on
/// memory. In particular, streaming bytes do not authorize upstream backpressure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DestinationFacts {
    /// The retained writer supports overwriting earlier output positions.
    pub seek_back: bool,
    /// The prepared container makes bytes available before finish.
    pub streams: bool,
    /// Finishing this layout requires overwriting previously emitted bytes.
    pub patches_at_finish: bool,
    /// The retained muxer guarantees exact supplied packet timing or fails.
    /// False means no such guarantee, not permission to discard timing.
    pub exact_timing: bool,
}
impl DestinationFacts {
    /// Describe a successfully prepared container/writer pair. Adapters must
    /// derive these facts from retained objects, never from requested settings.
    pub const fn new(
        seek_back: bool,
        streams: bool,
        patches_at_finish: bool,
        exact_timing: bool,
    ) -> Self {
        Self {
            seek_back,
            streams,
            patches_at_finish,
            exact_timing,
        }
    }
}

/// Where compressed payloads are retained before final destination I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StagingKind {
    /// Payloads remain in admitted RAM until muxing releases them.
    Memory,
    /// Spool I/O may block while the encoder owns raw/work state. This is not
    /// a live sequential/backpressure qualification.
    Disk,
}
/// Facts of the retained compressed store, separate from encoder workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StagingFacts {
    /// Actual selected storage mechanism.
    pub kind: StagingKind,
    /// Packet and addition bytes admitted so far; excludes index metadata.
    pub payload_bytes: u64,
    /// Store byte allowance, not a guarantee that future admission succeeds.
    pub payload_limit: u64,
}
impl StagingFacts {
    /// Describe the actual retained store without certifying native memory.
    pub const fn new(kind: StagingKind, payload_bytes: u64, payload_limit: u64) -> Self {
        Self {
            kind,
            payload_bytes,
            payload_limit,
        }
    }
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
    /// Effects and target of the retained conversion before this encoder.
    /// None means unqualified, not identity or sample preservation. Encoder
    /// fidelity alone never establishes end-to-end transcode fidelity.
    pub conversion: Option<FrameConversionOffer>,
    /// Actual retained destination evidence. None means unqualified; it does
    /// not imply forward-only, seekable, staged or safe encoder backpressure.
    pub destination: Option<DestinationFacts>,
    /// Actual compressed store; None means unqualified. Disk may block while
    /// raw encoder state is retained, independently of final writer access.
    pub staging: Option<StagingFacts>,
    /// Actual final-destination scheduling guarantee. None is unqualified.
    pub output_schedule: Option<super::OutputScheduleOffer>,
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
            conversion: None,
            destination: None,
            staging: None,
            output_schedule: None,
            admitted_encoded_bytes: 0,
            workspace_upper_bound: None,
        }
    }
}

/// Evidence for the conversion retained together with a frame encoder.
/// These are adapter guarantees, not instructions to reconstruct a plan.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct FrameConversionOffer {
    /// Interpretation and timing actually delivered to the encoder. Original
    /// input facts remain in the question; output epochs are independent.
    pub encoder_input: FrameFacts,
    /// True only for a qualified sample-preserving route; false includes
    /// arithmetic, quantization, clipping and unknown preservation.
    pub preserves_samples: bool,
    /// Whether conversion guarantees retention of extended-range values.
    /// This says nothing about the following encoder's precision or fidelity.
    pub preserves_extended_range: bool,
    /// Explicit fallback for undeclared siting. A declaration cannot be replaced.
    pub assumed_chroma_siting: Option<ChromaSiting>,
    /// Managed conversion storage retained/reserved for this selected route.
    /// Shared alternatives must not be summed. Native workspace is separate.
    pub admitted_bytes: usize,
}
impl FrameConversionOffer {
    /// Target facts with preservation unqualified until the adapter proves it.
    pub const fn new(encoder_input: FrameFacts) -> Self {
        Self {
            encoder_input,
            preserves_samples: false,
            preserves_extended_range: false,
            assumed_chroma_siting: None,
            admitted_bytes: 0,
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
    pulse: &dyn crate::Pulse,
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
        pulse,
    )
}
