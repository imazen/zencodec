//! Independent regression for the multipass prototype at 0ec4726.
//! Copy into that prototype's tests/ directory; it must refuse a detached proof.
use multipass_validation::{
    Input, MultiPassEncoder, Options, PassCount, PassOutput, PassPlan, PassProgress, PassRows,
    PassSource, Preset, Replay, ReplayPolicy, RowRequest, encode_passes, sources::Borrowed,
};
use std::{cell::RefCell, io, rc::Rc};
use zencodec::how_far::NoPulse;
use zencodec::{
    encode::{EncodeOutput, OutputTiming},
    operation::PolicySet,
    output_patch::{OutputAccess, OutputFinish},
    storage::StorageBudget,
};
use zencodec_io::{destination::Forward, row_encode::Description};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

struct Changing {
    proof: Replay,
    calls: u8,
}
impl PassSource for Changing {
    type Error = io::Error;
    fn description(&self) -> Description {
        Description::new(1, 1, PixelDescriptor::RGB8_SRGB)
    }
    fn replay(&self) -> Replay {
        self.proof
    }
    fn read_rows(
        &mut self,
        _: u32,
        mut rows: PixelSliceMut<'_>,
        _: &dyn zencodec::Pulse,
    ) -> io::Result<()> {
        rows.row_mut(0).fill(self.calls);
        self.calls += 1;
        Ok(())
    }
}
struct Twice {
    pass: u32,
    samples: Rc<RefCell<Vec<u8>>>,
}
impl MultiPassEncoder for Twice {
    type Error = io::Error;
    fn begin_passes(&mut self, _: &Description, _: OutputAccess) -> io::Result<PassPlan> {
        Ok(PassPlan {
            passes: PassCount::Exact(2),
            output: PassOutput::AnyPass,
            timing: OutputTiming::AT_FINISH,
        })
    }
    fn pull_rows(
        &mut self,
        rows: &mut dyn PassRows,
        _: &dyn zencodec::Pulse,
    ) -> io::Result<PassProgress> {
        if self.pass == 2 {
            return Ok(PassProgress::Done);
        }
        let mut pixels = [0; 3];
        rows.fill(RowRequest {
            pass: self.pass,
            final_pass: false,
            label: 0,
            first: 0,
            rows: PixelSliceMut::new(&mut pixels, 1, 1, 3, PixelDescriptor::RGB8_SRGB).unwrap(),
        })
        .map_err(|_| io::Error::other("source interrupted"))?;
        self.samples.borrow_mut().push(pixels[0]);
        self.pass += 1;
        Ok(PassProgress::Filled)
    }
    fn pending_output(&self) -> &[u8] {
        &[]
    }
    fn consume_output(&mut self, _: usize) {}
    fn finish_output(self) -> io::Result<OutputFinish<EncodeOutput>> {
        Ok(OutputFinish::new(
            EncodeOutput::new(vec![1], zencodec::ImageFormat::Unknown),
            vec![],
        ))
    }
}
#[test]
fn detached_borrow_proof_cannot_authorize_a_changing_source() {
    let proof = {
        let pixels = [7u8; 3];
        Borrowed::new(PixelSlice::new(&pixels, 1, 1, 3, PixelDescriptor::RGB8_SRGB).unwrap())
            .replay()
    }; // Original pixels and borrow are gone; only the token survives.
    let samples = Rc::new(RefCell::new(Vec::new()));
    let mut output = Vec::new();
    let result = encode_passes(
        Twice {
            pass: 0,
            samples: samples.clone(),
        },
        Input::source(Changing { proof, calls: 0 }),
        Forward(&mut output),
        PolicySet::new(),
        Preset,
        Options::new(StorageBudget::new(4096, 8), 4096).with_replay_policy(ReplayPolicy {
            require_exact_replay: true,
            forbid_memory_staging: true,
            ..Default::default()
        }),
        &NoPulse,
    );
    assert!(
        result.is_err(),
        "detached proof accepted different pass samples {:?} and published {:?}",
        samples.borrow(),
        output
    );
    assert!(output.is_empty());
}

#[test]
fn driver_retained_raster_is_exact_without_digest_or_raw_staging() {
    let pixels = [7u8, 8, 9, 222, 222, 222, 222, 222, 222];
    let samples = Rc::new(RefCell::new(Vec::new()));
    let mut output = Vec::new();
    let budget = StorageBudget::new(0, 0);
    let done = encode_passes(
        Twice {
            pass: 0,
            samples: samples.clone(),
        },
        Input::raster(PixelSlice::new(&pixels, 1, 1, 9, PixelDescriptor::RGB8_SRGB).unwrap()),
        Forward(&mut output),
        PolicySet::new(),
        Preset,
        Options::new(budget.clone(), 4096).with_replay_policy(ReplayPolicy {
            require_exact_replay: true,
            forbid_memory_staging: true,
            ..Default::default()
        }),
        &NoPulse,
    )
    .unwrap();
    assert_eq!(&*samples.borrow(), &[7, 7]);
    assert_eq!(output, [1]);
    assert_eq!(done.trace.digest_bytes, 0);
    assert_eq!(done.trace.staging_admitted, 0);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}

#[test]
fn staging_a_retained_raster_is_still_a_policy_decision() {
    use multipass_validation::{ReplayDecision, ReplayFacts, ReplayOffer, ReplayRoute};
    use std::error::Error;
    use zencodec::operation::StagingKind;
    let pixels = [7u8; 3];
    let samples = Rc::new(RefCell::new(Vec::new()));
    let budget = StorageBudget::new(4096, 8);
    let mut asked = false;
    let mut output = Vec::new();
    let result = encode_passes(
        Twice {
            pass: 0,
            samples: samples.clone(),
        },
        Input::raster(PixelSlice::new(&pixels, 1, 1, 3, PixelDescriptor::RGB8_SRGB).unwrap()),
        Forward(&mut output),
        PolicySet::new(),
        |_: &ReplayFacts,
         offers: &[ReplayOffer]|
         -> Result<ReplayDecision, Box<dyn Error + Send + Sync>> {
            asked = true;
            assert_eq!(offers.len(), 1);
            assert_eq!(offers[0].route, ReplayRoute::Stage(StagingKind::Memory));
            assert_eq!(budget.reserved_bytes(), 3);
            Ok(ReplayDecision::Decline)
        },
        Options::new(budget.clone(), 4096).with_replay_policy(ReplayPolicy {
            max_source_passes: Some(1),
            require_exact_replay: true,
            ..Default::default()
        }),
        &NoPulse,
    );
    assert!(asked, "staging cost must not bypass situational policy");
    assert!(result.is_err());
    assert!(samples.borrow().is_empty());
    assert!(output.is_empty());
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}
