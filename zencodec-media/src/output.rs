//! Checked synchronous container output for forward writers and seekable files.
//!
//! [`PreparedOutput::new`](crate::output::PreparedOutput::new) requires exact packet timing and owns the actual begun
//! muxer and destination without publishing its header. [`PreparedOutput::publish`](crate::output::PreparedOutput::publish)
//! consumes that preparation. [`Output::new`](crate::output::Output::new) combines those steps for callers
//! that do not need a policy decision between them. Unsupported timing is an
//! error, never a fallback to container-chosen timing.
//!
//! ```
//! use zencodec_media::{container::Muxer, output::{Failure, Forward, Output, Stats},
//!     packet::Packet, track::TrackInfo};
//! fn write<M: Muxer, W: std::io::Write>(
//!     muxer: M, writer: W, tracks: &[TrackInfo], packets: impl IntoIterator<Item = Packet>,
//! ) -> Result<Stats, Failure> {
//!     let pulse = &zencodec::how_far::NoPulse;
//!     let mut output = Output::new(muxer, Forward(writer), tracks, &pulse)?;
//!     for packet in packets { output.send(packet, &pulse)?; }
//!     output.finish(&pulse)
//! }
//! ```
//!
//! To ask policy before any write, replace `Output::new` with
//! [`PreparedOutput::new`](crate::output::PreparedOutput::new), retain the result while deciding, then call
//! [`PreparedOutput::publish`](crate::output::PreparedOutput::publish). Use [`SeekBack::new`](crate::output::SeekBack::new) instead of [`Forward`](crate::output::Forward) for
//! a file whose current position is the start of this output.
//!
//! These drivers may block on `Write`. Byte availability does not establish safe
//! encoder backpressure: finish or separately qualify upstream raw-buffer retention
//! before sending packets to a slow writer. No whole-operation memory bound is
//! implied. Dropping either state does not implicitly finish or publish.

use crate::{
    container::{MuxHint, MuxTiming, Muxer},
    packet::Packet,
    session::{ErasedError, ProtocolViolation, SessionError, UnsupportedMedia},
    track::TrackInfo,
};
use alloc::{boxed::Box, vec::Vec};

use std::{
    error::Error,
    fmt,
    io::{self, Write},
};
use zencodec::operation::DestinationFacts;

type Cause = Box<dyn Error + Send + Sync>;

/// Shared destination wrappers; media and raster routes use the same origin rules.
pub use zencodec_io::destination::{Destination, Forward, SeekBack};

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
/// Stage at which output stopped.
pub enum Stage {
    /// Validate and begin the retained muxer before publication.
    Prepare,
    /// Validate or process a packet, including draining its output.
    Packet,
    /// Publish the prepared header.
    Drain,
    /// Finalize, append the tail or flush.
    Finish,
    /// Apply or seek around a final patch.
    Patch,
}

