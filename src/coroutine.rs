//! Compiler-synthesized Sans-I/O coroutine decoders.
//!
//! This module allows codec authors to write straight-line procedural decode logic using
//! Rust's standard `async`/`await` syntax — with **zero runtime, zero Tokio,
//! zero OS threads, and 100% safe Rust**.
//!
//! # How It Works
//!
//! In Rust, `async fn` is a compile-time coroutine generator that `rustc` compiles into an
//! anonymous state-machine `enum` implementing [`core::future::Future`].
//!
//! Instead of `.await` waiting on a network socket, in a [`CoroutineDecoder`], `.await` means:
//! > *"Yield control to the driver: I need a byte range, or my output scanline buffer is full."*
//!
//! The outside driver (Tokio, C# FFI, C library, or local file reader) interacts strictly through
//! the synchronous [`IncrementalDecode`](crate::decode::IncrementalDecode) interface (`push_range`,
//! `requested_ranges`, `pull_batch`).

#![cfg(feature = "std")]

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::sync::Mutex;

use enough::Stop;
use zenpixels::{PixelDescriptor, PixelSlice};

use crate::ImageInfo;
use crate::cost::OutputInfo;
use crate::decode::{
    BatchMeta, ByteRange, IncrementalDecode, MetadataUpdate, PushOutcome, PushStatus,
};
use crate::sink::{DecodeRowSink, SinkError};
use crate::source::SourceError;

// ===========================================================================
// Bridge State
// ===========================================================================

struct PendingBatch {
    meta: BatchMeta,
    data: Vec<u8>,
    width: u32,
    rows: u32,
    stride: usize,
    desc: PixelDescriptor,
}

struct BridgeState {
    /// Active byte ranges requested by the coroutine.
    requested_ranges: Vec<ByteRange>,

    /// Sliding window of chunks pushed by the driver.
    provided_chunks: Vec<(u64, Vec<u8>)>,

    /// Batches yielded by the coroutine waiting to be pulled.
    pending_batches: Vec<PendingBatch>,

    /// Max pending batches allowed before backpressure pauses the coroutine.
    max_pending_batches: usize,

    /// Image metadata set by the coroutine.
    info: Option<ImageInfo>,

    /// Output descriptor declared by the coroutine.
    output_descriptor: Option<PixelDescriptor>,

    /// Metadata notification event.
    metadata_update: MetadataUpdate,

    /// Whether the coroutine completed successfully.
    complete: bool,

    /// Cooperative cancellation flag.
    cancelled: bool,
}

impl BridgeState {
    fn new(max_pending_batches: usize) -> Self {
        Self {
            requested_ranges: Vec::new(),
            provided_chunks: Vec::new(),
            pending_batches: Vec::new(),
            max_pending_batches: max_pending_batches.max(1),
            info: None,
            output_descriptor: None,
            metadata_update: MetadataUpdate::None,
            complete: false,
            cancelled: false,
        }
    }

