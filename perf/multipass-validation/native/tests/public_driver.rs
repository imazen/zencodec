//! Actual retained IMI jobs through public zencodec-io typed/erased drivers.
//! The existing experimental adapter/gates are not part of these call paths.
//! Byte and reader parity are native controls, not a third-party oracle.
#[path = "public_driver/support.rs"]
mod support;
use imij::{Error, codec::Scratch};
use std::{
    io,
    sync::{Arc, Mutex, atomic::Ordering},
};
use support::*;
use zencodec::{
    At, ResourceLimits,
    encode::{
        DynEncoderConfig, EncodeDeclaration, EncodeJob, EncodePlan, EncodeRows, Encoder,
        EncoderConfig, InputRequirement, OutputPhase, OutputTiming, PullProgress, RowDemand,
        RowFillError, SourcePasses,
    },
    find_cause,
    operation::{
        BoundPolicy, Decision, DecisionCode, InputAcquisition, OutputSchedule, PolicyFailure,
        PolicyRequirement, PolicyResolver, PolicySet, Question, RequirementEvidence, Selection,
        UnmetRequirements,
    },
    output_patch::OutputAccess,
};
use zencodec_io::row_encode::{self, ExactDepthContradiction, Raster, Refusal, Stage};
use zenpixels::PixelDescriptor;

#[derive(Clone, Copy, Debug)]
enum Route {
    Borrowed,
    Snapshot,
    Positional,
    Disk,
}
const ROUTES: [Route; 4] = [
    Route::Borrowed,
    Route::Snapshot,
    Route::Positional,
    Route::Disk,
];
fn passes(plan: &EncodePlan) -> SourcePasses {
    match plan.input {
        InputRequirement::Replay(passes) => passes,
        _ => panic!("native retained job must use shared replay"),
    }
}
fn declaration(f: &Fixture, access: Access) -> EncodeDeclaration<'_> {
    EncodeDeclaration::new(f.width, f.height, f.descriptor)
        .with_color(&f.color)
        .with_exact_depth(f.exact)
        .with_access(match access {
            Access::Forward => OutputAccess::Forward,
            Access::SeekBack => OutputAccess::SeekBack,
        })
}
struct Observe(Arc<Mutex<Option<EncodePlan>>>);
impl PolicyResolver for Observe {
    type Error = std::convert::Infallible;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<Self::Error>> {
        let Question::EncodeRoute { plan, facts, .. } = question else {
            panic!("one joint question required")
        };
        assert_eq!(plan.timing, OutputTiming::AT_FINISH);
        assert_eq!(plan.output_phase, OutputPhase::FinalPass);
        assert!(facts.evidence.metadata.is_none());
        assert!(facts.evidence.retains_opaque_profile.is_none());
        let mut seen = self.0.lock().unwrap();
        assert!(seen.is_none(), "recreated job/repeated decision");
        *seen = Some(plan.clone());
        Ok(Decision::UsePreset)
    }
}
fn acquisition(done: &row_encode::Completed) -> InputAcquisition {
    assert_eq!(done.report.decisions().count(), 1);
    match done.report.decisions().next().unwrap().selected {
        Some(Selection::EncodeRoute(offer)) => offer.acquisition,
        _ => panic!("joint selection missing"),
    }
}

