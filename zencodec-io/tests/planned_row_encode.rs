#![cfg(feature = "std")]
//! Temporal admission controls for the shared job-to-destination adapter.
//! The scripted encoder exposes construction/start/read/write boundaries;
//! native codec qualification lives in the pinned integration harnesses.
use std::{
    io,
    sync::{Arc, Mutex},
};
use zencodec::{
    At, Cicp, ImageFormat, Metadata, ResourceLimits, SharedPulse, UnsupportedOperation,
    encode::{
        BoxedError, DynEncoderConfig, EncodeDeclaration, EncodeJob, EncodeOutput, EncodePlan,
        EncodePlanError, Encoder, EncoderConfig, OutputTiming,
    },
    operation::{BoundPolicy, Decision, OperationPolicy, PolicyResolver, PolicySet, Question},
    storage::StorageBudget,
};
use zencodec_io::row_encode::{self, Description, Options, Source, Stage};
use zenpixels::{ColorContext, PixelDescriptor, PixelSlice, PixelSliceMut};

type Events = Arc<Mutex<Vec<&'static str>>>;
fn event(events: &Events, name: &'static str) {
    events.lock().unwrap().push(name);
}

#[derive(Clone)]
struct Job {
    events: Events,
    plan: EncodePlan,
    drift: bool,
    fail_start: bool,
    patches: bool,
}
impl EncodeJob for Job {
    type Error = io::Error;
    type Enc = Enc;
    type AnimationFrameEnc = ();
    fn with_pulse(self, _: SharedPulse) -> Self {
        self
    }
    fn with_limits(self, _: ResourceLimits) -> Self {
        self
    }
    fn with_metadata(self, _: Metadata) -> Self {
        self
    }
    fn plan_encode(&self, declaration: &EncodeDeclaration<'_>) -> Result<EncodePlan, BoxedError> {
        event(&self.events, "plan");
        assert_eq!(
            declaration.color,
            Some(&ColorContext::from_cicp(Cicp::DISPLAY_P3))
        );
        Ok(self.plan.clone())
    }
    fn encoder(self) -> Result<Enc, Self::Error> {
        event(&self.events, "construct");
        Ok(Enc {
            events: self.events,
            plan: self.plan,
            drift: self.drift,
            fail_start: self.fail_start,
            patches: self.patches,
            pending: vec![],
        })
    }
    fn animation_frame_encoder(self) -> Result<(), Self::Error> {
        Err(io::Error::other("no animation"))
    }
}
#[derive(Clone)]
struct Config(Job);
impl EncoderConfig for Config {
    type Error = io::Error;
    type Job = Job;
    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB]
    }
    fn job(self) -> Job {
        self.0
    }
}

