//! Private single-segment driver. Uses native progress before presentation filtering.
use presentation_map_experiment::{Error as MapError, Interval, Time, retained::RetainedCursor};
use zencodec::{
    Pulse,
    time::{TimeBase, Timestamp},
};
use zencodec_media::{
    frame::Frame,
    packet::Packet,
    session::{PumpError, SessionError, VideoDecoder, drain_decoder, send_packet_until_accepted},
};

#[derive(Debug)]
pub enum ProjectionError<E> {
    MissingTiming,
    Mapping(MapError),
    Output(E),
}
impl<E: core::fmt::Debug> core::fmt::Display for ProjectionError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::MissingTiming => "missing presentation timing",
            Self::Mapping(_) => "presentation mapping failed",
            Self::Output(_) => "presentation output failed",
        })
    }
}
impl<E: core::error::Error + 'static> core::error::Error for ProjectionError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Output(error) => Some(error),
            _ => None,
        }
    }
}
pub type Error<N, E> = PumpError<SessionError<N>, ProjectionError<E>>;

/// Native decoder must already be configured and admitted. Owns no frame queue.
/// Input/source metadata obligations are not discharged by finite frame coverage.
pub struct Pass<D> {
    state: Option<(D, RetainedCursor)>,
    clock: TimeBase,
}
impl<D: VideoDecoder> Pass<D> {
    pub fn new(decoder: D, cursor: RetainedCursor, clock: TimeBase) -> Self {
        Self {
            state: Some((decoder, cursor)),
            clock,
        }
    }
    pub fn send<E>(
        &mut self,
        packet: &mut Packet,
        pulse: Option<&dyn Pulse>,
        mut output: impl FnMut(Frame) -> Result<(), E>,
    ) -> Result<(), Error<D::Error, E>> {
        // Taking ownership first also makes callback/native unwinding terminal.
        let (mut decoder, mut cursor) = self
            .state
            .take()
            .ok_or(PumpError::Upstream(SessionError::Finished))?;
        send_packet_until_accepted(&mut decoder, packet, pulse, |frame| {
            project(&mut cursor, self.clock, frame, &mut output)
        })?;
        self.state = Some((decoder, cursor));
        Ok(())
    }
    pub fn finish<E>(
        &mut self,
        pulse: Option<&dyn Pulse>,
        mut output: impl FnMut(Frame) -> Result<(), E>,
    ) -> Result<Interval, Error<D::Error, E>> {
        let (mut decoder, mut cursor) = self
            .state
            .take()
            .ok_or(PumpError::Upstream(SessionError::Finished))?;
        // Always drain to actual source completion; coverage is not metadata EOF.
        drain_decoder(&mut decoder, pulse, |frame| {
            project(&mut cursor, self.clock, frame, &mut output)
        })?;
        cursor
            .finish()
            .map_err(|error| PumpError::Downstream(ProjectionError::Mapping(error)))
    }
}
fn project<E>(
    cursor: &mut RetainedCursor,
    clock: TimeBase,
    frame: Frame,
    output: &mut impl FnMut(Frame) -> Result<(), E>,
) -> Result<(), ProjectionError<E>> {
    let mut timing = frame.timing();
    let start = timing.pts.ok_or(ProjectionError::MissingTiming)?;
    let duration = i64::try_from(
        timing
            .duration_ticks
            .ok_or(ProjectionError::MissingTiming)?,
    )
    .map_err(|_| ProjectionError::Mapping(MapError::Overflow))?;
    let end = start
        .ticks()
        .checked_add(duration)
        .ok_or(ProjectionError::Mapping(MapError::Overflow))?;
    let interval = Interval::new(
        Time::timestamp(start),
        Time::timestamp(Timestamp::new(end, start.base())),
    )
    .map_err(ProjectionError::Mapping)?;
    let Some(mapped) = cursor.push(interval).map_err(ProjectionError::Mapping)? else {
        return Ok(());
    };
    let start = mapped
        .start()
        .on_clock(clock)
        .map_err(ProjectionError::Mapping)?;
    let end = mapped
        .end()
        .on_clock(clock)
        .map_err(ProjectionError::Mapping)?;
    let duration = end
        .ticks()
        .checked_sub(start.ticks())
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(ProjectionError::Mapping(MapError::Overflow))?;
    timing.pts = Some(start);
    timing.duration_ticks = Some(duration);
    output(frame.represent(timing, frame.presentation())).map_err(ProjectionError::Output)
}
