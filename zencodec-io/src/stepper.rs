//! The byte-stepping dialogue every driver serves.
//!
//! An image decoder ([`IncrementalDecode`]) and a container demuxer speak the
//! same protocol: the driver offers an [`Input`] at some offset, the stage
//! takes a prefix and either produces something or names the bytes it
//! [`requested`](ByteStepper::requested) next. [`ByteStepper`] is that
//! protocol with the stage's output left abstract, so one set of drivers
//! (slice, [`ReadAt`], `std::io::Read`, async) serves both.
//!
//! The drivers here are cursors: each call to `next` services `Need`s until
//! the stage produces one output, and returns it. A loop over a cursor is a
//! complete drive; a pull-style adapter (an animation view over a demuxer)
//! calls `next` once per item it needs.

use alloc::vec::Vec;
use core::convert::Infallible;
use core::fmt;

use enough::Stop;
use zencodec::decode::{ByteRange, Input, Request};
use zencodec::source::ReadAt;

use crate::drive::{DriveOptions, DriveStats, coalesce_into};
use crate::window::{ProtocolViolation, Window, check_need, read_target};

/// A stage that consumes bytes through the sans-I/O dialogue.
pub trait ByteStepper {
    /// What a productive step yields: an image decode event, a demuxed
    /// packet.
    type Output;
    /// The stage's error.
    type Error;

    /// Take a prefix of `input` and report what happened.
    fn step(
        &mut self,
        input: Input<'_>,
        stop: &dyn Stop,
    ) -> Result<Stepped<Self::Output>, Self::Error>;

    /// What the stage needs next, after a [`Step::Need`]. Never empty then.
    fn requested(&self) -> &[Request];
}

/// The result of one [`ByteStepper::step`].
#[derive(Debug)]
pub struct Stepped<O> {
    /// Bytes taken from the front of the input.
    pub consumed: usize,
    /// What the step produced.
    pub step: Step<O>,
}

/// What a step produced.
#[derive(Debug)]
pub enum Step<O> {
    /// Nothing until one of [`requested`](ByteStepper::requested) is met.
    Need,
    /// An output; the stage has more to do.
    Output(O),
    /// The last output: the stage is done.
    Finished(O),
}

/// What a cursor's `next` returns: an output, or the last one.
#[derive(Debug)]
pub enum Yield<O> {
    /// An output; call `next` again for more.
    Output(O),
    /// The last output.
    Finished(O),
}

impl<O> Yield<O> {
    /// The output, either way.
    #[inline]
    pub fn into_inner(self) -> O {
        match self {
            Self::Output(o) | Self::Finished(o) => o,
        }
    }

    /// Whether this was the last output.
    #[inline]
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Finished(_))
    }
}

/// Failure of a drive over any [`ByteStepper`].
#[derive(Debug)]
#[non_exhaustive]
pub enum DriveFailure<E, SE> {
    /// The stage failed (for an image decoder: the decoder, its sink or the
    /// stop).
    Step(E),
    /// The source failed.
    Source(SE),
    /// The stage broke the protocol.
    Protocol(ProtocolViolation),
}

impl<E: fmt::Display, SE: fmt::Display> fmt::Display for DriveFailure<E, SE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Step(e) => write!(f, "{e}"),
            Self::Source(e) => write!(f, "source: {e}"),
            Self::Protocol(e) => write!(f, "protocol: {e}"),
        }
    }
}

impl<E: core::error::Error + 'static, SE: core::error::Error + 'static> core::error::Error
    for DriveFailure<E, SE>
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Step(e) => Some(e),
            Self::Source(e) => Some(e),
            Self::Protocol(e) => Some(e),
        }
    }
}

impl<E> DriveFailure<E, Infallible> {
    /// Widen a source-free failure to any source error.
    pub fn widen<SE>(self) -> DriveFailure<E, SE> {
        match self {
            Self::Step(e) => DriveFailure::Step(e),
            Self::Protocol(p) => DriveFailure::Protocol(p),
            Self::Source(never) => match never {},
        }
    }
}

/// What a cursor's `next` returns for stage `S` over a source failing with
/// `SE`.
type Next<S, SE> =
    Result<Yield<<S as ByteStepper>::Output>, DriveFailure<<S as ByteStepper>::Error, SE>>;

