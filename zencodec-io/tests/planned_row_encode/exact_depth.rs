//! Draft regressions; these reuse the parent's recording job/encoder, unchanged.
use super::*;
use row_encode::{ExactDepthContradiction, Raster, Refusal};
use std::num::NonZeroU32;
use zencodec::operation::{OutputSchedule, Selection};
use zenpixels::{
    ChannelType, SignalRange,
    sample::{ExactDepth, ExactDepthError},
};

fn claim() -> ExactDepth {
    ExactDepth::new(ChannelType::U16, SignalRange::Full, 8, 16).unwrap()
}

#[derive(Clone)]
struct ClaimJob {
    inner: Job,
    expected: Option<ExactDepth>,
}
impl EncodeJob for ClaimJob {
    type Error = io::Error;
    type Enc = ClaimEncoder;
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
        assert_eq!(declaration.exact_depth, self.expected);
        self.inner.plan_encode(declaration)
    }
    fn encoder(self) -> Result<Self::Enc, Self::Error> {
        Ok(ClaimEncoder {
            inner: self.inner.encoder()?,
            expected: self.expected,
        })
    }
    fn animation_frame_encoder(self) -> Result<(), Self::Error> {
        panic!("no fallback")
    }
}
#[derive(Clone)]
struct ClaimConfig(ClaimJob);
impl EncoderConfig for ClaimConfig {
    type Error = io::Error;
    type Job = ClaimJob;
    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB16]
    }
    fn job(self) -> Self::Job {
        self.0
    }
}
struct ClaimEncoder {
    inner: Enc,
    expected: Option<ExactDepth>,
}
impl Encoder for ClaimEncoder {
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
        assert_eq!(declaration.exact_depth, self.expected);
        self.inner.begin_declared(declaration)
    }
    fn begin_output(
        &mut self,
        _: u32,
        _: u32,
        _: PixelDescriptor,
        _: zencodec::output_patch::OutputAccess,
    ) -> Result<OutputTiming, Self::Error> {
        // The legacy seam has no declaration parameter. Preserve its startup
        // protocol while recording that structural validation preceded it.
        event(&self.inner.events, "start");
        self.inner.pending.push(80);
        Ok(self.inner.plan.timing)
    }
    fn pending_output(&self) -> &[u8] {
        self.inner.pending_output()
    }
    fn consume_output(&mut self, n: usize) {
        self.inner.consume_output(n);
    }
    fn push_rows(&mut self, pixels: PixelSlice<'_>) -> Result<(), Self::Error> {
        assert_eq!(
            pixels.exact_depth(),
            self.expected,
            "driver must preserve verified evidence"
        );
        if let Some(exact) = self.expected {
            assert!(
                exact.holds_for(&pixels).unwrap(),
                "bad strip reached encoder"
            );
        }
        self.inner.push_rows(pixels)
    }
    fn finish(self) -> Result<EncodeOutput, Self::Error> {
        self.inner.finish()
    }
    fn finish_output(
        self,
    ) -> Result<zencodec::output_patch::OutputFinish<EncodeOutput>, Self::Error> {
        self.inner.finish_output()
    }
}

