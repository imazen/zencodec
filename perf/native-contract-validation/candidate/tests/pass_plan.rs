//! Candidate pass-plan contract, driven by a scripted encoder through the
//! patched prototype driver (copied into the candidate export's `tests/`).
//!
//! - Admission compares the encoder's planned maximum with the pass budget
//!   before any source read; an unbounded plan is never admitted under one.
//! - An encoder that asks for a pass its own plan excludes is refused before
//!   that pass reads anything.
//! - `PassOutput::FinalPass` forbids pending bytes before the final pass and
//!   is checked before publication, but promises nothing about bytes during
//!   the final pass: an empty or cancelled final pass is legal.
//! - The control is a `Pulse`: `check` only cancels, `advance` counts rows
//!   supplied to the encoder, and the total is exact only once known.
use enough::{Stop, StopReason};
use multipass_validation::{
    Completed, Failure, Input, MultiPassEncoder, Options, PassCount, PassOutput, PassPlan,
    PassProgress, PassRows, PassSource, Preset, Refusal, Replay, ReplayPolicy, ReplayRoute,
    RowRequest, Stage, encode_passes,
};
use std::{
    error::Error,
    fmt, io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, AtomicU64, Ordering::Relaxed},
    },
};
use zencodec::how_far::{Child, Execution, NoPulse, PhaseSpec, PlanError, Pulse, Report, Total};
use zencodec::{
    At,
    encode::{EncodeOutput, OutputTiming},
    find_cause,
    operation::{OutputSchedule, PolicySet, StagingKind},
    output_patch::{OutputAccess, OutputFinish},
    storage::StorageBudget,
};
use zencodec_io::{destination::Forward, row_encode::Description};
use zenpixels::{PixelDescriptor, PixelSliceMut};

const W: u32 = 4;
const H: u32 = 6;
const RB: usize = W as usize * 3;

#[derive(Debug)]
struct MockError;
impl fmt::Display for MockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("scripted encoder interrupted")
    }
}
impl Error for MockError {}