/// The `start..end` a slice drive offers next, for `requests` (post
/// [`check_need`], so `requests` is never empty) over `total` bytes; a
/// sequential request gets at least its minimum and up to `window` more, a
/// range exactly its span.
pub(crate) fn slice_span(
    start: usize,
    end: usize,
    window: usize,
    total: usize,
    requests: &[Request],
) -> (usize, usize) {
    match requests[0] {
        Request::AtLeast { offset, min_len } => {
            let s = offset as usize;
            (s, s.saturating_add(min_len.max(window)).min(total))
        }
        Request::Range(r) => (r.offset as usize, r.end() as usize),
        // Every input carries the total; `check_need` reports a stage that
        // keeps asking.
        _ => (start, end),
    }
}

/// Drives a stage over bytes already in memory, without copying them.
///
/// Every input is a sub-slice carrying the total length. A sequential
/// request gets at least its minimum and up to `window` bytes (a small
/// window exercises boundaries); a range gets exactly its span.
#[derive(Debug)]
pub struct SliceCursor {
    start: usize,
    end: usize,
    window: usize,
    stats: DriveStats,
}

impl SliceCursor {
    /// A cursor offering at most `window` bytes beyond each request's
    /// minimum (at least 1).
    pub fn new(window: usize) -> Self {
        Self {
            start: 0,
            end: 0,
            window: window.max(1),
            stats: DriveStats::default(),
        }
    }

    /// Steps, reads and bytes offered so far.
    pub fn stats(&self) -> DriveStats {
        self.stats
    }

    /// The input to offer the stage next: `bytes[start..end]` clamped to
    /// `bytes` (the clamps also keep a stage's over-consumption or a past
    /// range from slicing out of bounds).
    #[inline]
    pub(crate) fn offer<'b>(&mut self, bytes: &'b [u8]) -> Input<'b> {
        self.end = self.end.min(bytes.len());
        self.start = self.start.min(self.end);
        Input::new(self.start as u64, &bytes[self.start..self.end])
            .with_total_len(bytes.len() as u64)
    }

    /// Record a step and drop `consumed` bytes from the offered front.
    #[inline]
    pub(crate) fn take(&mut self, consumed: usize) {
        self.stats.steps += 1;
        self.start += consumed.min(self.end - self.start);
    }

    /// Point the next offer at the first of `requests` (post
    /// [`check_need`], so `requests` is never empty).
    pub(crate) fn serve(&mut self, bytes: &[u8], requests: &[Request]) {
        let (s, e) = slice_span(self.start, self.end, self.window, bytes.len(), requests);
        self.start = s;
        self.end = e;
        self.stats.reads += 1;
        self.stats.bytes_read += (e - s) as u64;
    }

    /// Step `stage` over `bytes` until it produces an output. Pass the same
    /// `bytes` on every call.
    #[inline]
    pub fn next<S: ByteStepper + ?Sized>(
        &mut self,
        bytes: &[u8],
        stage: &mut S,
        stop: &dyn Stop,
    ) -> Next<S, Infallible> {
        loop {
            let input = self.offer(bytes);
            let p = stage.step(input, stop).map_err(DriveFailure::Step)?;
            self.take(p.consumed);
            if matches!(p.step, Step::Need) {
                check_need(stage.requested(), &input, p.consumed)
                    .map_err(DriveFailure::Protocol)?;
            }
            match p.step {
                Step::Output(o) => return Ok(Yield::Output(o)),
                Step::Finished(o) => return Ok(Yield::Finished(o)),
                Step::Need => self.serve(bytes, stage.requested()),
            }
        }
    }
}

/// Drives a stage over a random-access source.
///
/// Sequential requests read forward into a driver-owned window (a short
/// read marks the end of the source; [`ReadAt::len`] is called only when the
/// stage asks for the length). Range requests are coalesced
/// ([`DriveOptions::max_gap`]) and fetched with one
/// [`read_many_at`](ReadAt::read_many_at) per round, so a concurrent source
/// overlaps their round trips; later ranges of the same round are served
/// from the fetched buffers. A source already in memory
/// ([`ReadAt::as_slice`]) is driven without copying.
#[derive(Debug)]
pub struct ReadAtCursor {
    opts: DriveOptions,
    stats: DriveStats,
    window: Window,
    started: bool,
    /// Buffers fetched by the last range round, by absolute offset.
    fetched: Vec<(u64, Vec<u8>)>,
    ranges: Vec<ByteRange>,
    coalesced: Vec<ByteRange>,
    /// Which input to offer next: the window, or a fetched buffer from `at`.
    from_fetched: Option<(usize, usize)>,
    /// Set once the source turned out to be in memory.
    slice: Option<SliceCursor>,
}

