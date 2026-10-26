use super::{
    checked_collection::{Checked, Stage},
    control::CheckStop,
    image_items::{Composition, ImageRequest, Rect, SelectionError},
    jxl_items::JxlItems,
    jxl_tests::fixture,
    output_tests::mapped_planes,
};
use std::{
    cell::Cell,
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zencodec::{
    At, StopToken, Unstoppable,
    enough::{Stop, StopReason},
    operation::{Decision, DecisionCode, Decline, PolicyResolver, PolicySet, Question},
};

fn open<'a, R: PolicyResolver>(
    data: &'a [u8],
    stop: StopToken,
    resolver: R,
) -> Checked<JxlItems<'a>, R> {
    Checked::open(stop, resolver, |stop| {
        JxlItems::open_with_stop(data, Composition::Composited, 64, stop)
    })
    .unwrap()
}

#[test]
fn checked_collection_guide_runs_native_factory_and_returns_owned_checked_pixels() {
    let data = fixture("named_frame_test.jxl");
    let expected = super::output_tests::expected_output(&data);
    let image = super::collection::checked_jxl_first(
        &data,
        super::jxl_items::Limits {
            images: 1,
            frame_headers: Some(1),
        },
        PolicySet::new(),
        StopToken::new(Unstoppable),
    )
    .unwrap()
    .unwrap();
    drop(data);
    super::jxl_tests::exact_rows(&expected[0], &mapped_planes(&image));
    assert_eq!(
        image.report().decisions().next().unwrap().code,
        DecisionCode::Selected
    );
}

#[test]
fn checked_collection_repeated_native_output_is_rechecked_without_copying() {
    let data = fixture("named_frame_test.jxl");
    let seen = Cell::new(0);
    let address = Cell::new(0);
    let resolver = |question: Question<'_>| -> Result<Decision, At<Infallible>> {
        let Question::Image { image, facts, .. } = question else {
            panic!("wrong question")
        };
        assert_eq!(facts.source_details, 1);
        seen.set(seen.get() + 1);
        address.set(image.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap());
        Ok(Decision::UsePreset)
    };
    let mut collection = open(&data, StopToken::new(Unstoppable), resolver);
    let mut cursor = collection.images().unwrap();
    let id = collection.next(&mut cursor).unwrap().unwrap();
    let mut expected = None;
    for i in 0..20 {
        let output = collection.decode(&id, &ImageRequest::default()).unwrap();
        let pixels = mapped_planes(&output);
        if let Some(expected) = &expected {
            assert_eq!(&pixels, expected);
        } else {
            expected = Some(pixels);
        }
        assert_eq!(
            output.with_pixels(|p| p.row(0).as_ptr() as usize).unwrap(),
            address.get()
        );
        assert_eq!(seen.get(), i + 1);
        assert_eq!(output.report().decisions().count(), 1);
        assert!(output.report().decisions().next().unwrap().used_preset);
    }
    assert_eq!(collection.report().decisions().count(), 16);
    assert_eq!(collection.report().dropped_decisions(), 4);
    assert!(!collection.is_finished());
}

#[test]
fn checked_collection_request_and_policy_failures_remain_request_local() {
    let data = fixture("named_frame_test.jxl");
    let seen = Cell::new(0);
    let resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        seen.set(seen.get() + 1);
        Ok(if seen.get() == 1 {
            Decision::decline(Decline::ApplicationRule)
        } else {
            Decision::UsePreset
        })
    };
    let mut collection = open(&data, StopToken::new(Unstoppable), resolver);
    let mut cursor = collection.images().unwrap();
    let id = collection.next(&mut cursor).unwrap().unwrap();
    let error = collection
        .decode(
            &id,
            &ImageRequest {
                region: Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                }
                .into(),
            },
        )
        .unwrap_err();
    assert_eq!(error.error().stage, Stage::Decode);
    assert_eq!(
        zencodec::find_cause::<SelectionError>(&error),
        Some(&SelectionError::UnsupportedRegion)
    );
    assert_eq!(
        seen.get(),
        0,
        "a failed native request cannot reach publication"
    );
    let error = collection
        .decode(&id, &ImageRequest::default())
        .unwrap_err();
    assert_eq!(error.error().stage, Stage::Policy);
    assert_eq!(
        zencodec::find_cause::<Decline>(&error),
        Some(&Decline::ApplicationRule)
    );
    assert_eq!(
        error.error().report.decisions().last().unwrap().code,
        DecisionCode::Declined
    );
    let image = collection.decode(&id, &ImageRequest::default()).unwrap();
    assert_eq!(seen.get(), 2);
    assert_eq!(image.interpretation().geometry.region.width, 8);
    assert_eq!(collection.report().decisions().count(), 2);
    assert!(!collection.is_finished());
}

#[test]
fn checked_collection_configured_preset_refuses_unproven_obligations() {
    let data = fixture("named_frame_test.jxl");
    for preset in [
        PolicySet::new().with_workspace_limit(1),
        PolicySet::new().allow_opaque_profiles(false),
    ] {
        let resolver =
            |_: Question<'_>| -> Result<Decision, At<Infallible>> { Ok(Decision::UsePreset) };
        let mut collection =
            open(&data, StopToken::new(Unstoppable), resolver).with_policy_set(preset);
        let error = collection
            .first_image(&ImageRequest::default())
            .unwrap_err();
        assert_eq!(error.error().stage, Stage::Policy);
        assert_eq!(
            zencodec::find_cause::<Decline>(&error),
            Some(&Decline::UnsupportedQuestion)
        );
        assert!(error.error().report.decisions().last().unwrap().used_preset);
    }
}

