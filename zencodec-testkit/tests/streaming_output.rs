//! `check_streaming_output_honesty` and
//! `check_animation_streaming_output_honesty` against the reference encoder
//! (honest) and against wrappers that misreport their timing (each must be
//! caught).

#![allow(deprecated)] // with_metadata delegates to the wrapped job

use zencodec::encode::{
    AnimationFrameEncoder, AnimationOutputTiming, EncodeCapabilities, EncodeJob, EncodeOutput,
    EncodePolicy, Encoder, EncoderConfig, OutputTiming,
};
use zencodec::{ImageFormat, Metadata, ResourceLimits, StopToken, UnsupportedOperation};
use zencodec_testkit::reference::{
    RefAnimEnc, RefEnc, RefEncodeJob, RefError, ReferenceDecoderConfig, ReferenceEncoderConfig,
};
use zencodec_testkit::{
    TestImage, check_animation_streaming_output_honesty, check_streaming_output_honesty,
};
use zenpixels::{PixelDescriptor, PixelSlice};

/// How a [`Liar`] misbehaves.
const HOLD_ROWS: u8 = 0; // reports lag 0, exposes only the header until finish
const LEAK: u8 = 1; // reports AT_FINISH, exposes bytes anyway
const UNDECLARED: u8 = 2; // honest bytes, but the codec omits streaming_output
const DECLINE: u8 = 3; // this configuration cannot push rows: begin rejects
const HONEST_LAG: u8 = 4; // holds rows back and says so
const HUGE_LAG: u8 = 5; // reports streaming(1_000_000) and holds every row
const STALL: u8 = 6; // reports lag 0, shows the first strip, then holds the rest
const BAD_CONSUME: u8 = 7; // a partial consume_output drops one byte too many
const BAD_TAKE: u8 = 8; // take_output drops the last pending byte
const ZERO_REFUSE: u8 = 9; // begin refuses a zero size that encode accepts

static STREAMING: EncodeCapabilities = EncodeCapabilities::new()
    .with_lossless(true)
    .with_native_alpha(true)
    .with_native_16bit(true)
    .with_push_rows(true)
    .with_streaming_output(true);
static NOT_STREAMING: EncodeCapabilities = EncodeCapabilities::new()
    .with_lossless(true)
    .with_native_alpha(true)
    .with_native_16bit(true)
    .with_push_rows(true);

#[derive(Clone, Debug, Default)]
struct Liar<const F: u8>(ReferenceEncoderConfig);

struct LiarJob<const F: u8>(RefEncodeJob);

struct LiarEnc<const F: u8> {
    inner: RefEnc,
    /// What the wrapper shows from `pending_output`.
    shown: Vec<u8>,
    drained: usize,
    /// Bytes taken from `inner` but not shown until `finish`.
    held: Vec<u8>,
    /// Rows still to hold back (HONEST_LAG).
    lag: u32,
    rows: u32,
}

impl<const F: u8> EncoderConfig for Liar<F> {
    type Error = RefError;
    type Job = LiarJob<F>;
    fn format() -> ImageFormat {
        ReferenceEncoderConfig::format()
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        ReferenceEncoderConfig::supported_descriptors()
    }
    fn capabilities() -> &'static EncodeCapabilities {
        if F == UNDECLARED {
            &NOT_STREAMING
        } else {
            &STREAMING
        }
    }
    fn job(self) -> LiarJob<F> {
        LiarJob(self.0.job())
    }
}

impl<const F: u8> EncodeJob for LiarJob<F> {
    type Error = RefError;
    type Enc = LiarEnc<F>;
    type AnimationFrameEnc = ();
    fn with_stop(self, stop: StopToken) -> Self {
        Self(self.0.with_stop(stop))
    }
    fn with_limits(self, limits: ResourceLimits) -> Self {
        Self(self.0.with_limits(limits))
    }
    fn with_policy(self, policy: EncodePolicy) -> Self {
        Self(self.0.with_policy(policy))
    }
    fn with_metadata(self, meta: Metadata) -> Self {
        Self(self.0.with_metadata(meta))
    }
    fn encoder(self) -> Result<LiarEnc<F>, RefError> {
        Ok(LiarEnc {
            inner: self.0.encoder()?,
            shown: Vec::new(),
            drained: 0,
            held: Vec::new(),
            lag: 8,
            rows: 0,
        })
    }
    fn animation_frame_encoder(self) -> Result<(), RefError> {
        Err(RefError::Unsupported(UnsupportedOperation::AnimationEncode))
    }
}