#[derive(Clone, Copy)]
enum Route {
    Legacy,
    Typed,
    Erased,
    ErasedDestination,
}
const ROUTES: [Route; 4] = [
    Route::Legacy,
    Route::Typed,
    Route::Erased,
    Route::ErasedDestination,
];
fn run<S: Source, W: io::Write, R: OperationPolicy>(
    route: Route,
    job: ClaimJob,
    source: S,
    writer: W,
    policy: R,
    options: Options,
) -> Result<row_encode::Completed, At<row_encode::Failure>> {
    run_with_pulse(
        route,
        job,
        source,
        writer,
        policy,
        options,
        &zencodec::NoPulse,
    )
}
fn run_with_pulse<S: Source, W: io::Write, R: OperationPolicy>(
    route: Route,
    job: ClaimJob,
    source: S,
    writer: W,
    policy: R,
    options: Options,
    pulse: &dyn zencodec::Pulse,
) -> Result<row_encode::Completed, At<row_encode::Failure>> {
    match route {
        Route::Legacy => row_encode::encode(
            job.encoder().unwrap(),
            source,
            writer,
            policy,
            options,
            pulse,
        ),
        Route::Typed => row_encode::encode_job(job, source, writer, policy, options, pulse),
        Route::Erased => row_encode::encode_dyn_job(
            ClaimConfig(job).dyn_job(),
            source,
            writer,
            policy,
            options,
            pulse,
        ),
        Route::ErasedDestination => row_encode::encode_dyn_job_to_destination(
            ClaimConfig(job).dyn_job(),
            source,
            zencodec_io::destination::Forward(writer),
            policy,
            options,
            pulse,
        ),
    }
}
fn options(storage: StorageBudget) -> Options {
    Options::new(storage, 4096).with_batch_rows(NonZeroU32::new(2).unwrap())
}
fn setup(descriptor: PixelDescriptor, height: u32, exact: Option<ExactDepth>) -> Setup {
    let s = Setup::new();
    let mut description = Description::new(2, height, descriptor)
        .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::DISPLAY_P3)));
    description.exact_depth = exact;
    *s.description.lock().unwrap() = description;
    s
}
fn job(s: &Setup) -> ClaimJob {
    ClaimJob {
        inner: s.job(),
        expected: s.description.lock().unwrap().exact_depth,
    }
}
struct Samples {
    inner: Rows,
    bad_row: Option<u32>,
    change_after_read: bool,
}
impl Source for Samples {
    type Error = io::Error;
    fn description(&self) -> Description {
        self.inner.description()
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        _: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error> {
        event(&self.inner.events, "read");
        for row in 0..rows.rows() {
            // Repeated byte 7 is canonical eight-bit U16 value 7 * 257.
            rows.row_mut(row).fill(7);
            if self.bad_row == Some(first + row) {
                rows.row_mut(row)[..2].copy_from_slice(&1u16.to_ne_bytes());
            }
        }
        if self.change_after_read {
            self.inner.description.lock().unwrap().exact_depth = None;
        }
        Ok(())
    }
}
fn samples(s: &Setup, bad_row: Option<u32>) -> Samples {
    Samples {
        inner: s.rows(),
        bad_row,
        change_after_read: false,
    }
}

#[test]
fn description_defaults_builders_debug_and_eq_include_evidence() {
    let plain = Description::new(2, 3, PixelDescriptor::RGB16);
    let claimed = plain.clone().with_exact_depth(claim());
    assert_eq!(plain.exact_depth, None);
    assert_ne!(plain, claimed);
    assert_eq!(claimed, claimed.clone());
    assert!(format!("{claimed:?}").contains("exact_depth: Some"));
}

#[test]
fn claims_reach_typed_erased_and_legacy_consumers_without_creating_a_fidelity_requirement() {
    for route in ROUTES {
        for exact in [None, Some(claim())] {
            let s = setup(PixelDescriptor::RGB16, 2, exact);
            let done = run(
                route,
                job(&s),
                samples(&s, None),
                s.writer(),
                PolicySet::new(),
                options(StorageBudget::new(4096, 16)),
            )
            .unwrap();
            assert_eq!(done.emitted, 26); // header + two 12-byte rows + tail
            assert_eq!(done.schedule, OutputSchedule::FinishFirst);
            assert_eq!(
                s.events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|e| **e == "push")
                    .count(),
                1
            );
        }
    }
}