#[derive(Debug)]
struct ApplicationFailure;
impl std::fmt::Display for ApplicationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private-app-marker")
    }
}
impl std::error::Error for ApplicationFailure {}

#[test]
fn checked_collection_retains_application_trace_without_default_payload_logging() {
    let data = fixture("named_frame_test.jxl");
    let resolver = |_: Question<'_>| -> Result<Decision, At<ApplicationFailure>> {
        Err(At::wrap(ApplicationFailure).at_named("application publication rule"))
    };
    let mut collection = open(&data, StopToken::new(Unstoppable), resolver);
    let error = collection
        .first_image(&ImageRequest::default())
        .unwrap_err();
    let native = zencodec::find_cause::<At<ApplicationFailure>>(&error).unwrap();
    assert!(format!("{native:?}").contains("application publication rule"));
    assert!(zencodec::find_cause::<ApplicationFailure>(&error).is_some());
    assert!(!format!("{error} {error:?}").contains("private-app-marker"));
    assert_eq!(
        error.error().report.decisions().last().unwrap().code,
        DecisionCode::CallbackFailed
    );
}

#[test]
fn checked_collection_stale_decision_cannot_authorize_repeated_image() {
    let data = fixture("named_frame_test.jxl");
    let mut ticket = None;
    let mut seen = 0;
    let resolver = move |q: Question<'_>| -> Result<Decision, At<Infallible>> {
        let Question::Image { choices, .. } = q else {
            panic!("wrong question")
        };
        seen += 1;
        if seen == 2 {
            return Ok(Decision::Choose(ticket.unwrap()));
        }
        let choice = choices.first().unwrap();
        ticket = Some(choice.id());
        Ok(Decision::choose(choice))
    };
    let mut collection = open(&data, StopToken::new(Unstoppable), resolver);
    let first = collection
        .first_image(&ImageRequest::default())
        .unwrap()
        .unwrap();
    let error = collection
        .first_image(&ImageRequest::default())
        .unwrap_err();
    assert_eq!(
        error.error().report.decisions().last().unwrap().code,
        DecisionCode::InvalidChoice
    );
    let third = collection
        .first_image(&ImageRequest::default())
        .unwrap()
        .unwrap();
    assert_eq!(mapped_planes(&first), mapped_planes(&third));
}

#[test]
fn checked_collection_callback_unwind_is_terminal_but_keeps_published_owner() {
    let data = fixture("named_frame_test.jxl");
    let seen = Cell::new(0);
    let stop = CheckStop::default();
    let resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        seen.set(seen.get() + 1);
        assert_ne!(seen.get(), 2, "controlled policy unwind");
        Ok(Decision::UsePreset)
    };
    let mut collection = open(&data, stop.token(), resolver);
    let first = collection
        .first_image(&ImageRequest::default())
        .unwrap()
        .unwrap();
    let expected = mapped_planes(&first);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        collection.first_image(&ImageRequest::default())
    }));
    assert!(result.is_err());
    assert!(collection.is_finished());
    let calls = stop.calls();
    let error = collection
        .first_image(&ImageRequest::default())
        .unwrap_err();
    assert_eq!(error.error().stage, Stage::Finished);
    assert_eq!(stop.calls(), calls);
    assert_eq!(seen.get(), 2);
    assert_eq!(mapped_planes(&first), expected);
}

#[derive(Clone)]
struct OnceStop {
    calls: Arc<AtomicUsize>,
    at: usize,
    reason: StopReason,
}
impl Stop for OnceStop {
    fn check(&self) -> Result<(), StopReason> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == self.at {
            Err(self.reason)
        } else {
            Ok(())
        }
    }
}
#[test]
fn checked_collection_native_stop_latches_every_observed_poll_and_keeps_exact_reason() {
    let data = fixture("named_frame_test.jxl");
    let baseline = CheckStop::default();
    let mut collection = open(&data, baseline.token(), PolicySet::new());
    collection
        .first_image(&ImageRequest::default())
        .unwrap()
        .unwrap();
    let count = baseline.calls();
    assert!(count > 10);
    for reason in [StopReason::Cancelled, StopReason::TimedOut] {
        for at in 0..count {
            let stop = OnceStop {
                calls: Arc::default(),
                at,
                reason,
            };
            let opened = Checked::open(StopToken::new(stop.clone()), PolicySet::new(), |token| {
                JxlItems::open_with_stop(&data, Composition::Composited, 64, token)
            });
            let error = match opened {
                Ok(mut collection) => {
                    let error = collection
                        .first_image(&ImageRequest::default())
                        .unwrap_err();
                    assert!(collection.is_finished(), "poll {at}");
                    let before = stop.calls.load(Ordering::SeqCst);
                    assert_eq!(
                        collection.images().err().unwrap().error().stage,
                        Stage::Finished
                    );
                    assert_eq!(stop.calls.load(Ordering::SeqCst), before);
                    error
                }
                Err(error) => error,
            };
            // Native JXL, policy, and wrapper observations retain their own
            // existing error types, all with the exact reason.
            if let Some(native) = zencodec::find_cause::<zenjxl_decoder::api::Error>(&error) {
                assert_eq!(native.stop_reason(), Some(reason));
            } else if let Some(policy) =
                zencodec::find_cause::<zencodec::operation::PolicyFailure>(&error)
            {
                assert_eq!(policy.record().code, DecisionCode::Stopped);
                assert_eq!(policy.stop_reason(), Some(reason));
            } else {
                use zencodec::CodecErrorExt;
                assert_eq!(
                    error.error_category(),
                    Some(zencodec::ErrorCategory::Stopped(reason)),
                    "poll {at}"
                );
            }
        }
    }
}