// Match the public replay fixture: route refusals expose unmet evidence in
// PolicyFailure's decision record; early workspace admission has a direct cause.
fn assert_requirement_refusal(failure: &At<row_encode::Failure>) {
    let policy = find_cause::<PolicyFailure>(failure).expect("typed policy failure");
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

#[derive(Clone, Copy, Default)]
struct Pass {
    rows: u32,
    final_pass: bool,
    label: u8,
}
struct Feed<'a> {
    fixture: &'a Fixture,
    records: [Pass; 8],
    count: usize,
    calls: usize,
}
impl EncodeRows for Feed<'_> {
    fn fill(&mut self, mut demand: RowDemand<'_>) -> Result<(), RowFillError> {
        let ordinal = demand.pass as usize;
        assert!(
            ordinal < self.records.len(),
            "bounded native trace overflow"
        );
        assert!(ordinal <= self.count);
        if ordinal == self.count {
            if ordinal != 0 {
                assert_eq!(self.records[ordinal - 1].rows, self.fixture.height);
                assert!(!self.records[ordinal - 1].final_pass);
            }
            self.records[ordinal] = Pass {
                rows: 0,
                final_pass: demand.final_pass,
                label: demand.label,
            };
            self.count += 1;
        }
        let record = &mut self.records[ordinal];
        assert_eq!(demand.first, record.rows);
        assert_eq!(demand.final_pass, record.final_pass);
        assert_eq!(demand.label, record.label); // Observed native phase labels, never scheduling authority.
        assert_eq!(demand.rows.color_context(), Some(&self.fixture.color));
        assert_eq!(demand.rows.descriptor(), self.fixture.descriptor);
        let pixels = self.fixture.pixels();
        for y in 0..demand.rows.rows() {
            demand
                .rows
                .row_mut(y)
                .copy_from_slice(pixels.row(demand.first + y));
        }
        record.rows += demand.rows.rows();
        assert!(record.rows <= self.fixture.height);
        self.calls += 1;
        Ok(())
    }
}
fn native_trace(f: &Fixture, checksum: bool, access: Access) -> (EncodePlan, usize, Vec<u8>) {
    let job = job(
        config(checksum),
        Scratch::Memory,
        metadata(),
        ResourceLimits::none(),
    );
    let declaration = declaration(f, access);
    let plan = job.plan_encode(&declaration).unwrap();
    let mut encoder = job.encoder().unwrap();
    assert_eq!(encoder.begin_declared(&declaration).unwrap(), plan);
    let mut feed = Feed {
        fixture: f,
        records: [Pass::default(); 8],
        count: 0,
        calls: 0,
    };
    loop {
        assert!(
            encoder.pending_output().is_empty(),
            "AT_FINISH cannot expose pending bytes"
        );
        let before = feed.calls;
        match encoder
            .pull_rows(&mut feed, Some(&zencodec::NoPulse))
            .unwrap()
        {
            PullProgress::Filled => assert_eq!(feed.calls, before + 1),
            PullProgress::Done => {
                assert_eq!(feed.calls, before);
                break;
            }
            _ => panic!("new native progress needs explicit qualification"),
        }
    }
    assert!(encoder.pending_output().is_empty());
    let bound = passes(&plan);
    assert!((bound.min()..=bound.max().unwrap()).contains(&(feed.count as u32)));
    assert_eq!(
        feed.records[..feed.count]
            .iter()
            .filter(|p| p.final_pass)
            .count(),
        1
    );
    assert!(feed.records[feed.count - 1].final_pass);
    assert_eq!(feed.records[feed.count - 1].label, 2, "native coding phase");
    assert!(
        feed.records[..feed.count]
            .iter()
            .all(|p| p.rows == f.height)
    );
    let output = encoder.finish_output().unwrap();
    assert!(
        output.patches.is_empty(),
        "native reserved-index rewrites stay private"
    );
    (plan, feed.count, output.tail.into_vec())
}