impl ReadAtCursor {
    /// A cursor with the given read sizes.
    pub fn new(opts: DriveOptions) -> Self {
        Self {
            opts,
            stats: DriveStats::default(),
            window: Window::new(),
            started: false,
            fetched: Vec::new(),
            ranges: Vec::new(),
            coalesced: Vec::new(),
            from_fetched: None,
            slice: None,
        }
    }

    /// Steps, reads and bytes read so far.
    pub fn stats(&self) -> DriveStats {
        match &self.slice {
            Some(s) => s.stats(),
            None => self.stats,
        }
    }

    /// The input to offer the stage next: the fetched buffer in play, or
    /// the window. The first call detects an in-memory source and starts
    /// the length hint.
    pub(crate) fn offer<'a, Src: ReadAt + ?Sized>(&'a mut self, source: &'a Src) -> Input<'a> {
        if !self.started {
            self.started = true;
            if source.as_slice().is_some() {
                self.slice = Some(SliceCursor::new(self.opts.read_size));
            } else {
                self.window.total = source.len_hint();
            }
        }
        if let Some(cursor) = &mut self.slice {
            let bytes = source.as_slice().expect("the source stays in memory");
            return cursor.offer(bytes);
        }
        match self.from_fetched {
            Some((i, at)) => {
                let (off, buf) = &self.fetched[i];
                Input::new(off + at as u64, &buf[at..]).with_total_len_opt(self.window.total)
            }
            None => self.window.input(),
        }
    }

    /// Record a step and drop `consumed` bytes from the offered front.
    #[inline]
    pub(crate) fn take(&mut self, consumed: usize) {
        if let Some(cursor) = &mut self.slice {
            return cursor.take(consumed);
        }
        self.stats.steps += 1;
        match &mut self.from_fetched {
            Some((_, at)) => *at += consumed,
            None => self.window.consume(consumed),
        }
    }

    /// Step `stage` over `source` until it produces an output. Pass the same
    /// source on every call.
    #[inline]
    pub fn next<S, Src>(
        &mut self,
        source: &mut Src,
        stage: &mut S,
        stop: &dyn Stop,
    ) -> Next<S, Src::Error>
    where
        S: ByteStepper + ?Sized,
        Src: ReadAt + ?Sized,
    {
        if !self.started && source.as_slice().is_some() {
            self.started = true;
            self.slice = Some(SliceCursor::new(self.opts.read_size));
        }
        if let Some(cursor) = &mut self.slice {
            let bytes = source.as_slice().expect("the source stays in memory");
            return cursor.next(bytes, stage, stop).map_err(DriveFailure::widen);
        }
        loop {
            let input = self.offer(source);
            let p = stage.step(input, stop).map_err(DriveFailure::Step)?;
            if matches!(p.step, Step::Need) {
                check_need(stage.requested(), &input, p.consumed)
                    .map_err(DriveFailure::Protocol)?;
            }
            self.take(p.consumed);
            match p.step {
                Step::Output(o) => return Ok(Yield::Output(o)),
                Step::Finished(o) => return Ok(Yield::Finished(o)),
                Step::Need => self.serve(stage.requested(), source)?,
            }
        }
    }

