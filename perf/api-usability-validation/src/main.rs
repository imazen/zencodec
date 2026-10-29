//! Count-based probes for docs/api-usability-review-2026-10-27.md.
//!
//! Each probe prints one tab-separated line: probe, case, measured values.
//! Counts come from dhat's heap counters (allocation calls and requested
//! bytes) or from pointers observed inside a recording encoder. They are not
//! timings and not RSS.
#![forbid(unsafe_code)]

use std::{cell::RefCell, fmt, num::NonZeroU32, rc::Rc};
use zencodec::{
    At, CodecSet, ImageFormat, UnsupportedOperation,
    decode::{DecodeBoundary, InterpretationFacts, InterpretationState},
    encode::{EncodeOutput, Encoder, EncoderConfig, OutputTiming},
    operation::{
        Decision, DecisionRecord, DecodeRequest, EncodeRequest, MetadataTransfer, OutputSchedule,
        OutputScheduleFacts, OutputScheduleOffer, OutputSource, PolicyFailure, PolicySet, Question,
        TranscodeOffer, resolve_boundary, resolve_output_schedule, resolve_transcode,
    },
    storage::StorageBudget,
};
use zencodec_io::row_encode::{self, Options, Raster};
use zencodec_testkit::{ReferenceDecoderConfig, ReferenceEncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};

/// The current core takes an optional `Pulse`; base 6b0caf34 took an `enough::Stop`
/// (feature `baseline-6b0caf34`, which reproduces results_2026-10-27.tsv).
#[cfg(not(feature = "baseline-6b0caf34"))]
static STOP: Option<&dyn zencodec::Pulse> = Some(&zencodec::NoPulse);
#[cfg(feature = "baseline-6b0caf34")]
static STOP: &enough::Unstoppable = &enough::Unstoppable;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// Allocation calls and requested bytes during `f`.
fn heap<R>(f: impl FnOnce() -> R) -> (R, u64, u64) {
    let before = dhat::HeapStats::get();
    let result = f();
    let after = dhat::HeapStats::get();
    (
        result,
        after.total_blocks - before.total_blocks,
        after.total_bytes - before.total_bytes,
    )
}

fn registry(registrations: usize) -> CodecSet {
    let mut codecs = CodecSet::new();
    for _ in 0..registrations {
        codecs = codecs
            .with_decoder(ReferenceDecoderConfig)
            .with_encoder(ReferenceEncoderConfig::new());
    }
    codecs
}

fn gradient(width: u32, height: u32) -> Vec<u8> {
    (0..width as usize * height as usize * 3)
        .map(|i| (i * 7 % 251) as u8)
        .collect()
}

/// P5: per-operation registry cloning.
fn registry_clone() {
    for n in [0usize, 1, 4, 16] {
        let (codecs, blocks, bytes) = heap(|| registry(n));
        println!("registry_build\tpairs={n}\tallocs={blocks}\tbytes={bytes}");
        let (_, blocks, bytes) = heap(|| drop(codecs.clone()));
        println!("registry_clone\tpairs={n}\tclone_allocs={blocks}\tclone_bytes={bytes}");
        let (session, blocks, bytes) = heap(|| {
            DecodeRequest::new().start_controlled(&codecs, StorageBudget::new(1 << 20, 16))
        });
        let ok = session.is_ok();
        drop(session);
        println!("start_controlled\tpairs={n}\tok={ok}\tallocs={blocks}\tbytes={bytes}");
    }
}

