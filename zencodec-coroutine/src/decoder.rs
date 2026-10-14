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
    BatchMeta, DecodeRowSink, Event, IncrementalDecode, Input, MetadataUpdate, OutputInfo,
    Progress, Request, SinkError, StepError,
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

    fn refresh_info(&mut self) {
        let shared = self.shared.lock();
        if shared.info_generation != self.cached_info_generation {
            self.cached_info = shared.info.clone();
            self.cached_info_generation = shared.info_generation;
        }
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
    fn absorb(&mut self, input: &Input<'_>, from: usize) -> usize {
        let mut shared = self.shared.lock();
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
            self.shared.lock().accepts_provisional = sink.accepts_provisional();
            self.begun = true;
        }
        Ok(())
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
        match &self.state {
            State::Done(info) => return Ok(Progress::new(0, Event::Done(info.clone()))),
            State::Failed => return Err(StepError::Finished),
            State::Running => {}
        }
        if let Err(reason) = stop.check() {
            self.shared.lock().cancelled = Some(reason);
            self.state = State::Failed;
            return Err(StepError::Cancelled(reason));
        }
        let mut consumed = self.absorb(&input, 0);
        loop {
            let waker = Waker::noop();
            let mut cx = Context::from_waker(waker);
            let polled = self.future.as_mut().poll(&mut cx);
            self.refresh_info();
            match polled {
                Poll::Ready(Ok(info)) => {
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
                    return Ok(Progress::new(consumed, Event::Done(info)));
                }
                Poll::Ready(Err(e)) => {
                    self.state = State::Failed;
                    self.need.clear();
                    return Err(StepError::Codec(e));
                }
                Poll::Pending => {}
            }
            let event = {
                let mut shared = self.shared.lock();
                if let Some(rows) = shared.rows.take() {
                    Pending::Rows(rows)
                } else if let Some(update) = shared.metadata.take() {
                    Pending::Metadata(update)
                } else if !shared.need.is_empty() {
                    Pending::Need
                } else {
                    Pending::Stalled
                }
            };
            match event {
                Pending::Rows(rows) => {
                    if let Err(e) = self.begin_if_ready(sink) {
                        self.state = State::Failed;
                        return Err(e);
                    }
                    if !self.begun {
                        self.state = State::Failed;
                        return Err(StepError::Codec(E::from(CoroutineError::RowsBeforeHeader)));
                    }
                    let accepts = self.shared.lock().accepts_provisional;
                    let copied = copy_rows(sink, &rows, accepts);
                    self.shared.lock().spare = rows.data;
                    return match copied {
                        Ok(()) => Ok(Progress::new(
                            consumed,
                            Event::Rows {
                                meta: rows.meta,
                                rows: rows.rows,
                            },
                        )),
                        Err(e) => {
                            self.state = State::Failed;
                            Err(StepError::Sink(e))
                        }
                    };
                }
                Pending::Metadata(update) => {
                    if update.is_header_ready()
                        && let Err(e) = self.begin_if_ready(sink)
                    {
                        self.state = State::Failed;
                        return Err(e);
                    }
                    return Ok(Progress::new(consumed, Event::Metadata(update)));
                }
                Pending::Need => {
                    // The same input may hold what the coroutine now asks
                    // for: take it and poll again rather than send the
                    // driver back for bytes it already offered.
                    let more = self.absorb(&input, consumed);
                    if more > consumed {
                        consumed = more;
                        continue;
                    }
                    let shared = self.shared.lock();
                    self.need.clear();
                    self.need.extend_from_slice(&shared.need);
                    return Ok(Progress::need(consumed));
                }
                Pending::Stalled => {
                    self.state = State::Failed;
                    return Err(StepError::Codec(E::from(CoroutineError::Stalled)));
                }
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

enum Pending {
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
