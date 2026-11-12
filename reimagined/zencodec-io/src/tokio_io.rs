//! Async drivers on Tokio.
//!
//! The codec contract has no `async` in it, and these drivers are why it
//! needs none. A codec step is plain CPU work that never waits, so the driver
//! decides where it runs ([`Offload`]) and the only things that await are the
//! reads and writes around it.
//!
//! What the contract's shape buys here:
//!
//! - **A batch hand-off moves one pointer.** The decoder, its sink and the
//!   input window live in one box that goes to the worker and comes back.
//!   The drivers are not generic over the codec, so there is one copy of
//!   this code however many codecs exist.
//! - **Reads are planned, then awaited.** What to read is worked out from
//!   the decoder's requests before the first `.await`, as owned data, so no
//!   borrow of the decoder is held across one.
//! - **Finishing is a step.** The encoder's final work runs in the last
//!   batch, on the worker, like every other step.
//! - **Writes overlap computation.** An encoder's bytes are taken out as an
//!   owned vector, so batch *n*'s bytes go to the socket while batch *n+1*
//!   computes. Two vectors alternate; a steady transcode allocates no
//!   output buffers.
//! - **Bytes already in memory are borrowed.** A source that reports
//!   [`as_slice`](AsyncSource::as_slice) is never read by an inline driver.
//! - **The buffered route is one hop.** Decoders are `'static` and borrow
//!   their input per step, so [`decode_buffered`] and [`transcode_buffered`]
//!   are a single `spawn_blocking` around the in-memory driver.
//!
//! Dropping a driver's future does not stop a batch that has already been
//! handed to a worker: signal the shared pulse as well.

use crate::drive::{Batch, DriveOptions, Next, RangeRead, Session, Window};
use crate::encoder_sink::EncoderSink;
use crate::memory::{SliceRun, decode_slice};
use crate::read::{sink_error, source_error};
use std::future::Future;
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zencodec::decode::{Decoder, Sink};
use zencodec::{CodecError, ErrorKind, Result, SharedPulse, check};

/// An input that is read at a position, asynchronously.
pub trait AsyncSource: Send {
    /// Read up to `out.len()` bytes at `offset`; `Ok(0)` at the end.
    fn read_at(
        &mut self,
        offset: u64,
        out: &mut [u8],
    ) -> impl Future<Output = io::Result<usize>> + Send;

    /// The total length, when the source knows it.
    fn total_len(&mut self) -> impl Future<Output = io::Result<Option<u64>>> + Send {
        async { Ok(None) }
    }

    /// The whole input, when it is already in memory. An inline driver
    /// borrows it instead of reading. A worker cannot borrow across threads:
    /// for resident bytes and a worker, use [`decode_buffered`] or
    /// [`transcode_buffered`], which hand the bytes over once.
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }

    /// Read several spans. A source with slow round trips (HTTP ranges)
    /// overrides this to fetch them concurrently. A span the input ends
    /// inside is shortened.
    fn read_ranges(
        &mut self,
        ranges: &mut [RangeRead],
    ) -> impl Future<Output = io::Result<()>> + Send {
        async move {
            for range in ranges {
                let mut got = 0;
                while got < range.buf.len() {
                    match self
                        .read_at(range.offset + got as u64, &mut range.buf[got..])
                        .await?
                    {
                        0 => break,
                        n => got += n,
                    }
                }
                range.buf.truncate(got);
            }
            Ok(())
        }
    }
}

/// Bytes already in memory, as an async source.
#[derive(Clone, Debug)]
pub struct InMemory<B>(pub B);

impl<B: AsRef<[u8]> + Send> AsyncSource for InMemory<B> {
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        let bytes = self.0.as_ref();
        let from = (offset.min(bytes.len() as u64)) as usize;
        let n = out.len().min(bytes.len() - from);
        out[..n].copy_from_slice(&bytes[from..from + n]);
        Ok(n)
    }

    async fn total_len(&mut self) -> io::Result<Option<u64>> {
        Ok(Some(self.0.as_ref().len() as u64))
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(self.0.as_ref())
    }
}

