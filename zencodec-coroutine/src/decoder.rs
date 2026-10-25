//! `IncrementalDecode` over a coroutine.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use enough::Stop;
use zencodec::ImageInfo;
use zencodec::decode::{
    BatchMeta, BoundaryAction, BoundaryId, ControlError, ControlledIncrementalDecode,
    ControlledProgress, DecodeBoundary, DecodeRowSink, Event, IncrementalDecode, Input,
    MetadataUpdate, OutputInfo, Progress, Request, SinkError, StepError,
};
use zencodec::source::AccessPattern;
use zenpixels::PixelDescriptor;

use crate::context::{CoroutineContext, CoroutineError, Shared, SharedCell};

/// Default cap on fed-but-unread input, in bytes.
pub const DEFAULT_INPUT_CAP: usize = 16 * 1024 * 1024;

/// A boxed decode future, for decoders whose concrete future type is not
/// nameable (every `async` block).
pub type BoxFuture<E> = Pin<Box<dyn Future<Output = Result<OutputInfo, E>> + Send + 'static>>;

/// A [`CoroutineDecoder`] over a [`BoxFuture`]: the type to name in a
/// codec's `DecodeJob::dyn_incremental_decoder` or a struct field.
pub type BoxedCoroutineDecoder<E> = CoroutineDecoder<BoxFuture<E>, E>;

enum State {
    Running,
    /// The future returned while rows or metadata it deposited were still
    /// undelivered (it polled `write_rows` by hand and did not await it):
    /// they go out first, then `Done`.
    Finishing(OutputInfo),
    Done(OutputInfo),
    Failed,
}

/// An [`IncrementalDecode`] whose logic is an `async` block.
///
/// Build one with [`new`](Self::new) from a closure that receives the
/// [`CoroutineContext`] and returns the decode future. The future's error
/// type is the codec's; it must be constructible from [`CoroutineError`]
/// so the context's `?` operator works.
pub struct CoroutineDecoder<F, E> {
    shared: SharedCell,
    future: Pin<Box<F>>,
    state: State,
    begun: bool,
    /// The sink's `accepts_provisional`, mirrored here when `begin` ran so
    /// the `Rows` arm does not take the shared lock just to read it.
    accepts_provisional: bool,
    access: AccessPattern,
    need: Vec<Request>,
    cached_info: Option<ImageInfo>,
    cached_info_generation: u64,
    _err: PhantomData<fn() -> E>,
}

impl<F, E> CoroutineDecoder<F, E>
where
    F: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    E: core::error::Error + Send + Sync + From<CoroutineError> + 'static,
{
    /// Create a decoder with the default input cap and
    /// [`AccessPattern::RandomAccess`].
    pub fn new(init: impl FnOnce(CoroutineContext) -> F) -> Self {
        Self::with_options(DEFAULT_INPUT_CAP, AccessPattern::RandomAccess, init)
    }

    /// Create a decoder requiring explicit interpretation decisions. No sink
    /// effects are allowed until `ctx.interpretation(...).await` has received
    /// adoption of final facts. Serve this through `step_controlled`; legacy
    /// `step` refuses before polling the future.
    pub fn controlled(
        input_cap: usize,
        access: AccessPattern,
        init: impl FnOnce(CoroutineContext) -> F,
    ) -> Self {
        let decoder = Self::with_options(input_cap, access, init);
        decoder.shared.lock().controlled = true;
        decoder
    }

    /// Create a decoder that keeps at most `input_cap` unread bytes and
    /// declares `access` to drivers.
    pub fn with_options(
        input_cap: usize,
        access: AccessPattern,
        init: impl FnOnce(CoroutineContext) -> F,
    ) -> Self {
        let shared = Arc::new(spin::Mutex::new(Shared::new(input_cap)));
        let future = Box::pin(init(CoroutineContext::new(Arc::clone(&shared))));
        Self {
            shared,
            future,
            state: State::Running,
            begun: false,
            accepts_provisional: false,
            access,
            need: Vec::new(),
            cached_info: None,
            cached_info_generation: 0,
            _err: PhantomData,
        }
    }

    /// Unread input currently retained, in bytes.
    pub fn retained_input(&self) -> usize {
        self.shared.lock().input.retained()
    }
}

