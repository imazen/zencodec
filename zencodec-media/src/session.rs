//! Codec sessions: packets in, frames out ([`VideoDecoder`]); frames in,
//! packets out ([`VideoEncoder`]).
//!
//! The protocol, for both directions:
//!
//! - `send` offers one input. [`SendStatus::Accepted`] means the session
//!   took it (a packet's payload is taken; a frame handle is cloned).
//!   [`SendStatus::ReceivePending`] means "receive first": the input is left
//!   untouched for the retry, and at least one `receive` now returns
//!   [`Receive::Ready`]. That is the backpressure: a session never queues
//!   without bound and never blocks.
//! - `receive` returns one output, [`Receive::NeedInput`], or, after
//!   `end_input` and once everything is drained, [`Receive::EndOfStream`],
//!   which is stable.
//! - `end_input` declares the end; it is idempotent. Sending after it is a
//!   [`ProtocolViolation::SendAfterEnd`].
//! - A decoder's `reset` discards queued input, output and references and
//!   keeps the configuration; frames already returned stay valid. Seeking is
//!   `reset` and `configure` (to re-inject a sequence header), then packets
//!   from the new position.
//!
//! Sessions do no I/O, need no async runtime and know nothing about
//! containers. Every call takes a `&dyn Stop`; [`FusedDecoder`] makes errors
//! and cancellation terminal until a successful `reset`.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::fmt;

use enough::{Stop, StopReason};
pub use zencodec::decode::ErasedError;

use crate::frame::{Frame, FrameInterpretation};
use crate::limits::MediaLimitExceeded;
use crate::packet::Packet;
use crate::track::{CodecId, TrackInfo, VideoTrackInfo};
use zencodec::time::TimeBase;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// What `send` did with an input.
pub enum SendStatus {
    /// Ownership moved (the packet is taken, or the frame handle was cloned).
    Accepted,
    /// Receive output first, then retry the unchanged input. At least one
    /// `receive` after this returns `Ready`.
    ReceivePending,
}

#[derive(Debug)]
#[non_exhaustive]
/// What `receive` returned.
pub enum Receive<T> {
    /// One output.
    Ready(T),
    /// Nothing now; more input may be sent.
    NeedInput,
    /// Input ended and everything was drained. Stable.
    EndOfStream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// A caller or a session broke the protocol.
pub enum ProtocolViolation {
    /// `send` after `end_input`.
    SendAfterEnd,
    /// A packet without payload.
    EmptyPacket,
    /// `send` before `configure`.
    NotConfigured,
    /// A packet from another configuration epoch than the configured one.
    EpochMismatch,
    /// An input without the timestamp the session needs.
    MissingTimestamp,
    /// A timestamp on another clock than the session's.
    ClockMismatch,
    /// A second `begin`.
    BeginTwice,
    /// Output requested before `begin`.
    NoBegin,
    /// A session answered `NeedInput` where it had promised output or an
    /// end (after `end_input`).
    NoProgress,
    /// `ReceivePending`, then no output from `receive`.
    ReceivePendingWithoutOutput,
    /// A second `finish`.
    FinishTwice,
    /// The operation needs an index the container did not provide.
    IndexRequired,
    /// `EndOfStream` before `end_input`.
    EndOfStreamBeforeEndInput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// A valid request this implementation does not support.
pub enum UnsupportedMedia {
    /// Repositioning.
    Seek,
    /// A mid-stream configuration change.
    ConfigChange,
    /// A layout that needs no patches (forward-only destination).
    ForwardOnlyLayout,
    /// This codec.
    Codec,
    /// This container feature.
    Container,
    /// This sample layout or depth.
    PixelFormat,
}

/// Every session error is terminal for the session except `Codec` errors a
/// backend documents as retryable validation.
#[derive(Debug)]
#[non_exhaustive]
pub enum SessionError<E> {
    /// The codec or container implementation failed (malformed input,
    /// backend error).
    Codec(E),
    /// The stop tripped.
    Cancelled(StopReason),
    /// Called again after a terminal error.
    Finished,
    /// The protocol was broken.
    Protocol(ProtocolViolation),
    /// A valid request this implementation does not support.
    Unsupported(UnsupportedMedia),
    /// A structural limit was exceeded.
    Limit(MediaLimitExceeded),
}

impl<E> SessionError<E> {
    /// Convert the implementation error, keeping every other variant.
    pub fn map_codec<F>(self, f: impl FnOnce(E) -> F) -> SessionError<F> {
        match self {
            Self::Codec(e) => SessionError::Codec(f(e)),
            Self::Cancelled(r) => SessionError::Cancelled(r),
            Self::Finished => SessionError::Finished,
            Self::Protocol(p) => SessionError::Protocol(p),
            Self::Unsupported(u) => SessionError::Unsupported(u),
            Self::Limit(l) => SessionError::Limit(l),
        }
    }
}

impl<E> From<StopReason> for SessionError<E> {
    fn from(r: StopReason) -> Self {
        Self::Cancelled(r)
    }
}

impl<E: fmt::Display> fmt::Display for SessionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "{e}"),
            Self::Cancelled(r) => write!(f, "cancelled: {r:?}"),
            Self::Finished => f.write_str("session used after a terminal error"),
            Self::Protocol(p) => write!(f, "protocol violation: {p:?}"),
            Self::Unsupported(u) => write!(f, "unsupported: {u:?}"),
            Self::Limit(l) => write!(f, "{l}"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for SessionError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            Self::Limit(l) => Some(l),
            _ => None,
        }
    }
}

