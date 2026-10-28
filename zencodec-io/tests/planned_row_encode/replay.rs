//! Shared-trait implementer controls, not native codec qualification.
use super::*;
use row_encode::{ExactDepthContradiction, Raster, Refusal};
use zencodec::{
    encode::{EncodeRows, OutputPhase, PullProgress, RowDemand, SourcePasses},
    operation::{DecisionCode, InputAcquisition, OutputSchedule, PolicyFailure, Selection},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behavior {
    Normal,
    SwallowFill,
    NoFill,
    FillThenDone,
    DoubleFill,
    SkipRow,
    ChangeFinal,
    EarlyDone,
    EarlyOutput,
    NativeError,
    StartDrift,
    InvalidPatch,
    Patch,
}
#[derive(Clone)]
struct PullJob {
    events: Events,
    plan: EncodePlan,
    passes: u32,
    behavior: Behavior,
}
impl PullJob {
    fn new(s: &Setup, passes: u32, maximum: Option<u32>) -> Self {
        Self {
            events: s.events.clone(),
            plan: EncodePlan::replay(
                SourcePasses::new(1, maximum).unwrap(),
                OutputTiming::streaming(0),
                OutputPhase::FinalPass,
            ),
            passes,
            behavior: Behavior::Normal,
        }
    }
}
impl EncodeJob for PullJob {
    type Error = io::Error;
    type Enc = PullEncoder;
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
    fn plan_encode(&self, _: &EncodeDeclaration<'_>) -> Result<EncodePlan, BoxedError> {
        event(&self.events, "plan");
        Ok(self.plan.clone())
    }
    fn encoder(self) -> Result<Self::Enc, Self::Error> {
        event(&self.events, "construct");
        Ok(PullEncoder {
            job: self,
            description: None,
            pass: 0,
            first: 0,
            pending: vec![],
        })
    }
    fn animation_frame_encoder(self) -> Result<(), Self::Error> {
        panic!("no fallback")
    }
}
#[derive(Clone)]
struct PullConfig(PullJob);
impl EncoderConfig for PullConfig {
    type Error = io::Error;
    type Job = PullJob;
    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB]
    }
    fn job(self) -> Self::Job {
        self.0
    }
}
struct PullEncoder {
    job: PullJob,
    description: Option<Description>,
    pass: u32,
    first: u32,
    pending: Vec<u8>,
}
impl Encoder for PullEncoder {
    type Error = io::Error;
    fn reject(op: UnsupportedOperation) -> Self::Error {
        io::Error::other(op)
    }
    fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
        panic!("no fallback")
    }
    fn begin_declared(
        &mut self,
        declaration: &EncodeDeclaration<'_>,
    ) -> Result<EncodePlan, Self::Error> {
        event(&self.job.events, "start");
        let mut description = Description::new(
            declaration.width,
            declaration.height,
            declaration.descriptor,
        );
        description.color_context = declaration.color.map(|c| Arc::new(c.clone()));
        description.exact_depth = declaration.exact_depth;
        self.description = Some(description);
        if self.job.behavior == Behavior::StartDrift {
            return Ok(EncodePlan::rows(OutputTiming::AT_FINISH));
        }
        Ok(self.job.plan.clone())
    }
    fn pull_rows(
        &mut self,
        filler: &mut dyn EncodeRows,
        _: &dyn zencodec::Pulse,
    ) -> Result<PullProgress, Self::Error> {
        event(&self.job.events, "pull");
        if self.job.behavior == Behavior::NoFill {
            return Ok(PullProgress::Filled);
        }
        if self.job.behavior == Behavior::EarlyDone || self.pass == self.job.passes {
            return Ok(PullProgress::Done);
        }
        let desc = self.description.as_ref().unwrap();
        let count = 2.min(desc.height - self.first);
        let row_bytes = desc.width as usize * desc.descriptor.bytes_per_pixel();
        let padding = 3 * desc.descriptor.bytes_per_pixel();
        let stride = row_bytes + padding;
        let mut bytes = vec![0xc3; stride * count as usize];
        let repeats = if self.job.behavior == Behavior::DoubleFill {
            2
        } else {
            1
        };
        for _ in 0..repeats {
            let mut rows =
                PixelSliceMut::new(&mut bytes, desc.width, count, stride, desc.descriptor).unwrap();
            if let Some(color) = &desc.color_context {
                rows = rows.with_color_context(color.clone());
            }
            let first = if self.job.behavior == Behavior::SkipRow {
                self.first + 1
            } else {
                self.first
            };
            let final_pass = self.pass + 1 == self.job.passes;
            let final_pass = if self.job.behavior == Behavior::ChangeFinal && self.first != 0 {
                !final_pass
            } else {
                final_pass
            };
            // Labels can vary within a pass. They never confer final-pass authority.
            let demand = RowDemand::new(self.pass, first, rows)
                .with_final_pass(final_pass)
                .with_label((self.first + self.pass) as u8);
            if let Err(error) = filler.fill(demand) {
                self.pending.extend_from_slice(b"must-not-publish");
                if self.job.behavior == Behavior::SwallowFill {
                    return Ok(PullProgress::Done);
                }
                return Err(io::Error::other(error));
            }
        }
        for y in 0..count as usize {
            assert_eq!(
                &bytes[y * stride + row_bytes..(y + 1) * stride],
                &vec![0xc3; padding]
            );
        }
        if self.job.behavior == Behavior::NativeError {
            self.pending.extend_from_slice(b"must-not-publish");
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "native-pull-control",
            ));
        }
        if self.pass + 1 == self.job.passes || self.job.behavior == Behavior::EarlyOutput {
            for y in 0..count as usize {
                self.pending
                    .extend_from_slice(&bytes[y * stride..][..row_bytes]);
            }
        }
        self.first += count;
        if self.first == desc.height {
            self.first = 0;
            self.pass += 1;
        }
        event(&self.job.events, "filled");
        Ok(if self.job.behavior == Behavior::FillThenDone {
            PullProgress::Done
        } else {
            PullProgress::Filled
        })
    }
    fn pending_output(&self) -> &[u8] {
        &self.pending
    }
    fn consume_output(&mut self, n: usize) {
        self.pending.drain(..n);
    }
    fn finish_output(
        mut self,
    ) -> Result<zencodec::output_patch::OutputFinish<EncodeOutput>, Self::Error> {
        event(&self.job.events, "finish");
        self.pending.push(90);
        let patches = if self.job.behavior == Behavior::Patch {
            vec![zencodec::output_patch::Patch {
                offset: 0,
                bytes: vec![42],
            }]
        } else if self.job.behavior == Behavior::InvalidPatch {
            vec![zencodec::output_patch::Patch {
                offset: u64::MAX,
                bytes: vec![1],
            }]
        } else {
            vec![]
        };
        Ok(zencodec::output_patch::OutputFinish::new(
            EncodeOutput::new(std::mem::take(&mut self.pending), ImageFormat::Pnm),
            patches,
        ))
    }
}
impl Drop for PullEncoder {
    fn drop(&mut self) {
        event(&self.job.events, "codec-drop");
    }
}
fn run<S: Source, W: io::Write, R: OperationPolicy>(
    erased: bool,
    job: PullJob,
    source: S,
    writer: W,
    policy: R,
    options: Options,
) -> Result<row_encode::Completed, At<row_encode::Failure>> {
    if erased {
        row_encode::encode_dyn_job(
            PullConfig(job).dyn_job(),
            source,
            writer,
            policy,
            options,
            &zencodec::NoPulse,
        )
    } else {
        row_encode::encode_job(job, source, writer, policy, options, &zencodec::NoPulse)
    }
}
struct ReplaySource {
    inner: Rows,
    positional: bool,
    mutate: bool,
    identity_changes: bool,
    fail: bool,
    false_depth: bool,
    reads: usize,
    next: u32,
}
impl ReplaySource {
    fn new(s: &Setup, positional: bool) -> Self {
        Self {
            inner: s.rows(),
            positional,
            mutate: false,
            identity_changes: false,
            fail: false,
            false_depth: false,
            reads: 0,
            next: 0,
        }
    }
}
impl Source for ReplaySource {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.inner.description()
    }
    fn supports_positional_replay(&self) -> bool {
        self.positional
    }
    fn replay_identity(&self) -> Result<Option<[u64; 4]>, Self::Error> {
        Ok(self
            .identity_changes
            .then_some([self.reads as u64, 0, 0, 0]))
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        _: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error> {
        event(&self.inner.events, "read");
        if self.fail {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "source-control",
            ));
        }
        if !self.positional {
            assert_eq!(first, self.next);
        }
        let changed = self.mutate && first == 0 && self.reads != 0;
        for y in 0..rows.rows() {
            rows.row_mut(y).fill(if changed { 8 } else { 7 });
        }
        if self.false_depth {
            let last = rows.rows() - 1;
            rows.row_mut(last)[0] = 1;
        }
        self.next = first + rows.rows();
        self.reads += 1;
        Ok(())
    }
}
fn selected(done: &row_encode::Completed) -> InputAcquisition {
    match done.report.decisions().next().unwrap().selected {
        Some(Selection::EncodeRoute(offer)) => offer.acquisition,
        _ => panic!("one joint route required"),
    }
}
fn reads(s: &Setup) -> usize {
    s.events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| **e == "read")
        .count()
}
// Mandatory route failures retain the domain Decline cause and expose their
// named unmet evidence in the decision record; the early workspace check has
// a different, direct UnmetRequirements error contract.
fn assert_requirement_refusal(failure: &At<row_encode::Failure>) {
    let policy = zencodec::find_cause::<PolicyFailure>(failure).expect("typed policy failure");
    let record = policy.record();
    assert_eq!(record.code, DecisionCode::Declined);
    assert_eq!(record.admitted, 0);
    assert!(!record.unmet_requirements.is_empty());
    let retained = failure
        .error()
        .report
        .decisions()
        .last()
        .expect("retained refusal record");
    assert_eq!(retained.question, record.question);
    assert_eq!(retained.unmet_requirements, record.unmet_requirements);
}

