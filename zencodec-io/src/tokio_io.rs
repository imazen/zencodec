//! Async drivers for tokio (and any executor): decode with no thread
//! blocked on I/O and no `async` in codec code.
//!
//! [`decode_async`] keeps the decoder, the sink and the input window in the
//! async task. It awaits the [`AsyncSource`] for bytes, then hands one
//! decode batch (steps until the decoder needs input or finishes) to a
//! [`Compute`] pool, and awaits that. While the client uploads, nothing
//! occupies a thread; CPU work runs on the pool in batches as large as the
//! input on hand. A source returns whatever bytes have arrived, so rows
//! come out while the upload is still running.
//!
//! [`transcode_async`] adds an encoder: rows go through an
//! [`EncoderSink`](crate::EncoderSink) and encoded bytes are written to an
//! `AsyncWrite` after every batch. The decoder and encoder stay alive while
//! a write waits; their raw input and intermediate storage are not bounded
//! by the encoded-output queue. Choose this coupling for configurations
//! whose retained state and upstream production are bounded.
//!
//! [`pull_decode_on_thread`] runs a pull-native decoder (one written against
//! [`DecodeJob::push_decoder_from_source`](zencodec::decode::DecodeJob::push_decoder_from_source))
//! on one pool thread, with a [`DynReadAt`] whose reads block on the same
//! [`AsyncSource`]. Such a codec blocks on reads by design, so it holds a
//! thread for the whole decode.
//!
//! # Cancellation
//!
//! Two mechanisms, for the two places a decode can be waiting:
//!
//! - **Inside a compute batch**, the [`SharedPulse`] is checked at every
//!   step, so a long batch stops within one step of the pulse tripping.
//!   The pulse is also checked before each batch starts.
//! - **While awaiting the source**, cancel the way tokio code always does:
//!   drop the future, or race it in `tokio::select!` against your
//!   cancellation token or a timeout. Nothing runs between batches, so the
//!   decoder and sink are dropped at once. A batch already on the pool
//!   finishes it (bounded by the input on hand) unless the pulse also trips.
//!
//! [`pull_decode_on_thread`] differs: the decode thread re-checks the pulse
//! while it waits for a read, returns [`AsyncDriveError::Cancelled`], and the
//! async side stops waiting on the source when the thread finishes.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;

use enough::{Stop, StopReason};
use how_far::{Pulse, SharedPulse};
use tokio::sync::{mpsc, oneshot};
use zencodec::decode::{
    BatchMeta, BoxedError, ByteRange, DecodeRowSink, Event, IncrementalDecode, InputRequest,
    OutputInfo, SinkError, StepError,
};
use zencodec::encode::Encoder;
use zencodec::how_far::NoPulse;
use zencodec::source::{DynReadAt, SourceError};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSliceMut};

use crate::admission::{self, InputError};
use crate::drive::{DriveOptions, Progressor};
use crate::window::{ProtocolViolation, Window, check_need, read_target};

// ───────────────────────────── sources and compute ─────────────────────────────

/// Where the async drivers get bytes: a socket, an HTTP body, an object
/// store with range requests.
// `len` is async and fallible; an `is_empty` would only wrap it.
#[allow(clippy::len_without_is_empty)]
pub trait AsyncSource: Send {
    /// Read up to `out.len()` bytes at `offset` into `out` and return the
    /// count. Like `AsyncRead`, a read may return fewer bytes than asked:
    /// whatever has arrived, so a decoder can start on it. Zero means the
    /// source ends at `offset`.
    fn read_at(
        &mut self,
        offset: u64,
        out: &mut [u8],
    ) -> impl Future<Output = Result<usize, BoxedError>> + Send;

    /// Fill each buffer from its offset. Sources that can serve ranges
    /// concurrently (HTTP/2, object stores) override this to overlap them;
    /// the default reads them one after another.
    fn read_ranges(
        &mut self,
        reqs: &mut [(u64, &mut [u8])],
    ) -> impl Future<Output = Result<(), BoxedError>> + Send {
        async move {
            for (offset, out) in reqs.iter_mut() {
                read_full(self, *offset, out).await?;
            }
            Ok(())
        }
    }

    /// Total length if known without reading ahead.
    fn len_hint(&self) -> Option<u64> {
        None
    }

    /// Total length. Sequential sources can only answer once they reached
    /// the end; they return an error before that.
    fn len(&mut self) -> impl Future<Output = Result<u64, BoxedError>> + Send;
}

/// Fill `out` from `offset`, reading until it is full; the source ending
/// first is [`SourceError::UnexpectedEof`].
async fn read_full<S: AsyncSource + ?Sized>(
    source: &mut S,
    offset: u64,
    out: &mut [u8],
) -> Result<(), BoxedError> {
    let mut filled = 0;
    while filled < out.len() {
        let n = source
            .read_at(offset + filled as u64, &mut out[filled..])
            .await?;
        if n == 0 {
            return Err(Box::new(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(offset + filled as u64),
            }));
        }
        filled += n;
    }
    Ok(())
}

/// An in-memory [`AsyncSource`] (tests, already-buffered bodies).
#[derive(Debug, Clone)]
pub struct BytesSource(pub alloc::sync::Arc<[u8]>);

impl AsyncSource for BytesSource {
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
        let start = (offset as usize).min(self.0.len());
        let n = (self.0.len() - start).min(out.len());
        out[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }

    fn len_hint(&self) -> Option<u64> {
        Some(self.0.len() as u64)
    }

    async fn len(&mut self) -> Result<u64, BoxedError> {
        Ok(self.0.len() as u64)
    }
}