/// P6: a required output descriptor refused only after decoding.
fn required_descriptor() {
    let (width, height) = (512u32, 512u32);
    let data = gradient(width, height);
    let pixels = PixelSlice::new(
        &data,
        width,
        height,
        width as usize * 3,
        PixelDescriptor::RGB8_SRGB,
    )
    .expect("valid raster");
    let encoded = ReferenceEncoderConfig::new()
        .encode(pixels)
        .expect("reference encode");
    let codecs = registry(1);
    let raster = (width * height * 3) as u64;
    for (case, request) in [
        (
            "required_match",
            DecodeRequest::new()
                .with_format(ImageFormat::Pnm)
                .with_output(PixelDescriptor::RGB8_SRGB),
        ),
        (
            "required_mismatch",
            DecodeRequest::new()
                .with_format(ImageFormat::Pnm)
                .with_output(PixelDescriptor::BGRA8_SRGB),
        ),
    ] {
        let (result, blocks, bytes) = heap(|| request.decode(&codecs, encoded.data()));
        let outcome = match &result {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("err={:?}", e.error()),
        };
        println!(
            "required_descriptor\t{case}\traster_bytes={raster}\tallocs={blocks}\tbytes={bytes}\t{outcome}"
        );
    }
}

#[derive(Debug)]
struct ProbeError;
impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("probe encoder refused")
    }
}
impl std::error::Error for ProbeError {}

/// Records the address of every pushed batch and emits `chunk` bytes per push.
struct Recorder {
    pushed: Rc<RefCell<Vec<(usize, usize)>>>,
    pending: Vec<u8>,
    chunk: usize,
}
impl Encoder for Recorder {
    type Error = ProbeError;
    fn reject(_: UnsupportedOperation) -> ProbeError {
        ProbeError
    }
    fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, ProbeError> {
        Err(ProbeError)
    }
    fn begin(&mut self, _: u32, _: u32, _: PixelDescriptor) -> Result<OutputTiming, ProbeError> {
        Ok(OutputTiming::streaming(0))
    }
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), ProbeError> {
        let first = rows.row(0).as_ptr() as usize;
        self.pushed.borrow_mut().push((first, rows.rows() as usize));
        self.pending.extend(std::iter::repeat_n(0u8, self.chunk));
        Ok(())
    }
    fn pending_output(&self) -> &[u8] {
        &self.pending
    }
    fn consume_output(&mut self, n: usize) {
        let n = n.min(self.pending.len());
        self.pending.drain(..n);
    }
    fn finish(self) -> Result<EncodeOutput, ProbeError> {
        Ok(EncodeOutput::new(self.pending, ImageFormat::Pnm))
    }
}

/// P7/P8: borrowed raster copies and per-chunk finish-first staging objects.
fn row_encode_copies_and_staging() {
    let (width, height) = (256u32, 64u32);
    let data = gradient(width, height);
    let base = data.as_ptr() as usize;
    let end = base + data.len();
    let pixels = PixelSlice::new(
        &data,
        width,
        height,
        width as usize * 3,
        PixelDescriptor::RGB8_SRGB,
    )
    .expect("valid raster");
    for (case, objects) in [("objects_4096", 4096usize), ("objects_8", 8)] {
        let pushed = Rc::new(RefCell::new(Vec::new()));
        let encoder = Recorder {
            pushed: pushed.clone(),
            pending: Vec::new(),
            chunk: 1,
        };
        let options = Options::new(StorageBudget::new(1 << 20, objects), 1 << 20)
            .with_batch_rows(NonZeroU32::new(1).expect("nonzero"));
        let mut out = Vec::new();
        let (result, blocks, bytes) = heap(|| {
            row_encode::encode(
                encoder,
                Raster::new(pixels.clone()),
                &mut out,
                PolicySet::new(),
                options,
                STOP,
            )
        });
        let pushed = pushed.borrow();
        let borrowed = pushed
            .iter()
            .filter(|(p, _)| *p >= base && *p < end)
            .count();
        let outcome = match &result {
            Ok(done) => format!("ok emitted={} schedule={:?}", done.emitted, done.schedule),
            Err(e) => format!(
                "err stage={:?} cause={:?}",
                e.error().stage,
                std::error::Error::source(e.error())
            ),
        };
        println!(
            "row_encode\t{case}\tpushes={}\tpushes_borrowing_caller_raster={borrowed}\traster_bytes={}\tallocs={blocks}\tbytes={bytes}\t{outcome}",
            pushed.len(),
            data.len()
        );
    }
}

