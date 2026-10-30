//! Animation as a layer over the video model, both ways, through the real
//! zencodec animation traits.
//!
//! - [`VideoAnimationDecoder`] implements
//!   [`AnimationFrameDecoder`] over a [`Demuxer`] and a [`VideoDecoder`]. A
//!   frame lasts until the next one's presentation timestamp; the last one
//!   lasts the explicit `final_duration` (IVF, WebM and MP4 do not reliably
//!   record it). Frames carry their timestamp in [`AnimationFrame::pts`].
//! - [`AnimationVideoEncoder`] implements [`AnimationFrameEncoder`] over a
//!   [`VideoEncoder`] and a [`Muxer`]. Durations become timestamps on the
//!   encoder's clock, exactly or not at all
//!   ([`UnsupportedOperation::AnimationTiming`]). Its
//!   [`begin`](AnimationFrameEncoder::begin) and `pending_output` map onto
//!   the muxer's pull output, so animation output streams when the muxer's
//!   does.
//!
//! Turning planes into a packed canvas and back is a colour conversion, and
//! colour conversion belongs to zenpixels-convert, not to a codec contract.
//! The adapters take it as a [`RenderFrame`] / [`CaptureFrame`]; [`Gray8`]
//! is the identity for 8-bit monochrome.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use zenpixels::sample::SampleEncoding;

use zencodec::decode::{
    AnimationFrame, AnimationFrameDecoder, DecodeRowSink, OutputInfo, SinkError,
};
use zencodec::encode::{AnimationFrameEncoder, AnimationOutputTiming, BoxedError, EncodeOutput};
use zencodec::time::{FrameDuration, Timestamp, TimingError, duration_to_ticks};
use zencodec::{ImageFormat, ImageInfo, ImageSequence, UnsupportedOperation};
use zencodec_io::{DriveFailure, SliceCursor};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice, PixelSliceMut};

use crate::container::{DemuxEvent, DemuxStage, Demuxer, MuxHint, Muxer};
use crate::frame::{
    ChromaSiting, Frame, FrameGeometry, FrameInterpretation, FrameTiming, MapError, OwnedPlane,
    OwnedPlanes, PlaneData, PlaneRole, PlaneSamples, Presentation, Subsampling,
};
use crate::session::{
    CodecConfig, ProtocolViolation, PumpError, SessionError, VideoDecoder, VideoEncoder,
    drain_decoder, drain_encoder, send_frame_until_accepted, send_packet_until_accepted,
};
use crate::track::{TrackInfo, TrackKind};

/// Writes a decoded frame onto a packed canvas: the step between a video
/// decoder and the animation API.
pub trait RenderFrame {
    /// The canvas descriptor for frames of `track`.
    ///
    /// # Errors
    ///
    /// When this renderer cannot produce a canvas for the track.
    fn canvas_descriptor(&self, track: &TrackInfo) -> Result<PixelDescriptor, BoxedError>;