/// A sequential [`AsyncSource`] over any `AsyncRead` (an HTTP body, a
/// socket). Reads ahead of the stream position skip forward; reads behind
/// it fail with [`SourceError::BeyondWindow`], and the length is known only
/// once the stream has ended.
#[derive(Debug)]
pub struct AsyncReadSource<R> {
    reader: R,
    pos: u64,
    eof: bool,
}

impl<R> AsyncReadSource<R> {
    /// Wrap `reader`, positioned at offset 0.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            pos: 0,
            eof: false,
        }
    }
}

impl<R: tokio::io::AsyncRead + Unpin + Send> AsyncSource for AsyncReadSource<R> {
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
        use tokio::io::AsyncReadExt;
        if offset < self.pos {
            return Err(Box::new(SourceError::BeyondWindow {
                offset,
                oldest_retained: self.pos,
            }));
        }
        let mut skip = [0u8; 4096];
        while self.pos < offset && !self.eof {
            let want = ((offset - self.pos) as usize).min(skip.len());
            let n = self.reader.read(&mut skip[..want]).await?;
            if n == 0 {
                self.eof = true;
            }
            self.pos += n as u64;
        }
        if self.eof || out.is_empty() {
            return Ok(0);
        }
        // One read: whatever has arrived, so decoding starts on it.
        let n = self.reader.read(out).await?;
        if n == 0 {
            self.eof = true;
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn len_hint(&self) -> Option<u64> {
        self.eof.then_some(self.pos)
    }

    async fn len(&mut self) -> Result<u64, BoxedError> {
        if self.eof {
            Ok(self.pos)
        } else {
            Err("a sequential stream cannot report its length before it ends".into())
        }
    }
}

/// Work handed to a [`Compute`].
pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// Where decode batches run. Implemented for any `Fn(Job)`:
/// `|job| rayon::spawn(job)`, `|job| { tokio::task::spawn_blocking(job); }`,
/// or [`Inline`] to run on the calling task (single-threaded hosts, tiny
/// images).
pub trait Compute: Send + Sync {
    /// Run `job` somewhere; the driver awaits its result.
    fn spawn(&self, job: Job);

    /// Whether `spawn` would run the job right here, on the calling task.
    /// The drivers then call the batch directly, without boxing it or
    /// creating a channel for its result. Default `false`.
    fn runs_inline(&self) -> bool {
        false
    }
}

impl<F: Fn(Job) + Send + Sync> Compute for F {
    fn spawn(&self, job: Job) {
        self(job)
    }
}

/// Runs each batch on the calling task. For single-threaded hosts (WASM)
/// and small images, where moving work to another thread costs more than it
/// saves; it blocks the executor for one batch.
#[derive(Debug, Clone, Copy, Default)]
pub struct Inline;

impl Compute for Inline {
    fn spawn(&self, job: Job) {
        job()
    }

    fn runs_inline(&self) -> bool {
        true
    }
}

/// Run `f` on `compute` and await its result.
///
/// Spawns (or, for an inline compute, runs) before returning, and the
/// returned future holds only the result or its receiver, so the caller's
/// future stays `Send` whatever `compute` is.
fn run_on<C: Compute + ?Sized, T: Send + 'static>(
    compute: &C,
    f: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Result<T, AsyncDriveError<core::convert::Infallible>>> + Send + 'static {
    let (done, rx) = if compute.runs_inline() {
        (Some(f()), None)
    } else {
        let (tx, rx) = oneshot::channel();
        compute.spawn(Box::new(move || {
            let _ = tx.send(f());
        }));
        (None, Some(rx))
    };
    async move {
        match (done, rx) {
            (Some(v), _) => Ok(v),
            (None, Some(rx)) => rx.await.map_err(|_| AsyncDriveError::Disconnected),
            (None, None) => Err(AsyncDriveError::Disconnected),
        }
    }
}

// ───────────────────────────── errors ─────────────────────────────

/// Failure of an async drive.
#[derive(Debug)]
#[non_exhaustive]
pub enum AsyncDriveError<E> {
    /// The decoder failed, the sink failed, or the pulse tripped inside a batch.
    Step(StepError<E>),
    /// The source failed.
    Source(BoxedError),
    /// Writing encoded output failed.
    Write(std::io::Error),
    /// The decoder broke the protocol.
    Protocol(ProtocolViolation),
    /// Driver-owned input could not be admitted or allocated.
    Input(InputError),
    /// The pulse tripped while the driver was waiting.
    Cancelled(StopReason),
    /// A compute job was dropped without a result (its pool shut down or
    /// it panicked).
    Disconnected,
}

/// Lift a codec-free error into any codec error type.
fn widen<F>(e: AsyncDriveError<core::convert::Infallible>) -> AsyncDriveError<F> {
    {
        match e {
            AsyncDriveError::Step(s) => match s {
                StepError::Codec(never) => match never {},
                StepError::Sink(e) => AsyncDriveError::Step(StepError::Sink(e)),
                StepError::Cancelled(r) => AsyncDriveError::Step(StepError::Cancelled(r)),
                StepError::Finished => AsyncDriveError::Step(StepError::Finished),
                _ => AsyncDriveError::Disconnected,
            },
            AsyncDriveError::Source(e) => AsyncDriveError::Source(e),
            AsyncDriveError::Write(e) => AsyncDriveError::Write(e),
            AsyncDriveError::Protocol(p) => AsyncDriveError::Protocol(p),
            AsyncDriveError::Input(e) => AsyncDriveError::Input(e),
            AsyncDriveError::Cancelled(r) => AsyncDriveError::Cancelled(r),
            AsyncDriveError::Disconnected => AsyncDriveError::Disconnected,
        }
    }
}

