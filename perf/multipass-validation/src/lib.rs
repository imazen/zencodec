//! Prototype of bounded multi-pass raster input (see this harness's README.md).
//!
//! The codec drives passes by asking for rows; the driver decides where a
//! later pass gets them. A borrowed raster is read again through the retained immutable view. A positional
//! source is read again only with per-row digests, plus identity evidence when
//! the source has it. A forward source needs explicitly admitted staging or a
//! refusal before any read. The codec never spools raw rows itself. The output
//! schedule still goes through zencodec's existing resolver.
//!
//! Pass plan candidate (native contract validation): the encoder states its
//! source passes as a range from its own plan, admission compares the
//! largest count before any read, and a final-pass output declaration is
//! enforced before publication. The operation control is a `Pulse`: `check`
//! is cancellation only, `advance` counts rows actually supplied to the
//! encoder, and the total is exact only once the pass count is resolved.
#![forbid(unsafe_code)]

pub mod sources;

use std::{
    error::Error,
    fmt,
    fs::File,
    io::{self, Write},
    os::unix::fs::FileExt,
};

use zencodec::how_far::Total;
use zencodec::{
    At,
    encode::{EncodeDeclaration, EncodeOutput, OutputTiming},
    operation::{
        OperationPolicy, OperationReport, OutputSchedule, OutputScheduleFacts, OutputScheduleOffer,
        OutputSource, PolicySet, StagingFacts, StagingKind, resolve_output_schedule,
    },
    output_patch::{OutputAccess, OutputFinish},
    storage::{OwnedStorage, PreparedStorage, StorageBudget},
};
use zencodec_io::{
    destination::Destination,
    row_encode::{Description, ExactDepthContradiction},
};
use zenpixels::PixelSliceMut;

// ---------------------------------------------------------------- source side

/// How a later pass can obtain the same samples again. This describes the
/// actual source, not the format. Seek support alone is never evidence of
/// stable content; see [`ReplayEvidence`] for what each route can establish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Replay {
    /// Readable again by row position (file, object store range reads).
    /// Replayed rows are compared with per-row digests of their first read;
    /// that detects accidental change, it does not prove identical content.
    Positional,
    /// Readable once, top to bottom (socket, decoder output, pipe).
    Forward,
}

/// What a replay route can establish about later passes seeing the same
/// samples as the first. Ordered from strongest to weakest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ReplayEvidence {
    /// One pass only: nothing is replayed.
    NotReplayed,
    /// An immutable borrow (language guarantee, retained directly by `Input::raster`).
    Borrow,
    /// Driver-owned memory written once from the first pass.
    OwnedStaging,
    /// The source's own version evidence (identity unchanged at every pass
    /// start and at completion) plus per-row digests. Only as strong as the
    /// source's version mechanism: an ETag/If-Match conditional read or a
    /// filesystem snapshot is strong; inode+length+mtime is weak (in-place
    /// writes within the timestamp granularity are invisible to it).
    IdentityAndDigest,
    /// Per-row 64-bit digests only (positional source without identity, or
    /// a caller-owned disk staging file). Detects accidental change; it is not
    /// a cryptographic commitment and cannot prove exact equality.
    Digest,
}

/// Opaque version evidence (length, inode, mtime, ETag hash...). It allows
/// an early refusal before rows are read. It is not proof of identical bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceIdentity(pub [u64; 4]);

/// Proposed additions to `zencodec_io::row_encode::Source`: `replay` and
/// `identity` with conservative defaults, so existing sources stay `Forward`.
/// `read_rows` keeps its existing signature; the source does not need to know
/// which pass is reading.
pub trait PassSource {
    /// Original source error; retained in the failure chain.
    type Error: Error + Send + Sync + 'static;
    /// Fixed declaration, re-checked around every read.
    fn description(&self) -> Description;
    /// Repeatability of this actual source.
    fn replay(&self) -> Replay {
        Replay::Forward
    }
    /// Current version evidence, if the source has any.
    fn identity(&mut self) -> Result<Option<SourceIdentity>, Self::Error> {
        Ok(None)
    }
    /// Fill every active sample of `rows` starting at row `first`, honoring the
    /// destination stride. `Forward` sources are called once per row in order.
    fn read_rows(
        &mut self,
        first: u32,
        rows: PixelSliceMut<'_>,
        stop: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error>;
}

/// Actual input retained by the driver, not an implementor's replay claim.
/// A raster view is frozen once; callbacks cannot substitute its rows later.
pub enum Input<'a, S = NoSource> {
    /// The driver itself reads this immutable view on every pass.
    Raster(zenpixels::PixelSlice<'a>),
    /// An external source; exact replay requires admitted owned staging.
    Source(S),
}
/// Uninhabited source parameter for a direct immutable raster.
pub enum NoSource {}
impl PassSource for NoSource {
    type Error = core::convert::Infallible;
    fn description(&self) -> Description {
        match *self {}
    }
    fn read_rows(
        &mut self,
        _: u32,
        _: PixelSliceMut<'_>,
        _: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error> {
        match *self {}
    }
}
impl<'a> Input<'a, NoSource> {
    /// Retain the actual pixels, including stride and color context.
    pub fn raster(pixels: zenpixels::PixelSlice<'a>) -> Self {
        Self::Raster(pixels)
    }
}
impl<S> Input<'_, S> {
    /// Use an external positional or forward source.
    pub fn source(source: S) -> Self {
        Self::Source(source)
    }
}

// --------------------------------------------------------------- encoder side

/// How many times the configured encoder will read each source row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PassCount {
    /// Known before the first read.
    Exact(u32),
    /// Lower bound; the codec states no upper bound.
    AtLeast(u32),
    /// Content decides, within bounds taken from the codec's own plan.
    Range {
        /// Passes every input takes.
        min: u32,
        /// Most passes any input can take.
        max: u32,
    },
}
impl PassCount {
    /// Lower bound.
    pub const fn min(self) -> u32 {
        match self {
            Self::Exact(n) | Self::AtLeast(n) | Self::Range { min: n, .. } => n,
        }
    }
    /// Upper bound from the codec's plan; `None` when it states none. A source
    /// pass budget never re-reads a source for an unbounded count.
    pub const fn max(self) -> Option<u32> {
        match self {
            Self::Exact(n) | Self::Range { max: n, .. } => Some(n),
            Self::AtLeast(_) => None,
        }
    }
    fn admits(self, passes: u32) -> bool {
        passes >= self.min() && self.max().is_none_or(|max| passes <= max)
    }
}