    /// Write `frame`'s pixels into `canvas`, which has the track's coded size
    /// and [`canvas_descriptor`](Self::canvas_descriptor).
    ///
    /// # Errors
    ///
    /// When the frame cannot be converted.
    fn render(&mut self, frame: &Frame, canvas: &mut PixelSliceMut<'_>) -> Result<(), BoxedError>;
}

/// Makes a frame from packed pixels: the step between the animation API and
/// a video encoder.
pub trait CaptureFrame {
    /// A frame holding `pixels` with `timing`, in the layout the encoder
    /// configured by `config` takes.
    ///
    /// # Errors
    ///
    /// When the pixels cannot be converted.
    fn capture(
        &mut self,
        pixels: &PixelSlice<'_>,
        config: &CodecConfig,
        timing: FrameTiming,
    ) -> Result<Frame, BoxedError>;
}

/// The identity conversion for 8-bit monochrome: one luma plane to
/// `GRAY8_SRGB` and back.
#[derive(Clone, Debug, Default)]
pub struct Gray8 {
    interpretation: Option<Arc<FrameInterpretation>>,
}

#[derive(Debug)]
struct ConvertError(&'static str);

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl core::error::Error for ConvertError {}

fn convert_error(msg: &'static str) -> BoxedError {
    Box::new(ConvertError(msg))
}

impl RenderFrame for Gray8 {
    fn canvas_descriptor(&self, track: &TrackInfo) -> Result<PixelDescriptor, BoxedError> {
        if track.video_info().is_none() {
            return Err(convert_error("Gray8 renders video tracks only"));
        }
        Ok(PixelDescriptor::GRAY8_SRGB)
    }

    fn render(&mut self, frame: &Frame, canvas: &mut PixelSliceMut<'_>) -> Result<(), BoxedError> {
        let interp = frame.interpretation();
        if interp.subsampling != Subsampling::Mono
            || interp.encoding != SampleEncoding::new(zenpixels::ChannelType::U8, 8, 0).unwrap()
        {
            return Err(convert_error("Gray8 renders 8-bit monochrome frames only"));
        }
        let mapped = frame.map()?;
        let plane = mapped
            .plane(0)
            .ok_or_else(|| convert_error("frame has no plane"))?;
        if plane.width() != canvas.width() as usize || plane.height() != canvas.rows() as usize {
            return Err(convert_error("frame size differs from the canvas"));
        }
        for y in 0..plane.height() {
            let PlaneSamples::U8(row) = plane.row(y).expect("in range") else {
                return Err(convert_error("8-bit frame with 16-bit storage"));
            };
            canvas.row_mut(y as u32)[..row.len()].copy_from_slice(row);
        }
        Ok(())
    }
}

impl CaptureFrame for Gray8 {
    fn capture(
        &mut self,
        pixels: &PixelSlice<'_>,
        config: &CodecConfig,
        timing: FrameTiming,
    ) -> Result<Frame, BoxedError> {
        if pixels.descriptor() != PixelDescriptor::GRAY8_SRGB {
            return Err(Box::new(UnsupportedOperation::PixelFormat));
        }
        let (w, h) = (pixels.width(), pixels.rows());
        if config
            .video
            .is_some_and(|v| (v.coded_width, v.coded_height) != (w, h))
        {
            return Err(convert_error("frame size differs from the encoder's"));
        }
        let interpretation = match &self.interpretation {
            Some(i) if i.geometry == FrameGeometry::full(w, h) => Arc::clone(i),
            _ => {
                let i = Arc::new(FrameInterpretation::new(
                    FrameGeometry::full(w, h),
                    Subsampling::Mono,
                    ChromaSiting::Unknown,
                    SampleEncoding::new(zenpixels::ChannelType::U8, 8, 0).unwrap(),
                    vec![PlaneRole::Luma],
                ));
                self.interpretation = Some(Arc::clone(&i));
                i
            }
        };
        let (wu, hu) = (w as usize, h as usize);
        let mut data = Vec::new();
        data.try_reserve_exact(wu * hu)
            .map_err(|_| convert_error("frame does not fit in memory"))?;
        for y in 0..h {
            data.extend_from_slice(&pixels.row(y)[..wu]);
        }
        let planes = OwnedPlanes::new(
            vec![OwnedPlane {
                data: PlaneData::U8(data),
                width: wu,
                height: hu,
                stride_bytes: wu,
            }],
            SampleEncoding::new(zenpixels::ChannelType::U8, 8, 0).unwrap(),
        )?;
        Ok(Frame::new(
            Arc::new(planes),
            interpretation,
            timing,
            Presentation::default(),
        ))
    }
}

/// Failure of an adapter. `A` is the first stage's error (demuxer or
/// encoder), `B` the second's (decoder or muxer).
#[derive(Debug)]
#[non_exhaustive]
pub enum AdaptError<A, B> {
    /// Driving the demuxer failed.
    Demux(DriveFailure<SessionError<A>, core::convert::Infallible>),
    /// The decoder failed.
    Decode(SessionError<B>),
    /// The encoder failed.
    Encode(SessionError<A>),
    /// The muxer failed.
    Mux(SessionError<B>),
    /// A duration or timestamp did not convert.
    Time(TimingError),
    /// A frame could not be mapped.
    Map(MapError),
    /// The render or capture conversion failed.
    Convert(BoxedError),
    /// The row sink failed.
    Sink(SinkError),
    /// The animation API asked for something unsupported.
    Unsupported(UnsupportedOperation),
    /// A video frame without a presentation timestamp: its duration cannot
    /// be derived.
    MissingTimestamp,
    /// No video track, or a frame or canvas of the wrong shape.
    Shape,
    /// Finished after a different number of frames than `begin` declared.
    FrameCountMismatch,
    /// The muxer produced patches for a forward-only destination.
    PatchesNotAllowed,
}

impl<A: fmt::Display, B: fmt::Display> fmt::Display for AdaptError<A, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Demux(e) => write!(f, "demux: {e}"),
            Self::Decode(e) => write!(f, "decode: {e}"),
            Self::Encode(e) => write!(f, "encode: {e}"),
            Self::Mux(e) => write!(f, "mux: {e}"),
            Self::Time(e) => write!(f, "time: {e}"),
            Self::Map(e) => write!(f, "map: {e}"),
            Self::Convert(e) => write!(f, "convert: {e}"),
            Self::Sink(e) => write!(f, "sink: {e}"),
            Self::Unsupported(op) => write!(f, "unsupported: {op}"),
            Self::MissingTimestamp => f.write_str("video frame has no presentation timestamp"),
            Self::Shape => {
                f.write_str("no video track, or frame geometry differs from the animation canvas")
            }
            Self::FrameCountMismatch => {
                f.write_str("finished after a different number of frames than declared")
            }
            Self::PatchesNotAllowed => {
                f.write_str("muxer produced patches for a forward-only destination")
            }
        }
    }
}

