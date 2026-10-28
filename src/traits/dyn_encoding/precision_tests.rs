//! Source-only draft: the existing shims must retain evidence for each object form.
use super::*;
use crate::UnsupportedOperation;
use crate::encode::{EncodeDeclaration, EncodePlan, OutputTiming};
use zenpixels::{ChannelType, SignalRange, sample::ExactDepth};

struct Job(ExactDepth);
impl EncodeJob for Job {
    type Error = UnsupportedOperation;
    type Enc = Declared;
    type AnimationFrameEnc = ();
    fn with_pulse(self, _: how_far::SharedPulse) -> Self {
        self
    }
    fn with_limits(self, _: ResourceLimits) -> Self {
        self
    }
    fn with_metadata(self, _: Metadata) -> Self {
        self
    }
    fn plan_encode(&self, declaration: &EncodeDeclaration<'_>) -> Result<EncodePlan, BoxedError> {
        assert_eq!(declaration.exact_depth, Some(self.0));
        Ok(EncodePlan::rows(OutputTiming::AT_FINISH))
    }
    fn encoder(self) -> Result<Declared, Self::Error> {
        Ok(Declared(self.0))
    }
    fn animation_frame_encoder(self) -> Result<(), Self::Error> {
        panic!("no fallback")
    }
}
struct Declared(ExactDepth);
impl Encoder for Declared {
    type Error = UnsupportedOperation;
    fn reject(error: Self::Error) -> Self::Error {
        error
    }
    fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
        panic!("no fallback")
    }
    fn begin_declared(
        &mut self,
        declaration: &EncodeDeclaration<'_>,
    ) -> Result<EncodePlan, Self::Error> {
        assert_eq!(declaration.exact_depth, Some(self.0));
        Ok(EncodePlan::rows(OutputTiming::AT_FINISH))
    }
}

#[test]
fn evidence_survives_plain_send_and_send_sync_erased_jobs_and_encoders() {
    let exact = ExactDepth::new(ChannelType::U16, SignalRange::Full, 8, 16).unwrap();
    let declaration = EncodeDeclaration::new(2, 3, PixelDescriptor::RGB16).with_exact_depth(exact);
    let jobs: [Box<dyn DynEncodeJob>; 3] = [
        Box::new(EncodeJobShim(Some(Job(exact)))),
        {
            let job: Box<dyn DynEncodeJob + Send> = Box::new(EncodeJobShim(Some(Job(exact))));
            job
        },
        {
            let job: Box<dyn DynEncodeJob + Send + Sync> =
                Box::new(EncodeJobShim(Some(Job(exact))));
            job
        },
    ];
    for job in jobs {
        let plan = job.plan_encode(&declaration).unwrap();
        plan.validate(&declaration).unwrap();
        let mut encoder = job.into_encoder().unwrap();
        assert_eq!(plan, encoder.begin_declared(&declaration).unwrap());
    }
    // Direct DynEncoder objects have Send as a supertrait; exercise explicit
    // auto-trait spellings as well as the bare object produced by the job.
    let encoders: [Box<dyn DynEncoder>; 3] = [
        Box::new(EncoderShim(Declared(exact))),
        {
            let encoder: Box<dyn DynEncoder + Send> = Box::new(EncoderShim(Declared(exact)));
            encoder
        },
        {
            let encoder: Box<dyn DynEncoder + Send + Sync> = Box::new(EncoderShim(Declared(exact)));
            encoder
        },
    ];
    for mut encoder in encoders {
        assert_eq!(
            encoder.begin_declared(&declaration).unwrap(),
            EncodePlan::rows(OutputTiming::AT_FINISH)
        );
    }
}