/// When an encoder's bytes may first be pending, relative to source passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PassOutput {
    /// No byte is pending before the final pass's first request is filled.
    /// The driver refuses an earlier byte before publishing or staging it.
    /// This does not promise bytes during the final pass: its timing may be
    /// `AT_FINISH`, and an empty or cancelled final pass publishes nothing.
    FinalPass,
    /// Bytes may be pending during any pass.
    AnyPass,
}

/// What `begin_passes` commits to.
#[derive(Clone, Copy, Debug)]
pub struct PassPlan {
    /// Source reads per row.
    pub passes: PassCount,
    /// When bytes may first be pending.
    pub output: PassOutput,
    /// Byte availability and patches, as `Encoder::begin_output` reports them.
    /// With `PassOutput::FinalPass`, `lag_rows` counts rows of the final pass.
    pub timing: OutputTiming,
}

// Compatibility names keep the experiment's callers on the shared row protocol.
pub use zencodec::encode::{
    EncodeRows as PassRows, PullProgress as PassProgress, RowDemand as RowRequest,
    RowFillError as Interrupted,
};

/// Compatibility interface for the experimental driver and protocol fixtures.
/// Shared `Encoder` implementations use the blanket projection below. The
/// driver owns and drops the encoder on any refusal. In particular, projection
/// can refuse after successful `begin_declared` committed native state; callers
/// must discard that instance instead of retrying or draining its output.
pub trait MultiPassEncoder: Sized {
    /// Codec error.
    type Error: Error + Send + Sync + 'static;
    /// Declare the image and actual output access before any row moves.
    /// Access selects layout (seek-back vs forward); never quality/effort/metadata.
    fn begin_passes(
        &mut self,
        desc: &Description,
        access: OutputAccess,
    ) -> Result<PassPlan, Self::Error>;
    /// Run native work until one row request is filled or every pass is done.
    fn pull_rows(
        &mut self,
        rows: &mut dyn PassRows,
        stop: &dyn zencodec::Pulse,
    ) -> Result<PassProgress, Self::Error>;
    /// Bytes ready to send, as `Encoder::pending_output`.
    fn pending_output(&self) -> &[u8];
    /// Mark pending bytes sent.
    fn consume_output(&mut self, n: usize);
    /// Remaining bytes and ordered patches, as `Encoder::finish_output`.
    fn finish_output(self) -> Result<OutputFinish<EncodeOutput>, Self::Error>;
}

// Drive a real shared Encoder through the existing experimental replay and
// publication checks. The duplicate plan is only a compatibility projection;
// production policy/driver integration remains a separate gate.
impl<E: zencodec::encode::Encoder> MultiPassEncoder for E {
    type Error = E::Error;
    fn begin_passes(
        &mut self,
        desc: &Description,
        access: OutputAccess,
    ) -> Result<PassPlan, E::Error> {
        let mut declaration =
            EncodeDeclaration::new(desc.width, desc.height, desc.descriptor).with_access(access);
        declaration.color = desc.color_context.as_deref();
        declaration.exact_depth = desc.exact_depth;
        let plan = self.begin_declared(&declaration)?;
        let zencodec::encode::InputRequirement::Replay(range) = plan.input else {
            return Err(E::reject(zencodec::UnsupportedOperation::PullEncode));
        };
        let passes = match range.max() {
            Some(max) => PassCount::Range {
                min: range.min(),
                max,
            },
            None => PassCount::AtLeast(range.min()),
        };
        let output = match plan.output_phase {
            zencodec::encode::OutputPhase::AnyPass => PassOutput::AnyPass,
            zencodec::encode::OutputPhase::FinalPass => PassOutput::FinalPass,
            _ => return Err(E::reject(zencodec::UnsupportedOperation::PullEncode)),
        };
        Ok(PassPlan {
            passes,
            output,
            timing: plan.timing,
        })
    }
    fn pull_rows(
        &mut self,
        rows: &mut dyn PassRows,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<PassProgress, E::Error> {
        zencodec::encode::Encoder::pull_rows(self, rows, pulse)
    }
    fn pending_output(&self) -> &[u8] {
        zencodec::encode::Encoder::pending_output(self)
    }
    fn consume_output(&mut self, n: usize) {
        zencodec::encode::Encoder::consume_output(self, n)
    }
    fn finish_output(self) -> Result<OutputFinish<EncodeOutput>, E::Error> {
        zencodec::encode::Encoder::finish_output(self)
    }
}

// --------------------------------------------------------------- replay policy

/// Where later passes read rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReplayRoute {
    /// Read the source again (borrowed or digest-verified positional).
    Source,
    /// Copy the first pass into admitted staging; later passes read it.
    Stage(StagingKind),
}

/// Facts at the replay decision: no pixel bytes.
#[derive(Clone, Copy, Debug)]
pub struct ReplayFacts {
    /// Actual source repeatability.
    pub replay: Replay,
    /// Encoder's declared pass count.
    pub passes: PassCount,
    /// Active sample bytes of one complete pass (padding excluded).
    pub pass_bytes: u64,
}