/// What a decoder needs before the first packet of a configuration epoch:
/// derived from a `TrackInfo`, or from a muxer's expectations.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CodecConfig {
    /// The codec.
    pub codec: CodecId,
    /// Codec configuration bytes (`av1C`, a sequence header, `avcC`).
    pub private: Option<Arc<[u8]>>,
    /// The epoch these bytes start.
    pub epoch: u32,
    /// The clock of the timestamps that follow.
    pub time_base: TimeBase,
    /// Declared video shape, when known.
    pub video: Option<VideoTrackInfo>,
}

impl CodecConfig {
    /// A configuration for `codec` on `time_base`, epoch 0.
    pub fn new(codec: CodecId, time_base: TimeBase) -> Self {
        Self {
            codec,
            private: None,
            epoch: 0,
            time_base,
            video: None,
        }
    }
    /// The declared video shape.
    pub fn with_video(mut self, video: VideoTrackInfo) -> Self {
        self.video = Some(video);
        self
    }

    /// Codec configuration bytes.
    pub fn with_private(mut self, private: Arc<[u8]>) -> Self {
        self.private = Some(private);
        self
    }

    /// The epoch.
    pub fn with_epoch(mut self, epoch: u32) -> Self {
        self.epoch = epoch;
        self
    }
}

impl From<&TrackInfo> for CodecConfig {
    fn from(t: &TrackInfo) -> Self {
        Self {
            codec: t.codec().clone(),
            private: t.codec_private().cloned(),
            epoch: t.config_epoch(),
            time_base: t.time_base(),
            video: t.video_info().copied(),
        }
    }
}

/// Per-stream facts, known once the stream has been seen; not static flags.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct VideoStreamInfo {
    /// The interpretation of the latest frame.
    pub interpretation: Arc<FrameInterpretation>,
    /// Packets the decoder may hold before their frame comes out, when the
    /// stream declares it.
    pub reorder_depth: Option<u32>,
}

impl VideoStreamInfo {
    /// Facts with an interpretation and no declared reorder depth.
    pub fn new(interpretation: Arc<FrameInterpretation>) -> Self {
        Self {
            interpretation,
            reorder_depth: None,
        }
    }
}

/// A video decoder session: packets in, frames out, in presentation order.
/// See the [module docs](self) for the protocol.
pub trait VideoDecoder {
    /// The implementation's error.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Start a configuration epoch. Called before the first `send` and on
    /// every epoch change (new sequence header, new `stsd` entry).
    fn configure(
        &mut self,
        config: &CodecConfig,
        stop: &dyn Stop,
    ) -> Result<(), SessionError<Self::Error>>;

    /// Offer one packet. `Accepted` takes its payload; `ReceivePending`
    /// leaves it intact for retry after `receive`.
    fn send(
        &mut self,
        packet: &mut Packet,
        stop: &dyn Stop,
    ) -> Result<SendStatus, SessionError<Self::Error>>;

    /// One frame at a time, in presentation order.
    fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Frame>, SessionError<Self::Error>>;

    /// Declare end of input; idempotent. Then `receive` until `EndOfStream`.
    fn end_input(&mut self) -> Result<(), SessionError<Self::Error>>;

    /// Discard queued input, output and references; keep configuration.
    /// Frames already returned stay valid.
    fn reset(&mut self) -> Result<(), SessionError<Self::Error>>;

    /// Facts about the stream seen so far.
    fn stream_info(&self) -> Option<&VideoStreamInfo>;
}

