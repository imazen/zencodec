//! Seekable byte sources for pull-based (random-access) decoding.
//!
//! [`ReadAt`] is the pull counterpart to the push-based
//! [`IncrementalDecode`](crate::decode::IncrementalDecode): the codec reads
//! exactly the byte ranges it needs, at offsets it chooses, in straight-line
//! code — no resumable state machine. Drivers provide sources over files
//! (`pread`), HTTP Range / object stores, or host callbacks across FFI; the
//! codec never knows which.
//!
//! A sequential stream is adapted with [`BufferedReadAt`], which serves
//! random-access reads from a windowed cache, a retained prefix cache, or a
//! full in-memory spool, according to its [`BufferPolicy`]. A codec declares
//! its [`AccessPattern`] so the driver can pick the cheapest policy that
//! cannot fail: forward-only codecs run on a small window, arbitrary-seek
//! codecs get a spool.
//!
//! Cancellation: a source only reads bytes. Cooperative cancellation is the
//! caller's `enough::Stop` — which any rich progress type reaches by
//! downgrade, since `how_far::Pulse: Stop`, and any plain flag reaches via
//! `almost_enough::FnStop` over an `AtomicBool` wrapped in a
//! [`StopToken`](crate::StopToken) at job construction.

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::fmt;

use crate::traits::BoxedError;

/// How a decoder reads its input, declared so drivers can choose a
/// buffering policy for unseekable streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum AccessPattern {
    /// Reads move forward; any backtrack is incidental and tiny.
    Sequential,
    /// Reads may revisit at most `window_bytes` behind the furthest read.
    BoundedBacktrack {
        /// Maximum bytes behind the read watermark a read may target.
        window_bytes: u64,
    },
    /// Reads may target any offset at any time (directories, tile tables).
    #[default]
    RandomAccess,
}

/// Failure modes of a byte source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceError {
    /// A read targeted bytes already evicted from a windowed buffer.
    BeyondWindow {
        /// Offset the codec asked for.
        offset: u64,
        /// Oldest offset still retained.
        oldest_retained: u64,
    },
    /// A buffering policy's byte cap would be exceeded.
    BeyondLimit {
        /// Bytes the operation would have retained.
        needed: u64,
        /// Configured cap.
        limit: u64,
    },
    /// A read ran past the end of the source.
    UnexpectedEof {
        /// Offset the codec asked for.
        offset: u64,
        /// Bytes requested.
        len: u64,
        /// Total source length, if known.
        total: Option<u64>,
    },
    /// The underlying transport or filesystem failed.
    Io {
        /// Driver-supplied description of the failure.
        message: alloc::string::String,
    },
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeyondWindow {
                offset,
                oldest_retained,
            } => write!(
                f,
                "source read at {offset} is beyond the buffered window (oldest retained: {oldest_retained})"
            ),
            Self::BeyondLimit { needed, limit } => write!(
                f,
                "source buffering would retain {needed} bytes, over the {limit}-byte limit"
            ),
            Self::UnexpectedEof { offset, len, total } => write!(
                f,
                "source read of {len} bytes at {offset} ran past end of source (total: {total:?})"
            ),
            Self::Io { message } => write!(f, "source I/O error: {message}"),
        }
    }
}

impl core::error::Error for SourceError {}

/// Random-access byte source. Modelled on imij's `ReadAt`, so an imij-style
/// codec bridges to this trait by renaming.
///
/// `read_many_at` exists so a source that can serve ranges concurrently
/// (HTTP Range, object stores, parallel `pread`) overlaps their round
/// trips. Callers pass every range they can ask for at once.
pub trait ReadAt {
    /// The source-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Total length in bytes. A buffered stream source may consume to the
    /// end of the stream to answer this.
    fn len(&mut self) -> Result<u64, Self::Error>;

    /// Whether the source is empty.
    fn is_empty(&mut self) -> Result<bool, Self::Error> {
        Ok(self.len()? == 0)
    }

    /// Fill `out` with the bytes at `offset`. Fails rather than returning
    /// a short read.
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error>;