#[test]
fn inconsistent_claims_refuse_before_native_start_source_reads_or_publication() {
    let cases = [
        (
            PixelDescriptor::RGB8,
            claim(),
            ExactDepthError::DescriptorMismatch,
        ),
        (
            PixelDescriptor::RGB16.with_signal_range(SignalRange::Narrow),
            claim(),
            ExactDepthError::DescriptorMismatch,
        ),
        (
            PixelDescriptor::CMYK8,
            ExactDepth::new(ChannelType::U8, SignalRange::Full, 8, 8).unwrap(),
            ExactDepthError::UnsupportedLayout,
        ),
    ];
    for route in ROUTES {
        for (descriptor, exact, expected) in cases {
            let s = setup(descriptor, 2, Some(exact));
            let storage = StorageBudget::new(4096, 16);
            let error = run(
                route,
                job(&s),
                samples(&s, None),
                s.writer(),
                PolicySet::new(),
                options(storage.clone()),
            )
            .unwrap_err();
            assert_eq!(error.error().stage, Stage::Prepare);
            assert_eq!(
                zencodec::find_cause::<ExactDepthError>(&error),
                Some(&expected)
            );
            assert_eq!(error.error().emitted, 0);
            assert!(s.bytes.lock().unwrap().is_empty());
            assert!(
                !s.events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| ["plan", "start", "read", "push", "write", "finish"].contains(e))
            );
            assert_eq!(storage.reserved_bytes(), 0);
        }
    }
}

#[test]
fn false_initial_and_later_claims_never_push_a_partial_bad_strip_or_retry() {
    for route in ROUTES {
        for schedule in [OutputSchedule::Live, OutputSchedule::FinishFirst] {
            for first_bad in [0, 5] {
                let s = setup(PixelDescriptor::RGB16, 6, Some(claim()));
                let storage = StorageBudget::new(4096, 16);
                let error = run(
                    route,
                    job(&s),
                    samples(&s, Some(first_bad)),
                    s.writer(),
                    PolicySet::new().require_output_schedule(schedule),
                    options(storage.clone()),
                )
                .unwrap_err();
                assert_eq!(error.error().stage, Stage::Encode);
                let detail = zencodec::find_cause::<ExactDepthContradiction>(&error).unwrap();
                assert_eq!(detail.first_row, first_bad);
                assert_eq!(detail.claim, claim());
                let good_strips = if first_bad == 0 { 0 } else { 2 };
                let events = s.events.lock().unwrap();
                assert_eq!(events.iter().filter(|e| **e == "push").count(), good_strips);
                assert_eq!(
                    events.iter().filter(|e| **e == "read").count(),
                    good_strips + 1
                );
                assert_eq!(events.iter().filter(|e| **e == "start").count(), 1);
                assert!(!events.contains(&"finish"));
                let emitted = if schedule == OutputSchedule::Live {
                    1 + good_strips * 24
                } else {
                    0
                };
                assert_eq!(error.error().emitted, emitted as u64);
                assert_eq!(error.error().patched_bytes, 0);
                let mut expected = if emitted == 0 { vec![] } else { vec![80] };
                if emitted != 0 {
                    expected.extend(vec![7; good_strips * 24]);
                }
                assert_eq!(*s.bytes.lock().unwrap(), expected);
                let records: Vec<_> = error.error().report.decisions().collect();
                assert_eq!(records.len(), 1);
                assert!(match records[0].selected {
                    Some(Selection::OutputSchedule(offer)) if matches!(route, Route::Legacy) =>
                        offer.schedule == schedule,
                    Some(Selection::EncodeRoute(offer)) if !matches!(route, Route::Legacy) =>
                        offer.output.schedule == schedule,
                    _ => false,
                });
                assert_eq!(storage.reserved_bytes(), 0);
            }
        }
    }
}

#[test]
fn changing_only_evidence_in_policy_or_source_read_is_terminal() {
    for route in [Route::Typed, Route::Erased, Route::ErasedDestination] {
        for during_read in [false, true] {
            let s = setup(PixelDescriptor::RGB16, 2, Some(claim()));
            let mut input = samples(&s, None);
            input.change_after_read = during_read;
            let error = run(
                route,
                job(&s),
                input,
                s.writer(),
                BoundPolicy::new(
                    PolicySet::new(),
                    Resolver(|| {
                        if !during_read {
                            s.description.lock().unwrap().exact_depth = None;
                        }
                    }),
                ),
                options(StorageBudget::new(4096, 16)),
            )
            .unwrap_err();
            assert_eq!(error.error().stage, Stage::Read);
            assert_eq!(
                zencodec::find_cause::<Refusal>(&error),
                Some(&Refusal::SourceChanged)
            );
            assert_eq!(error.error().emitted, 0);
            assert!(s.bytes.lock().unwrap().is_empty());
            let events = s.events.lock().unwrap();
            assert!(!events.contains(&"push"));
            assert!(!events.contains(&"finish"));
            assert_eq!(events.contains(&"start"), during_read);
        }
    }
}

