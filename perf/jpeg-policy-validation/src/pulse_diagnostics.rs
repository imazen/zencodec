//! Native checkpoint observations; no wall-clock performance assertions.
use how_far_along::{Outcome, Phase, PulseTree, Total};
use how_far_really::{
    diagnostics::{DiagnosticPulse, Kind, Options},
    profile::{Profiler, SpanKind, StdClock},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use zencodec::enough::{Stop, StopReason};

struct StopAt {
    calls: Arc<AtomicUsize>,
    at: usize,
    reason: StopReason,
}
impl Stop for StopAt {
    fn check(&self) -> Result<(), StopReason> {
        // A one-shot stop detects codecs which poll again and accidentally
        // replace the first interruption with a later successful check.
        if self.calls.fetch_add(1, Ordering::SeqCst) == self.at {
            Err(self.reason)
        } else {
            Ok(())
        }
    }
}

fn observed_decode(bytes: &[u8], progressive: bool, at: usize, reason: StopReason) -> usize {
    let calls = Arc::new(AtomicUsize::new(0));
    let profiler = Profiler::new(StdClock::new(), 4096);
    let operation = profiler.span(None, "native JPEG decode", SpanKind::Work);
    let pulse = DiagnosticPulse::new(
        PulseTree::new(
            Phase::new("decode", Total::Unknown),
            StopAt {
                calls: calls.clone(),
                at,
                reason,
            },
        ),
        &profiler,
    );
    let observer = pulse.observer();
    let result = zenjpeg::decoder::Decoder::new().decode_with_pulse(bytes, Some(&pulse));
    let outcome = if at == usize::MAX {
        assert!(result.is_ok(), "{result:?}");
        Outcome::Succeeded
    } else {
        assert!(
            matches!(result.as_ref().map_err(|e| e.kind()),
            Err(zenjpeg::decoder::ErrorKind::Cancelled(actual)) if *actual == reason),
            "checkpoint {at}, reason {reason:?}: {result:?}"
        );
        Outcome::Cancelled
    };
    // Synchronous decoder call has returned; no spawned task is inferred done.
    profiler.operation_returned();
    pulse.finish(outcome).unwrap();
    operation.finish(outcome);
    let trace = profiler.snapshot().with_progress(observer.snapshot());
    assert_eq!(trace.active_spans, 0);
    assert_eq!(trace.dropped_spans, 0);
    assert_eq!(trace.dropped_incidents, 0);
    if at != usize::MAX {
        assert!(
            trace
                .spans
                .iter()
                .any(|s| s.stats.stop_reason == Some(reason))
        );
    }
    let root = std::env::var_os("JPEG_POLICY_ARTIFACTS")
        .expect("set JPEG_POLICY_ARTIFACTS to retain diagnostic traces");
    let dir = std::path::PathBuf::from(root).join(format!("pulse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut json = String::new();
    trace.write_json(&mut json).unwrap();
    std::fs::write(
        dir.join(format!("progressive-{progressive}-at-{at}-{reason:?}.json")),
        json,
    )
    .unwrap();
    if at == usize::MAX {
        assert!(
            trace.spans.iter().any(|s| s.stats.reports > 0),
            "successful native decode must report progress through DiagnosticPulse"
        );
    }
    let findings = trace.diagnose(&Options::default());
    assert!(
        !findings.iter().any(|f| f.kind == Kind::StopOutcomeMismatch),
        "{findings:?}"
    );
    calls.load(Ordering::SeqCst)
}

#[test]
fn native_jpeg_preserves_one_shot_stops_with_diagnostics() {
    for progressive in [false, true] {
        let bytes = crate::fixtures::base(progressive);
        let checks = observed_decode(&bytes, progressive, usize::MAX, StopReason::Cancelled);
        assert!(
            checks > 2,
            "need distinct early/middle/last native checkpoints"
        );
        for at in 0..checks {
            for reason in [StopReason::Cancelled, StopReason::TimedOut] {
                observed_decode(&bytes, progressive, at, reason);
            }
        }
    }
}

#[test]
fn native_jpeg_meets_shared_incremental_contract() {
    for progressive in [false, true] {
        let bytes = crate::fixtures::base(progressive);
        zencodec_testkit::check_incremental_contract(zenjpeg::JpegDecoderConfig::new(), &bytes)
            .unwrap();
    }
}

fn observed_png(bytes: &[u8], alpha: bool, at: usize, reason: StopReason) -> usize {
    use zencodec::{
        CodecErrorExt, Pulse,
        decode::{Decode, DecodeJob, DecoderConfig},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let profiler = Profiler::new(StdClock::new(), 4096);
    let operation = profiler.span(None, "PNG retained job decode", SpanKind::Work);
    let pulse = DiagnosticPulse::new(
        PulseTree::new(
            Phase::new("decode", Total::Unknown),
            StopAt {
                calls: calls.clone(),
                at,
                reason,
            },
        ),
        &profiler,
    );
    let observer = pulse.observer();
    let result = zenpng::PngDecoderConfig::new()
        .job()
        .with_pulse(pulse.share().unwrap())
        .decoder(std::borrow::Cow::Borrowed(bytes), &[])
        .and_then(|decoder| decoder.decode());
    let outcome = if at == usize::MAX {
        assert!(result.is_ok(), "{result:?}");
        Outcome::Succeeded
    } else {
        let error = result.expect_err("native PNG must preserve the one-shot stop");
        assert_eq!(
            error.error_category(),
            Some(zencodec::ErrorCategory::Stopped(reason)),
            "{error}"
        );
        Outcome::Cancelled
    };
    profiler.operation_returned();
    pulse.finish(outcome).unwrap();
    operation.finish(outcome);
    let trace = profiler.snapshot().with_progress(observer.snapshot());
    assert_eq!(trace.active_spans, 0);
    assert_eq!(trace.dropped_spans, 0);
    assert_eq!(trace.dropped_incidents, 0);
    if at != usize::MAX {
        assert!(
            trace
                .spans
                .iter()
                .any(|s| s.stats.stop_reason == Some(reason))
        );
    }
    // Retain report counts even when zero: absence is evidence, not a skip or
    // a false claim that a cancellation-capable path reports progress.
    let root = std::env::var_os("JPEG_POLICY_ARTIFACTS")
        .expect("set JPEG_POLICY_ARTIFACTS to retain diagnostic traces");
    let dir = std::path::PathBuf::from(root).join(format!("pulse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut json = String::new();
    trace.write_json(&mut json).unwrap();
    std::fs::write(
        dir.join(format!("png-alpha-{alpha}-at-{at}-{reason:?}.json")),
        json,
    )
    .unwrap();
    let findings = trace.diagnose(&Options::default());
    assert!(
        !findings.iter().any(|f| f.kind == Kind::StopOutcomeMismatch),
        "{findings:?}"
    );
    calls.load(Ordering::SeqCst)
}

#[test]
fn native_png_retains_pulse_and_meets_incremental_contract() {
    use zencodec::encode::EncoderConfig;
    for alpha in [false, true] {
        let pixels = if alpha {
            zencodec_testkit::TestImage::rgba8_gradient(97, 65)
        } else {
            zencodec_testkit::TestImage::rgb8_gradient(97, 65)
        };
        let encoded = zenpng::PngEncoderConfig::new()
            .encode(pixels.as_slice())
            .unwrap();
        let bytes = encoded.data();
        // At this pin the standard incremental API uses WholeFileIncremental;
        // this does not qualify PNG's separate controlled row-streaming route.
        zencodec_testkit::check_incremental_contract(zenpng::PngDecoderConfig::new(), bytes)
            .unwrap();
        let checks = observed_png(bytes, alpha, usize::MAX, StopReason::Cancelled);
        assert!(checks > 2);
        for at in 0..checks {
            for reason in [StopReason::Cancelled, StopReason::TimedOut] {
                observed_png(bytes, alpha, at, reason);
            }
        }
    }
}

#[test]
fn native_png_controlled_streaming_meets_incremental_contract() {
    use zencodec::{
        decode::{DecodeJob, DecoderConfig, erase_incremental},
        encode::EncoderConfig,
        operation::PolicySet,
        storage::StorageBudget,
    };
    for alpha in [false, true] {
        let pixels = if alpha {
            zencodec_testkit::TestImage::rgba8_gradient(97, 65)
        } else {
            zencodec_testkit::TestImage::rgb8_gradient(97, 65)
        };
        let encoded = zenpng::PngEncoderConfig::new()
            .encode(pixels.as_slice())
            .unwrap();
        let bytes = encoded.data();
        zencodec_testkit::check_incremental_decoder(
            || {
                let native = zenpng::PngDecoderConfig::new()
                    .job()
                    .dyn_controlled_decoder(&[], StorageBudget::new(bytes.len() + 65536, 8))
                    .map_err(|e| e.to_string())?;
                Ok(erase_incremental(zencodec_io::PolicyDecoder::new(
                    native,
                    PolicySet::new(),
                )))
            },
            |input| {
                zenpng::PngDecoderConfig::new()
                    .decode(input)
                    .map_err(|e| e.to_string())
            },
            bytes,
        )
        .unwrap();
    }
}
