//! Adversarial controls for the pull protocol and replay gates, driven by a
//! scripted encoder (no codec, no private dependency). Each control runs the
//! real driver; native parity lives in `imi_native.rs`.
use enough::{Stop, };
use zencodec::how_far::NoPulse;
use multipass_validation::{
    Input, Interrupted, MultiPassEncoder, Options, PassCount, PassPlan, PassProgress, PassRows,
    PassSource, Preset, Refusal, Replay, ReplayDecision, ReplayEvidence, ReplayFacts, ReplayOffer,
    ReplayPolicy, ReplayRoute, RowRequest, Stage, encode_passes,
    sources::{Borrowed, ReaderRows},
};
use std::{cell::Cell, error::Error, fmt, io, io::Cursor, rc::Rc};
use zencodec::{
    At,
    encode::{EncodeOutput, OutputTiming},
    find_cause,
    operation::{PolicySet, StagingKind},
    output_patch::{OutputAccess, OutputFinish},
    storage::StorageBudget,
};
use zencodec_io::{destination::Forward, row_encode::Description};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

const W: u32 = 4;
const H: u32 = 6;
const RB: usize = W as usize * 3;

#[derive(Debug)]
struct MockError;
impl fmt::Display for MockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("mock encoder error")
    }
}
impl Error for MockError {}

/// One `pull_rows` call.
#[derive(Clone, Copy, Debug)]
enum Step {
    /// Fill (pass, first, rows) and return Filled; on Interrupted, either
    /// propagate an error or swallow it and return `then`.
    Fill(u32, u32, u32, Swallow),
    /// Two fills in one call.
    FillTwice(u32, u32, u32),
    /// Return Filled without filling.
    FilledEmpty,
    /// Return Done.
    Done,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Swallow {
    No,
    AsFilled,
    AsDone,
}

struct Scripted {
    plan: PassCount,
    steps: Vec<Step>,
    at: usize,
    buf: Vec<u8>,
    seen: Rc<Cell<u32>>,
    finished: Rc<Cell<bool>>,
}
impl Scripted {
    fn new(plan: PassCount, steps: Vec<Step>) -> Self {
        Self {
            plan,
            steps,
            at: 0,
            buf: vec![0; RB * H as usize],
            seen: Rc::new(Cell::new(0)),
            finished: Rc::new(Cell::new(false)),
        }
    }
    /// Every pass, `band` rows at a time, then Done.
    fn full(passes: u32, band: u32) -> Self {
        let mut steps = Vec::new();
        for p in 0..passes {
            let mut y = 0;
            while y < H {
                let n = band.min(H - y);
                steps.push(Step::Fill(p, y, n, Swallow::No));
                y += n;
            }
        }
        steps.push(Step::Done);
        Self::new(PassCount::Exact(passes), steps)
    }
    fn request(
        &mut self,
        rows: &mut dyn PassRows,
        pass: u32,
        first: u32,
        n: u32,
    ) -> Result<(), Interrupted> {
        let view = PixelSliceMut::new(
            &mut self.buf[..RB * n as usize],
            W,
            n,
            RB,
            PixelDescriptor::RGB8_SRGB,
        )
        .unwrap();
        let r = rows.fill(RowRequest {
            pass,
            label: pass as u8,
            first,
            rows: view,
        });
        if r.is_ok() {
            self.seen.set(self.seen.get() + n);
        }
        r
    }
}
impl MultiPassEncoder for Scripted {
    type Error = MockError;
    fn begin_passes(&mut self, _: &Description, _: OutputAccess) -> Result<PassPlan, MockError> {
        Ok(PassPlan {
            passes: self.plan,
            timing: OutputTiming::AT_FINISH,
        })
    }
    fn pull_rows(
        &mut self,
        rows: &mut dyn PassRows,
        _: &dyn zencodec::Pulse,
    ) -> Result<PassProgress, MockError> {
        let step = *self.steps.get(self.at).unwrap_or(&Step::FilledEmpty);
        self.at += 1;
        match step {
            Step::Fill(p, f, n, swallow) => match (self.request(rows, p, f, n), swallow) {
                (Ok(()), _) => Ok(PassProgress::Filled),
                (Err(_), Swallow::No) => Err(MockError),
                (Err(_), Swallow::AsFilled) => Ok(PassProgress::Filled),
                (Err(_), Swallow::AsDone) => Ok(PassProgress::Done),
            },
            Step::FillTwice(p, f, n) => {
                let _ = self.request(rows, p, f, n);
                let _ = self.request(rows, p, f + n, n);
                Ok(PassProgress::Filled)
            }
            Step::FilledEmpty => Ok(PassProgress::Filled),
            Step::Done => Ok(PassProgress::Done),
        }
    }
    fn pending_output(&self) -> &[u8] {
        &[]
    }
    fn consume_output(&mut self, _: usize) {}
    fn finish_output(self) -> Result<OutputFinish<EncodeOutput>, MockError> {
        self.finished.set(true);
        Ok(OutputFinish::new(
            EncodeOutput::new(b"ok".to_vec(), zencodec::ImageFormat::Unknown),
            Vec::new(),
        ))
    }
}

fn raster() -> Vec<u8> {
    (0..RB * H as usize).map(|i| (i * 7 % 251) as u8).collect()
}
fn desc() -> Description {
    Description::new(W, H, PixelDescriptor::RGB8_SRGB)
}
fn options() -> Options {
    Options::new(StorageBudget::new(1 << 20, 64), u64::MAX)
}
fn refusal(e: &At<multipass_validation::Failure>) -> Option<Refusal> {
    find_cause::<Refusal>(e).copied()
}

/// A callback-backed positional source: `gen(call, row)` produces each row.
/// It never holds a borrow, so it cannot claim `Borrowed`.
struct Callback<F> {
    make: F,
    calls: Rc<Cell<u32>>,
    fail_at: Option<u32>,
    replay: Replay,
}
impl<F: FnMut(u32, u32) -> u8> PassSource for Callback<F> {
    type Error = io::Error;
    fn description(&self) -> Description {
        desc()
    }
    fn replay(&self) -> Replay {
        self.replay
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        _: &dyn zencodec::Pulse,
    ) -> io::Result<()> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        if self.fail_at == Some(call) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "callback source failed",
            ));
        }
        for y in 0..rows.rows() {
            let v = (self.make)(call, first + y);
            rows.row_mut(y).fill(v);
        }
        Ok(())
    }
}
fn stable(_: u32, row: u32) -> u8 {
    row as u8
}

