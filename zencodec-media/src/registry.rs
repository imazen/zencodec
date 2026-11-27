//! Media roles and factories registered in [`zencodec::CodecSet`].
//!
//! Defines [`ContainerId`], the four media roles ([`DemuxRole`], [`MuxRole`],
//! [`VideoDecodeRole`], [`VideoEncodeRole`]), and the quote/open factory traits
//! (`DemuxFactory`, `MuxFactory`, `VideoDecodeFactory`, `VideoEncodeFactory`).
//!
//! All media backends register into the exact same [`zencodec::CodecSet`] as image
//! decoders/encoders via [`zencodec::CodecSet::with_role`], avoiding any second
//! product registry or dependency cycle.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::fmt;

use zencodec::decode::{BoxedError, ErasedError, Input};
use zencodec::encode::Fidelity;
use zencodec::plane::PlanarLayout;
use zencodec::source::{AccessPattern, NativeBound, SourceAccess};
use zencodec::time::TimeBase;
use zencodec::{FactoryRole, JobResources, Pulse};

use crate::container::{DynDemuxer, DynMuxer, MuxHint, MuxOffer, SeekOffer, SeekTarget};
use crate::frame::FrameInterpretation;
use crate::limits::MediaLimitExceeded;
use crate::session::{CodecConfig, SessionError, UnsupportedMedia, VideoDecoder, VideoEncoder};
use crate::track::{CodecId, TrackInfo};

/// Owned custom container identifier for containers not listed in [`ContainerId`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContainerTag(Box<str>);

impl ContainerTag {
    /// Construct a custom container tag.
    #[must_use]
    pub fn new(tag: &str) -> Self {
        Self(String::from(tag).into_boxed_str())
    }

    /// Borrow the tag text.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContainerTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Media container identity, kept strictly distinct from still [`zencodec::ImageFormat`]
/// and bitstream [`CodecId`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ContainerId {
    /// ISO Base Media File Format / MP4 (`.mp4`, `.m4v`, fragmented CMAF).
    Mp4,
    /// WebM / Matroska video subset (`.webm`, `.mkv`).
    Webm,
    /// Indeo Video Format raw elementary container (`.ivf`).
    Ivf,
    /// Extension container identified by a caller-owned tag.
    Custom(ContainerTag),
}

impl fmt::Display for ContainerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mp4 => f.write_str("MP4"),
            Self::Webm => f.write_str("WebM"),
            Self::Ivf => f.write_str("IVF"),
            Self::Custom(tag) => f.write_str(tag.as_str()),
        }
    }
}

/// Typed [`CodecSet`](zencodec::CodecSet) role for container demuxers keyed by [`ContainerId`].
pub enum DemuxRole {}
impl FactoryRole for DemuxRole {
    type Key = ContainerId;
    type Factory = dyn DemuxFactory;
}

/// Typed [`CodecSet`](zencodec::CodecSet) role for container muxers keyed by [`ContainerId`].
pub enum MuxRole {}
impl FactoryRole for MuxRole {
    type Key = ContainerId;
    type Factory = dyn MuxFactory;
}

/// Typed [`CodecSet`](zencodec::CodecSet) role for video decoders keyed by [`CodecId`].
pub enum VideoDecodeRole {}
impl FactoryRole for VideoDecodeRole {
    type Key = CodecId;
    type Factory = dyn VideoDecodeFactory;
}

/// Typed [`CodecSet`](zencodec::CodecSet) role for video encoders keyed by [`CodecId`].
pub enum VideoEncodeRole {}
impl FactoryRole for VideoEncodeRole {
    type Key = CodecId;
    type Factory = dyn VideoEncodeFactory;
}

/// Thread-local (non-`Send` permitted) type-erased video decoder session.
pub type LocalVideoDecoder = dyn VideoDecoder<Error = ErasedError>;
/// Thread-local (non-`Send` permitted) type-erased video encoder session.
pub type LocalVideoEncoder = dyn VideoEncoder<Error = ErasedError>;