#[test]
fn raster_copies_the_claim_but_not_stride_or_rgbx_padding_into_the_verifier() {
    for route in ROUTES {
        // Undefined X bytes and row padding deliberately contradict the 1-bit
        // assertion; only real RGB values may be checked. Final row has no pad.
        let bytes = [0, 255, 0, 7, 3, 4, 5, 6, 255, 0, 255, 9];
        let exact = ExactDepth::new(ChannelType::U8, SignalRange::Full, 1, 8).unwrap();
        let pixels = PixelSlice::new(&bytes, 1, 2, 8, PixelDescriptor::RGBX8)
            .unwrap()
            .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::DISPLAY_P3)))
            .with_exact_depth(exact)
            .unwrap();
        let raster = Raster::new(pixels);
        assert_eq!(raster.description().exact_depth, Some(exact));
        let s = setup(PixelDescriptor::RGBX8, 2, Some(exact));
        let done = run(
            route,
            job(&s),
            raster,
            s.writer(),
            PolicySet::new(),
            options(StorageBudget::new(4096, 16)),
        )
        .unwrap();
        assert_eq!(done.emitted, 10);
        assert_eq!(
            *s.bytes.lock().unwrap(),
            [80, 0, 255, 0, 7, 255, 0, 255, 9, 90]
        );
    }
}

#[repr(align(2))]
struct Aligned<const N: usize>([u8; N]);

#[test]
fn strided_u16_narrow_color_checks_full_scale_alpha_and_excludes_row_padding() {
    for route in ROUTES {
        for bad_alpha in [false, true] {
            let descriptor = PixelDescriptor::GRAYA16.with_signal_range(SignalRange::Narrow);
            let exact = ExactDepth::new(ChannelType::U16, SignalRange::Narrow, 10, 10).unwrap();
            let mut bytes = Aligned([1; 12]);
            for (offset, value) in [
                (0, 64u16),
                (2, 65535),
                (8, 128),
                (10, if bad_alpha { 32768 } else { 65535 }),
            ] {
                bytes.0[offset..offset + 2].copy_from_slice(&value.to_ne_bytes());
            }
            let pixels = PixelSlice::new(&bytes.0, 1, 2, 8, descriptor)
                .unwrap()
                .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::DISPLAY_P3)))
                .with_exact_depth(exact)
                .unwrap();
            let s = setup(descriptor, 2, Some(exact));
            let result = run(
                route,
                job(&s),
                Raster::new(pixels),
                s.writer(),
                PolicySet::new(),
                options(StorageBudget::new(4096, 16)),
            );
            if bad_alpha {
                let error = result.unwrap_err();
                assert_eq!(
                    zencodec::find_cause::<ExactDepthContradiction>(&error)
                        .unwrap()
                        .first_row,
                    1
                );
                assert_eq!(error.error().emitted, 0);
                assert!(!s.events.lock().unwrap().contains(&"push"));
                assert!(!s.events.lock().unwrap().contains(&"finish"));
            } else {
                assert_eq!(result.unwrap().emitted, 10);
                let mut expected = vec![80];
                expected.extend_from_slice(&bytes.0[..4]);
                expected.extend_from_slice(&bytes.0[8..]);
                expected.push(90);
                assert_eq!(*s.bytes.lock().unwrap(), expected);
            }
        }
    }
}