/// A forward-only async reader as a source. A read behind the current
/// position fails: pair it with a decoder whose plan is sequential.
#[derive(Debug)]
pub struct Streamed<R> {
    reader: R,
    position: u64,
}

impl<R> Streamed<R> {
    /// Wrap a reader positioned at the start of the image.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            position: 0,
        }
    }
}

impl<R: AsyncRead + Unpin + Send> AsyncSource for Streamed<R> {
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        if offset < self.position {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "backward read on a forward-only source",
            ));
        }
        let mut scratch = [0u8; 4096];
        while self.position < offset {
            let want = (offset - self.position).min(scratch.len() as u64) as usize;
            match self.reader.read(&mut scratch[..want]).await? {
                0 => return Ok(0),
                n => self.position += n as u64,
            }
        }
        let n = self.reader.read(out).await?;
        self.position += n as u64;
        Ok(n)
    }
}

/// A unit of codec work handed to an executor.
pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// Where codec steps run.
#[derive(Clone, Default)]
#[non_exhaustive]
pub enum Offload {
    /// On the task that polls the driver. For short steps on small images;
    /// costs nothing, and blocks that runtime thread while a step runs.
    Inline,
    /// On Tokio's blocking pool.
    #[default]
    Blocking,
    /// On an executor of the caller's choosing, such as a CPU pool. It must
    /// run every job it is given.
    Custom(Arc<dyn Fn(Job) + Send + Sync>),
}

impl std::fmt::Debug for Offload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Inline => "Inline",
            Self::Blocking => "Blocking",
            Self::Custom(_) => "Custom",
        })
    }
}

/// Work that has been handed off and, unless inline, is already running.
enum Pending<T, F> {
    Inline(F),
    Blocking(tokio::task::JoinHandle<T>),
    Custom(tokio::sync::oneshot::Receiver<T>),
}

impl Offload {
    fn start<T, F>(&self, work: F) -> Pending<T, F>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        match self {
            Self::Inline => Pending::Inline(work),
            Self::Blocking => Pending::Blocking(tokio::task::spawn_blocking(work)),
            Self::Custom(run) => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                run(Box::new(move || {
                    let _ = tx.send(work());
                }));
                Pending::Custom(rx)
            }
        }
    }
}

impl<T, F: FnOnce() -> T> Pending<T, F> {
    async fn finish(self) -> Result<T> {
        let lost = |cause: Option<tokio::task::JoinError>| {
            let error = CodecError::new(ErrorKind::Internal)
                .with_message("codec work did not complete on its worker");
            match cause {
                Some(cause) => error.with_native(cause).at(),
                None => error.at(),
            }
        };
        match self {
            Self::Inline(work) => Ok(work()),
            Self::Blocking(handle) => handle.await.map_err(|e| lost(Some(e))),
            Self::Custom(rx) => rx.await.map_err(|_| lost(None)),
        }
    }
}

/// One batch of steps: the session goes in and comes back with how it ended.
fn batch<S: Sink>(
    mut session: Box<Session<S>>,
    quota: usize,
    pulse: &SharedPulse,
    done: impl FnOnce(&mut S) -> Result<()>,
) -> (Box<Session<S>>, Result<Batch>) {
    let mut end = session.run(quota, pulse);
    if matches!(end, Ok(Batch::Done)) {
        // Whatever finishing the sink needs runs here, on the worker.
        if let Err(e) = check(pulse).and_then(|()| done(&mut session.sink)) {
            end = Err(e);
        }
    }
    (session, end)
}

