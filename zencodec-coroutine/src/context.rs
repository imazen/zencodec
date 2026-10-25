//! The codec author's API: read bytes, write rows, announce metadata.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use enough::StopReason;
use zencodec::ImageInfo;
use zencodec::decode::{
    BatchMeta, BoundaryAction, BoundaryId, ByteRange, ControlError, DecodeBoundary,
    InterpretationFacts, MetadataUpdate, Request, SinkError,
};
use zencodec::source::SourceError;
use zencodec::storage::{OwnedStorage, StorageBudget, StorageError};
use zenpixels::{PixelDescriptor, PixelSlice};

use crate::input::InputStore;

/// Failure inside a coroutine decode, convertible into the codec's error.
#[derive(Debug)]
#[non_exhaustive]
pub enum CoroutineError {
    /// Input payload, range index or row staging could not be admitted.
    Storage(StorageError),
    /// An absolute byte range overflowed the source address space.
    InputRangeOverflow,
    /// The admitted input range index cannot represent another disjoint span.
    InputSpanLimit {
        /// Maximum number of independently retained spans.
        limit: usize,
    },
    /// A required interpretation boundary was violated.
    Control(ControlError),
    /// A row buffer size overflowed or its allocation failed.
    RowAllocation,
    /// A read could not be satisfied (past the announced end).
    Source(SourceError),
    /// The sink refused rows.
    Sink(SinkError),
    /// The driver's stop token tripped.
    Cancelled(StopReason),
    /// The decoder was stepped after it finished or failed.
    Finished,
    /// The coroutine suspended without asking for anything (author bug).
    Stalled,
    /// The coroutine wrote rows before setting the image info and output
    /// descriptor, so the sink could not be told the geometry (author bug).
    RowsBeforeHeader,
    /// One read asked for more bytes than the decoder's input cap can hold,
    /// so it could never complete.
    ReadExceedsInputCap {
        /// Bytes the read needs.
        len: usize,
        /// The configured cap.
        cap: usize,
    },
}

impl fmt::Display for CoroutineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(e) => e.fmt(f),
            Self::InputRangeOverflow => f.write_str("coroutine input range overflow"),
            Self::InputSpanLimit { limit } => {
                write!(f, "coroutine input exceeds its {limit}-span index")
            }
            Self::Control(e) => e.fmt(f),
            Self::RowAllocation => f.write_str("coroutine row allocation failed"),
            Self::Source(e) => write!(f, "coroutine decode: {e}"),
            Self::Sink(e) => write!(f, "coroutine decode: sink: {e}"),
            Self::Cancelled(r) => write!(f, "coroutine decode cancelled: {r:?}"),
            Self::Finished => write!(f, "coroutine decoder stepped after it finished"),
            Self::Stalled => write!(f, "coroutine suspended without a request"),
            Self::ReadExceedsInputCap { len, cap } => write!(
                f,
                "coroutine read of {len} bytes exceeds its {cap}-byte input cap"
            ),
            Self::RowsBeforeHeader => write!(
                f,
                "coroutine wrote rows before set_info and set_output_descriptor"
            ),
        }
    }
}

impl core::error::Error for CoroutineError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Storage(e) => Some(e),
            Self::Control(e) => Some(e),
            Self::Source(e) => Some(e),
            Self::Sink(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<StorageError> for CoroutineError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

pub(crate) enum RowStorage {
    Legacy(Vec<u8>),
    Admitted(OwnedStorage<u8>),
}
impl Default for RowStorage {
    fn default() -> Self {
        Self::Legacy(Vec::new())
    }
}
impl RowStorage {
    fn prepare(
        &mut self,
        len: usize,
        budget: Option<&StorageBudget>,
    ) -> Result<(), CoroutineError> {
        if let Some(budget) = budget {
            if matches!(self, Self::Admitted(data) if data.len() >= len) {
                return Ok(());
            }
            // An unused spare is not live output. Release it before growing so
            // growth does not require two simultaneous staging allocations.
            *self = Self::default();
            *self = Self::Admitted(budget.bytes(len)?);
        } else {
            let Self::Legacy(data) = self else {
                unreachable!("storage mode is fixed at construction")
            };
            data.clear();
            data.try_reserve_exact(len)
                .map_err(|_| CoroutineError::RowAllocation)?;
            data.resize(len, 0);
        }
        Ok(())
    }
    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Legacy(data) => data,
            Self::Admitted(data) => data.as_slice(),
        }
    }
    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Self::Legacy(data) => data,
            Self::Admitted(data) => data.as_mut_slice(),
        }
    }
}