#[test]
fn live_writer_mutation_of_only_the_claim_retains_header_count_and_prevents_read() {
    struct MutatingWriter<'a>(&'a Setup);
    impl io::Write for MutatingWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.description.lock().unwrap().exact_depth = None;
            self.0.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    for route in ROUTES {
        let s = setup(PixelDescriptor::RGB16, 2, Some(claim()));
        let error = run(
            route,
            job(&s),
            samples(&s, None),
            MutatingWriter(&s),
            PolicySet::new().require_output_schedule(OutputSchedule::Live),
            options(StorageBudget::new(4096, 16)),
        )
        .unwrap_err();
        assert_eq!(error.error().stage, Stage::Read);
        assert_eq!(error.error().emitted, 1);
        assert_eq!(*s.bytes.lock().unwrap(), [80]);
        assert_eq!(
            zencodec::find_cause::<Refusal>(&error),
            Some(&Refusal::SourceChanged)
        );
        let events = s.events.lock().unwrap();
        assert!(!events.contains(&"read"));
        assert!(!events.contains(&"push"));
        assert!(!events.contains(&"finish"));
    }
}

#[test]
fn raster_false_assertion_is_checked_and_absent_evidence_does_not_ban_the_samples() {
    for route in ROUTES {
        for stated in [false, true] {
            let mut pixels = zenpixels::PixelBuffer::new(1, 2, PixelDescriptor::GRAY16);
            pixels
                .as_slice_mut()
                .row_mut(1)
                .copy_from_slice(&1u16.to_ne_bytes());
            let mut view = pixels
                .as_slice()
                .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::DISPLAY_P3)));
            if stated {
                view = view.with_exact_depth(claim()).unwrap();
            }
            let exact = if stated { Some(claim()) } else { None };
            let s = setup(PixelDescriptor::GRAY16, 2, exact);
            let result = run(
                route,
                job(&s),
                Raster::new(view),
                s.writer(),
                PolicySet::new(),
                options(StorageBudget::new(4096, 16)),
            );
            if stated {
                let error = result.unwrap_err();
                assert_eq!(
                    zencodec::find_cause::<ExactDepthContradiction>(&error)
                        .unwrap()
                        .first_row,
                    1
                );
                assert_eq!(error.error().emitted, 0);
                assert!(!s.events.lock().unwrap().contains(&"push"));
                assert!(!s.events.lock().unwrap().contains(&"finish"));
            } else {
                assert_eq!(result.unwrap().emitted, 6);
                assert!(s.events.lock().unwrap().contains(&"push"));
                assert!(s.events.lock().unwrap().contains(&"finish"));
            }
        }
    }
}

// Activate from source events, not from an operation-wide poll number. The
// post-read check is followed by checks for each verification row; trigger on
// the second row of the second strip, after one strip has reached the encoder.
struct DuringVerification {
    events: Events,
    description: Arc<Mutex<Description>>,
    checks: std::sync::atomic::AtomicUsize,
    mutate: bool,
}
impl enough::Stop for DuringVerification {
    fn check(&self) -> Result<(), enough::StopReason> {
        use std::sync::atomic::Ordering;
        let reads = self
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == "read")
            .count();
        if reads == 2 && self.checks.fetch_add(1, Ordering::SeqCst) == 2 {
            if self.mutate {
                self.description.lock().unwrap().exact_depth = None;
            } else {
                return Err(enough::StopReason::TimedOut);
            }
        }
        Ok(())
    }
}