impl<const F: u8> LiarEnc<F> {
    /// Move the inner encoder's pending bytes to `shown` or `held`.
    fn take(&mut self, show: bool) {
        let bytes = self.inner.pending_output();
        let n = bytes.len();
        if show {
            self.shown.extend_from_slice(bytes);
        } else {
            self.held.extend_from_slice(bytes);
        }
        self.inner.consume_output(n);
    }
}

impl<const F: u8> Encoder for LiarEnc<F> {
    type Error = RefError;
    fn reject(op: UnsupportedOperation) -> RefError {
        RefError::Unsupported(op)
    }
    fn preferred_strip_height(&self) -> u32 {
        self.inner.preferred_strip_height()
    }
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, RefError> {
        self.inner.encode(pixels)
    }
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        desc: PixelDescriptor,
    ) -> Result<OutputTiming, RefError> {
        if F == DECLINE {
            return Err(RefError::Unsupported(UnsupportedOperation::RowLevelEncode));
        }
        if F == ZERO_REFUSE && (width == 0 || height == 0) {
            return Err(RefError::Invalid("zero size".into()));
        }
        let real = self.inner.begin(width, height, desc)?;
        assert_eq!(real, OutputTiming::streaming(0));
        self.take(true); // the header
        Ok(match F {
            LEAK => OutputTiming::AT_FINISH,
            HONEST_LAG => OutputTiming::streaming(self.lag),
            HUGE_LAG => OutputTiming::streaming(1_000_000),
            _ => real,
        })
    }
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), RefError> {
        let n = rows.rows();
        self.inner.push_rows(rows)?;
        self.rows += n;
        // HONEST_LAG holds back everything until `lag` rows have arrived,
        // then shows it all: never more than `lag` rows unwritten.
        let show = match F {
            HOLD_ROWS | HUGE_LAG => false,
            HONEST_LAG => self.rows >= self.lag,
            // The first push's bytes, then nothing.
            STALL => self.rows == n,
            _ => true,
        };
        if show && !self.held.is_empty() {
            let held = core::mem::take(&mut self.held);
            self.shown.extend_from_slice(&held);
        }
        self.take(show);
        Ok(())
    }
    fn pending_output(&self) -> &[u8] {
        &self.shown[self.drained..]
    }
    fn consume_output(&mut self, n: usize) {
        let pending = self.shown.len() - self.drained;
        let n = if F == BAD_CONSUME && n > 0 && n < pending {
            n + 1
        } else {
            n
        };
        self.drained = (self.drained + n).min(self.shown.len());
    }
    fn take_output(&mut self, into: &mut Vec<u8>) {
        let pending = &self.shown[self.drained..];
        let keep = if F == BAD_TAKE {
            pending.len().saturating_sub(1)
        } else {
            pending.len()
        };
        into.extend_from_slice(&pending[..keep]);
        self.drained = self.shown.len();
    }
    fn finish(mut self) -> Result<EncodeOutput, RefError> {
        let mut rest = self.shown.split_off(self.drained);
        rest.extend_from_slice(&self.held);
        rest.extend_from_slice(self.inner.finish()?.data());
        Ok(EncodeOutput::new(rest, ReferenceEncoderConfig::format()))
    }
}

fn img() -> TestImage {
    TestImage::rgba8_gradient(16, 32)
}

#[test]
fn the_reference_encoder_streams_and_says_so() {
    let mut enc = ReferenceEncoderConfig::new().job().encoder().unwrap();
    let timing = enc.begin(4, 4, PixelDescriptor::RGBA8_SRGB).unwrap();
    assert_eq!(timing, OutputTiming::streaming(0));
    assert!(timing.streams());
    assert!(!enc.pending_output().is_empty(), "header at begin");
    check_streaming_output_honesty(
        ReferenceEncoderConfig::new(),
        ReferenceDecoderConfig,
        &img(),
    )
    .unwrap();
}

