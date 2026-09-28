//! Animation integration outside zencodec's still-image API.
//!
//! Exact source durations are the default. Optional quantization rounds
//! cumulative endpoints, so rounding each individual frame cannot accumulate
//! drift. The codec remains responsible for native field widths and composition.

use crate::time::Rounding;
use enough::Stop;
use std::fmt;
use zencodec::{
    ImageInfo,
    animation::FrameDuration,
    decode::AnimationFrameDecoder,
    encode::{AnimationFrameEncoder, EncodeOutput},
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimingPolicy {
    Exact,
    /// Integer ticks per second; destination field widths still apply. Zero
    /// output durations remain zero. A backend may reject zero display frames.
    Quantize {
        ticks_per_second: u32,
        rounding: Rounding,
    },
}

/// Incremental cumulative-endpoint timing adapter. The exact running sum uses
/// reduced u128 arithmetic and fails on overflow. It never switches to floats
/// or saturates when an adversarial series of mutually-prime clocks grows.
#[derive(Clone, Debug)]
pub struct AnimationClock {
    policy: TimingPolicy,
    numerator: u128,
    denominator: u128,
    last_ticks: u128,
}

impl AnimationClock {
    pub fn new(policy: TimingPolicy) -> Result<Self, AnimationError> {
        if matches!(
            policy,
            TimingPolicy::Quantize {
                ticks_per_second: 0,
                ..
            }
        ) {
            return Err(AnimationError::InvalidClock);
        }
        Ok(Self {
            policy,
            numerator: 0,
            denominator: 1,
            last_ticks: 0,
        })
    }

    /// Map one duration. On error, the previous clock state is retained.
    pub fn push(&mut self, duration: FrameDuration) -> Result<FrameDuration, AnimationError> {
        if self.policy == TimingPolicy::Exact {
            return Ok(duration);
        }
        let TimingPolicy::Quantize {
            ticks_per_second,
            rounding,
        } = self.policy
        else {
            unreachable!()
        };
        let denominator = u128::from(duration.denominator());
        let common = gcd(self.denominator, denominator);
        let left = denominator / common;
        let right = self.denominator / common;
        let num = self
            .numerator
            .checked_mul(left)
            .and_then(|a| {
                u128::from(duration.numerator())
                    .checked_mul(right)
                    .and_then(|b| a.checked_add(b))
            })
            .ok_or(AnimationError::TimelineOverflow)?;
        let den = self
            .denominator
            .checked_mul(left)
            .ok_or(AnimationError::TimelineOverflow)?;
        let common = gcd(num, den);
        let (num, den) = (num / common, den / common);
        // Cancel the clock factor against the denominator before multiplication.
        let common = gcd(u128::from(ticks_per_second), den);
        let scaled = num
            .checked_mul(u128::from(ticks_per_second) / common)
            .ok_or(AnimationError::TimelineOverflow)?;
        let divisor = den / common;
        let mut endpoint = scaled / divisor;
        let remainder = scaled % divisor;
        let up = match rounding {
            Rounding::Floor => false,
            Rounding::Ceil => remainder != 0,
            // Exact halfway ties choose the earlier presentation endpoint.
            Rounding::Nearest => remainder > divisor - remainder,
        };
        if up {
            endpoint = endpoint
                .checked_add(1)
                .ok_or(AnimationError::TimelineOverflow)?;
        }
        let ticks = endpoint
            .checked_sub(self.last_ticks)
            .and_then(|v| u64::try_from(v).ok())
            .ok_or(AnimationError::TimelineOverflow)?;
        let output = FrameDuration::new(ticks, ticks_per_second)
            .map_err(|_| AnimationError::InvalidClock)?;
        self.numerator = num;
        self.denominator = den;
        self.last_ticks = endpoint;
        Ok(output)
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Limits for accepted display frames and cumulative logical packed pixel
/// bytes. This bounds work, not all codec allocations: configure each codec's
/// resource limits as well. Buffered encoders retain their native buffering.
#[derive(Clone, Copy, Debug)]
pub struct AnimationLimits {
    frames: u64,
    pixel_bytes: u64,
}
impl AnimationLimits {
    pub fn new(max_frames: u64, max_total_pixel_bytes: u64) -> Result<Self, AnimationError> {
        if max_frames == 0 || max_total_pixel_bytes == 0 {
            return Err(AnimationError::InvalidLimit);
        }
        Ok(Self {
            frames: max_frames,
            pixel_bytes: max_total_pixel_bytes,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AnimationReport {
    frames: u64,
    pixel_bytes: u64,
    changed_durations: u64,
    source_plays: Option<u32>,
}
impl AnimationReport {
    pub fn frames(self) -> u64 {
        self.frames
    }
    pub fn pixel_bytes(self) -> u64 {
        self.pixel_bytes
    }
    pub fn changed_durations(self) -> u64 {
        self.changed_durations
    }
    /// Zero means infinite, positive means total plays including the first.
    pub fn source_plays(self) -> Option<u32> {
        self.source_plays
    }
}

/// Transcode rendered full-canvas frames, without whole-animation pixel
/// buffering in this layer. `make_encoder` receives source metadata and total
/// play count so the caller can choose loss, color, loop and buffering policies
/// explicitly in the destination configuration. Unknown play count stays None.
///
/// Pixels and their ColorContext are passed unchanged. Unsupported input color,
/// alpha, precision or exact duration must be rejected by the selected encoder.
/// The function does not silently reduce precision, drop alpha, set a playback
/// minimum, add repeated frames, or expand an infinite loop.
pub fn transcode<D, E, F>(
    decoder: &mut D,
    make_encoder: F,
    timing: TimingPolicy,
    limits: AnimationLimits,
    stop: Option<&dyn Stop>,
) -> Result<(EncodeOutput, AnimationReport), AnimationError>
where
    D: AnimationFrameDecoder,
    E: AnimationFrameEncoder,
    F: FnOnce(&ImageInfo, Option<u32>) -> Result<E, BoxError>,
{
    check_stop(stop)?;
    let mut clock = AnimationClock::new(timing)?;
    let source_plays = decoder.loop_count();
    let mut encoder = make_encoder(decoder.info(), source_plays).map_err(AnimationError::Encode)?;
    let mut report = AnimationReport {
        frames: 0,
        pixel_bytes: 0,
        changed_durations: 0,
        source_plays,
    };
    loop {
        check_stop(stop)?;
        let Some(frame) = decoder
            .render_next_frame(stop)
            .map_err(|e| AnimationError::Decode(Box::new(e)))?
        else {
            break;
        };
        if report.frames == limits.frames {
            return Err(AnimationError::FrameLimit);
        }
        let pixels = frame.pixels();
        let bytes = u64::from(pixels.width())
            .checked_mul(u64::from(pixels.rows()))
            .and_then(|n| n.checked_mul(pixels.descriptor().bytes_per_pixel() as u64))
            .and_then(|n| n.checked_add(report.pixel_bytes))
            .ok_or(AnimationError::PixelLimit)?;
        if bytes > limits.pixel_bytes {
            return Err(AnimationError::PixelLimit);
        }
        let duration = clock.push(frame.duration())?;
        encoder
            .push_frame_timed(pixels.clone(), duration, stop)
            .map_err(|e| AnimationError::Encode(Box::new(e)))?;
        report.frames += 1;
        report.pixel_bytes = bytes;
        report.changed_durations += u64::from(duration != frame.duration());
    }
    check_stop(stop)?;
    let output = encoder
        .finish(stop)
        .map_err(|e| AnimationError::Encode(Box::new(e)))?;
    Ok((output, report))
}

/// Object-safe transcode for runtime-selected formats, without whole-animation pixel
/// buffering in this layer. `make_encoder` receives source metadata and total
/// play count so the caller can choose loss, color, loop and buffering policies
/// explicitly in the destination configuration. Unknown play count stays None.
///
/// Pixels and their ColorContext are passed unchanged. Unsupported input color,
/// alpha, precision or exact duration must be rejected by the selected encoder.
/// The function does not silently reduce precision, drop alpha, set a playback
/// minimum, add repeated frames, or expand an infinite loop.
pub fn transcode_dyn<F>(
    decoder: &mut dyn zencodec::decode::DynAnimationFrameDecoder,
    make_encoder: F,
    timing: TimingPolicy,
    limits: AnimationLimits,
    stop: Option<&dyn Stop>,
) -> Result<(EncodeOutput, AnimationReport), AnimationError>
where
    F: FnOnce(
        &ImageInfo,
        Option<u32>,
    ) -> Result<Box<dyn zencodec::encode::DynAnimationFrameEncoder>, BoxError>,
{
    transcode_dyn_with(decoder, make_encoder, Ok, timing, limits, stop)
}

/// Transcode with an explicit owned-frame conversion, such as an ICC transform
/// or a caller-selected tone map. The callback must return pixels with their
/// resulting ColorContext. This layer preserves timing and loop semantics.
///
/// The pixel-byte work budget charges the larger of each input/output canvas;
/// configure the callback's own allocation limit before it allocates. The
/// callback also owns cancellation inside its processing kernels.
pub fn transcode_dyn_with<F, T>(
    decoder: &mut dyn zencodec::decode::DynAnimationFrameDecoder,
    make_encoder: F,
    mut transform: T,
    timing: TimingPolicy,
    limits: AnimationLimits,
    stop: Option<&dyn Stop>,
) -> Result<(EncodeOutput, AnimationReport), AnimationError>
where
    F: FnOnce(
        &ImageInfo,
        Option<u32>,
    ) -> Result<Box<dyn zencodec::encode::DynAnimationFrameEncoder>, BoxError>,
    T: FnMut(zenpixels::PixelBuffer) -> Result<zenpixels::PixelBuffer, BoxError>,
{
    check_stop(stop)?;
    let mut clock = AnimationClock::new(timing)?;
    let source_plays = decoder.loop_count();
    let mut encoder = make_encoder(decoder.info(), source_plays).map_err(AnimationError::Encode)?;
    let mut report = AnimationReport {
        frames: 0,
        pixel_bytes: 0,
        changed_durations: 0,
        source_plays,
    };
    loop {
        check_stop(stop)?;
        let Some(frame) = decoder
            .render_next_frame_owned(stop)
            .map_err(AnimationError::Decode)?
        else {
            break;
        };
        if report.frames == limits.frames {
            return Err(AnimationError::FrameLimit);
        }
        let source_duration = frame.duration();
        let input_bytes = logical_bytes(&frame.pixels())?;
        let bytes = report
            .pixel_bytes
            .checked_add(input_bytes)
            .ok_or(AnimationError::PixelLimit)?;
        if bytes > limits.pixel_bytes {
            return Err(AnimationError::PixelLimit);
        }
        let converted = transform(frame.into_buffer()).map_err(AnimationError::Transform)?;
        check_stop(stop)?;
        let pixels = converted.as_slice();
        let bytes = report
            .pixel_bytes
            .checked_add(input_bytes.max(logical_bytes(&pixels)?))
            .ok_or(AnimationError::PixelLimit)?;
        if bytes > limits.pixel_bytes {
            return Err(AnimationError::PixelLimit);
        }
        let duration = clock.push(source_duration)?;
        encoder
            .push_frame_timed(pixels, duration, stop)
            .map_err(AnimationError::Encode)?;
        report.frames += 1;
        report.pixel_bytes = bytes;
        report.changed_durations += u64::from(duration != source_duration);
    }
    check_stop(stop)?;
    let output = encoder.finish(stop).map_err(AnimationError::Encode)?;
    Ok((output, report))
}

fn logical_bytes(pixels: &zenpixels::PixelSlice<'_>) -> Result<u64, AnimationError> {
    u64::from(pixels.width())
        .checked_mul(u64::from(pixels.rows()))
        .and_then(|n| n.checked_mul(pixels.descriptor().bytes_per_pixel() as u64))
        .ok_or(AnimationError::PixelLimit)
}

fn check_stop(stop: Option<&dyn Stop>) -> Result<(), AnimationError> {
    if let Some(stop) = stop {
        stop.check().map_err(|_| AnimationError::Cancelled)?;
    }
    Ok(())
}

#[derive(Debug)]
#[non_exhaustive]
pub enum AnimationError {
    InvalidClock,
    TimelineOverflow,
    InvalidLimit,
    FrameLimit,
    PixelLimit,
    Cancelled,
    Decode(BoxError),
    Encode(BoxError),
    Transform(BoxError),
}
impl fmt::Display for AnimationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidClock => f.write_str("animation tick clock must be positive"),
            Self::TimelineOverflow => {
                f.write_str("animation timeline exceeds exact integer arithmetic")
            }
            Self::InvalidLimit => f.write_str("animation limits must be positive"),
            Self::FrameLimit => f.write_str("animation exceeds the accepted-frame limit"),
            Self::PixelLimit => f.write_str("animation exceeds the cumulative pixel-byte limit"),
            Self::Cancelled => f.write_str("animation transcode cancelled"),
            Self::Decode(e) => write!(f, "animation decode: {e}"),
            Self::Encode(e) => write!(f, "animation encode: {e}"),
            Self::Transform(e) => write!(f, "animation frame conversion: {e}"),
        }
    }
}
impl std::error::Error for AnimationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) | Self::Encode(e) | Self::Transform(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}