fn run<E: MultiPassEncoder, S: PassSource>(
    enc: E,
    src: S,
    opts: Options,
) -> (
    Result<multipass_validation::Completed, At<multipass_validation::Failure>>,
    Vec<u8>,
) {
    let mut out = Vec::new();
    let r = encode_passes(
        enc,
        Input::source(src),
        Forward(&mut out),
        PolicySet::new(),
        Preset,
        opts,
        &NoPulse,
    );
    (r, out)
}

#[test]
fn baseline_scripted_encode_completes() {
    let r = raster();
    let px = PixelSlice::new(&r, W, H, RB, PixelDescriptor::RGB8_SRGB).unwrap();
    let (done, out) = run(Scripted::full(3, 4), Borrowed::new(px), options());
    let done = done.unwrap();
    assert_eq!(out, b"ok");
    assert_eq!(done.trace.passes, 3);
}

#[test]
fn swallowed_interruption_never_publishes_success() {
    for swallow in [Swallow::AsDone, Swallow::AsFilled] {
        let calls = Rc::new(Cell::new(0));
        let mut enc = Scripted::full(2, 3);
        // The second pass's first request fails at the source; the codec lies.
        enc.steps[2] = Step::Fill(1, 0, 3, swallow);
        enc.steps.truncate(3);
        enc.steps.push(Step::Done);
        let finished = enc.finished.clone();
        let src = Callback {
            make: stable,
            calls: calls.clone(),
            fail_at: Some(2),
            replay: Replay::Positional,
        };
        let (r, out) = run(enc, src, options());
        let err = r.unwrap_err();
        assert_eq!(err.error().stage, Stage::Read, "{swallow:?}");
        assert_eq!(
            find_cause::<io::Error>(&err).unwrap().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(!finished.get(), "{swallow:?}: finish never runs");
        assert!(out.is_empty());
    }
}

#[test]
fn filled_without_a_fill_is_a_protocol_error_not_a_spin() {
    let r = raster();
    let px = PixelSlice::new(&r, W, H, RB, PixelDescriptor::RGB8_SRGB).unwrap();
    let enc = Scripted::new(PassCount::Exact(2), vec![Step::FilledEmpty]);
    let (r, out) = run(enc, Borrowed::new(px), options());
    assert_eq!(refusal(&r.unwrap_err()), Some(Refusal::Protocol));
    assert!(out.is_empty());
}

#[test]
fn second_fill_in_one_pull_is_refused_before_reading() {
    let calls = Rc::new(Cell::new(0));
    let enc = Scripted::new(PassCount::Exact(2), vec![Step::FillTwice(0, 0, 3)]);
    let src = Callback {
        make: stable,
        calls: calls.clone(),
        fail_at: None,
        replay: Replay::Positional,
    };
    let (r, out) = run(enc, src, options());
    let err = r.unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::Protocol));
    assert_eq!(
        calls.get(),
        1,
        "the second request never reached the source"
    );
    assert!(out.is_empty());
}

