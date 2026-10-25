//! Production resolver behavior, with an instrumented configuration control.
//! This control tests route execution and ownership, not lossy image quality.
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use zencodec::{
    At, CodecSet, ImageFormat, Metadata, ResourceLimits, StopToken, UnsupportedOperation,
    encode::{EncodeJob, EncodeOutput, Encoder, EncoderConfig, Fidelity},
    operation::{
        Decision, DecisionCode, Decline, EncodeRequest, OperationError, PolicyFailure, PolicySet,
        Question, Selection,
    },
};
use zencodec_testkit::{
    ReferenceEncoderConfig, TestImage,
    reference::{RefEnc, RefEncodeJob, RefError},
};
use zenpixels::{PixelDescriptor, PixelSlice};

#[derive(Default)]
struct Counts {
    made: AtomicUsize,
    dropped: AtomicUsize,
    executed: AtomicUsize,
    last_id: AtomicUsize,
    live_at_encode: AtomicUsize,
    fail: AtomicBool,
    stop: AtomicBool,
}
#[derive(Clone)]
struct Control {
    counts: Arc<Counts>,
    target: Option<Fidelity>,
}
impl EncoderConfig for Control {
    type Error = RefError;
    type Job = Job;
    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB]
    }
    fn with_fidelity(mut self, target: Fidelity) -> Self {
        self.target = Some(if target == Fidelity::codec_quality(80.0) {
            target
        } else {
            Fidelity::Lossless
        });
        self
    }
    fn resolved_target_fidelity(&self) -> Option<Fidelity> {
        self.target
    }
    fn job(self) -> Job {
        let id = self.counts.made.fetch_add(1, Ordering::SeqCst) + 1;
        Job {
            counts: self.counts,
            inner: Some(ReferenceEncoderConfig::new().job()),
            id,
        }
    }
}
struct Job {
    counts: Arc<Counts>,
    inner: Option<RefEncodeJob>,
    id: usize,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.counts.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl EncodeJob for Job {
    type Error = RefError;
    type Enc = Enc;
    type AnimationFrameEnc = ();
    // Deliberately uncooperative: the outer operation must check cancellation.
    fn with_stop(self, _: StopToken) -> Self {
        self
    }
    fn with_limits(self, _: ResourceLimits) -> Self {
        self
    }
    #[allow(deprecated)]
    fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.inner = Some(self.inner.take().unwrap().with_metadata(metadata));
        self
    }
    fn encoder(mut self) -> Result<Enc, RefError> {
        Ok(Enc {
            counts: self.counts.clone(),
            inner: self.inner.take().unwrap().encoder()?,
            id: self.id,
        })
    }
    fn animation_frame_encoder(self) -> Result<(), RefError> {
        Err(RefError::Unsupported(UnsupportedOperation::AnimationEncode))
    }
}
struct Enc {
    counts: Arc<Counts>,
    inner: RefEnc,
    id: usize,
}
impl Encoder for Enc {
    type Error = RefError;
    fn reject(op: UnsupportedOperation) -> RefError {
        RefError::Unsupported(op)
    }
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, RefError> {
        self.counts.executed.fetch_add(1, Ordering::SeqCst);
        self.counts.last_id.store(self.id, Ordering::SeqCst);
        self.counts.live_at_encode.store(
            self.counts.made.load(Ordering::SeqCst) - self.counts.dropped.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
        if self.counts.fail.load(Ordering::SeqCst) {
            return Err(RefError::Invalid("native execution failure".into()));
        }
        self.inner.encode(pixels)
    }
}
#[derive(Clone)]
struct Stop(Arc<Counts>);
impl enough::Stop for Stop {
    fn check(&self) -> Result<(), enough::StopReason> {
        if self.0.stop.load(Ordering::SeqCst) {
            Err(enough::StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}
fn setup() -> (Arc<Counts>, CodecSet, TestImage) {
    let counts = Arc::new(Counts::default());
    let set = CodecSet::new()
        .with_encoder(Control {
            counts: counts.clone(),
            target: None,
        })
        .with_stop(StopToken::new(Stop(counts.clone())));
    (counts, set, TestImage::rgb8_gradient(3, 2))
}
fn request() -> EncodeRequest {
    EncodeRequest::new(ImageFormat::Pnm).preferring_fidelity(vec![
        Fidelity::codec_quality(25.0),
        Fidelity::codec_quality(80.0),
        Fidelity::Lossless,
    ])
}
fn selected(report: &zencodec::operation::OperationReport) -> zencodec::operation::EncodeOffer {
    let record = report.decisions().last().unwrap();
    match record.selected.unwrap() {
        Selection::Encode(offer) => offer,
        _ => panic!("unexpected domain"),
    }
}

#[test]
fn prepared_choices_are_filtered_but_the_selected_job_and_original_index_survive() {
    let (counts, set, pixels) = setup();
    let mut calls = 0;
    let output = request()
        .encode_with(
            &set,
            pixels.as_slice(),
            &mut |question: Question<'_>| -> Result<_, At<Infallible>> {
                calls += 1;
                let Question::Encode { facts, choices } = question else {
                    panic!("unexpected question")
                };
                assert_eq!(
                    (facts.width, facts.height, facts.descriptor),
                    (3, 2, PixelDescriptor::RGB8_SRGB)
                );
                assert_eq!(counts.executed.load(Ordering::SeqCst), 0);
                assert_eq!(counts.made.load(Ordering::SeqCst), 2);
                assert_eq!(counts.dropped.load(Ordering::SeqCst), 0);
                assert_eq!(choices.len(), 2);
                let choice = choices
                    .iter()
                    .find(|c| c.value().fidelity == Some(Fidelity::Lossless))
                    .unwrap();
                assert_eq!(choice.value().preference_index, 2);
                Ok(Decision::choose(choice))
            },
        )
        .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(counts.executed.load(Ordering::SeqCst), 1);
    assert_eq!(counts.last_id.load(Ordering::SeqCst), 2);
    assert_eq!(
        counts.live_at_encode.load(Ordering::SeqCst),
        0,
        "unselected jobs must be released before the chosen encoder runs"
    );
    assert_eq!(selected(output.report()).preference_index, 2);
    assert_eq!(selected(output.report()).fidelity, Some(Fidelity::Lossless));
    assert_eq!(
        output.data(),
        ReferenceEncoderConfig::new()
            .encode(pixels.as_slice())
            .unwrap()
            .data()
    );
}

#[test]
fn builtin_policy_is_configurable_directly_or_through_one_delegation() {
    let (_, set, pixels) = setup();
    let first = request().encode(&set, pixels.as_slice()).unwrap();
    assert_eq!(selected(first.report()).preference_index, 1);
    let req = request().with_policy_set(PolicySet::new().prefer_lossless(true));
    let lossless = req.encode(&set, pixels.as_slice()).unwrap();
    assert_eq!(selected(lossless.report()).preference_index, 2);
    assert!(lossless.report().decisions().next().unwrap().used_preset);
    let direct = request()
        .encode_with(
            &set,
            pixels.as_slice(),
            &mut PolicySet::new().prefer_lossless(true),
        )
        .unwrap();
    assert_eq!(selected(direct.report()).preference_index, 2);
    assert!(!direct.report().decisions().next().unwrap().used_preset);
}

#[test]
fn invalid_requests_refuse_before_preparing_any_job_or_calling_policy() {
    let (counts, set, pixels) = setup();
    let mut calls = 0;
    let mut resolver = |_: Question<'_>| -> Result<_, At<Infallible>> {
        calls += 1;
        Ok(Decision::UsePreset)
    };
    for targets in [
        vec![],
        vec![Fidelity::Lossless; 65],
        vec![Fidelity::Lossless, Fidelity::codec_quality(f32::NAN)],
    ] {
        assert!(
            EncodeRequest::new(ImageFormat::Pnm)
                .preferring_fidelity(targets)
                .encode_with(&set, pixels.as_slice(), &mut resolver)
                .is_err()
        );
    }
    assert_eq!(calls, 0);
    assert_eq!(counts.made.load(Ordering::SeqCst), 0);
    // The inclusive bound succeeds; only one prepared job is executed.
    EncodeRequest::new(ImageFormat::Pnm)
        .preferring_fidelity(vec![Fidelity::Lossless; 64])
        .encode(&set, pixels.as_slice())
        .unwrap();
    assert_eq!(counts.executed.load(Ordering::SeqCst), 1);
}

#[test]
fn old_tokens_and_tokens_from_nested_operations_cannot_authorize_another_job() {
    let (counts, set, pixels) = setup();
    let mut token = None;
    request()
        .encode_with(&set, pixels.as_slice(), &mut |q: Question<'_>| -> Result<
            _,
            At<Infallible>,
        > {
            let Question::Encode { choices, .. } = q else {
                panic!()
            };
            token = Some(choices.first().unwrap().id());
            Ok(Decision::UsePreset)
        })
        .unwrap();
    let error = request()
        .encode_with(&set, pixels.as_slice(), &mut |_: Question<'_>| -> Result<
            _,
            At<Infallible>,
        > {
            Ok(Decision::Choose(token.unwrap()))
        })
        .unwrap_err();
    assert_eq!(
        zencodec::find_cause::<PolicyFailure>(&error)
            .unwrap()
            .record()
            .code,
        DecisionCode::InvalidChoice
    );
    assert_eq!(counts.executed.load(Ordering::SeqCst), 1);
    let error = request()
        .encode_with(&set, pixels.as_slice(), &mut |_: Question<'_>| -> Result<
            _,
            At<Infallible>,
        > {
            // The nested operation is independent; only its own selected job runs.
            request()
                .encode_with(&set, pixels.as_slice(), &mut |q: Question<'_>| -> Result<
                    _,
                    At<Infallible>,
                > {
                    let Question::Encode { choices, .. } = q else {
                        panic!()
                    };
                    token = Some(choices.first().unwrap().id());
                    Ok(Decision::UsePreset)
                })
                .unwrap();
            Ok(Decision::Choose(token.unwrap()))
        })
        .unwrap_err();
    assert_eq!(
        zencodec::find_cause::<PolicyFailure>(&error)
            .unwrap()
            .record()
            .code,
        DecisionCode::InvalidChoice
    );
    assert_eq!(counts.executed.load(Ordering::SeqCst), 2);
}

#[derive(Debug)]
struct AppError(&'static str);
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for AppError {}

#[test]
fn callback_errors_keep_native_trace_without_leaking_payload_in_default_formatting() {
    let (counts, set, pixels) = setup();
    let error = request()
        .encode_with(&set, pixels.as_slice(), &mut |_: Question<'_>| -> Result<
            Decision,
            At<AppError>,
        > {
            Err(At::wrap(AppError("private-policy-payload")).at_named("entitlement lookup"))
        })
        .unwrap_err();
    assert_eq!(
        zencodec::find_cause::<AppError>(&error).unwrap().0,
        "private-policy-payload"
    );
    let original = zencodec::find_cause::<At<AppError>>(&error).unwrap();
    assert!(
        original
            .full_trace()
            .to_string()
            .contains("entitlement lookup")
    );
    assert!(!format!("{error:?} {error}").contains("private-policy-payload"));
    assert_eq!(
        zencodec::find_cause::<PolicyFailure>(&error)
            .unwrap()
            .record()
            .code,
        DecisionCode::CallbackFailed
    );
    assert_eq!(counts.executed.load(Ordering::SeqCst), 0);
    assert_eq!(
        counts.made.load(Ordering::SeqCst),
        counts.dropped.load(Ordering::SeqCst)
    );
}

#[test]
fn predefined_decline_and_native_execution_failure_have_distinct_inspectable_causes() {
    let (counts, set, pixels) = setup();
    let error = request()
        .encode_with(&set, pixels.as_slice(), &mut |_: Question<'_>| -> Result<
            _,
            At<Infallible>,
        > {
            Ok(Decision::decline(Decline::ApplicationRule))
        })
        .unwrap_err();
    assert_eq!(
        zencodec::find_cause::<Decline>(&error),
        Some(&Decline::ApplicationRule)
    );
    assert_eq!(
        zencodec::find_cause::<PolicyFailure>(&error)
            .unwrap()
            .record()
            .code,
        DecisionCode::Declined
    );
    counts.fail.store(true, Ordering::SeqCst);
    let error = request().encode(&set, pixels.as_slice()).unwrap_err();
    assert!(
        matches!(zencodec::find_cause::<RefError>(&error), Some(RefError::Invalid(text)) if text == "native execution failure")
    );
    let Some(OperationError::Execution { report, .. }) =
        zencodec::find_cause::<OperationError>(&error)
    else {
        panic!()
    };
    assert_eq!(selected(report).preference_index, 1);
    assert_eq!(counts.executed.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_before_preparation_or_after_callback_prevents_uncooperative_encode() {
    let (counts, set, pixels) = setup();
    counts.stop.store(true, Ordering::SeqCst);
    let error = request().encode(&set, pixels.as_slice()).unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<OperationError>(&error),
        Some(OperationError::Stopped(enough::StopReason::Cancelled))
    ));
    assert_eq!(counts.made.load(Ordering::SeqCst), 0);
    counts.stop.store(false, Ordering::SeqCst);
    let error = request()
        .encode_with(&set, pixels.as_slice(), &mut |_: Question<'_>| -> Result<
            _,
            At<Infallible>,
        > {
            counts.stop.store(true, Ordering::SeqCst);
            Ok(Decision::UsePreset)
        })
        .unwrap_err();
    let failure = zencodec::find_cause::<PolicyFailure>(&error).unwrap();
    assert_eq!(failure.record().code, DecisionCode::Stopped);
    assert_eq!(failure.stop_reason(), Some(enough::StopReason::Cancelled));
    assert_eq!(counts.executed.load(Ordering::SeqCst), 0);
    assert_eq!(
        counts.made.load(Ordering::SeqCst),
        counts.dropped.load(Ordering::SeqCst)
    );
}

#[test]
fn a_caught_callback_panic_drops_prepared_jobs_without_encoding() {
    let (counts, set, pixels) = setup();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        request().encode_with(&set, pixels.as_slice(), &mut |_: Question<'_>| -> Result<
            Decision,
            At<Infallible>,
        > {
            panic!("callback unwind control")
        })
    }));
    assert!(panic.is_err());
    assert_eq!(counts.executed.load(Ordering::SeqCst), 0);
    assert_eq!(
        counts.made.load(Ordering::SeqCst),
        counts.dropped.load(Ordering::SeqCst)
    );
    request().encode(&set, pixels.as_slice()).unwrap(); // a fresh operation
    assert_eq!(counts.executed.load(Ordering::SeqCst), 1);
}