impl From<SourceError> for CoroutineError {
    fn from(e: SourceError) -> Self {
        Self::Source(e)
    }
}
impl From<ControlError> for CoroutineError {
    fn from(error: ControlError) -> Self {
        Self::Control(error)
    }
}

impl From<SinkError> for CoroutineError {
    fn from(e: SinkError) -> Self {
        Self::Sink(e)
    }
}

impl From<StopReason> for CoroutineError {
    fn from(r: StopReason) -> Self {
        Self::Cancelled(r)
    }
}

/// Rows the coroutine handed over, waiting for the decoder to copy them
/// into the sink.
pub(crate) struct PendingRows {
    pub(crate) meta: BatchMeta,
    pub(crate) data: RowStorage,
    pub(crate) width: u32,
    pub(crate) rows: u32,
    pub(crate) stride: usize,
    pub(crate) descriptor: PixelDescriptor,
}

/// State shared between the decoder and the coroutine it polls. Only one
/// side runs at a time, so the lock is never contended.
pub(crate) struct Shared {
    pub(crate) controlled: bool,
    pub(crate) boundary: Option<DecodeBoundary>,
    pub(crate) reply: Option<(BoundaryId, BoundaryAction)>,
    pub(crate) adopted: Option<InterpretationFacts>,
    pub(crate) control_failure: Option<ControlError>,
    pub(crate) input: InputStore,
    pub(crate) need: Vec<Request>,
    /// Scratch for a read's satisfied spans and missing gaps, reused across polls.
    pub(crate) satisfied: Vec<(u64, usize)>,
    pub(crate) gaps: Vec<(u64, u64)>,
    pub(crate) rows: Option<PendingRows>,
    /// Strip buffer handed back after each `Rows` step, so steady state
    /// allocates nothing.
    pub(crate) spare: RowStorage,
    pub(crate) storage: Option<StorageBudget>,
    pub(crate) metadata: Option<MetadataUpdate>,
    pub(crate) info: Option<ImageInfo>,
    /// Bumped on every `set_info`, so the decoder refreshes its cached copy.
    pub(crate) info_generation: u64,
    pub(crate) descriptor: Option<PixelDescriptor>,
    pub(crate) accepts_provisional: bool,
    pub(crate) cancelled: Option<StopReason>,
}

impl Shared {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            controlled: false,
            boundary: None,
            reply: None,
            adopted: None,
            control_failure: None,
            input: InputStore::new(cap),
            need: Vec::new(),
            satisfied: Vec::new(),
            gaps: Vec::new(),
            rows: None,
            spare: RowStorage::default(),
            storage: None,
            metadata: None,
            info: None,
            info_generation: 0,
            descriptor: None,
            accepts_provisional: false,
            cancelled: None,
        }
    }

    pub(crate) fn with_storage(cap: usize, budget: StorageBudget) -> Result<Self, CoroutineError> {
        let input = InputStore::with_storage(cap, &budget)?;
        Ok(Self {
            input,
            storage: Some(budget),
            ..Self::new(cap)
        })
    }
}

pub(crate) type SharedCell = Arc<spin::Mutex<Shared>>;

/// Handle a coroutine decode uses to read input and emit output.
///
/// Every `async fn` here suspends the coroutine when it cannot proceed and
/// resumes it on the step after the driver supplied what was missing.
pub struct CoroutineContext {
    shared: SharedCell,
    cursor: u64,
}

impl CoroutineContext {
    pub(crate) fn new(shared: SharedCell) -> Self {
        Self { shared, cursor: 0 }
    }