fn no_native_or_reads(s: &Setup) {
    let events = s.events.lock().unwrap();
    assert!(!events.contains(&"construct"));
    assert!(!events.contains(&"read"));
    assert!(!events.contains(&"write"));
}

#[test]
fn retained_padded_borrow_replays_actual_samples_in_typed_and_erased_drivers() {
    for erased in [false, true] {
        let s = Setup::new();
        let bytes = [1, 2, 3, 4, 5, 6, 0xee, 0xee, 0xee, 11, 12, 13, 14, 15, 16];
        let pixels = PixelSlice::new(&bytes, 2, 2, 9, PixelDescriptor::RGB8_SRGB).unwrap();
        let done = run(
            erased,
            PullJob::new(&s, 2, Some(2)),
            Raster::new(pixels),
            s.writer(),
            PolicySet::new()
                .require_exact_replay(true)
                .allow_raw_memory_staging(false),
            s.options(),
        )
        .unwrap();
        assert_eq!(selected(&done), InputAcquisition::BorrowedRaster);
        assert_eq!(
            *s.bytes.lock().unwrap(),
            [1, 2, 3, 4, 5, 6, 11, 12, 13, 14, 15, 16, 90]
        );
        assert_eq!(reads(&s), 0);
        assert_eq!(
            s.events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| **e == "filled")
                .count(),
            2
        );
    }
}