    /// Fill each buffer from its offset. The default serves requests in
    /// the order given; concurrent sources override to overlap them.
    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error> {
        for (offset, out) in reqs.iter_mut() {
            self.read_exact_at(*offset, out)?;
        }
        Ok(())
    }
}

/// [`ReadAt`] over an in-memory slice. The zero-copy source for tests,
/// embedded data, and already-buffered input.
#[derive(Debug, Clone)]
pub struct SliceReadAt<'a> {
    bytes: &'a [u8],
}

impl<'a> SliceReadAt<'a> {
    /// Create a source over `bytes`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl ReadAt for SliceReadAt<'_> {
    type Error = SourceError;

    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(self.bytes.len() as u64)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        let start = usize::try_from(offset).map_err(|_| SourceError::UnexpectedEof {
            offset,
            len: out.len() as u64,
            total: Some(self.bytes.len() as u64),
        })?;
        let end = start
            .checked_add(out.len())
            .ok_or(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(self.bytes.len() as u64),
            })?;
        let data = self
            .bytes
            .get(start..end)
            .ok_or(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(self.bytes.len() as u64),
            })?;
        out.copy_from_slice(data);
        Ok(())
    }
}

/// Object-safe [`ReadAt`]. Drivers hand this to seekable decoders; codecs
/// wrap its boxed errors with their own source-error constructor.
pub trait DynReadAt: Send {
    /// Total length in bytes.
    fn len(&mut self) -> Result<u64, BoxedError>;
    /// Whether the source is empty.
    fn is_empty(&mut self) -> Result<bool, BoxedError> {
        Ok(self.len()? == 0)
    }
    /// Fill `out` with the bytes at `offset`.
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BoxedError>;
    /// Fill each buffer from its offset, concurrently where possible.
    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
        for (offset, out) in reqs.iter_mut() {
            self.read_exact_at(*offset, out)?;
        }
        Ok(())
    }
}

/// Shim wrapping a concrete [`ReadAt`] as a [`DynReadAt`].
pub struct ReadAtShim<S>(pub S);

impl<S: ReadAt + Send> DynReadAt for ReadAtShim<S> {
    fn len(&mut self) -> Result<u64, BoxedError> {
        self.0.len().map_err(|e| Box::new(e) as BoxedError)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), BoxedError> {
        self.0
            .read_exact_at(offset, out)
            .map_err(|e| Box::new(e) as BoxedError)
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), BoxedError> {
        self.0
            .read_many_at(reqs)
            .map_err(|e| Box::new(e) as BoxedError)
    }
}

impl<S: ReadAt + Send + 'static> From<S> for Box<dyn DynReadAt> {
    fn from(source: S) -> Self {
        Box::new(ReadAtShim(source))
    }
}

/// How a [`BufferedReadAt`] retains bytes from a sequential stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BufferPolicy {
    /// Retain only the most recent bytes. A read older than the window
    /// fails with [`SourceError::BeyondWindow`]. Cheapest policy; correct
    /// for [`AccessPattern::Sequential`] and small
    /// [`AccessPattern::BoundedBacktrack`] codecs.
    Window {
        /// Bytes retained behind the read watermark.
        window_bytes: usize,
    },
    /// Retain every byte consumed, up to a cap. A read that would retain
    /// more fails with [`SourceError::BeyondLimit`]. Suits codecs whose
    /// seeks stay within an already-read prefix.
    Cache {
        /// Maximum retained bytes.
        max_bytes: usize,
    },
    /// Retain every byte consumed, up to a cap, treating the source as a
    /// spool: any read can be served once its bytes have arrived. Same
    /// storage as [`BufferPolicy::Cache`]; the name records the driver's
    /// intent that arbitrary [`AccessPattern::RandomAccess`] reads are
    /// expected, so the cap should be sized to the file, not a prefix.
    Spool {
        /// Maximum retained bytes.
        max_bytes: usize,
    },
}

#[cfg(feature = "std")]
impl BufferPolicy {
    fn max_retained(&self) -> Option<usize> {
        match self {
            Self::Window { window_bytes } => Some(*window_bytes),
            Self::Cache { max_bytes } | Self::Spool { max_bytes } => Some(*max_bytes),
        }
    }

    fn evicts(&self) -> bool {
        matches!(self, Self::Window { .. })
    }
}

