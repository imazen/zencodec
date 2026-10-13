//! Fixture-driven checks for contracts introduced after the published testkit.
use super::*;
use zencodec::animation::FrameDuration;
use zencodec::decode::DecodePolicy;
use zencodec::encode::DynEncoderConfig;
use zencodec::encode::EncodePolicy;
use zencodec::metadata_rewrite::{Error as RewriteError, Plan, Request};
use zencodec::{LimitKind, ResourceLimits};

/// Verify source timing through borrowed, owned and erased animation decoders.
/// `expected` names every displayed frame; zero-duration compositing helpers
/// belong in the codec fixture's expected display sequence, not an inferred count.
/// The sink API returns only `OutputInfo`, so cannot expose frame timing.
pub fn check_animation_durations<D: DecoderConfig + 'static>(
    dec: D,
    bytes: &[u8],
    expected: &[FrameDuration],
) -> Conformance {
    const CHECK: &str = "animation durations";
    let err = |e: &dyn std::fmt::Display| fail(CHECK, e.to_string());
    let mut borrowed = dec
        .clone()
        .job()
        .animation_frame_decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| err(&e))?;
    let mut owned = dec
        .clone()
        .job()
        .animation_frame_decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| err(&e))?;
    let mut dynamic = dec
        .dyn_job()
        .into_animation_frame_decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| err(&e))?;
    for (index, duration) in expected.iter().enumerate() {
        let a = borrowed
            .render_next_frame(None)
            .map_err(|e| err(&e))?
            .ok_or_else(|| fail(CHECK, format!("borrowed frame {index} missing")))?;
        let b = owned
            .render_next_frame_owned(None)
            .map_err(|e| err(&e))?
            .ok_or_else(|| fail(CHECK, format!("owned frame {index} missing")))?;
        let c = dynamic
            .render_next_frame_owned(None)
            .map_err(|e| err(&e))?
            .ok_or_else(|| fail(CHECK, format!("erased frame {index} missing")))?;
        for (path, actual, frame_index) in [
            ("borrowed", a.duration(), a.frame_index()),
            ("owned", b.duration(), b.frame_index()),
            ("erased", c.duration(), c.frame_index()),
        ] {
            if actual != *duration || frame_index != index as u32 {
                return Err(fail(
                    CHECK,
                    format!(
                        "{path} frame {index}: duration {actual:?}, index {frame_index}; expected {duration:?}"
                    ),
                ));
            }
        }
        if grab_ref(a.pixels()) != grab(b.pixels()) || grab(b.pixels()) != grab(c.pixels()) {
            return Err(fail(
                CHECK,
                format!("frame {index}: generic/owned/erased pixels disagree"),
            ));
        }
    }
    if borrowed
        .render_next_frame(None)
        .map_err(|e| err(&e))?
        .is_some()
        || owned
            .render_next_frame_owned(None)
            .map_err(|e| err(&e))?
            .is_some()
        || dynamic
            .render_next_frame_owned(None)
            .map_err(|e| err(&e))?
            .is_some()
    {
        return Err(fail(
            CHECK,
            "unexpected extra frame (possibly accepted a rejected input)",
        ));
    }
    Ok(())
}

