//! Sequential stream pump: `std::io::Read` into a resumable decoder.

use how_far::Pulse;
use zencodec::decode::{DecodeRowSink, Event, IncrementalDecode, OutputInfo, StepError};

use crate::drive::Progressor;
use crate::stepper::{PumpFailure, ReadCursor};
use crate::window::check_need;

/// Failure of [`pump_read_to_sink`]: a [`PumpFailure`] whose stage error is
/// the decoder's [`StepError`].
pub type PumpError<E> = PumpFailure<StepError<E>>;

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
    let mut cursor = ReadCursor::new(read_size, max_window);
    loop {
        let input = cursor.offer();
        let p = decoder.step(input, sink, pulse).map_err(PumpError::Step)?;
        if p.event == Event::Need {
            check_need(decoder.requested(), &input, p.consumed).map_err(PumpError::Protocol)?;
        }
        cursor.take(p.consumed);
        progress.observe(decoder, &p.event);
        match p.event {
            Event::Done(info) => return Ok(info),
            Event::Need => cursor.serve(&mut reader, decoder.requested())?,
            _ => {}
        }
    }
}