/// A video encoder session: frames in, packets out, in decode order. See
/// the [module docs](self) for the protocol.
pub trait VideoEncoder {
    /// The implementation's error.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Offer a frame handle. The encoder clones it to queue (no pixel copy
    /// is required by the contract; a backend may copy into padded planes).
    fn send(
        &mut self,
        frame: &Frame,
        stop: &dyn Stop,
    ) -> Result<SendStatus, SessionError<Self::Error>>;

    /// One packet at a time, in decode order, carrying the presentation
    /// timestamp of the frame it shows.
    fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Packet>, SessionError<Self::Error>>;

    /// Declare end of input; idempotent. Then `receive` until `EndOfStream`.
    fn end_input(&mut self) -> Result<(), SessionError<Self::Error>>;

    /// Codec configuration a muxer needs before the first packet.
    fn output_config(&self) -> Option<&CodecConfig>;
}

/// A type-erased, sendable decoder session.
pub type DynVideoDecoder = dyn VideoDecoder<Error = ErasedError> + Send;
/// A type-erased, sendable encoder session.
pub type DynVideoEncoder = dyn VideoEncoder<Error = ErasedError> + Send;

fn erase<E: core::error::Error + Send + Sync + 'static>(
    e: SessionError<E>,
) -> SessionError<ErasedError> {
    e.map_codec(|c| ErasedError::new(Box::new(c)))
}

/// Erases a decoder's error type.
pub struct VideoDecoderShim<D>(pub D);

impl<D: VideoDecoder> VideoDecoder for VideoDecoderShim<D> {
    type Error = ErasedError;
    fn configure(
        &mut self,
        config: &CodecConfig,
        stop: &dyn Stop,
    ) -> Result<(), SessionError<ErasedError>> {
        self.0.configure(config, stop).map_err(erase)
    }
    fn send(
        &mut self,
        packet: &mut Packet,
        stop: &dyn Stop,
    ) -> Result<SendStatus, SessionError<ErasedError>> {
        self.0.send(packet, stop).map_err(erase)
    }
    fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Frame>, SessionError<ErasedError>> {
        self.0.receive(stop).map_err(erase)
    }
    fn end_input(&mut self) -> Result<(), SessionError<ErasedError>> {
        self.0.end_input().map_err(erase)
    }
    fn reset(&mut self) -> Result<(), SessionError<ErasedError>> {
        self.0.reset().map_err(erase)
    }
    fn stream_info(&self) -> Option<&VideoStreamInfo> {
        self.0.stream_info()
    }
}

/// Erases an encoder's error type.
pub struct VideoEncoderShim<E>(pub E);

impl<E: VideoEncoder> VideoEncoder for VideoEncoderShim<E> {
    type Error = ErasedError;
    fn send(
        &mut self,
        frame: &Frame,
        stop: &dyn Stop,
    ) -> Result<SendStatus, SessionError<ErasedError>> {
        self.0.send(frame, stop).map_err(erase)
    }
    fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Packet>, SessionError<ErasedError>> {
        self.0.receive(stop).map_err(erase)
    }
    fn end_input(&mut self) -> Result<(), SessionError<ErasedError>> {
        self.0.end_input().map_err(erase)
    }
    fn output_config(&self) -> Option<&CodecConfig> {
        self.0.output_config()
    }
}

/// Makes errors and cancellation terminal for any decoder, as `Fused` does
/// for `IncrementalDecode`. A successful `reset` reopens the session.
pub struct FusedDecoder<D> {
    inner: D,
    failed: bool,
}

impl<D> FusedDecoder<D> {
    /// Wrap `inner`.
    pub const fn new(inner: D) -> Self {
        Self {
            inner,
            failed: false,
        }
    }
    /// The wrapped decoder.
    pub fn into_inner(self) -> D {
        self.inner
    }
}

impl<D: VideoDecoder> FusedDecoder<D> {
    fn guard<T>(
        &mut self,
        stop: &dyn Stop,
        f: impl FnOnce(&mut D) -> Result<T, SessionError<D::Error>>,
    ) -> Result<T, SessionError<D::Error>> {
        if self.failed {
            return Err(SessionError::Finished);
        }
        if let Err(r) = stop.check() {
            self.failed = true;
            return Err(SessionError::Cancelled(r));
        }
        let r = f(&mut self.inner);
        if r.is_err() {
            self.failed = true;
        }
        r
    }
}

