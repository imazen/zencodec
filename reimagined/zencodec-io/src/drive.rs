//! The decode loop and the input window, shared by the blocking and async drivers.
//!
//! A driver owns the input. It reads what the decoder's first request needs,
//! lends that window for as many steps as the decoder can take, and reads
//! again. The blocking and async drivers differ only in how the read happens,
//! so the planning ([`Window::next_read`]) is synchronous and the read itself
//! is the only thing that awaits.

use zencodec::decode::{DecodeEvent, Decoder, Sink};
use zencodec::{
    ByteRange, CodecError, ErrorKind, Input, InputRequest, LimitExceeded, LimitKind, Result,
    SharedPulse, check, error_msg,
};

/// How a driver reads and schedules.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct DriveOptions {
    /// Bytes to ask a source for beyond what the decoder needs at minimum.
    pub read_size: usize,
    /// Most input bytes the driver will hold at once.
    pub max_input_bytes: Option<u64>,
    /// Steps an async driver runs before yielding to its runtime.
    pub steps_per_batch: usize,
    /// Whether an async transcode writes one batch's bytes while the next
    /// batch computes.
    pub pipeline_writes: bool,
}

impl Default for DriveOptions {
    fn default() -> Self {
        Self {
            read_size: 64 * 1024,
            max_input_bytes: None,
            steps_per_batch: 64,
            pipeline_writes: true,
        }
    }
}

impl DriveOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the read-ahead size.
    pub fn with_read_size(mut self, read_size: usize) -> Self {
        self.read_size = read_size.max(1);
        self
    }

    /// Gate the input the driver holds.
    pub fn with_max_input_bytes(mut self, bytes: u64) -> Self {
        self.max_input_bytes = Some(bytes);
        self
    }

    /// Set the steps per async batch.
    pub fn with_steps_per_batch(mut self, steps: usize) -> Self {
        self.steps_per_batch = steps.max(1);
        self
    }

    /// Choose whether writes overlap computation.
    pub fn with_pipeline_writes(mut self, pipeline: bool) -> Self {
        self.pipeline_writes = pipeline;
        self
    }
}

/// One span a source is asked to read. The source shortens `buf` if the
/// input ends inside the span.
#[derive(Debug)]
pub struct RangeRead {
    /// Offset of `buf[0]`.
    pub offset: u64,
    /// Destination, sized to the span.
    pub buf: Vec<u8>,
}

/// What the window must read before the request in hand is met.
pub(crate) enum Next {
    /// Nothing: lend the window.
    Ready,
    /// Append up to `len` bytes at `offset` to the sequential window.
    Read { offset: u64, len: usize },
    /// Read these spans.
    Ranges(Vec<RangeRead>),
    /// Learn the total length.
    Len,
}

/// The bytes a driver holds: one sequential run and any ranges read ahead.
pub(crate) struct Window {
    buf: Vec<u8>,
    /// First live byte of `buf`.
    head: usize,
    /// Offset of `buf[head]` in the input.
    base: u64,
    total: Option<u64>,
    ranges: Vec<RangeRead>,
    read_size: usize,
    max_bytes: Option<u64>,
}

impl Window {
    pub(crate) fn new(opts: &DriveOptions) -> Self {
        Self {
            buf: Vec::new(),
            head: 0,
            base: 0,
            total: None,
            ranges: Vec::new(),
            read_size: opts.read_size.max(1),
            max_bytes: opts.max_input_bytes,
        }
    }

    fn live(&self) -> &[u8] {
        &self.buf[self.head..]
    }

    fn end(&self) -> u64 {
        self.base + self.live().len() as u64
    }

    fn at_end(&self) -> bool {
        self.total == Some(self.end())
    }

    fn admit(&self, more: u64) -> Result<()> {
        let held = self.live().len() as u64
            + self.ranges.iter().map(|r| r.buf.len() as u64).sum::<u64>()
            + more;
        match self.max_bytes {
            Some(limit) if held > limit => Err(CodecError::new(ErrorKind::Limit(
                LimitKind::InputBytes,
            ))
            .with_native(LimitExceeded {
                kind: LimitKind::InputBytes,
                limit,
                actual: Some(held),
            })
            .at()),
            _ => Ok(()),
        }
    }