/// Bounds placed on safe container header probing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ProbeLimits {
    /// Maximum prefix bytes inspected during container probing.
    pub max_probe_bytes: usize,
    /// Maximum container boxes/elements walked while confirming format identity.
    pub max_boxes: u32,
}

impl Default for ProbeLimits {
    fn default() -> Self {
        Self {
            max_probe_bytes: 64 * 1024,
            max_boxes: 64,
        }
    }
}

impl ProbeLimits {
    /// Construct explicit probe bounds.
    #[inline]
    #[must_use]
    pub const fn new(max_probe_bytes: usize, max_boxes: u32) -> Self {
        Self {
            max_probe_bytes,
            max_boxes,
        }
    }
}

/// Outcome of probing a container prefix with a [`DemuxFactory`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContainerProbe {
    /// Prefix definitively does not match this container.
    NoMatch,
    /// Prefix is consistent so far, but at least `min_bytes` total prefix bytes are needed.
    Need {
        /// Minimum total bytes from offset 0 needed to reach a verdict.
        min_bytes: usize,
    },
    /// Prefix structurally confirmed this container.
    Confirmed {
        /// Confirmed container identity.
        container: ContainerId,
        /// Confidence score (`1..=100`; higher wins tiebreaks).
        confidence: u8,
    },
}

/// Timing exactness requirement for container muxing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TimingPolicy {
    /// Allow carrier-standard timestamp representation where exact ticks are not mandatory.
    #[default]
    BestEffort,
    /// Require exact PTS, DTS, and explicit duration preservation (refuses rounding/rebasing).
    Exact,
}

/// Caller requirements when quoting a [`DemuxFactory`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct DemuxNeeds {
    /// Optional target seek required on the demuxed stream.
    pub seek: Option<SeekTarget>,
    /// Whether the route requires immutable byte replay (e.g., index build or rewind).
    pub immutable_replay_required: bool,
}

/// Immutable, bounded admission quote returned by [`DemuxFactory::quote`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DemuxOffer {
    /// Container served by this offer.
    pub container: ContainerId,
    /// Required byte access pattern (`Sequential`, `BoundedBacktrack`, or `RandomAccess`).
    pub required_access: AccessPattern,
    /// Conservative working memory bound for demuxer headers/tables.
    pub workspace: NativeBound<u64>,
    /// Optional seek admission details if `DemuxNeeds::seek` was requested.
    pub seek_offer: Option<SeekOffer>,
    /// Whether the opened session is safe to move across threads.
    pub is_send: bool,
}

impl DemuxOffer {
    /// Construct a basic demuxer admission quote.
    #[must_use]
    pub fn new(container: ContainerId, required_access: AccessPattern) -> Self {
        Self {
            container,
            required_access,
            workspace: NativeBound::Unknown,
            seek_offer: None,
            is_send: true,
        }
    }

    /// Declare conservative demuxer workspace bound.
    #[must_use]
    pub const fn with_workspace(mut self, workspace: NativeBound<u64>) -> Self {
        self.workspace = workspace;
        self
    }

    /// Attach seek admission details.
    #[must_use]
    pub const fn with_seek_offer(mut self, offer: SeekOffer) -> Self {
        self.seek_offer = Some(offer);
        self
    }
}

/// Caller requirements when quoting a [`MuxFactory`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MuxNeeds {
    /// Tracks to be written into the container.
    pub tracks: Arc<[TrackInfo]>,
    /// Actual destination capabilities (e.g. `forward_only` for unseekable pipes/sockets).
    pub destination: MuxHint,
    /// Required timestamp preservation policy.
    pub timing: TimingPolicy,
    /// Optional upper bound on packet count when finite and known upfront.
    pub maximum_packets: Option<u64>,
}

impl MuxNeeds {
    /// Construct muxer requirements for `tracks`.
    #[must_use]
    pub fn new(tracks: impl Into<Arc<[TrackInfo]>>, destination: MuxHint) -> Self {
        Self {
            tracks: tracks.into(),
            destination,
            timing: TimingPolicy::BestEffort,
            maximum_packets: None,
        }
    }