/// One executable replay route, prepared before the question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayOffer {
    /// Route executed if selected.
    pub route: ReplayRoute,
    /// Raw sample bytes this route stages (already reserved for memory).
    pub staged_bytes: u64,
    /// Source passes this route performs: the encoder's count for `Source`, one when staged.
    pub source_passes: PassCount,
    /// What this route can establish about replayed samples.
    pub evidence: ReplayEvidence,
}

/// Answer to a replay question.
#[derive(Debug)]
pub enum ReplayDecision {
    /// Select the offer at this index.
    Choose(usize),
    /// Let `ReplayPolicy` answer.
    UsePreset,
    /// Refuse the operation.
    Decline,
}

/// Prototype stand-in for a proposed `Question::SourceReplay`.
pub trait ReplayResolver {
    /// Pick a route or refuse.
    fn resolve(
        &mut self,
        facts: &ReplayFacts,
        offers: &[ReplayOffer],
    ) -> Result<ReplayDecision, Box<dyn Error + Send + Sync>>;
}
impl<F> ReplayResolver for F
where
    F: FnMut(&ReplayFacts, &[ReplayOffer]) -> Result<ReplayDecision, Box<dyn Error + Send + Sync>>,
{
    fn resolve(
        &mut self,
        facts: &ReplayFacts,
        offers: &[ReplayOffer],
    ) -> Result<ReplayDecision, Box<dyn Error + Send + Sync>> {
        self(facts, offers)
    }
}
/// Always delegate to the preset.
pub struct Preset;
impl ReplayResolver for Preset {
    fn resolve(
        &mut self,
        _: &ReplayFacts,
        _: &[ReplayOffer],
    ) -> Result<ReplayDecision, Box<dyn Error + Send + Sync>> {
        Ok(ReplayDecision::UsePreset)
    }
}

/// Built-in replay preferences (proposed `PolicySet` fields).
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplayPolicy {
    /// Most source passes permitted. A `Source` route is offered only when the
    /// encoder's planned maximum is within it, so the budget is decided before
    /// any read; an unbounded count needs staging. `None` means unlimited.
    pub max_source_passes: Option<u32>,
    /// Never offer memory staging of raw rows.
    pub forbid_memory_staging: bool,
    /// Offer only routes whose replay is exact by construction (borrow or
    /// driver-owned memory), never digest-based evidence.
    pub require_exact_replay: bool,
}

// --------------------------------------------------------------------- driver

/// Driver options. One `StorageBudget` admits raw staging, digests and
/// finish-first compressed staging. Native codec workspace is not covered.
pub struct Options {
    storage: StorageBudget,
    max_output_bytes: u64,
    stage_file: Option<File>,
    max_stage_file_bytes: u64,
    replay: ReplayPolicy,
    output_preset: PolicySet,
}
impl Options {
    /// Storage pool and total output allowance.
    pub fn new(storage: StorageBudget, max_output_bytes: u64) -> Self {
        Self {
            storage,
            max_output_bytes,
            stage_file: None,
            max_stage_file_bytes: 0,
            replay: ReplayPolicy::default(),
            output_preset: PolicySet::new(),
        }
    }
    /// Offer raw-row disk staging in this caller-owned file (written from
    /// offset 0, never truncated or deleted by the driver), up to `max_bytes`.
    pub fn with_stage_file(mut self, file: File, max_bytes: u64) -> Self {
        self.stage_file = Some(file);
        self.max_stage_file_bytes = max_bytes;
        self
    }
    /// Built-in replay policy.
    pub fn with_replay_policy(mut self, policy: ReplayPolicy) -> Self {
        self.replay = policy;
        self
    }
    /// Built-in output schedule policy (existing resolver).
    pub fn with_output_policy(mut self, preset: PolicySet) -> Self {
        self.output_preset = preset;
        self
    }
}

/// Operation boundary that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// Declarations and `begin_passes`.
    Prepare,
    /// Replay or output schedule selection.
    Policy,
    /// Source reads, identity and digest checks.
    Read,
    /// Raw staging write or read.
    Staging,
    /// Native encoding or compressed staging.
    Encode,
    /// `finish_output`.
    Finish,
    /// Final destination writes.
    Write,
    /// Seek-back patches.
    Patch,
}

/// Driver contract refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// Empty geometry.
    Empty,
    /// Size arithmetic overflow.
    Overflow,
    /// Declaration, identity or replayed content changed at this pass.
    SourceChanged {
        /// Pass ordinal that observed the change.
        pass: u32,
    },
    /// Multiple passes, forward source, and no admitted staging route.
    NotReplayable,
    /// This pass would exceed `max_source_passes`.
    PassBudget {
        /// Pass ordinal refused before reading.
        pass: u32,
    },
    /// The encoder broke the request contract (order, geometry or pass ordinal).
    RequestOrder {
        /// Pass ordinal of the offending request.
        pass: u32,
        /// Its first row.
        first: u32,
    },
    /// Replay policy refused every route.
    Declined,
    /// No route meets `require_exact_replay`.
    EvidenceRequired,
    /// The encoder broke the pull protocol: `Filled` without exactly one
    /// fill, a second fill in one call, a fill after an interrupted request,
    /// or `Done` with a fill.
    Protocol,
    /// `Done` arrived before this pass covered every row, or before the
    /// declared pass count, or without the declared final pass.
    IncompletePass {
        /// Last pass ordinal started (0 when none started).
        pass: u32,
    },
    /// The encoder asked for a pass beyond its own declared maximum, or for
    /// another pass after its final one. Refused before reading it.
    PassLimit {
        /// Pass ordinal refused before reading.
        pass: u32,
    },
    /// Bytes were pending before the declared final pass began. Refused
    /// before they were published or staged.
    EarlyOutput {
        /// Last pass ordinal started.
        pass: u32,
    },
    /// The pass plan is contradictory (no pass, or maximum below minimum).
    InvalidPlan,
    /// Output exceeds the allowance.
    OutputLimit,
    /// Patches need a seek-back destination.
    Patches,
}
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "multi-pass encoding refused: {self:?}")
    }
}
impl Error for Refusal {}