/// Perform the read a session asked for.
async fn read_next<Src: AsyncSource + ?Sized>(
    window: &mut Window,
    source: &mut Src,
    next: Next,
) -> Result<()> {
    match next {
        Next::Ready => {}
        Next::Read { offset, len } => {
            let n = source
                .read_at(offset, window.spare(len))
                .await
                .map_err(source_error)?;
            window.commit(len, n);
        }
        Next::Ranges(mut reads) => {
            let wanted: Vec<usize> = reads.iter().map(|r| r.buf.len()).collect();
            source.read_ranges(&mut reads).await.map_err(source_error)?;
            window.store_ranges(reads, &wanted);
        }
        Next::Len => match source.total_len().await.map_err(source_error)? {
            Some(total) => window.set_total(total),
            None => {
                return Err(CodecError::new(ErrorKind::Source)
                    .with_message("the source cannot report its length")
                    .at());
            }
        },
    }
    Ok(())
}

/// Decode from an async source into `sink`.
///
/// Returns the decoder, whose info is now complete, and the sink.
// Written as a function returning a future, not an `async fn`, so that the
// `Send` bound is checked here, once, for every source type.
#[allow(
    clippy::manual_async_fn,
    reason = "check the Send future bound at the driver boundary"
)]
pub fn decode<'a, S, Src>(
    decoder: Box<dyn Decoder>,
    source: &'a mut Src,
    sink: S,
    offload: Offload,
    opts: DriveOptions,
    pulse: SharedPulse,
) -> impl Future<Output = Result<(Box<dyn Decoder>, S)>> + Send + 'a
where
    S: Sink + Send + 'static,
    Src: AsyncSource + ?Sized,
{
    async move {
        if let (Offload::Inline, Some(bytes)) = (&offload, source.as_slice()) {
            // Resident bytes on the polling task: borrow them, read nothing.
            let (mut decoder, mut sink, mut run) = (decoder, sink, SliceRun::new());
            loop {
                check(&pulse)?;
                if run.run(
                    &mut *decoder,
                    bytes,
                    &mut sink,
                    opts.steps_per_batch,
                    || check(&pulse),
                )? {
                    return Ok((decoder, sink));
                }
                tokio::task::yield_now().await;
            }
        }
        let mut session = Box::new(Session::new(decoder, sink, &opts));
        loop {
            check(&pulse)?;
            let shared = pulse.clone();
            let quota = opts.steps_per_batch;
            let (back, end) = offload
                .start(move || batch(session, quota, &shared, |_| Ok(())))
                .finish()
                .await?;
            session = back;
            match end? {
                Batch::Done => {
                    let session = *session;
                    return Ok((session.decoder, session.sink));
                }
                Batch::Yield => tokio::task::yield_now().await,
                Batch::Read(next) => read_next(&mut session.window, source, next).await?,
            }
        }
    }
}

/// Write `carry` and leave it empty. Nothing is published once the pulse
/// has stopped.
async fn publish<W: AsyncWrite + Unpin + Send + ?Sized>(
    out: &mut W,
    carry: &mut Vec<u8>,
    written: &mut u64,
    pulse: &SharedPulse,
) -> Result<()> {
    if !carry.is_empty() {
        check(pulse)?;
        out.write_all(carry).await.map_err(sink_error)?;
        *written += carry.len() as u64;
        carry.clear();
    }
    Ok(())
}

/// Close out a transcode whose bytes have all been written.
async fn finish_output<W: AsyncWrite + Unpin + Send + ?Sized>(
    out: &mut W,
    sink: &mut EncoderSink,
    written: u64,
    pulse: &SharedPulse,
) -> Result<u64> {
    if !sink.take_patches().is_empty() {
        return Err(CodecError::new(ErrorKind::Sink)
            .with_message("an async writer cannot be patched")
            .at());
    }
    check(pulse)?;
    out.flush().await.map_err(sink_error)?;
    Ok(written)
}