/// [`ReadAt`] over a sequential [`std::io::Read`] stream, buffered per a
/// [`BufferPolicy`].
///
/// This is the adapter that lets a pull-based codec run on an unseekable
/// stream: buffering manufactures the seeks the stream cannot provide.
/// The price is explicit — window eviction can fail a far-back read, and
/// cache/spool retention is capped — never silent.
#[cfg(feature = "std")]
pub struct BufferedReadAt<R: std::io::Read> {
    reader: R,
    policy: BufferPolicy,
    /// Retained bytes; `buf[0]` is at `buf_start`.
    buf: Vec<u8>,
    buf_start: u64,
    /// Bytes consumed from the reader so far (end of `buf` when retained).
    watermark: u64,
    eof: bool,
    total: Option<u64>,
}

#[cfg(feature = "std")]
impl<R: std::io::Read> BufferedReadAt<R> {
    /// Create a buffered source over `reader` with `policy`.
    pub fn new(reader: R, policy: BufferPolicy) -> Self {
        Self {
            reader,
            policy,
            buf: Vec::new(),
            buf_start: 0,
            watermark: 0,
            eof: false,
            total: None,
        }
    }

    /// Bytes consumed from the underlying reader so far.
    pub fn watermark(&self) -> u64 {
        self.watermark
    }

    /// Consume the stream until `watermark >= end` or EOF. Bytes at or
    /// after `protect_from` survive window eviction during the fill.
    fn fill_to(&mut self, end: u64, protect_from: u64) -> Result<(), SourceError> {
        let mut chunk = [0u8; 16 * 1024];
        while self.watermark < end && !self.eof {
            let want = ((end - self.watermark).min(chunk.len() as u64)) as usize;
            let n = self
                .reader
                .read(&mut chunk[..want.max(1)])
                .map_err(|e| SourceError::Io {
                    message: e.to_string(),
                })?;
            if n == 0 {
                self.eof = true;
                self.total = Some(self.watermark);
                break;
            }
            self.buf.extend_from_slice(&chunk[..n]);
            self.watermark += n as u64;
            self.enforce_policy(protect_from)?;
        }
        Ok(())
    }

    /// Evict or reject retained bytes per policy. `protect_from` is an
    /// offset whose bytes must survive eviction (the in-flight request).
    fn enforce_policy(&mut self, protect_from: u64) -> Result<(), SourceError> {
        if self.policy.evicts() {
            let window = self.policy.max_retained().unwrap_or(usize::MAX) as u64;
            let oldest_allowed = self.watermark.saturating_sub(window).max(protect_from);
            if oldest_allowed > self.buf_start {
                let drop = ((oldest_allowed - self.buf_start) as usize).min(self.buf.len());
                self.buf.drain(..drop);
                self.buf_start += drop as u64;
            }
        } else if let Some(cap) = self.policy.max_retained()
            && (self.buf.len() as u64) > cap as u64
        {
            return Err(SourceError::BeyondLimit {
                needed: self.buf.len() as u64,
                limit: cap as u64,
            });
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
impl<R: std::io::Read> ReadAt for BufferedReadAt<R> {
    type Error = SourceError;

    fn len(&mut self) -> Result<u64, Self::Error> {
        if let Some(total) = self.total {
            return Ok(total);
        }
        // Drain to EOF; retention policy still applies, so a windowed
        // source reports its length without keeping the bytes.
        let mut chunk = [0u8; 16 * 1024];
        while !self.eof {
            let n = self.reader.read(&mut chunk).map_err(|e| SourceError::Io {
                message: e.to_string(),
            })?;
            if n == 0 {
                self.eof = true;
                self.total = Some(self.watermark);
                break;
            }
            self.buf.extend_from_slice(&chunk[..n]);
            self.watermark += n as u64;
            self.enforce_policy(0)?;
        }
        Ok(self.watermark)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        if out.is_empty() {
            return Ok(());
        }
        if offset < self.buf_start {
            return Err(SourceError::BeyondWindow {
                offset,
                oldest_retained: self.buf_start,
            });
        }
        let end = offset
            .checked_add(out.len() as u64)
            .ok_or(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: self.total,
            })?;
        self.fill_to(end, offset)?;
        if end > self.watermark {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: self.total,
            });
        }
        let start = (offset - self.buf_start) as usize;
        out.copy_from_slice(&self.buf[start..start + out.len()]);
        // The served bytes were protected during fill; eviction for
        // window policies happens on the next fill.
        Ok(())
    }

    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error> {
        // Serve in stream order so a sequential reader is read at most
        // once per batch and the window only moves forward.
        reqs.sort_by_key(|(offset, _)| *offset);
        for (offset, out) in reqs.iter_mut() {
            self.read_exact_at(*offset, out)?;
        }
        Ok(())
    }
}

