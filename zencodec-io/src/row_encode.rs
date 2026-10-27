//! Owned row-source encoding with policy before final-destination effects.
//!
//! Finish-first is the default offer. It stages compressed bytes under the
//! supplied storage budget, releases the source and encoder, then writes.
//! Live output is offered only when begin reports streaming; it does not imply
//! a memory bound. No native workspace or source allocation bound is inferred.
//! Failures consume the operation, retain the selected decision and report
//! confirmed destination appends. Partial output is never retried.
//!
//! ```
//! use zencodec::{encode::Encoder, operation::PolicySet, storage::StorageBudget};
//! use zencodec_io::row_encode::{self, Options, Raster};
//! use zenpixels::PixelSlice;
//! fn save<E: Encoder, W: std::io::Write>(
//!     encoder: E, pixels: PixelSlice<'_>, writer: W,
//! ) -> Result<row_encode::Completed, Box<dyn core::error::Error>> {
//!     Ok(row_encode::encode(
//!         encoder, Raster::new(pixels), writer, PolicySet::new(),
//!         Options::new(StorageBudget::new(64 * 1024 * 1024, 4096), 64 * 1024 * 1024),
//!         &zencodec::how_far::NoPulse,
//!     )?)
//! }
//! ```
use crate::destination::{Destination, Forward};
use alloc::{boxed::Box, collections::VecDeque, sync::Arc};
use core::{
    error::Error,
    fmt,
    num::{NonZeroU32, NonZeroUsize},
};

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
};
use zencodec::output_patch::OutputAccess;
use zencodec::{
    At,
    encode::{
        EncodeDeclaration, EncodeJob, EncodePlan, EncodePlanError, Encoder, InputRequirement,
    },
    operation::{
        OperationReport, OutputSchedule, OutputScheduleFacts, OutputScheduleOffer, OutputSource,
        PolicyResolver, PolicySet, StagingFacts, StagingKind, resolve_output_schedule,
    },
    storage::{OwnedStorage, StorageBudget},
};
use zenpixels::{ColorContext, PixelDescriptor, PixelSlice, PixelSliceMut};