/// O2: one reusable tenant preset applied to a plain encode.
#[allow(deprecated)] // Compare legacy policy placement with earlier probe runs.
fn tenant_preset_on_encode() {
    let (width, height) = (16u32, 16u32);
    let data = gradient(width, height);
    let pixels = PixelSlice::new(
        &data,
        width,
        height,
        width as usize * 3,
        PixelDescriptor::RGB8_SRGB,
    )
    .expect("valid raster");
    let codecs = registry(1);
    for (case, preset) in [
        ("default", PolicySet::new()),
        (
            "require_metadata_discard",
            PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
        ),
        (
            "forbid_opaque_profiles",
            PolicySet::new().allow_opaque_profiles(false),
        ),
        (
            "workspace_limit",
            PolicySet::new().with_workspace_limit(10 << 30),
        ),
    ] {
        let result = EncodeRequest::new(ImageFormat::Pnm)
            .with_policy_set(preset)
            .encode(&codecs, pixels.clone());
        let outcome = match &result {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("display=\"{}\" debug={:?}", e.error(), e.error()),
        };
        println!("tenant_preset_encode\t{case}\t{outcome}");
    }
    // The same requirement with a callback that selects instead of delegating.
    let request = EncodeRequest::new(ImageFormat::Pnm).with_policy_set(
        PolicySet::new()
            .require_metadata(MetadataTransfer::DiscardDescriptive)
            .allow_opaque_profiles(false)
            .with_workspace_limit(1),
    );
    let mut choose_first = |question: Question<'_>| -> Result<Decision, At<ProbeError>> {
        match question {
            Question::Encode { choices, .. } => Ok(choices
                .first()
                .map(Decision::choose)
                .unwrap_or(Decision::UsePreset)),
            _ => Ok(Decision::UsePreset),
        }
    };
    let result = request.encode_with(&codecs, pixels.clone(), &mut choose_first);
    let outcome = match &result {
        Ok(out) => format!(
            "ok used_preset={:?}",
            out.report()
                .decisions()
                .map(|d| d.used_preset)
                .collect::<Vec<_>>()
        ),
        Err(e) => format!("display=\"{}\"", e.error()),
    };
    println!("tenant_preset_encode\tcallback_chooses_with_unproven_requirements\t{outcome}");
}

type Resolved = Result<(usize, DecisionRecord), At<PolicyFailure>>;

fn label(result: &Resolved) -> String {
    match result {
        Ok((index, record)) => format!("selected index={index} used_preset={}", record.used_preset),
        Err(e) => format!(
            "refused code={:?} display=\"{}\"",
            e.error().record().code,
            e.error()
        ),
    }
}

/// Delegates every question to the configured preset.
fn delegate(_: Question<'_>) -> Result<Decision, At<ProbeError>> {
    Ok(Decision::UsePreset)
}

/// Selects the k-th offer of a transcode question directly.
fn choose_transcode(k: usize) -> impl FnMut(Question<'_>) -> Result<Decision, At<ProbeError>> {
    move |question: Question<'_>| match question {
        Question::Transcode { choices, .. } => Ok(choices
            .iter()
            .nth(k)
            .map(Decision::choose)
            .unwrap_or(Decision::UsePreset)),
        _ => Ok(Decision::UsePreset),
    }
}