fn run_job(
    erased: bool,
    job: Job,
    rows: Rows,
    writer: Writer,
    resolver: impl OperationPolicy,
    options: Options,
    pulse: &dyn zencodec::Pulse,
) -> Result<row_encode::Completed, At<row_encode::Failure>> {
    if erased {
        row_encode::encode_dyn_job(
            Config(job).dyn_job(),
            rows,
            writer,
            resolver,
            options,
            pulse,
        )
    } else {
        row_encode::encode_job(job, rows, writer, resolver, options, pulse)
    }
}
struct Enc {
    events: Events,
    plan: EncodePlan,
    drift: bool,
    fail_start: bool,
    patches: bool,
    pending: Vec<u8>,
}
impl Encoder for Enc {
    type Error = io::Error;
    fn reject(op: UnsupportedOperation) -> Self::Error {
        io::Error::other(op)
    }
    fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
        panic!("no fallback")
    }
    fn begin_output(
        &mut self,
        _: u32,
        _: u32,
        _: PixelDescriptor,
        _: zencodec::output_patch::OutputAccess,
    ) -> Result<OutputTiming, Self::Error> {
        panic!("must declare color before headers")
    }
    fn begin_declared(
        &mut self,
        declaration: &EncodeDeclaration<'_>,
    ) -> Result<EncodePlan, Self::Error> {
        event(&self.events, "start");
        assert_eq!(
            declaration.color,
            Some(&ColorContext::from_cicp(Cicp::DISPLAY_P3))
        );
        if self.fail_start {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "native-start-control",
            ));
        }
        self.pending.push(80);
        if self.drift {
            return Ok(EncodePlan::rows(OutputTiming::AT_FINISH));
        }
        Ok(self.plan.clone())
    }
    fn pending_output(&self) -> &[u8] {
        &self.pending
    }
    fn consume_output(&mut self, n: usize) {
        self.pending.drain(..n);
    }
    fn push_rows(&mut self, pixels: PixelSlice<'_>) -> Result<(), Self::Error> {
        event(&self.events, "push");
        assert_eq!(
            pixels.color_context().map(Arc::as_ref),
            Some(&ColorContext::from_cicp(Cicp::DISPLAY_P3))
        );
        for row in 0..pixels.rows() {
            self.pending.extend_from_slice(pixels.row(row));
        }
        Ok(())
    }
    fn finish_output(
        self,
    ) -> Result<zencodec::output_patch::OutputFinish<EncodeOutput>, Self::Error> {
        let patches = if self.patches {
            vec![zencodec::output_patch::Patch {
                offset: 0,
                bytes: vec![81],
            }]
        } else {
            vec![]
        };
        Ok(zencodec::output_patch::OutputFinish::new(
            self.finish()?,
            patches,
        ))
    }
    fn finish(mut self) -> Result<EncodeOutput, Self::Error> {
        event(&self.events, "finish");
        self.pending.push(90);
        Ok(EncodeOutput::new(self.pending, ImageFormat::Unknown))
    }
}
struct Rows {
    events: Events,
    description: Arc<Mutex<Description>>,
}
impl Source for Rows {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.description.lock().unwrap().clone()
    }
    fn read_rows(
        &mut self,
        _: u32,
        mut pixels: PixelSliceMut<'_>,
        _: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error> {
        event(&self.events, "read");
        for row in 0..pixels.rows() {
            pixels.row_mut(row).fill(7);
        }
        Ok(())
    }
}
struct Writer {
    events: Events,
    bytes: Arc<Mutex<Vec<u8>>>,
}
impl io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        event(&self.events, "write");
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Resolver<F>(F);
impl<F: FnMut()> PolicyResolver for Resolver<F> {
    type Error = std::convert::Infallible;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<Self::Error>> {
        (self.0)();
        let Question::OutputSchedule { choices, .. } = question else {
            panic!("unexpected question")
        };
        Ok(Decision::Choose(choices.first().unwrap().id()))
    }
}
struct Setup {
    events: Events,
    bytes: Arc<Mutex<Vec<u8>>>,
    description: Arc<Mutex<Description>>,
}
impl Setup {
    fn new() -> Self {
        Self {
            events: Arc::default(),
            bytes: Arc::default(),
            description: Arc::new(Mutex::new(
                Description::new(2, 2, PixelDescriptor::RGB8_SRGB)
                    .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::DISPLAY_P3))),
            )),
        }
    }
    fn job(&self) -> Job {
        Job {
            events: self.events.clone(),
            plan: EncodePlan::rows(OutputTiming::streaming(0)),
            drift: false,
            fail_start: false,
            patches: false,
        }
    }
    fn rows(&self) -> Rows {
        Rows {
            events: self.events.clone(),
            description: self.description.clone(),
        }
    }
    fn writer(&self) -> Writer {
        Writer {
            events: self.events.clone(),
            bytes: self.bytes.clone(),
        }
    }
    fn options(&self) -> Options {
        Options::new(StorageBudget::new(4096, 16), 4096)
    }
}