impl<A, B> core::error::Error for AdaptError<A, B>
where
    A: core::error::Error + 'static,
    B: core::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Demux(e) => Some(e),
            Self::Decode(e) | Self::Mux(e) => Some(e),
            Self::Encode(e) => Some(e),
            Self::Time(e) => Some(e),
            Self::Map(e) => Some(e),
            Self::Convert(e) => Some(&**e),
            Self::Sink(e) => Some(&**e),
            Self::Unsupported(e) => Some(e),
            _ => None,
        }
    }
}

type DecErr<D, C> = AdaptError<<D as Demuxer>::Error, <C as VideoDecoder>::Error>;

/// A composited-canvas animation view over a demuxed, decoded video: the
/// first video track, in presentation order.
pub struct VideoAnimationDecoder<D, C, R> {
    bytes: Arc<[u8]>,
    cursor: SliceCursor,
    dem: D,
    dec: C,
    render: R,
    track: Option<u32>,
    pending: VecDeque<Frame>,
    eos: bool,
    final_duration: FrameDuration,
    canvas: PixelBuffer,
    info: ImageInfo,
    index: u32,
    frames_decoded: u64,
}

impl<D: Demuxer, C: VideoDecoder, R: RenderFrame> VideoAnimationDecoder<D, C, R> {
    /// Decode `bytes` with `dem` and `dec`, rendering through `render`.
    /// `final_duration` is how long the last frame lasts: a video stream
    /// has none for its last presentation unless the container records
    /// one. `window` is the slice driver's read size (bytes per step).
    pub fn new(
        bytes: Arc<[u8]>,
        dem: D,
        dec: C,
        render: R,
        final_duration: FrameDuration,
        window: usize,
    ) -> Self {
        Self {
            bytes,
            cursor: SliceCursor::new(window),
            dem,
            dec,
            render,
            track: None,
            pending: VecDeque::new(),
            eos: false,
            final_duration,
            canvas: PixelBuffer::new(1, 1, PixelDescriptor::GRAY8_SRGB),
            info: ImageInfo::new(1, 1, ImageFormat::Unknown),
            index: 0,
            frames_decoded: 0,
        }
    }