/// Requests `passes` full passes, `band` rows at a time, and flags pass
/// `final_at` as final. After each filled request in a pass listed in `emit`
/// it makes two bytes pending.
struct Script {
    declared: PassCount,
    output: PassOutput,
    timing: OutputTiming,
    passes: u32,
    final_at: u32,
    band: u32,
    emit: Vec<u32>,
    pass: u32,
    next: u32,
    buf: Vec<u8>,
    pending: Vec<u8>,
    taken: usize,
}
impl Script {
    /// `passes` passes, the last flagged final, no bytes before finish.
    fn new(declared: PassCount, passes: u32) -> Self {
        Self {
            declared,
            output: PassOutput::FinalPass,
            timing: OutputTiming::AT_FINISH,
            passes,
            final_at: passes - 1,
            band: 4,
            emit: Vec::new(),
            pass: 0,
            next: 0,
            buf: vec![0; RB * H as usize],
            pending: Vec::new(),
            taken: 0,
        }
    }
    fn emitting(mut self, passes: &[u32]) -> Self {
        self.emit = passes.to_vec();
        self.timing = OutputTiming::streaming(0);
        self
    }
}
impl MultiPassEncoder for Script {
    type Error = MockError;
    fn begin_passes(&mut self, _: &Description, _: OutputAccess) -> Result<PassPlan, MockError> {
        Ok(PassPlan {
            passes: self.declared,
            output: self.output,
            timing: self.timing,
        })
    }
    fn pull_rows(
        &mut self,
        rows: &mut dyn PassRows,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<PassProgress, MockError> {
        if self.pass == self.passes {
            return Ok(PassProgress::Done);
        }
        let n = self.band.min(H - self.next);
        let view = PixelSliceMut::new(
            &mut self.buf[..RB * n as usize],
            W,
            n,
            RB,
            PixelDescriptor::RGB8_SRGB,
        )
        .unwrap();
        rows.fill(RowRequest {
            pass: self.pass,
            final_pass: self.pass == self.final_at,
            label: self.pass as u8,
            first: self.next,
            rows: view,
        })
        .map_err(|_| MockError)?;
        if self.emit.contains(&self.pass) {
            self.pending.extend_from_slice(&[self.pass as u8; 2]);
        }
        self.next += n;
        if self.next == H {
            self.next = 0;
            self.pass += 1;
        }
        Ok(PassProgress::Filled)
    }
    fn pending_output(&self) -> &[u8] {
        &self.pending[self.taken..]
    }
    fn consume_output(&mut self, n: usize) {
        self.taken = (self.taken + n).min(self.pending.len());
    }
    fn finish_output(self) -> Result<OutputFinish<EncodeOutput>, MockError> {
        let mut tail = self.pending[self.taken..].to_vec();
        tail.extend_from_slice(b"end");
        Ok(OutputFinish::new(
            EncodeOutput::new(tail, zencodec::ImageFormat::Unknown),
            Vec::new(),
        ))
    }
}

/// A positional source whose counter increments on every read attempt.
struct Counting {
    rows: Arc<AtomicU32>,
    replay: Replay,
}
impl PassSource for Counting {
    type Error = io::Error;
    fn description(&self) -> Description {
        Description::new(W, H, PixelDescriptor::RGB8_SRGB)
    }
    fn replay(&self) -> Replay {
        self.replay
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        _: Option<&dyn zencodec::Pulse>,
    ) -> io::Result<()> {
        self.rows.fetch_add(rows.rows(), Relaxed);
        for y in 0..rows.rows() {
            rows.row_mut(y).fill((first + y) as u8);
        }
        Ok(())
    }
}
fn positional() -> (Counting, Arc<AtomicU32>) {
    let rows = Arc::new(AtomicU32::new(0));
    (
        Counting {
            rows: rows.clone(),
            replay: Replay::Positional,
        },
        rows,
    )
}

/// Records progress and totals; optionally cancels once `trip` reads happened.
#[derive(Default)]
struct Recording {
    advanced: AtomicU64,
    checks: AtomicU64,
    totals: Mutex<Vec<Total>>,
    trip: Option<(Arc<AtomicU32>, u32)>,
}
impl Stop for Recording {
    fn check(&self) -> Result<(), StopReason> {
        self.checks.fetch_add(1, Relaxed);
        match &self.trip {
            Some((rows, at)) if rows.load(Relaxed) >= *at => Err(StopReason::Cancelled),
            _ => Ok(()),
        }
    }
}
impl Report for Recording {
    fn advance(&self, completed: u64) {
        self.advanced.fetch_add(completed, Relaxed);
    }
}
impl Pulse for Recording {
    fn split(&self, _: Execution, _: &[PhaseSpec<'_>]) -> Result<Vec<Child<'_>>, PlanError> {
        Err(PlanError::Unsupported)
    }
    fn set_total(&self, total: Total) -> Result<(), PlanError> {
        self.totals.lock().unwrap().push(total);
        Ok(())
    }
}

fn options(policy: ReplayPolicy) -> Options {
    Options::new(StorageBudget::new(1 << 20, 64), u64::MAX).with_replay_policy(policy)
}
fn budget(max: u32, staging: bool) -> ReplayPolicy {
    ReplayPolicy {
        max_source_passes: Some(max),
        forbid_memory_staging: !staging,
        ..Default::default()
    }
}
fn refusal(e: &At<Failure>) -> Option<Refusal> {
    find_cause::<Refusal>(e).copied()
}

/// Runs with live output when the script streams, else finish-first.
fn run(
    enc: Script,
    src: Counting,
    opts: Options,
    pulse: &dyn Pulse,
) -> (Result<Completed, At<Failure>>, Vec<u8>) {
    let policy = if enc.timing.streams() {
        PolicySet::new().require_output_schedule(OutputSchedule::Live)
    } else {
        PolicySet::new()
    };
    let mut out = Vec::new();
    let r = encode_passes(
        enc,
        Input::source(src),
        Forward(&mut out),
        policy,
        Preset,
        opts.with_output_policy(policy),
        Some(pulse),
    );
    (r, out)
}

const PASS: u32 = H;

#[test]
fn planned_maximum_over_the_budget_is_refused_before_any_read() {
    // The content would need two passes; the plan allows three.
    let (src, rows) = positional();
    let (r, out) = run(
        Script::new(PassCount::Range { min: 2, max: 3 }, 2),
        src,
        options(budget(2, false)),
        &NoPulse,
    );
    let err = r.unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::PassBudget { pass: 0 }));
    assert_eq!(err.error().stage, Stage::Policy);
    assert_eq!(rows.load(Relaxed), 0, "admission precedes every read");
    assert_eq!(err.error().trace.source_bytes, 0);
    assert!(out.is_empty());