#[test]
fn policy_precedes_construction_and_finish_precedes_destination_waits() {
    for erased in [false, true] {
        let s = Setup::new();
        let storage = StorageBudget::new(4096, 16);
        let completed = run_job(
            erased,
            s.job(),
            s.rows(),
            s.writer(),
            BoundPolicy::new(
                PolicySet::new(),
                Resolver(|| {
                    assert_eq!(storage.reserved_bytes(), 0);
                    assert_eq!(*s.events.lock().unwrap(), ["plan"]);
                    event(&s.events, "policy");
                }),
            ),
            Options::new(storage.clone(), 4096),
            &zencodec::NoPulse,
        )
        .unwrap();
        assert_eq!(
            &s.events.lock().unwrap()[..6],
            ["plan", "policy", "construct", "start", "read", "push"]
        );
        let events = s.events.lock().unwrap();
        assert!(
            events.iter().position(|e| *e == "finish").unwrap()
                < events.iter().position(|e| *e == "write").unwrap()
        );
        assert_eq!(
            *s.bytes.lock().unwrap(),
            [80, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 90]
        );
        assert_eq!(completed.emitted, 14);
        assert_eq!(storage.reserved_bytes(), 0);
    }
}

#[test]
#[allow(deprecated)] // Preserve coverage of legacy and bound requirement conjunction.
fn strict_unknown_workspace_refuses_before_planning_or_any_effect() {
    use zencodec::operation::{PolicyRequirement, RequirementEvidence, UnmetRequirements};
    for erased in [false, true] {
        for (legacy, bound) in [
            (None, Some(0)),
            (Some(0), None),
            (Some(u64::MAX), Some(0)),
            (Some(0), Some(u64::MAX)),
        ] {
            let s = Setup::new();
            let storage = StorageBudget::new(4096, 16);
            let mut options = Options::new(storage.clone(), 4096);
            if let Some(limit) = legacy {
                options = options.with_policy_set(PolicySet::new().with_workspace_limit(limit));
            }
            let requirements = bound.map_or(PolicySet::new(), |limit| {
                PolicySet::new().with_workspace_limit(limit)
            });
            let error = run_job(
                erased,
                s.job(),
                s.rows(),
                s.writer(),
                BoundPolicy::new(
                    requirements,
                    Resolver(|| panic!("inadmissible route reached callback")),
                ),
                options,
                &zencodec::NoPulse,
            )
            .unwrap_err();
            assert_eq!(error.error().stage, Stage::Policy);
            assert!(
                s.events.lock().unwrap().is_empty(),
                "no planning/construction/read/publication before admission"
            );
            assert_eq!(storage.reserved_bytes(), 0);
            assert!(s.bytes.lock().unwrap().is_empty());
            let cause =
                zencodec::find_cause::<UnmetRequirements>(&error).expect("typed admission cause");
            assert_eq!(
                cause.iter().collect::<Vec<_>>(),
                [(PolicyRequirement::Workspace, RequirementEvidence::Unknown)]
            );
        }
    }
}

#[test]
fn source_changed_by_policy_refuses_before_native_startup() {
    for erased in [false, true] {
        let s = Setup::new();
        let error = run_job(
            erased,
            s.job(),
            s.rows(),
            s.writer(),
            BoundPolicy::new(
                PolicySet::new(),
                Resolver(|| {
                    s.description.lock().unwrap().width = 3;
                }),
            ),
            s.options(),
            &zencodec::NoPulse,
        )
        .unwrap_err();
        assert_eq!(error.error().stage, Stage::Read);
        assert_eq!(
            zencodec::find_cause::<row_encode::Refusal>(&error),
            Some(&row_encode::Refusal::SourceChanged)
        );
        assert_eq!(*s.events.lock().unwrap(), ["plan"]);
    }
}

#[test]
fn unsupported_input_and_storage_refuse_before_construction() {
    for erased in [false, true] {
        for raster in [true, false] {
            let s = Setup::new();
            let mut job = s.job();
            if raster {
                job.plan = EncodePlan::whole_raster();
            }
            let error = run_job(
                erased,
                job,
                s.rows(),
                s.writer(),
                PolicySet::new(),
                Options::new(StorageBudget::new(0, 0), 4096),
                &zencodec::NoPulse,
            )
            .unwrap_err();
            assert_eq!(error.error().stage, Stage::Prepare);
            assert_eq!(*s.events.lock().unwrap(), ["plan"]);
            if raster {
                assert_eq!(
                    zencodec::find_cause::<EncodePlanError>(&error),
                    Some(&EncodePlanError::InputProtocol)
                );
            }
        }
    }
}