#[test]
fn native_retained_jobs_match_in_place_bytes_across_public_routes_and_dispatches() {
    for descriptor in [PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGB16_SRGB] {
        let fixture = Fixture::new(descriptor);
        for checksum in [false, true] {
            let expected = ordinary(&fixture, checksum);
            let expected_pixels = readback(&expected);
            assert_eq!(
                expected_pixels.len(),
                fixture.width as usize * fixture.height as usize * descriptor.bytes_per_pixel()
            );
            // The unbounded whole-image coder can frame differently. Its decoded
            // samples must still equal the bounded in-place/native result.
            let unbounded = config(checksum)
                .job()
                .with_metadata_policy(metadata(), zencodec::MetadataPolicy::PreserveExact)
                .encoder()
                .unwrap()
                .encode(fixture.pixels())
                .unwrap();
            assert_eq!(imij::decode(unbounded.data()).unwrap().1, expected_pixels);
            for access in ACCESSES {
                let (native_plan, native_passes, traced) = native_trace(&fixture, checksum, access);
                assert_eq!(traced, expected);
                for dispatch in DISPATCHES {
                    for route in ROUTES {
                        let native = ScratchDir::new();
                        let raw_dir = ScratchDir::new();
                        let mut raw = RawFile::new(&raw_dir);
                        let counts = Arc::new(Counts::default());
                        let tracked = (!matches!(route, Route::Borrowed)).then(|| counts.clone());
                        let (mut run, output, storage) = run_setup(checksum, &native, tracked);
                        let prefix: &[u8] = if matches!(access, Access::SeekBack) {
                            b"existing destination prefix"
                        } else {
                            b""
                        };
                        {
                            let mut output = output.lock().unwrap();
                            output.bytes.get_mut().extend_from_slice(prefix);
                            output.bytes.set_position(prefix.len() as u64);
                        }
                        let seen = Arc::new(Mutex::new(None));
                        let policy = match route {
                            Route::Borrowed => PolicySet::new()
                                .require_exact_replay(true)
                                .allow_raw_memory_staging(false),
                            Route::Snapshot => PolicySet::new()
                                .with_max_source_passes(1)
                                .require_exact_replay(true),
                            Route::Positional => PolicySet::new().allow_raw_memory_staging(false),
                            Route::Disk => PolicySet::new()
                                .with_max_source_passes(1)
                                .allow_raw_memory_staging(false),
                        };
                        if matches!(route, Route::Disk) {
                            run.options = run
                                .options
                                .with_raw_disk_staging(
                                    raw.take(),
                                    (fixture.width as usize
                                        * fixture.height as usize
                                        * descriptor.bytes_per_pixel())
                                        as u64,
                                )
                                .unwrap();
                        }
                        let policy = BoundPolicy::new(policy, Observe(seen.clone()));
                        let done = if matches!(route, Route::Borrowed) {
                            run.execute(
                                dispatch,
                                access,
                                Raster::new(fixture.pixels()),
                                policy,
                                Some(&zencodec::NoPulse),
                            )
                        } else {
                            run.execute(
                                dispatch,
                                access,
                                Rows::new(
                                    &fixture,
                                    matches!(route, Route::Positional),
                                    counts.clone(),
                                ),
                                policy,
                                Some(&zencodec::NoPulse),
                            )
                        }
                        .unwrap();
                        assert_eq!(*seen.lock().unwrap(), Some(native_plan.clone()));
                        assert_eq!(
                            acquisition(&done),
                            match route {
                                Route::Borrowed => InputAcquisition::BorrowedRaster,
                                Route::Snapshot => InputAcquisition::MemorySnapshot,
                                Route::Positional => InputAcquisition::DirectRows,
                                Route::Disk => InputAcquisition::DiskReplay,
                            }
                        );
                        let traversals = match route {
                            Route::Borrowed => 0,
                            Route::Positional => native_passes as u64,
                            _ => 1,
                        };
                        assert_eq!(counts.traversals.load(Ordering::SeqCst), traversals);
                        assert_eq!(
                            counts.rows.load(Ordering::SeqCst),
                            traversals * u64::from(fixture.height)
                        );
                        assert_eq!(done.schedule, OutputSchedule::FinishFirst);
                        assert_eq!(done.emitted, expected.len() as u64);
                        assert_eq!(done.patched_bytes, 0);
                        let output = output.lock().unwrap();
                        assert_eq!(
                            &output.bytes.get_ref()[prefix.len()..],
                            expected.as_slice(),
                            "{descriptor:?} {checksum} {dispatch:?} {access:?} {route:?}"
                        );
                        assert_eq!(&output.bytes.get_ref()[..prefix.len()], prefix);
                        assert_eq!(
                            readback(&output.bytes.get_ref()[prefix.len()..]),
                            expected_pixels
                        );
                        assert_eq!(output.flushes, 1);
                        assert_eq!(output.seeks, u32::from(matches!(access, Access::SeekBack)));
                        assert_eq!(storage.reserved_bytes(), 0);
                        native.empty();
                    }
                }
            }
        }
    }
}

#[test]
fn source_caps_and_exact_replay_filter_real_native_routes_before_source_or_output() {
    let fixture = Fixture::new(PixelDescriptor::RGB8_SRGB);
    let probe = job(
        config(false),
        Scratch::Memory,
        metadata(),
        ResourceLimits::none(),
    );
    let planned = probe
        .plan_encode(&declaration(&fixture, Access::Forward))
        .unwrap();
    let maximum = passes(&planned).max().unwrap();
    assert!(maximum >= 2, "fixture must require native replay");
    for dispatch in DISPATCHES {
        for cap in [0, 1, 2] {
            for positional in [false, true] {
                for exact in [false, true] {
                    let native = ScratchDir::new();
                    let counts = Arc::new(Counts::default());
                    let (run, output, storage) = run_setup(false, &native, Some(counts.clone()));
                    // Positional excludes raw RAM; forward necessarily snapshots.
                    let policy = PolicySet::new()
                        .with_max_source_passes(cap)
                        .require_exact_replay(exact)
                        .allow_raw_memory_staging(!positional);
                    let result = run.execute(
                        dispatch,
                        Access::Forward,
                        Rows::new(&fixture, positional, counts.clone()),
                        policy,
                        Some(&zencodec::NoPulse),
                    );
                    let allowed = if positional {
                        cap >= maximum && !exact
                    } else {
                        cap >= 1
                    };
                    if allowed {
                        assert!(result.is_ok());
                    } else {
                        let error = result.unwrap_err();
                        assert_eq!(error.error().stage, Stage::Policy);
                        assert_requirement_refusal(&error);
                        assert_eq!(counts.calls.load(Ordering::SeqCst), 0);
                        assert_eq!(output.lock().unwrap().writes, 0);
                    }
                    assert!(counts.dropped.load(Ordering::SeqCst));
                    assert_eq!(storage.reserved_bytes(), 0);
                    native.empty();
                }
            }
        }
        // A real immutable borrow does not exempt source cap zero.
        let native = ScratchDir::new();
        let (run, output, _) = run_setup(false, &native, None);
        let failure = run
            .execute(
                dispatch,
                Access::Forward,
                Raster::new(fixture.pixels()),
                PolicySet::new()
                    .with_max_source_passes(0)
                    .require_exact_replay(true),
                Some(&zencodec::NoPulse),
            )
            .unwrap_err();
        assert_requirement_refusal(&failure);
        assert_eq!(output.lock().unwrap().writes, 0);
        native.empty();
    }
}