/// Merge overlapping or near-adjacent ranges (local copy of the public
/// `coalesce_byte_ranges` algorithm, kept here so range driving stays
/// available without the `std` helper module).
fn coalesce_ranges(
    ranges: &[crate::decode::ByteRange],
    max_gap: u64,
) -> Vec<crate::decode::ByteRange> {
    let mut valid: Vec<crate::decode::ByteRange> =
        ranges.iter().copied().filter(|r| !r.is_empty()).collect();
    if valid.is_empty() {
        return Vec::new();
    }
    valid.sort_by_key(|r| (r.offset, r.length));
    let mut coalesced = Vec::with_capacity(valid.len());
    let mut cur = valid[0];
    for r in valid.into_iter().skip(1) {
        let cur_end = cur.end();
        if r.offset <= cur_end.saturating_add(max_gap) {
            let new_end = cur_end.max(r.end());
            cur.length = new_end.saturating_sub(cur.offset);
        } else {
            coalesced.push(cur);
            cur = r;
        }
    }
    coalesced.push(cur);
    coalesced
}

/// Outcome counters from [`drive_ranges`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DriveStats {
    /// Fetch rounds (one batched source read per round).
    pub rounds: u64,
    /// Source read calls issued.
    pub requests: u64,
    /// Bytes fetched from the source.
    pub bytes_fetched: u64,
}

/// Failure of [`drive_ranges`].
#[derive(Debug)]
pub enum DriveError<CE, SE> {
    /// The codec returned an error.
    Codec(CE),
    /// The source returned an error.
    Source(SE),
    /// The decoder asked for sequential input, which a range driver
    /// cannot manufacture; feed it with `push_chunk` instead.
    NeedsSequentialInput,
    /// The decoder reported `NeedRanges` with an empty request list.
    EmptyRangeRequest,
}

impl<CE: fmt::Display, SE: fmt::Display> fmt::Display for DriveError<CE, SE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "codec error during range drive: {e}"),
            Self::Source(e) => write!(f, "source error during range drive: {e}"),
            Self::NeedsSequentialInput => {
                write!(f, "decoder needs sequential input, not ranges")
            }
            Self::EmptyRangeRequest => {
                write!(f, "decoder requested ranges but named none")
            }
        }
    }
}

impl<CE: core::error::Error + 'static, SE: core::error::Error + 'static> core::error::Error
    for DriveError<CE, SE>
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            Self::Source(e) => Some(e),
            _ => None,
        }
    }
}