impl<E> CoroutineDecoder<BoxFuture<E>, E>
where
    E: core::error::Error + Send + Sync + From<CoroutineError> + 'static,
{
    /// Like [`new`](Self::new), boxing the future so the decoder type is
    /// [`BoxedCoroutineDecoder`] regardless of the `async` block.
    pub fn boxed<Fut>(init: impl FnOnce(CoroutineContext) -> Fut) -> Self
    where
        Fut: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    {
        Self::new(|ctx| Box::pin(init(ctx)) as BoxFuture<E>)
    }

    /// Like [`with_options`](Self::with_options), boxing the future.
    pub fn boxed_with_options<Fut>(
        input_cap: usize,
        access: AccessPattern,
        init: impl FnOnce(CoroutineContext) -> Fut,
    ) -> Self
    where
        Fut: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    {
        Self::with_options(input_cap, access, |ctx| Box::pin(init(ctx)) as BoxFuture<E>)
    }
}

impl<F, E> CoroutineDecoder<F, E>
where
    F: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    E: core::error::Error + Send + Sync + From<CoroutineError> + 'static,
{
    /// Copy into the store the bytes of `input[from..]` that the
    /// outstanding reads need, and report how far into `input` that took
    /// (a prefix: needed bytes plus any unneeded bytes before them).
    fn absorb(shared: &mut Shared, input: &Input<'_>, from: usize) -> usize {
        if let Some(t) = input.total_len {
            shared.input.set_total(t);
        }
        let front = input.offset + from as u64;
        let end = input.end();
        if front >= end {
            return from;
        }
        let mut taken_to = front;
        let needs = core::mem::take(&mut shared.need);
        for r in &needs {
            let (lo, hi, sequential) = match *r {
                Request::AtLeast { offset, .. } => (offset, end, true),
                Request::Range(range) => (range.offset, range.end(), false),
                _ => continue,
            };
            let lo = lo.max(front);
            let hi = hi.min(end);
            if lo >= hi {
                continue;
            }
            let slice = &input.bytes[(lo - input.offset) as usize..(hi - input.offset) as usize];
            let stored = shared.input.feed(lo, slice, false);
            let reached = lo + stored as u64;
            if stored == slice.len() || sequential {
                taken_to = taken_to.max(reached);
            } else {
                // The store is full: take only up to where storing stopped.
                taken_to = taken_to.max(reached);
                break;
            }
        }
        shared.need = needs;
        (taken_to - input.offset) as usize
    }

    fn begin_if_ready(&mut self, sink: &mut dyn DecodeRowSink) -> Result<(), StepError<E>> {
        self.check_interpretation()?;
        if self.begun {
            return Ok(());
        }
        let (dims, desc) = {
            let shared = self.shared.lock();
            (
                shared.info.as_ref().map(|i| (i.width, i.height)),
                shared.descriptor,
            )
        };
        if let (Some((w, h)), Some(desc)) = (dims, desc) {
            sink.begin(w, h, desc).map_err(StepError::Sink)?;
            let accepts = sink.accepts_provisional();
            self.shared.lock().accepts_provisional = accepts;
            self.accepts_provisional = accepts;
            self.begun = true;
        }
        Ok(())
    }

    fn check_interpretation(&self) -> Result<(), StepError<E>> {
        let shared = self.shared.lock();
        if let Some(error) = shared.control_failure {
            return Err(StepError::Control(error));
        }
        if shared.controlled && (shared.boundary.is_some() || shared.adopted.is_none()) {
            return Err(StepError::Control(ControlError::OutputBeforeInterpretation));
        }
        Ok(())
    }
}