#[test]
fn forward_source_is_read_once_into_private_snapshot_even_for_unknown_native_maximum() {
    for erased in [false, true] {
        for maximum in [Some(2), None] {
            let s = Setup::new();
            let done = run(
                erased,
                PullJob::new(&s, 2, maximum),
                ReplaySource::new(&s, false),
                s.writer(),
                PolicySet::new()
                    .with_max_source_passes(1)
                    .require_exact_replay(true),
                s.options(),
            )
            .unwrap();
            assert_eq!(selected(&done), InputAcquisition::MemorySnapshot);
            assert_eq!(reads(&s), 1);
            assert_eq!(
                *s.bytes.lock().unwrap(),
                [7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 90]
            );
        }
    }
}

#[test]
fn direct_positional_replay_checks_digest_and_version_and_keeps_original_cause() {
    for erased in [false, true] {
        for (mutate, identity_changes) in [(false, false), (true, false), (false, true)] {
            let s = Setup::new();
            let mut source = ReplaySource::new(&s, true);
            source.mutate = mutate;
            source.identity_changes = identity_changes;
            let result = run(
                erased,
                PullJob::new(&s, 2, Some(2)),
                source,
                s.writer(),
                PolicySet::new().allow_raw_memory_staging(false),
                s.options(),
            );
            if mutate || identity_changes {
                let failure = result.unwrap_err();
                assert!(matches!(
                    zencodec::find_cause::<Refusal>(&failure),
                    Some(Refusal::ReplayChanged { .. })
                ));
                assert_eq!(failure.error().emitted, 0);
                assert!(!s.events.lock().unwrap().contains(&"finish"));
            } else {
                assert_eq!(selected(&result.unwrap()), InputAcquisition::DirectRows);
                assert_eq!(reads(&s), 2);
            }
        }
    }
}