#[test]
fn startup_drift_or_native_failure_preserves_cause_and_publishes_nothing() {
    for erased in [false, true] {
        for drift in [true, false] {
            let s = Setup::new();
            let mut job = s.job();
            job.drift = drift;
            job.fail_start = !drift;
            let error = run_job(
                erased,
                job,
                s.rows(),
                s.writer(),
                PolicySet::new(),
                s.options(),
                &zencodec::NoPulse,
            )
            .unwrap_err();
            assert_eq!(error.error().stage, Stage::Start);
            assert_eq!(error.error().emitted, 0);
            assert_eq!(*s.events.lock().unwrap(), ["plan", "construct", "start"]);
            assert!(s.bytes.lock().unwrap().is_empty());
            if drift {
                assert_eq!(
                    zencodec::find_cause::<EncodePlanError>(&error),
                    Some(&EncodePlanError::Changed)
                );
            } else {
                assert_eq!(
                    zencodec::find_cause::<io::Error>(&error).unwrap().kind(),
                    io::ErrorKind::InvalidData
                );
            }
        }
    }
}

#[test]
fn erased_destination_preserves_negotiated_patches_and_forward_refusal() {
    let s = Setup::new();
    let mut job = s.job();
    job.patches = true;
    job.plan = EncodePlan::rows(OutputTiming::streaming(0).with_patches());
    let error = row_encode::encode_dyn_job(
        Config(job.clone()).dyn_job(),
        s.rows(),
        s.writer(),
        PolicySet::new(),
        s.options(),
        &zencodec::NoPulse,
    )
    .unwrap_err();
    assert_eq!(error.error().stage, Stage::Prepare);
    assert_eq!(
        zencodec::find_cause::<EncodePlanError>(&error),
        Some(&EncodePlanError::DestinationAccess)
    );
    assert_eq!(*s.events.lock().unwrap(), ["plan"]);
    assert!(s.bytes.lock().unwrap().is_empty());
    s.events.lock().unwrap().clear();
    let mut writer = io::Cursor::new(Vec::new());
    let completed = row_encode::encode_dyn_job_to_destination(
        Config(job).dyn_job(),
        s.rows(),
        zencodec_io::destination::SeekBack::new(&mut writer).unwrap(),
        PolicySet::new(),
        s.options(),
        &zencodec::NoPulse,
    )
    .unwrap();
    assert_eq!(completed.emitted, 14);
    assert_eq!(completed.patched_bytes, 1);
    assert_eq!(
        writer.into_inner(),
        [81, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 90]
    );
}

// The small application-facing path shown in the source guide. Registry jobs
// retain their configured controls; the row driver gets current pixel color.
fn registry_job_to_writer<W: io::Write>(
    codecs: &zencodec::CodecSet,
    pixels: PixelSlice<'_>,
    writer: W,
    storage: StorageBudget,
    pulse: &dyn zencodec::Pulse,
) -> Result<row_encode::Completed, BoxedError> {
    let job = codecs.encode_job(ImageFormat::Pnm)?;
    Ok(row_encode::encode_dyn_job(
        job,
        row_encode::Raster::new(pixels),
        writer,
        PolicySet::new(),
        Options::new(storage, 4096),
        pulse,
    )?)
}

#[test]
fn registry_job_uses_the_shared_planned_driver_without_a_caller_bridge() {
    let s = Setup::new();
    let codecs = zencodec::CodecSet::new().with_encoder(Config(s.job()));
    let bytes = [7; 12];
    let pixels = PixelSlice::new(&bytes, 2, 2, 6, PixelDescriptor::RGB8_SRGB)
        .unwrap()
        .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::DISPLAY_P3)));
    let mut output = Vec::new();
    let done = registry_job_to_writer(
        &codecs,
        pixels,
        &mut output,
        StorageBudget::new(4096, 16),
        &zencodec::NoPulse,
    )
    .unwrap();
    assert_eq!(done.emitted, 14);
    assert_eq!(output, [80, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 90]);
    assert_eq!(
        *s.events.lock().unwrap(),
        ["plan", "construct", "start", "push", "finish"]
    );
}

// Additive draft controls: reuse the existing recording encoders and assertions.
#[path = "planned_row_encode/exact_depth.rs"]
mod exact_depth_tests;