impl<E: fmt::Display> fmt::Display for AsyncDriveError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Step(e) => write!(f, "{e}"),
            Self::Source(e) => write!(f, "source: {e}"),
            Self::Write(e) => write!(f, "writing output: {e}"),
            Self::Protocol(e) => write!(f, "protocol: {e}"),
            Self::Input(e) => write!(f, "input: {e}"),
            Self::Cancelled(r) => write!(f, "cancelled: {r:?}"),
            Self::Disconnected => write!(f, "compute job dropped without a result"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for AsyncDriveError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Step(e) => Some(e),
            Self::Source(e) => Some(e.as_ref()),
            Self::Write(e) => Some(e),
            Self::Protocol(e) => Some(e),
            Self::Input(e) => Some(e),
            _ => None,
        }
    }
}

// ───────────────────────────── resumable decode ─────────────────────────────

/// What one compute batch hands back.
struct Batch<D: IncrementalDecode, S> {
    decoder: D,
    sink: S,
    window: Window,
    outcome: Result<BatchEnd, BatchFail<D>>,
}

enum BatchEnd {
    /// The decoder's `requested()` holds what it needs.
    Need,
    Yield,
    Done(OutputInfo),
}

enum BatchFail<D: IncrementalDecode> {
    Step(StepError<D::Error>),
    Protocol(ProtocolViolation),
}

/// Step until the decoder needs input or finishes.
fn run_batch<D, S>(
    mut decoder: D,
    mut sink: S,
    mut window: Window,
    pulse: Option<SharedPulse>,
    quota: crate::WorkQuota,
) -> Batch<D, S>
where
    D: IncrementalDecode,
    S: DecodeRowSink,
{
    let pulse_ref: Option<&dyn Pulse> = pulse.as_ref().map(|p| p as &dyn Pulse);
    let mut progress = Progressor::new(pulse_ref);
    let pulse = progress.pulse();
    let mut steps = 0usize;
    let outcome = loop {
        if steps == quota.max_steps() {
            break Ok(BatchEnd::Yield);
        }
        let input = window.input();
        let p = match decoder.step(input, &mut sink, pulse) {
            Ok(p) => p,
            Err(e) => break Err(BatchFail::Step(e)),
        };
        steps += 1;
        progress.observe(&decoder, &p.event);
        if p.event == Event::Need
            && let Err(v) = check_need(decoder.requested(), &input, p.consumed)
        {
            break Err(BatchFail::Protocol(v));
        }
        window.consume(p.consumed);
        match p.event {
            Event::Need => break Ok(BatchEnd::Need),
            Event::Done(info) => break Ok(BatchEnd::Done(info)),
            _ => continue,
        }
    };
    Batch {
        decoder,
        sink,
        window,
        outcome,
    }
}

/// Satisfy `requests` into `window` from `source`. Range requests are
/// coalesced and read with one `read_ranges` call; the first one's bytes
/// become the window, later ones wait in `fetched`.
// Written out rather than `async fn` so the `Send` bound is proven here,
// for every type parameter: with `async fn`, rustc's higher-ranked
// lifetime check rejects `tokio::spawn` of the caller's future.
#[allow(clippy::manual_async_fn)]
fn fill<'a, Src: AsyncSource + ?Sized, E: 'a>(
    source: &'a mut Src,
    window: &'a mut Window,
    fetched: &'a mut Vec<(u64, Vec<u8>)>,
    requests: &'a [InputRequest],
    opts: DriveOptions,
) -> impl Future<Output = Result<(), AsyncDriveError<E>>> + Send + 'a {
    async move {
        let limits = opts.limits;
        let request = admission::select(requests, limits).map_err(AsyncDriveError::Input)?;
        match request {
            InputRequest::AtLeast { offset, min_len } => {
                window.position(offset, fetched);
                // Read until the request is met (or the source ends), asking
                // for up to `read_size` more each time; a network source returns
                // what has arrived, so the decoder steps as soon as it can.
                let target = read_target(min_len, opts.read_size)
                    .min(limits.max_input_bytes())
                    .min(usize::try_from(u64::MAX - offset).unwrap_or(usize::MAX));
                admission::make_room(window, fetched, target, limits);
                while window.len() < min_len.max(1) && window.total != Some(window.end()) {
                    limits.reading().map_err(AsyncDriveError::Input)?;
                    let end = window.end();
                    let want = (target - window.len().min(target))
                        .min(usize::try_from(u64::MAX - end).unwrap_or(usize::MAX));
                    let spare = window
                        .try_spare(
                            want,
                            limits.max_input_bytes() - admission::capacity(fetched),
                        )
                        .map_err(AsyncDriveError::Input)?;
                    let n = source
                        .read_at(end, spare)
                        .await
                        .map_err(AsyncDriveError::Source)?;
                    window.commit(n);
                    if n == 0 {
                        window.total = Some(end);
                    }
                }
            }
            InputRequest::Range(r) => {
                if r.offset >= window.base() && r.end() <= window.end() {
                    window.consume((r.offset - window.base()) as usize);
                    return Ok(());
                }
                if let Some(i) = fetched
                    .iter()
                    .position(|(off, buf)| r.offset >= *off && r.end() <= off + buf.len() as u64)
                {
                    let (off, buf) = fetched.swap_remove(i);
                    window.adopt(off, buf);
                    window.consume((r.offset - off) as usize);
                    return Ok(());
                }
                if window.position(r.offset, fetched) {
                    let missing = (r.end() - window.end()) as usize;
                    let offset = window.end();
                    limits.reading().map_err(AsyncDriveError::Input)?;
                    admission::make_room(window, fetched, r.length as usize, limits);
                    let spare = window
                        .try_spare(
                            missing,
                            limits.max_input_bytes() - admission::capacity(fetched),
                        )
                        .map_err(AsyncDriveError::Input)?;
                    read_full(source, offset, spare)
                        .await
                        .map_err(AsyncDriveError::Source)?;
                    window.commit(missing);
                    return Ok(());
                }
                let mut coalesced = Vec::new();
                admission::plan(r, requests, opts.max_gap, limits, &mut coalesced)
                    .map_err(AsyncDriveError::Input)?;
                window.release();
                let mut pending = core::mem::take(fetched);
                admission::allocate_ranges(&coalesced, limits, &mut pending)
                    .map_err(AsyncDriveError::Input)?;
                {
                    let mut reqs = Vec::new();
                    admission::reserve(&mut reqs, pending.len()).map_err(AsyncDriveError::Input)?;
                    reqs.extend(pending.iter_mut().map(|(o, b)| (*o, &mut b[..])));
                    for batch in reqs.chunks_mut(limits.max_in_flight()) {
                        source
                            .read_ranges(batch)
                            .await
                            .map_err(AsyncDriveError::Source)?;
                    }
                }
                // Cancellation drops pending storage without publishing zeros.
                *fetched = pending;
                let i = fetched
                    .iter()
                    .position(|(off, buf)| r.offset >= *off && r.end() <= off + buf.len() as u64)
                    .expect("coalesced ranges cover every request");
                let (off, buf) = fetched.swap_remove(i);
                window.adopt(off, buf);
                window.consume((r.offset - off) as usize);
            }
            InputRequest::Len => {
                if window.total.is_none() {
                    limits.reading().map_err(AsyncDriveError::Input)?;
                    window.total = Some(source.len().await.map_err(AsyncDriveError::Source)?);
                }
            }
            _ => return Err(AsyncDriveError::Protocol(ProtocolViolation::EmptyRequest)),
        }
        Ok(())
    }
}