type Cause = Box<dyn Error + Send + Sync>;

/// Where a request's rows came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadFrom {
    /// The caller's source.
    Source,
    /// Driver staging.
    Staging,
}

/// One filled request, recorded in the trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadRecord {
    /// Pass ordinal.
    pub pass: u32,
    /// Codec pass label.
    pub label: u8,
    /// First row.
    pub first: u32,
    /// Row count.
    pub rows: u32,
    /// Origin of the samples.
    pub from: ReadFrom,
}

/// Bounded execution evidence. `reads` grows with requests (bands x passes)
/// and is diagnostic storage outside the admitted budget.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    /// Every filled request in order.
    pub reads: Vec<ReadRecord>,
    /// Passes started.
    pub passes: u32,
    /// Active bytes read from the caller's source.
    pub source_bytes: u64,
    /// Active bytes read back from staging.
    pub staging_reads: u64,
    /// Raw staging admitted (memory) or allowed (disk) for this route.
    pub staging_admitted: u64,
    /// Bytes charged for per-row digests.
    pub digest_bytes: u64,
}

/// Terminal error. Display omits payloads.
pub struct Failure {
    /// Boundary that failed.
    pub stage: Stage,
    /// Pass ordinal at failure, if passes began.
    pub pass: Option<u32>,
    /// Confirmed destination appends.
    pub emitted: u64,
    /// Confirmed overwritten bytes.
    pub patched_bytes: u64,
    /// Selected replay route, if selection completed.
    pub route: Option<ReplayRoute>,
    /// Reads performed before failure.
    pub trace: Box<Trace>,
    /// Output schedule history.
    pub report: Box<OperationReport>,
    cause: Cause,
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiPassFailure")
            .field("stage", &self.stage)
            .field("pass", &self.pass)
            .field("emitted", &self.emitted)
            .field("patched_bytes", &self.patched_bytes)
            .field("route", &self.route)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "multi-pass encoding failed during {:?} (pass {:?}) after {} appended bytes",
            self.stage, self.pass, self.emitted
        )
    }
}
impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

/// Successful completion.
#[derive(Debug)]
pub struct Completed {
    /// Confirmed appended bytes.
    pub emitted: u64,
    /// Confirmed overwritten bytes.
    pub patched_bytes: u64,
    /// Executed output schedule.
    pub schedule: OutputSchedule,
    /// Executed replay route.
    pub route: ReplayRoute,
    /// Reads and admission evidence.
    pub trace: Trace,
    /// Output schedule history.
    pub report: OperationReport,
}

fn check(stop: &dyn zencodec::Pulse) -> Result<(), Cause> {
    stop.check()
        .map_err(|r| At::wrap(zencodec::CodecError::new(None, r.into())).into())
}

/// Word-at-a-time 64-bit digest of one row's active bytes. Detects accidental
/// change between passes; it is not a cryptographic commitment.
fn digest(bytes: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut h = (bytes.len() as u64).wrapping_mul(K);
    let (words, rest) = bytes.as_chunks::<8>();
    for w in words {
        h = (h ^ u64::from_le_bytes(*w)).wrapping_mul(K).rotate_left(29);
    }
    let mut tail = [0u8; 8];
    tail[..rest.len()].copy_from_slice(rest);
    (h ^ u64::from_le_bytes(tail)).wrapping_mul(K) ^ (h >> 31)
}

enum Staging {
    None,
    Memory(OwnedStorage<u8>),
    Disk(File),
}

/// State shared by the driver's `PassRows` implementation.
struct Filler<'a, S: PassSource> {
    source: &'a mut S,
    desc: &'a Description,
    row_bytes: usize,
    route: ReplayRoute,
    staging: Staging,
    digests: Option<OwnedStorage<u64>>,
    identity: Option<SourceIdentity>,
    max_source_passes: Option<u32>,
    /// The encoder's own pass plan, enforced before each pass is read.
    declared: PassCount,
    /// The current pass is the encoder's final pass.
    final_started: bool,
    pass: Option<u32>,
    next_row: u32,
    trace: Trace,
    failure: Option<(Stage, Cause)>,
    fills: u32,
    stop: &'a dyn zencodec::Pulse,
}