/// Fixed sample declaration for this operation. Color conversion is not implicit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Description {
    /// Width in samples.
    pub width: u32,
    /// Total rows.
    pub height: u32,
    /// Complete sample descriptor accepted by the configured encoder.
    pub descriptor: PixelDescriptor,
    /// Original color context carried on every source and encoder row view.
    pub color_context: Option<Arc<ColorContext>>,
}
impl Description {
    /// Construct a declaration; execution checks dimensions and size arithmetic.
    pub const fn new(width: u32, height: u32, descriptor: PixelDescriptor) -> Self {
        Self {
            width,
            height,
            descriptor,
            color_context: None,
        }
    }
    /// Attach the actual sample context without changing the descriptor.
    pub fn with_color_context(mut self, context: Arc<ColorContext>) -> Self {
        self.color_context = Some(context);
        self
    }
}
/// A fixed-interpretation source, retained across policy and read on demand.
pub trait Source {
    /// Original source error; retained in the operation's error chain.
    type Error: Error + Send + Sync + 'static;
    /// Fixed declaration. Changing it during read fails before the next encode.
    fn description(&self) -> Description;
    /// Acquisition mechanism, not a retained-memory guarantee.
    fn mechanism(&self) -> OutputSource {
        OutputSource::PullRows
    }
    /// Fill every active sample of the requested consecutive rows, honoring
    /// destination stride. No reading beyond the declared image is requested.
    /// A late interpretation change must fail, never relabel earlier samples.
    fn read_rows(
        &mut self,
        first: u32,
        rows: PixelSliceMut<'_>,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error>;
}
/// Borrow a complete raster through the same row-source interface.
pub struct Raster<'a>(PixelSlice<'a>);
impl<'a> Raster<'a> {
    /// Retain the caller's raster borrow; source padding is not copied.
    pub const fn new(pixels: PixelSlice<'a>) -> Self {
        Self(pixels)
    }
}
impl Source for Raster<'_> {
    type Error = zencodec::CodecError;
    fn description(&self) -> Description {
        let mut desc = Description::new(self.0.width(), self.0.rows(), self.0.descriptor());
        desc.color_context = self.0.color_context().cloned();
        desc
    }
    fn mechanism(&self) -> OutputSource {
        OutputSource::BorrowedRaster
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: PixelSliceMut<'_>,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<(), Self::Error> {
        if rows.width() != self.0.width()
            || rows.descriptor() != self.0.descriptor()
            || rows.color_context() != self.0.color_context()
            || first
                .checked_add(rows.rows())
                .is_none_or(|end| end > self.0.rows())
        {
            return Err(zencodec::CodecError::new(
                None,
                zencodec::InvalidKind::Buffer.into(),
            ));
        }
        for y in 0..rows.rows() {
            pulse
                .check()
                .map_err(|r| zencodec::CodecError::new(None, r.into()))?;
            rows.row_mut(y).copy_from_slice(self.0.row(first + y));
        }
        Ok(())
    }
}
/// Managed payload storage and output limits. Neither bounds native allocations
/// or queue bookkeeping.
pub struct Options {
    storage: StorageBudget,
    max_output_bytes: u64,
    batch_rows: NonZeroU32,
    preset: PolicySet,
    disk: Option<Disk>,
}
impl Options {
    /// Row buffers and compressed staging share this admission pool.
    /// Zero output allowance is a real limit; it is not an unlimited sentinel.
    pub fn new(storage: StorageBudget, max_output_bytes: u64) -> Self {
        Self {
            storage,
            max_output_bytes,
            batch_rows: NonZeroU32::new(16).unwrap(),
            preset: PolicySet::new(),
            disk: None,
        }
    }
    /// Stage finish-first output in an owned private scratch file at its end.
    /// Existing prefixes are preserved. The output limit also caps disk payload.
    /// Replay admits at most `replay_bytes` from the shared storage budget.
    /// Spool writes may block with native state alive. The caller owns path,
    /// permissions and deletion; aliases/external mutation are unsupported.
    /// No fallback, file creation or deletion occurs. Live selection ignores
    /// the scratch file and writes directly to the final destination.
    pub fn with_disk_staging(
        mut self,
        mut file: File,
        replay_bytes: NonZeroUsize,
    ) -> io::Result<Self> {
        let origin = file.stream_position()?;
        if file.seek(SeekFrom::End(0))? != origin {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "scratch must start at its end",
            ));
        }
        self.disk = Some(Disk {
            file,
            origin,
            replay_bytes,
        });
        Ok(self)
    }
    /// Maximum rows requested at once; native row preferences do not override it.
    pub fn with_batch_rows(mut self, rows: NonZeroU32) -> Self {
        self.batch_rows = rows;
        self
    }
    /// Built-in policy used when the supplied callback returns UsePreset.
    pub fn with_policy_set(mut self, preset: PolicySet) -> Self {
        self.preset = preset;
        self
    }
}
/// Operation boundary that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// Validate declarations, begin the encoder and admit row storage.
    Prepare,
    /// Select the actual output schedule.
    Policy,
    /// Request source samples or detect changed interpretation.
    Read,
    /// Encode rows or admit pending compressed storage.
    Encode,
    /// Finish and release the encoder.
    Finish,
    /// Flush or read staged compressed output, or admit its replay buffer.
    Replay,
    /// Write or flush the final destination.
    Write,
    /// Seek or overwrite final patches, including restoration to the output end.
    Patch,
    /// Construct and start the admitted job, or reject a changed native plan.
    Start,
}
/// Driver contract refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// Empty geometry is not a raster.
    Empty,
    /// Size arithmetic overflow.
    Overflow,
    /// Source declaration changed after preparation.
    SourceChanged,
    /// Total encoded bytes exceed the supplied output allowance.
    OutputLimit,
    /// Layout requires unavailable or unannounced seek-back writes.
    Patches,
    /// Destination access changed while policy was deciding.
    DestinationChanged,
}
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "row encoding refused: {self:?}")
    }
}
impl Error for Refusal {}
type Cause = Box<dyn Error + Send + Sync>;
/// Terminal error. Default formatting omits backend payloads and source data.
pub struct Failure {
    /// Boundary that failed.
    pub stage: Stage,
    /// Confirmed destination appends, including partial writes.
    pub emitted: u64,
    /// Confirmed overwritten bytes, separate from appended extent.
    pub patched_bytes: u64,
    /// Bounded policy history, including after later source/codec/I/O failure.
    pub report: Box<OperationReport>,
    cause: Cause,
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowEncodeFailure")
            .field("stage", &self.stage)
            .field("emitted", &self.emitted)
            .field("patched_bytes", &self.patched_bytes)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "row encoding failed during {:?} after {} appended bytes",
            self.stage, self.emitted
        )
    }
}
impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
/// Successful destination completion and bounded decision history.
#[derive(Debug)]
pub struct Completed {
    /// Confirmed appended bytes.
    pub emitted: u64,
    /// Confirmed overwritten bytes, separate from appended extent.
    pub patched_bytes: u64,
    /// Executed schedule.
    pub schedule: OutputSchedule,
    /// Decisions made before source reads and final output writes.
    pub report: OperationReport,
}
fn check(pulse: &dyn zencodec::Pulse) -> Result<(), Cause> {
    pulse
        .check()
        .map_err(|r| At::wrap(zencodec::CodecError::new(None, r.into())).into())
}
fn validate_source<S: Source>(
    source: &S,
    desc: &Description,
    mechanism: OutputSource,
) -> Result<(), Cause> {
    if source.description() != *desc || source.mechanism() != mechanism {
        return Err(Refusal::SourceChanged.into());
    }
    Ok(())
}
fn count(total: &mut u64, len: usize, limit: u64) -> Result<(), Cause> {
    let next = total.checked_add(len as u64).ok_or(Refusal::Overflow)?;
    if next > limit {
        return Err(Refusal::OutputLimit.into());
    }
    *total = next;
    Ok(())
}
fn write_all(
    writer: &mut impl Write,
    mut bytes: &[u8],
    emitted: &mut u64,
    pulse: &dyn zencodec::Pulse,
) -> Result<(), Cause> {
    while !bytes.is_empty() {
        check(pulse)?;
        match writer.write(bytes) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
            Ok(n) if n <= bytes.len() => {
                *emitted += n as u64;
                bytes = &bytes[n..];
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "writer returned an invalid count",
                )
                .into());
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    check(pulse)
}
struct Disk {
    file: File,
    origin: u64,
    replay_bytes: NonZeroUsize,
}
impl Disk {
    fn append(&mut self, bytes: &[u8], pulse: &dyn zencodec::Pulse) -> Result<(), Cause> {
        let mut written = 0;
        write_all(&mut self.file, bytes, &mut written, pulse)
    }
    fn replay(
        &mut self,
        total: u64,
        storage: &StorageBudget,
        writer: &mut impl Write,
        emitted: &mut u64,
        boundary: &mut Stage,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<(), Cause> {
        *boundary = Stage::Replay;
        check(pulse)?;
        self.file.flush()?;
        self.file.seek(SeekFrom::Start(self.origin))?;
        let size = total.min(self.replay_bytes.get() as u64) as usize;
        let mut buffer = storage.bytes(size)?;
        let mut remaining = total;
        while remaining != 0 {
            *boundary = Stage::Replay;
            check(pulse)?;
            let n = remaining.min(size as u64) as usize;
            self.file.read_exact(&mut buffer.as_mut_slice()[..n])?;
            check(pulse)?;
            *boundary = Stage::Write;
            write_all(writer, &buffer.as_slice()[..n], emitted, pulse)?;
            remaining -= n as u64;
        }
        Ok(())
    }
}
fn stage(
    bytes: &[u8],
    storage: &StorageBudget,
    packets: &mut VecDeque<OwnedStorage<u8>>,
) -> Result<(), Cause> {
    if !bytes.is_empty() {
        packets.try_reserve(1)?;
        let mut owned = storage.bytes(bytes.len())?;
        owned.as_mut_slice().copy_from_slice(bytes);
        packets.push_back(owned);
    }
    Ok(())
}
/// Encode a retained fixed-interpretation source through a prepared schedule.
///
/// Default policy chooses finish-first. No row-level refusal falls back to
/// whole-image allocation. Policy runs once before reading source rows or
/// publishing the header. The encoder, source, storage and writer stay retained
/// across that decision. Unknown total wait retention is reported honestly.
///
/// Finish-first admits compressed chunks as they appear and drops source,
/// row buffer and encoder before destination writes. Source I/O and explicit
/// staging can still wait while native state is alive. Live mode pauses source
/// reads during synchronous writes; this alone proves no native memory bound.
/// Cancellation is checked around native calls and each writer operation; it
/// cannot interrupt a blocking OS write. No retries or implicit finish on drop.
pub fn encode<E: Encoder, S: Source, W: Write, R: PolicyResolver>(
    encoder: E,
    source: S,
    writer: W,
    resolver: R,
    options: Options,
    pulse: &dyn zencodec::Pulse,
) -> Result<Completed, At<Failure>> {
    encode_to_destination(encoder, source, Forward(writer), resolver, options, pulse)
}

/// Encode to an actual forward or seek-back destination. Layout admission occurs
/// before policy, source reads and output publication. Patches are fully checked
/// before final tail writes. I/O failure can leave partial overwrites; their
/// confirmed byte count is distinct from appended extent. No automatic spooling
/// converts a forward destination into a seekable one.
pub fn encode_to_destination<E: Encoder, S: Source, D: Destination, R: PolicyResolver>(
    encoder: E,
    source: S,
    writer: D,
    resolver: R,
    options: Options,
    pulse: &dyn zencodec::Pulse,
) -> Result<Completed, At<Failure>> {
    run_rows(
        LegacyStartup(encoder),
        source,
        writer,
        resolver,
        options,
        pulse,
    )
}

/// Encode a configured job to a writer, admitting its plan before native startup.
///
/// Unlike the raw-encoder adapter, this retains the lightweight job across policy
/// and supplies current color before header creation. Unsupported preflight or
/// non-row input refuses before encoder construction; there is no fallback.
/// Use `options.with_policy_set` for mandatory schedule/wait requirements. This
/// primitive does not add or infer metadata/fidelity requirements on the job.
/// The complete checked request-to-destination convenience is a separate layer.
pub fn encode_job<J: EncodeJob, S: Source, W: Write, R: PolicyResolver>(
    job: J,
    source: S,
    writer: W,
    resolver: R,
    options: Options,
    pulse: &dyn zencodec::Pulse,
) -> Result<Completed, At<Failure>>
where
    J::Enc: Encoder,
{
    encode_job_to_destination(job, source, Forward(writer), resolver, options, pulse)
}

/// Execute a retained job's declared row route against actual destination access.
///
/// Sequence: plan without native startup, admit policy, revalidate the retained
/// source/destination, reserve driver rows, construct once, begin with current
/// color, compare the complete plan, then read/publish. A refusal or startup
/// failure never retries another route. Strict unknown workspace/wait evidence
/// refuses; byte timing alone never establishes either bound.
///
/// This is an adapter primitive for an already configured job. It does not
/// select registry configurations or certify descriptive metadata treatment.
pub fn encode_job_to_destination<J: EncodeJob, S: Source, D: Destination, R: PolicyResolver>(
    job: J,
    source: S,
    writer: D,
    resolver: R,
    options: Options,
    pulse: &dyn zencodec::Pulse,
) -> Result<Completed, At<Failure>>
where
    J::Enc: Encoder,
{
    run_rows(
        PlannedStartup(job),
        source,
        writer,
        resolver,
        options,
        pulse,
    )
}

trait RowStartup {
    type Enc: Encoder;
    // Preserve legacy allocation/start ordering on the existing raw adapter.
    const EARLY_BUFFER: bool;
    fn plan(&mut self, declaration: &EncodeDeclaration<'_>) -> Result<EncodePlan, Cause>;
    fn start(
        self,
        declaration: &EncodeDeclaration<'_>,
        plan: &EncodePlan,
    ) -> Result<Self::Enc, Cause>;
}
struct LegacyStartup<E>(E);
impl<E: Encoder> RowStartup for LegacyStartup<E> {
    type Enc = E;
    const EARLY_BUFFER: bool = true;
    fn plan(&mut self, declaration: &EncodeDeclaration<'_>) -> Result<EncodePlan, Cause> {
        let timing = self.0.begin_output(
            declaration.width,
            declaration.height,
            declaration.descriptor,
            declaration.access,
        )?;
        Ok(EncodePlan::rows(timing))
    }
    fn start(self, _: &EncodeDeclaration<'_>, _: &EncodePlan) -> Result<E, Cause> {
        Ok(self.0)
    }
}
struct PlannedStartup<J>(J);
impl<J: EncodeJob> RowStartup for PlannedStartup<J>
where
    J::Enc: Encoder,
{
    type Enc = J::Enc;
    const EARLY_BUFFER: bool = false;
    fn plan(&mut self, declaration: &EncodeDeclaration<'_>) -> Result<EncodePlan, Cause> {
        let plan = self.0.plan_encode(declaration)?;
        plan.validate(declaration)?;
        if plan.input != InputRequirement::Rows {
            return Err(EncodePlanError::InputProtocol.into());
        }
        Ok(plan)
    }
    fn start(
        self,
        declaration: &EncodeDeclaration<'_>,
        plan: &EncodePlan,
    ) -> Result<Self::Enc, Cause> {
        let mut encoder = self.0.encoder()?;
        let observed = encoder.begin_declared(declaration)?;
        plan.validate_started(&observed)?;
        Ok(encoder)
    }
}

fn run_rows<P: RowStartup, S: Source, D: Destination, R: PolicyResolver>(
    mut startup: P,
    mut source: S,
    mut writer: D,
    mut resolver: R,
    mut options: Options,
    pulse: &dyn zencodec::Pulse,
) -> Result<Completed, At<Failure>> {
    let mut boundary = Stage::Prepare;
    let mut emitted = 0;
    let mut patched_bytes = 0;
    let mut report = OperationReport::default();
    let run = || -> Result<OutputSchedule, Cause> {
        check(pulse)?;
        let desc = source.description();
        let mechanism = source.mechanism();
        if desc.width == 0 || desc.height == 0 {
            return Err(Refusal::Empty.into());
        }
        let stride = (desc.width as usize)
            .checked_mul(desc.descriptor.bytes_per_pixel())
            .ok_or(Refusal::Overflow)?;
        let rows = options.batch_rows.get().min(desc.height);
        let len = stride.checked_mul(rows as usize).ok_or(Refusal::Overflow)?;
        let early_buffer = if P::EARLY_BUFFER {
            Some(options.storage.bytes(len)?)
        } else {
            None
        };
        let access = if writer.can_patch() {
            OutputAccess::SeekBack
        } else {
            OutputAccess::Forward
        };
        let mut declaration =
            EncodeDeclaration::new(desc.width, desc.height, desc.descriptor).with_access(access);
        if let Some(color) = &desc.color_context {
            declaration = declaration.with_color(color);
        }
        let plan = startup.plan(&declaration)?;
        let timing = plan.timing;
        if timing.patches_at_finish() && !writer.can_patch() {
            return Err(Refusal::Patches.into());
        }
        check(pulse)?;
        let mut finish_first = OutputScheduleOffer::new(OutputSchedule::FinishFirst);
        finish_first.staging = Some(StagingFacts::new(
            if options.disk.is_some() {
                StagingKind::Disk
            } else {
                StagingKind::Memory
            },
            0,
            if options.disk.is_some() {
                options.max_output_bytes
            } else {
                options.storage.max_bytes() as u64
            },
        ));
        let offers = [finish_first, OutputScheduleOffer::new(OutputSchedule::Live)];
        let offers = &offers[..if timing.streams() { 2 } else { 1 }];
        boundary = Stage::Policy;
        let (index, record) = match resolve_output_schedule(
            OutputScheduleFacts::new(timing, mechanism),
            offers,
            &mut resolver,
            options.preset,
            pulse,
        ) {
            Ok(selected) => selected,
            Err(error) => {
                report.record(error.error().record());
                return Err(error.into());
            }
        };
        report.record(record);
        let schedule = offers[index].schedule;
        check(pulse)?;
        boundary = Stage::Read;
        validate_source(&source, &desc, mechanism)?;
        let current_access = if writer.can_patch() {
            OutputAccess::SeekBack
        } else {
            OutputAccess::Forward
        };
        if current_access != access {
            return Err(Refusal::DestinationChanged.into());
        }
        boundary = Stage::Prepare;
        let mut buffer = match early_buffer {
            Some(buffer) => buffer,
            None => options.storage.bytes(len)?,
        };
        check(pulse)?;
        boundary = Stage::Start;
        let mut encoder = startup.start(&declaration, &plan)?;
        // Exact carrier evidence can own ICC bytes. Release it before final
        // destination waits; the executing encoder owns its admitted metadata.
        drop(plan);
        let mut staged = VecDeque::new();
        let mut produced = 0;
        let mut y = 0;
        loop {
            check(pulse)?;
            boundary = Stage::Read;
            validate_source(&source, &desc, mechanism)?;
            // The final pending bytes belong to finish. Do not introduce an
            // avoidable writer wait just before releasing native/source state.
            if y == desc.height {
                break;
            }
            boundary = Stage::Encode;
            let pending = encoder.pending_output();
            count(&mut produced, pending.len(), options.max_output_bytes)?;
            if schedule == OutputSchedule::Live {
                boundary = Stage::Write;
                write_all(&mut writer, pending, &mut emitted, pulse)?;
            } else {
                boundary = Stage::Encode;
                if let Some(disk) = &mut options.disk {
                    disk.origin.checked_add(produced).ok_or(Refusal::Overflow)?;
                    disk.append(pending, pulse)?;
                } else {
                    stage(pending, &options.storage, &mut staged)?;
                }
            }
            encoder.consume_output(pending.len());
            let count = rows.min(desc.height - y);
            let size = stride
                .checked_mul(count as usize)
                .ok_or(Refusal::Overflow)?;
            boundary = Stage::Read;
            validate_source(&source, &desc, mechanism)?;
            let mut destination = PixelSliceMut::new(
                &mut buffer.as_mut_slice()[..size],
                desc.width,
                count,
                stride,
                desc.descriptor,
            )?;
            if let Some(context) = &desc.color_context {
                destination = destination.with_color_context(context.clone());
            }
            source.read_rows(y, destination, pulse)?;
            check(pulse)?;
            validate_source(&source, &desc, mechanism)?;
            boundary = Stage::Encode;
            let mut pixels = PixelSlice::new(
                &buffer.as_slice()[..size],
                desc.width,
                count,
                stride,
                desc.descriptor,
            )?;
            if let Some(context) = &desc.color_context {
                pixels = pixels.with_color_context(context.clone());
            }
            encoder.push_rows(pixels)?;
            y += count;
        }
        drop(source);
        drop(buffer);
        drop(desc);
        boundary = Stage::Finish;
        check(pulse)?;
        let finished = encoder.finish_output()?;
        check(pulse)?;
        finished.validate(produced)?;
        if !finished.patches.is_empty() && (!writer.can_patch() || !timing.patches_at_finish()) {
            return Err(Refusal::Patches.into());
        }
        let patches = finished.patches;
        let rest = finished.tail;
        count(&mut produced, rest.len(), options.max_output_bytes)?;
        if schedule == OutputSchedule::FinishFirst {
            if let Some(disk) = &mut options.disk {
                disk.origin.checked_add(produced).ok_or(Refusal::Overflow)?;
                disk.append(rest.data(), pulse)?;
                drop(rest);
                disk.replay(
                    produced,
                    &options.storage,
                    &mut writer,
                    &mut emitted,
                    &mut boundary,
                    pulse,
                )?;
            } else {
                stage(rest.data(), &options.storage, &mut staged)?;
                drop(rest);
                boundary = Stage::Write;
                while let Some(bytes) = staged.pop_front() {
                    write_all(&mut writer, bytes.as_slice(), &mut emitted, pulse)?;
                }
            }
        } else {
            boundary = Stage::Write;
            write_all(&mut writer, rest.data(), &mut emitted, pulse)?;
        }
        check(pulse)?;
        if !patches.is_empty() {
            boundary = Stage::Patch;
            for patch in patches {
                check(pulse)?;
                writer.position(patch.offset)?;
                write_all(&mut writer, &patch.bytes, &mut patched_bytes, pulse)?;
            }
            writer.position(produced)?;
        }
        boundary = Stage::Write;
        writer.flush()?;
        check(pulse)?;
        Ok(schedule)
    };
    match run() {
        Ok(schedule) => Ok(Completed {
            emitted,
            patched_bytes,
            schedule,
            report,
        }),
        Err(cause) => Err(At::wrap(Failure {
            stage: boundary,
            emitted,
            patched_bytes,
            report: Box::new(report),
            cause,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_honors_both_strides_and_refuses_context_mismatch_before_copy() {
        let input = [1, 2, 3, 99, 98, 97, 4, 5, 6];
        let context = Arc::new(ColorContext::from_cicp(zencodec::Cicp::SRGB));
        let pixels = PixelSlice::new(&input, 1, 2, 6, PixelDescriptor::RGB8_SRGB)
            .unwrap()
            .with_color_context(context.clone());
        let mut source = Raster::new(pixels);
        let mut output = [77; 12];
        let view = PixelSliceMut::new(&mut output, 1, 2, 9, PixelDescriptor::RGB8_SRGB).unwrap();
        assert!(
            source
                .read_rows(0, view, &zencodec::how_far::NoPulse)
                .is_err()
        );
        assert_eq!(output, [77; 12]);
        let view = PixelSliceMut::new(&mut output, 1, 2, 9, PixelDescriptor::RGB8_SRGB)
            .unwrap()
            .with_color_context(context);
        source
            .read_rows(0, view, &zencodec::how_far::NoPulse)
            .unwrap();
        assert_eq!(output, [1, 2, 3, 77, 77, 77, 77, 77, 77, 4, 5, 6]);
    }
}

#[cfg(test)]
mod patch_tests {
    use super::*;
    use crate::destination::SeekBack;
    use zencodec::{
        encode::{EncodeOutput, OutputTiming},
        output_patch::{OutputFinish, Patch, PatchError},
    };
    struct Patching {
        pending: bool,
        bad: bool,
    }
    impl Encoder for Patching {
        type Error = io::Error;
        fn reject(op: zencodec::UnsupportedOperation) -> Self::Error {
            io::Error::other(op)
        }
        fn encode(self, _: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error> {
            Err(io::Error::other("negotiated path only"))
        }
        fn begin_output(
            &mut self,
            _: u32,
            _: u32,
            _: PixelDescriptor,
            _: OutputAccess,
        ) -> Result<OutputTiming, Self::Error> {
            self.pending = true;
            Ok(OutputTiming::streaming(0).with_patches())
        }
        fn pending_output(&self) -> &[u8] {
            if self.pending { &[0, 0] } else { &[] }
        }
        fn consume_output(&mut self, _: usize) {
            self.pending = false;
        }
        fn push_rows(&mut self, _: PixelSlice<'_>) -> Result<(), Self::Error> {
            Ok(())
        }
        fn finish_output(self) -> Result<OutputFinish<EncodeOutput>, Self::Error> {
            Ok(OutputFinish::new(
                EncodeOutput::new(alloc::vec![3], zencodec::ImageFormat::Unknown),
                alloc::vec![Patch {
                    offset: if self.bad { u64::MAX } else { 0 },
                    bytes: alloc::vec![1, 2],
                }],
            ))
        }
    }
    struct Short {
        cursor: io::Cursor<alloc::vec::Vec<u8>>,
        patch: bool,
        fail: bool,
    }
    impl Write for Short {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.patch && self.fail && self.cursor.position() == 7 {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            self.cursor.write(&bytes[..bytes.len().min(1)])
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Seek for Short {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            if matches!(from, SeekFrom::Start(6)) {
                self.patch = true;
            }
            self.cursor.seek(from)
        }
    }
    fn source() -> Raster<'static> {
        Raster::new(PixelSlice::new(&[7, 8, 9], 1, 1, 3, PixelDescriptor::RGB8_SRGB).unwrap())
    }
    fn options() -> Options {
        Options::new(StorageBudget::new(1024, 16), 1024)
    }
    #[test]
    fn incompatible_destination_refuses_before_policy_or_any_output() {
        let mut bytes = alloc::vec::Vec::new();
        let error =
            encode(
                Patching {
                    pending: false,
                    bad: false,
                },
                source(),
                &mut bytes,
                |_: zencodec::operation::Question<'_>| -> Result<
                    zencodec::operation::Decision,
                    zencodec::At<core::convert::Infallible>,
                > { panic!("must refuse before policy") },
                options(),
                &zencodec::how_far::NoPulse,
            )
            .unwrap_err();
        assert!(bytes.is_empty());
        assert_eq!(
            zencodec::find_cause::<Refusal>(&error),
            Some(&Refusal::Patches)
        );
        assert_eq!(error.error().emitted, 0);
    }
    #[test]
    fn patches_preflight_all_ranges_and_report_partial_overwrites_separately() {
        for schedule in [OutputSchedule::FinishFirst, OutputSchedule::Live] {
            for (bad, fail) in [(false, false), (true, false), (false, true)] {
                let mut writer = Short {
                    cursor: io::Cursor::new(b"prefix".to_vec()),
                    patch: false,
                    fail,
                };
                writer.cursor.set_position(6);
                let result = encode_to_destination(
                    Patching {
                        pending: false,
                        bad,
                    },
                    source(),
                    SeekBack::new(&mut writer).unwrap(),
                    PolicySet::new().require_output_schedule(schedule),
                    options(),
                    &zencodec::how_far::NoPulse,
                );
                assert_eq!(&writer.cursor.get_ref()[..6], b"prefix");
                if bad {
                    let error = result.unwrap_err();
                    assert_eq!(
                        zencodec::find_cause::<PatchError>(&error),
                        Some(&PatchError::OutOfRange)
                    );
                    let prior = if schedule == OutputSchedule::Live {
                        2
                    } else {
                        0
                    };
                    assert_eq!(error.error().emitted, prior);
                    assert_eq!(writer.cursor.get_ref().len(), 6 + prior as usize);
                    assert_eq!(error.error().patched_bytes, 0);
                } else if fail {
                    let error = result.unwrap_err();
                    assert_eq!(error.error().stage, Stage::Patch);
                    assert_eq!(error.error().emitted, 3);
                    assert_eq!(error.error().patched_bytes, 1);
                    assert_eq!(
                        zencodec::find_cause::<io::Error>(&error).unwrap().kind(),
                        io::ErrorKind::BrokenPipe
                    );
                    assert_eq!(&writer.cursor.get_ref()[6..], &[1, 0, 3]);
                } else {
                    let completed = result.unwrap();
                    assert_eq!(completed.emitted, 3);
                    assert_eq!(completed.patched_bytes, 2);
                    assert_eq!(writer.cursor.position(), 9);
                    assert_eq!(&writer.cursor.get_ref()[6..], &[1, 2, 3]);
                }
            }
        }
    }
}