impl<D: VideoDecoder> VideoDecoder for FusedDecoder<D> {
    type Error = D::Error;
    fn configure(
        &mut self,
        config: &CodecConfig,
        stop: &dyn Stop,
    ) -> Result<(), SessionError<D::Error>> {
        self.guard(stop, |d| d.configure(config, stop))
    }
    fn send(
        &mut self,
        packet: &mut Packet,
        stop: &dyn Stop,
    ) -> Result<SendStatus, SessionError<D::Error>> {
        self.guard(stop, |d| d.send(packet, stop))
    }
    fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Frame>, SessionError<D::Error>> {
        self.guard(stop, |d| d.receive(stop))
    }
    fn end_input(&mut self) -> Result<(), SessionError<D::Error>> {
        if self.failed {
            return Err(SessionError::Finished);
        }
        let r = self.inner.end_input();
        if r.is_err() {
            self.failed = true;
        }
        r
    }
    fn reset(&mut self) -> Result<(), SessionError<D::Error>> {
        let r = self.inner.reset();
        self.failed = r.is_err();
        r
    }
    fn stream_info(&self) -> Option<&VideoStreamInfo> {
        self.inner.stream_info()
    }
}

/// Erase a decoder: errors become [`ErasedError`], and errors and
/// cancellation become terminal until `reset` ([`FusedDecoder`]).
pub fn erase_video_decoder<D: VideoDecoder + Send + 'static>(d: D) -> Box<DynVideoDecoder> {
    Box::new(VideoDecoderShim(FusedDecoder::new(d)))
}

/// Erase an encoder's error type.
pub fn erase_video_encoder<E: VideoEncoder + Send + 'static>(e: E) -> Box<DynVideoEncoder> {
    Box::new(VideoEncoderShim(e))
}

impl<D: VideoDecoder + ?Sized> VideoDecoder for Box<D> {
    type Error = D::Error;
    fn configure(
        &mut self,
        config: &CodecConfig,
        stop: &dyn Stop,
    ) -> Result<(), SessionError<D::Error>> {
        (**self).configure(config, stop)
    }
    fn send(
        &mut self,
        packet: &mut Packet,
        stop: &dyn Stop,
    ) -> Result<SendStatus, SessionError<D::Error>> {
        (**self).send(packet, stop)
    }
    fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Frame>, SessionError<D::Error>> {
        (**self).receive(stop)
    }
    fn end_input(&mut self) -> Result<(), SessionError<D::Error>> {
        (**self).end_input()
    }
    fn reset(&mut self) -> Result<(), SessionError<D::Error>> {
        (**self).reset()
    }
    fn stream_info(&self) -> Option<&VideoStreamInfo> {
        (**self).stream_info()
    }
}

impl<E: VideoEncoder + ?Sized> VideoEncoder for Box<E> {
    type Error = E::Error;
    fn send(
        &mut self,
        frame: &Frame,
        stop: &dyn Stop,
    ) -> Result<SendStatus, SessionError<E::Error>> {
        (**self).send(frame, stop)
    }
    fn receive(&mut self, stop: &dyn Stop) -> Result<Receive<Packet>, SessionError<E::Error>> {
        (**self).receive(stop)
    }
    fn end_input(&mut self) -> Result<(), SessionError<E::Error>> {
        (**self).end_input()
    }
    fn output_config(&self) -> Option<&CodecConfig> {
        (**self).output_config()
    }
}

/// The backpressure loop every driver writes once: send, and while the codec
/// says `ReceivePending`, receive output into `on_output` and retry; then
/// deliver whatever output is ready.
///
/// # Errors
///
/// The decoder's error as `Upstream`, `on_output`'s as `Downstream`.
pub fn send_packet_until_accepted<D, F, E2>(
    dec: &mut D,
    packet: &mut Packet,
    stop: &dyn Stop,
    mut on_output: F,
) -> Result<(), PumpError<SessionError<D::Error>, E2>>
where
    D: VideoDecoder + ?Sized,
    F: FnMut(Frame) -> Result<(), E2>,
{
    loop {
        match dec.send(packet, stop).map_err(PumpError::Upstream)? {
            SendStatus::Accepted => break,
            SendStatus::ReceivePending => match dec.receive(stop).map_err(PumpError::Upstream)? {
                Receive::Ready(f) => on_output(f).map_err(PumpError::Downstream)?,
                _ => {
                    return Err(PumpError::Upstream(SessionError::Protocol(
                        ProtocolViolation::ReceivePendingWithoutOutput,
                    )));
                }
            },
        }
    }
    // Accepted: deliver whatever output is available now, so frames never
    // sit in the codec until the next backpressure event.
    loop {
        match dec.receive(stop).map_err(PumpError::Upstream)? {
            Receive::Ready(f) => on_output(f).map_err(PumpError::Downstream)?,
            Receive::NeedInput => return Ok(()),
            Receive::EndOfStream => {
                return Err(PumpError::Upstream(SessionError::Protocol(
                    ProtocolViolation::EndOfStreamBeforeEndInput,
                )));
            }
        }
    }
}