#[test]
fn early_done_short_pass_count_and_gaps_are_refused() {
    let r = raster();
    let cases: Vec<(&str, Scripted, Refusal)> = vec![
        (
            "done before any row",
            Scripted::new(PassCount::AtLeast(2), vec![Step::Done]),
            Refusal::IncompletePass { pass: 0 },
        ),
        (
            "done mid-pass",
            Scripted::new(
                PassCount::AtLeast(1),
                vec![Step::Fill(0, 0, 3, Swallow::No), Step::Done],
            ),
            Refusal::IncompletePass { pass: 0 },
        ),
        (
            "done before declared minimum",
            {
                let mut s = Scripted::full(1, 6);
                s.plan = PassCount::AtLeast(2);
                s
            },
            Refusal::IncompletePass { pass: 0 },
        ),
        (
            "more passes than exact",
            {
                let mut s = Scripted::full(3, 6);
                s.plan = PassCount::Exact(2);
                s
            },
            Refusal::IncompletePass { pass: 2 },
        ),
        (
            "gap within a replay pass",
            Scripted::new(
                PassCount::Exact(2),
                vec![
                    Step::Fill(0, 0, 6, Swallow::No),
                    Step::Fill(1, 0, 2, Swallow::No),
                    Step::Fill(1, 3, 3, Swallow::No),
                ],
            ),
            Refusal::RequestOrder { pass: 1, first: 3 },
        ),
        (
            "next pass before the previous one completed",
            Scripted::new(
                PassCount::Exact(2),
                vec![
                    Step::Fill(0, 0, 3, Swallow::No),
                    Step::Fill(1, 0, 3, Swallow::No),
                ],
            ),
            Refusal::RequestOrder { pass: 1, first: 0 },
        ),
        (
            "skipped pass ordinal",
            Scripted::new(
                PassCount::AtLeast(2),
                vec![
                    Step::Fill(0, 0, 6, Swallow::No),
                    Step::Fill(2, 0, 6, Swallow::No),
                ],
            ),
            Refusal::RequestOrder { pass: 2, first: 0 },
        ),
    ];
    for (name, enc, want) in cases {
        let finished = enc.finished.clone();
        let px = PixelSlice::new(&r, W, H, RB, PixelDescriptor::RGB8_SRGB).unwrap();
        let (res, out) = run(enc, Borrowed::new(px), options());
        let err = res.unwrap_err();
        assert_eq!(refusal(&err), Some(want), "{name}");
        assert!(!finished.get(), "{name}");
        assert!(out.is_empty(), "{name}");
    }
}

#[test]
fn zero_pass_budget_gates_every_route_before_effects() {
    let r = raster();
    let budget = StorageBudget::new(1 << 20, 64);
    for forward in [true, false] {
        let calls = Rc::new(Cell::new(0));
        let opts = Options::new(budget.clone(), u64::MAX).with_replay_policy(ReplayPolicy {
            max_source_passes: Some(0),
            ..Default::default()
        });
        let enc = Scripted::full(2, 3);
        let (res, out) = if forward {
            run(
                enc,
                Callback {
                    make: stable,
                    calls: calls.clone(),
                    fail_at: None,
                    replay: Replay::Forward,
                },
                opts,
            )
        } else {
            let px = PixelSlice::new(&r, W, H, RB, PixelDescriptor::RGB8_SRGB).unwrap();
            run(enc, Borrowed::new(px), opts)
        };
        let err = res.unwrap_err();
        assert_eq!(
            refusal(&err),
            Some(Refusal::PassBudget { pass: 0 }),
            "forward={forward}"
        );
        assert_eq!(err.error().stage, Stage::Policy);
        assert_eq!(calls.get(), 0);
        assert_eq!(budget.reserved_bytes(), 0, "no staging reserved");
        assert!(out.is_empty());
    }
}