    /// Frames the video decoder produced so far (some may still be queued
    /// for duration lookahead).
    pub fn frames_decoded(&self) -> u64 {
        self.frames_decoded
    }

    /// The demuxer.
    pub fn demuxer(&self) -> &D {
        &self.dem
    }

    fn on_headers(&mut self, pulse: Option<&dyn zencodec::Pulse>) -> Result<(), DecErr<D, C>> {
        let tracks = self.dem.tracks();
        let track = tracks
            .iter()
            .find(|t| t.kind() == TrackKind::Video)
            .ok_or(AdaptError::Shape)?;
        let video = track.video_info().ok_or(AdaptError::Shape)?;
        let desc = self
            .render
            .canvas_descriptor(track)
            .map_err(AdaptError::Convert)?;
        self.canvas = PixelBuffer::try_new(video.coded_width, video.coded_height, desc)
            .map_err(|_| AdaptError::Shape)?;
        self.info = ImageInfo::new(video.coded_width, video.coded_height, ImageFormat::Unknown)
            .with_sequence(ImageSequence::Animation {
                frame_count: track.declared_packets().and_then(|n| u32::try_from(n).ok()),
                loop_count: Some(1),
                random_access: false,
            });
        self.track = Some(track.index());
        self.dec
            .configure(&CodecConfig::from(track), pulse)
            .map_err(AdaptError::Decode)
    }

    fn fill(
        &mut self,
        min: usize,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), DecErr<D, C>> {
        while self.pending.len() < min && !self.eos {
            let ev = self
                .cursor
                .next(&self.bytes, &mut DemuxStage(&mut self.dem), pulse)
                .map_err(AdaptError::Demux)?
                .into_inner();
            match ev {
                DemuxEvent::Headers => self.on_headers(pulse)?,
                DemuxEvent::ConfigChanged { track, .. } if Some(track) == self.track => {
                    let t = self
                        .dem
                        .tracks()
                        .iter()
                        .find(|t| t.index() == track)
                        .ok_or(AdaptError::Shape)?;
                    self.dec
                        .configure(&CodecConfig::from(t), pulse)
                        .map_err(AdaptError::Decode)?;
                }
                DemuxEvent::Packet(mut p) if Some(p.track()) == self.track => {
                    let Self {
                        dec,
                        pending,
                        frames_decoded,
                        ..
                    } = self;
                    send_packet_until_accepted(dec, &mut p, pulse, |f| {
                        *frames_decoded += 1;
                        pending.push_back(f);
                        Ok::<(), SessionError<C::Error>>(())
                    })
                    .map_err(|e| match e {
                        PumpError::Upstream(e) | PumpError::Downstream(e) => AdaptError::Decode(e),
                    })?;
                }
                DemuxEvent::Done => {
                    let Self {
                        dec,
                        pending,
                        frames_decoded,
                        ..
                    } = self;
                    drain_decoder(dec, pulse, |f| {
                        *frames_decoded += 1;
                        pending.push_back(f);
                        Ok::<(), SessionError<C::Error>>(())
                    })
                    .map_err(|e| match e {
                        PumpError::Upstream(e) | PumpError::Downstream(e) => AdaptError::Decode(e),
                    })?;
                    self.eos = true;
                }
                // Other tracks, other configurations, seek reports.
                _ => {}
            }
        }
        Ok(())
    }
}