    /// Move the sequential run so it starts at `offset`.
    fn seek(&mut self, offset: u64) {
        if offset >= self.base && offset <= self.end() {
            self.head += (offset - self.base) as usize;
        } else {
            self.buf.clear();
            self.head = 0;
        }
        self.base = offset;
    }

    fn range_holding(&self, range: &ByteRange) -> Option<&RangeRead> {
        self.ranges.iter().find(|r| {
            let end = r.offset + r.buf.len() as u64;
            r.offset <= range.offset
                && (end >= range.end() || self.total.is_some_and(|t| end >= t.min(range.end())))
        })
    }

    fn in_run(&self, range: &ByteRange) -> bool {
        range.offset >= self.base && (range.end() <= self.end() || self.at_end())
    }

    /// Plan the next read for the decoder's requests. The first request must
    /// be met; later range requests are fetched with it.
    pub(crate) fn next_read(&mut self, requests: &[InputRequest]) -> Result<Next> {
        let Some(first) = requests.first() else {
            return Ok(Next::Ready);
        };
        match *first {
            InputRequest::AtLeast { offset, min_len } => {
                self.seek(offset);
                let have = self.live().len();
                if have >= min_len.max(1) || self.at_end() {
                    return Ok(Next::Ready);
                }
                let len = (min_len - have).max(self.read_size);
                self.admit(len as u64)?;
                Ok(Next::Read {
                    offset: self.end(),
                    len,
                })
            }
            InputRequest::All => {
                self.seek(0);
                if self.at_end() {
                    return Ok(Next::Ready);
                }
                let len = self
                    .total
                    .map_or(self.read_size, |t| (t - self.end()) as usize)
                    .max(1);
                self.admit(len as u64)?;
                Ok(Next::Read {
                    offset: self.end(),
                    len,
                })
            }
            InputRequest::Len => Ok(if self.total.is_some() {
                Next::Ready
            } else {
                Next::Len
            }),
            InputRequest::Range(_) => {
                // Keep only what is still wanted, then fetch every wanted
                // range that is missing in one go.
                let wanted: Vec<ByteRange> = requests
                    .iter()
                    .filter_map(|r| match r {
                        InputRequest::Range(range) => Some(*range),
                        _ => None,
                    })
                    .collect();
                self.ranges.retain(|held| {
                    let end = held.offset + held.buf.len() as u64;
                    wanted.iter().any(|w| held.offset < w.end() && w.offset < end)
                });
                let missing: Vec<ByteRange> = wanted
                    .into_iter()
                    .filter(|w| !self.in_run(w) && self.range_holding(w).is_none())
                    .collect();
                if missing.is_empty() {
                    return Ok(Next::Ready);
                }
                let mut reads = Vec::with_capacity(missing.len());
                for range in missing {
                    let len = usize::try_from(range.length).map_err(|_| {
                        error_msg(ErrorKind::Limit(LimitKind::InputBytes), "range too large")
                    })?;
                    self.admit(len as u64 + reads.iter().map(|r: &RangeRead| r.buf.len() as u64).sum::<u64>())?;
                    let mut buf = Vec::new();
                    buf.try_reserve_exact(len)
                        .map_err(|_| zencodec::error(ErrorKind::OutOfMemory))?;
                    buf.resize(len, 0);
                    reads.push(RangeRead {
                        offset: range.offset,
                        buf,
                    });
                }
                Ok(Next::Ranges(reads))
            }
            _ => Err(error_msg(ErrorKind::Protocol, "unknown input request")),
        }
    }

    /// Room for a sequential read of `len` bytes.
    pub(crate) fn spare(&mut self, len: usize) -> &mut [u8] {
        if self.head > 0 {
            self.buf.drain(..self.head);
            self.head = 0;
        }
        let old = self.buf.len();
        self.buf.resize(old + len, 0);
        &mut self.buf[old..]
    }