/// Exercise exact or ordinary timing through both generic and erased encoders.
/// Each pair is `(requested, expected_encoded)`; `None` requires rejection.
/// Include accepted frames before and after rejected inputs to detect state
/// mutation. Expected rounded durations must come from the destination clock,
/// independently of the codec/quantizer under test. At least two must succeed.
/// For exact mode each successful duration must equal its request.
pub fn check_animation_timing<E, D>(
    enc: E,
    dec: D,
    img: &TestImage,
    exact: bool,
    cases: &[(FrameDuration, Option<FrameDuration>)],
) -> Conformance
where
    E: EncoderConfig + 'static,
    D: DecoderConfig + 'static,
    <E::Job as EncodeJob>::Enc: Encoder + Send,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    const CHECK: &str = "animation timing";
    let err = |e: &dyn std::fmt::Display| fail(CHECK, e.to_string());
    let expected: Vec<_> = cases.iter().filter_map(|(_, result)| *result).collect();
    if expected.len() < 2 {
        return Err(fail(CHECK, "supply at least two accepted frames"));
    }
    let mut generic = enc
        .clone()
        .job()
        .animation_frame_encoder()
        .map_err(|e| err(&e))?;
    let mut erased = enc
        .dyn_job()
        .into_animation_frame_encoder()
        .map_err(|e| err(&e))?;
    for (index, (requested, expected)) in cases.iter().enumerate() {
        let mut data = img.data.clone();
        // Vary content to prevent identical-frame coalescing from concealing
        // a consumed rejected input. Preserve the caller's native descriptor.
        for byte in &mut data {
            *byte ^= index as u8;
        }
        let frame = TestImage {
            width: img.width,
            height: img.height,
            desc: img.desc,
            data,
        };
        if exact && expected.is_some_and(|v| v != *requested) {
            return Err(fail(CHECK, "exact test case requests rounding"));
        }
        let a = if exact {
            generic
                .push_frame_exact(frame.as_slice(), *requested, None)
                .map(|()| *requested)
        } else {
            generic.push_frame_timed(frame.as_slice(), *requested, None)
        };
        let b = if exact {
            erased
                .push_frame_exact(frame.as_slice(), *requested, None)
                .map(|()| *requested)
        } else {
            erased.push_frame_timed(frame.as_slice(), *requested, None)
        };
        for (path, result) in [
            ("generic", a.map_err(|e| e.to_string())),
            ("erased", b.map_err(|e| e.to_string())),
        ] {
            match (expected, result) {
                (Some(want), Ok(got)) if *want == got => (),
                (None, Err(_)) => (),
                (want, got) => {
                    return Err(fail(
                        CHECK,
                        format!("{path} case {index}: expected {want:?}, got {got:?}"),
                    ));
                }
            }
        }
    }
    let a = generic.finish(None).map_err(|e| err(&e))?;
    let b = erased.finish(None).map_err(|e| err(&e))?;
    check_animation_durations(dec.clone(), a.data(), &expected)?;
    check_animation_durations(dec.clone(), b.data(), &expected)?;
    if decode_anim_owned(&dec, a.data()).map_err(|e| fail(CHECK, e))?
        != decode_anim_owned(&dec, b.data()).map_err(|e| fail(CHECK, e))?
    {
        return Err(fail(CHECK, "generic/erased encoded pixels disagree"));
    }
    Ok(())
}

/// Verify the inventory input-byte boundary and typed errors in both dispatch
/// paths. This is opt-in for inventory-enabled builds; disabled builds use
/// `check_inventory_dispatch`. It does not equate a part cap with a memory cap.
pub fn check_inventory_input_limit<D: DecoderConfig + 'static>(
    dec: D,
    valid: &[u8],
) -> Conformance {
    const CHECK: &str = "inventory input limit";
    if valid.is_empty() || !D::capabilities().inventory() {
        return Err(fail(CHECK, "requires nonempty input and enabled inventory"));
    }
    for cap in [valid.len() - 1, valid.len()] {
        let limits = ResourceLimits::none().with_max_input_bytes(cap as u64);
        let a = dec.clone().job().with_limits(limits).inventory(valid);
        let mut job = dec.dyn_job();
        job.set_limits(limits);
        let b = job.inventory(valid);
        if cap == valid.len() {
            let a = a.map_err(|e| fail(CHECK, format!("exact boundary: {e}")))?;
            let b = b.map_err(|e| fail(CHECK, format!("erased exact boundary: {e}")))?;
            a.validate().map_err(|e| fail(CHECK, e.to_string()))?;
            if a != b {
                return Err(fail(CHECK, "boundary inventories disagree"));
            }
        } else {
            let wanted = Some(ErrorCategory::Resource(zencodec::ResourceError::Limits(
                LimitKind::InputSize,
            )));
            let a = a
                .err()
                .ok_or_else(|| fail(CHECK, "generic accepted oversized input"))?;
            let b = b
                .err()
                .ok_or_else(|| fail(CHECK, "erased accepted oversized input"))?;
            if a.error_category() != wanted || b.error_category() != wanted {
                return Err(fail(
                    CHECK,
                    format!("expected InputSize, got {a:?} and {b:?}"),
                ));
            }
        }
    }
    Ok(())
}

