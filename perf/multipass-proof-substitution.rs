//! Independent regression for the multipass prototype at 0ec4726.
//! Copy into that prototype's tests/ directory; it must refuse a detached proof.
use enough::{Stop, };
use zencodec::how_far::NoPulse;
use multipass_validation::{
    MultiPassEncoder, Options, PassCount, PassPlan, PassProgress, PassRows, PassSource, Preset,
    Replay, ReplayPolicy, RowRequest, encode_passes, sources::Borrowed,
};
use std::{cell::RefCell, io, rc::Rc};
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
    fn read_rows(&mut self, _: u32, mut rows: PixelSliceMut<'_>, _: &dyn zencodec::Pulse) -> io::Result<()> {
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
            timing: OutputTiming::AT_FINISH,
        })
    }
    fn pull_rows(&mut self, rows: &mut dyn PassRows, _: &dyn zencodec::Pulse) -> io::Result<PassProgress> {
        if self.pass == 2 {
            return Ok(PassProgress::Done);
        }
        let mut pixels = [0; 3];
        rows.fill(RowRequest {
            pass: self.pass,
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
        Changing { proof, calls: 0 },
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