#[test]
fn verification_pulse_cancellation_or_claim_change_withholds_the_entire_strip() {
    for route in ROUTES {
        for schedule in [OutputSchedule::Live, OutputSchedule::FinishFirst] {
            for mutate in [false, true] {
                let s = setup(PixelDescriptor::RGB16, 6, Some(claim()));
                let storage = StorageBudget::new(4096, 16);
                let control = zencodec::how_far::StopOnly::new(DuringVerification {
                    events: s.events.clone(),
                    description: s.description.clone(),
                    checks: std::sync::atomic::AtomicUsize::new(0),
                    mutate,
                });
                let failure = run_with_pulse(
                    route,
                    job(&s),
                    samples(&s, None),
                    s.writer(),
                    PolicySet::new().require_output_schedule(schedule),
                    options(storage.clone()),
                    &control,
                )
                .unwrap_err();
                if mutate {
                    assert_eq!(failure.error().stage, Stage::Read);
                    assert_eq!(
                        zencodec::find_cause::<Refusal>(&failure),
                        Some(&Refusal::SourceChanged)
                    );
                } else {
                    assert_eq!(failure.error().stage, Stage::Encode);
                    let cause = zencodec::find_cause::<zencodec::CodecError>(&failure).unwrap();
                    assert_eq!(
                        cause.category(),
                        zencodec::ErrorCategory::Stopped(enough::StopReason::TimedOut)
                    );
                }
                let events = s.events.lock().unwrap();
                assert_eq!(events.iter().filter(|e| **e == "push").count(), 1);
                assert_eq!(events.iter().filter(|e| **e == "read").count(), 2);
                assert!(!events.contains(&"finish"));
                let emitted = if schedule == OutputSchedule::Live {
                    25
                } else {
                    0
                };
                assert_eq!(failure.error().emitted, emitted);
                assert_eq!(s.bytes.lock().unwrap().len(), emitted as usize);
                assert_eq!(failure.error().report.decisions().count(), 1);
                assert_eq!(storage.reserved_bytes(), 0);
            }
        }
    }
}

struct OneShotOnly;
impl Encoder for OneShotOnly {
    type Error = UnsupportedOperation;
    fn reject(op: UnsupportedOperation) -> Self::Error {
        op
    }
    fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
        panic!("row execution must not fall back to one-shot encoding")
    }
}
struct Unplanned;
impl EncodeJob for Unplanned {
    type Error = UnsupportedOperation;
    type Enc = OneShotOnly;
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
    fn encoder(self) -> Result<Self::Enc, Self::Error> {
        Ok(OneShotOnly)
    }
    fn animation_frame_encoder(self) -> Result<(), Self::Error> {
        panic!("no fallback")
    }
}
struct PlannedButNoDeclaredStart;
impl EncodeJob for PlannedButNoDeclaredStart {
    type Error = UnsupportedOperation;
    type Enc = OneShotOnly;
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
        Ok(EncodePlan::rows(OutputTiming::AT_FINISH))
    }
    fn encoder(self) -> Result<Self::Enc, Self::Error> {
        Ok(OneShotOnly)
    }
    fn animation_frame_encoder(self) -> Result<(), Self::Error> {
        panic!("no fallback")
    }
}

#[test]
fn supported_claims_preserve_inherited_startup_refusals_without_fallback() {
    for exact in [None, Some(claim())] {
        for seam in 0..3 {
            let s = setup(PixelDescriptor::RGB16, 2, exact);
            let storage = StorageBudget::new(4096, 16);
            let source = samples(&s, None);
            let writer = s.writer();
            let options = options(storage.clone());
            let error = match seam {
                0 => row_encode::encode(
                    OneShotOnly,
                    source,
                    writer,
                    PolicySet::new(),
                    options,
                    &zencodec::NoPulse,
                ),
                1 => row_encode::encode_job(
                    Unplanned,
                    source,
                    writer,
                    PolicySet::new(),
                    options,
                    &zencodec::NoPulse,
                ),
                2 => row_encode::encode_job(
                    PlannedButNoDeclaredStart,
                    source,
                    writer,
                    PolicySet::new(),
                    options,
                    &zencodec::NoPulse,
                ),
                _ => unreachable!(),
            }
            .unwrap_err();
            let expected = if seam == 0 {
                UnsupportedOperation::RowLevelEncode
            } else {
                UnsupportedOperation::EncodePlanning
            };
            assert_eq!(
                zencodec::find_cause::<UnsupportedOperation>(&error),
                Some(&expected)
            );
            assert!(s.events.lock().unwrap().is_empty());
            assert!(s.bytes.lock().unwrap().is_empty());
            assert_eq!(error.error().emitted, 0);
            assert_eq!(
                error.error().report.decisions().count(),
                usize::from(seam == 2)
            );
            assert_eq!(storage.reserved_bytes(), 0);
        }
    }
}