impl<F, E> CoroutineDecoder<F, E>
where
    F: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    E: core::error::Error + Send + Sync + From<CoroutineError> + 'static,
{
    fn advance(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn Stop,
    ) -> Result<ControlledProgress, StepError<E>> {
        match &self.state {
            State::Done(info) => return Ok(Progress::new(0, Event::Done(info.clone())).into()),
            State::Failed => return Err(StepError::Finished),
            State::Finishing(info) => {
                let info = info.clone();
                return self.finish(sink, info, 0).map(Into::into);
            }
            State::Running => {}
        }
        if let Err(reason) = stop.check() {
            self.shared.lock().cancelled = Some(reason);
            self.state = State::Failed;
            return Err(StepError::Cancelled(reason));
        }
        if let Some(boundary) = self.shared.lock().boundary {
            return Ok(ControlledProgress::Suspended {
                consumed: 0,
                boundary,
            });
        }
        let mut consumed = Self::absorb(&mut self.shared.lock(), &input, 0);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        loop {
            // A caught panic never permits polling the half-mutated future again.
            self.state = State::Failed;
            let polled = self.future.as_mut().poll(&mut cx);
            self.state = State::Running;
            if let Err(reason) = stop.check() {
                self.shared.lock().cancelled = Some(reason);
                self.state = State::Failed;
                return Err(StepError::Cancelled(reason));
            }
            let event = {
                let mut shared = self.shared.lock();
                if shared.info_generation != self.cached_info_generation {
                    self.cached_info = shared.info.clone();
                    self.cached_info_generation = shared.info_generation;
                }
                if let Some(error) = shared.control_failure {
                    self.state = State::Failed;
                    return Err(StepError::Control(error));
                }
                if let Some(boundary) = shared.boundary {
                    if shared.rows.is_some() {
                        self.state = State::Failed;
                        return Err(StepError::Control(ControlError::OutputBeforeInterpretation));
                    }
                    Pending::Boundary(boundary)
                } else if let Some(rows) = shared.rows.take() {
                    Pending::Rows(rows)
                } else if let Some(update) = shared.metadata.take() {
                    Pending::Metadata(update)
                } else if !shared.need.is_empty() {
                    Pending::Need
                } else {
                    Pending::Stalled
                }
            };
            match polled {
                Poll::Ready(Ok(info)) => {
                    if matches!(event, Pending::Boundary(_)) {
                        self.state = State::Failed;
                        return Err(StepError::Control(ControlError::InconsistentFacts));
                    }
                    if matches!(event, Pending::Rows(_) | Pending::Metadata(_)) {
                        // Deliver what was deposited before finishing; the
                        // future is complete and must not be polled again.
                        self.state = State::Finishing(info);
                        self.need.clear();
                    } else {
                        return self.finish(sink, info, consumed).map(Into::into);
                    }
                }
                Poll::Ready(Err(e)) => {
                    self.state = State::Failed;
                    self.need.clear();
                    return Err(StepError::Codec(e));
                }
                Poll::Pending => {}
            }
            match event {
                Pending::Boundary(boundary) => {
                    self.need.clear();
                    return Ok(ControlledProgress::Suspended { consumed, boundary });
                }
                Pending::Rows(rows) => {
                    let inconsistent = {
                        let shared = self.shared.lock();
                        shared.controlled
                            && (shared.descriptor != Some(rows.descriptor)
                                || shared
                                    .info
                                    .as_ref()
                                    .is_none_or(|info| info.width != rows.width))
                    };
                    if inconsistent {
                        self.state = State::Failed;
                        return Err(StepError::Control(ControlError::InconsistentFacts));
                    }
                    if let Err(e) = self.begin_if_ready(sink) {
                        self.state = State::Failed;
                        return Err(e);
                    }
                    if !self.begun {
                        self.state = State::Failed;
                        return Err(StepError::Codec(E::from(CoroutineError::RowsBeforeHeader)));
                    }
                    let copied = copy_rows(sink, &rows, self.accepts_provisional);
                    self.shared.lock().spare = rows.data;
                    return match copied {
                        Ok(()) => Ok(Progress::new(
                            consumed,
                            Event::Rows {
                                meta: rows.meta,
                                rows: rows.rows,
                            },
                        )
                        .into()),
                        Err(e) => {
                            self.state = State::Failed;
                            Err(StepError::Sink(e))
                        }
                    };
                }
                Pending::Metadata(update) => {
                    let controlled = self.shared.lock().controlled;
                    if !controlled
                        && update.is_header_ready()
                        && let Err(e) = self.begin_if_ready(sink)
                    {
                        self.state = State::Failed;
                        return Err(e);
                    }
                    return Ok(Progress::new(consumed, Event::Metadata(update)).into());
                }
                Pending::Need => {
                    // The same input may hold what the coroutine now asks
                    // for: take it and poll again rather than send the
                    // driver back for bytes it already offered.
                    let mut shared = self.shared.lock();
                    let more = Self::absorb(&mut shared, &input, consumed);
                    if more > consumed {
                        consumed = more;
                        continue;
                    }
                    self.need.clear();
                    self.need.extend_from_slice(&shared.need);
                    return Ok(Progress::need(consumed).into());
                }
                Pending::Stalled => {
                    self.state = State::Failed;
                    return Err(StepError::Codec(E::from(CoroutineError::Stalled)));
                }
            }
        }
    }
}

impl<F, E> IncrementalDecode for CoroutineDecoder<F, E>
where
    F: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    E: core::error::Error + Send + Sync + From<CoroutineError> + 'static,
{
    type Error = E;

    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn Stop,
    ) -> Result<Progress, StepError<E>> {
        if matches!(self.state, State::Failed) {
            return Err(StepError::Finished);
        }
        if self.shared.lock().controlled {
            self.state = State::Failed;
            return Err(StepError::Control(ControlError::UnhandledBoundary));
        }
        match self.advance(input, sink, stop)? {
            ControlledProgress::Decode(progress) => Ok(progress),
            ControlledProgress::Suspended { .. } => {
                self.state = State::Failed;
                Err(StepError::Control(ControlError::UnhandledBoundary))
            }
        }
    }

    fn requested(&self) -> &[Request] {
        match self.state {
            State::Running => &self.need,
            _ => &[],
        }
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.cached_info.as_ref()
    }

    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.shared.lock().descriptor
    }

    fn access_pattern(&self) -> AccessPattern {
        self.access
    }
}