#[test]
fn exact_replay_requirement_excludes_digest_evidence() {
    let budget = StorageBudget::new(1 << 20, 64);
    let calls = Rc::new(Cell::new(0));
    let mut seen: Vec<ReplayOffer> = Vec::new();
    let mut out = Vec::new();
    let done = encode_passes(
        Scripted::full(2, 3),
        Input::source(Callback {
            make: stable,
            calls: calls.clone(),
            fail_at: None,
            replay: Replay::Positional,
        }),
        Forward(&mut out),
        PolicySet::new(),
        |_: &ReplayFacts,
         o: &[ReplayOffer]|
         -> Result<ReplayDecision, Box<dyn Error + Send + Sync>> {
            seen = o.to_vec();
            Ok(ReplayDecision::UsePreset)
        },
        Options::new(budget.clone(), u64::MAX).with_replay_policy(ReplayPolicy {
            require_exact_replay: true,
            ..Default::default()
        }),
        &NoPulse,
    )
    .unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].evidence, ReplayEvidence::OwnedStaging);
    assert_eq!(done.route, ReplayRoute::Stage(StagingKind::Memory));
    assert_eq!(calls.get(), 2, "source read once, in two requests");
    // Without memory staging, nothing exact remains: refuse before reads.
    let calls = Rc::new(Cell::new(0));
    let (res, out) = run(
        Scripted::full(2, 3),
        Callback {
            make: stable,
            calls: calls.clone(),
            fail_at: None,
            replay: Replay::Positional,
        },
        Options::new(budget, u64::MAX).with_replay_policy(ReplayPolicy {
            require_exact_replay: true,
            forbid_memory_staging: true,
            ..Default::default()
        }),
    );
    assert_eq!(refusal(&res.unwrap_err()), Some(Refusal::EvidenceRequired));
    assert_eq!(calls.get(), 0);
    assert!(out.is_empty());
}

#[test]
fn offers_disclose_evidence_per_route() {
    let mut seen: Vec<ReplayOffer> = Vec::new();
    let (file, path) = {
        let dir = std::env::var_os("MULTIPASS_SCRATCH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("evidence-{}", std::process::id()));
        (
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&p)
                .unwrap(),
            p,
        )
    };
    let mut out = Vec::new();
    encode_passes(
        Scripted::full(2, 3),
        Input::source(Callback {
            make: stable,
            calls: Rc::new(Cell::new(0)),
            fail_at: None,
            replay: Replay::Positional,
        }),
        Forward(&mut out),
        PolicySet::new(),
        |_: &ReplayFacts,
         o: &[ReplayOffer]|
         -> Result<ReplayDecision, Box<dyn Error + Send + Sync>> {
            seen = o.to_vec();
            Ok(ReplayDecision::UsePreset)
        },
        options().with_stage_file(file, u64::MAX),
        &NoPulse,
    )
    .unwrap();
    std::fs::remove_file(path).unwrap();
    let ev: Vec<_> = seen.iter().map(|o| (o.route, o.evidence)).collect();
    assert_eq!(
        ev,
        [
            (ReplayRoute::Source, ReplayEvidence::Digest),
            (
                ReplayRoute::Stage(StagingKind::Disk),
                ReplayEvidence::Digest
            ),
            (
                ReplayRoute::Stage(StagingKind::Memory),
                ReplayEvidence::OwnedStaging
            ),
        ]
    );
}

#[test]
fn callback_source_that_changes_between_passes_is_refused() {
    // Content depends on the call count: pass 0 sees row values, pass 1 sees
    // them shifted from row 4 on. Digests catch it at that row, before the
    // encoder receives it and before finish.
    let calls = Rc::new(Cell::new(0));
    let enc = Scripted::full(2, 2);
    let finished = enc.finished.clone();
    let seen = enc.seen.clone();
    let src = Callback {
        make: |call: u32, row: u32| {
            if call >= 3 && row >= 4 {
                row as u8 + 1
            } else {
                row as u8
            }
        },
        calls: calls.clone(),
        fail_at: None,
        replay: Replay::Positional,
    };
    let (r, out) = run(enc, src, options());
    let err = r.unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::SourceChanged { pass: 1 }));
    assert_eq!(err.error().stage, Stage::Read);
    assert_eq!(
        seen.get(),
        6 + 4,
        "rows 4.. of pass 1 never reached the encoder"
    );
    assert!(!finished.get());
    assert!(out.is_empty());
}

#[test]
fn forward_staged_replay_is_immune_to_later_source_behavior() {
    // A forward callback that would change on any second read: staging means
    // there is no second read.
    let calls = Rc::new(Cell::new(0));
    let enc = Scripted::full(3, 2);
    let src = Callback {
        make: |call: u32, row: u32| (row as u8).wrapping_add(call as u8 * 50 * u8::from(call >= 3)),
        calls: calls.clone(),
        fail_at: None,
        replay: Replay::Forward,
    };
    let (r, out) = run(enc, src, options());
    let done = r.unwrap();
    assert_eq!(done.route, ReplayRoute::Stage(StagingKind::Memory));
    assert_eq!(calls.get(), 3, "one source pass of three requests");
    assert_eq!(out, b"ok");
}

#[test]
fn reader_rows_refuse_rewind_even_if_misused() {
    // A forward reader cannot replay; the driver never asks it to.
    let r = raster();
    let (res, _) = run(
        Scripted::full(2, 3),
        ReaderRows::new(Cursor::new(r), RB, desc()),
        options(),
    );
    assert_eq!(res.unwrap().trace.source_bytes, (RB * H as usize) as u64);
}