    /// Read exactly `out.len()` bytes at `offset`.
    ///
    /// Suspends with `Event::Need` (a `Request::Range`) until the driver
    /// offers the span. Fails with [`SourceError::UnexpectedEof`] once the
    /// announced length shows the span can never arrive.
    pub fn read_exact_at<'a>(
        &'a mut self,
        offset: u64,
        out: &'a mut [u8],
    ) -> impl Future<Output = Result<(), CoroutineError>> + 'a {
        ReadMany {
            shared: &self.shared,
            reqs: ReqSlot::One(offset, out),
            release: true,
        }
    }

    /// Read several spans at once. The driver sees one `Event::Need` naming
    /// every missing span, so a batching source fetches them in one round.
    pub fn read_many_at<'a, 'b: 'a>(
        &'a mut self,
        reqs: &'a mut [(u64, &'b mut [u8])],
    ) -> impl Future<Output = Result<(), CoroutineError>> + 'a {
        ReadMany {
            shared: &self.shared,
            reqs: ReqSlot::Many(reqs),
            release: true,
        }
    }

    /// Read the next `out.len()` bytes at the sequential cursor and
    /// advance it. Suspends with one `Request::AtLeast` for the rest of the
    /// read, which stream pumps serve by reading forward. Bytes behind the
    /// cursor are released.
    pub async fn read_exact(&mut self, out: &mut [u8]) -> Result<(), CoroutineError> {
        let offset = self.cursor;
        ReadMany {
            shared: &self.shared,
            reqs: ReqSlot::Sequential(offset, out),
            release: true,
        }
        .await?;
        self.cursor = offset + out.len() as u64;
        self.shared.lock().input.release_before(self.cursor);
        Ok(())
    }

    /// The sequential cursor used by [`read_exact`](Self::read_exact).
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Move the sequential cursor (bytes behind it are released).
    pub fn seek(&mut self, offset: u64) {
        self.cursor = offset;
        self.shared.lock().input.release_before(offset);
    }

    /// Total source length. Suspends with `Request::Len` until a driver
    /// offers an input carrying it.
    pub fn len(&mut self) -> impl Future<Output = Result<u64, CoroutineError>> + '_ {
        Len {
            shared: &self.shared,
        }
    }

    /// Hand a strip of rows to the driver. Suspends until the decoder has
    /// copied it into the sink (one `Event::Rows`).
    ///
    /// Provisional rows (`!meta.is_final`) may be written only when
    /// [`accepts_provisional`](Self::accepts_provisional) is `true`.
    pub fn write_rows<'a>(
        &'a mut self,
        meta: BatchMeta,
        rows: &PixelSlice<'_>,
    ) -> impl Future<Output = Result<(), CoroutineError>> + 'a {
        let refusal = {
            let shared = self.shared.lock();
            shared.control_failure.or_else(|| {
                (shared.controlled && (shared.adopted.is_none() || shared.boundary.is_some()))
                    .then_some(ControlError::OutputBeforeInterpretation)
            })
        };
        if let Some(error) = refusal {
            return WriteRows {
                shared: &self.shared,
                pending: None,
                error: Some(error.into()),
            };
        }
        let width = rows.width();
        let count = rows.rows();
        let descriptor = rows.descriptor();
        let size = (width as usize)
            .checked_mul(descriptor.bytes_per_pixel())
            .and_then(|stride| stride.checked_mul(count as usize).map(|len| (stride, len)));
        let Some((stride, len)) = size else {
            return WriteRows {
                shared: &self.shared,
                pending: None,
                error: Some(CoroutineError::RowAllocation),
            };
        };
        let (mut data, prepared) = {
            let mut shared = self.shared.lock();
            let mut data = core::mem::take(&mut shared.spare);
            let prepared = data.prepare(len, shared.storage.as_ref());
            (data, prepared)
        };
        if let Err(error) = prepared {
            return WriteRows {
                shared: &self.shared,
                pending: None,
                error: Some(error),
            };
        }
        for r in 0..count {
            let at = r as usize * stride;
            data.as_mut_slice()[at..at + stride].copy_from_slice(&rows.row(r)[..stride]);
        }
        WriteRows {
            shared: &self.shared,
            error: None,
            pending: Some(PendingRows {
                meta,
                data,
                width,
                rows: count,
                stride,
                descriptor,
            }),
        }
    }

    /// Report a metadata event to the driver (one `Event::Metadata`).
    /// `HeaderReady` with dimensions and descriptor set lets the decoder
    /// call `sink.begin` early.
    pub fn announce(
        &mut self,
        update: MetadataUpdate,
    ) -> impl Future<Output = Result<(), CoroutineError>> + '_ {
        Announce {
            shared: &self.shared,
            update: Some(update),
        }
    }

    /// Set image metadata (visible through `IncrementalDecode::info`).
    pub fn set_info(&mut self, info: ImageInfo) {
        let mut shared = self.shared.lock();
        if shared.adopted.is_some()
            && shared.info.as_ref().is_some_and(|old| {
                old.width != info.width
                    || old.height != info.height
                    || old.orientation != info.orientation
                    || old.source_color != info.source_color
            })
        {
            shared.control_failure = Some(ControlError::FinalInterpretationChanged);
        }
        shared.info = Some(info);
        shared.info_generation += 1;
    }

    /// Set the negotiated output descriptor.
    pub fn set_output_descriptor(&mut self, descriptor: PixelDescriptor) {
        let mut shared = self.shared.lock();
        if shared.adopted.is_some() && shared.descriptor != Some(descriptor) {
            shared.control_failure = Some(ControlError::FinalInterpretationChanged);
        }
        shared.descriptor = Some(descriptor);
    }

    /// Suspend before work affected by these interpretation facts. Await this
    /// before conversion or output. `ContinueReading` permits discovery only;
    /// rows stay blocked until final interpretation is adopted. This future is
    /// served only by the controlled protocol; legacy execution refuses it.
    pub fn interpretation(
        &mut self,
        facts: InterpretationFacts,
    ) -> impl Future<Output = Result<BoundaryAction, CoroutineError>> + '_ {
        Interpret {
            shared: &self.shared,
            facts: Some(facts),
            id: None,
        }
    }

    /// Whether the driver's sink takes provisional (progressive) rows.
    /// Known once the header has been announced (the decoder reads it when
    /// it calls the sink's `begin`); `false` before that.
    pub fn accepts_provisional(&self) -> bool {
        self.shared.lock().accepts_provisional
    }

    /// Check the driver's stop token without suspending.
    pub fn check_stop(&self) -> Result<(), CoroutineError> {
        let shared = self.shared.lock();
        if let Some(error) = shared.control_failure {
            return Err(error.into());
        }
        match shared.cancelled {
            Some(r) => Err(CoroutineError::Cancelled(r)),
            None => Ok(()),
        }
    }
}

