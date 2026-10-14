//! `check_streaming_output_honesty` against the reference encoder (honest)
//! and against wrappers that misreport their `OutputTiming` (each must be
//! caught).

#![allow(deprecated)] // with_metadata delegates to the wrapped job

use zencodec::encode::{
    EncodeCapabilities, EncodeJob, EncodeOutput, EncodePolicy, Encoder, EncoderConfig, OutputTiming,
};
use zencodec::{ImageFormat, Metadata, ResourceLimits, StopToken, UnsupportedOperation};
use zencodec_testkit::reference::{
    RefEnc, RefEncodeJob, RefError, ReferenceDecoderConfig, ReferenceEncoderConfig,
};
use zencodec_testkit::{TestImage, check_streaming_output_honesty};
use zenpixels::{PixelDescriptor, PixelSlice};

/// How a [`Liar`] misbehaves.
const HOLD_ROWS: u8 = 0; // reports lag 0, exposes only the header until finish
const LEAK: u8 = 1; // reports AT_FINISH, exposes bytes anyway
const UNDECLARED: u8 = 2; // honest bytes, but the codec omits streaming_output
const DECLINE: u8 = 3; // this configuration cannot push rows: begin rejects
const HONEST_LAG: u8 = 4; // holds rows back and says so

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
        let real = self.inner.begin(width, height, desc)?;
        assert_eq!(real, OutputTiming::streaming(0));
        self.take(true); // the header
        Ok(match F {
            LEAK => OutputTiming::AT_FINISH,
            HONEST_LAG => OutputTiming::streaming(self.lag),
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
            HOLD_ROWS => false,
            HONEST_LAG => self.rows >= self.lag,
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
        self.drained += n;
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