    // Admitted staging reads the source once; later passes replay from it.
    let (src, rows) = positional();
    let (r, out) = run(
        Script::new(PassCount::Range { min: 2, max: 3 }, 3),
        src,
        options(budget(2, true)),
        &NoPulse,
    );
    let done = r.unwrap();
    assert_eq!(done.route, ReplayRoute::Stage(StagingKind::Memory));
    assert_eq!(rows.load(Relaxed), PASS);
    assert_eq!(done.trace.passes, 3);
    assert_eq!(out, b"end");

    // A budget at the planned maximum admits re-reading the source.
    let (src, rows) = positional();
    let (r, _) = run(
        Script::new(PassCount::Range { min: 2, max: 3 }, 3),
        src,
        options(budget(3, false)),
        &NoPulse,
    );
    assert_eq!(r.unwrap().route, ReplayRoute::Source);
    assert_eq!(rows.load(Relaxed), 3 * PASS);
}

#[test]
fn unbounded_plan_is_never_admitted_to_reread_under_a_budget() {
    let (src, rows) = positional();
    let (r, out) = run(
        Script::new(PassCount::AtLeast(2), 2),
        src,
        options(budget(1000, false)),
        &NoPulse,
    );
    let err = r.unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::PassBudget { pass: 0 }));
    assert_eq!(rows.load(Relaxed), 0);
    assert!(out.is_empty());
    // Without a budget it runs: no bound is invented either way.
    let (src, rows) = positional();
    let (r, _) = run(
        Script::new(PassCount::AtLeast(2), 3),
        src,
        options(ReplayPolicy::default()),
        &NoPulse,
    );
    assert_eq!(r.unwrap().trace.passes, 3);
    assert_eq!(rows.load(Relaxed), 3 * PASS);
}

#[test]
fn passes_outside_the_declared_plan_are_refused_before_reading_them() {
    // (declared, passes requested, final pass ordinal, refusal, rows read)
    let cases = [
        (
            PassCount::Exact(2),
            3,
            2,
            Refusal::PassLimit { pass: 2 },
            2 * PASS,
        ),
        (
            PassCount::Range { min: 2, max: 3 },
            4,
            3,
            Refusal::PassLimit { pass: 3 },
            3 * PASS,
        ),
        // Another pass after the one flagged final.
        (
            PassCount::AtLeast(2),
            3,
            1,
            Refusal::PassLimit { pass: 2 },
            2 * PASS,
        ),
        // A final pass that would end below the declared minimum.
        (
            PassCount::Range { min: 3, max: 4 },
            2,
            1,
            Refusal::IncompletePass { pass: 1 },
            PASS,
        ),
    ];
    for (declared, passes, final_at, want, read) in cases {
        let (src, rows) = positional();
        let mut enc = Script::new(declared, passes);
        enc.final_at = final_at;
        let (r, out) = run(enc, src, options(ReplayPolicy::default()), &NoPulse);
        let err = r.unwrap_err();
        assert_eq!(refusal(&err), Some(want), "{declared:?}");
        assert_eq!(err.error().stage, Stage::Read, "{declared:?}");
        assert_eq!(
            rows.load(Relaxed),
            read,
            "{declared:?}: refused pass unread"
        );
        assert!(out.is_empty(), "{declared:?}");
    }
}

#[test]
fn invalid_plans_are_refused_before_reading() {
    for declared in [PassCount::Exact(0), PassCount::Range { min: 3, max: 2 }] {
        let (src, rows) = positional();
        let (r, _) = run(
            Script::new(declared, 2),
            src,
            options(ReplayPolicy::default()),
            &NoPulse,
        );
        assert_eq!(refusal(&r.unwrap_err()), Some(Refusal::InvalidPlan));
        assert_eq!(rows.load(Relaxed), 0);
    }
}