#[test]
fn real_native_refusals_do_not_hide_staging_or_fabricate_job_evidence() {
    let fixture = Fixture::new(PixelDescriptor::RGB8_SRGB);
    for dispatch in DISPATCHES {
        for (policy, early_workspace) in [
            (PolicySet::new().allow_raw_memory_staging(false), false),
            (
                PolicySet::new().require_output_schedule(OutputSchedule::Live),
                false,
            ),
            (PolicySet::new().with_workspace_limit(u64::MAX), true),
            (PolicySet::new().with_output_wait_limit(u64::MAX), false),
            (PolicySet::new().allow_opaque_profiles(false), false),
            (
                PolicySet::new()
                    .require_metadata(zencodec::operation::MetadataTransfer::DiscardDescriptive),
                false,
            ),
        ] {
            let native = ScratchDir::new();
            let counts = Arc::new(Counts::default());
            let (run, output, storage) = run_setup(false, &native, Some(counts.clone()));
            let failure = run
                .execute(
                    dispatch,
                    Access::Forward,
                    Rows::new(&fixture, false, counts.clone()),
                    policy,
                    Some(&zencodec::NoPulse),
                )
                .unwrap_err();
            if early_workspace {
                let cause = find_cause::<UnmetRequirements>(&failure)
                    .expect("typed early workspace admission cause");
                assert_eq!(
                    cause.iter().collect::<Vec<_>>(),
                    [(PolicyRequirement::Workspace, RequirementEvidence::Unknown)]
                );
                assert_eq!(failure.error().report.decisions().count(), 0);
            } else {
                assert_requirement_refusal(&failure);
            }
            assert_eq!(failure.error().stage, Stage::Policy);
            assert_eq!(counts.calls.load(Ordering::SeqCst), 0);
            assert_eq!(output.lock().unwrap().writes, 0);
            assert_eq!(storage.reserved_bytes(), 0);
            native.empty();
        }
        let native = ScratchDir::new();
        let raw_dir = ScratchDir::new();
        let mut raw = RawFile::new(&raw_dir);
        let counts = Arc::new(Counts::default());
        let (mut run, output, _) = run_setup(false, &native, Some(counts.clone()));
        run.options = run
            .options
            .with_raw_disk_staging(raw.take(), BUDGET as u64)
            .unwrap();
        let error = run
            .execute(
                dispatch,
                Access::Forward,
                Rows::new(&fixture, false, counts.clone()),
                PolicySet::new()
                    .require_exact_replay(true)
                    .allow_raw_memory_staging(false),
                Some(&zencodec::NoPulse),
            )
            .unwrap_err();
        assert_requirement_refusal(&error);
        assert_eq!(counts.calls.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::metadata(&raw.path).unwrap().len(), 0);
        assert_eq!(output.lock().unwrap().writes, 0);
        native.empty();
    }
}

