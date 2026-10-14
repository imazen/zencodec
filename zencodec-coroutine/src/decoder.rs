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
    BatchMeta, ByteRange, DecodeRowSink, IncrementalDecode, MetadataUpdate, OutputInfo, SinkError,
    Step,
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
    need: Vec<ByteRange>,
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

    fn begin_if_ready(&mut self, sink: &mut dyn DecodeRowSink) -> Result<(), E> {
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
            sink.begin(w, h, desc)
                .map_err(|e| E::from(CoroutineError::Sink(e)))?;
            self.begun = true;
        }
        Ok(())
    }

    fn fail<T>(&mut self, err: E) -> Result<T, E> {
        self.state = State::Failed;
        Err(err)
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

impl<F, E> IncrementalDecode for CoroutineDecoder<F, E>
where
    F: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    E: core::error::Error + Send + Sync + From<CoroutineError> + 'static,
{
    type Error = E;

    fn feed(&mut self, offset: u64, bytes: &[u8], eof: bool) -> Result<usize, E> {
        if !matches!(self.state, State::Running) {
            // A finished decoder takes nothing more; the driver must not
            // re-offer, so report the bytes as accepted.
            return Ok(bytes.len());
        }
        // Everything fed is kept until read, under the cap: a stream
        // driver cannot re-read what it hands over, and a range driver
        // only hands over what was asked for.
        Ok(self.shared.lock().input.feed(offset, bytes, eof))
    }

    fn step(
        &mut self,
        sink: &mut dyn DecodeRowSink,
        stop: Option<&dyn Stop>,
    ) -> Result<Step<'_>, E> {
        match &self.state {
            State::Done(info) => return Ok(Step::Done(info.clone())),
            State::Failed => return Err(E::from(CoroutineError::Finished)),
            State::Running => {}
        }
        if let Some(stop) = stop
            && let Err(reason) = stop.check()
        {
            self.shared.lock().cancelled = Some(reason);
            return self.fail(E::from(CoroutineError::Cancelled(reason)));
        }
        self.shared.lock().accepts_provisional = sink.accepts_provisional();

        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let polled = self.future.as_mut().poll(&mut cx);
        self.refresh_info();
        match polled {
            Poll::Ready(Ok(info)) => {
                self.begin_if_ready(sink).or_else(|e| self.fail(e))?;
                if self.begun {
                    sink.finish()
                        .map_err(|e| E::from(CoroutineError::Sink(e)))
                        .or_else(|e| self.fail(e))?;
                }
                self.state = State::Done(info.clone());
                Ok(Step::Done(info))
            }
            Poll::Ready(Err(e)) => self.fail(e),
            Poll::Pending => {
                let event = {
                    let mut shared = self.shared.lock();
                    if let Some(rows) = shared.rows.take() {
                        Pending::Rows(rows)
                    } else if let Some(update) = shared.metadata.take() {
                        Pending::Metadata(update)
                    } else if shared.need_len {
                        Pending::NeedLen
                    } else if !shared.need.is_empty() {
                        Pending::Need(shared.need.clone())
                    } else {
                        Pending::Stalled
                    }
                };
                match event {
                    Pending::Rows(rows) => {
                        self.begin_if_ready(sink).or_else(|e| self.fail(e))?;
                        let copied = copy_rows(sink, &rows);
                        self.shared.lock().spare = rows.data;
                        match copied {
                            Ok(()) => Ok(Step::Rows {
                                meta: rows.meta,
                                rows: rows.rows,
                            }),
                            Err(e) => self.fail(E::from(CoroutineError::Sink(e))),
                        }
                    }
                    Pending::Metadata(update) => {
                        if update.is_header_ready() {
                            self.begin_if_ready(sink).or_else(|e| self.fail(e))?;
                        }
                        Ok(Step::Metadata(update))
                    }
                    Pending::NeedLen => Ok(Step::NeedLen),
                    Pending::Need(ranges) => {
                        self.need = ranges;
                        Ok(Step::Need(&self.need))
                    }
                    Pending::Stalled => self.fail(E::from(CoroutineError::Stalled)),
                }
            }
        }
    }

    fn wrap_sink_error(err: SinkError) -> E {
        E::from(CoroutineError::Sink(err))
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
    NeedLen,
    Need(Vec<ByteRange>),
    Stalled,
}

fn copy_rows(
    sink: &mut dyn DecodeRowSink,
    rows: &crate::context::PendingRows,
) -> Result<(), SinkError> {
    let meta: BatchMeta = rows.meta;
    let mut dst = if meta.is_final && meta.pass == 0 {
        sink.provide_next_buffer(meta.y, rows.rows, rows.width, rows.descriptor)?
    } else {
        sink.provide_pass_buffer(meta, rows.rows, rows.width, rows.descriptor)?
    };
    for r in 0..rows.rows {
        let start = r as usize * rows.stride;
        dst.row_mut(r)
            .copy_from_slice(&rows.data[start..start + rows.stride]);
    }
    Ok(())
}