/// Drives any [`ByteStepper`](crate::ByteStepper) from an [`AsyncSource`]:
/// the async twin of [`ReadAtCursor`](crate::ReadAtCursor).
///
/// Each `next` steps the stage on the calling task and awaits the source
/// only when the stage needs bytes, so a container demuxer (cheap per step)
/// streams packets from a socket or an object store without holding a
/// thread during I/O and without becoming async itself. Range requests are
/// coalesced and fetched with one [`AsyncSource::read_ranges`] per round.
/// CPU-heavy stages (image decoders) use [`decode_async`], which moves the
/// work off the async runtime.
#[derive(Debug)]
pub struct AsyncCursor {
    opts: DriveOptions,
    window: Window,
    fetched: Vec<(u64, Vec<u8>)>,
    started: bool,
    steps_since_yield: usize,
}

impl AsyncCursor {
    /// A cursor with the given read sizes.
    pub fn new(opts: DriveOptions) -> Self {
        Self {
            opts,
            window: Window::new(),
            fetched: Vec::new(),
            started: false,
            steps_since_yield: 0,
        }
    }

    /// Retained driver-owned payload capacity, excluding source caches and outputs.
    pub fn retained_input_capacity(&self) -> usize {
        self.window
            .capacity()
            .saturating_add(admission::capacity(&self.fetched))
    }