    /// Set timing exactness policy.
    #[must_use]
    pub const fn with_timing(mut self, timing: TimingPolicy) -> Self {
        self.timing = timing;
        self
    }

    /// Declare finite packet count bound.
    #[must_use]
    pub const fn with_maximum_packets(mut self, max_packets: u64) -> Self {
        self.maximum_packets = Some(max_packets);
        self
    }
}

/// Immutable, bounded admission plan returned by [`MuxFactory::quote`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MuxPlan {
    /// Container produced by this plan.
    pub container: ContainerId,
    /// Admitted tracks.
    pub tracks: Arc<[TrackInfo]>,
    /// Destination hint bound at quote time.
    pub destination: MuxHint,
    /// Timing exactness bound at quote time.
    pub timing_policy: TimingPolicy,
    /// Output schedule, pending buffer, workspace, and finish-tail bounds.
    pub offer: MuxOffer,
    /// Whether the opened muxer implements [`BoundedMuxer`](crate::container::BoundedMuxer).
    pub supports_bounded_writes: bool,
}

impl MuxPlan {
    /// Construct a muxer admission plan.
    #[must_use]
    pub fn new(
        container: ContainerId,
        tracks: Arc<[TrackInfo]>,
        destination: MuxHint,
        timing_policy: TimingPolicy,
        offer: MuxOffer,
    ) -> Self {
        Self {
            container,
            tracks,
            destination,
            timing_policy,
            offer,
            supports_bounded_writes: false,
        }
    }
}

/// Caller requirements when quoting a [`VideoDecodeFactory`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct VideoDecodeNeeds {
    /// Maximum coded `(width, height)` admitted for this stream.
    pub maximum_coded_extent: (u32, u32),
    /// Accepted output planar layouts (empty means any layout within the admitted envelope).
    pub accepted_layouts: Arc<[PlanarLayout]>,
    /// Maximum simultaneously held decoded pictures (leased outputs + reorder).
    pub max_held_pictures: usize,
}

impl VideoDecodeNeeds {
    /// Construct decode requirements for a maximum coded extent.
    #[must_use]
    pub fn new(maximum_coded_extent: (u32, u32), max_held_pictures: usize) -> Self {
        Self {
            maximum_coded_extent,
            accepted_layouts: Arc::from([]),
            max_held_pictures,
        }
    }
}

/// Immutable, bounded admission plan returned by [`VideoDecodeFactory::quote`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct VideoDecodePlan {
    /// Bound initial codec configuration.
    pub config: CodecConfig,
    /// Admitted maximum coded `(width, height)`.
    pub maximum_coded_extent: (u32, u32),
    /// Conservative native decoder workspace bound (references + reorder + working buffers).
    pub workspace: NativeBound<u64>,
    /// Maximum concurrent picture slots retained by the decoder.
    pub max_retained_pictures: usize,
    /// Whether the opened decoder is safe to move across threads (`Send`).
    pub is_send: bool,
}

impl VideoDecodePlan {
    /// Construct a video decode admission plan.
    #[must_use]
    pub fn new(
        config: CodecConfig,
        maximum_coded_extent: (u32, u32),
        workspace: NativeBound<u64>,
        max_retained_pictures: usize,
    ) -> Self {
        Self {
            config,
            maximum_coded_extent,
            workspace,
            max_retained_pictures,
            is_send: true,
        }
    }
}

/// Caller requirements when quoting a [`VideoEncodeFactory`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct VideoEncodeNeeds {
    /// Target compressed bitstream codec.
    pub codec: CodecId,
    /// Exact input frame geometry, planar layout, and color interpretation.
    pub interpretation: Arc<FrameInterpretation>,
    /// Presentation clock for encoded frames.
    pub clock: TimeBase,
    /// Optional requested fidelity (quality / lossless / target).
    pub fidelity: Option<Fidelity>,
    /// Maximum simultaneously in-flight frames held by the encoder (lookahead + references).
    pub max_in_flight_frames: usize,
}