/// The encoder twin of [`send_packet_until_accepted`].
///
/// # Errors
///
/// The encoder's error as `Upstream`, `on_output`'s as `Downstream`.
pub fn send_frame_until_accepted<E, F, E2>(
    enc: &mut E,
    frame: &Frame,
    stop: &dyn Stop,
    mut on_output: F,
) -> Result<(), PumpError<SessionError<E::Error>, E2>>
where
    E: VideoEncoder + ?Sized,
    F: FnMut(Packet) -> Result<(), E2>,
{
    loop {
        match enc.send(frame, stop).map_err(PumpError::Upstream)? {
            SendStatus::Accepted => break,
            SendStatus::ReceivePending => match enc.receive(stop).map_err(PumpError::Upstream)? {
                Receive::Ready(p) => on_output(p).map_err(PumpError::Downstream)?,
                _ => {
                    return Err(PumpError::Upstream(SessionError::Protocol(
                        ProtocolViolation::ReceivePendingWithoutOutput,
                    )));
                }
            },
        }
    }
    loop {
        match enc.receive(stop).map_err(PumpError::Upstream)? {
            Receive::Ready(p) => on_output(p).map_err(PumpError::Downstream)?,
            Receive::NeedInput => return Ok(()),
            Receive::EndOfStream => {
                return Err(PumpError::Upstream(SessionError::Protocol(
                    ProtocolViolation::EndOfStreamBeforeEndInput,
                )));
            }
        }
    }
}

/// End input and drain: every `Ready` goes to `on_output` until
/// `EndOfStream`.
///
/// # Errors
///
/// The decoder's error as `Upstream`, `on_output`'s as `Downstream`.
pub fn drain_decoder<D, F, E2>(
    dec: &mut D,
    stop: &dyn Stop,
    mut on_output: F,
) -> Result<(), PumpError<SessionError<D::Error>, E2>>
where
    D: VideoDecoder + ?Sized,
    F: FnMut(Frame) -> Result<(), E2>,
{
    dec.end_input().map_err(PumpError::Upstream)?;
    loop {
        match dec.receive(stop).map_err(PumpError::Upstream)? {
            Receive::Ready(f) => on_output(f).map_err(PumpError::Downstream)?,
            Receive::EndOfStream => return Ok(()),
            Receive::NeedInput => {
                return Err(PumpError::Upstream(SessionError::Protocol(
                    ProtocolViolation::NoProgress,
                )));
            }
        }
    }
}

/// The encoder twin of [`drain_decoder`].
///
/// # Errors
///
/// The encoder's error as `Upstream`, `on_output`'s as `Downstream`.
pub fn drain_encoder<E, F, E2>(
    enc: &mut E,
    stop: &dyn Stop,
    mut on_output: F,
) -> Result<(), PumpError<SessionError<E::Error>, E2>>
where
    E: VideoEncoder + ?Sized,
    F: FnMut(Packet) -> Result<(), E2>,
{
    enc.end_input().map_err(PumpError::Upstream)?;
    loop {
        match enc.receive(stop).map_err(PumpError::Upstream)? {
            Receive::Ready(p) => on_output(p).map_err(PumpError::Downstream)?,
            Receive::EndOfStream => return Ok(()),
            Receive::NeedInput => {
                return Err(PumpError::Upstream(SessionError::Protocol(
                    ProtocolViolation::NoProgress,
                )));
            }
        }
    }
}

/// Where a pump failed: the stage being driven, or the stage it feeds.
#[derive(Debug)]
pub enum PumpError<U, D> {
    /// The stage being driven failed.
    Upstream(U),
    /// The stage it feeds failed.
    Downstream(D),
}

impl<U: fmt::Display, D: fmt::Display> fmt::Display for PumpError<U, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Upstream(e) => write!(f, "upstream: {e}"),
            Self::Downstream(e) => write!(f, "downstream: {e}"),
        }
    }
}
impl<U: core::error::Error + 'static, D: core::error::Error + 'static> core::error::Error
    for PumpError<U, D>
{
}