    /// Step `stage` until it produces an output, awaiting `source` for every
    /// request. Pass the same source on every call.
    pub async fn next<S, Src>(
        &mut self,
        source: &mut Src,
        stage: &mut S,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<crate::Yield<S::Output>, crate::DriveFailure<S::Error, BoxedError>>
    where
        S: crate::ByteStepper + ?Sized,
        Src: AsyncSource + ?Sized,
    {
        use crate::{DriveFailure, Step, Yield};
        if !self.started {
            self.started = true;
            self.window.total = source.len_hint();
        }
        loop {
            if self.steps_since_yield == self.opts.work_quota.max_steps() {
                crate::work_quota::yield_once().await;
                self.steps_since_yield = 0;
            }
            let input = self.window.input();
            let p = stage.step(input, pulse).map_err(DriveFailure::Step)?;
            self.steps_since_yield += 1;
            if matches!(p.step, Step::Need) {
                check_need(stage.requested(), &input, p.consumed)
                    .map_err(DriveFailure::Protocol)?;
            }
            self.window.consume(p.consumed);
            match p.step {
                Step::Output(o) => return Ok(Yield::Output(o)),
                Step::Finished(o) => return Ok(Yield::Finished(o)),
                Step::Need => {}
            }
            fill::<_, core::convert::Infallible>(
                source,
                &mut self.window,
                &mut self.fetched,
                stage.requested(),
                self.opts,
            )
            .await
            .map_err(|e| match e {
                AsyncDriveError::Source(e) => DriveFailure::Source(e),
                AsyncDriveError::Protocol(p) => DriveFailure::Protocol(p),
                AsyncDriveError::Input(e) => DriveFailure::Input(e),
                // `fill` reports only source and protocol failures.
                _ => DriveFailure::Protocol(ProtocolViolation::EmptyRequest),
            })?;
        }
    }
}

/// Write every byte `enc` has pending to `w`, then consume it: the async
/// twin of [`write_pending`](crate::write_pending). Returns how many bytes
/// were written.
///
/// Encoding is CPU work, so where `push_rows` runs is the caller's choice
/// (a blocking thread, a compute pool); this only moves the bytes. Between
/// pushes, awaiting it is the backpressure point: no more rows go in until
/// the transport took the last batch. This also keeps the encoder's raw and
/// intermediate state alive during the wait. For an admitted buffered job,
/// completing into separately budgeted encoded storage can release that state
/// earlier. On an error nothing is consumed and the transport holds an unknown
/// prefix: abort it.
pub async fn write_pending_async<E, W>(enc: &mut E, w: &mut W) -> std::io::Result<usize>
where
    E: Encoder,
    W: tokio::io::AsyncWrite + Unpin + ?Sized,
{
    use tokio::io::AsyncWriteExt as _;
    let n = enc.pending_output().len();
    if n > 0 {
        w.write_all(enc.pending_output()).await?;
        enc.consume_output(n);
    }
    Ok(n)
}

/// Decode from an async source with no thread blocked on I/O. See the
/// [module documentation](self).
///
/// Returns the output info and the sink. `pulse` cancels at the next step
/// inside a batch and between batches, and receives the row total and
/// per-strip progress.
// Written out rather than `async fn` so the `Send` bound is proven here,
// for every type parameter: with `async fn`, rustc's higher-ranked
// lifetime check rejects `tokio::spawn` of the caller's future.
#[allow(clippy::manual_async_fn)]
pub fn decode_async<'a, D, S, Src, C>(
    decoder: D,
    sink: S,
    source: &'a mut Src,
    compute: C,
    opts: DriveOptions,
    pulse: Option<SharedPulse>,
) -> impl Future<Output = Result<(OutputInfo, S), AsyncDriveError<D::Error>>> + Send + 'a
where
    D: IncrementalDecode + Send + 'static,
    S: DecodeRowSink + Send + 'static,
    Src: AsyncSource + ?Sized,
    C: Compute + 'a,
{
    async move {
        let mut state = Some((decoder, sink, Window::new()));
        if let Some((_, _, w)) = state.as_mut() {
            w.total = source.len_hint();
        }
        let mut fetched = Vec::new();
        let mut requests: Vec<InputRequest> = Vec::new();
        loop {
            if let Some(p) = &pulse
                && let Err(r) = p.check()
            {
                return Err(AsyncDriveError::Cancelled(r));
            }
            let (decoder, sink, window) = state.take().expect("restored after every batch");
            let pulse_for_batch = pulse.clone();
            let batch = run_on(&compute, move || {
                run_batch(decoder, sink, window, pulse_for_batch, opts.work_quota)
            })
            .await
            .map_err(widen)?;
            let Batch {
                decoder,
                sink,
                mut window,
                outcome,
            } = batch;
            match outcome {
                Ok(BatchEnd::Done(info)) => return Ok((info, sink)),
                Ok(BatchEnd::Yield) => crate::work_quota::yield_once().await,
                Ok(BatchEnd::Need) => {
                    // Copied into a reused list: the decoder is not `Sync`, so it
                    // cannot be borrowed across the reads.
                    admission::copy_requests(decoder.requested(), opts.limits, &mut requests)
                        .map_err(AsyncDriveError::Input)?;
                    fill(source, &mut window, &mut fetched, &requests, opts).await?;
                }
                Err(BatchFail::Step(e)) => return Err(AsyncDriveError::Step(e)),
                Err(BatchFail::Protocol(v)) => return Err(AsyncDriveError::Protocol(v)),
            }
            state = Some((decoder, sink, window));
        }
    }
}