impl VideoEncodeNeeds {
    /// Construct encode requirements.
    #[must_use]
    pub fn new(
        codec: CodecId,
        interpretation: Arc<FrameInterpretation>,
        clock: TimeBase,
        max_in_flight_frames: usize,
    ) -> Self {
        Self {
            codec,
            interpretation,
            clock,
            fidelity: None,
            max_in_flight_frames,
        }
    }

    /// Attach a requested [`Fidelity`].
    #[must_use]
    pub fn with_fidelity(mut self, fidelity: Fidelity) -> Self {
        self.fidelity = Some(fidelity);
        self
    }
}

/// Immutable, bounded admission plan returned by [`VideoEncodeFactory::quote`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct VideoEncodePlan {
    /// Target codec.
    pub codec: CodecId,
    /// Admitted input frame interpretation expected on every `send`.
    pub interpretation: Arc<FrameInterpretation>,
    /// Admitted clock.
    pub clock: TimeBase,
    /// Admitted fidelity, if bound.
    pub fidelity: Option<Fidelity>,
    /// Conservative native encoder workspace bound.
    pub workspace: NativeBound<u64>,
    /// Maximum frames held in lookahead/reference queue before emitting packets.
    pub lag_frames: u32,
    /// Whether the opened encoder is safe to move across threads (`Send`).
    pub is_send: bool,
}

impl VideoEncodePlan {
    /// Construct a video encode admission plan.
    #[must_use]
    pub fn new(
        codec: CodecId,
        interpretation: Arc<FrameInterpretation>,
        clock: TimeBase,
        workspace: NativeBound<u64>,
        lag_frames: u32,
    ) -> Self {
        Self {
            codec,
            interpretation,
            clock,
            fidelity: None,
            workspace,
            lag_frames,
            is_send: true,
        }
    }

    /// Record bound fidelity on the plan.
    #[must_use]
    pub fn with_fidelity(mut self, fidelity: Option<Fidelity>) -> Self {
        self.fidelity = fidelity;
        self
    }
}

/// Error returned by media factories, probing, and erased demuxer operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum MediaError {
    /// Valid request this provider cannot satisfy (reversible for candidate fallback before open).
    Unsupported(UnsupportedMedia),
    /// A structural or resource limit was exceeded.
    Limit(MediaLimitExceeded),
    /// Operation was cancelled via pulse.
    Cancelled(enough::StopReason),
    /// Invalid or malformed input or configuration.
    Invalid(&'static str),
    /// Underlying codec, container, or session failure with preserved cause chain.
    Backend(BoxedError),
}

impl MediaError {
    /// Wrap any `Error + Send + Sync + 'static` preserving its cause chain.
    #[must_use]
    pub fn backend(err: impl core::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(Box::new(err))
    }

    /// Returns `true` when this error is a pre-open capability refusal eligible for
    /// candidate fallback (malformed input, limits, and cancellation are never reversible).
    #[inline]
    #[must_use]
    pub const fn is_capability_refusal(&self) -> bool {
        matches!(self, Self::Unsupported(_))
    }
}

impl From<UnsupportedMedia> for MediaError {
    fn from(u: UnsupportedMedia) -> Self {
        Self::Unsupported(u)
    }
}

impl From<MediaLimitExceeded> for MediaError {
    fn from(l: MediaLimitExceeded) -> Self {
        Self::Limit(l)
    }
}

impl From<enough::StopReason> for MediaError {
    fn from(r: enough::StopReason) -> Self {
        Self::Cancelled(r)
    }
}

impl<E: core::error::Error + Send + Sync + 'static> From<SessionError<E>> for MediaError {
    fn from(err: SessionError<E>) -> Self {
        match err {
            SessionError::Unsupported(u) => Self::Unsupported(u),
            SessionError::Limit(l) => Self::Limit(l),
            SessionError::Cancelled(r) => Self::Cancelled(r),
            other => Self::Backend(Box::new(other)),
        }
    }
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(u) => write!(f, "unsupported media capability: {u:?}"),
            Self::Limit(l) => write!(f, "media limit exceeded: {l}"),
            Self::Cancelled(r) => write!(f, "media operation cancelled: {r:?}"),
            Self::Invalid(msg) => write!(f, "invalid media request: {msg}"),
            Self::Backend(e) => write!(f, "media backend error: {e}"),
        }
    }
}