#[test]
fn bytes_before_the_final_pass_are_refused_before_publication() {
    let (src, _) = positional();
    let (r, out) = run(
        Script::new(PassCount::Exact(2), 2).emitting(&[0, 1]),
        src,
        options(ReplayPolicy::default()),
        &NoPulse,
    );
    let err = r.unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::EarlyOutput { pass: 0 }));
    assert_eq!(err.error().emitted, 0);
    assert!(out.is_empty(), "live output wrote nothing");
    // The same bytes are legal when the encoder declares any-pass output.
    let (src, _) = positional();
    let mut enc = Script::new(PassCount::Exact(2), 2).emitting(&[0, 1]);
    enc.output = PassOutput::AnyPass;
    let (r, out) = run(enc, src, options(ReplayPolicy::default()), &NoPulse);
    r.unwrap();
    assert_eq!(&out[..2], &[0, 0], "first-pass bytes published live");
}

#[test]
fn a_final_pass_need_not_produce_bytes() {
    // Final-pass output declared, nothing pending during any pass.
    let (src, _) = positional();
    let (r, out) = run(
        Script::new(PassCount::Exact(2), 2),
        src,
        options(ReplayPolicy::default()),
        &NoPulse,
    );
    assert_eq!(r.unwrap().schedule, OutputSchedule::FinishFirst);
    assert_eq!(out, b"end");
    // Streaming in the final pass only is published live.
    let (src, _) = positional();
    let (r, out) = run(
        Script::new(PassCount::Exact(2), 2).emitting(&[1]),
        src,
        options(ReplayPolicy::default()),
        &NoPulse,
    );
    assert_eq!(r.unwrap().schedule, OutputSchedule::Live);
    assert_eq!(out, [1, 1, 1, 1, b'e', b'n', b'd']);
}

#[test]
fn cancellation_in_the_final_pass_is_not_a_plan_violation() {
    let (src, rows) = positional();
    let pulse = Recording {
        trip: Some((rows.clone(), PASS + 2)),
        ..Default::default()
    };
    let (r, out) = run(
        Script::new(PassCount::Exact(2), 2).emitting(&[1]),
        src,
        options(ReplayPolicy::default()),
        &pulse,
    );
    let err = r.unwrap_err();
    assert!(
        find_cause::<zencodec::CodecError>(&err).is_some(),
        "stop retained"
    );
    assert_eq!(
        refusal(&err),
        None,
        "no plan refusal for a cancelled final pass"
    );
    assert_eq!(err.error().pass, Some(1));
    assert_eq!(err.error().emitted, out.len() as u64);
}

#[test]
fn progress_counts_supplied_rows_and_totals_are_exact_only_when_known() {
    // Exact plan: exact from the start, again when the final pass begins.
    let (src, _) = positional();
    let pulse = Recording::default();
    let (r, _) = run(
        Script::new(PassCount::Exact(3), 3),
        src,
        options(ReplayPolicy::default()),
        &pulse,
    );
    r.unwrap();
    let rows = u64::from(3 * PASS);
    assert_eq!(pulse.advanced.load(Relaxed), rows);
    assert_eq!(
        *pulse.totals.lock().unwrap(),
        [Total::Exact(rows), Total::Exact(rows)]
    );
    assert!(pulse.checks.load(Relaxed) > 0, "checks are not progress");

    // A range is unknown until the final pass resolves it.
    let (src, _) = positional();
    let pulse = Recording::default();
    let (r, _) = run(
        Script::new(PassCount::Range { min: 2, max: 4 }, 3),
        src,
        options(ReplayPolicy::default()),
        &pulse,
    );
    r.unwrap();
    assert_eq!(pulse.advanced.load(Relaxed), rows);
    assert_eq!(
        *pulse.totals.lock().unwrap(),
        [Total::Unknown, Total::Exact(rows)]
    );

    // Staged replay: rows supplied from staging are progress too, while the
    // forward source is read once.
    let rows_read = Arc::new(AtomicU32::new(0));
    let src = Counting {
        rows: rows_read.clone(),
        replay: Replay::Forward,
    };
    let pulse = Recording::default();
    let (r, _) = run(
        Script::new(PassCount::Exact(3), 3),
        src,
        options(ReplayPolicy::default()),
        &pulse,
    );
    assert_eq!(r.unwrap().route, ReplayRoute::Stage(StagingKind::Memory));
    assert_eq!(rows_read.load(Relaxed), PASS);
    assert_eq!(pulse.advanced.load(Relaxed), rows);
}
