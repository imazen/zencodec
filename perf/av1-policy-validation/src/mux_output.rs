//! Private native witness: destination capabilities come from the actual writer,
//! while packet clocks/epochs and finish patches are checked before their effects.
use enough::Stop;
use std::{
    error::Error,
    fmt,
    io::{self, Seek, SeekFrom, Write},
};
use zencodec_media::{
    container::{MuxHint, MuxTiming, Muxer},
    packet::Packet,
    session::{ErasedError, ProtocolViolation, SessionError, UnsupportedMedia},
    track::TrackInfo,
};

type Cause = Box<dyn Error + Send + Sync>;

pub trait Destination: Write {
    fn can_patch(&self) -> bool;
    fn position(&mut self, relative: u64) -> io::Result<()>;
}

pub struct Forward<W>(pub W);
impl<W: Write> Write for Forward<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
impl<W: Write> Destination for Forward<W> {
    fn can_patch(&self) -> bool {
        false
    }
    fn position(&mut self, _: u64) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub struct SeekBack<W> {
    writer: W,
    start: u64,
}
impl<W: Write + Seek> SeekBack<W> {
    pub fn new(mut writer: W) -> io::Result<Self> {
        let start = writer.stream_position()?;
        Ok(Self { writer, start })
    }
}
impl<W: Write> Write for SeekBack<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writer.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}
impl<W: Write + Seek> Destination for SeekBack<W> {
    fn can_patch(&self) -> bool {
        true
    }
    fn position(&mut self, relative: u64) -> io::Result<()> {
        let absolute = self
            .start
            .checked_add(relative)
            .ok_or(io::ErrorKind::InvalidInput)?;
        let actual = self.writer.seek(SeekFrom::Start(absolute))?;
        if actual != absolute {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Stage {
    Prepare,
    Packet,
    Drain,
    Finish,
    Patch,
}

#[derive(Debug)]
pub struct Failure {
    pub stage: Stage,
    pub emitted: u64,
    pub cause: Cause,
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "mux {:?} failed after {} appended bytes",
            self.stage, self.emitted
        )
    }
}
impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.cause)
    }
}

#[derive(Default, Debug)]
pub struct Stats {
    pub emitted: u64,
    pub patches: usize,
    pub patch_bytes: usize,
    // Observed pending byte lengths, not allocation/RSS measurements.
    pub max_pending: usize,
}

pub struct Output<M, D> {
    mux: Option<M>,
    destination: D,
    tracks: Vec<TrackInfo>,
    timing: MuxTiming,
    stats: Stats,
}

fn stopped(stop: &dyn Stop) -> Result<(), Cause> {
    stop.check()
        .map_err(|reason| Box::new(SessionError::<ErasedError>::Cancelled(reason)) as Cause)
}
fn protocol(kind: ProtocolViolation) -> Cause {
    Box::new(SessionError::<ErasedError>::Protocol(kind))
}

