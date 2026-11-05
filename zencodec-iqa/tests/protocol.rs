//! Cross-crate controls for Pulse ownership and located error propagation.
use how_far::{Child, ChildPulse, Execution, Outcome, PhaseSpec, PlanError, Report, Stop, Total};
use how_far::{Pulse, SharedPulse, StopReason};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use whereat::{At, ErrorAtExt};
use zencodec_iqa::iqa::*;
use zencodec_iqa::iqa::{how_far, whereat, zenpixels};
use zenpixels::{PixelDescriptor, PixelSlice};

#[derive(Default)]
struct Log {
    outcomes: Mutex<BTreeMap<String, Outcome>>,
    reports: Mutex<BTreeMap<String, u64>>,
    totals: Mutex<BTreeMap<String, Total>>,
    issues: Mutex<Vec<PlanError>>,
    stopped: AtomicBool,
    reject_plan: AtomicBool,
}
struct Recorder {
    path: String,
    log: Arc<Log>,
    owns_completion: bool,
    finished: bool,
}
impl Recorder {
    fn root(log: &Arc<Log>) -> Self {
        Self {
            path: "caller".into(),
            log: log.clone(),
            owns_completion: false,
            finished: false,
        }
    }
}
impl Stop for Recorder {
    fn check(&self) -> core::result::Result<(), StopReason> {
        if self.log.stopped.load(Ordering::Acquire) {
            Err(StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}
impl Report for Recorder {
    fn advance(&self, completed: u64) {
        *self
            .log
            .reports
            .lock()
            .unwrap()
            .entry(self.path.clone())
            .or_default() += completed;
    }
}
impl Pulse for Recorder {
    fn record_issue(&self, issue: PlanError) {
        self.log.issues.lock().unwrap().push(issue);
    }
    fn split(
        &self,
        _: Execution,
        parts: &[PhaseSpec<'_>],
    ) -> core::result::Result<Vec<Child<'_>>, PlanError> {
        PhaseSpec::validate_split(parts)?;
        if self.log.reject_plan.load(Ordering::Relaxed) {
            return Err(PlanError::Unsupported);
        }
        Ok(parts
            .iter()
            .map(|part| {
                Child::new(Self {
                    path: format!("{}/{}", self.path, part.name),
                    log: self.log.clone(),
                    owns_completion: true,
                    finished: false,
                })
            })
            .collect())
    }
    fn set_total(&self, total: Total) -> core::result::Result<(), PlanError> {
        self.log
            .totals
            .lock()
            .unwrap()
            .insert(self.path.clone(), total);
        Ok(())
    }
    fn share(&self) -> core::result::Result<SharedPulse, PlanError> {
        Ok(SharedPulse::new(Self {
            path: self.path.clone(),
            log: self.log.clone(),
            owns_completion: false,
            finished: false,
        }))
    }
}
impl ChildPulse for Recorder {
    fn finish(mut self: Box<Self>, outcome: Outcome) -> core::result::Result<(), PlanError> {
        assert!(
            self.owns_completion,
            "a shared view cannot acquire completion rights"
        );
        assert!(
            self.log
                .outcomes
                .lock()
                .unwrap()
                .insert(self.path.clone(), outcome)
                .is_none(),
            "complete a child exactly once"
        );
        self.finished = true;
        Ok(())
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        if self.owns_completion && !self.finished {
            self.log
                .outcomes
                .lock()
                .unwrap()
                .insert(self.path.clone(), Outcome::Abandoned);
        }
    }
}

fn pixels() -> PixelSlice<'static> {
    PixelSlice::new(&[0; 12], 2, 2, 6, PixelDescriptor::RGB8_SRGB).unwrap()
}
#[derive(Debug)]
struct Native(Box<u32>);
impl std::fmt::Display for Native {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native {}", self.0)
    }
}
impl std::error::Error for Native {}
struct Probe {
    calls: usize,
}
impl Metric for Probe {
    fn id(&self) -> MetricId {
        MetricId::new("probe", Implementation::new("adapter-control", "1", 1))
    }
    fn outputs(&self) -> std::borrow::Cow<'static, [ScoreSpec]> {
        std::borrow::Cow::Owned(vec![ScoreSpec::new("score", ScoreKind::Feature)])
    }
    fn accepts(&self, f: &InputFormat) -> bool {
        is_rgb8_srgb(f)
    }
    fn evaluate(
        &mut self,
        r: Image<'_>,
        d: Image<'_>,
        _: EvalOpts<'_>,
    ) -> MetricResult<Box<dyn Evaluation>> {
        self.calls += 1;
        let (r, d) = rgb8_pair(&r, &d)?;
        assert_eq!((r.width, r.height), (d.width, d.height));
        Ok(Box::new(ScalarEval::single("score", 1.0)))
    }
}
#[test]
fn mapping_refusal_keeps_native_location_payload_and_classification() {
    use zencodec::{
        collection::Composition,
        frame::MapError,
        image::{
            ImageGeometry, ImageInterpretation, ImageOutput, OutputError, PackedImageOwner, Rect,
        },
    };
    use zenpixels::Orientation;
    struct Owner(AtomicBool, bool);
    impl PackedImageOwner for Owner {
        fn pixels(&self) -> core::result::Result<PixelSlice<'_>, At<OutputError>> {
            if self.0.swap(true, Ordering::AcqRel) {
                let cause: Box<dyn std::error::Error + Send + Sync> = if self.1 {
                    Box::new(
                        zencodec::CodecError::new(
                            None,
                            zencodec::ErrorCategory::Stopped(StopReason::TimedOut),
                        )
                        .start_at(),
                    )
                } else {
                    Box::new(Native(Box::new(19)))
                };
                return Err(OutputError::Mapping(MapError::Backend(cause))
                    .start_at()
                    .at_str("mapping origin"));
            }
            Ok(pixels())
        }
    }
    for stopped in [false, true] {
        let mut interpretation = ImageInterpretation::packed(
            ImageGeometry {
                canvas_width: 2,
                canvas_height: 2,
                region: Rect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                },
            },
            Orientation::Identity,
        );
        interpretation.composition = Some(Composition::Composited);
        let output = ImageOutput::from_packed_owner(
            Owner(AtomicBool::new(false), stopped),
            vec![],
            interpretation,
        )
        .unwrap();
        let log = Arc::new(Log::default());
        let mut metric = Probe { calls: 0 };
        let pulse = Recorder::root(&log);
        let error = zencodec_iqa::score_outputs(
            &mut metric,
            &output,
            &output,
            EvalOpts::new().with_pulse(&pulse),
        )
        .unwrap_err();
        assert_eq!(
            error.error().kind(),
            if stopped {
                ErrorKind::Stopped(StopReason::TimedOut)
            } else {
                ErrorKind::Backend
            }
        );
        assert_eq!(stop_reason(&error), stopped.then_some(StopReason::TimedOut));
        assert!(
            error
                .contexts()
                .any(|context| context.as_text() == Some("mapping origin"))
        );
        assert!(
            std::error::Error::source(error.error())
                .unwrap()
                .is::<OutputError>()
        );
        assert_eq!(
            *log.outcomes.lock().unwrap(),
            BTreeMap::from([(
                "caller/score decoded outputs".into(),
                if stopped {
                    Outcome::Cancelled
                } else {
                    Outcome::Failed
                }
            ),])
        );
    }
}