/// Decode from an async source and encode to an async writer, row by row.
///
/// Returns the bytes written. On an error, bytes already written are a
/// prefix of a file that will not be finished: abandon the destination.
#[allow(
    clippy::manual_async_fn,
    reason = "check the Send future bound at the driver boundary"
)]
pub fn transcode<'a, Src, W>(
    decoder: Box<dyn Decoder>,
    sink: EncoderSink,
    source: &'a mut Src,
    out: &'a mut W,
    offload: Offload,
    opts: DriveOptions,
    pulse: SharedPulse,
) -> impl Future<Output = Result<u64>> + Send + 'a
where
    Src: AsyncSource + ?Sized,
    W: AsyncWrite + Unpin + Send + ?Sized,
{
    async move {
        if let (Offload::Inline, Some(bytes)) = (&offload, source.as_slice()) {
            // Resident bytes on the polling task: borrow them, read nothing,
            // and reuse one output vector.
            let (mut decoder, mut sink, mut run) = (decoder, sink, SliceRun::new());
            let mut written = 0u64;
            loop {
                check(&pulse)?;
                let done = run.run(
                    &mut *decoder,
                    bytes,
                    &mut sink,
                    opts.steps_per_batch,
                    || check(&pulse),
                )?;
                if done {
                    check(&pulse)?;
                    sink.finish()?;
                }
                let mut chunk = sink.take_output();
                publish(out, &mut chunk, &mut written, &pulse).await?;
                sink.recycle(chunk);
                if done {
                    return finish_output(out, &mut sink, written, &pulse).await;
                }
                tokio::task::yield_now().await;
            }
        }
        let mut session = Box::new(Session::new(decoder, sink, &opts));
        // Bytes the last batch made, and the vector the batch before used.
        let (mut carry, mut spare) = (Vec::new(), Vec::new());
        let mut written = 0u64;
        loop {
            check(&pulse)?;
            if !opts.pipeline_writes {
                publish(out, &mut carry, &mut written, &pulse).await?;
            }
            // The encoder gets the spare vector to fill while `carry` is
            // on its way out.
            session.sink.recycle(std::mem::take(&mut spare));
            let shared = pulse.clone();
            let quota = opts.steps_per_batch;
            let pending =
                offload.start(move || batch(session, quota, &shared, EncoderSink::finish));
            // With a worker, this write overlaps the batch just started.
            publish(out, &mut carry, &mut written, &pulse).await?;
            spare = std::mem::take(&mut carry);
            let (back, end) = pending.finish().await?;
            session = back;
            let end = end?;
            carry = session.sink.take_output();
            match end {
                Batch::Done => {
                    publish(out, &mut carry, &mut written, &pulse).await?;
                    return finish_output(out, &mut session.sink, written, &pulse).await;
                }
                Batch::Yield => tokio::task::yield_now().await,
                Batch::Read(next) => {
                    // Do not sit on finished bytes while waiting for input.
                    publish(out, &mut carry, &mut written, &pulse).await?;
                    read_next(&mut session.window, source, next).await?;
                }
            }
        }
    }
}

/// Decode bytes already in memory on the blocking pool: one hand-off.
pub async fn decode_buffered<S, B>(
    decoder: Box<dyn Decoder>,
    bytes: B,
    mut sink: S,
) -> Result<(Box<dyn Decoder>, S)>
where
    S: Sink + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
{
    Offload::Blocking
        .start(move || decode_slice(decoder, bytes.as_ref(), &mut sink).map(|d| (d, sink)))
        .finish()
        .await?
}

/// Transcode bytes already in memory on the blocking pool: one hand-off.
/// The encoder's output allocation is returned as is.
pub async fn transcode_buffered<B>(
    decoder: Box<dyn Decoder>,
    bytes: B,
    mut sink: EncoderSink,
) -> Result<Vec<u8>>
where
    B: AsRef<[u8]> + Send + 'static,
{
    Offload::Blocking
        .start(move || {
            decode_slice(decoder, bytes.as_ref(), &mut sink)?;
            sink.finish()?;
            Ok(sink.take_output())
        })
        .finish()
        .await?
}
