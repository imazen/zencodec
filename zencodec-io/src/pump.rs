//! Sequential stream pump: `std::io::Read` into a resumable decoder.

use core::fmt;

use how_far::Pulse;
use zencodec::decode::{DecodeRowSink, Event, IncrementalDecode, OutputInfo, Request, StepError};

use crate::drive::Progressor;
use crate::window::{ProtocolViolation, Window, check_need};

/// Failure of [`pump_read_to_sink`].
#[derive(Debug)]
#[non_exhaustive]
pub enum PumpError<E> {
    /// The decoder failed, the sink failed, or the stop tripped.
    Step(StepError<E>),
    /// Reading the stream failed.
    Io(std::io::Error),
    /// The decoder asked for bytes behind the stream position, or for the
    /// total length before the stream ended. A stream cannot serve either;
    /// wrap it in [`BufferedReadAt`](crate::BufferedReadAt) and use
    /// [`drive_from_read_at`](crate::drive_from_read_at).
    NeedsRandomAccess(Request),
    /// The decoder needs more contiguous bytes than `max_window` allows.
    WindowLimit {
        /// Bytes the decoder asked for in one input.
        needed: usize,
        /// The configured limit.
        limit: usize,
    },
    /// The decoder broke the protocol.
    Protocol(ProtocolViolation),
}

impl<E: fmt::Display> fmt::Display for PumpError<E> {
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

impl<E: core::error::Error + 'static> core::error::Error for PumpError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Step(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Protocol(e) => Some(e),
            _ => None,
        }
    }
}

/// Pump a sequential stream into `decoder`, writing rows into `sink`.
///
/// The pump owns one window over the stream. It reads `read_size` bytes at a
/// time (more when the decoder's request needs a larger contiguous span, up
/// to `max_window`), offers the window, and drops what the decoder took. A
/// decoder decodes straight from the window; rows are emitted as soon as it
/// has enough. Bytes the decoder skips ahead over are read and discarded; a
/// request behind the stream position is [`PumpError::NeedsRandomAccess`].
pub fn pump_read_to_sink<R, D>(
    mut reader: R,
    decoder: &mut D,
    sink: &mut dyn DecodeRowSink,
    pulse: Option<&dyn Pulse>,
    read_size: usize,
    max_window: usize,
) -> Result<OutputInfo, PumpError<D::Error>>
where
    R: std::io::Read,
    D: IncrementalDecode + ?Sized,
{
    let mut progress = Progressor::new(pulse);
    let stop = progress.stop();
    let read_size = read_size.max(512);
    let mut window = Window::new();
    loop {
        let input = window.input();
        let p = decoder.step(input, sink, stop).map_err(PumpError::Step)?;
        progress.observe(decoder, &p.event);
        if p.event == Event::Need {
            check_need(decoder.requested(), &input, p.consumed).map_err(PumpError::Protocol)?;
        }
        window.consume(p.consumed);
        match p.event {
            Event::Done(info) => return Ok(info),
            Event::Need => {}
            _ => continue,
        }
        let requests = decoder.requested();
        // Prefer a request the stream can serve: at or ahead of the window.
        let request = requests
            .iter()
            .copied()
            .find(|r| r.offset().is_some_and(|o| o >= window.base()))
            .unwrap_or(requests[0]);
        let (offset, min_len) = match request {
            Request::AtLeast { offset, min_len } => (offset, min_len),
            Request::Range(r) => (r.offset, r.length as usize),
            _ => return Err(PumpError::NeedsRandomAccess(request)),
        };
        if offset < window.base() {
            return Err(PumpError::NeedsRandomAccess(request));
        }
        if min_len > max_window {
            return Err(PumpError::WindowLimit {
                needed: min_len,
                limit: max_window,
            });
        }
        // Skip forward to `offset`, reading and discarding.
        while offset > window.end() && window.total.is_none() {
            let end = window.end();
            window.reset_to(end);
            let want = ((offset - end) as usize).min(read_size);
            let spare = window.spare(want);
            let n = reader.read(spare).map_err(PumpError::Io)?;
            window.commit(want, n);
            if n == 0 {
                window.total = Some(end);
            }
        }
        if offset <= window.end() {
            window.consume((offset - window.base()) as usize);
        }
        // Read until `min_len` bytes from `offset` are held, or the end.
        while window.len() < min_len.max(1) && window.total.is_none() {
            let end = window.end();
            let want = read_size.max(min_len - window.len().min(min_len));
            let spare = window.spare(want);
            let n = reader.read(spare).map_err(PumpError::Io)?;
            window.commit(want, n);
            if n == 0 {
                window.total = Some(end);
            }
        }
    }
}