impl<F, E> ControlledIncrementalDecode for CoroutineDecoder<F, E>
where
    F: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    E: core::error::Error + Send + Sync + From<CoroutineError> + 'static,
{
    fn step_controlled(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn Stop,
    ) -> Result<ControlledProgress, StepError<E>> {
        if matches!(self.state, State::Failed) {
            return Err(StepError::Finished);
        }
        if !self.shared.lock().controlled {
            self.state = State::Failed;
            return Err(StepError::Control(ControlError::NotControlled));
        }
        self.advance(input, sink, stop)
    }

    fn resume(
        &mut self,
        id: BoundaryId,
        action: BoundaryAction,
        stop: &dyn Stop,
    ) -> Result<(), StepError<E>> {
        let state = core::mem::replace(&mut self.state, State::Failed);
        if !matches!(state, State::Running) {
            return Err(StepError::Finished);
        }
        stop.check()?;
        let mut shared = self.shared.lock();
        if !shared.controlled {
            return Err(StepError::Control(ControlError::NotControlled));
        }
        if let Some(error) = shared.control_failure {
            return Err(StepError::Control(error));
        }
        let Some(boundary) = shared.boundary.take() else {
            return Err(StepError::Control(ControlError::InvalidReply));
        };
        if boundary.id() != id || !action.permits(boundary.facts()) || shared.reply.is_some() {
            return Err(StepError::Control(ControlError::InvalidReply));
        }
        let facts = boundary.facts();
        if facts.descriptor != shared.descriptor
            || shared
                .info
                .as_ref()
                .is_none_or(|info| info.width != facts.width || info.height != facts.height)
        {
            return Err(StepError::Control(ControlError::InconsistentFacts));
        }
        if action == BoundaryAction::AdoptInterpretation {
            shared.adopted = Some(facts);
        }
        shared.reply = Some((id, action));
        self.state = State::Running;
        Ok(())
    }
}

enum Pending {
    Boundary(DecodeBoundary),
    Rows(crate::context::PendingRows),
    Metadata(MetadataUpdate),
    Need,
    Stalled,
}

/// Route a batch to the sink: final rows through `provide_next_buffer`,
/// provisional rows through `provide_pass_buffer`, and only to a sink that
/// accepts them. The coroutine API (`accepts_provisional` on the context)
/// lets a decoder skip provisional passes nobody wants; sending one anyway
/// is an error rather than a silent drop.
fn copy_rows(
    sink: &mut dyn DecodeRowSink,
    rows: &crate::context::PendingRows,
    accepts_provisional: bool,
) -> Result<(), SinkError> {
    let meta: BatchMeta = rows.meta;
    let mut dst = if meta.is_final {
        sink.provide_next_buffer(meta.y, rows.rows, rows.width, rows.descriptor)?
    } else if accepts_provisional {
        sink.provide_pass_buffer(meta, rows.rows, rows.width, rows.descriptor)?
    } else {
        return Err("decoder wrote provisional rows to a sink that does not accept them".into());
    };
    for r in 0..rows.rows {
        let start = r as usize * rows.stride;
        dst.row_mut(r)
            .copy_from_slice(&rows.data[start..start + rows.stride]);
    }
    Ok(())
}

impl<F, E> CoroutineDecoder<F, E>
where
    F: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    E: core::error::Error + Send + Sync + From<CoroutineError> + 'static,
{
    /// The future completed with `info`: begin if the header never was,
    /// finish the sink, and report `Done`.
    fn finish(
        &mut self,
        sink: &mut dyn DecodeRowSink,
        info: OutputInfo,
        consumed: usize,
    ) -> Result<Progress, StepError<E>> {
        let inconsistent = {
            let shared = self.shared.lock();
            shared.controlled
                && shared.adopted.is_some_and(|facts| {
                    facts.width != info.width
                        || facts.height != info.height
                        || facts.descriptor != Some(info.native_format)
                })
        };
        if inconsistent {
            self.state = State::Failed;
            return Err(StepError::Control(ControlError::InconsistentFacts));
        }
        if let Err(e) = self.begin_if_ready(sink) {
            self.state = State::Failed;
            return Err(e);
        }
        if self.begun
            && let Err(e) = sink.finish()
        {
            self.state = State::Failed;
            return Err(StepError::Sink(e));
        }
        self.state = State::Done(info.clone());
        self.need.clear();
        Ok(Progress::new(consumed, Event::Done(info)))
    }
}
