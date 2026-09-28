//! Bounded AV1 input queues and incremental packet/IVF output through zenrav1e.

use crate::{
    color::{ChromaLocation, Subsampling, YuvView},
    ivf::IvfWriter,
    plane::Plane,
    time::{TimeBase, Timestamp},
};
use std::{collections::BTreeMap, fmt, io, io::Write, sync::Arc};
use zenpixels::Cicp;
use zenrav1e::{Context, EncoderConfig, EncoderStatus, Pixel, prelude::ChromaSampling};

enum Backend {
    U8(Box<Context<u8>>),
    U16(Box<Context<u16>>),
}

/// An owned temporal unit, associated with its actual submitted presentation.
/// Byte output is ready for a muxer; it is not an AVIF/MP4 file by itself.
#[derive(Debug)]
pub struct EncodedAv1 {
    data: Vec<u8>,
    timestamp: Timestamp,
    input_index: u64,
}
impl EncodedAv1 {
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn timestamp(&self) -> Timestamp {
        self.timestamp
    }
    pub fn input_index(&self) -> u64 {
        self.input_index
    }
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubmitStatus {
    Accepted,
    ReceivePending,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum EncodeReceive {
    Packet(EncodedAv1),
    NeedInput,
    EndOfStream,
}

/// Incremental encoder. Input is copied into backend-owned padded frames;
/// caller plane borrows may end immediately after an accepted submission.
///
/// `max_queued_frames` bounds accepted but not yet emitted presentations. Codec
/// references, reconstructions and lookahead allocations are additional backend
/// storage; this is not a process-byte memory limit. `config.max_pixel_count`
/// must also be nonzero. A bound too small for lookahead returns an explicit
/// error when progress would otherwise stall. No whole-video buffer is used.
pub struct Av1Encoder {
    backend: Backend,
    width: usize,
    height: usize,
    bits: u8,
    sampling: Subsampling,
    location: ChromaLocation,
    color: Cicp,
    clock: TimeBase,
    queued: BTreeMap<u64, Timestamp>,
    limit: usize,
    submitted: u64,
    last_pts: Option<i64>,
    ended: bool,
    drained: bool,
    failed: bool,
}

impl Av1Encoder {
    /// The packet clock is independent of `config.time_base`, which controls
    /// nominal frame rate for encoder decisions. Bitstream equal-picture timing
    /// must be disabled: submitted PTS may be variable-rate. Configured color
    /// and sampling must match every input exactly; no conversion is implicit.
    pub fn new(
        config: EncoderConfig,
        clock: TimeBase,
        max_queued_frames: usize,
        threads: usize,
    ) -> Result<Self, EncodeError> {
        if max_queued_frames == 0 || config.max_pixel_count == 0 {
            return Err(EncodeError::InvalidConfiguration(
                "frame count and pixel limits must be positive",
            ));
        }
        if config.still_picture || config.enable_timing_info {
            return Err(EncodeError::InvalidConfiguration(
                "media encoding requires sequence mode without equal-picture timing",
            ));
        }
        let sampling = match config.chroma_sampling {
            ChromaSampling::Cs400 => Subsampling::Monochrome,
            ChromaSampling::Cs420 => Subsampling::Yuv420,
            ChromaSampling::Cs422 => Subsampling::Yuv422,
            ChromaSampling::Cs444 => Subsampling::Yuv444,
        };
        use zenrav1e::color::ChromaSamplePosition;
        let location = match config.chroma_sample_position {
            ChromaSamplePosition::Unknown => ChromaLocation::Unknown,
            ChromaSamplePosition::Vertical => ChromaLocation::Left,
            ChromaSamplePosition::Colocated => ChromaLocation::TopLeft,
        };
        if sampling != Subsampling::Yuv420 && location != ChromaLocation::Unknown {
            return Err(EncodeError::InvalidConfiguration(
                "AV1 signals chroma position only for 4:2:0",
            ));
        }
        let (cp, tc, mc) = config.color_description.map_or((2, 2, 2), |c| {
            (
                c.color_primaries as u8,
                c.transfer_characteristics as u8,
                c.matrix_coefficients as u8,
            )
        });
        if mc == 0 && sampling != Subsampling::Yuv444 && sampling != Subsampling::Monochrome {
            return Err(EncodeError::InvalidConfiguration(
                "identity matrix requires 4:4:4",
            ));
        }
        let color = Cicp::new(
            cp,
            tc,
            mc,
            config.pixel_range == zenrav1e::color::PixelRange::Full,
        );
        let (width, height) = (config.width, config.height);
        let bits = u8::try_from(config.bit_depth)
            .map_err(|_| EncodeError::InvalidConfiguration("invalid AV1 precision"))?;
        if ![8, 10, 12].contains(&bits) {
            return Err(EncodeError::InvalidConfiguration(
                "AV1 precision must be 8, 10 or 12",
            ));
        }
        let config = zenrav1e::Config::new()
            .with_encoder_config(config)
            .with_threads(threads);
        let backend = if bits == 8 {
            Backend::U8(Box::new(
                config
                    .new_context()
                    .map_err(EncodeError::BackendConfiguration)?,
            ))
        } else {
            Backend::U16(Box::new(
                config
                    .new_context()
                    .map_err(EncodeError::BackendConfiguration)?,
            ))
        };
        Ok(Self {
            backend,
            width,
            height,
            bits,
            sampling,
            location,
            color,
            clock,
            queued: BTreeMap::new(),
            limit: max_queued_frames,
            submitted: 0,
            last_pts: None,
            ended: false,
            drained: false,
            failed: false,
        })
    }

    pub fn queued_frames(&self) -> usize {
        self.queued.len()
    }

    /// Cancellation is checked inside the encoder, including superblock work.
    /// A cancelled encoder is poisoned and must be recreated.
    pub fn set_stop(&mut self, stop: Arc<dyn zenrav1e::Stop>) {
        match &mut self.backend {
            Backend::U8(c) => c.set_stop(stop),
            Backend::U16(c) => c.set_stop(stop),
        }
    }

    pub fn submit(
        &mut self,
        view: YuvView<'_>,
        timestamp: Timestamp,
    ) -> Result<SubmitStatus, EncodeError> {
        if self.failed {
            return Err(EncodeError::FailedStream);
        }
        if self.ended {
            return Err(EncodeError::InputEnded);
        }
        if timestamp.time_base() != self.clock {
            return Err(EncodeError::ClockMismatch);
        }
        if timestamp.ticks() == i64::MIN {
            return Err(EncodeError::ReservedTimestamp);
        }
        if self.last_pts.is_some_and(|last| timestamp.ticks() < last) {
            return Err(EncodeError::NonmonotonicTimestamp);
        }
        if (
            view.width(),
            view.height(),
            view.bit_depth(),
            view.subsampling(),
            view.color(),
        ) != (
            self.width,
            self.height,
            self.bits,
            self.sampling,
            self.color,
        ) {
            return Err(EncodeError::InputFormatMismatch);
        }
        if matches!(self.sampling, Subsampling::Yuv420 | Subsampling::Yuv422)
            && view.chroma_location() != self.location
        {
            return Err(EncodeError::InputFormatMismatch);
        }
        let planes = view.uncropped_planes().ok_or(EncodeError::CroppedInput)?;
        if self.queued.len() == self.limit {
            return Ok(SubmitStatus::ReceivePending);
        }
        let next = self
            .submitted
            .checked_add(1)
            .ok_or(EncodeError::TooManyFrames)?;
        let result = match &mut self.backend {
            Backend::U8(c) => copy_and_send(c, planes),
            Backend::U16(c) => copy_and_send(c, planes),
        };
        if let Err(e) = result {
            self.failed = true;
            return Err(EncodeError::Backend(e));
        }
        self.queued.insert(self.submitted, timestamp);
        self.submitted = next;
        self.last_pts = Some(timestamp.ticks());
        Ok(SubmitStatus::Accepted)
    }

    pub fn end_input(&mut self) -> Result<(), EncodeError> {
        if self.failed {
            return Err(EncodeError::FailedStream);
        }
        if self.ended {
            return Ok(());
        }
        let result = match &mut self.backend {
            Backend::U8(c) => c.send_frame(None),
            Backend::U16(c) => c.send_frame(None),
        };
        if let Err(e) = result {
            self.failed = true;
            return Err(EncodeError::Backend(e));
        }
        self.ended = true;
        Ok(())
    }

    pub fn receive(&mut self) -> Result<EncodeReceive, EncodeError> {
        if self.failed {
            return Err(EncodeError::FailedStream);
        }
        if self.drained {
            return Ok(EncodeReceive::EndOfStream);
        }
        let result = self.receive_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn receive_inner(&mut self) -> Result<EncodeReceive, EncodeError> {
        loop {
            let packet = match &mut self.backend {
                Backend::U8(c) => c.receive_packet().map(|p| (p.data, p.input_frameno)),
                Backend::U16(c) => c.receive_packet().map(|p| (p.data, p.input_frameno)),
            };
            match packet {
                Ok((data, input_index)) => {
                    let timestamp = self
                        .queued
                        .remove(&input_index)
                        .ok_or(EncodeError::UnexpectedOutput)?;
                    return Ok(EncodeReceive::Packet(EncodedAv1 {
                        data,
                        timestamp,
                        input_index,
                    }));
                }
                Err(EncoderStatus::Encoded) => continue,
                Err(EncoderStatus::NeedMoreData) if !self.ended => {
                    if self.queued.len() == self.limit {
                        return Err(EncodeError::QueueLimitInsufficient);
                    }
                    return Ok(EncodeReceive::NeedInput);
                }
                Err(EncoderStatus::LimitReached) if self.ended && self.queued.is_empty() => {
                    self.drained = true;
                    return Ok(EncodeReceive::EndOfStream);
                }
                Err(e) => return Err(EncodeError::Backend(e)),
            }
        }
    }
}

fn copy_and_send<P: Pixel>(
    context: &mut Context<P>,
    planes: [Option<Plane<'_>>; 3],
) -> Result<(), EncoderStatus> {
    let mut frame = context.new_frame();
    for (index, source) in planes.into_iter().enumerate() {
        let Some(source) = source else {
            continue;
        };
        let mut slice = frame.planes[index].mut_slice(Default::default());
        for (y, row) in slice.rows_iter_mut().take(source.height()).enumerate() {
            for (x, out) in row[..source.width()].iter_mut().enumerate() {
                *out = P::cast_from(source.code(x, y).expect("validated visible plane"));
            }
        }
    }
    context.send_frame(frame)
}

/// Blocking streaming encoder/muxer for any `Write`, including a network sink.
/// Backpressure is supplied by the sink's writes. Partial output errors poison
/// the stream; `finish` drains delayed packets and propagates flush errors.
pub struct Av1IvfEncoder<W> {
    encoder: Av1Encoder,
    writer: IvfWriter<W>,
    failed: bool,
}
impl<W: Write> Av1IvfEncoder<W> {
    pub fn new(
        writer: W,
        config: EncoderConfig,
        clock: TimeBase,
        max_queued_frames: usize,
        threads: usize,
    ) -> Result<Self, EncodeError> {
        let width = u16::try_from(config.width)
            .map_err(|_| EncodeError::InvalidConfiguration("IVF width exceeds 16 bits"))?;
        let height = u16::try_from(config.height)
            .map_err(|_| EncodeError::InvalidConfiguration("IVF height exceeds 16 bits"))?;
        let encoder = Av1Encoder::new(config, clock, max_queued_frames, threads)?;
        Ok(Self {
            encoder,
            writer: IvfWriter::new(writer, width, height, clock)?,
            failed: false,
        })
    }

    pub fn set_stop(&mut self, stop: Arc<dyn zenrav1e::Stop>) {
        self.encoder.set_stop(stop);
    }
    pub fn queued_frames(&self) -> usize {
        self.encoder.queued_frames()
    }
    pub fn packets_written(&self) -> u64 {
        self.writer.packets_written()
    }

    /// Submit a presentation, then write every currently available packet.
    /// Once input is accepted, an output error cannot be retried on this stream.
    pub fn push(&mut self, view: YuvView<'_>, timestamp: Timestamp) -> Result<(), EncodeError> {
        if self.failed {
            return Err(EncodeError::FailedStream);
        }
        // Pre-submission validation does not consume input or alter the sink.
        // Backend errors poison the inner encoder at their source.
        match self.encoder.submit(view, timestamp)? {
            SubmitStatus::Accepted => {}
            SubmitStatus::ReceivePending => return Err(EncodeError::QueueLimitInsufficient),
        }
        let result = self.write_available(false);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn write_available(&mut self, finishing: bool) -> Result<(), EncodeError> {
        loop {
            match self.encoder.receive()? {
                EncodeReceive::Packet(p) => self.writer.write_packet(p.data(), p.timestamp())?,
                EncodeReceive::NeedInput if !finishing => return Ok(()),
                EncodeReceive::EndOfStream if finishing => return Ok(()),
                _ => return Err(EncodeError::UnexpectedOutput),
            }
        }
    }

    pub fn finish(mut self) -> Result<W, EncodeError> {
        if self.failed {
            return Err(EncodeError::FailedStream);
        }
        self.encoder.end_input()?;
        self.write_available(true)?;
        Ok(self.writer.finish()?)
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum EncodeError {
    InvalidConfiguration(&'static str),
    BackendConfiguration(zenrav1e::InvalidConfig),
    Backend(EncoderStatus),
    Io(io::Error),
    FailedStream,
    InputEnded,
    ClockMismatch,
    ReservedTimestamp,
    NonmonotonicTimestamp,
    InputFormatMismatch,
    CroppedInput,
    TooManyFrames,
    QueueLimitInsufficient,
    UnexpectedOutput,
}
impl From<io::Error> for EncodeError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(s) => f.write_str(s),
            Self::BackendConfiguration(e) => e.fmt(f),
            Self::Backend(e) => e.fmt(f),
            Self::Io(e) => e.fmt(f),
            Self::FailedStream => f.write_str("encoder cannot resume after a stream failure"),
            Self::InputEnded => f.write_str("encoder input already ended"),
            Self::ClockMismatch => {
                f.write_str("input timestamp clock differs from the packet clock")
            }
            Self::ReservedTimestamp => {
                f.write_str("i64::MIN is reserved by the paired AV1 decoder")
            }
            Self::NonmonotonicTimestamp => f.write_str("input presentation time moved backwards"),
            Self::InputFormatMismatch => {
                f.write_str("native input format/color differs from encoder configuration")
            }
            Self::CroppedInput => {
                f.write_str("encoder needs materialized native planes for a cropped view")
            }
            Self::TooManyFrames => f.write_str("encoder input counter overflow"),
            Self::QueueLimitInsufficient => {
                f.write_str("queued frame limit cannot satisfy encoder lookahead")
            }
            Self::UnexpectedOutput => {
                f.write_str("encoder output does not match submitted presentations")
            }
        }
    }
}
impl std::error::Error for EncodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BackendConfiguration(e) => Some(e),
            Self::Backend(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