/// Decode from an async source straight into a streaming encoder, writing
/// encoded bytes to `out` after every batch. This does not establish an
/// end-to-end memory bound: codec-owned raw/intermediate state and caller-owned
/// input can remain live while the output is stalled. The current driver also
/// drains pending output before finalizing the encoder. Select an operation
/// strategy based on the actual configuration's retention and release behavior.
///
/// The encoder is told the image size through [`Encoder::begin`] when the
/// decoder reaches its header; an encoder that answers with a streaming
/// [`OutputTiming`](zencodec::encode::OutputTiming) then emits bytes as rows
/// arrive, and one that answers
/// [`AT_FINISH`](zencodec::encode::OutputTiming::AT_FINISH) writes
/// everything at the end. Returns the number of bytes written.
// Written out rather than `async fn` so the `Send` bound is proven here,
// for every type parameter: with `async fn`, rustc's higher-ranked
// lifetime check rejects `tokio::spawn` of the caller's future.
#[allow(clippy::manual_async_fn)]
pub fn transcode_async<'a, D, E, Src, W, C>(
    decoder: D,
    encoder: E,
    source: &'a mut Src,
    out: &'a mut W,
    compute: C,
    opts: DriveOptions,
    pulse: Option<SharedPulse>,
) -> impl Future<Output = Result<u64, AsyncDriveError<D::Error>>> + Send + 'a
where
    D: IncrementalDecode + Send + 'static,
    E: Encoder + Send + 'static,
    E::Error: Send + Sync,
    Src: AsyncSource + ?Sized,
    W: tokio::io::AsyncWrite + Unpin + Send + ?Sized,
    C: Compute + 'a,
{
    async move {
        use tokio::io::AsyncWriteExt;
        let mut written = 0u64;
        let mut state = Some((decoder, crate::EncoderSink::new(encoder), Window::new()));
        if let Some((_, _, w)) = state.as_mut() {
            w.total = source.len_hint();
        }
        let mut fetched = Vec::new();
        let mut requests: Vec<InputRequest> = Vec::new();
        loop {
            if let Some(p) = &pulse
                && let Err(r) = p.check()
            {
                return Err(AsyncDriveError::Cancelled(r));
            }
            let (decoder, sink, window) = state.take().expect("restored after every batch");
            let pulse_for_batch = pulse.clone();
            let batch = run_on(&compute, move || {
                run_batch(decoder, sink, window, pulse_for_batch, opts.work_quota)
            })
            .await
            .map_err(widen)?;
            let Batch {
                decoder,
                mut sink,
                mut window,
                outcome,
            } = batch;
            let pending = sink.pending_output().len();
            if pending > 0 && matches!(outcome, Ok(BatchEnd::Need | BatchEnd::Done(_))) {
                out.write_all(sink.pending_output())
                    .await
                    .map_err(AsyncDriveError::Write)?;
                sink.consume_output(pending);
                written += pending as u64;
            }
            match outcome {
                Ok(BatchEnd::Yield) => crate::work_quota::yield_once().await,
                Ok(BatchEnd::Done(_)) => {
                    let rest = sink
                        .finish()
                        .map_err(|e| AsyncDriveError::Step(StepError::Sink(Box::new(e))))?;
                    out.write_all(rest.data())
                        .await
                        .map_err(AsyncDriveError::Write)?;
                    out.flush().await.map_err(AsyncDriveError::Write)?;
                    return Ok(written + rest.data().len() as u64);
                }
                Ok(BatchEnd::Need) => {
                    // Copied into a reused list: the decoder is not `Sync`, so it
                    // cannot be borrowed across the reads.
                    admission::copy_requests(decoder.requested(), opts.limits, &mut requests)
                        .map_err(AsyncDriveError::Input)?;
                    fill(source, &mut window, &mut fetched, &requests, opts).await?;
                }
                Err(BatchFail::Step(e)) => return Err(AsyncDriveError::Step(e)),
                Err(BatchFail::Protocol(v)) => return Err(AsyncDriveError::Protocol(v)),
            }
            state = Some((decoder, sink, window));
        }
    }
}

// ───────────────────────────── pull-native bridge ─────────────────────────────

/// Outcome of one non-blocking attempt in [`wait_with_stop`].
enum Attempt<T> {
    Ready(T),
    Pending,
    Closed,
}

/// Block until `attempt` is ready, re-checking `pulse` between attempts.
///
/// Without a pulse the caller uses the channel's own blocking call; with
/// one, a short spin then a sleep that doubles from 50 µs to 2 ms bounds
/// both the cancellation latency and the idle wakeups.
fn wait_with_stop<T>(
    pulse: Option<&dyn zencodec::Pulse>,
    mut attempt: impl FnMut() -> Attempt<T>,
) -> Result<T, AsyncDriveError<core::convert::Infallible>> {
    let mut sleep = core::time::Duration::from_micros(50);
    let mut spins = 0u32;
    loop {
        match attempt() {
            Attempt::Ready(v) => return Ok(v),
            Attempt::Closed => return Err(AsyncDriveError::Disconnected),
            Attempt::Pending => {}
        }
        if let Some(s) = pulse
            && let Err(reason) = s.check()
        {
            return Err(AsyncDriveError::Cancelled(reason));
        }
        if spins < 64 {
            spins += 1;
            core::hint::spin_loop();
        } else {
            std::thread::sleep(sleep);
            sleep = (sleep * 2).min(core::time::Duration::from_millis(2));
        }
    }
}

/// A read the pull-native decoder is blocked on.
enum PullRequest {
    ReadAt { offset: u64, len: usize },
    Ranges(Vec<ByteRange>),
    Len,
}

enum PullReply {
    Bytes(Vec<u8>),
    Ranges(Vec<Vec<u8>>),
    Len(u64),
    Failed(BoxedError),
}

struct Links {
    requests: mpsc::Sender<PullRequest>,
    replies: mpsc::Receiver<PullReply>,
    pulse: Option<SharedPulse>,
}

impl Links {
    fn ask(
        &mut self,
        req: PullRequest,
    ) -> Result<PullReply, AsyncDriveError<core::convert::Infallible>> {
        let pulse: &dyn zencodec::Pulse = match &self.pulse {
            Some(p) => p,
            None => &NoPulse,
        };
        pulse.check().map_err(AsyncDriveError::Cancelled)?;
        if self.pulse.is_none() {
            self.requests
                .blocking_send(req)
                .map_err(|_| AsyncDriveError::Disconnected)?;
            return self
                .replies
                .blocking_recv()
                .ok_or(AsyncDriveError::Disconnected);
        }
        let mut slot = Some(req);
        let tx = &self.requests;
        wait_with_stop(Some(pulse), || {
            let Some(r) = slot.take() else {
                return Attempt::Ready(());
            };
            match tx.try_send(r) {
                Ok(()) => Attempt::Ready(()),
                Err(mpsc::error::TrySendError::Full(r)) => {
                    slot = Some(r);
                    Attempt::Pending
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Attempt::Closed,
            }
        })?;
        let rx = &mut self.replies;
        wait_with_stop(Some(pulse), || match rx.try_recv() {
            Ok(reply) => Attempt::Ready(reply),
            Err(mpsc::error::TryRecvError::Empty) => Attempt::Pending,
            Err(mpsc::error::TryRecvError::Disconnected) => Attempt::Closed,
        })
    }
}

/// [`DynReadAt`] whose reads block on the async side's [`AsyncSource`].
struct ChannelReadAt {
    links: Links,
    len: Option<u64>,
}

fn bridge_err(e: AsyncDriveError<core::convert::Infallible>) -> BoxedError {
    Box::new(e.into_infallible_display())
}

impl AsyncDriveError<core::convert::Infallible> {
    fn into_infallible_display(self) -> BridgeError {
        BridgeError(self)
    }
}

/// An [`AsyncDriveError`] carried through a pull-native codec's error.
#[derive(Debug)]
pub struct BridgeError(pub AsyncDriveError<core::convert::Infallible>);

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl core::error::Error for BridgeError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        core::error::Error::source(&self.0)
    }
}