#[derive(Debug)]
#[non_exhaustive]
/// Terminal failure with original cause and confirmed append progress.
/// `emitted` excludes overwrites; a failed patch may have partial effects.
pub struct Failure {
    /// Stage of the failure.
    pub stage: Stage,
    /// Confirmed appended bytes, including any partial failed write.
    pub emitted: u64,
    /// Original I/O, native codec, cancellation or protocol error.
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
#[non_exhaustive]
/// Successful output progress; byte counts are not memory measurements.
pub struct Stats {
    /// Confirmed appended bytes, including any partial failed write.
    pub emitted: u64,
    /// Number of completed finish patches.
    pub patches: usize,
    /// Total bytes overwritten by finish patches.
    pub patch_bytes: usize,
    /// Largest observed pending byte slice, not an allocation or RSS bound.
    pub max_pending: usize,
}

/// Active output driver. Any `send` failure or unwind makes further sends fail.
/// `finish` consumes the driver; dropping it never retries effects.
pub struct Output<M, D> {
    mux: Option<M>,
    destination: D,
    tracks: Vec<TrackInfo>,
    timing: MuxTiming,
    exact_timing: bool,
    stats: Stats,
}

/// Owns the actual begun muxer and destination without publishing its header.
/// Dropping it abandons only private preparation. Publication consumes the token.
pub struct PreparedOutput<M, D>(Output<M, D>);
impl<M: Muxer, D: Destination> PreparedOutput<M, D> {
    /// Require exact packet timing from the actual muxer, or fail before writing.
    pub fn new(
        mux: M,
        destination: D,
        tracks: &[TrackInfo],
        pulse: &dyn zencodec::Pulse,
    ) -> Result<Self, Failure> {
        Output::prepare_unpublished(mux, destination, tracks, pulse, true, M::begin_exact_timing)
            .map(Self)
    }
    /// Timing/layout of the retained muxer. Streaming bytes do not certify safe
    /// backpressure against an upstream encoder's raw-buffer ownership.
    pub const fn timing(&self) -> MuxTiming {
        self.0.timing
    }
    /// Facts of this exact preparation for policy and diagnostics. No bytes are
    /// written, and the prepared muxer/writer remain owned by this object.
    pub fn facts(&self) -> DestinationFacts {
        self.0.facts()
    }
    pub(crate) fn validate_packet(&self, packet: &Packet) -> Result<(), Cause> {
        self.0.validate_packet(packet)
    }
    /// Publish the retained header; any failure consumes the preparation.
    pub fn publish(self, pulse: &dyn zencodec::Pulse) -> Result<Output<M, D>, Failure> {
        let mut output = self.0;
        let mut mux = output.mux.take().expect("prepared muxer is retained");
        output
            .drain(&mut mux, pulse)
            .map_err(|cause| output.failure(Stage::Drain, cause))?;
        output.mux = Some(mux);
        Ok(output)
    }
}

fn stopped(pulse: &dyn zencodec::Pulse) -> Result<(), Cause> {
    pulse
        .check()
        .map_err(|reason| Box::new(SessionError::<ErasedError>::Cancelled(reason)) as Cause)
}
fn protocol(kind: ProtocolViolation) -> Cause {
    Box::new(SessionError::<ErasedError>::Protocol(kind))
}

fn write_all(
    writer: &mut impl Write,
    mut bytes: &[u8],
    count: &mut u64,
    pulse: &dyn zencodec::Pulse,
) -> Result<(), Cause> {
    while !bytes.is_empty() {
        stopped(pulse)?;
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
    stopped(pulse)
}

impl<M: Muxer, D: Destination> Output<M, D> {
    /// Explicitly allow the muxer's ordinary timing behavior, then publish.
    /// This does not guarantee exact packet durations. Prefer [`Output::new`](crate::output::Output::new).
    pub fn with_container_timing(
        mux: M,
        destination: D,
        tracks: &[TrackInfo],
        pulse: &dyn zencodec::Pulse,
    ) -> Result<Self, Failure> {
        Self::prepare(mux, destination, tracks, pulse, false, M::begin)
    }

    /// Require exact packet timing from the actual muxer, or fail before writing.
    pub fn new(
        mux: M,
        destination: D,
        tracks: &[TrackInfo],
        pulse: &dyn zencodec::Pulse,
    ) -> Result<Self, Failure> {
        Self::prepare(mux, destination, tracks, pulse, true, M::begin_exact_timing)
    }

    fn prepare(
        mux: M,
        destination: D,
        tracks: &[TrackInfo],
        pulse: &dyn zencodec::Pulse,
        exact_timing: bool,
        begin: impl FnOnce(&mut M, &[TrackInfo], MuxHint) -> Result<MuxTiming, SessionError<M::Error>>,
    ) -> Result<Self, Failure> {
        PreparedOutput(Self::prepare_unpublished(
            mux,
            destination,
            tracks,
            pulse,
            exact_timing,
            begin,
        )?)
        .publish(pulse)
    }

    fn prepare_unpublished(
        mut mux: M,
        destination: D,
        tracks: &[TrackInfo],
        pulse: &dyn zencodec::Pulse,
        exact_timing: bool,
        begin: impl FnOnce(&mut M, &[TrackInfo], MuxHint) -> Result<MuxTiming, SessionError<M::Error>>,
    ) -> Result<Self, Failure> {
        let prepare = || -> Result<_, Cause> {
            stopped(pulse)?;
            let hint = if destination.can_patch() {
                MuxHint::default()
            } else {
                MuxHint::forward_only()
            };
            let timing = begin(&mut mux, tracks, hint)?;
            if timing.patches_at_finish() && !destination.can_patch() {
                return Err(Box::new(SessionError::<ErasedError>::Unsupported(
                    UnsupportedMedia::ForwardOnlyLayout,
                )));
            }
            stopped(pulse)?;
            Ok(timing)
        };
        let timing = prepare().map_err(|cause| Failure {
            stage: Stage::Prepare,
            emitted: 0,
            cause,
        })?;
        Ok(Self {
            mux: Some(mux),
            destination,
            tracks: tracks.to_vec(),
            timing,
            exact_timing,
            stats: Stats::default(),
        })
    }
    /// Facts of the retained container/writer, unchanged by publication.
    /// Progress and partial effects are reported separately through stats/errors.
    pub fn facts(&self) -> DestinationFacts {
        DestinationFacts::new(
            self.destination.can_patch(),
            self.timing.streams(),
            self.timing.patches_at_finish(),
            self.exact_timing,
        )
    }
    fn validate_packet(&self, packet: &Packet) -> Result<(), Cause> {
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
        Ok(())
    }
    fn failure(&self, stage: Stage, cause: Cause) -> Failure {
        Failure {
            stage,
            emitted: self.stats.emitted,
            cause,
        }
    }
    fn drain(&mut self, mux: &mut M, pulse: &dyn zencodec::Pulse) -> Result<(), Cause> {
        let bytes = mux.pending_output();
        self.stats.max_pending = self.stats.max_pending.max(bytes.len());
        write_all(&mut self.destination, bytes, &mut self.stats.emitted, pulse)?;
        mux.consume_output(bytes.len());
        Ok(())
    }
    /// Confirmed progress so far, including before a terminal failure.
    pub const fn stats(&self) -> &Stats {
        &self.stats
    }
    /// Whether a prior failure or unwind made this driver terminal.
    pub fn is_failed(&self) -> bool {
        self.mux.is_none()
    }
    /// Validate and write one packet. Failure is terminal and preserves its cause.
    pub fn send(&mut self, packet: Packet, pulse: &dyn zencodec::Pulse) -> Result<(), Failure> {
        let mut mux = self.mux.take().ok_or_else(|| {
            self.failure(
                Stage::Packet,
                Box::new(SessionError::<ErasedError>::Finished),
            )
        })?;
        let work = || -> Result<(), Cause> {
            stopped(pulse)?;
            self.validate_packet(&packet)?;
            mux.write_packet(packet, pulse)?;
            self.drain(&mut mux, pulse)?;
            Ok(())
        };
        work().map_err(|cause| self.failure(Stage::Packet, cause))?;
        self.mux = Some(mux);
        Ok(())
    }
    /// Preflight final patches, append the tail, apply patches and flush.
    /// A patch I/O failure can leave partial overwrites; it is not a transaction.
    pub fn finish(mut self, pulse: &dyn zencodec::Pulse) -> Result<Stats, Failure> {
        let mux = self.mux.take().ok_or_else(|| {
            self.failure(
                Stage::Finish,
                Box::new(SessionError::<ErasedError>::Finished),
            )
        })?;
        stopped(pulse).map_err(|cause| self.failure(Stage::Finish, cause))?;
        let finish = mux
            .finish(pulse)
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
            pulse,
        )
        .map_err(|cause| self.failure(Stage::Finish, cause))?;
        self.stats.patches = finish.patches.len();
        for patch in finish.patches {
            self.destination
                .position(patch.offset)
                .map_err(|e| self.failure(Stage::Patch, Box::new(e)))?;
            let mut written = 0;
            write_all(&mut self.destination, &patch.bytes, &mut written, pulse)
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
        stopped(pulse).map_err(|cause| self.failure(Stage::Finish, cause))?;
        Ok(self.stats)
    }
}