/// Verify checked encode-policy admission and rejection atomicity. `initial`
/// must be supported; `request` must match `accepted`. Decode observations must
/// equal `expected` metadata, making preservation and denial explicit instead
/// of treating a successful setter as proof of delivery. `metadata` is the
/// unfiltered input. Color/fidelity observations remain codec-specific.
#[allow(clippy::too_many_arguments)]
pub fn check_encode_policy_admission<E, D>(
    enc: E,
    dec: D,
    img: &TestImage,
    metadata: Metadata,
    initial: EncodePolicy,
    request: EncodePolicy,
    accepted: bool,
    expected: &Metadata,
) -> Conformance
where
    E: EncoderConfig + 'static,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error> + Send,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    const CHECK: &str = "checked encode policy";
    let mut job = enc
        .clone()
        .job()
        .with_metadata_policy(metadata.clone(), MetadataPolicy::PreserveExact)
        .try_with_policy(initial)
        .map_err(|e| fail(CHECK, format!("initial: {e}")))?;
    let mut dynamic = enc.dyn_job();
    dynamic.set_metadata_policy(metadata, MetadataPolicy::PreserveExact);
    dynamic
        .try_set_policy(initial)
        .map_err(|e| fail(CHECK, format!("erased initial: {e}")))?;
    let a = job.try_set_policy(request);
    let b = dynamic.try_set_policy(request);
    if a.is_ok() != accepted || b.is_ok() != accepted {
        return Err(fail(CHECK, format!("admission mismatch: {a:?}, {b:?}")));
    }
    if !accepted
        && (a.err() != Some(UnsupportedOperation::EncodePolicy)
            || b.err()
                .and_then(|e| e.downcast_ref::<UnsupportedOperation>().copied())
                != Some(UnsupportedOperation::EncodePolicy))
    {
        return Err(fail(CHECK, "rejection lost typed EncodePolicy error"));
    }
    let a = job
        .encode(img.as_slice())
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let b = dynamic
        .encode(img.as_slice())
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let (pa, ma) = dec_oneshot(&dec, a.data()).map_err(|e| fail(CHECK, e))?;
    let (pb, mb) = dec_oneshot(&dec, b.data()).map_err(|e| fail(CHECK, e))?;
    if pa != pb || ma != *expected || mb != *expected {
        return Err(fail(
            CHECK,
            format!("output mismatch: generic {ma:?}, erased {mb:?}, expected {expected:?}"),
        ));
    }
    Ok(())
}

/// Verify checked decode-policy admission, typed rejection, and unchanged output
/// after rejection. Supply explicit expected metadata to test supported metadata
/// denials. All paths decode the same caller-provided fixture.
#[allow(clippy::too_many_arguments)]
pub fn check_decode_policy_admission<D: DecoderConfig + 'static>(
    dec: D,
    bytes: &[u8],
    initial: DecodePolicy,
    request: DecodePolicy,
    accepted: bool,
    expected: &Metadata,
) -> Conformance {
    const CHECK: &str = "checked decode policy";
    let mut job = dec
        .clone()
        .job()
        .try_with_policy(initial)
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let mut dynamic = dec.dyn_job();
    dynamic
        .try_set_policy(initial)
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let a = job.try_set_policy(request);
    let b = dynamic.try_set_policy(request);
    if a.is_ok() != accepted || b.is_ok() != accepted {
        return Err(fail(CHECK, format!("admission mismatch: {a:?}, {b:?}")));
    }
    if !accepted
        && (a.err() != Some(UnsupportedOperation::DecodePolicy)
            || b.err()
                .and_then(|e| e.downcast_ref::<UnsupportedOperation>().copied())
                != Some(UnsupportedOperation::DecodePolicy))
    {
        return Err(fail(CHECK, "rejection lost typed DecodePolicy error"));
    }
    let a = job
        .decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| fail(CHECK, e.to_string()))?
        .decode()
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let b = dynamic
        .into_decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| fail(CHECK, e.to_string()))?
        .decode()
        .map_err(|e| fail(CHECK, e.to_string()))?;
    if grab(a.pixels()) != grab(b.pixels())
        || a.metadata() != *expected
        || b.metadata() != *expected
    {
        return Err(fail(
            CHECK,
            "decoded pixels/metadata disagree with expected policy outcome",
        ));
    }
    if !accepted {
        let baseline = dec
            .job()
            .try_with_policy(initial)
            .map_err(|e| fail(CHECK, e.to_string()))?
            .decoder(Cow::Borrowed(bytes), &[])
            .map_err(|e| fail(CHECK, e.to_string()))?
            .decode()
            .map_err(|e| fail(CHECK, e.to_string()))?;
        if grab(a.pixels()) != grab(baseline.pixels()) || a.metadata() != baseline.metadata() {
            return Err(fail(CHECK, "rejected policy changed output"));
        }
    }
    Ok(())
}

