//! Dedicated-thread bridge between an async runtime and a synchronous decoder.
//!
//! The decoder runs on its own OS thread and never touches the runtime.
//! The async side owns the transport: it awaits the decoder's byte requests
//! on a channel, fetches them however it likes (HTTP Range, a body stream,
//! an object store, concurrently with a `JoinSet`), and sends the bytes
//! back. Rows flow out through whatever [`DecodeRowSink`] the caller moves
//! onto the thread; [`ChannelRowSink`] streams them back to the async side
//! strip by strip with backpressure.
//!
//! This is the shape that keeps CPU work off runtime workers without
//! `spawn_blocking` per chunk: one thread per decode (or one from a pool the
//! caller supplies), two bounded channels, and no `async` anywhere in codec
//! code. Dropping the async future closes the request channel and the
//! thread exits at its next request. A [`SharedPulse`] cancels it at the
//! next step and inside every blocking wait (for a fetch reply, or for a
//! slow strip consumer), and receives the row total and per-strip progress.
//! A caller with only a `StopToken` passes
//! `how_far::StopOnly::new(token).share()`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;

use enough::{Stop, StopReason};
use how_far::{Pulse, SharedPulse, Total};
use tokio::sync::{mpsc, oneshot};
use zencodec::decode::{
    BatchMeta, BoxedError, ByteRange, DecodeRowSink, OutputInfo, SinkError, Step,
};
use zencodec::source::{DynReadAt, SourceError};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSliceMut};

use crate::drivable::Drivable;
use crate::feeder::Feeder;

/// Bytes the decoder is waiting for, sent from the thread to the async side.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceRequest {
    /// Fetch these ranges. A zero-length range means "the next chunk from
    /// this offset" (sequential decoders); the fetcher chooses its size.
    Ranges(Vec<ByteRange>),
    /// The total source length.
    Len,
}

/// One fetched chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Absolute offset of `bytes[0]`.
    pub offset: u64,
    /// The bytes.
    pub bytes: Vec<u8>,
    /// Whether `offset + bytes.len()` is the end of the source.
    pub eof: bool,
}

/// The async side's answer to a [`SourceRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceReply {
    /// Chunks covering the requested ranges (any order, any split).
    Chunks(Vec<Chunk>),
    /// The total length.
    Len(u64),
}

/// Failure of a threaded decode.
#[derive(Debug)]
#[non_exhaustive]
pub enum ThreadError {
    /// The decoder failed.
    Codec(BoxedError),
    /// The fetcher failed.
    Fetch(BoxedError),
    /// The other side went away (the future was dropped, or the thread
    /// panicked or exited without a result).
    Disconnected,
    /// The decoder refused bytes it keeps asking for.
    Stalled,
    /// The fetcher answered a request with the wrong kind of reply.
    ProtocolMismatch,
    /// The pulse tripped while the thread was waiting on a channel.
    Cancelled(StopReason),
}

impl fmt::Display for ThreadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "codec error on decode thread: {e}"),
            Self::Fetch(e) => write!(f, "fetch error while serving decode thread: {e}"),
            Self::Disconnected => write!(f, "decode thread and driver disconnected"),
            Self::Stalled => write!(f, "decoder refused bytes it keeps requesting"),
            Self::ProtocolMismatch => write!(f, "fetcher replied with the wrong kind of data"),
            Self::Cancelled(r) => write!(f, "decode thread cancelled while waiting: {r:?}"),
        }
    }
}

impl core::error::Error for ThreadError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(e) | Self::Fetch(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

/// Work handed to a [`ThreadOptions::with_spawner`] spawner.
pub type ThreadWork = Box<dyn FnOnce() + Send + 'static>;

type Spawner = Box<dyn FnOnce(ThreadWork) + Send>;

/// How the decode thread is started.
pub struct ThreadOptions {
    /// Thread name (ignored when a spawner is supplied).
    pub name: String,
    spawner: Option<Spawner>,
}

impl Default for ThreadOptions {
    fn default() -> Self {
        Self {
            name: String::from("zencodec-decode"),
            spawner: None,
        }
    }
}

impl fmt::Debug for ThreadOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThreadOptions")
            .field("name", &self.name)
            .field("spawner", &self.spawner.is_some())
            .finish()
    }
}