impl DynReadAt for ChannelReadAt {
    fn len(&mut self) -> Result<u64, BoxedError> {
        if let Some(len) = self.len {
            return Ok(len);
        }
        match self.links.ask(PullRequest::Len).map_err(bridge_err)? {
            PullReply::Len(len) => {
                self.len = Some(len);
                Ok(len)
            }
            PullReply::Failed(e) => Err(e),
            _ => Err("pull bridge: unexpected reply".into()),
        }
    }

    fn len_hint(&self) -> Option<u64> {
        self.len
    }

    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, BoxedError> {
        match self
            .links
            .ask(PullRequest::ReadAt {
                offset,
                len: out.len(),
            })
            .map_err(bridge_err)?
        {
            PullReply::Bytes(b) => {
                out[..b.len()].copy_from_slice(&b);
                if b.len() < out.len() {
                    self.len = Some(offset + b.len() as u64);
                }
                Ok(b.len())
            }
            PullReply::Failed(e) => Err(e),
            _ => Err("pull bridge: unexpected reply".into()),
        }
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BoxedError> {
        let n = self.read_at(offset, out)?;
        if n < out.len() {
            return Err(Box::new(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: self.len,
            }));
        }
        Ok(())
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
        let ranges = reqs
            .iter()
            .map(|(o, b)| ByteRange::new(*o, b.len() as u64))
            .collect();
        match self
            .links
            .ask(PullRequest::Ranges(ranges))
            .map_err(bridge_err)?
        {
            PullReply::Ranges(bufs) => {
                for ((_, out), b) in reqs.iter_mut().zip(bufs) {
                    out.copy_from_slice(&b);
                }
                Ok(())
            }
            PullReply::Failed(e) => Err(e),
            _ => Err("pull bridge: unexpected reply".into()),
        }
    }
}

/// Run a pull-native decode on one `compute` thread. `run` receives a
/// [`DynReadAt`] whose reads block on `source`, so a codec written against
/// [`push_decoder_from_source`](zencodec::decode::DecodeJob::push_decoder_from_source)
/// streams from an async transport, with `read_many_at` becoming one
/// concurrent [`AsyncSource::read_ranges`].
///
/// `pulse` cancels at the next read: each read checks it, and the wait for
/// the reply re-checks it. Row progress is whatever the sink observes; the
/// bridge cannot see rows written by code it does not drive.
// Written out rather than `async fn` so the `Send` bound is proven here,
// for every type parameter: with `async fn`, rustc's higher-ranked
// lifetime check rejects `tokio::spawn` of the caller's future.
#[allow(clippy::manual_async_fn)]
pub fn pull_decode_on_thread<'a, R, S, Src, C>(
    run: R,
    mut sink: S,
    source: &'a mut Src,
    compute: C,
    pulse: Option<SharedPulse>,
) -> impl Future<Output = Result<(OutputInfo, S), AsyncDriveError<BoxedError>>> + Send + 'a
where
    R: FnOnce(&mut dyn DynReadAt, &mut dyn DecodeRowSink) -> Result<OutputInfo, BoxedError>
        + Send
        + 'static,
    S: DecodeRowSink + Send + 'static,
    Src: AsyncSource + ?Sized,
    C: Compute + 'a,
{
    async move {
        let (req_tx, mut req_rx) = mpsc::channel(1);
        let (rep_tx, rep_rx) = mpsc::channel(1);
        let (done_tx, mut done_rx) = oneshot::channel();
        let mut reader = ChannelReadAt {
            links: Links {
                requests: req_tx,
                replies: rep_rx,
                pulse,
            },
            len: source.len_hint(),
        };
        compute.spawn(Box::new(move || {
            let result = run(&mut reader, &mut sink);
            drop(reader);
            let _ = done_tx.send(result.map(|info| (info, sink)));
        }));
        let finish = |done: Result<
            Result<(OutputInfo, S), BoxedError>,
            oneshot::error::RecvError,
        >| match done {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => match e.downcast::<BridgeError>() {
                Ok(b) => Err(widen(b.0)),
                Err(e) => Err(AsyncDriveError::Step(StepError::Codec(e))),
            },
            Err(_) => Err(AsyncDriveError::Disconnected),
        };
        let mut requests_open = true;
        loop {
            // Wait for the decode to finish or ask for bytes. Serving a request
            // also races the decode finishing: a decode cancelled by its pulse
            // while the source hangs returns without waiting for the source.
            let req = tokio::select! {
                biased;
                done = &mut done_rx => return finish(done),
                req = req_rx.recv(), if requests_open => req,
            };
            let Some(req) = req else {
                requests_open = false;
                continue;
            };
            let reply = tokio::select! {
                biased;
                done = &mut done_rx => return finish(done),
                reply = serve_pull(source, req) => reply,
            };
            // The decode thread waits for this reply; if it has gone, the next
            // loop turn collects its result.
            let _ = rep_tx.send(reply).await;
        }
    }
}