/// Requirement enforcement across direct Choose, UsePreset, conflicting
/// metadata, unknown evidence, deferred discovery and an unrelated domain.
/// Drives the public resolve_* functions with constructed offers.
fn policy_matrix() {
    let codecs = registry(1);
    let encoder = EncodeRequest::new(ImageFormat::Pnm)
        .prepare(&codecs)
        .expect("prepared reference encoder")
        .remove(0)
        .offer();
    let route = |metadata, opaque: Option<bool>, workspace: Option<u64>| {
        let mut offer = TranscodeOffer::new(encoder, PixelDescriptor::RGB8_SRGB, metadata);
        offer.retains_opaque_profile = opaque;
        offer.workspace_upper_bound = workspace;
        offer
    };
    let compliant = route(
        MetadataTransfer::DiscardDescriptive,
        Some(false),
        Some(1 << 20),
    );
    let conflicting = route(
        MetadataTransfer::PreserveReported,
        Some(false),
        Some(1 << 20),
    );
    let unknown = route(MetadataTransfer::DiscardDescriptive, None, None);
    let strict = PolicySet::new()
        .require_metadata(MetadataTransfer::DiscardDescriptive)
        .allow_opaque_profiles(false)
        .with_workspace_limit(1 << 30);
    let boundary = |state| {
        DecodeBoundary::new(InterpretationFacts::new(
            16,
            16,
            Some(PixelDescriptor::RGB8_SRGB),
            state,
        ))
        .expect("valid boundary")
    };
    let stop = STOP;
    let cases: [(&str, Vec<TranscodeOffer>); 4] = [
        ("compliant", vec![compliant]),
        ("conflicting_metadata", vec![conflicting]),
        ("unknown_evidence", vec![unknown]),
        ("noncompliant_then_compliant", vec![conflicting, compliant]),
    ];
    for (case, offers) in &cases {
        let final_boundary = boundary(InterpretationState::Final);
        let preset = resolve_transcode(final_boundary, offers, &mut delegate, strict, stop);
        let direct = resolve_transcode(
            final_boundary,
            offers,
            &mut choose_transcode(0),
            strict,
            stop,
        );
        println!(
            "policy_matrix\ttranscode/{case}\tuse_preset: {}",
            label(&preset)
        );
        println!(
            "policy_matrix\ttranscode/{case}\tdirect_choose_first: {}",
            label(&direct)
        );
    }
    // Deferred discovery: requirements must not stop non-final reading.
    for state in [
        InterpretationState::Pending,
        InterpretationState::Provisional,
    ] {
        let result = resolve_boundary(boundary(state), &mut delegate, strict, stop);
        let action = result.as_ref().ok().map(|(action, _)| *action);
        let result = result.map(|(_, record)| (0, record));
        println!(
            "policy_matrix\tboundary/{state:?}\tuse_preset: {} action={action:?}",
            label(&result)
        );
    }
    // Unrelated domain: a destination schedule question under a metadata
    // requirement that this question neither proves nor affects.
    let mut finish_first = OutputScheduleOffer::new(OutputSchedule::FinishFirst);
    finish_first.wait_retained_upper_bound = Some(1 << 20);
    let facts = OutputScheduleFacts::new(OutputTiming::AT_FINISH, OutputSource::PullRows);
    for (case, preset) in [
        (
            "metadata_requirement",
            PolicySet::new().require_metadata(MetadataTransfer::DiscardDescriptive),
        ),
        (
            "wait_limit_proven",
            PolicySet::new().with_output_wait_limit(10 << 30),
        ),
    ] {
        let result = resolve_output_schedule(facts, &[finish_first], &mut delegate, preset, stop);
        println!(
            "policy_matrix\toutput_schedule/{case}\tuse_preset: {}",
            label(&result)
        );
    }
    let unknown_wait = OutputScheduleOffer::new(OutputSchedule::FinishFirst);
    let result = resolve_output_schedule(
        facts,
        &[unknown_wait],
        &mut delegate,
        PolicySet::new().with_output_wait_limit(10 << 30),
        stop,
    );
    println!(
        "policy_matrix\toutput_schedule/wait_limit_unknown\tuse_preset: {}",
        label(&result)
    );
}

fn main() {
    let profiler = dhat::Profiler::builder().testing().build();
    registry_clone();
    if std::env::args().any(|arg| arg == "--registry-only") {
        std::mem::forget(profiler);
        return;
    }
    required_descriptor();
    row_encode_copies_and_staging();
    tenant_preset_on_encode();
    policy_matrix();
    // Testing mode keeps the counters; skip writing a profile file.
    std::mem::forget(profiler);
}