    /// Make the input for the first of `requests` available.
    pub(crate) fn serve<E, Src: ReadAt + ?Sized>(
        &mut self,
        requests: &[Request],
        source: &mut Src,
    ) -> Result<(), DriveFailure<E, Src::Error>> {
        let window = &mut self.window;
        match requests[0] {
            Request::AtLeast { offset, min_len } => {
                self.from_fetched = None;
                if offset < window.base() || offset > window.end() {
                    window.reset_to(offset);
                } else {
                    window.consume((offset - window.base()) as usize);
                }
                // Read until the window holds a whole number of requests at
                // least `read_size` long, or the source ends (a short read
                // marks the end).
                let target = read_target(min_len, self.opts.read_size);
                while window.len() < target && window.total != Some(window.end()) {
                    let end = window.end();
                    let n_want = target - window.len();
                    let spare = window.spare(n_want);
                    let n = source.read_at(end, spare).map_err(DriveFailure::Source)?;
                    window.commit(n);
                    self.stats.reads += 1;
                    self.stats.bytes_read += n as u64;
                    if n < n_want {
                        window.total = Some(end + n as u64);
                    }
                }
            }
            Request::Range(first) => {
                let holds = |(off, buf): &(u64, Vec<u8>)| {
                    first.offset >= *off && first.end() <= off + buf.len() as u64
                };
                // Serve from the last round's buffers if one holds it.
                if let Some(i) = self.fetched.iter().position(holds) {
                    self.from_fetched = Some((i, (first.offset - self.fetched[i].0) as usize));
                    return Ok(());
                }
                // New round: every range request, coalesced, one batch.
                self.ranges.clear();
                self.ranges.extend(requests.iter().filter_map(|r| match r {
                    Request::Range(r) => Some(*r),
                    _ => None,
                }));
                coalesce_into(&self.ranges, self.opts.max_gap, &mut self.coalesced);
                self.fetched
                    .resize_with(self.coalesced.len(), Default::default);
                for (slot, r) in self.fetched.iter_mut().zip(&self.coalesced) {
                    slot.0 = r.offset;
                    slot.1.clear();
                    slot.1.resize(r.length as usize, 0);
                }
                {
                    let mut reqs: Vec<(u64, &mut [u8])> = self
                        .fetched
                        .iter_mut()
                        .map(|(off, buf)| (*off, &mut buf[..]))
                        .collect();
                    source
                        .read_many_at(&mut reqs)
                        .map_err(DriveFailure::Source)?;
                }
                self.stats.reads += 1;
                self.stats.bytes_read += self.coalesced.iter().map(|r| r.length).sum::<u64>();
                let i = self
                    .fetched
                    .iter()
                    .position(holds)
                    .expect("coalesced ranges cover every request");
                self.from_fetched = Some((i, (first.offset - self.fetched[i].0) as usize));
            }
            Request::Len => {
                self.from_fetched = None;
                if window.total.is_none() {
                    window.total = Some(source.len().map_err(DriveFailure::Source)?);
                }
            }
            _ => return Err(DriveFailure::Protocol(ProtocolViolation::EmptyRequest)),
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
pub use read::{PumpFailure, ReadCursor};

#[cfg(feature = "std")]
mod read {
    use super::*;

    /// Failure of a drive over a sequential `std::io::Read` stream.
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum PumpFailure<E> {
        /// The stage failed.
        Step(E),
        /// Reading the stream failed.
        Io(std::io::Error),
        /// The stage asked for bytes behind the stream position, or for the
        /// total length before the stream ended. A stream cannot serve
        /// either; wrap it in [`BufferedReadAt`](crate::BufferedReadAt) and
        /// use a [`ReadAtCursor`].
        NeedsRandomAccess(Request),
        /// The stage needs more contiguous bytes than `max_window` allows.
        WindowLimit {
            /// Bytes the stage asked for in one input.
            needed: usize,
            /// The configured limit.
            limit: usize,
        },
        /// The stage broke the protocol.
        Protocol(ProtocolViolation),
    }

    impl<E: fmt::Display> fmt::Display for PumpFailure<E> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Step(e) => write!(f, "{e}"),
                Self::Io(e) => write!(f, "reading the stream: {e}"),
                Self::NeedsRandomAccess(r) => write!(
                    f,
                    "decoder needs random access ({r:?}); a sequential stream cannot serve it"
                ),
                Self::WindowLimit { needed, limit } => write!(
                    f,
                    "decoder needs {needed} contiguous bytes, over the {limit}-byte window limit"
                ),
                Self::Protocol(e) => write!(f, "protocol: {e}"),
            }
        }
    }