#[test]
fn native_execution_preserves_typed_source_stop_depth_and_replay_causes_and_drops_owners() {
    let fixture = Fixture::new(PixelDescriptor::RGB16_SRGB);
    for dispatch in DISPATCHES {
        for fault in [
            Fault::Source,
            Fault::Cancel,
            Fault::Depth,
            Fault::ReplayChange,
        ] {
            let native = ScratchDir::new();
            let counts = Arc::new(Counts::default());
            let (run, output, storage) = run_setup(true, &native, Some(counts.clone()));
            let metadata_owner = run.writer.metadata.clone();
            let positional = matches!(fault, Fault::ReplayChange);
            let mut source = Rows::new(&fixture, positional, counts.clone());
            source.fault = fault;
            let pulse = zencodec::how_far::StopOnly::new(Cancel(counts.clone()));
            let failure = run
                .execute(
                    dispatch,
                    Access::Forward,
                    source,
                    PolicySet::new().allow_raw_memory_staging(!positional),
                    Some(&pulse),
                )
                .unwrap_err();
            match fault {
                Fault::Source => {
                    let cause = find_cause::<io::Error>(&failure).unwrap();
                    assert_eq!(cause.kind(), io::ErrorKind::PermissionDenied);
                    assert_eq!(cause.to_string(), "retained-native-source-control");
                }
                Fault::Cancel => assert_eq!(
                    find_cause::<zencodec::CodecError>(&failure)
                        .unwrap()
                        .category(),
                    zencodec::ErrorCategory::Stopped(enough::StopReason::TimedOut)
                ),
                Fault::Depth => assert_eq!(
                    find_cause::<ExactDepthContradiction>(&failure)
                        .unwrap()
                        .first_row,
                    0
                ),
                Fault::ReplayChange => assert!(matches!(
                    find_cause::<Refusal>(&failure),
                    Some(Refusal::ReplayChanged { pass: 1, first: 0 })
                )),
                Fault::None => unreachable!(),
            }
            assert_eq!(failure.error().emitted, 0);
            assert_eq!(failure.error().report.decisions().count(), 1);
            assert_eq!(output.lock().unwrap().writes, 0);
            assert!(counts.dropped.load(Ordering::SeqCst));
            assert!(metadata_owner.upgrade().is_none());
            assert_eq!(storage.reserved_bytes(), 0);
            native.empty();
        }
    }
}

#[test]
fn native_output_cap_and_scratch_start_failures_keep_native_types_and_publish_nothing() {
    let fixture = Fixture::new(PixelDescriptor::RGB8_SRGB);
    for dispatch in DISPATCHES {
        for missing_scratch in [false, true] {
            let native = ScratchDir::new();
            let counts = Arc::new(Counts::default());
            let (mut run, output, storage) = run_setup(true, &native, Some(counts.clone()));
            let metadata_owner = run.writer.metadata.clone();
            if missing_scratch {
                run.scratch = Scratch::Dir(native.0.join("missing"));
            } else {
                run.limits.max_output_bytes = Some(1);
            }
            let failure = run
                .execute(
                    dispatch,
                    Access::Forward,
                    Rows::new(&fixture, false, counts.clone()),
                    PolicySet::new(),
                    Some(&zencodec::NoPulse),
                )
                .unwrap_err();
            assert_eq!(
                find_cause::<Error>(&failure),
                Some(if missing_scratch {
                    &Error::InvalidInput("scratch directory")
                } else {
                    &Error::TooLarge
                })
            );
            if missing_scratch {
                assert_eq!(failure.error().stage, Stage::Start);
                assert_eq!(counts.calls.load(Ordering::SeqCst), 0);
            }
            assert_eq!(output.lock().unwrap().writes, 0);
            assert_eq!(failure.error().emitted, 0);
            assert!(counts.dropped.load(Ordering::SeqCst));
            assert!(metadata_owner.upgrade().is_none());
            assert_eq!(storage.reserved_bytes(), 0);
            native.empty();
        }
    }
}

#[test]
fn unconfigured_registry_job_exposes_native_scratch_selection_gap() {
    let fixture = Fixture::new(PixelDescriptor::RGB8_SRGB);
    let storage = zencodec::storage::StorageBudget::new(BUDGET, 1024);
    let mut output = Vec::new();
    // Direct native registry erasure has no scratch setter; it must refuse,
    // rather than silently fall back or begin the experimental session adapter.
    let job = config(false).dyn_job();
    let error = row_encode::encode_dyn_job(
        job,
        Raster::new(fixture.pixels()),
        &mut output,
        PolicySet::new(),
        row_encode::Options::new(storage.clone(), BUDGET as u64),
        Some(&zencodec::NoPulse),
    )
    .unwrap_err();
    assert_eq!(error.error().stage, Stage::Prepare);
    assert_eq!(
        find_cause::<Error>(&error),
        Some(&Error::Unsupported(
            zencodec::UnsupportedOperation::EncodePlanning.name()
        ))
    );
    assert!(output.is_empty());
    assert_eq!(error.error().emitted, 0);
    assert_eq!(storage.reserved_bytes(), 0);
}
