//! Explicit bridges between rendered animations and timestamped native video.
//!
//! IVF has neither a loop count nor a final-frame duration. These losses must
//! be chosen by the caller and remain visible in the returned report. The
//! functions retain one converted frame; video decoding additionally retains
//! one presentation of lookahead to determine variable frame durations.

use crate::time::{TimeBase, Timestamp};
#[cfg(feature = "av1-decode")]
use crate::{
    color::ChromaLocation,
    display::{DisplayConversion, OutOfRange},
    frame::{self, RgbStorage},
};
use enough::Stop;
use std::fmt;
use zencodec::animation::FrameDuration;
#[cfg(feature = "av1-decode")]
use zencodec::encode::EncodeOutput;
#[cfg(feature = "av1-decode")]
use zenpixels::PixelBuffer;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Packed frame output, including an optional explicit display conversion.
#[cfg(feature = "av1-decode")]
pub struct VideoRgb<'a> {
    storage: RgbStorage,
    clipping: OutOfRange,
    display: Option<&'a DisplayConversion>,
    chroma: Option<ChromaLocation>,
    max_bytes: usize,
}
#[cfg(feature = "av1-decode")]
impl<'a> VideoRgb<'a> {
    pub fn new(storage: RgbStorage, clipping: OutOfRange, max_bytes: usize) -> Self {
        Self {
            storage,
            clipping,
            display: None,
            chroma: None,
            max_bytes,
        }
    }
    /// Override unspecified bitstream chroma position only after choosing an
    /// interpretation. An explicit bitstream position is never overwritten.
    pub fn with_unspecified_chroma(mut self, location: ChromaLocation) -> Self {
        self.chroma = Some(location);
        self
    }
    pub fn with_display(mut self, display: &'a DisplayConversion) -> Self {
        self.display = Some(display);
        self
    }
    #[cfg(feature = "av1-decode")]
    pub fn render(
        &self,
        source: &crate::av1::Av1Frame,
        stop: Option<&dyn Stop>,
    ) -> Result<PixelBuffer, VideoError> {
        let mapping = source.map();
        let mut view = mapping.yuv_view().map_err(VideoError::cause)?;
        if view.chroma_location() == ChromaLocation::Unknown
            && let Some(location) = self.chroma
        {
            view = view.with_chroma_location(location);
        }
        frame::to_rgb(
            view,
            self.storage,
            self.clipping,
            self.display,
            self.max_bytes,
            stop,
        )
        .map_err(VideoError::cause)
    }
}

/// Whether an animation's repetitions may be omitted from an IVF stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[cfg(feature = "av1-encode")]
pub enum LoopHandling {
    /// Require a known total play count of exactly one.
    RequireSinglePlay,
    /// Encode the authored frames once, reporting the original play count.
    OneIteration,
}