impl<S: PassSource> Filler<'_, S> {
    fn source_pass(&self, pass: u32) -> bool {
        pass == 0 || self.route == ReplayRoute::Source
    }
    fn validate(&self, pass: u32) -> Result<(), Cause> {
        if self.source.description() != *self.desc {
            return Err(Refusal::SourceChanged { pass }.into());
        }
        Ok(())
    }
    fn begin_pass(&mut self, pass: u32, final_pass: bool) -> Result<(), (Stage, Cause)> {
        let stage = Stage::Read;
        match self.pass {
            None if pass == 0 => {}
            // Every pass covers every row once before the next begins; staging
            // in particular must hold every row before anything replays from it.
            Some(p) if pass == p + 1 && self.next_row == self.desc.height => {}
            _ => return Err((stage, Refusal::RequestOrder { pass, first: 0 }.into())),
        }
        // The encoder's own plan bounds its passes: nothing after its final
        // pass or beyond its declared maximum, and a final pass that ends below
        // the declared minimum. All refused before this pass reads anything.
        if self.final_started || self.declared.max().is_some_and(|max| pass >= max) {
            return Err((stage, Refusal::PassLimit { pass }.into()));
        }
        if final_pass && pass + 1 < self.declared.min() {
            return Err((stage, Refusal::IncompletePass { pass }.into()));
        }
        if self.source_pass(pass)
            && let Some(max) = self.max_source_passes
            && pass >= max
        {
            return Err((Stage::Policy, Refusal::PassBudget { pass }.into()));
        }
        if self.source_pass(pass) && pass > 0 {
            self.validate(pass).map_err(|e| (stage, e))?;
            if self.identity.is_some() {
                let now = self
                    .source
                    .identity()
                    .map_err(|e| (stage, Box::new(e) as Cause))?;
                if now != self.identity {
                    return Err((stage, Refusal::SourceChanged { pass }.into()));
                }
            }
        }
        self.pass = Some(pass);
        self.final_started = final_pass;
        self.trace.passes = pass + 1;
        self.next_row = 0;
        if final_pass {
            // The pass count is resolved: the denominator is exact from here.
            let rows = u64::from(pass + 1) * u64::from(self.desc.height);
            let _ = self.stop.set_total(Total::Exact(rows));
        }
        Ok(())
    }
    fn fill_inner(&mut self, req: RowRequest<'_>) -> Result<(), (Stage, Cause)> {
        let RowRequest {
            pass,
            final_pass,
            label,
            first,
            mut rows,
            ..
        } = req;
        check(self.stop).map_err(|e| (Stage::Read, e))?;
        if self.pass != Some(pass) {
            self.begin_pass(pass, final_pass)?;
        } else if final_pass != self.final_started {
            // The final flag is a property of the whole pass.
            return Err((Stage::Read, Refusal::RequestOrder { pass, first }.into()));
        }
        let order = |pass, first| {
            (
                Stage::Read,
                Box::new(Refusal::RequestOrder { pass, first }) as Cause,
            )
        };
        let count = rows.rows();
        if rows.width() != self.desc.width
            || rows.descriptor() != self.desc.descriptor
            || rows.color_context() != self.desc.color_context.as_ref()
            || count == 0
            || first
                .checked_add(count)
                .is_none_or(|end| end > self.desc.height)
        {
            return Err(order(pass, first));
        }
        // Every pass is top to bottom without gaps or repeats.
        if first != self.next_row {
            return Err(order(pass, first));
        }
        let from = if self.source_pass(pass) {
            self.validate(pass).map_err(|e| (Stage::Read, e))?;
            let stride = rows.stride();
            let desc = rows.descriptor();
            let ctx = rows.color_context().cloned();
            let reborrow = {
                let mut v = PixelSliceMut::new(
                    rows.as_strided_bytes_mut(),
                    self.desc.width,
                    count,
                    stride,
                    desc,
                )
                .map_err(|e| (Stage::Read, Box::new(e) as Cause))?;
                if let Some(c) = ctx {
                    v = v.with_color_context(c);
                }
                v
            };
            self.source
                .read_rows(first, reborrow, self.stop)
                .map_err(|e| (Stage::Read, Box::new(e) as Cause))?;
            check(self.stop).map_err(|e| (Stage::Read, e))?;
            self.validate(pass).map_err(|e| (Stage::Read, e))?;
            self.trace.source_bytes += (self.row_bytes * count as usize) as u64;
            ReadFrom::Source
        } else {
            ReadFrom::Staging
        };
        let rb = self.row_bytes;
        for y in 0..count {
            let row = rows.row_mut(y);
            let at = (first + y) as usize;
            let offset = (at * rb) as u64;
            match (&mut self.staging, from) {
                (Staging::Memory(m), ReadFrom::Source) => {
                    m.as_mut_slice()[at * rb..][..rb].copy_from_slice(row)
                }
                (Staging::Memory(m), ReadFrom::Staging) => {
                    row.copy_from_slice(&m.as_slice()[at * rb..][..rb])
                }
                (Staging::Disk(f), ReadFrom::Source) => f
                    .write_all_at(row, offset)
                    .map_err(|e| (Stage::Staging, Box::new(e) as Cause))?,
                (Staging::Disk(f), ReadFrom::Staging) => f
                    .read_exact_at(row, offset)
                    .map_err(|e| (Stage::Staging, Box::new(e) as Cause))?,
                (Staging::None, _) => {}
            }
            if let Some(exact) = self.desc.exact_depth {
                check(self.stop).map_err(|e| (Stage::Read, e))?;
                // Every supplied strip must satisfy the source declaration,
                // including rows replayed from private staging. Keep the whole
                // request unaccepted when any active sample contradicts it.
                let pixels =
                    zenpixels::PixelSlice::new(row, self.desc.width, 1, rb, self.desc.descriptor)
                        .map_err(|e| (Stage::Read, Box::new(e) as Cause))?;
                if !exact
                    .holds_for(&pixels)
                    .map_err(|e| (Stage::Read, Box::new(e) as Cause))?
                {
                    return Err((
                        Stage::Read,
                        ExactDepthContradiction {
                            first_row: first + y,
                            claim: exact,
                        }
                        .into(),
                    ));
                }
            }
            if let Some(d) = &mut self.digests {
                let h = digest(row);
                let slot = &mut d.as_mut_slice()[at];
                if pass == 0 {
                    *slot = h;
                } else if *slot != h {
                    let stage = if from == ReadFrom::Source {
                        Stage::Read
                    } else {
                        Stage::Staging
                    };
                    return Err((stage, Refusal::SourceChanged { pass }.into()));
                }
            }
        }
        if from == ReadFrom::Staging {
            self.trace.staging_reads += (rb * count as usize) as u64;
        }
        // Cancellation callbacks during verification may change source facts.
        if self.desc.exact_depth.is_some() {
            self.validate(pass).map_err(|e| (Stage::Read, e))?;
        }
        // Progress is rows supplied to the encoder, never a cancellation check.
        self.stop.advance(u64::from(count));
        self.next_row = first + count;
        self.trace.reads.push(ReadRecord {
            pass,
            label,
            first,
            rows: count,
            from,
        });
        Ok(())
    }
}

