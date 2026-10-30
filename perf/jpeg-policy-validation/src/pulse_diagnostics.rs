//! Native checkpoint observations through the reusable testkit oracle.
//! Work/phase assertions remain codec-specific; no wall-clock thresholds.
use how_far_really::profile::Trace;
use zencodec::enough::StopReason;
use zencodec_testkit::{
    Conformance,
    pulse::{Case, Limits, check_checkpoints},
};

fn retain_trace(case: Case, trace: &Trace, prefix: &str, require_reports: bool) -> Conformance {
    let (at, reason) = match case {
        Case::Uninterrupted => (usize::MAX, StopReason::Cancelled),
        Case::StopAt { checkpoint, reason } => (checkpoint, reason),
        _ => panic!("new checkpoint case needs an explicit artifact label"),
    };
    let root = std::env::var_os("JPEG_POLICY_ARTIFACTS")
        .expect("set JPEG_POLICY_ARTIFACTS to retain diagnostic traces");
    let dir = std::path::PathBuf::from(root).join(format!("pulse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut json = String::new();
    trace.write_json(&mut json).unwrap();
    std::fs::write(dir.join(format!("{prefix}-at-{at}-{reason:?}.json")), json).unwrap();
    if case == Case::Uninterrupted {
        // DiagnosticPulse records each delegated stop poll exactly once.
        // Keep the existing requirement for distinct early/middle/last checks.
        assert!(
            trace
                .spans
                .iter()
                .map(|span| span.stats.checks)
                .sum::<u64>()
                > 2
        );
        if require_reports {
            assert!(
                trace.spans.iter().any(|span| span.stats.reports > 0),
                "successful native decode must report progress"
            );
        }
    }
    Ok(())
}

fn limits() -> Limits {
    Limits {
        checkpoints: 4096,
        trace_capacity: 4096,
    }
}

#[test]
fn native_jpeg_preserves_one_shot_stops_with_diagnostics() {
    for progressive in [false, true] {
        let bytes = crate::fixtures::base(progressive);
        check_checkpoints(
            limits(),
            |pulse| {
                zenjpeg::decoder::Decoder::new()
                    .decode_with_pulse(&bytes, Some(pulse))
                    .map(|_| ())
            },
            |error| match error.kind() {
                zenjpeg::decoder::ErrorKind::Cancelled(reason) => Some(*reason),
                _ => None,
            },
            |case, trace| retain_trace(case, trace, &format!("progressive-{progressive}"), true),
        )
        .unwrap();
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

#[test]
fn native_png_retains_pulse_and_meets_incremental_contract() {
    use zencodec::{
        CodecErrorExt,
        decode::{Decode, DecodeJob, DecoderConfig},
        encode::EncoderConfig,
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
        // WholeFileIncremental here; controlled row streaming is qualified separately.
        zencodec_testkit::check_incremental_contract(zenpng::PngDecoderConfig::new(), bytes)
            .unwrap();
        check_checkpoints(
            limits(),
            |pulse| {
                zenpng::PngDecoderConfig::new()
                    .job()
                    .with_pulse(pulse.share().unwrap())
                    .decoder(std::borrow::Cow::Borrowed(bytes), &[])
                    .and_then(|decoder| decoder.decode())
                    .map(|_| ())
            },
            |error| match error.error_category() {
                Some(zencodec::ErrorCategory::Stopped(reason)) => Some(reason),
                _ => None,
            },
            // Retain zero report counts; this route does not promise useful progress.
            |case, trace| retain_trace(case, trace, &format!("png-alpha-{alpha}"), false),
        )
        .unwrap();
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

#[test]
fn native_png_preserves_results_through_preparation() {
    for image in [
        zencodec_testkit::TestImage::rgb8_gradient(97, 65),
        zencodec_testkit::TestImage::rgba8_gradient(97, 65),
    ] {
        zencodec_testkit::check_cross_path_pixel_equivalence(
            zenpng::PngEncoderConfig::new(),
            zenpng::PngDecoderConfig::new(),
            &image,
        )
        .unwrap();
    }
}