    /// Try to read exact bytes at `offset` from provided chunks.
    /// Supports spans that cross multiple contiguous provided chunks.
    fn try_read_exact(&self, offset: u64, out: &mut [u8]) -> bool {
        if out.is_empty() {
            return true;
        }
        let req_len = out.len();
        // First pass: verify full coverage across provided chunks
        let mut pos = offset;
        let end = offset.saturating_add(req_len as u64);
        while pos < end {
            let mut found = false;
            for (chunk_off, data) in &self.provided_chunks {
                let chunk_end = chunk_off.saturating_add(data.len() as u64);
                if pos >= *chunk_off && pos < chunk_end {
                    pos = chunk_end;
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
        }

        // Second pass: copy the bytes from the matching chunks
        let mut filled = 0;
        while filled < req_len {
            let target_off = offset + filled as u64;
            for (chunk_off, data) in &self.provided_chunks {
                let chunk_end = chunk_off.saturating_add(data.len() as u64);
                if target_off >= *chunk_off && target_off < chunk_end {
                    let start_in_chunk = (target_off - chunk_off) as usize;
                    let avail_in_chunk = data.len() - start_in_chunk;
                    let to_copy = avail_in_chunk.min(req_len - filled);
                    out[filled..filled + to_copy]
                        .copy_from_slice(&data[start_in_chunk..start_in_chunk + to_copy]);
                    filled += to_copy;
                    break;
                }
            }
        }
        true
    }

    /// Provide a new chunk from the driver.
    fn provide_chunk(&mut self, offset: u64, chunk: &[u8]) {
        // Retain up to 32 recent chunks for sub-slice, stitching, and coalesced lookups
        if self.provided_chunks.len() >= 32 {
            self.provided_chunks.remove(0);
        }
        self.provided_chunks.push((offset, chunk.to_vec()));
    }
}

// ===========================================================================
// Coroutine Context (The Codec Author's API)
// ===========================================================================

/// Context provided to a Sans-I/O coroutine decoder.
///
/// Codec authors use straight-line `async`/`await` methods on this context
/// to read byte ranges and yield scanline batches.
#[derive(Clone)]
pub struct CoroutineContext {
    state: Arc<Mutex<BridgeState>>,
    read_cursor: u64,
}

impl CoroutineContext {
    /// Read exactly `out.len()` bytes at `offset`.
    ///
    /// If the byte range is not yet available, this pauses the coroutine with
    /// `PushStatus::NeedRanges` until the driver delivers the requested span.
    pub async fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
        ReadExactFuture {
            state: Arc::clone(&self.state),
            offset,
            out,
        }
        .await
    }

    /// Read `len` bytes starting at `offset` into an allocated `Vec<u8>`.
    pub async fn read_range_vec(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, SourceError> {
        let mut buf = alloc::vec![0u8; len as usize];
        self.read_exact_at(offset, &mut buf).await?;
        Ok(buf)
    }

    /// Request multiple non-contiguous byte ranges in parallel.
    ///
    /// The driver receives all requested spans in a single round, enabling
    /// concurrent HTTP/2 range queries or parallel file reads.
    pub async fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), SourceError> {
        ReadManyFuture {
            state: Arc::clone(&self.state),
            reqs,
        }
        .await
    }

    /// Read the next `out.len()` bytes sequentially from the stream cursor.
    pub async fn read_exact(&mut self, out: &mut [u8]) -> Result<(), SourceError> {
        let offset = self.read_cursor;
        self.read_cursor = self.read_cursor.saturating_add(out.len() as u64);
        self.read_exact_at(offset, out).await
    }

    /// Yield a batch of decoded scanlines with pass metadata.
    ///
    /// Honors backpressure: if the driver's output queue is full, this pauses
    /// until the driver pulls the pending scanlines.
    pub async fn yield_batch(
        &mut self,
        meta: BatchMeta,
        slice: &PixelSlice<'_>,
    ) -> Result<(), SinkError> {
        let rows = slice.rows();
        let width = slice.width();
        let desc = slice.descriptor();
        let stride = width as usize * desc.bytes_per_pixel();
        let mut slice_data = Vec::with_capacity(stride * rows as usize);
        for r in 0..rows {
            slice_data.extend_from_slice(slice.row(r));
        }

        YieldBatchFuture {
            state: Arc::clone(&self.state),
            meta,
            slice_data,
            width,
            rows,
            stride,
            desc,
        }
        .await
    }

    /// Yield a batch of decoded scanlines with baseline metadata (`pass = 0`, `is_final = true`).
    pub async fn yield_scanlines(
        &mut self,
        y: u32,
        slice: &PixelSlice<'_>,
    ) -> Result<(), SinkError> {
        self.yield_batch(BatchMeta::new(y, true), slice).await
    }

    /// Set image metadata once header parsing completes.
    pub fn set_info(&mut self, info: ImageInfo) {
        let mut state = self.state.lock().unwrap();
        state.info = Some(info);
        state.metadata_update = MetadataUpdate::HeaderReady;
    }

    /// Declare the negotiated output pixel descriptor.
    pub fn set_output_descriptor(&mut self, desc: PixelDescriptor) {
        let mut state = self.state.lock().unwrap();
        state.output_descriptor = Some(desc);
    }

    /// Returns `true` if cancellation has been requested by the driver.
    pub fn is_cancelled(&self) -> bool {
        self.state.lock().unwrap().cancelled
    }
}

// ===========================================================================
// Coroutine Futures
// ===========================================================================

struct ReadExactFuture<'a> {
    state: Arc<Mutex<BridgeState>>,
    offset: u64,
    out: &'a mut [u8],
}

impl Future for ReadExactFuture<'_> {
    type Output = Result<(), SourceError>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let state_arc = Arc::clone(&self.state);
        let mut state = state_arc.lock().unwrap();
        if state.cancelled {
            return Poll::Ready(Err(SourceError::Io {
                message: "operation cancelled".into(),
            }));
        }

        let offset = self.offset;
        let out_slice = &mut *self.out;
        if state.try_read_exact(offset, out_slice) {
            state.requested_ranges.clear();
            Poll::Ready(Ok(()))
        } else {
            let req = ByteRange::new(offset, out_slice.len() as u64);
            state.requested_ranges = alloc::vec![req];
            Poll::Pending
        }
    }
}

struct ReadManyFuture<'a, 'b> {
    state: Arc<Mutex<BridgeState>>,
    reqs: &'a mut [(u64, &'b mut [u8])],
}