impl<S: PassSource> PassRows for Filler<'_, S> {
    fn fill(&mut self, request: RowRequest<'_>) -> Result<(), Interrupted> {
        self.fills += 1;
        if self.failure.is_some() {
            return Err(Interrupted);
        }
        // One request per pull: a second fill would read the source again
        // before the driver can drain output or check the first.
        if self.fills > 1 {
            self.failure = Some((Stage::Encode, Refusal::Protocol.into()));
            return Err(Interrupted);
        }
        self.fill_inner(request).map_err(|f| {
            self.failure = Some(f);
            Interrupted
        })
    }
}

fn count_output(total: &mut u64, len: usize, limit: u64) -> Result<(), Cause> {
    let next = total.checked_add(len as u64).ok_or(Refusal::Overflow)?;
    if next > limit {
        return Err(Refusal::OutputLimit.into());
    }
    *total = next;
    Ok(())
}

fn write_all(
    writer: &mut impl Write,
    mut bytes: &[u8],
    emitted: &mut u64,
    stop: &dyn zencodec::Pulse,
) -> Result<(), Cause> {
    while !bytes.is_empty() {
        check(stop)?;
        match writer.write(bytes) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
            Ok(n) if n <= bytes.len() => {
                *emitted += n as u64;
                bytes = &bytes[n..];
            }
            Ok(_) => return Err(io::Error::other("writer returned an invalid count").into()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn stage_output(
    bytes: &[u8],
    storage: &StorageBudget,
    packets: &mut Vec<OwnedStorage<u8>>,
) -> Result<(), Cause> {
    if !bytes.is_empty() {
        packets.try_reserve(1)?;
        let mut owned = storage.bytes(bytes.len())?;
        owned.as_mut_slice().copy_from_slice(bytes);
        packets.push(owned);
    }
    Ok(())
}

/// Encode a multi-pass configuration from `source` to `writer`.
///
/// Before any source read or output byte: `begin_passes`, replay route
/// selection (asking `replay` only when the source is not borrowed and more
/// than one pass is possible), and the existing output schedule question.
/// Forward sources with no admitted staging refuse here, and so does a pass
/// budget below the encoder's planned maximum without staging. Positional
/// replay is digest-verified per row and identity-checked per pass. Every
/// failure keeps its original cause, stage, pass and confirmed output counts.
///
/// `stop` is a `Pulse`: checked for cancellation and advanced by the rows
/// supplied to the encoder; its total is exact only when the pass count is known.
pub fn encode_passes<E, S, D, R, Q>(
    encoder: E,
    input: Input<'_, S>,
    writer: D,
    resolver: R,
    replay: Q,
    options: Options,
    stop: &dyn zencodec::Pulse,
) -> Result<Completed, At<Failure>>
where
    E: MultiPassEncoder,
    S: PassSource,
    D: Destination,
    R: OperationPolicy,
    Q: ReplayResolver,
{
    match input {
        Input::Raster(pixels) => encode_inner(
            encoder,
            sources::Borrowed::new(pixels),
            true,
            writer,
            resolver,
            replay,
            options,
            stop,
        ),
        Input::Source(source) => encode_inner(
            encoder, source, false, writer, resolver, replay, options, stop,
        ),
    }
}

// `borrowed` is true only for the concrete Raster arm above. Never derive it
// from a public token or an arbitrary source's claim.
#[allow(clippy::too_many_arguments)]
fn encode_inner<E, S, D, R, Q>(
    mut encoder: E,
    mut source: S,
    borrowed: bool,
    mut writer: D,
    mut resolver: R,
    mut replay: Q,
    mut options: Options,
    stop: &dyn zencodec::Pulse,
) -> Result<Completed, At<Failure>>
where
    E: MultiPassEncoder,
    S: PassSource,
    D: Destination,
    R: OperationPolicy,
    Q: ReplayResolver,
{
    let mut boundary = Stage::Prepare;
    let mut pass = None;
    let mut route = None;
    let mut emitted = 0;
    let mut patched_bytes = 0;
    let mut report = OperationReport::default();
    let mut trace = Trace::default();
    let run = || -> Result<OutputSchedule, Cause> {
        check(stop)?;
        let desc = source.description();
        let kind = source.replay();
        if desc.width == 0 || desc.height == 0 {
            return Err(Refusal::Empty.into());
        }
        let row_bytes = (desc.width as usize)
            .checked_mul(desc.descriptor.bytes_per_pixel())
            .ok_or(Refusal::Overflow)?;
        let pass_bytes = (row_bytes as u64)
            .checked_mul(u64::from(desc.height))
            .ok_or(Refusal::Overflow)?;
        let access = if writer.can_patch() {
            OutputAccess::SeekBack
        } else {
            OutputAccess::Forward
        };
        // Reuse the production declaration rules before even legacy native
        // startup. A source assertion is not a scan or a fidelity certificate.
        if let Some(exact) = desc.exact_depth {
            EncodeDeclaration::new(desc.width, desc.height, desc.descriptor)
                .with_exact_depth(exact)
                .validate_exact_depth()?;
        }
        let plan = encoder.begin_passes(&desc, access)?;
        if plan.timing.patches_at_finish() && !writer.can_patch() {
            return Err(Refusal::Patches.into());
        }
        if plan.passes.min() == 0 || plan.passes.max().is_some_and(|max| max < plan.passes.min()) {
            return Err(Refusal::InvalidPlan.into());
        }
        check(stop)?;
        // Rows supplied to the encoder: exact only when the pass count is.
        let _ = stop.set_total(match plan.passes.max() {
            Some(max) if max == plan.passes.min() => {
                Total::Exact(u64::from(max) * u64::from(desc.height))
            }
            _ => Total::Unknown,
        });

        // ---- replay route, before any read
        boundary = Stage::Policy;
        let single = plan.passes.max() == Some(1);
        let max = options.replay.max_source_passes;
        let policy = options.replay;
        // Version evidence is read before any sample; it decides the offer's evidence.
        let identity = if kind == Replay::Positional && !single && !borrowed {
            source.identity()?
        } else {
            None
        };
        let mut candidates: Vec<ReplayOffer> = Vec::new();
        if single || kind != Replay::Forward {
            candidates.push(ReplayOffer {
                route: ReplayRoute::Source,
                staged_bytes: 0,
                source_passes: plan.passes,
                evidence: match kind {
                    _ if single => ReplayEvidence::NotReplayed,
                    _ if borrowed => ReplayEvidence::Borrow,
                    _ if identity.is_some() => ReplayEvidence::IdentityAndDigest,
                    _ => ReplayEvidence::Digest,
                },
            });
        }
        // A retained raster re-reads its own memory, but a pass budget still
        // counts it: an encoder whose planned maximum exceeds the budget, or
        // that states none, needs staging.
        let borrowed_needs_staging =
            borrowed && max.is_some_and(|m| plan.passes.max().is_none_or(|x| x > m));
        if !single && (!borrowed || borrowed_needs_staging) {
            if options.stage_file.is_some() && pass_bytes <= options.max_stage_file_bytes {
                candidates.push(ReplayOffer {
                    route: ReplayRoute::Stage(StagingKind::Disk),
                    staged_bytes: pass_bytes,
                    source_passes: PassCount::Exact(1),
                    evidence: ReplayEvidence::Digest,
                });
            }
            if !policy.forbid_memory_staging {
                candidates.push(ReplayOffer {
                    route: ReplayRoute::Stage(StagingKind::Memory),
                    staged_bytes: pass_bytes,
                    source_passes: PassCount::Exact(1),
                    evidence: ReplayEvidence::OwnedStaging,
                });
            }
        }
        if candidates.is_empty() {
            return Err(Refusal::NotReplayable.into());
        }
        // Every route reads the source at least once: gate all of them by the
        // pass budget, and by evidence, before reserving or reading anything.
        // A route is admitted by the most source passes its plan can take; an
        // unbounded plan is never admitted under a budget.
        candidates.retain(|o| max.is_none_or(|m| o.source_passes.max().is_some_and(|x| x <= m)));
        if candidates.is_empty() {
            return Err(Refusal::PassBudget { pass: 0 }.into());
        }
        if policy.require_exact_replay {
            candidates.retain(|o| o.evidence <= ReplayEvidence::OwnedStaging);
            if candidates.is_empty() {
                return Err(Refusal::EvidenceRequired.into());
            }
        }
        let mut prepared: Option<PreparedStorage<u8>> = None;
        let mut offers: Vec<ReplayOffer> = Vec::with_capacity(candidates.len());
        for o in candidates {
            if o.route == ReplayRoute::Stage(StagingKind::Memory) {
                match usize::try_from(pass_bytes)
                    .ok()
                    .and_then(|len| options.storage.prepare_filled(len, 0u8).ok())
                {
                    Some(p) => prepared = Some(p),
                    None => continue,
                }
            }
            offers.push(o);
        }
        if offers.is_empty() {
            return Err(Refusal::NotReplayable.into());
        }
        let facts = ReplayFacts {
            replay: kind,
            passes: plan.passes,
            pass_bytes,
        };
        // A direct retained raster has no replay tradeoff. If a pass budget
        // requires staging instead, its cost remains a situational decision.
        let direct_borrow = borrowed
            && offers
                .first()
                .is_some_and(|o| o.route == ReplayRoute::Source);
        let index = if direct_borrow || single {
            0
        } else {
            match replay.resolve(&facts, &offers)? {
                ReplayDecision::Choose(i) if i < offers.len() => i,
                ReplayDecision::Choose(_) => return Err(Refusal::Declined.into()),
                ReplayDecision::UsePreset => 0,
                ReplayDecision::Decline => return Err(Refusal::Declined.into()),
            }
        };
        let selected = offers[index];
        route = Some(selected.route);
        let staging = match selected.route {
            ReplayRoute::Source => Staging::None,
            ReplayRoute::Stage(StagingKind::Memory) => {
                Staging::Memory(prepared.take().expect("offered").allocate()?)
            }
            ReplayRoute::Stage(_) => Staging::Disk(options.stage_file.take().expect("offered")),
        };
        drop(prepared);
        trace.staging_admitted = selected.staged_bytes;
        let verify = matches!(
            selected.evidence,
            ReplayEvidence::IdentityAndDigest | ReplayEvidence::Digest
        );
        let digests = if verify {
            let d = options.storage.filled(desc.height as usize, 0u64)?;
            trace.digest_bytes = (desc.height as u64) * 8;
            Some(d)
        } else {
            None
        };
        let identity = if selected.evidence == ReplayEvidence::IdentityAndDigest {
            identity
        } else {
            None
        };

        // ---- output schedule, existing resolver
        let mut finish_first = OutputScheduleOffer::new(OutputSchedule::FinishFirst);
        finish_first.staging = Some(StagingFacts::new(
            StagingKind::Memory,
            0,
            options.storage.max_bytes() as u64,
        ));
        let schedules = [finish_first, OutputScheduleOffer::new(OutputSchedule::Live)];
        let schedules = &schedules[..if plan.timing.streams() { 2 } else { 1 }];
        let mechanism = if borrowed {
            OutputSource::BorrowedRaster
        } else {
            OutputSource::PullRows
        };
        let (chosen, record) = match resolve_output_schedule(
            OutputScheduleFacts::new(plan.timing, mechanism),
            schedules,
            &mut resolver,
            options.output_preset,
            stop,
        ) {
            Ok(s) => s,
            Err(error) => {
                report.record(error.error().record());
                return Err(error.into());
            }
        };
        report.record(record);
        let schedule = schedules[chosen].schedule;

        // ---- execute
        let mut filler = Filler {
            source: &mut source,
            desc: &desc,
            row_bytes,
            route: selected.route,
            staging,
            digests,
            identity,
            max_source_passes: max,
            declared: plan.passes,
            final_started: false,
            pass: None,
            next_row: 0,
            trace: core::mem::take(&mut trace),
            failure: None,
            fills: 0,
            stop,
        };
        let mut staged = Vec::new();
        let mut produced = 0u64;
        let result = (|| -> Result<(), Cause> {
            loop {
                check(stop)?;
                boundary = Stage::Encode;
                let pending = encoder.pending_output();
                // A final-pass declaration is checked before any byte leaves
                // the encoder: nothing is written or staged ahead of it.
                if plan.output == PassOutput::FinalPass
                    && !pending.is_empty()
                    && !filler.final_started
                {
                    return Err(Refusal::EarlyOutput {
                        pass: filler.pass.unwrap_or(0),
                    }
                    .into());
                }
                count_output(&mut produced, pending.len(), options.max_output_bytes)?;
                if schedule == OutputSchedule::Live {
                    boundary = Stage::Write;
                    write_all(&mut writer, pending, &mut emitted, stop)?;
                } else {
                    stage_output(pending, &options.storage, &mut staged)?;
                }
                let n = pending.len();
                encoder.consume_output(n);
                boundary = Stage::Encode;
                filler.fills = 0;
                let progress = encoder.pull_rows(&mut filler, stop);
                // A retained driver cause wins over any encoder result,
                // including a codec that swallowed `Interrupted`.
                if let Some((stage, cause)) = filler.failure.take() {
                    boundary = stage;
                    return Err(cause);
                }
                match progress {
                    Ok(PassProgress::Filled) if filler.fills == 1 => {}
                    Ok(PassProgress::Filled) => return Err(Refusal::Protocol.into()),
                    Ok(PassProgress::Done) => {
                        if filler.fills != 0 {
                            return Err(Refusal::Protocol.into());
                        }
                        let started = filler.pass.map_or(0, |p| p + 1);
                        let complete = filler.next_row == filler.desc.height;
                        let count_ok = plan.passes.admits(started);
                        // A final-pass declaration needs its final pass; the
                        // pass itself may have produced no bytes.
                        let final_ok = plan.output != PassOutput::FinalPass || filler.final_started;
                        if started == 0 || !complete || !count_ok || !final_ok {
                            boundary = Stage::Read;
                            return Err(Refusal::IncompletePass {
                                pass: filler.pass.unwrap_or(0),
                            }
                            .into());
                        }
                        return Ok(());
                    }
                    Ok(_) => return Err(Refusal::Protocol.into()),
                    Err(e) => {
                        if let Some((stage, cause)) = filler.failure.take() {
                            boundary = stage;
                            return Err(cause);
                        }
                        // A native stop is reported as the caller's stop reason.
                        check(stop)?;
                        return Err(Box::new(e));
                    }
                }
            }
        })();
        pass = filler.pass;
        let final_identity = if result.is_ok() && filler.identity.is_some() {
            Some(filler.source.identity())
        } else {
            None
        };
        let expected = filler.identity;
        trace = core::mem::take(&mut filler.trace);
        drop(filler);
        result?;
        if let Some(now) = final_identity {
            boundary = Stage::Read;
            if now? != expected {
                return Err(Refusal::SourceChanged {
                    pass: pass.unwrap_or(0),
                }
                .into());
            }
        }
        drop(source);
        drop(desc);
        boundary = Stage::Finish;
        check(stop)?;
        let finished = encoder.finish_output()?;
        finished.validate(produced)?;
        if !finished.patches.is_empty() && (!writer.can_patch() || !plan.timing.patches_at_finish())
        {
            return Err(Refusal::Patches.into());
        }
        let patches = finished.patches;
        let tail = finished.tail;
        count_output(&mut produced, tail.len(), options.max_output_bytes)?;
        boundary = Stage::Write;
        for bytes in staged.drain(..) {
            write_all(&mut writer, bytes.as_slice(), &mut emitted, stop)?;
        }
        write_all(&mut writer, tail.data(), &mut emitted, stop)?;
        if !patches.is_empty() {
            boundary = Stage::Patch;
            for patch in patches {
                check(stop)?;
                writer.position(patch.offset)?;
                write_all(&mut writer, &patch.bytes, &mut patched_bytes, stop)?;
            }
            writer.position(produced)?;
        }
        boundary = Stage::Write;
        writer.flush()?;
        check(stop)?;
        Ok(schedule)
    };
    match run() {
        Ok(schedule) => Ok(Completed {
            emitted,
            patched_bytes,
            schedule,
            route: route.expect("selected"),
            trace,
            report,
        }),
        Err(cause) => Err(At::wrap(Failure {
            stage: boundary,
            pass,
            emitted,
            patched_bytes,
            route,
            trace: Box::new(trace),
            report: Box::new(report),
            cause,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::digest;
    #[test]
    fn digest_sees_every_byte_and_length() {
        let a = [7u8; 13];
        for i in 0..13 {
            let mut b = a;
            b[i] ^= 1;
            assert_ne!(digest(&a), digest(&b), "byte {i}");
        }
        assert_ne!(digest(&a[..12]), digest(&a));
    }
}