impl core::error::Error for MediaError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Limit(l) => Some(l),
            Self::Backend(e) => Some(&**e),
            _ => None,
        }
    }
}

/// Shareable factory for probing, quoting, and opening container demuxers.
pub trait DemuxFactory: Send + Sync {
    /// Inspect an initial byte prefix within `caps` without mutating state.
    fn probe(&self, input: Input<'_>, caps: &ProbeLimits) -> Result<ContainerProbe, MediaError>;

    /// Produce an immutable admission quote for `need` against actual `source` capabilities
    /// and `run` limits without reading bytes or allocating session workspace.
    fn quote(
        &self,
        need: &DemuxNeeds,
        source: &SourceAccess,
        run: &JobResources,
    ) -> Result<DemuxOffer, MediaError>;

    /// Instantiate a fresh type-erased demuxer session bound to `offer` and `run`.
    fn open(
        &self,
        offer: &DemuxOffer,
        run: &JobResources,
    ) -> Result<Box<dyn DynDemuxer + Send>, MediaError>;
}

/// Shareable factory for quoting and opening container muxers.
pub trait MuxFactory: Send + Sync {
    /// Produce an immutable admission plan for `need` under `run`.
    fn quote(&self, need: &MuxNeeds, run: &JobResources) -> Result<MuxPlan, MediaError>;

    /// Instantiate a fresh type-erased muxer session bound to `plan` and `run`.
    fn open(
        &self,
        plan: &MuxPlan,
        run: &JobResources,
    ) -> Result<Box<dyn DynMuxer + Send>, MediaError>;
}

/// Shareable factory for quoting and opening video decoder sessions.
pub trait VideoDecodeFactory: Send + Sync {
    /// Produce an immutable admission plan for `config` and `need` under `run`.
    fn quote(
        &self,
        config: &CodecConfig,
        need: &VideoDecodeNeeds,
        run: &JobResources,
    ) -> Result<VideoDecodePlan, MediaError>;

    /// Instantiate a fresh video decoder session bound to `plan` and `run`.
    fn open(
        &self,
        plan: &VideoDecodePlan,
        run: &JobResources,
    ) -> Result<Box<LocalVideoDecoder>, MediaError>;

    /// Optional `Send` session constructor when the backend supports moving sessions across threads.
    fn open_send(
        &self,
        plan: &VideoDecodePlan,
        run: &JobResources,
    ) -> Result<Box<crate::session::DynVideoDecoder>, MediaError> {
        let _ = (plan, run);
        Err(MediaError::Unsupported(UnsupportedMedia::Codec))
    }
}

/// Shareable factory for quoting and opening video encoder sessions.
pub trait VideoEncodeFactory: Send + Sync {
    /// Produce an immutable admission plan for `need` under `run`.
    fn quote(
        &self,
        need: &VideoEncodeNeeds,
        run: &JobResources,
    ) -> Result<VideoEncodePlan, MediaError>;

    /// Instantiate a fresh video encoder session bound to `plan` and `run`.
    fn open(
        &self,
        plan: &VideoEncodePlan,
        run: &JobResources,
    ) -> Result<Box<LocalVideoEncoder>, MediaError>;

    /// Optional `Send` session constructor when the backend supports moving sessions across threads.
    fn open_send(
        &self,
        plan: &VideoEncodePlan,
        run: &JobResources,
    ) -> Result<Box<crate::session::DynVideoEncoder>, MediaError> {
        let _ = (plan, run);
        Err(MediaError::Unsupported(UnsupportedMedia::Codec))
    }
}

/// Convenience helper: check whether `pulse` has tripped, mapping to [`MediaError::Cancelled`].
#[inline]
pub fn check_media_pulse(pulse: Option<&dyn Pulse>) -> Result<(), MediaError> {
    if let Some(p) = pulse {
        p.check().map_err(MediaError::Cancelled)?;
    }
    Ok(())
}