    impl<E: core::error::Error + 'static> core::error::Error for PumpFailure<E> {
        fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
            match self {
                Self::Step(e) => Some(e),
                Self::Io(e) => Some(e),
                Self::Protocol(e) => Some(e),
                _ => None,
            }
        }
    }

    /// Drives a stage over a sequential stream.
    ///
    /// The cursor owns one window over the stream. It reads `read_size`
    /// bytes at a time (more when a request needs a larger contiguous span,
    /// up to `max_window`), offers the window, and drops what the stage
    /// took. Bytes the stage skips ahead over are read and discarded; a
    /// request behind the stream position is
    /// [`PumpFailure::NeedsRandomAccess`].
    #[derive(Debug)]
    pub struct ReadCursor {
        read_size: usize,
        max_window: usize,
        window: Window,
    }

    impl ReadCursor {
        /// A cursor reading `read_size` bytes at a time (at least 512) and
        /// holding at most `max_window` contiguous bytes for one request.
        pub fn new(read_size: usize, max_window: usize) -> Self {
            Self {
                read_size: read_size.max(512),
                max_window,
                window: Window::new(),
            }
        }

        /// The input to offer the stage next: the whole window.
        #[inline]
        pub(crate) fn offer(&self) -> Input<'_> {
            self.window.input()
        }

        /// Drop `consumed` bytes from the offered front.
        #[inline]
        pub(crate) fn take(&mut self, consumed: usize) {
            self.window.consume(consumed);
        }

        /// Read from `reader` until the first of `requests` the stream can
        /// serve is met (post [`check_need`], so `requests` is never empty).
        pub(crate) fn serve<R, E>(
            &mut self,
            reader: &mut R,
            requests: &[Request],
        ) -> Result<(), PumpFailure<E>>
        where
            R: std::io::Read + ?Sized,
        {
            let window = &mut self.window;
            // Prefer a request the stream can serve: at or ahead of the
            // window.
            let request = requests
                .iter()
                .copied()
                .find(|r| r.offset().is_some_and(|o| o >= window.base()))
                .unwrap_or(requests[0]);
            let (offset, min_len) = match request {
                Request::AtLeast { offset, min_len } => (offset, min_len),
                Request::Range(r) => (r.offset, r.length as usize),
                _ => return Err(PumpFailure::NeedsRandomAccess(request)),
            };
            if offset < window.base() {
                return Err(PumpFailure::NeedsRandomAccess(request));
            }
            if min_len > self.max_window {
                return Err(PumpFailure::WindowLimit {
                    needed: min_len,
                    limit: self.max_window,
                });
            }
            // Skip forward to `offset`, reading and discarding.
            while offset > window.end() && window.total.is_none() {
                let end = window.end();
                window.reset_to(end);
                let want = ((offset - end) as usize).min(self.read_size);
                let spare = window.spare(want);
                let n = reader.read(spare).map_err(PumpFailure::Io)?;
                window.commit(n);
                if n == 0 {
                    window.total = Some(end);
                }
            }
            if offset <= window.end() {
                window.consume((offset - window.base()) as usize);
            }
            // Read until `min_len` bytes from `offset` are held, or the
            // end.
            let target = read_target(min_len, self.read_size);
            while window.len() < min_len.max(1) && window.total.is_none() {
                let end = window.end();
                let want = target - window.len().min(target);
                let spare = window.spare(want);
                let n = reader.read(spare).map_err(PumpFailure::Io)?;
                window.commit(n);
                if n == 0 {
                    window.total = Some(end);
                }
            }
            Ok(())
        }

        /// Step `stage` over `reader` until it produces an output. Pass the
        /// same reader on every call.
        #[inline]
        pub fn next<S, R>(
            &mut self,
            reader: &mut R,
            stage: &mut S,
            stop: &dyn Stop,
        ) -> Result<Yield<S::Output>, PumpFailure<S::Error>>
        where
            S: ByteStepper + ?Sized,
            R: std::io::Read + ?Sized,
        {
            loop {
                let input = self.offer();
                let p = stage.step(input, stop).map_err(PumpFailure::Step)?;
                if matches!(p.step, Step::Need) {
                    check_need(stage.requested(), &input, p.consumed)
                        .map_err(PumpFailure::Protocol)?;
                }
                self.take(p.consumed);
                match p.step {
                    Step::Output(o) => return Ok(Yield::Output(o)),
                    Step::Finished(o) => return Ok(Yield::Finished(o)),
                    Step::Need => self.serve(reader, stage.requested())?,
                }
            }
        }
    }
}
