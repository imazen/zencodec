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
//!         &enough::Unstoppable,
//!     )?)
//! }
//! ```
use alloc::{boxed::Box, collections::VecDeque, sync::Arc};
use core::{error::Error, fmt, num::NonZeroU32};
use enough::Stop;
use std::io::{self, Write};
use zencodec::{
    At,
    encode::Encoder,
    operation::{
        OperationReport, OutputSchedule, OutputScheduleFacts, OutputScheduleOffer, OutputSource,
        PolicyResolver, PolicySet, resolve_output_schedule,
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
        stop: &dyn Stop,
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
        stop: &dyn Stop,
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
            stop.check()
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
        }
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
    /// Write or flush the final destination.
    Write,
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
    /// Bounded policy history, including after later source/codec/I/O failure.
    pub report: Box<OperationReport>,
    cause: Cause,
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowEncodeFailure")
            .field("stage", &self.stage)
            .field("emitted", &self.emitted)
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
    /// Executed schedule.
    pub schedule: OutputSchedule,
    /// Decisions made before source reads and final output writes.
    pub report: OperationReport,
}
fn check(stop: &dyn Stop) -> Result<(), Cause> {
    stop.check()
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
    stop: &dyn Stop,
) -> Result<(), Cause> {
    while !bytes.is_empty() {
        check(stop)?;
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
    check(stop)
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
    mut encoder: E,
    mut source: S,
    mut writer: W,
    mut resolver: R,
    options: Options,
    stop: &dyn Stop,
) -> Result<Completed, At<Failure>> {
    let mut boundary = Stage::Prepare;
    let mut emitted = 0;
    let mut report = OperationReport::default();
    let run = || -> Result<OutputSchedule, Cause> {
        check(stop)?;
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
        let mut buffer = options.storage.bytes(len)?;
        let timing = encoder.begin(desc.width, desc.height, desc.descriptor)?;
        check(stop)?;
        let offers = [
            OutputScheduleOffer::new(OutputSchedule::FinishFirst),
            OutputScheduleOffer::new(OutputSchedule::Live),
        ];
        let offers = &offers[..if timing.streams() { 2 } else { 1 }];
        boundary = Stage::Policy;
        let (index, record) = match resolve_output_schedule(
            OutputScheduleFacts::new(timing, mechanism),
            offers,
            &mut resolver,
            options.preset,
            stop,
        ) {
            Ok(selected) => selected,
            Err(error) => {
                report.record(error.error().record());
                return Err(error.into());
            }
        };
        report.record(record);
        let schedule = offers[index].schedule;
        let mut staged = VecDeque::new();
        let mut produced = 0;
        let mut y = 0;
        loop {
            check(stop)?;
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
                write_all(&mut writer, pending, &mut emitted, stop)?;
            } else {
                boundary = Stage::Encode;
                stage(pending, &options.storage, &mut staged)?;
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
            source.read_rows(y, destination, stop)?;
            check(stop)?;
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
        check(stop)?;
        let rest = encoder.finish()?;
        check(stop)?;
        count(&mut produced, rest.len(), options.max_output_bytes)?;
        if schedule == OutputSchedule::FinishFirst {
            stage(rest.data(), &options.storage, &mut staged)?;
            drop(rest);
            boundary = Stage::Write;
            while let Some(bytes) = staged.pop_front() {
                write_all(&mut writer, bytes.as_slice(), &mut emitted, stop)?;
            }
        } else {
            boundary = Stage::Write;
            write_all(&mut writer, rest.data(), &mut emitted, stop)?;
        }
        check(stop)?;
        writer.flush()?;
        check(stop)?;
        Ok(schedule)
    };
    match run() {
        Ok(schedule) => Ok(Completed {
            emitted,
            schedule,
            report,
        }),
        Err(cause) => Err(At::wrap(Failure {
            stage: boundary,
            emitted,
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
        assert!(source.read_rows(0, view, &enough::Unstoppable).is_err());
        assert_eq!(output, [77; 12]);
        let view = PixelSliceMut::new(&mut output, 1, 2, 9, PixelDescriptor::RGB8_SRGB)
            .unwrap()
            .with_color_context(context);
        source.read_rows(0, view, &enough::Unstoppable).unwrap();
        assert_eq!(output, [1, 2, 3, 77, 77, 77, 77, 77, 77, 4, 5, 6]);
    }
}