fn write_all(
    writer: &mut impl Write,
    mut bytes: &[u8],
    count: &mut u64,
    stop: &dyn Stop,
) -> Result<(), Cause> {
    while !bytes.is_empty() {
        stopped(stop)?;
        match writer.write(bytes) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
            Ok(n) if n <= bytes.len() => {
                *count = count
                    .checked_add(n as u64)
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
                bytes = &bytes[n..];
            }
            Ok(_) => return Err(io::Error::from(io::ErrorKind::InvalidData).into()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    stopped(stop)
}

impl<M: Muxer, D: Destination> Output<M, D> {
    pub fn new(
        mut mux: M,
        destination: D,
        tracks: &[TrackInfo],
        stop: &dyn Stop,
    ) -> Result<Self, Failure> {
        let mut prepare = || -> Result<_, Cause> {
            stopped(stop)?;
            let hint = if destination.can_patch() {
                MuxHint::default()
            } else {
                MuxHint::forward_only()
            };
            let timing = mux.begin(tracks, hint)?;
            if timing.patches_at_finish() && !destination.can_patch() {
                return Err(Box::new(SessionError::<ErasedError>::Unsupported(
                    UnsupportedMedia::ForwardOnlyLayout,
                )));
            }
            stopped(stop)?;
            Ok(timing)
        };
        let timing = prepare().map_err(|cause| Failure {
            stage: Stage::Prepare,
            emitted: 0,
            cause,
        })?;
        let mut this = Self {
            mux: None,
            destination,
            tracks: tracks.to_vec(),
            timing,
            stats: Stats::default(),
        };
        this.drain(&mut mux, stop)
            .map_err(|cause| this.failure(Stage::Drain, cause))?;
        this.mux = Some(mux);
        Ok(this)
    }
    fn failure(&self, stage: Stage, cause: Cause) -> Failure {
        Failure {
            stage,
            emitted: self.stats.emitted,
            cause,
        }
    }
    fn drain(&mut self, mux: &mut M, stop: &dyn Stop) -> Result<(), Cause> {
        let bytes = mux.pending_output();
        self.stats.max_pending = self.stats.max_pending.max(bytes.len());
        write_all(&mut self.destination, bytes, &mut self.stats.emitted, stop)?;
        mux.consume_output(bytes.len());
        Ok(())
    }
    pub fn is_failed(&self) -> bool {
        self.mux.is_none()
    }
    pub fn send(&mut self, packet: Packet, stop: &dyn Stop) -> Result<(), Failure> {
        let mut mux = self.mux.take().ok_or_else(|| {
            self.failure(
                Stage::Packet,
                Box::new(SessionError::<ErasedError>::Finished),
            )
        })?;
        let work = || -> Result<(), Cause> {
            stopped(stop)?;
            let track = self
                .tracks
                .iter()
                .find(|track| track.index() == packet.track())
                .ok_or_else(|| protocol(ProtocolViolation::NotConfigured))?;
            if packet.config_epoch() != track.config_epoch() {
                return Err(protocol(ProtocolViolation::EpochMismatch));
            }
            for timestamp in [packet.pts(), packet.dts()].into_iter().flatten() {
                if timestamp.base() != track.time_base() {
                    return Err(protocol(ProtocolViolation::ClockMismatch));
                }
            }
            mux.write_packet(packet, stop)?;
            self.drain(&mut mux, stop)?;
            Ok(())
        };
        work().map_err(|cause| self.failure(Stage::Packet, cause))?;
        self.mux = Some(mux);
        Ok(())
    }
    pub fn finish(mut self, stop: &dyn Stop) -> Result<Stats, Failure> {
        let mux = self.mux.take().ok_or_else(|| {
            self.failure(
                Stage::Finish,
                Box::new(SessionError::<ErasedError>::Finished),
            )
        })?;
        stopped(stop).map_err(|cause| self.failure(Stage::Finish, cause))?;
        let finish = mux
            .finish(stop)
            .map_err(|e| self.failure(Stage::Finish, Box::new(e)))?;
        let total = finish
            .validate(self.stats.emitted)
            .map_err(|e| self.failure(Stage::Finish, Box::new(e)))?;
        if !finish.patches.is_empty()
            && (!self.destination.can_patch() || !self.timing.patches_at_finish())
        {
            return Err(self.failure(
                Stage::Finish,
                Box::new(SessionError::<ErasedError>::Unsupported(
                    UnsupportedMedia::ForwardOnlyLayout,
                )),
            ));
        }
        self.stats.max_pending = self.stats.max_pending.max(finish.tail.len());
        write_all(
            &mut self.destination,
            &finish.tail,
            &mut self.stats.emitted,
            stop,
        )
        .map_err(|cause| self.failure(Stage::Finish, cause))?;
        self.stats.patches = finish.patches.len();
        for patch in finish.patches {
            self.destination
                .position(patch.offset)
                .map_err(|e| self.failure(Stage::Patch, Box::new(e)))?;
            let mut written = 0;
            write_all(&mut self.destination, &patch.bytes, &mut written, stop)
                .map_err(|cause| self.failure(Stage::Patch, cause))?;
            self.stats.patch_bytes += patch.bytes.len();
        }
        if self.stats.patches > 0 {
            self.destination
                .position(total)
                .map_err(|e| self.failure(Stage::Patch, Box::new(e)))?;
        }
        self.destination
            .flush()
            .map_err(|e| self.failure(Stage::Finish, Box::new(e)))?;
        stopped(stop).map_err(|cause| self.failure(Stage::Finish, cause))?;
        Ok(self.stats)
    }
}

#[path = "mux_output_tests.rs"]
mod tests;