impl ThreadOptions {
    /// Run the decode on a thread the caller starts (a pool sized to the
    /// machine's cores, for instance) instead of a fresh OS thread.
    pub fn with_spawner(mut self, spawn: impl FnOnce(ThreadWork) + Send + 'static) -> Self {
        self.spawner = Some(Box::new(spawn));
        self
    }

    fn spawn(self, work: ThreadWork) -> Result<(), ThreadError> {
        match self.spawner {
            Some(spawn) => {
                spawn(work);
                Ok(())
            }
            None => std::thread::Builder::new()
                .name(self.name)
                .spawn(work)
                .map(|_| ())
                .map_err(|e| ThreadError::Fetch(Box::new(e))),
        }
    }
}

/// Outcome of one non-blocking attempt in [`wait_with_stop`].
enum Attempt<T> {
    Ready(T),
    Pending,
    Closed,
}

/// Block until `attempt` is ready, re-checking `stop` between attempts.
///
/// Without a stop the caller uses the channel's own blocking call; with
/// one, a short spin then a sleep that doubles from 50 µs to 2 ms bounds
/// both the cancellation latency and the idle wakeups.
fn wait_with_stop<T>(
    stop: Option<&dyn Stop>,
    mut attempt: impl FnMut() -> Attempt<T>,
) -> Result<T, ThreadError> {
    let mut sleep = core::time::Duration::from_micros(50);
    let mut spins = 0u32;
    loop {
        match attempt() {
            Attempt::Ready(v) => return Ok(v),
            Attempt::Closed => return Err(ThreadError::Disconnected),
            Attempt::Pending => {}
        }
        if let Some(s) = stop
            && let Err(reason) = s.check()
        {
            return Err(ThreadError::Cancelled(reason));
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

struct Links {
    requests: mpsc::Sender<SourceRequest>,
    replies: mpsc::Receiver<SourceReply>,
    pulse: Option<SharedPulse>,
}

impl Links {
    fn stop(&self) -> Option<&dyn Stop> {
        self.pulse.as_ref().map(|p| p as &dyn Stop)
    }

    fn ask(&mut self, req: SourceRequest) -> Result<SourceReply, ThreadError> {
        if let Some(s) = self.stop()
            && let Err(reason) = s.check()
        {
            return Err(ThreadError::Cancelled(reason));
        }
        match &self.pulse {
            None => {
                self.requests
                    .blocking_send(req)
                    .map_err(|_| ThreadError::Disconnected)?;
                self.replies
                    .blocking_recv()
                    .ok_or(ThreadError::Disconnected)
            }
            Some(pulse) => {
                let stop: &dyn Stop = pulse;
                let mut slot = Some(req);
                let tx = &self.requests;
                wait_with_stop(Some(stop), || {
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
                wait_with_stop(Some(stop), || match rx.try_recv() {
                    Ok(reply) => Attempt::Ready(reply),
                    Err(mpsc::error::TryRecvError::Empty) => Attempt::Pending,
                    Err(mpsc::error::TryRecvError::Disconnected) => Attempt::Closed,
                })
            }
        }
    }
}

/// Serve the thread's requests from `fetch` until it finishes.
async fn serve<F, Fut, T>(
    mut requests: mpsc::Receiver<SourceRequest>,
    replies: mpsc::Sender<SourceReply>,
    done: oneshot::Receiver<Result<T, ThreadError>>,
    mut fetch: F,
) -> Result<T, ThreadError>
where
    F: FnMut(SourceRequest) -> Fut,
    Fut: Future<Output = Result<SourceReply, BoxedError>>,
{
    let mut fetch_error = None;
    while let Some(req) = requests.recv().await {
        // Race the fetch against the thread going away (cancelled by its
        // pulse, failed, or finished): a fetch that never completes must
        // not keep the driver alive.
        let fetched = tokio::select! {
            biased;
            () = replies.closed() => break,
            r = fetch(req) => r,
        };
        match fetched {
            Ok(reply) => {
                if replies.send(reply).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                fetch_error = Some(e);
                break;
            }
        }
    }
    // Closing the reply channel makes the thread exit at its next request.
    drop(replies);
    match done.await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(ThreadError::Disconnected)) | Err(_) => match fetch_error {
            Some(e) => Err(ThreadError::Fetch(e)),
            None => Err(ThreadError::Disconnected),
        },
        Ok(Err(e)) => Err(e),
    }
}

/// Run a resumable decoder on its own thread, serving its byte requests
/// with `fetch` on the async side.
///
/// `fetch` receives each [`SourceRequest`] and returns the bytes; it is
/// the only async code involved. The sink is moved onto the thread and
/// returned with the result. `pulse` cancels the thread at its next step
/// or blocking wait, is told the row total once the header is known, and
/// is advanced by every strip.
pub async fn decode_on_thread<D, S, F, Fut>(
    mut decoder: D,
    mut sink: S,
    pulse: Option<SharedPulse>,
    opts: ThreadOptions,
    fetch: F,
) -> Result<(OutputInfo, S), ThreadError>
where
    D: Drivable + Send + 'static,
    S: DecodeRowSink + Send + 'static,
    F: FnMut(SourceRequest) -> Fut,
    Fut: Future<Output = Result<SourceReply, BoxedError>>,
{
    let (req_tx, req_rx) = mpsc::channel(1);
    let (rep_tx, rep_rx) = mpsc::channel(1);
    let (done_tx, done_rx) = oneshot::channel();
    let mut links = Links {
        requests: req_tx,
        replies: rep_rx,
        pulse,
    };
    opts.spawn(Box::new(move || {
        let result = run_resumable(&mut decoder, &mut sink, &mut links);
        let _ = done_tx.send(result.map(|info| (info, sink)));
    }))?;
    serve(req_rx, rep_tx, done_rx, fetch).await
}

fn run_resumable<D: Drivable + ?Sized>(
    decoder: &mut D,
    sink: &mut dyn DecodeRowSink,
    links: &mut Links,
) -> Result<OutputInfo, ThreadError> {
    let pulse = links.pulse.clone();
    let pulse: Option<&dyn Pulse> = pulse.as_ref().map(|p| p as &dyn Pulse);
    let stop = pulse.map(|p| p as &dyn Stop);
    let mut feeder = Feeder::new();
    let mut need: Vec<ByteRange> = Vec::new();
    let mut told_total = false;
    let mut last_refused: Option<Vec<ByteRange>> = None;
    let codec = |e: D::Error| ThreadError::Codec(Box::new(e));
    loop {
        match decoder.step(sink, stop).map_err(codec)? {
            Step::Need(ranges) => {
                need.clear();
                need.extend_from_slice(ranges);
            }
            Step::NeedLen => match links.ask(SourceRequest::Len)? {
                SourceReply::Len(len) => {
                    decoder.feed(len, &[], true).map_err(codec)?;
                    continue;
                }
                _ => return Err(ThreadError::ProtocolMismatch),
            },
            Step::Rows { rows, .. } => {
                if let Some(p) = pulse {
                    p.advance(rows as u64);
                }
                continue;
            }
            Step::Done(info) => return Ok(info),
            _ => continue,
        }
        if !told_total && let (Some(p), Some(info)) = (pulse, decoder.info()) {
            let _ = p.set_total(Total::Exact(info.height as u64));
            told_total = true;
        }
        if feeder.has_carry() {
            if !feeder.flush(decoder).map_err(codec)? {
                if last_refused.as_deref() == Some(&need[..]) {
                    return Err(ThreadError::Stalled);
                }
                last_refused = Some(need.clone());
                continue;
            }
            last_refused = None;
            continue;
        }
        match links.ask(SourceRequest::Ranges(need.clone()))? {
            SourceReply::Chunks(chunks) => {
                for chunk in chunks {
                    feeder
                        .offer(decoder, chunk.offset, &chunk.bytes, chunk.eof)
                        .map_err(codec)?;
                }
            }
            _ => return Err(ThreadError::ProtocolMismatch),
        }
    }
}

/// Run a pull-native decode on its own thread. `run` receives a
/// [`DynReadAt`] whose reads block on the async fetcher, so a codec written
/// against [`push_decoder_from_source`](zencodec::decode::DecodeJob::push_decoder_from_source)
/// streams from an async transport with no buffering beyond each read.
///
/// `pulse` cancels at the next read (every read checks it, and the waits
/// for a fetch reply re-check it); the codec's own job-level stop still
/// applies inside `run`. Progress is whatever the sink observes: wrap it
/// in [`ChannelRowSink`] or a counting sink; the bridge cannot see rows
/// written by a pull-native core.
pub async fn pull_decode_on_thread<R, S, F, Fut>(
    run: R,
    mut sink: S,
    pulse: Option<SharedPulse>,
    opts: ThreadOptions,
    fetch: F,
) -> Result<(OutputInfo, S), ThreadError>
where
    R: FnOnce(&mut dyn DynReadAt, &mut dyn DecodeRowSink) -> Result<OutputInfo, BoxedError>
        + Send
        + 'static,
    S: DecodeRowSink + Send + 'static,
    F: FnMut(SourceRequest) -> Fut,
    Fut: Future<Output = Result<SourceReply, BoxedError>>,
{
    let (req_tx, req_rx) = mpsc::channel(1);
    let (rep_tx, rep_rx) = mpsc::channel(1);
    let (done_tx, done_rx) = oneshot::channel();
    let mut source = ChannelReadAt {
        links: Links {
            requests: req_tx,
            replies: rep_rx,
            pulse,
        },
        len: None,
    };
    opts.spawn(Box::new(move || {
        let result = run(&mut source, &mut sink).map_err(|e| match e.downcast::<ThreadError>() {
            Ok(t) => *t,
            Err(e) => ThreadError::Codec(e),
        });
        let _ = done_tx.send(result.map(|info| (info, sink)));
    }))?;
    serve(req_rx, rep_tx, done_rx, fetch).await
}

/// [`DynReadAt`] whose reads block on the async fetcher.
struct ChannelReadAt {
    links: Links,
    len: Option<u64>,
}

impl ChannelReadAt {
    fn assemble(
        chunks: &[Chunk],
        offset: u64,
        out: &mut [u8],
        total: Option<u64>,
    ) -> Result<(), BoxedError> {
        let mut filled = 0usize;
        while filled < out.len() {
            let pos = offset + filled as u64;
            let Some(chunk) = chunks
                .iter()
                .find(|c| pos >= c.offset && pos < c.offset + c.bytes.len() as u64)
            else {
                return Err(Box::new(SourceError::UnexpectedEof {
                    offset,
                    len: out.len() as u64,
                    total,
                }));
            };
            let start = (pos - chunk.offset) as usize;
            let n = (chunk.bytes.len() - start).min(out.len() - filled);
            out[filled..filled + n].copy_from_slice(&chunk.bytes[start..start + n]);
            filled += n;
        }
        Ok(())
    }
}

impl DynReadAt for ChannelReadAt {
    fn len(&mut self) -> Result<u64, BoxedError> {
        if let Some(len) = self.len {
            return Ok(len);
        }
        match self
            .links
            .ask(SourceRequest::Len)
            .map_err(|e| Box::new(e) as BoxedError)?
        {
            SourceReply::Len(len) => {
                self.len = Some(len);
                Ok(len)
            }
            _ => Err(Box::new(ThreadError::ProtocolMismatch)),
        }
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BoxedError> {
        self.read_many_at(&mut [(offset, out)])
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
        let ranges: Vec<ByteRange> = reqs
            .iter()
            .map(|(off, buf)| ByteRange::new(*off, buf.len() as u64))
            .collect();
        match self
            .links
            .ask(SourceRequest::Ranges(ranges))
            .map_err(|e| Box::new(e) as BoxedError)?
        {
            SourceReply::Chunks(chunks) => {
                for c in &chunks {
                    if c.eof {
                        self.len = Some(c.offset + c.bytes.len() as u64);
                    }
                }
                for (offset, out) in reqs.iter_mut() {
                    Self::assemble(&chunks, *offset, out, self.len)?;
                }
                Ok(())
            }
            _ => Err(Box::new(ThreadError::ProtocolMismatch)),
        }
    }
}

/// A fetcher over an in-memory image, for tests and already-buffered
/// input: answers every request from the slice.
pub fn fetch_from_bytes(
    bytes: alloc::sync::Arc<[u8]>,
    sequential_chunk: usize,
) -> impl FnMut(SourceRequest) -> core::future::Ready<Result<SourceReply, BoxedError>> {
    move |req| {
        let total = bytes.len() as u64;
        let reply = match req {
            SourceRequest::Len => Ok(SourceReply::Len(total)),
            SourceRequest::Ranges(ranges) => {
                let mut chunks = Vec::with_capacity(ranges.len());
                for r in ranges {
                    let start = r.offset.min(total);
                    let len = if r.is_empty() {
                        (total - start).min(sequential_chunk as u64)
                    } else {
                        r.length
                    };
                    let end = start + len;
                    if end > total {
                        return core::future::ready(Err(Box::new(SourceError::UnexpectedEof {
                            offset: r.offset,
                            len: r.length,
                            total: Some(total),
                        }) as BoxedError));
                    }
                    chunks.push(Chunk {
                        offset: start,
                        bytes: bytes[start as usize..end as usize].to_vec(),
                        eof: end == total,
                    });
                }
                Ok(SourceReply::Chunks(chunks))
            }
        };
        core::future::ready(reply)
    }
}

/// A fetcher over a sequential async reader (an HTTP body, a socket).
///
/// Serves only sequential requests at the stream cursor; a sized range
/// behind the cursor, or a length request, is an error, because a stream
/// cannot answer them. Each request reads one chunk of up to `chunk` bytes.
pub fn fetch_from_async_read<R>(
    reader: R,
    chunk: usize,
) -> impl FnMut(
    SourceRequest,
) -> core::pin::Pin<Box<dyn Future<Output = Result<SourceReply, BoxedError>> + Send>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::AsyncReadExt;
    let state = alloc::sync::Arc::new(tokio::sync::Mutex::new((reader, 0u64, false)));
    move |req| {
        let state = state.clone();
        Box::pin(async move {
            let mut guard = state.lock().await;
            let (reader, cursor, eof) = &mut *guard;
            let ranges = match req {
                SourceRequest::Ranges(r) => r,
                SourceRequest::Len => {
                    return Err(Box::new(SourceError::Io {
                        message: String::from("a sequential stream cannot report its length"),
                    }) as BoxedError);
                }
            };
            for r in &ranges {
                if !r.is_empty() && r.offset < *cursor {
                    return Err(Box::new(SourceError::BeyondWindow {
                        offset: r.offset,
                        oldest_retained: *cursor,
                    }) as BoxedError);
                }
            }
            if *eof {
                return Ok(SourceReply::Chunks(alloc::vec![Chunk {
                    offset: *cursor,
                    bytes: Vec::new(),
                    eof: true,
                }]));
            }
            let mut buf = alloc::vec![0u8; chunk.max(1)];
            let n = reader
                .read(&mut buf)
                .await
                .map_err(|e| Box::new(e) as BoxedError)?;
            buf.truncate(n);
            let offset = *cursor;
            *cursor += n as u64;
            if n == 0 {
                *eof = true;
            }
            Ok(SourceReply::Chunks(alloc::vec![Chunk {
                offset,
                bytes: buf,
                eof: n == 0,
            }]))
        })
    }
}

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
                let stop: &dyn Stop = pulse;
                let mut slot = Some(event);
                wait_with_stop(Some(stop), || {
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