#[test]
fn native_one_pass_requires_no_snapshot_but_caps_and_exact_multi_pass_stay_strict() {
    for erased in [false, true] {
        for maximum in [Some(1), Some(2), None] {
            for cap in [0, 1, 2] {
                for exact in [false, true] {
                    let s = Setup::new();
                    let passes = maximum.unwrap_or(2);
                    let result = run(
                        erased,
                        PullJob::new(&s, passes, maximum),
                        ReplaySource::new(&s, true),
                        s.writer(),
                        PolicySet::new()
                            .with_max_source_passes(cap)
                            .require_exact_replay(exact)
                            .allow_raw_memory_staging(false),
                        s.options(),
                    );
                    let accepted = maximum.is_some_and(|max| max <= cap && (!exact || max == 1));
                    if accepted {
                        assert_eq!(selected(&result.unwrap()), InputAcquisition::DirectRows);
                        assert_eq!(reads(&s), passes as usize);
                    } else {
                        assert_requirement_refusal(&result.unwrap_err());
                        no_native_or_reads(&s);
                    }
                }
            }
        }
    }
}

#[test]
#[allow(deprecated)]
fn bound_and_legacy_requirements_cannot_choose_different_halves_of_a_route() {
    for erased in [false, true] {
        let s = Setup::new();
        let failure = run(
            erased,
            PullJob::new(&s, 2, Some(2)),
            ReplaySource::new(&s, true),
            s.writer(),
            PolicySet::new().require_exact_replay(true),
            s.options()
                .with_policy_set(PolicySet::new().allow_raw_memory_staging(false)),
        )
        .unwrap_err();
        assert_requirement_refusal(&failure);
        no_native_or_reads(&s);
    }
}