impl<D, C, R> AnimationFrameDecoder for VideoAnimationDecoder<D, C, R>
where
    D: Demuxer + Send,
    C: VideoDecoder + Send,
    R: RenderFrame + Send,
{
    type Error = DecErr<D, C>;

    fn wrap_sink_error(err: SinkError) -> Self::Error {
        AdaptError::Sink(err)
    }

    fn info(&self) -> &ImageInfo {
        &self.info
    }

    fn render_next_frame(
        &mut self,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Option<AnimationFrame<'_>>, Self::Error> {
        // One frame of lookahead: a frame lasts until the next one starts.
        self.fill(2, pulse)?;
        let Some(frame) = self.pending.pop_front() else {
            return Ok(None);
        };
        let pts = frame.timing().pts.ok_or(AdaptError::MissingTimestamp)?;
        let duration = match self.pending.front() {
            Some(next) => {
                let next_pts = next.timing().pts.ok_or(AdaptError::MissingTimestamp)?;
                pts.duration_until(next_pts).map_err(AdaptError::Time)?
            }
            None => self.final_duration,
        };
        self.render
            .render(&frame, &mut self.canvas.as_slice_mut())
            .map_err(AdaptError::Convert)?;
        let index = self.index;
        self.index += 1;
        Ok(Some(
            AnimationFrame::with_duration(self.canvas.as_slice(), duration, index)
                .with_pts(Some(pts)),
        ))
    }

    fn render_next_frame_to_sink(
        &mut self,
        pulse: Option<&dyn zencodec::Pulse>,
        sink: &mut dyn DecodeRowSink,
    ) -> Result<Option<OutputInfo>, Self::Error> {
        zencodec::helpers::copy_frame_to_sink(self, pulse, sink)
    }
}

type EncErr<E, M> = AdaptError<<E as VideoEncoder>::Error, <M as Muxer>::Error>;

/// A full-canvas animation encoder over a video encoder and a muxer.
///
/// Frames get timestamps on the encoder's clock, starting at zero; a
/// duration that is not a whole number of ticks is refused as
/// [`UnsupportedOperation::AnimationTiming`] before the frame is accepted.
pub struct AnimationVideoEncoder<E, M, P> {
    enc: E,
    mux: M,
    capture: P,
    next_pts: Timestamp,
    begun: bool,
    pushed: u32,
    declared: Option<u32>,
    lag_frames: u32,
}

impl<E: VideoEncoder, M: Muxer, P: CaptureFrame> AnimationVideoEncoder<E, M, P> {
    /// Encode with `enc` into `mux`, converting through `capture`.
    /// `lag_frames` is the encoder's lookahead, reported through
    /// [`begin`](AnimationFrameEncoder::begin) when the muxer streams.
    ///
    /// # Errors
    ///
    /// [`AdaptError::Shape`] when the encoder has no video output
    /// configuration yet.
    pub fn new(enc: E, mux: M, capture: P, lag_frames: u32) -> Result<Self, EncErr<E, M>> {
        let cfg = enc.output_config().ok_or(AdaptError::Shape)?;
        let next_pts = Timestamp::new(0, cfg.time_base);
        Ok(Self {
            enc,
            mux,
            capture,
            next_pts,
            begun: false,
            pushed: 0,
            declared: None,
            lag_frames,
        })
    }

    fn begin_inner(
        &mut self,
        width: u32,
        height: u32,
        frame_count: Option<u32>,
    ) -> Result<AnimationOutputTiming, EncErr<E, M>> {
        if self.begun {
            return Err(AdaptError::Mux(SessionError::Protocol(
                ProtocolViolation::BeginTwice,
            )));
        }
        let cfg = self.enc.output_config().ok_or(AdaptError::Shape)?;
        let video = cfg.video.ok_or(AdaptError::Shape)?;
        if (video.coded_width, video.coded_height) != (width, height) {
            return Err(AdaptError::Shape);
        }
        let track = TrackInfo::video(0, cfg.codec.clone(), cfg.time_base, video)
            .build()
            .map_err(|_| AdaptError::Shape)?;
        let timing = self
            .mux
            .begin(&[track], MuxHint::forward_only())
            .map_err(AdaptError::Mux)?;
        self.begun = true;
        self.declared = frame_count;
        Ok(match frame_count {
            Some(n) if self.lag_frames >= n => AnimationOutputTiming::AT_FINISH,
            _ if timing.streams() => AnimationOutputTiming::streaming(self.lag_frames),
            _ => AnimationOutputTiming::AT_FINISH,
        })
    }
}

