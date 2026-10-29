//! The sans-I/O conformance checks against both in-crate codecs: the
//! reference declares `incremental` and `seekable` (true direction), the
//! minimal codec declares neither (false direction).

use zencodec::decode::DecoderConfig;
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec::source::{ReadAtShim, SliceReadAt};
use zencodec_testkit::reference::{ReferenceDecoderConfig, ReferenceEncoderConfig};
use zencodec_testkit::{
    TestImage, check_incremental_capability_honesty, check_read_at_contract,
    check_seekable_capability_honesty,
};

fn reference_bytes() -> Vec<u8> {
    let img = TestImage::rgba8_gradient(40, 24);
    ReferenceEncoderConfig::new()
        .job()
        .encoder()
        .expect("reference encoder")
        .encode(img.as_slice())
        .expect("reference encode")
        .into_vec()
}

#[test]
fn slice_source_meets_the_read_at_contract() {
    let bytes = reference_bytes();
    let mut src = ReadAtShim(SliceReadAt::new(&bytes));
    check_read_at_contract(&mut src, &bytes).unwrap();
    let mut empty = ReadAtShim(SliceReadAt::new(&[]));
    check_read_at_contract(&mut empty, &[]).unwrap();
}

#[test]
fn reference_codec_is_honest_in_the_true_direction() {
    let bytes = reference_bytes();
    assert!(ReferenceDecoderConfig::capabilities().incremental());
    assert!(ReferenceDecoderConfig::capabilities().seekable());
    check_incremental_capability_honesty(ReferenceDecoderConfig, &bytes).unwrap();
    check_seekable_capability_honesty(ReferenceDecoderConfig, &bytes).unwrap();
}

#[test]
fn check_all_covers_the_sansio_checks() {
    zencodec_testkit::check_all(ReferenceEncoderConfig::new(), ReferenceDecoderConfig).unwrap();
}

// Mutate only the first, already-cancelled call. The previous trip-after-two
// scenario never exercises this path, so these are harness regression controls.
struct BadPreCancel {
    inner: Box<zencodec::decode::DynIncrementalDecoder>,
    first: bool,
    mode: u8,
}

impl zencodec::decode::IncrementalDecode for BadPreCancel {
    type Error = zencodec::decode::ErasedError;

    fn step(
        &mut self,
        input: zencodec::decode::Input<'_>,
        sink: &mut dyn zencodec::decode::DecodeRowSink,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<zencodec::decode::Progress, zencodec::decode::StepError<Self::Error>> {
        let first = std::mem::replace(&mut self.first, false);
        if first && pulse.is_some_and(|p| p.check().is_err()) {
            match self.mode {
                0 => return Ok(zencodec::decode::Progress::need(0)),
                1 => {
                    sink.begin(1, 1, zenpixels::PixelDescriptor::RGBA8_SRGB)
                        .unwrap();
                }
                _ => {
                    return self
                        .inner
                        .step(input, sink, pulse)
                        .map_err(|error| match error {
                            zencodec::decode::StepError::Cancelled(_) => {
                                zencodec::decode::StepError::Cancelled(
                                    enough::StopReason::Cancelled,
                                )
                            }
                            other => other,
                        });
                }
            }
        }
        self.inner.step(input, sink, pulse)
    }

    fn requested(&self) -> &[zencodec::decode::InputRequest] {
        self.inner.requested()
    }
    fn info(&self) -> Option<&zencodec::ImageInfo> {
        self.inner.info()
    }
    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor> {
        self.inner.output_descriptor()
    }
}

#[test]
fn harness_detects_precancellation_violations() {
    use zencodec::decode::{Decode, DecodeJob};
    let bytes = reference_bytes();
    for (mode, expected) in [
        (0, "pre-cancellation"),
        (1, "sink effects"),
        (2, "TimedOut"),
    ] {
        let result = zencodec_testkit::check_incremental_decoder(
            || {
                Ok(Box::new(BadPreCancel {
                    inner: ReferenceDecoderConfig
                        .job()
                        .dyn_incremental_decoder(&[])
                        .unwrap(),
                    first: true,
                    mode,
                }))
            },
            |data| {
                ReferenceDecoderConfig
                    .job()
                    .decoder(std::borrow::Cow::Borrowed(data), &[])
                    .and_then(|d| d.decode())
                    .map_err(|e| e.to_string())
            },
            &bytes,
        );
        let failure = result.expect_err("faulty decoder must fail conformance");
        assert!(failure.detail.contains(expected), "{mode}: {failure}");
    }
}

#[test]
fn diagnostic_pulse_detects_a_swallowed_stop() {
    use how_far_along::{Outcome, Phase, PulseTree, Total};
    use how_far_really::diagnostics::{DiagnosticPulse, Kind, Options};
    use how_far_really::profile::{Profiler, SpanKind, StdClock};
    use zencodec::decode::{DecodeJob, Input, StepError};

    struct Stopped(enough::StopReason);
    impl enough::Stop for Stopped {
        fn check(&self) -> Result<(), enough::StopReason> {
            Err(self.0)
        }
    }

    for reason in [enough::StopReason::Cancelled, enough::StopReason::TimedOut] {
        for swallow in [false, true] {
            let profiler = Profiler::new(StdClock::new(), 16);
            let outer = profiler.span(None, "incremental operation", SpanKind::Work);
            let pulse = DiagnosticPulse::new(
                PulseTree::new(Phase::new("decode", Total::Unknown), Stopped(reason)),
                &profiler,
            );
            let observer = pulse.observer();
            let inner = ReferenceDecoderConfig
                .job()
                .dyn_incremental_decoder(&[])
                .unwrap();
            let mut decoder: Box<zencodec::decode::DynIncrementalDecoder> = if swallow {
                Box::new(BadPreCancel {
                    inner,
                    first: true,
                    mode: 0,
                })
            } else {
                inner
            };
            let mut sink = zencodec_io::CollectSink::new();
            profiler.cancellation_requested();
            let result = decoder.step(Input::empty(), &mut sink, Some(&pulse));
            // Drop the operation before recording return: cancellation request is
            // not worker exit or resource release.
            drop(decoder);
            profiler.operation_returned();
            let outcome = match &result {
                Err(StepError::Cancelled(actual)) => {
                    assert_eq!(*actual, reason);
                    Outcome::Cancelled
                }
                Err(_) => Outcome::Failed,
                Ok(_) => Outcome::Succeeded,
            };
            pulse.finish(outcome).unwrap();
            outer.finish(outcome);
            let trace = profiler.snapshot().with_progress(observer.snapshot());
            assert_eq!(trace.active_spans, 0);
            assert_eq!(trace.dropped_spans, 0);
            assert_eq!(trace.dropped_incidents, 0);
            assert!(
                trace
                    .spans
                    .iter()
                    .any(|s| s.stats.stop_reason == Some(reason))
            );
            let findings = trace.diagnose(&Options::default());
            assert_eq!(
                findings.iter().any(|f| f.kind == Kind::StopOutcomeMismatch),
                swallow
            );
            assert_eq!(result.is_ok(), swallow);
        }
    }
}