struct Interpret<'a> {
    shared: &'a SharedCell,
    facts: Option<InterpretationFacts>,
    id: Option<BoundaryId>,
}
impl Future for Interpret<'_> {
    type Output = Result<BoundaryAction, CoroutineError>;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut shared = this.shared.lock();
        if let Some(reason) = shared.cancelled {
            return Poll::Ready(Err(reason.into()));
        }
        if let Some(error) = shared.control_failure {
            return Poll::Ready(Err(error.into()));
        }
        if !shared.controlled {
            return Poll::Ready(Err(ControlError::UnhandledBoundary.into()));
        }
        if let Some(facts) = this.facts.take() {
            if shared.boundary.is_some() || shared.reply.is_some() {
                return Poll::Ready(Err(ControlError::InvalidReply.into()));
            }
            if let Some(adopted) = shared.adopted {
                return Poll::Ready(if adopted == facts {
                    Ok(BoundaryAction::AdoptInterpretation)
                } else {
                    Err(ControlError::FinalInterpretationChanged.into())
                });
            }
            if facts.descriptor != shared.descriptor
                || shared
                    .info
                    .as_ref()
                    .is_none_or(|info| info.width != facts.width || info.height != facts.height)
            {
                return Poll::Ready(Err(ControlError::InconsistentFacts.into()));
            }
            let boundary = DecodeBoundary::new(facts)?;
            this.id = Some(boundary.id());
            shared.boundary = Some(boundary);
            return Poll::Pending;
        }
        match shared.reply.take() {
            Some((id, action)) if Some(id) == this.id => Poll::Ready(Ok(action)),
            Some(_) => Poll::Ready(Err(ControlError::InvalidReply.into())),
            None => Poll::Pending,
        }
    }
}

enum ReqSlot<'a, 'b> {
    One(u64, &'a mut [u8]),
    Sequential(u64, &'a mut [u8]),
    Many(&'a mut [(u64, &'b mut [u8])]),
}

struct ReadMany<'a, 'b> {
    shared: &'a SharedCell,
    reqs: ReqSlot<'a, 'b>,
    release: bool,
}