impl Future for ReadManyFuture<'_, '_> {
    type Output = Result<(), SourceError>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let state_arc = Arc::clone(&self.state);
        let mut state = state_arc.lock().unwrap();
        if state.cancelled {
            return Poll::Ready(Err(SourceError::Io {
                message: "operation cancelled".into(),
            }));
        }

        let mut all_satisfied = true;
        let mut missing_ranges = Vec::new();

        for (offset, buf) in self.reqs.iter_mut() {
            if !state.try_read_exact(*offset, buf) {
                all_satisfied = false;
                missing_ranges.push(ByteRange::new(*offset, buf.len() as u64));
            }
        }

        if all_satisfied {
            state.requested_ranges.clear();
            Poll::Ready(Ok(()))
        } else {
            state.requested_ranges = missing_ranges;
            Poll::Pending
        }
    }
}

struct YieldBatchFuture {
    state: Arc<Mutex<BridgeState>>,
    meta: BatchMeta,
    slice_data: Vec<u8>,
    width: u32,
    rows: u32,
    stride: usize,
    desc: PixelDescriptor,
}

impl Future for YieldBatchFuture {
    type Output = Result<(), SinkError>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let state_arc = Arc::clone(&self.state);
        let mut state = state_arc.lock().unwrap();
        if state.cancelled {
            let err_box: SinkError = Box::new(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "coroutine decode cancelled",
            ));
            return Poll::Ready(Err(err_box));
        }

        if state.pending_batches.len() >= state.max_pending_batches {
            // Backpressure: paused until driver drains output
            Poll::Pending
        } else {
            let this = self.as_mut().get_mut();
            state.pending_batches.push(PendingBatch {
                meta: this.meta,
                data: core::mem::take(&mut this.slice_data),
                width: this.width,
                rows: this.rows,
                stride: this.stride,
                desc: this.desc,
            });
            Poll::Ready(Ok(()))
        }
    }
}

// ===========================================================================
// CoroutineDecoder (The IncrementalDecode Implementation)
// ===========================================================================

/// A Sans-I/O incremental decoder driven by a compiler-generated coroutine.
///
/// Wraps an `async` decode function and exposes the standard synchronous
/// [`IncrementalDecode`] trait surface to drivers.
pub struct CoroutineDecoder<F, E = SourceError> {
    state: Arc<Mutex<BridgeState>>,
    coroutine: Pin<Box<F>>,
    cached_ranges: Vec<ByteRange>,
    cached_info: Option<ImageInfo>,
    cached_descriptor: Option<PixelDescriptor>,
    active_slice_buf: Option<Vec<u8>>,
    stream_in_cursor: u64,
    _err: PhantomData<fn() -> E>,
}

impl<F, E> CoroutineDecoder<F, E>
where
    F: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    E: core::error::Error + Send + Sync + From<SinkError> + 'static,
{
    /// Create a new coroutine decoder from an async factory function.
    pub fn new<Init>(init: Init) -> Self
    where
        Init: FnOnce(CoroutineContext) -> F,
    {
        Self::with_max_pending_batches(1, init)
    }

    /// Create a new coroutine decoder with a custom backpressure capacity.
    pub fn with_max_pending_batches<Init>(max_batches: usize, init: Init) -> Self
    where
        Init: FnOnce(CoroutineContext) -> F,
    {
        let state = Arc::new(Mutex::new(BridgeState::new(max_batches)));
        let ctx = CoroutineContext {
            state: Arc::clone(&state),
            read_cursor: 0,
        };
        let coroutine = Box::pin(init(ctx));
        Self {
            state,
            coroutine,
            cached_ranges: Vec::new(),
            cached_info: None,
            cached_descriptor: None,
            active_slice_buf: None,
            stream_in_cursor: 0,
            _err: PhantomData,
        }
    }

    fn poll_coroutine(&mut self) -> Result<PushStatus, E> {
        {
            let state = self.state.lock().unwrap();
            if state.complete {
                return if !state.pending_batches.is_empty() {
                    Ok(PushStatus::OutputReady)
                } else {
                    Ok(PushStatus::Complete)
                };
            }
        }

        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        let status = match self.coroutine.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(_output_info)) => {
                let mut state = self.state.lock().unwrap();
                state.complete = true;
                if !state.pending_batches.is_empty() {
                    PushStatus::OutputReady
                } else {
                    PushStatus::Complete
                }
            }
            Poll::Ready(Err(e)) => return Err(e),
            Poll::Pending => {
                let state = self.state.lock().unwrap();
                if state.pending_batches.len() >= state.max_pending_batches {
                    PushStatus::BlockedOnOutput
                } else if !state.pending_batches.is_empty() {
                    PushStatus::OutputReady
                } else if !state.requested_ranges.is_empty() {
                    PushStatus::NeedRanges
                } else {
                    PushStatus::NeedMoreInput
                }
            }
        };

        // Synchronize cached metadata for safe, zero-unsafe trait references
        let state = self.state.lock().unwrap();
        self.cached_ranges = state.requested_ranges.clone();
        self.cached_info = state.info.clone();
        self.cached_descriptor = state.output_descriptor;

        Ok(status)
    }
}