    /// Record that the last [`spare`](Self::spare) of `len` bytes received `n`.
    pub(crate) fn commit(&mut self, len: usize, n: usize) {
        let unused = len - n.min(len);
        self.buf.truncate(self.buf.len() - unused);
        if n == 0 {
            self.total = Some(self.end());
        }
    }

    pub(crate) fn store_ranges(&mut self, reads: Vec<RangeRead>, wanted: &[usize]) {
        for (read, want) in reads.into_iter().zip(wanted) {
            if read.buf.len() < *want {
                // The input ended inside the span.
                let end = read.offset + read.buf.len() as u64;
                self.total = Some(self.total.map_or(end, |t| t.min(end)));
            }
            self.ranges.push(read);
        }
    }

    pub(crate) fn set_total(&mut self, total: u64) {
        self.total = Some(total);
    }

    /// The window that answers `request`.
    pub(crate) fn input(&self, request: &InputRequest) -> Input<'_> {
        if let InputRequest::Range(range) = request {
            if let Some(held) = self.range_holding(range) {
                let skip = (range.offset - held.offset) as usize;
                return Input::new(range.offset, &held.buf[skip.min(held.buf.len())..])
                    .with_total_len(self.total);
            }
            if range.offset >= self.base {
                let skip = ((range.offset - self.base) as usize).min(self.live().len());
                return Input::new(range.offset, &self.live()[skip..]).with_total_len(self.total);
            }
        }
        Input::new(self.base, self.live()).with_total_len(self.total)
    }

    /// Drop `n` bytes the decoder consumed from the window lent for `request`.
    pub(crate) fn consume(&mut self, request: &InputRequest, n: usize) {
        if matches!(request, InputRequest::Range(_)) {
            return;
        }
        let n = n.min(self.live().len());
        self.head += n;
        self.base += n as u64;
    }
}

/// How a run of steps ended.
pub(crate) enum Batch {
    /// The decoder's next request needs this read. Never [`Next::Ready`].
    Read(Next),
    /// The step quota ran out.
    Yield,
    /// The decode is complete.
    Done,
}

/// A decode in flight: the decoder, where its rows go, and the bytes it reads.
pub(crate) struct Session<S> {
    pub(crate) decoder: Box<dyn Decoder>,
    pub(crate) sink: S,
    pub(crate) window: Window,
    /// The window last lent to a step that took nothing and asked for more.
    stalled: Option<(InputRequest, u64, usize)>,
}

impl<S: Sink> Session<S> {
    pub(crate) fn new(decoder: Box<dyn Decoder>, sink: S, opts: &DriveOptions) -> Self {
        Self {
            decoder,
            sink,
            window: Window::new(opts),
            stalled: None,
        }
    }

    /// Step until the decoder needs a read, finishes, or `quota` steps pass.
    ///
    /// Before every step the window that answers the decoder's first request
    /// is lent. When the window cannot answer it, the read that would is
    /// returned for the caller to perform, blocking or not.
    pub(crate) fn run(&mut self, quota: usize, pulse: &SharedPulse) -> Result<Batch> {
        for _ in 0..quota {
            check(pulse)?;
            let requests = self.decoder.requested();
            let request = requests.first().copied().unwrap_or(InputRequest::AtLeast {
                offset: 0,
                min_len: 1,
            });
            match self.window.next_read(requests)? {
                Next::Ready => {}
                read => return Ok(Batch::Read(read)),
            }
            let input = self.window.input(&request);
            let view = (request, input.offset, input.bytes.len());
            // Asking again for exactly what it was just lent, having taken
            // none of it, would never end.
            if self.stalled == Some(view) {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "decoder asked again for input it already holds",
                ));
            }
            let step = self.decoder.step(input, &mut self.sink)?;
            if step.consumed > view.2 {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "decoder consumed more than it was lent",
                ));
            }
            self.window.consume(&request, step.consumed);
            self.stalled =
                (step.event == DecodeEvent::NeedInput && step.consumed == 0).then_some(view);
            if step.event == DecodeEvent::Done {
                return Ok(Batch::Done);
            }
        }
        Ok(Batch::Yield)
    }
}