#[cfg(feature = "av1-encode")]
pub struct AnimationToVideo {
    clock: TimeBase,
    conversion: crate::encode_color::RgbToYuv,
    alpha: crate::encode_color::AlphaHandling,
    loops: LoopHandling,
    max_frames: u64,
    max_conversion_bytes: usize,
}
#[cfg(feature = "av1-encode")]
impl AnimationToVideo {
    pub fn new(
        clock: TimeBase,
        conversion: crate::encode_color::RgbToYuv,
        alpha: crate::encode_color::AlphaHandling,
        max_frames: u64,
        max_conversion_bytes: usize,
    ) -> Self {
        Self {
            clock,
            conversion,
            alpha,
            loops: LoopHandling::RequireSinglePlay,
            max_frames,
            max_conversion_bytes,
        }
    }
    pub fn with_loops(mut self, loops: LoopHandling) -> Self {
        self.loops = loops;
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VideoReport {
    frames: u64,
    first: Timestamp,
    end: Timestamp,
    last_duration: FrameDuration,
    source_plays: Option<u32>,
}
impl VideoReport {
    pub fn frames(self) -> u64 {
        self.frames
    }
    pub fn first_timestamp(self) -> Timestamp {
        self.first
    }
    /// End-exclusive timeline endpoint; IVF itself does not store this.
    pub fn end_timestamp(self) -> Timestamp {
        self.end
    }
    pub fn last_duration(self) -> FrameDuration {
        self.last_duration
    }
    pub fn source_plays(self) -> Option<u32> {
        self.source_plays
    }
}

/// Encode one iteration to a forward-only Write sink. All durations must be
/// positive integral ticks of the supplied clock: zero-duration animation
/// control frames and timing quantization need an explicit preceding adapter.
/// Errors can leave bytes in the sink; use a temporary sink for atomic output.
/// Set the encoder's owned Stop token as well to cancel inside AV1 kernels.
#[cfg(feature = "av1-encode")]
pub fn animation_to_ivf<W: std::io::Write>(
    source: &mut dyn zencodec::decode::DynAnimationFrameDecoder,
    mut target: crate::av1_encode::Av1IvfEncoder<W>,
    options: &AnimationToVideo,
    stop: Option<&dyn Stop>,
) -> Result<(W, VideoReport), VideoError> {
    check_stop(stop)?;
    let first = Timestamp::new(0, options.clock);
    let mut timestamp = first;
    let mut frames = 0;
    let mut last_duration = FrameDuration::new(0, 1).expect("constant");
    let mut source_plays = source.loop_count();
    loop {
        check_stop(stop)?;
        let Some(frame) = source
            .render_next_frame_owned(stop)
            .map_err(VideoError::Codec)?
        else {
            break;
        };
        source_plays = source.loop_count().or(source_plays);
        if options.loops == LoopHandling::RequireSinglePlay && source_plays != Some(1) {
            return Err(VideoError::Invalid(
                "IVF cannot retain unknown or repeated animation plays",
            ));
        }
        if frames >= options.max_frames {
            return Err(VideoError::Invalid("frame limit exceeded"));
        }
        let duration = frame.duration();
        let ticks = duration_ticks(duration, options.clock)?;
        if ticks == 0 {
            return Err(VideoError::Invalid(
                "video presentations require positive durations",
            ));
        }
        let next = timestamp
            .checked_add_ticks(ticks)
            .map_err(VideoError::cause)?;
        let converted = options
            .conversion
            .convert(
                &frame.pixels(),
                options.alpha,
                options.max_conversion_bytes,
                stop,
            )
            .map_err(VideoError::cause)?;
        target
            .push(converted.view(), timestamp)
            .map_err(VideoError::cause)?;
        timestamp = next;
        last_duration = duration;
        frames += 1;
    }
    if frames == 0 {
        return Err(VideoError::Invalid("empty animation"));
    }
    check_stop(stop)?;
    let writer = target.finish().map_err(VideoError::cause)?;
    Ok((
        writer,
        VideoReport {
            frames,
            first,
            end: timestamp,
            last_duration,
            source_plays,
        },
    ))
}

/// Decode presentation-order AV1 into a caller-selected animation encoder.
/// Durations are exact adjacent PTS differences, independent of packet order.
/// The final duration is mandatory because IVF cannot supply it. Duplicate or
/// decreasing PTS are rejected instead of being silently reordered/dropped.
/// Set the decoder's owned Stop token to cancel inside AV1 decode kernels.
#[cfg(feature = "av1-decode")]
pub fn ivf_to_animation<R: std::io::Read>(
    source: &mut crate::av1::Av1IvfDecoder<R>,
    mut target: Box<dyn zencodec::encode::DynAnimationFrameEncoder>,
    render: &VideoRgb<'_>,
    final_duration: FrameDuration,
    max_frames: u64,
    stop: Option<&dyn Stop>,
) -> Result<(EncodeOutput, VideoReport), VideoError> {
    check_stop(stop)?;
    if final_duration.numerator() == 0 {
        return Err(VideoError::Invalid("final duration must be positive"));
    }
    if max_frames == 0 {
        return Err(VideoError::Invalid("frame limit must be positive"));
    }
    // Reject an unrepresentable final endpoint before consuming a presentation.
    let final_ticks = duration_ticks(final_duration, source.info().time_base())?;
    let Some(mut current) = source.next_frame().map_err(VideoError::cause)? else {
        return Err(VideoError::Invalid("empty video"));
    };
    let first = current
        .timestamp()
        .ok_or(VideoError::Invalid("missing presentation time"))?;
    let mut count = 0;
    loop {
        check_stop(stop)?;
        if count >= max_frames {
            return Err(VideoError::Invalid("frame limit exceeded"));
        }
        let timestamp = current
            .timestamp()
            .ok_or(VideoError::Invalid("missing presentation time"))?;
        let next = source.next_frame().map_err(VideoError::cause)?;
        let duration = if let Some(ref next) = next {
            let next = next
                .timestamp()
                .ok_or(VideoError::Invalid("missing presentation time"))?;
            if timestamp.time_base() != next.time_base() || next.ticks() <= timestamp.ticks() {
                return Err(VideoError::Invalid(
                    "presentation times must increase on one clock",
                ));
            }
            let ticks = u64::try_from(i128::from(next.ticks()) - i128::from(timestamp.ticks()))
                .map_err(|_| VideoError::Invalid("duration overflow"))?;
            ticks_duration(ticks, timestamp.time_base())?
        } else {
            final_duration
        };
        let pixels = render.render(&current, stop)?;
        target
            .push_frame_timed(pixels.as_slice(), duration, stop)
            .map_err(VideoError::Codec)?;
        count += 1;
        if let Some(frame) = next {
            current = frame;
        } else {
            let end = timestamp
                .checked_add_ticks(final_ticks)
                .map_err(VideoError::cause)?;
            check_stop(stop)?;
            let encoded = target.finish(stop).map_err(VideoError::Codec)?;
            return Ok((
                encoded,
                VideoReport {
                    frames: count,
                    first,
                    end,
                    last_duration: final_duration,
                    source_plays: None,
                },
            ));
        }
    }
}

/// Encode a previously selected frame as a still image. Selection and its
/// actual PTS remain in `ExtractedFrame`; this does not substitute the request
/// timestamp or apply display-size resampling/orientation implicitly. Configure
/// the image encoder job with its own Stop token for cancellation in its kernels;
/// the borrowed token here guards conversion and the encode boundaries.
#[cfg(feature = "av1-decode")]
pub fn encode_extracted<E: zencodec::encode::Encoder>(
    source: &crate::av1_index::ExtractedFrame,
    encoder: E,
    render: &VideoRgb<'_>,
    stop: Option<&dyn Stop>,
) -> Result<EncodeOutput, VideoError> {
    check_stop(stop)?;
    let pixels = render.render(source.frame(), stop)?;
    let output = encoder
        .encode(pixels.as_slice())
        .map_err(VideoError::cause)?;
    check_stop(stop)?;
    Ok(output)
}

fn duration_ticks(duration: FrameDuration, clock: TimeBase) -> Result<u64, VideoError> {
    let numerator = u128::from(duration.numerator()) * u128::from(clock.denominator());
    let denominator = u128::from(duration.denominator()) * u128::from(clock.numerator());
    if !numerator.is_multiple_of(denominator) {
        return Err(VideoError::Invalid(
            "duration is not integral in the video clock",
        ));
    }
    u64::try_from(numerator / denominator).map_err(|_| VideoError::Invalid("duration overflow"))
}
#[cfg(feature = "av1-decode")]
fn ticks_duration(ticks: u64, clock: TimeBase) -> Result<FrameDuration, VideoError> {
    // Reduce before multiplying, so long ticks need not overflow needlessly.
    let mut a = ticks;
    let mut b = u64::from(clock.denominator());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let numerator = (ticks / a)
        .checked_mul(u64::from(clock.numerator()))
        .ok_or(VideoError::Invalid("duration overflow"))?;
    FrameDuration::new(numerator, clock.denominator() / a as u32).map_err(VideoError::cause)
}
fn check_stop(stop: Option<&dyn Stop>) -> Result<(), VideoError> {
    if let Some(stop) = stop {
        stop.check().map_err(VideoError::Stopped)?;
    }
    Ok(())
}
#[derive(Debug)]
#[non_exhaustive]
pub enum VideoError {
    Invalid(&'static str),
    Codec(BoxError),
    Stopped(enough::StopReason),
}
impl VideoError {
    fn cause(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Codec(Box::new(error))
    }
}
impl fmt::Display for VideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Codec(e) => e.fmt(f),
            Self::Stopped(s) => write!(f, "stopped: {s:?}"),
        }
    }
}
impl std::error::Error for VideoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}