impl<F, E> IncrementalDecode for CoroutineDecoder<F, E>
where
    F: Future<Output = Result<OutputInfo, E>> + Send + 'static,
    E: core::error::Error + Send + Sync + From<SinkError> + 'static,
{
    type Error = E;

    fn push_chunk(
        &mut self,
        chunk: &[u8],
        is_eof: bool,
        stop: Option<&dyn Stop>,
    ) -> Result<PushOutcome, Self::Error> {
        let offset = self.stream_in_cursor;
        self.stream_in_cursor = self.stream_in_cursor.saturating_add(chunk.len() as u64);
        self.push_range(offset, chunk, is_eof, stop)
    }

    fn requested_ranges(&self) -> &[ByteRange] {
        &self.cached_ranges
    }

    fn push_range(
        &mut self,
        offset: u64,
        chunk: &[u8],
        _is_eof: bool,
        stop: Option<&dyn Stop>,
    ) -> Result<PushOutcome, Self::Error> {
        {
            let mut state = self.state.lock().unwrap();
            if let Some(s) = stop
                && s.check().is_err()
            {
                state.cancelled = true;
            }
            state.provide_chunk(offset, chunk);
        }

        let status = self.poll_coroutine()?;
        let meta = {
            let mut state = self.state.lock().unwrap();
            let m = state.metadata_update;
            state.metadata_update = MetadataUpdate::None;
            m
        };

        Ok(PushOutcome::new(chunk.len(), status).with_metadata(meta))
    }

    fn pull_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error> {
        let should_poll = {
            let state = self.state.lock().unwrap();
            state.pending_batches.is_empty() && !state.complete
        };
        if should_poll {
            let _ = self.poll_coroutine()?;
        }

        let mut state = self.state.lock().unwrap();
        if state.pending_batches.is_empty() {
            return Ok(None);
        }

        let batch = state.pending_batches.remove(0);
        let y = batch.meta.y;
        self.active_slice_buf = Some(batch.data);

        let data_ref = self.active_slice_buf.as_ref().unwrap();
        let slice = PixelSlice::new(data_ref, batch.width, batch.rows, batch.stride, batch.desc)
            .expect("coroutine decoder yielded valid pixel buffer");

        Ok(Some((y, slice)))
    }

    fn pull_batch_meta(&mut self) -> Result<Option<(BatchMeta, PixelSlice<'_>)>, Self::Error> {
        let should_poll = {
            let state = self.state.lock().unwrap();
            state.pending_batches.is_empty() && !state.complete
        };
        if should_poll {
            let _ = self.poll_coroutine()?;
        }

        let mut state = self.state.lock().unwrap();
        if state.pending_batches.is_empty() {
            return Ok(None);
        }

        let batch = state.pending_batches.remove(0);
        let meta = batch.meta;
        self.active_slice_buf = Some(batch.data);

        let data_ref = self.active_slice_buf.as_ref().unwrap();
        let slice = PixelSlice::new(data_ref, batch.width, batch.rows, batch.stride, batch.desc)
            .expect("coroutine decoder yielded valid pixel buffer");

        Ok(Some((meta, slice)))
    }

    fn pull_batch_into(
        &mut self,
        sink: &mut dyn DecodeRowSink,
    ) -> Result<Option<u32>, Self::Error> {
        let should_poll = {
            let state = self.state.lock().unwrap();
            state.pending_batches.is_empty() && !state.complete
        };
        if should_poll {
            let _ = self.poll_coroutine()?;
        }

        let mut state = self.state.lock().unwrap();
        if state.pending_batches.is_empty() {
            return Ok(None);
        }

        let batch = state.pending_batches.remove(0);
        let mut dst = sink
            .provide_next_buffer(batch.meta.y, batch.rows, batch.width, batch.desc)
            .map_err(Self::wrap_sink_error)?;

        for r in 0..batch.rows {
            let src_start = r as usize * batch.stride;
            let src_end = src_start + (batch.width as usize * batch.desc.bytes_per_pixel());
            dst.row_mut(r)
                .copy_from_slice(&batch.data[src_start..src_end]);
        }

        Ok(Some(batch.rows))
    }

    fn wrap_sink_error(err: SinkError) -> Self::Error {
        E::from(err)
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.cached_info.as_ref()
    }

    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.cached_descriptor
    }

    fn is_complete(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.complete && state.pending_batches.is_empty()
    }
}