/// Answer one blocking read from the pull-native decode thread.
async fn serve_pull<Src: AsyncSource + ?Sized>(source: &mut Src, req: PullRequest) -> PullReply {
    match req {
        PullRequest::ReadAt { offset, len } => {
            // `DynReadAt::read_at` is short only at the end: read until full
            // or the source ends.
            let mut buf = vec![0u8; len];
            let mut filled = 0;
            while filled < len {
                match source
                    .read_at(offset + filled as u64, &mut buf[filled..])
                    .await
                {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) => return PullReply::Failed(e),
                }
            }
            buf.truncate(filled);
            PullReply::Bytes(buf)
        }
        PullRequest::Ranges(ranges) => {
            let mut bufs: Vec<Vec<u8>> = ranges
                .iter()
                .map(|r| vec![0u8; r.length as usize])
                .collect();
            let result = {
                let mut reqs: Vec<(u64, &mut [u8])> = ranges
                    .iter()
                    .zip(bufs.iter_mut())
                    .map(|(r, b)| (r.offset, &mut b[..]))
                    .collect();
                source.read_ranges(&mut reqs).await
            };
            match result {
                Ok(()) => PullReply::Ranges(bufs),
                Err(e) => PullReply::Failed(e),
            }
        }
        PullRequest::Len => match source.len().await {
            Ok(n) => PullReply::Len(n),
            Err(e) => PullReply::Failed(e),
        },
    }
}

// ───────────────────────────── strips to the async side ─────────────────────────────

/// What a [`ChannelRowSink`] sends to the async side.
#[derive(Debug)]
#[non_exhaustive]
pub enum StripEvent {
    /// The decoder announced the output geometry.
    Begin {
        /// Output width.
        width: u32,
        /// Output height.
        height: u32,
        /// Output pixel format.
        descriptor: PixelDescriptor,
    },
    /// One finished strip.
    Strip {
        /// Row position, pass and finality.
        meta: BatchMeta,
        /// The rows.
        pixels: PixelBuffer,
    },
    /// The decoder finished.
    Finish,
}

/// A sink that streams finished strips to the async side over a bounded
/// channel. Each strip is an owned [`PixelBuffer`]; `provide_*` hands the
/// decoder a fresh strip and ships the previous one, blocking the decode
/// thread while the channel is full, so a slow consumer throttles the
/// decoder instead of the decoder filling memory.
///
/// It blocks, so it belongs on a thread outside the runtime: a [`Compute`]
/// such as `spawn_blocking` or a rayon pool, or [`pull_decode_on_thread`].
/// Under [`Inline`] the batch runs on a runtime worker, where tokio refuses
/// to block.
pub struct ChannelRowSink {
    tx: mpsc::Sender<StripEvent>,
    current: Option<(BatchMeta, PixelBuffer)>,
    provisional: bool,
    pulse: Option<SharedPulse>,
}

impl fmt::Debug for ChannelRowSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelRowSink")
            .field("provisional", &self.provisional)
            .field("pulse", &self.pulse.is_some())
            .finish_non_exhaustive()
    }
}

impl ChannelRowSink {
    /// A sink sending into `tx`. `provisional` says whether progressive
    /// passes should be forwarded as they arrive.
    pub fn new(tx: mpsc::Sender<StripEvent>, provisional: bool) -> Self {
        Self {
            tx,
            current: None,
            provisional,
            pulse: None,
        }
    }

    /// Re-check `pulse` while blocked on a full channel, so a cancelled
    /// decode is not held hostage by a consumer that stopped consuming.
    pub fn with_pulse(mut self, pulse: SharedPulse) -> Self {
        self.pulse = Some(pulse);
        self
    }

    fn send(&self, event: StripEvent) -> Result<(), SinkError> {
        match &self.pulse {
            None => self
                .tx
                .blocking_send(event)
                .map_err(|_| SinkError::from("strip consumer went away")),
            Some(pulse) => {
                let pulse: &dyn zencodec::Pulse = pulse;
                let mut slot = Some(event);
                wait_with_stop(Some(pulse), || {
                    let Some(ev) = slot.take() else {
                        return Attempt::Ready(());
                    };
                    match self.tx.try_send(ev) {
                        Ok(()) => Attempt::Ready(()),
                        Err(mpsc::error::TrySendError::Full(ev)) => {
                            slot = Some(ev);
                            Attempt::Pending
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => Attempt::Closed,
                    }
                })
                .map_err(|e| SinkError::from(alloc::format!("{e}")))
            }
        }
    }

    fn ship(&mut self) -> Result<(), SinkError> {
        if let Some((meta, pixels)) = self.current.take() {
            self.send(StripEvent::Strip { meta, pixels })?;
        }
        Ok(())
    }

    fn strip(
        &mut self,
        meta: BatchMeta,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.ship()?;
        let (_, pixels) = self
            .current
            .insert((meta, PixelBuffer::new(width, height, descriptor)));
        Ok(pixels.as_slice_mut())
    }
}

impl DecodeRowSink for ChannelRowSink {
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<(), SinkError> {
        self.send(StripEvent::Begin {
            width,
            height,
            descriptor,
        })
    }

    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.strip(BatchMeta::final_rows(y), height, width, descriptor)
    }

    fn accepts_provisional(&self) -> bool {
        self.provisional
    }

    fn provide_pass_buffer(
        &mut self,
        meta: BatchMeta,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.strip(meta, height, width, descriptor)
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        self.ship()?;
        self.send(StripEvent::Finish)
    }
}