/// Exercise a codec's metadata-only rewrite entry point with caller-owned
/// fixtures. The observer must independently verify image payloads, timing,
/// rendering metadata and requested retention (prefer a third-party reader).
/// Checks plan materialization, report ranges, deterministic output, typed input,
/// output and carrier limits, and pre-cancellation. Services are retained.
pub fn check_metadata_rewrite<'a>(
    request: Request<'a>,
    rewrite: impl Fn(&Request<'a>) -> Result<Plan<'a>, RewriteError>,
    observe: impl Fn(&[u8]) -> Conformance,
) -> Conformance {
    const CHECK: &str = "metadata rewrite";
    let plan = rewrite(&request).map_err(|e| fail(CHECK, e.to_string()))?;
    let output = plan.to_vec().map_err(|e| fail(CHECK, e.to_string()))?;
    if output.is_empty() || request.input().is_empty() {
        return Err(fail(CHECK, "requires nonempty encoded input and output"));
    }
    if plan.output_len() != output.len()
        || plan.chunks().flatten().copied().collect::<Vec<_>>() != output
    {
        return Err(fail(CHECK, "chunk/materialized length disagreement"));
    }
    for entry in plan.report() {
        let r = entry.source_range();
        if r.start > r.end || r.end > request.input().len() {
            return Err(fail(CHECK, "report outside original input"));
        }
    }
    observe(&output)?;
    let again = rewrite(&request).map_err(|e| fail(CHECK, e.to_string()))?;
    if again.to_vec().map_err(|e| fail(CHECK, e.to_string()))? != output
        || again.report() != plan.report()
    {
        return Err(fail(CHECK, "identical request produced different plan"));
    }
    let exact = request
        .clone()
        .with_byte_limits(request.input().len(), output.len());
    if rewrite(&exact)
        .map_err(|e| fail(CHECK, format!("exact budget: {e}")))?
        .to_vec()
        .map_err(|e| fail(CHECK, e.to_string()))?
        != output
    {
        return Err(fail(CHECK, "exact budgets changed output"));
    }
    for (limited, expected) in [
        (
            request
                .clone()
                .with_byte_limits(request.input().len() - 1, usize::MAX),
            LimitKind::InputSize,
        ),
        (
            request
                .clone()
                .with_byte_limits(usize::MAX, output.len() - 1),
            LimitKind::OutputSize,
        ),
        (
            request.clone().with_carrier_limit(0),
            LimitKind::MetadataCarriers,
        ),
    ] {
        if !matches!(rewrite(&limited),Err(RewriteError::Limit(kind)) if kind==expected) {
            return Err(fail(CHECK, format!("missing typed {expected:?} limit")));
        }
    }
    struct Cancelled;
    impl enough::Stop for Cancelled {
        fn check(&self) -> Result<(), enough::StopReason> {
            Err(enough::StopReason::Cancelled)
        }
    }
    let cancelled = request.with_stop(&Cancelled);
    if !matches!(
        rewrite(&cancelled),
        Err(RewriteError::Cancelled) | Err(RewriteError::Stopped(enough::StopReason::Cancelled))
    ) {
        return Err(fail(CHECK, "pre-cancellation was not honored"));
    }
    Ok(())
}