impl<E, M, P> AnimationFrameEncoder for AnimationVideoEncoder<E, M, P>
where
    E: VideoEncoder + Send,
    M: Muxer + Send,
    P: CaptureFrame + Send,
{
    type Error = EncErr<E, M>;

    fn reject(op: UnsupportedOperation) -> Self::Error {
        AdaptError::Unsupported(op)
    }

    fn begin(
        &mut self,
        width: u32,
        height: u32,
        _descriptor: PixelDescriptor,
        frame_count: Option<u32>,
    ) -> Result<AnimationOutputTiming, Self::Error> {
        self.begin_inner(width, height, frame_count)
    }

    fn pending_output(&self) -> &[u8] {
        self.mux.pending_output()
    }

    fn consume_output(&mut self, n: usize) {
        self.mux.consume_output(n)
    }

    fn take_output(&mut self, into: &mut Vec<u8>) {
        self.mux.take_output(into)
    }

    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), Self::Error> {
        self.push_frame_exact(pixels, FrameDuration::from_millis(duration_ms), pulse)
    }

    fn push_frame_exact(
        &mut self,
        pixels: PixelSlice<'_>,
        duration: FrameDuration,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), Self::Error> {
        if self.declared.is_some_and(|n| self.pushed >= n) {
            return Err(AdaptError::FrameCountMismatch);
        }
        let ticks = duration_to_ticks(duration, self.next_pts.base())
            .map_err(|_| AdaptError::Unsupported(UnsupportedOperation::AnimationTiming))?;
        let next = self
            .next_pts
            .checked_add(duration)
            .map_err(AdaptError::Time)?;
        if !self.begun {
            self.begin_inner(pixels.width(), pixels.rows(), None)?;
        }
        let cfg = self.enc.output_config().ok_or(AdaptError::Shape)?.clone();
        let frame = self
            .capture
            .capture(
                &pixels,
                &cfg,
                FrameTiming::at(self.next_pts).with_duration_ticks(ticks),
            )
            .map_err(AdaptError::Convert)?;
        let mux = &mut self.mux;
        send_frame_until_accepted(&mut self.enc, &frame, pulse, |p| mux.write_packet(p, pulse))
            .map_err(|e| match e {
                PumpError::Upstream(e) => AdaptError::Encode(e),
                PumpError::Downstream(e) => AdaptError::Mux(e),
            })?;
        self.next_pts = next;
        self.pushed += 1;
        Ok(())
    }

    fn finish(mut self, pulse: Option<&dyn zencodec::Pulse>) -> Result<EncodeOutput, Self::Error> {
        if !self.begun {
            return Err(AdaptError::Mux(SessionError::Protocol(
                ProtocolViolation::NoBegin,
            )));
        }
        if self.declared.is_some_and(|n| n != self.pushed) {
            return Err(AdaptError::FrameCountMismatch);
        }
        let mux = &mut self.mux;
        drain_encoder(&mut self.enc, pulse, |p| mux.write_packet(p, pulse)).map_err(
            |e| match e {
                PumpError::Upstream(e) => AdaptError::Encode(e),
                PumpError::Downstream(e) => AdaptError::Mux(e),
            },
        )?;
        let fin = self.mux.finish(pulse).map_err(AdaptError::Mux)?;
        if !fin.patches.is_empty() {
            return Err(AdaptError::PatchesNotAllowed);
        }
        Ok(EncodeOutput::new(fin.tail, ImageFormat::Unknown))
    }
}