#[test]
fn sticky_fill_failure_survives_native_swallow_and_never_drains_failed_output() {
    for erased in [false, true] {
        let s = Setup::new();
        let mut job = PullJob::new(&s, 1, Some(1));
        job.behavior = Behavior::SwallowFill;
        let mut source = ReplaySource::new(&s, false);
        source.fail = true;
        let failure = run(
            erased,
            job,
            source,
            s.writer(),
            PolicySet::new().require_output_schedule(OutputSchedule::Live),
            s.options(),
        )
        .unwrap_err();
        let original = zencodec::find_cause::<io::Error>(&failure).unwrap();
        assert_eq!(original.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(original.to_string(), "source-control");
        assert_eq!(failure.error().emitted, 0);
        assert!(s.bytes.lock().unwrap().is_empty());
        assert!(!s.events.lock().unwrap().contains(&"finish"));
    }
}

#[test]
fn shared_pull_protocol_refuses_bad_progress_order_final_flags_and_early_output() {
    for erased in [false, true] {
        for behavior in [
            Behavior::NoFill,
            Behavior::FillThenDone,
            Behavior::DoubleFill,
            Behavior::SkipRow,
            Behavior::ChangeFinal,
            Behavior::EarlyDone,
            Behavior::EarlyOutput,
            Behavior::StartDrift,
        ] {
            let s = Setup::new();
            s.description.lock().unwrap().height = 4;
            let mut job = PullJob::new(&s, 2, Some(2));
            job.behavior = behavior;
            let failure = run(
                erased,
                job,
                ReplaySource::new(&s, false),
                s.writer(),
                PolicySet::new(),
                s.options(),
            )
            .unwrap_err();
            assert_eq!(failure.error().emitted, 0);
            if behavior == Behavior::StartDrift {
                assert!(zencodec::find_cause::<EncodePlanError>(&failure).is_some());
                assert_eq!(reads(&s), 0);
            } else {
                assert!(zencodec::find_cause::<Refusal>(&failure).is_some());
            }
            assert!(!s.events.lock().unwrap().contains(&"finish"));
        }
    }
}

#[test]
fn finite_plan_maximum_cannot_be_exceeded_and_minimum_requires_complete_final_pass() {
    for erased in [false, true] {
        for (actual, min, max) in [(3, 1, Some(2)), (1, 2, Some(2))] {
            let s = Setup::new();
            let mut job = PullJob::new(&s, actual, max);
            job.plan.input =
                zencodec::encode::InputRequirement::Replay(SourcePasses::new(min, max).unwrap());
            let failure = run(
                erased,
                job,
                ReplaySource::new(&s, false),
                s.writer(),
                PolicySet::new(),
                s.options(),
            )
            .unwrap_err();
            assert!(matches!(
                zencodec::find_cause::<Refusal>(&failure),
                Some(Refusal::PassLimit { .. } | Refusal::IncompletePass { .. })
            ));
            assert_eq!(failure.error().emitted, 0);
            assert!(!s.events.lock().unwrap().contains(&"finish"));
        }
    }
}

#[test]
fn exact_depth_is_verified_before_successful_fill_for_every_native_strip() {
    use zenpixels::{ChannelType, SignalRange, sample::ExactDepth};
    for erased in [false, true] {
        for false_depth in [false, true] {
            let s = Setup::new();
            let claim = ExactDepth::new(ChannelType::U16, SignalRange::Full, 8, 16).unwrap();
            {
                let mut desc = s.description.lock().unwrap();
                desc.descriptor = PixelDescriptor::RGB16;
                desc.exact_depth = Some(claim);
            }
            let mut source = ReplaySource::new(&s, false);
            source.false_depth = false_depth;
            let result = run(
                erased,
                PullJob::new(&s, 2, Some(2)),
                source,
                s.writer(),
                PolicySet::new(),
                s.options(),
            );
            if false_depth {
                let failure = result.unwrap_err();
                assert_eq!(failure.error().stage, Stage::Encode);
                assert_eq!(
                    zencodec::find_cause::<ExactDepthContradiction>(&failure)
                        .unwrap()
                        .first_row,
                    1
                );
                assert!(!s.events.lock().unwrap().contains(&"filled"));
                assert!(!s.events.lock().unwrap().contains(&"finish"));
            } else {
                assert!(result.is_ok());
            }
        }
    }
}

#[test]
fn native_pull_error_and_invalid_finish_patch_never_publish_staged_bytes() {
    for erased in [false, true] {
        for behavior in [Behavior::NativeError, Behavior::InvalidPatch] {
            let s = Setup::new();
            let mut job = PullJob::new(&s, 1, Some(1));
            job.behavior = behavior;
            let failure = run(
                erased,
                job,
                ReplaySource::new(&s, false),
                s.writer(),
                PolicySet::new(),
                s.options(),
            )
            .unwrap_err();
            if behavior == Behavior::NativeError {
                assert_eq!(
                    zencodec::find_cause::<io::Error>(&failure).unwrap().kind(),
                    io::ErrorKind::BrokenPipe
                );
                assert!(!s.events.lock().unwrap().contains(&"finish"));
            }
            assert_eq!(failure.error().emitted, 0);
            assert!(s.bytes.lock().unwrap().is_empty());
        }
    }
}

struct Scratch {
    path: std::path::PathBuf,
    file: Option<std::fs::File>,
}
impl Scratch {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .expect("HOME or USERPROFILE required for replay scratch");
        let directory = std::path::PathBuf::from(home).join("tmp");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!(
            "zencodec-replay-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        Self {
            path,
            file: Some(file),
        }
    }
    fn take(&mut self) -> std::fs::File {
        self.file.take().unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn raw_disk_and_compressed_spool_are_distinct_and_disk_never_claims_exact_replay() {
    for erased in [false, true] {
        for exact in [false, true] {
            let s = Setup::new();
            let mut raw = Scratch::new();
            let mut compressed = Scratch::new();
            let options = s
                .options()
                .with_raw_disk_staging(raw.take(), 12)
                .unwrap()
                .with_disk_staging(compressed.take(), std::num::NonZeroUsize::new(4).unwrap())
                .unwrap();
            let result = run(
                erased,
                PullJob::new(&s, 2, Some(2)),
                ReplaySource::new(&s, false),
                s.writer(),
                PolicySet::new()
                    .allow_raw_memory_staging(false)
                    .with_max_source_passes(1)
                    .require_exact_replay(exact),
                options,
            );
            if exact {
                assert_requirement_refusal(&result.unwrap_err());
                no_native_or_reads(&s);
                assert_eq!(std::fs::metadata(&raw.path).unwrap().len(), 0);
            } else {
                assert_eq!(selected(&result.unwrap()), InputAcquisition::DiskReplay);
                assert_eq!(reads(&s), 1);
                assert_eq!(std::fs::read(&raw.path).unwrap(), vec![7; 12]);
                assert_eq!(
                    std::fs::read(&compressed.path).unwrap(),
                    *s.bytes.lock().unwrap()
                );
                assert_eq!(s.bytes.lock().unwrap().last(), Some(&90));
            }
        }
    }
}

#[test]
fn insufficient_raw_disk_or_snapshot_allowance_refuses_before_effects() {
    for erased in [false, true] {
        let s = Setup::new();
        let mut raw = Scratch::new();
        let failure = run(
            erased,
            PullJob::new(&s, 2, Some(2)),
            ReplaySource::new(&s, false),
            s.writer(),
            PolicySet::new().allow_raw_memory_staging(false),
            s.options().with_raw_disk_staging(raw.take(), 11).unwrap(),
        )
        .unwrap_err();
        assert_requirement_refusal(&failure);
        no_native_or_reads(&s);
        assert_eq!(std::fs::metadata(&raw.path).unwrap().len(), 0);

        let s = Setup::new();
        let storage = StorageBudget::new(11, 16);
        assert!(
            run(
                erased,
                PullJob::new(&s, 2, Some(2)),
                ReplaySource::new(&s, false),
                s.writer(),
                PolicySet::new(),
                Options::new(storage.clone(), 4096)
            )
            .is_err()
        );
        no_native_or_reads(&s);
        assert_eq!(storage.reserved_bytes(), 0);
    }
}

struct ClaimedBorrow(Rows);
impl Source for ClaimedBorrow {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.0.description()
    }
    fn mechanism(&self) -> zencodec::operation::OutputSource {
        zencodec::operation::OutputSource::BorrowedRaster
    }
    fn read_rows(
        &mut self,
        first: u32,
        rows: PixelSliceMut<'_>,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error> {
        self.0.read_rows(first, rows, pulse)
    }
}
#[test]
fn informational_source_mechanism_never_substitutes_for_a_retained_borrow() {
    for erased in [false, true] {
        let s = Setup::new();
        let failure = run(
            erased,
            PullJob::new(&s, 2, Some(2)),
            ClaimedBorrow(s.rows()),
            s.writer(),
            PolicySet::new()
                .require_exact_replay(true)
                .allow_raw_memory_staging(false),
            s.options(),
        )
        .unwrap_err();
        assert_requirement_refusal(&failure);
        no_native_or_reads(&s);
    }
}

struct DropSource(ReplaySource);
impl Source for DropSource {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.0.description()
    }
    fn read_rows(
        &mut self,
        first: u32,
        rows: PixelSliceMut<'_>,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error> {
        self.0.read_rows(first, rows, pulse)
    }
}
impl Drop for DropSource {
    fn drop(&mut self) {
        event(&self.0.inner.events, "source-drop");
    }
}
#[test]
fn finish_first_releases_source_and_native_before_any_destination_wait() {
    for erased in [false, true] {
        let s = Setup::new();
        run(
            erased,
            PullJob::new(&s, 2, Some(2)),
            DropSource(ReplaySource::new(&s, false)),
            s.writer(),
            PolicySet::new(),
            s.options(),
        )
        .unwrap();
        let events = s.events.lock().unwrap();
        let write = events.iter().position(|e| *e == "write").unwrap();
        for before in ["source-drop", "finish", "codec-drop"] {
            assert!(events.iter().position(|e| *e == before).unwrap() < write);
        }
        for once in ["plan", "construct", "start"] {
            assert_eq!(events.iter().filter(|e| **e == once).count(), 1);
        }
    }
}