impl Future for ReadMany<'_, '_> {
    type Output = Result<(), CoroutineError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut shared = this.shared.lock();
        if let Some(r) = shared.cancelled {
            return Poll::Ready(Err(CoroutineError::Cancelled(r)));
        }
        // Reuse the shared scratch vectors: `need` collects the gaps and
        // `satisfied` the spans to release, so a poll allocates nothing.
        let mut missing = core::mem::take(&mut shared.need);
        let mut satisfied = core::mem::take(&mut shared.satisfied);
        let mut gaps = core::mem::take(&mut shared.gaps);
        missing.clear();
        satisfied.clear();
        gaps.clear();
        let mut check = |shared: &Shared, offset: u64, out: &mut [u8], sequential: bool| {
            offset
                .checked_add(out.len() as u64)
                .ok_or(CoroutineError::InputRangeOverflow)?;
            if out.len() > shared.input.cap() {
                return Err(CoroutineError::ReadExceedsInputCap {
                    len: out.len(),
                    cap: shared.input.cap(),
                });
            }
            if shared.input.try_read(offset, out) {
                satisfied.push((offset, out.len()));
                Ok(())
            } else if shared.input.beyond_end(offset, out.len()) {
                Err(CoroutineError::Source(SourceError::UnexpectedEof {
                    offset,
                    len: out.len() as u64,
                    total: shared.input.total(),
                }))
            } else {
                // Name only the gaps, so a driver that fetched part of the
                // span (a sequential chunk, a coalesced neighbour) is asked
                // for the rest rather than the same bytes again.
                let before = gaps.len();
                shared.input.missing_into(offset, out.len(), &mut gaps);
                if sequential {
                    // One contiguous request from the first missing byte to
                    // the end of the read, so a stream driver supplies it
                    // in one input.
                    let first = gaps.get(before).map(|(o, _)| *o).unwrap_or(offset);
                    gaps.truncate(before);
                    let end = offset + out.len() as u64;
                    missing.push(Request::at_least(first, (end - first) as usize));
                } else {
                    missing.extend(
                        gaps.drain(before..)
                            .map(|(o, l)| Request::Range(ByteRange::new(o, l))),
                    );
                }
                Ok(())
            }
        };
        let result = match &mut this.reqs {
            ReqSlot::One(offset, out) => check(&shared, *offset, out, false),
            ReqSlot::Sequential(offset, out) => check(&shared, *offset, out, true),
            ReqSlot::Many(reqs) => reqs
                .iter_mut()
                .try_for_each(|(offset, out)| check(&shared, *offset, out, false)),
        };
        shared.gaps = gaps;
        if let Err(e) = result {
            shared.need = missing;
            shared.satisfied = satisfied;
            return Poll::Ready(Err(e));
        }
        let ready = missing.is_empty();
        if ready && this.release {
            for (offset, len) in &satisfied {
                if let Err(error) = shared.input.release(*offset, *len) {
                    shared.need = missing;
                    shared.satisfied = satisfied;
                    return Poll::Ready(Err(error));
                }
            }
        }
        shared.need = missing;
        shared.satisfied = satisfied;
        if ready {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

struct Len<'a> {
    shared: &'a SharedCell,
}

impl Future for Len<'_> {
    type Output = Result<u64, CoroutineError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared = self.shared.lock();
        if let Some(r) = shared.cancelled {
            return Poll::Ready(Err(CoroutineError::Cancelled(r)));
        }
        match shared.input.total() {
            Some(t) => Poll::Ready(Ok(t)),
            None => {
                shared.need.clear();
                shared.need.push(Request::Len);
                Poll::Pending
            }
        }
    }
}

struct WriteRows<'a> {
    shared: &'a SharedCell,
    pending: Option<PendingRows>,
    error: Option<CoroutineError>,
}

impl Future for WriteRows<'_> {
    type Output = Result<(), CoroutineError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(error) = this.error.take() {
            return Poll::Ready(Err(error));
        }
        let mut shared = this.shared.lock();
        if let Some(r) = shared.cancelled {
            return Poll::Ready(Err(CoroutineError::Cancelled(r)));
        }
        match this.pending.take() {
            Some(rows) => {
                shared.rows = Some(rows);
                Poll::Pending
            }
            // Resumed: the decoder took the rows (or failed; it then never
            // resumes us).
            None => Poll::Ready(Ok(())),
        }
    }
}

struct Announce<'a> {
    shared: &'a SharedCell,
    update: Option<MetadataUpdate>,
}

impl Future for Announce<'_> {
    type Output = Result<(), CoroutineError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut shared = this.shared.lock();
        if let Some(r) = shared.cancelled {
            return Poll::Ready(Err(CoroutineError::Cancelled(r)));
        }
        match this.update.take() {
            Some(update) => {
                shared.metadata = Some(update);
                Poll::Pending
            }
            None => Poll::Ready(Ok(())),
        }
    }
}