/// Drive an incremental decoder that is waiting for byte ranges, fetching
/// each round from `source` in one batched read.
///
/// Call after a `push_*` returns
/// [`PushStatus::NeedRanges`](crate::decode::PushStatus::NeedRanges).
/// Ranges are coalesced with `max_gap`, fetched via one
/// [`read_many_at`](ReadAt::read_many_at) per round (so concurrent sources
/// overlap round trips), and delivered with `push_range`. Returns when the
/// decoder leaves `NeedRanges`; the caller then drains output normally.
pub fn drive_ranges<D, S>(
    decoder: &mut D,
    source: &mut S,
    max_gap: u64,
    stop: Option<&dyn enough::Stop>,
) -> Result<DriveStats, DriveError<D::Error, S::Error>>
where
    D: crate::decode::IncrementalDecode,
    S: ReadAt,
{
    use crate::decode::PushStatus;

    let mut stats = DriveStats::default();
    loop {
        let requested: Vec<crate::decode::ByteRange> = decoder.requested_ranges().to_vec();
        if requested.is_empty() {
            return Err(DriveError::EmptyRangeRequest);
        }
        let coalesced = coalesce_ranges(&requested, max_gap);
        stats.rounds += 1;

        // One batched read for the whole round.
        let mut buffers: Vec<(u64, Vec<u8>)> = coalesced
            .iter()
            .map(|r| (r.offset, alloc::vec![0u8; r.length as usize]))
            .collect();
        {
            let mut reqs: Vec<(u64, &mut [u8])> = buffers
                .iter_mut()
                .map(|(off, buf)| (*off, &mut buf[..]))
                .collect();
            source.read_many_at(&mut reqs).map_err(DriveError::Source)?;
        }

        let mut status = None;
        for (offset, buf) in &buffers {
            stats.requests += 1;
            stats.bytes_fetched += buf.len() as u64;
            let outcome = decoder
                .push_range(*offset, buf, false, stop)
                .map_err(DriveError::Codec)?;
            status = Some(outcome.status);
            if outcome.status != PushStatus::NeedRanges {
                break;
            }
        }
        match status {
            Some(PushStatus::NeedRanges) => continue,
            Some(PushStatus::NeedMoreInput) => return Err(DriveError::NeedsSequentialInput),
            _ => return Ok(stats),
        }
    }
}

/// Adapter presenting a [`SeekableDecode`](crate::decode::SeekableDecode)
/// as an [`IncrementalDecode`](crate::decode::IncrementalDecode): pushed
/// bytes accumulate, the seekable decode runs once at EOF over a slice
/// source, and batches are then served from the decoded rows.
///
/// This is the buffering pattern a codec would otherwise hand-roll to
/// offer push-based decoding from a pull-based core. Memory is the full
/// input plus the decoded rows; drivers with a real seekable source
/// should prefer running the seekable decoder directly.
pub struct SeekableIncrementalAdapter<S: crate::decode::SeekableDecode> {
    inner: S,
    pushed: Vec<u8>,
    eof: bool,
    ran: bool,
    complete: bool,
    rows: Vec<u8>,
    width: u32,
    height: u32,
    descriptor: Option<zenpixels::PixelDescriptor>,
    next_y: u32,
    info_cache: Option<crate::ImageInfo>,
}

impl<S: crate::decode::SeekableDecode> SeekableIncrementalAdapter<S> {
    /// Create an adapter wrapping `inner`.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            pushed: Vec::new(),
            eof: false,
            ran: false,
            complete: false,
            rows: Vec::new(),
            width: 0,
            height: 0,
            descriptor: None,
            next_y: 0,
            info_cache: None,
        }
    }

    fn run_once(&mut self) -> Result<(), S::Error> {
        if self.ran {
            return Ok(());
        }
        self.ran = true;
        let data = core::mem::take(&mut self.pushed);
        let mut source: Box<dyn DynReadAt> = Box::new(ReadAtShim(SliceReadAt::new(&data)));
        let mut sink = CollectSink::default();
        let output = self.inner.run(&mut *source, &mut sink, None)?;
        self.width = sink.width;
        self.height = sink.height;
        self.descriptor = sink.descriptor;
        self.rows = sink.rows;
        self.info_cache = self.inner.info().cloned();
        let _ = output;
        Ok(())
    }
}

#[derive(Default)]
struct CollectSink {
    rows: Vec<u8>,
    width: u32,
    height: u32,
    descriptor: Option<zenpixels::PixelDescriptor>,
    stride: usize,
}

impl crate::DecodeRowSink for CollectSink {
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: zenpixels::PixelDescriptor,
    ) -> Result<(), crate::decode::SinkError> {
        self.width = width;
        self.height = height;
        self.descriptor = Some(descriptor);
        self.stride = width as usize * descriptor.bytes_per_pixel();
        self.rows = alloc::vec![0u8; self.stride * height as usize];
        Ok(())
    }

    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: zenpixels::PixelDescriptor,
    ) -> Result<zenpixels::PixelSliceMut<'_>, crate::decode::SinkError> {
        if self.descriptor.is_none() {
            self.begin(width, height, descriptor)?;
        }
        let start = y as usize * self.stride;
        let len = height as usize * self.stride;
        Ok(zenpixels::PixelSliceMut::new(
            &mut self.rows[start..start + len],
            width,
            height,
            self.stride,
            descriptor,
        )
        .map_err(|e| alloc::format!("collect sink: {e}"))?)
    }

    fn finish(&mut self) -> Result<(), crate::decode::SinkError> {
        Ok(())
    }
}

