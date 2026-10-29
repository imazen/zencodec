//! Witnesses of the current multi-pass prototype (`perf/multipass-validation`,
//! unpatched). Each test passes by observing a contract gap; the candidate
//! patch in `candidate/` closes it. Read counts increment on every read attempt.
use multipass_validation::{
    Completed, Failure, Input, MultiPassEncoder, Options, PassCount, PassPlan, PassProgress,
    PassRows, PassSource, Preset, Refusal, Replay, ReplayPolicy, RowRequest, Stage, encode_passes,
};
use std::{cell::Cell, error::Error, fmt, io, rc::Rc};
use zencodec::how_far::NoPulse;
use zencodec::{
    At,
    encode::{EncodeOutput, OutputTiming},
    find_cause,
    operation::{OutputSchedule, PolicySet},
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

/// Requests `passes` full passes of 4-row bands. After each filled request in
/// a pass listed in `emit` it makes two bytes pending.
struct Script {
    declared: PassCount,
    timing: OutputTiming,
    passes: u32,
    emit: Vec<u32>,
    pass: u32,
    next: u32,
    buf: Vec<u8>,
    pending: Vec<u8>,
    taken: usize,
}
impl Script {
    fn new(declared: PassCount, passes: u32, emit: &[u32]) -> Self {
        Self {
            declared,
            timing: if emit.is_empty() {
                OutputTiming::AT_FINISH
            } else {
                OutputTiming::streaming(0)
            },
            passes,
            emit: emit.to_vec(),
            pass: 0,
            next: 0,
            buf: vec![0; RB * H as usize],
            pending: Vec::new(),
            taken: 0,
        }
    }
}
impl MultiPassEncoder for Script {
    type Error = MockError;
    fn begin_passes(&mut self, _: &Description, _: OutputAccess) -> Result<PassPlan, MockError> {
        Ok(PassPlan {
            passes: self.declared,
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
        let n = 4.min(H - self.next);
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
        Ok(OutputFinish::new(
            EncodeOutput::new(
                self.pending[self.taken..].to_vec(),
                zencodec::ImageFormat::Unknown,
            ),
            Vec::new(),
        ))
    }
}

/// A positional source whose counter increments on every read attempt.
struct Counting(Rc<Cell<u32>>);
impl PassSource for Counting {
    type Error = io::Error;
    fn description(&self) -> Description {
        Description::new(W, H, PixelDescriptor::RGB8_SRGB)
    }
    fn replay(&self) -> Replay {
        Replay::Positional
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        _: Option<&dyn zencodec::Pulse>,
    ) -> io::Result<()> {
        self.0.set(self.0.get() + rows.rows());
        for y in 0..rows.rows() {
            rows.row_mut(y).fill((first + y) as u8);
        }
        Ok(())
    }
}

fn run(enc: Script, policy: ReplayPolicy) -> (Result<Completed, At<Failure>>, Vec<u8>, u32) {
    let schedule = if enc.timing.streams() {
        PolicySet::new().require_output_schedule(OutputSchedule::Live)
    } else {
        PolicySet::new()
    };
    let rows = Rc::new(Cell::new(0));
    let mut out = Vec::new();
    let r = encode_passes(
        enc,
        Input::source(Counting(rows.clone())),
        Forward(&mut out),
        schedule,
        Preset,
        Options::new(StorageBudget::new(1 << 20, 64), u64::MAX)
            .with_replay_policy(policy)
            .with_output_policy(schedule),
        Some(&NoPulse),
    );
    (r, out, rows.get())
}

fn refusal(e: &At<Failure>) -> Option<Refusal> {
    find_cause::<Refusal>(e).copied()
}

fn budget(max: u32) -> ReplayPolicy {
    ReplayPolicy {
        max_source_passes: Some(max),
        forbid_memory_staging: true,
        ..Default::default()
    }
}

#[test]
fn current_budget_admits_an_open_count_by_its_minimum_then_refuses_after_reading() {
    let (r, out, rows) = run(Script::new(PassCount::AtLeast(2), 3, &[]), budget(2));
    let err = r.unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::PassBudget { pass: 2 }));
    assert_eq!(err.error().stage, Stage::Policy);
    assert_eq!(
        rows,
        2 * H,
        "two complete source passes read before the refusal"
    );
    assert!(out.is_empty());
}

#[test]
fn current_exact_overrun_reads_the_extra_pass_before_refusing() {
    let (r, _, rows) = run(
        Script::new(PassCount::Exact(2), 3, &[]),
        ReplayPolicy::default(),
    );
    assert_eq!(
        refusal(&r.unwrap_err()),
        Some(Refusal::IncompletePass { pass: 2 })
    );
    assert_eq!(rows, 3 * H, "the undeclared third pass was read");
}

#[test]
fn current_live_bytes_from_an_analysis_pass_are_published_before_a_budget_refusal() {
    let (r, out, rows) = run(Script::new(PassCount::AtLeast(2), 3, &[0, 1]), budget(2));
    let err = r.unwrap_err();
    assert_eq!(refusal(&err), Some(Refusal::PassBudget { pass: 2 }));
    assert_eq!(rows, 2 * H);
    assert!(!out.is_empty(), "bytes from passes 0 and 1 were published");
    assert_eq!(err.error().emitted, out.len() as u64);
}