#[test]
fn an_honest_lag_passes() {
    check_streaming_output_honesty(
        Liar::<HONEST_LAG>::default(),
        ReferenceDecoderConfig,
        &img(),
    )
    .unwrap();
}

#[test]
fn declining_row_encode_at_begin_passes() {
    check_streaming_output_honesty(Liar::<DECLINE>::default(), ReferenceDecoderConfig, &img())
        .unwrap();
}

#[test]
fn holding_rows_behind_a_zero_lag_is_caught() {
    let e = check_streaming_output_honesty(
        Liar::<HOLD_ROWS>::default(),
        ReferenceDecoderConfig,
        &img(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("lag of 0 rows"), "{e}");
}

#[test]
fn bytes_behind_at_finish_are_caught() {
    let e = check_streaming_output_honesty(Liar::<LEAK>::default(), ReferenceDecoderConfig, &img())
        .unwrap_err();
    assert!(e.to_string().contains("AT_FINISH"), "{e}");
}

#[test]
fn streaming_without_the_capability_is_caught() {
    let e = check_streaming_output_honesty(
        Liar::<UNDECLARED>::default(),
        ReferenceDecoderConfig,
        &img(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("not declared"), "{e}");
}

#[test]
fn output_timing_accessors() {
    assert!(!OutputTiming::AT_FINISH.streams());
    assert_eq!(OutputTiming::AT_FINISH.lag_rows(), None);
    assert_eq!(OutputTiming::default(), OutputTiming::AT_FINISH);
    assert_eq!(OutputTiming::streaming(16).lag_rows(), Some(16));
}

#[test]
fn a_lag_beyond_the_image_is_caught() {
    let e =
        check_streaming_output_honesty(Liar::<HUGE_LAG>::default(), ReferenceDecoderConfig, &img())
            .unwrap_err();
    assert!(e.to_string().contains("reports AT_FINISH"), "{e}");
}

#[test]
fn stalling_after_the_first_strip_is_caught() {
    let e =
        check_streaming_output_honesty(Liar::<STALL>::default(), ReferenceDecoderConfig, &img())
            .unwrap_err();
    assert!(e.to_string().contains("no bytes became pending"), "{e}");
}

#[test]
fn a_partial_consume_that_drops_a_byte_is_caught() {
    let e = check_streaming_output_honesty(
        Liar::<BAD_CONSUME>::default(),
        ReferenceDecoderConfig,
        &img(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("consume_output"), "{e}");
}

#[test]
fn an_image_too_short_for_the_lag_is_refused() {
    // HONEST_LAG holds 8 rows; with strips of 4 that needs more than 16 rows.
    let short = TestImage::rgba8_gradient(16, 16);
    let e = check_streaming_output_honesty(
        Liar::<HONEST_LAG>::default(),
        ReferenceDecoderConfig,
        &short,
    )
    .unwrap_err();
    assert!(e.to_string().contains("cannot verify a lag"), "{e}");
}

#[test]
fn check_all_picks_an_image_tall_enough_for_the_lag() {
    // The reference streams with lag 0; check_all must still run the whole
    // streaming check (it would fail on a 24-row image only for a larger lag).
    zencodec_testkit::check_all(ReferenceEncoderConfig::new(), ReferenceDecoderConfig).unwrap();
}

#[test]
fn a_take_output_that_drops_a_byte_is_caught() {
    let e =
        check_streaming_output_honesty(Liar::<BAD_TAKE>::default(), ReferenceDecoderConfig, &img())
            .unwrap_err();
    assert!(e.to_string().contains("take_output"), "{e}");
}

#[test]
fn a_begin_stricter_than_encode_about_zero_sizes_is_caught() {
    let e = check_streaming_output_honesty(
        Liar::<ZERO_REFUSE>::default(),
        ReferenceDecoderConfig,
        &img(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("begin(0, 32) was refused"), "{e}");
}

// ===========================================================================
// Animation
// ===========================================================================

const A_HONEST: u8 = 0; // the reference, passed through
const A_HOLD: u8 = 1; // reports lag 0, holds every frame until finish
const A_LEAK: u8 = 2; // reports AT_FINISH, exposes bytes anyway
const A_HUGE: u8 = 3; // reports a lag as long as the animation
const A_LOOSE_COUNT: u8 = 4; // finishes after fewer frames than declared
const A_PIXELS: u8 = 5; // streams, but changes a pixel byte of every frame

#[derive(Clone, Debug, Default)]
struct AnimLiar<const F: u8>(ReferenceEncoderConfig);

struct AnimLiarJob<const F: u8>(RefEncodeJob);

struct AnimLiarEnc<const F: u8> {
    inner: RefAnimEnc,
    shown: Vec<u8>,
    held: Vec<u8>,
    declared: Option<u32>,
    pushed: u32,
}

impl<const F: u8> EncoderConfig for AnimLiar<F> {
    type Error = RefError;
    type Job = AnimLiarJob<F>;
    fn format() -> ImageFormat {
        ReferenceEncoderConfig::format()
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        ReferenceEncoderConfig::supported_descriptors()
    }
    fn capabilities() -> &'static EncodeCapabilities {
        ReferenceEncoderConfig::capabilities()
    }
    fn job(self) -> AnimLiarJob<F> {
        AnimLiarJob(self.0.job())
    }
}

impl<const F: u8> EncodeJob for AnimLiarJob<F> {
    type Error = RefError;
    type Enc = RefEnc;
    type AnimationFrameEnc = AnimLiarEnc<F>;
    fn with_stop(self, stop: StopToken) -> Self {
        Self(self.0.with_stop(stop))
    }
    fn with_limits(self, limits: ResourceLimits) -> Self {
        Self(self.0.with_limits(limits))
    }
    fn with_policy(self, policy: EncodePolicy) -> Self {
        Self(self.0.with_policy(policy))
    }
    fn with_metadata(self, meta: Metadata) -> Self {
        Self(self.0.with_metadata(meta))
    }
    fn with_loop_count(self, count: Option<u32>) -> Self {
        Self(self.0.with_loop_count(count))
    }
    fn encoder(self) -> Result<RefEnc, RefError> {
        self.0.encoder()
    }
    fn animation_frame_encoder(self) -> Result<AnimLiarEnc<F>, RefError> {
        Ok(AnimLiarEnc {
            inner: self.0.animation_frame_encoder()?,
            shown: Vec::new(),
            held: Vec::new(),
            declared: None,
            pushed: 0,
        })
    }
}

impl<const F: u8> AnimLiarEnc<F> {
    fn take(&mut self) {
        let mut bytes = Vec::new();
        self.inner.take_output(&mut bytes);
        if F == A_PIXELS
            && self.pushed > 0
            && let Some(last) = bytes.last_mut()
        {
            *last ^= 1;
        }
        if F == A_HOLD {
            self.held.extend_from_slice(&bytes);
        } else {
            self.shown.extend_from_slice(&bytes);
        }
    }
}

impl<const F: u8> AnimationFrameEncoder for AnimLiarEnc<F> {
    type Error = RefError;
    fn reject(op: UnsupportedOperation) -> RefError {
        RefError::Unsupported(op)
    }
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        desc: PixelDescriptor,
        frame_count: Option<u32>,
    ) -> Result<AnimationOutputTiming, RefError> {
        let real = self.inner.begin(width, height, desc, frame_count)?;
        self.declared = frame_count;
        // The header stays out of sight for A_HOLD.
        self.take();
        Ok(match (F, frame_count) {
            (A_LEAK, _) => AnimationOutputTiming::AT_FINISH,
            (A_HUGE, Some(n)) => AnimationOutputTiming::streaming(n),
            _ => real,
        })
    }
    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        stop: Option<&dyn zencodec::enough::Stop>,
    ) -> Result<(), RefError> {
        self.inner.push_frame(pixels, duration_ms, stop)?;
        self.pushed += 1;
        self.take();
        Ok(())
    }
    fn pending_output(&self) -> &[u8] {
        &self.shown
    }
    fn consume_output(&mut self, n: usize) {
        let n = n.min(self.shown.len());
        self.shown.drain(..n);
    }
    fn finish(self, stop: Option<&dyn zencodec::enough::Stop>) -> Result<EncodeOutput, RefError> {
        if F == A_LOOSE_COUNT && self.declared.is_some_and(|n| n != self.pushed) {
            // Pretend all is well: hand back what was produced.
            let mut rest = self.shown;
            rest.extend_from_slice(&self.held);
            return Ok(EncodeOutput::new(rest, ReferenceEncoderConfig::format()));
        }
        let mut rest = self.shown;
        rest.extend_from_slice(&self.held);
        rest.extend_from_slice(self.inner.finish(stop)?.data());
        Ok(EncodeOutput::new(rest, ReferenceEncoderConfig::format()))
    }
}

fn frames(n: u8) -> Vec<TestImage> {
    (0..n)
        .map(|i| TestImage::rgba8_gradient_seeded(12, 8, i.wrapping_mul(40)))
        .collect()
}

#[test]
fn the_reference_animation_streams_only_with_a_frame_count() {
    let job = || {
        ReferenceEncoderConfig::new()
            .job()
            .with_loop_count(Some(0))
            .animation_frame_encoder()
            .unwrap()
    };
    let mut a = job();
    let t = a
        .begin(12, 8, PixelDescriptor::RGBA8_SRGB, Some(3))
        .unwrap();
    assert_eq!(t, AnimationOutputTiming::streaming(0));
    assert!(!a.pending_output().is_empty(), "header at begin");
    let mut a = job();
    let t = a.begin(12, 8, PixelDescriptor::RGBA8_SRGB, None).unwrap();
    assert_eq!(t, AnimationOutputTiming::AT_FINISH);
    assert!(a.pending_output().is_empty());
    check_animation_streaming_output_honesty(
        AnimLiar::<A_HONEST>::default(),
        ReferenceDecoderConfig,
        &frames(4),
    )
    .unwrap();
}

#[test]
fn animation_frames_held_behind_a_zero_lag_are_caught() {
    let e = check_animation_streaming_output_honesty(
        AnimLiar::<A_HOLD>::default(),
        ReferenceDecoderConfig,
        &frames(4),
    )
    .unwrap_err();
    assert!(e.to_string().contains("lag of 0 frames"), "{e}");
}

#[test]
fn animation_bytes_behind_at_finish_are_caught() {
    let e = check_animation_streaming_output_honesty(
        AnimLiar::<A_LEAK>::default(),
        ReferenceDecoderConfig,
        &frames(4),
    )
    .unwrap_err();
    assert!(e.to_string().contains("AT_FINISH"), "{e}");
}

#[test]
fn an_animation_lag_as_long_as_the_animation_is_caught() {
    let e = check_animation_streaming_output_honesty(
        AnimLiar::<A_HUGE>::default(),
        ReferenceDecoderConfig,
        &frames(4),
    )
    .unwrap_err();
    assert!(e.to_string().contains("holds every frame"), "{e}");
}

#[test]
fn finishing_short_of_the_declared_frames_is_caught() {
    let e = check_animation_streaming_output_honesty(
        AnimLiar::<A_LOOSE_COUNT>::default(),
        ReferenceDecoderConfig,
        &frames(4),
    )
    .unwrap_err();
    assert!(
        e.to_string().contains("declared frames was accepted"),
        "{e}"
    );
}

#[test]
fn animation_output_timing_accessors() {
    assert!(!AnimationOutputTiming::AT_FINISH.streams());
    assert_eq!(
        AnimationOutputTiming::default(),
        AnimationOutputTiming::AT_FINISH
    );
    assert_eq!(AnimationOutputTiming::streaming(2).lag_frames(), Some(2));
}

#[test]
fn a_streamed_animation_with_other_pixels_is_caught() {
    let e = check_animation_streaming_output_honesty(
        AnimLiar::<A_PIXELS>::default(),
        ReferenceDecoderConfig,
        &frames(4),
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("different pixels than the encode without begin"),
        "{e}"
    );
}