struct PartialWriter {
    remaining: usize,
    actual: Arc<Mutex<Vec<u8>>>,
}
impl io::Write for PartialWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "write-control"));
        }
        let n = bytes.len().min(self.remaining).min(2);
        self.actual.lock().unwrap().extend_from_slice(&bytes[..n]);
        self.remaining -= n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn partial_publication_preserves_confirmed_byte_count_and_typed_writer_error() {
    for erased in [false, true] {
        for schedule in [OutputSchedule::Live, OutputSchedule::FinishFirst] {
            let s = Setup::new();
            let writer = PartialWriter {
                remaining: 5,
                actual: s.bytes.clone(),
            };
            let failure = run(
                erased,
                PullJob::new(&s, 2, Some(2)),
                ReplaySource::new(&s, false),
                writer,
                PolicySet::new().require_output_schedule(schedule),
                s.options(),
            )
            .unwrap_err();
            assert_eq!(failure.error().emitted, 5);
            assert_eq!(failure.error().stage, Stage::Write);
            assert_eq!(s.bytes.lock().unwrap().len(), 5);
            assert_eq!(failure.error().report.decisions().count(), 1);
            assert_eq!(
                zencodec::find_cause::<io::Error>(&failure).unwrap().kind(),
                io::ErrorKind::BrokenPipe
            );
        }
    }
}