impl<S: crate::decode::SeekableDecode + Send> crate::decode::IncrementalDecode
    for SeekableIncrementalAdapter<S>
{
    type Error = S::Error;

    fn push_chunk(
        &mut self,
        chunk: &[u8],
        is_eof: bool,
        _stop: Option<&dyn enough::Stop>,
    ) -> Result<crate::decode::PushOutcome, Self::Error> {
        use crate::decode::PushStatus;
        if self.complete {
            return Ok(crate::decode::PushOutcome::new(0, PushStatus::Complete));
        }
        self.pushed.extend_from_slice(chunk);
        if is_eof {
            self.eof = true;
            Ok(crate::decode::PushOutcome::new(
                chunk.len(),
                PushStatus::OutputReady,
            ))
        } else {
            Ok(crate::decode::PushOutcome::new(
                chunk.len(),
                PushStatus::NeedMoreInput,
            ))
        }
    }

    fn pull_batch(&mut self) -> Result<Option<(u32, zenpixels::PixelSlice<'_>)>, Self::Error> {
        if !self.eof {
            return Ok(None);
        }
        self.run_once()?;
        let Some(desc) = self.descriptor else {
            self.complete = true;
            return Ok(None);
        };
        if self.next_y >= self.height {
            self.complete = true;
            return Ok(None);
        }
        let y = self.next_y;
        let rows = self.height - y;
        self.next_y = self.height;
        let stride = self.width as usize * desc.bytes_per_pixel();
        let start = y as usize * stride;
        let slice = zenpixels::PixelSlice::new(
            &self.rows[start..start + rows as usize * stride],
            self.width,
            rows,
            stride,
            desc,
        )
        .map_err(|_| {
            S::wrap_sink_error(
                "adapter: pixel slice construction failed"
                    .to_string()
                    .into(),
            )
        })?;
        Ok(Some((y, slice)))
    }

    fn wrap_sink_error(err: crate::decode::SinkError) -> Self::Error {
        S::wrap_sink_error(err)
    }

    fn info(&self) -> Option<&crate::ImageInfo> {
        if self.info_cache.is_some() {
            self.info_cache.as_ref()
        } else {
            self.inner.info()
        }
    }

    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        self.descriptor.or_else(|| self.inner.output_descriptor())
    }

    fn access_pattern(&self) -> AccessPattern {
        self.inner.access_pattern()
    }

    fn is_complete(&self) -> bool {
        self.complete
    }
}

/// [`ReadAt`] over a file, using seek + read per request. Drivers on
/// Unix can substitute `pread` for concurrency; this portable form is
/// correct everywhere and needs no `unsafe`.
#[cfg(feature = "std")]
pub struct FileReadAt {
    file: std::fs::File,
    len: u64,
}

#[cfg(feature = "std")]
impl FileReadAt {
    /// Create a source over an already-open file.
    pub fn new(mut file: std::fs::File) -> Result<Self, SourceError> {
        use std::io::{Seek, SeekFrom};
        let len = file.seek(SeekFrom::End(0)).map_err(|e| SourceError::Io {
            message: e.to_string(),
        })?;
        Ok(Self { file, len })
    }
}

#[cfg(feature = "std")]
impl ReadAt for FileReadAt {
    type Error = SourceError;

    fn len(&mut self) -> Result<u64, Self::Error> {
        Ok(self.len)
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
        use std::io::{Read, Seek, SeekFrom};
        if offset
            .checked_add(out.len() as u64)
            .is_none_or(|e| e > self.len)
        {
            return Err(SourceError::UnexpectedEof {
                offset,
                len: out.len() as u64,
                total: Some(self.len),
            });
        }
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| SourceError::Io {
                message: e.to_string(),
            })?;
        self.file.read_exact(out).map_err(|e| SourceError::Io {
            message: e.to_string(),
        })
    }
}