struct FinishStop(Events);
impl enough::Stop for FinishStop {
    fn check(&self) -> Result<(), enough::StopReason> {
        if self.0.lock().unwrap().contains(&"finish") {
            Err(enough::StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}
#[test]
fn finish_cancellation_precedes_publication_and_flush_in_both_dispatches() {
    for erased in [false, true] {
        let s = Setup::new();
        let job = PullJob::new(&s, 2, Some(2));
        let stop = zencodec::how_far::StopOnly::new(FinishStop(s.events.clone()));
        let result = if erased {
            row_encode::encode_dyn_job(
                PullConfig(job).dyn_job(),
                s.rows(),
                s.writer(),
                PolicySet::new(),
                s.options(),
                &stop,
            )
        } else {
            row_encode::encode_job(
                job,
                s.rows(),
                s.writer(),
                PolicySet::new(),
                s.options(),
                &stop,
            )
        };
        let failure = result.unwrap_err();
        assert_eq!(failure.error().stage, Stage::Finish);
        assert_eq!(failure.error().emitted, 0);
        assert!(!s.events.lock().unwrap().contains(&"write"));
        assert_eq!(
            zencodec::find_cause::<zencodec::CodecError>(&failure)
                .unwrap()
                .category(),
            zencodec::ErrorCategory::Stopped(enough::StopReason::TimedOut)
        );
    }
}

#[test]
fn replay_job_applies_finish_patches_relative_to_the_existing_destination_prefix() {
    use std::io::{Seek, Write};
    for erased in [false, true] {
        let s = Setup::new();
        let mut job = PullJob::new(&s, 1, Some(1));
        job.plan.timing = job.plan.timing.with_patches();
        job.behavior = Behavior::Patch;
        let mut output = io::Cursor::new(Vec::new());
        output.write_all(b"prefix").unwrap();
        let writer = zencodec_io::destination::SeekBack::new(&mut output).unwrap();
        let result = if erased {
            row_encode::encode_dyn_job_to_destination(
                PullConfig(job).dyn_job(),
                s.rows(),
                writer,
                PolicySet::new(),
                s.options(),
                &zencodec::NoPulse,
            )
        } else {
            row_encode::encode_job_to_destination(
                job,
                s.rows(),
                writer,
                PolicySet::new(),
                s.options(),
                &zencodec::NoPulse,
            )
        }
        .unwrap();
        assert_eq!(result.emitted, 13);
        assert_eq!(result.patched_bytes, 1);
        assert_eq!(output.stream_position().unwrap(), 19);
        assert_eq!(&output.get_ref()[..7], b"prefix*");
        assert_eq!(output.get_ref().last(), Some(&90));
    }
}

struct ObservedCursor {
    bytes: io::Cursor<Vec<u8>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
    written: usize,
    cancel_after: usize,
}
impl io::Write for ObservedCursor {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = io::Write::write(&mut self.bytes, bytes)?;
        self.written += n;
        if self.written >= self.cancel_after {
            self.stopped
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.calls.lock().unwrap().push("flush");
        Ok(())
    }
}
impl io::Seek for ObservedCursor {
    fn seek(&mut self, position: io::SeekFrom) -> io::Result<u64> {
        self.calls.lock().unwrap().push("seek");
        io::Seek::seek(&mut self.bytes, position)
    }
}
struct WriteStop(Arc<std::sync::atomic::AtomicBool>);
impl enough::Stop for WriteStop {
    fn check(&self) -> Result<(), enough::StopReason> {
        if self.0.load(std::sync::atomic::Ordering::SeqCst) {
            Err(enough::StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}
#[test]
fn replay_cancellation_precedes_patch_seek_restore_and_flush_with_exact_counts() {
    for erased in [false, true] {
        for cancel_after in [13, 14] {
            let s = Setup::new();
            let mut job = PullJob::new(&s, 1, Some(1));
            job.behavior = Behavior::Patch;
            job.plan.timing = job.plan.timing.with_patches();
            let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let pulse = zencodec::how_far::StopOnly::new(WriteStop(stopped.clone()));
            let calls: Arc<Mutex<Vec<&'static str>>> = Arc::default();
            let mut cursor = ObservedCursor {
                bytes: io::Cursor::new(Vec::new()),
                calls: calls.clone(),
                stopped,
                written: 0,
                cancel_after,
            };
            let destination = zencodec_io::destination::SeekBack::new(&mut cursor).unwrap();
            calls.lock().unwrap().clear();
            let result = if erased {
                row_encode::encode_dyn_job_to_destination(
                    PullConfig(job).dyn_job(),
                    s.rows(),
                    destination,
                    PolicySet::new(),
                    s.options(),
                    &pulse,
                )
            } else {
                row_encode::encode_job_to_destination(
                    job,
                    s.rows(),
                    destination,
                    PolicySet::new(),
                    s.options(),
                    &pulse,
                )
            };
            let failure = result.unwrap_err();
            assert_eq!(failure.error().emitted, 13);
            assert_eq!(failure.error().patched_bytes, (cancel_after - 13) as u64);
            assert_eq!(cursor.bytes.get_ref().len(), 13);
            assert_eq!(
                *calls.lock().unwrap(),
                if cancel_after == 13 {
                    vec![]
                } else {
                    vec!["seek"]
                }
            );
            assert_eq!(
                zencodec::find_cause::<zencodec::CodecError>(&failure)
                    .unwrap()
                    .category(),
                zencodec::ErrorCategory::Stopped(enough::StopReason::TimedOut)
            );
        }
    }
}

#[test]
fn unknown_job_metadata_profile_range_and_wait_evidence_refuse_before_construction() {
    use zencodec::operation::MetadataTransfer;
    for erased in [false, true] {
        for policy in [
            PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
            PolicySet::new().allow_opaque_profiles(false),
            PolicySet::new().require_extended_range(true),
            PolicySet::new().with_output_wait_limit(u64::MAX),
        ] {
            let s = Setup::new();
            let failure = run(
                erased,
                PullJob::new(&s, 1, Some(1)),
                s.rows(),
                s.writer(),
                policy,
                s.options(),
            )
            .unwrap_err();
            assert_requirement_refusal(&failure);
            no_native_or_reads(&s);
        }
    }
}

struct AliasTamper {
    file: Mutex<std::fs::File>,
    events: Events,
    fired: std::sync::atomic::AtomicBool,
}
impl enough::Stop for AliasTamper {
    fn check(&self) -> Result<(), enough::StopReason> {
        use std::io::{Seek, Write};
        let completed = self
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == "filled")
            .count();
        if completed == 1 && !self.fired.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let mut file = self.file.lock().unwrap();
            file.seek(io::SeekFrom::Start(0)).unwrap();
            file.write_all(&[9]).unwrap();
        }
        Ok(())
    }
}
#[test]
fn raw_disk_alias_mutation_is_detected_before_replayed_fill_succeeds() {
    for erased in [false, true] {
        let s = Setup::new();
        let job = PullJob::new(&s, 2, Some(2));
        let mut raw = Scratch::new();
        let alias = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&raw.path)
            .unwrap();
        let pulse = zencodec::how_far::StopOnly::new(AliasTamper {
            file: Mutex::new(alias),
            events: s.events.clone(),
            fired: std::sync::atomic::AtomicBool::new(false),
        });
        let options = s.options().with_raw_disk_staging(raw.take(), 12).unwrap();
        let policy = PolicySet::new().allow_raw_memory_staging(false);
        let result = if erased {
            row_encode::encode_dyn_job(
                PullConfig(job).dyn_job(),
                s.rows(),
                s.writer(),
                policy,
                options,
                &pulse,
            )
        } else {
            row_encode::encode_job(job, s.rows(), s.writer(), policy, options, &pulse)
        };
        let failure = result.unwrap_err();
        assert_eq!(
            zencodec::find_cause::<Refusal>(&failure),
            Some(&Refusal::ReplayChanged { pass: 1, first: 0 })
        );
        assert_eq!(failure.error().emitted, 0);
        assert_eq!(
            s.events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| **e == "filled")
                .count(),
            1
        );
        assert!(!s.events.lock().unwrap().contains(&"finish"));
    }
}
