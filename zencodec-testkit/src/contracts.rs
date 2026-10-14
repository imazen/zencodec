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
    pixels: PixelSlice<'_>,
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
        if exact && expected.is_some_and(|v| v != *requested) {
            return Err(fail(CHECK, "exact test case requests rounding"));
        }
        let a = if exact {
            generic
                .push_frame_exact(pixels.sub_rows(0, pixels.rows()), *requested, None)
                .map(|()| *requested)
        } else {
            generic.push_frame_timed(pixels.sub_rows(0, pixels.rows()), *requested, None)
        };
        let b = if exact {
            erased
                .push_frame_exact(pixels.sub_rows(0, pixels.rows()), *requested, None)
                .map(|()| *requested)
        } else {
            erased.push_frame_timed(pixels.sub_rows(0, pixels.rows()), *requested, None)
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
    pixels: PixelSlice<'_>,
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
    dynamic.set_metadata_policy(metadata.clone(), MetadataPolicy::PreserveExact);
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
    let builder = enc
        .clone()
        .job()
        .with_metadata_policy(metadata.clone(), MetadataPolicy::PreserveExact)
        .try_with_policy(initial)
        .map_err(|e| fail(CHECK, e.to_string()))?
        .try_with_policy(request);
    if builder.is_ok() != accepted {
        return Err(fail(CHECK, "builder and mutable admission disagree"));
    }
    let baseline = match builder {
        Ok(job) => job,
        Err(UnsupportedOperation::EncodePolicy) => enc
            .job()
            .with_metadata_policy(metadata, MetadataPolicy::PreserveExact)
            .try_with_policy(initial)
            .map_err(|e| fail(CHECK, e.to_string()))?,
        Err(e) => return Err(fail(CHECK, format!("wrong builder rejection: {e}"))),
    }
    .encode(pixels.sub_rows(0, pixels.rows()))
    .map_err(|e| fail(CHECK, e.to_string()))?;
    let (baseline_pixels, baseline_metadata) =
        dec_oneshot(&dec, baseline.data()).map_err(|e| fail(CHECK, e))?;
    let a = job
        .encode(pixels.sub_rows(0, pixels.rows()))
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let b = dynamic
        .encode(pixels.sub_rows(0, pixels.rows()))
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let (pa, ma) = dec_oneshot(&dec, a.data()).map_err(|e| fail(CHECK, e))?;
    let (pb, mb) = dec_oneshot(&dec, b.data()).map_err(|e| fail(CHECK, e))?;
    if pa != pb
        || pa != baseline_pixels
        || ma != baseline_metadata
        || ma != *expected
        || mb != *expected
    {
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
    let builder = dec
        .clone()
        .job()
        .try_with_policy(initial)
        .map_err(|e| fail(CHECK, e.to_string()))?
        .try_with_policy(request);
    if builder.is_ok() != accepted {
        return Err(fail(CHECK, "builder and mutable admission disagree"));
    }
    let baseline = match builder {
        Ok(job) => job,
        Err(UnsupportedOperation::DecodePolicy) => dec
            .clone()
            .job()
            .try_with_policy(initial)
            .map_err(|e| fail(CHECK, e.to_string()))?,
        Err(e) => return Err(fail(CHECK, format!("wrong builder rejection: {e}"))),
    }
    .decoder(Cow::Borrowed(bytes), &[])
    .map_err(|e| fail(CHECK, e.to_string()))?
    .decode()
    .map_err(|e| fail(CHECK, e.to_string()))?;
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
    if grab(a.pixels()) != grab(baseline.pixels()) || a.metadata() != baseline.metadata() {
        return Err(fail(
            CHECK,
            "builder/mutable output differs or rejection changed output",
        ));
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

/// Verify the conservative compatibility default for an animation adapter that
/// has not adopted rational encoding. Both timing methods must reject with
/// `AnimationTiming`, leave buffered frames intact, and allow legacy encoding
/// to continue. A false timing capability must not silently quantize requests.
pub fn check_animation_timing_unsupported<E, D>(
    enc: E,
    dec: D,
    pixels: PixelSlice<'_>,
) -> Conformance
where
    E: EncoderConfig + 'static,
    D: DecoderConfig + 'static,
    <E::Job as EncodeJob>::Enc: Encoder + Send,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    const CHECK: &str = "unsupported animation timing";
    if E::capabilities().animation_timing() {
        return Err(fail(
            CHECK,
            "timing-enabled adapter needs explicit timing cases",
        ));
    }
    let mut generic = enc
        .clone()
        .job()
        .animation_frame_encoder()
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let mut dynamic = enc
        .dyn_job()
        .into_animation_frame_encoder()
        .map_err(|e| fail(CHECK, e.to_string()))?;
    for delay in [40, 60] {
        for duration in [
            FrameDuration::from_millis(0),
            FrameDuration::from_millis(10),
            FrameDuration::new(1, 30000).unwrap(),
            FrameDuration::new(u64::MAX, 1).unwrap(),
        ] {
            for exact in [true, false] {
                let a = if exact {
                    generic.push_frame_exact(pixels.sub_rows(0, pixels.rows()), duration, None)
                } else {
                    generic
                        .push_frame_timed(pixels.sub_rows(0, pixels.rows()), duration, None)
                        .map(|_| ())
                };
                let b = if exact {
                    dynamic.push_frame_exact(pixels.sub_rows(0, pixels.rows()), duration, None)
                } else {
                    dynamic
                        .push_frame_timed(pixels.sub_rows(0, pixels.rows()), duration, None)
                        .map(|_| ())
                };
                let a = a
                    .err()
                    .ok_or_else(|| fail(CHECK, "generic accepted unsupported timing"))?;
                let b = b
                    .err()
                    .ok_or_else(|| fail(CHECK, "erased accepted unsupported timing"))?;
                if a.unsupported_operation() != Some(&UnsupportedOperation::AnimationTiming)
                    || b.unsupported_operation() != Some(&UnsupportedOperation::AnimationTiming)
                {
                    return Err(fail(CHECK, format!("wrong rejection: {a}, {b}")));
                }
            }
        }
        generic
            .push_frame(pixels.sub_rows(0, pixels.rows()), delay, None)
            .map_err(|e| fail(CHECK, e.to_string()))?;
        dynamic
            .push_frame(pixels.sub_rows(0, pixels.rows()), delay, None)
            .map_err(|e| fail(CHECK, e.to_string()))?;
    }
    let a = generic
        .finish(None)
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let b = dynamic
        .finish(None)
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let expected = [
        FrameDuration::from_millis(40),
        FrameDuration::from_millis(60),
    ];
    check_animation_durations(dec.clone(), a.data(), &expected)?;
    check_animation_durations(dec, b.data(), &expected)
}

/// Verify a [`DynReadAt`](zencodec::source::DynReadAt) driver source
/// against the bytes it claims to serve: length, exact reads at the
/// edges and middle, one batched out-of-order multi-range read, and an
/// error (never a short or zero-filled read) past the end.
pub fn check_read_at_contract(
    source: &mut dyn zencodec::source::DynReadAt,
    expected: &[u8],
) -> Conformance {
    const CHECK: &str = "read_at_contract";
    let err = |e: &dyn std::fmt::Display| fail(CHECK, e.to_string());

    let total = source.len().map_err(|e| err(&e))?;
    if total != expected.len() as u64 {
        return Err(fail(
            CHECK,
            format!("len() = {total}, expected {}", expected.len()),
        ));
    }
    if expected.is_empty() {
        return Ok(());
    }
    let mut all = vec![0u8; expected.len()];
    source.read_exact_at(0, &mut all).map_err(|e| err(&e))?;
    if all != expected {
        return Err(fail(CHECK, "full read bytes differ"));
    }
    let mid = expected.len() / 2;
    let take = (expected.len() - mid).min(16);
    let mut slice = vec![0u8; take];
    source
        .read_exact_at(mid as u64, &mut slice)
        .map_err(|e| err(&e))?;
    if slice != expected[mid..mid + take] {
        return Err(fail(CHECK, "middle read bytes differ"));
    }
    let mut head = vec![0u8; take];
    let mut tail = vec![0u8; take];
    {
        let mut reqs: Vec<(u64, &mut [u8])> = vec![
            ((expected.len() - take) as u64, &mut tail[..]),
            (0, &mut head[..]),
        ];
        source.read_many_at(&mut reqs).map_err(|e| err(&e))?;
    }
    if head != expected[..take] || tail != expected[expected.len() - take..] {
        return Err(fail(CHECK, "batched read bytes differ"));
    }
    let mut one = [0u8; 1];
    if source
        .read_exact_at(expected.len() as u64, &mut one)
        .is_ok()
    {
        return Err(fail(CHECK, "read past end unexpectedly succeeded"));
    }
    Ok(())
}

fn pixels_of(out: &zencodec::decode::DecodeOutput) -> (u32, u32, PixelDescriptor, Vec<u8>) {
    let ps = out.pixels();
    let mut bytes = Vec::new();
    for y in 0..ps.rows() {
        bytes.extend_from_slice(ps.row(y));
    }
    (ps.width(), ps.rows(), ps.descriptor(), bytes)
}

/// Verify the `seekable` capability flag in both directions. A decoder
/// declaring it must decode `data` through `push_decoder_from_source`
/// over a slice source with the same pixels as the one-shot path; one not
/// declaring it must reject with [`UnsupportedOperation::SeekableDecode`].
pub fn check_seekable_capability_honesty<D: DecoderConfig>(dec: D, data: &[u8]) -> Conformance {
    const CHECK: &str = "seekable_capability_honesty";
    let declared = D::capabilities().seekable();
    let mut source = zencodec::source::ReadAtShim(zencodec::source::SliceReadAt::new(data));
    let mut sink = zencodec_io::CollectSink::new();
    let result = dec
        .clone()
        .job()
        .push_decoder_from_source(&mut source, &mut sink, &[]);
    match (declared, result) {
        (true, Ok(info)) => {
            let reference = dec
                .job()
                .decoder(std::borrow::Cow::Borrowed(data), &[])
                .and_then(|d| d.decode())
                .map_err(|e| fail(CHECK, format!("one-shot decode: {e}")))?;
            let (w, h, desc, bytes) = pixels_of(&reference);
            if (info.width, info.height) != (w, h) || sink.descriptor() != Some(desc) {
                return Err(fail(
                    CHECK,
                    format!(
                        "source decode {}x{} {:?} differs from one-shot {w}x{h} {desc:?}",
                        info.width,
                        info.height,
                        sink.descriptor()
                    ),
                ));
            }
            if !sink.finished() {
                return Err(fail(CHECK, "source decode did not call sink.finish"));
            }
            if sink.bytes() != bytes {
                return Err(fail(CHECK, "source decode pixels differ from one-shot"));
            }
            Ok(())
        }
        (true, Err(e)) => Err(fail(
            CHECK,
            format!("declared seekable but push_decoder_from_source failed: {e}"),
        )),
        (false, Ok(_)) => Err(fail(
            CHECK,
            "did not declare seekable but push_decoder_from_source succeeded",
        )),
        (false, Err(e)) => match e.downcast_ref::<UnsupportedOperation>().copied() {
            Some(UnsupportedOperation::SeekableDecode) => Ok(()),
            _ => Err(fail(
                CHECK,
                format!("undeclared seekable rejected with wrong error: {e}"),
            )),
        },
    }
}

/// Verify the `incremental` capability flag in both directions, then run
/// [`check_incremental_contract`] on a declared incremental decoder.
pub fn check_incremental_capability_honesty<D: DecoderConfig>(dec: D, data: &[u8]) -> Conformance {
    const CHECK: &str = "incremental_capability_honesty";
    let declared = D::capabilities().incremental();
    match (declared, dec.clone().job().dyn_incremental_decoder(&[])) {
        (true, Ok(_)) => check_incremental_contract(dec, data),
        (true, Err(e)) => Err(fail(
            CHECK,
            format!("declared incremental but dyn_incremental_decoder failed: {e}"),
        )),
        (false, Ok(_)) => Err(fail(
            CHECK,
            "did not declare incremental but dyn_incremental_decoder succeeded",
        )),
        (false, Err(e)) => match e.downcast_ref::<UnsupportedOperation>().copied() {
            Some(UnsupportedOperation::IncrementalDecode) => Ok(()),
            _ => Err(fail(
                CHECK,
                format!("undeclared incremental rejected with wrong error: {e}"),
            )),
        },
    }
}

/// A sink that records every row write, for ordering checks.
struct OrderSink {
    inner: zencodec_io::CollectSink,
    final_ys: Vec<(u32, u32)>,
    began: u32,
}

impl zencodec::decode::DecodeRowSink for OrderSink {
    fn begin(
        &mut self,
        w: u32,
        h: u32,
        d: PixelDescriptor,
    ) -> Result<(), zencodec::decode::SinkError> {
        self.began += 1;
        self.inner.begin(w, h, d)
    }
    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<zenpixels::PixelSliceMut<'_>, zencodec::decode::SinkError> {
        self.final_ys.push((y, height));
        self.inner.provide_next_buffer(y, height, width, descriptor)
    }
    fn finish(&mut self) -> Result<(), zencodec::decode::SinkError> {
        self.inner.finish()
    }
}

/// A sink that accepts provisional rows and records every batch (tightly
/// packed copies of the rows it was given), final or provisional.
#[derive(Default)]
struct PassSink {
    batches: Vec<(zencodec::decode::BatchMeta, u32, Vec<u8>)>,
    current: Option<(zencodec::decode::BatchMeta, zenpixels::PixelBuffer)>,
    final_through_pass: u32,
}

impl PassSink {
    fn ship(&mut self) {
        if let Some((meta, buf)) = self.current.take() {
            let s = buf.as_slice();
            let mut packed = Vec::new();
            for r in 0..s.rows() {
                packed.extend_from_slice(s.row(r));
            }
            self.batches.push((meta, s.rows(), packed));
        }
    }
}

impl zencodec::decode::DecodeRowSink for PassSink {
    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<zenpixels::PixelSliceMut<'_>, zencodec::decode::SinkError> {
        self.ship();
        let (_, buf) = self.current.insert((
            zencodec::decode::BatchMeta::final_rows(y),
            zenpixels::PixelBuffer::new(width, height, descriptor),
        ));
        Ok(buf.as_slice_mut())
    }
    fn accepts_provisional(&self) -> bool {
        true
    }
    fn provide_pass_buffer(
        &mut self,
        meta: zencodec::decode::BatchMeta,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<zenpixels::PixelSliceMut<'_>, zencodec::decode::SinkError> {
        if meta.is_final {
            self.final_through_pass += 1;
        }
        self.ship();
        let (_, buf) = self
            .current
            .insert((meta, zenpixels::PixelBuffer::new(width, height, descriptor)));
        Ok(buf.as_slice_mut())
    }
    fn finish(&mut self) -> Result<(), zencodec::decode::SinkError> {
        self.ship();
        Ok(())
    }
}

/// A stop that trips after `n` checks.
struct TripAfter {
    left: std::sync::atomic::AtomicU32,
}

impl enough::Stop for TripAfter {
    fn check(&self) -> Result<(), enough::StopReason> {
        use std::sync::atomic::Ordering;
        // Single-threaded use: a load and a store are enough.
        let left = self.left.load(Ordering::SeqCst);
        if left == 0 {
            return Err(enough::StopReason::Cancelled);
        }
        self.left.store(left - 1, Ordering::SeqCst);
        Ok(())
    }
}

/// Exercise a decoder's [`IncrementalDecode`](zencodec::decode::IncrementalDecode)
/// contract through the erased decoder callers receive
/// (`dyn_incremental_decoder`), against the one-shot pixels of `data`:
///
/// - driven from a slice with windows of 1, 7 and 64 bytes and the whole
///   file, the pixels, descriptor and `Done` dimensions match one-shot, the
///   sink is begun once and finished, and rows reach a final-only sink each
///   exactly once in increasing order;
/// - an input at an offset the decoder did not request is ignored without
///   error (nothing taken, still asking);
/// - after `Done`, further steps return `Done` again and take nothing;
/// - a stop that trips mid-decode yields `Cancelled` within one step, then
///   `Finished`;
/// - a sink that accepts provisional rows still gets every final row once
///   through `provide_next_buffer` (never through `provide_pass_buffer`),
///   reproducing the one-shot pixels;
/// - truncated input (cut at a quarter and at half) ends in a codec error,
///   never a protocol violation;
/// - no drive exceeds a step budget, so a decoder that spins fails here
///   instead of hanging.
pub fn check_incremental_contract<D: DecoderConfig>(dec: D, data: &[u8]) -> Conformance {
    const CHECK: &str = "incremental_contract";
    let reference = dec
        .clone()
        .job()
        .decoder(std::borrow::Cow::Borrowed(data), &[])
        .and_then(|d| d.decode())
        .map_err(|e| fail(CHECK, format!("one-shot decode: {e}")))?;
    check_incremental_decoder(
        || {
            dec.clone()
                .job()
                .dyn_incremental_decoder(&[])
                .map_err(|e| e.to_string())
        },
        data,
        &reference,
    )
}

/// [`check_incremental_contract`] for any decoder factory, against the
/// one-shot `reference` output of `data`: for incremental decoders not
/// reached through a codec's `DecoderConfig`, such as one built with
/// `zencodec::helpers::WholeFileIncremental`.
pub fn check_incremental_decoder<F>(
    new_decoder: F,
    data: &[u8],
    reference: &zencodec::decode::DecodeOutput,
) -> Conformance
where
    F: Fn() -> Result<Box<zencodec::decode::DynIncrementalDecoder>, String>,
{
    use zencodec::decode::{Event, IncrementalDecode, Input, StepError};
    const CHECK: &str = "incremental_contract";
    let new_dec = || new_decoder().map_err(|e| fail(CHECK, format!("new decoder: {e}")));
    let (w, h, desc, bytes) = pixels_of(reference);
    let budget = 4 * data.len() as u64 + 4 * h as u64 + 1000;

    for window in [1usize, 7, 64, data.len().max(1)] {
        let mut inc = new_dec()?;
        let mut sink = OrderSink {
            inner: zencodec_io::CollectSink::new(),
            final_ys: Vec::new(),
            began: 0,
        };
        let (info, stats) = zencodec_io::drive_from_slice(&mut *inc, data, &mut sink, None, window)
            .map_err(|e| fail(CHECK, format!("window {window}: {e}")))?;
        if stats.steps > budget {
            return Err(fail(
                CHECK,
                format!(
                    "window {window}: {} steps for {} bytes",
                    stats.steps,
                    data.len()
                ),
            ));
        }
        if (info.width, info.height) != (w, h) {
            return Err(fail(
                CHECK,
                format!(
                    "window {window}: Done reports {}x{}, one-shot is {w}x{h}",
                    info.width, info.height
                ),
            ));
        }
        if sink.inner.descriptor() != Some(desc) {
            return Err(fail(
                CHECK,
                format!(
                    "window {window}: sink began with {:?}, one-shot is {desc:?}",
                    sink.inner.descriptor()
                ),
            ));
        }
        if sink.began != 1 || !sink.inner.finished() {
            return Err(fail(
                CHECK,
                format!(
                    "window {window}: begin called {} times, finish {}",
                    sink.began,
                    sink.inner.finished()
                ),
            ));
        }
        let mut next = 0u32;
        for &(y, rows) in &sink.final_ys {
            if y != next {
                return Err(fail(
                    CHECK,
                    format!(
                        "window {window}: final rows out of order (got y={y}, expected {next})"
                    ),
                ));
            }
            next = y + rows;
        }
        if next != h {
            return Err(fail(
                CHECK,
                format!("window {window}: {next} of {h} rows written"),
            ));
        }
        if sink.inner.bytes() != bytes {
            return Err(fail(
                CHECK,
                format!("window {window}: pixels differ from one-shot"),
            ));
        }
        if inc.info().map(|i| (i.width, i.height)).is_none() {
            return Err(fail(
                CHECK,
                format!("window {window}: info() is None after Done"),
            ));
        }
        // Done repeats and takes nothing.
        let again = inc
            .step(Input::new(0, data), &mut sink, &enough::Unstoppable)
            .map_err(|e| fail(CHECK, format!("step after Done: {e}")))?;
        if !again.event.is_done() || again.consumed != 0 {
            return Err(fail(CHECK, format!("step after Done returned {again:?}")));
        }
    }

    // Unrequested input is ignored, not an error.
    {
        let mut inc = new_dec()?;
        let mut sink = zencodec_io::CollectSink::new();
        let p = inc
            .step(Input::empty(), &mut sink, &enough::Unstoppable)
            .map_err(|e| fail(CHECK, format!("first step: {e}")))?;
        if p.event == Event::Need {
            let wanted: Vec<zencodec::decode::Request> = inc.requested().to_vec();
            let first = wanted.first().and_then(|r| r.offset()).unwrap_or(0);
            let bogus = first.saturating_add(1).min(data.len() as u64);
            let tail = &data[bogus as usize..];
            let p = inc
                .step(Input::new(bogus, tail), &mut sink, &enough::Unstoppable)
                .map_err(|e| fail(CHECK, format!("unrequested input at {bogus} failed: {e}")))?;
            if p.consumed != 0 || p.event != Event::Need {
                return Err(fail(
                    CHECK,
                    format!("unrequested input at {bogus} was used: {p:?}"),
                ));
            }
        }
    }

    // Cancellation is terminal.
    {
        let mut inc = new_dec()?;
        let mut sink = zencodec_io::CollectSink::new();
        let stop = TripAfter {
            left: std::sync::atomic::AtomicU32::new(2),
        };
        let mut input = Input::empty().with_total_len(data.len() as u64);
        let mut saw_cancel = false;
        for step in 0..budget {
            match inc.step(input, &mut sink, &stop) {
                Ok(p) => {
                    if p.event.is_done() {
                        break;
                    }
                    // The stop allows two checks; checked at least once per
                    // step, the third step cannot succeed.
                    if step >= 2 {
                        return Err(fail(
                            CHECK,
                            format!("step {step} succeeded after the stop tripped: {p:?}"),
                        ));
                    }
                    let next = match inc.requested().first() {
                        Some(r) if p.event == Event::Need => r.offset().unwrap_or(0),
                        _ => input.offset + p.consumed as u64,
                    };
                    let start = (next as usize).min(data.len());
                    input =
                        Input::new(start as u64, &data[start..]).with_total_len(data.len() as u64);
                }
                Err(StepError::Cancelled(_)) => {
                    saw_cancel = true;
                    break;
                }
                Err(e) => return Err(fail(CHECK, format!("cancelling: unexpected {e}"))),
            }
        }
        if saw_cancel {
            match inc.step(input, &mut sink, &enough::Unstoppable) {
                Err(StepError::Finished) => {}
                other => {
                    return Err(fail(
                        CHECK,
                        format!("step after cancellation returned {other:?}, not Finished"),
                    ));
                }
            }
        }
    }

    // A sink that accepts provisional rows still receives final rows through
    // provide_next_buffer; they cover the image once with the one-shot
    // pixels.
    {
        let mut inc = new_dec()?;
        let mut sink = PassSink::default();
        zencodec_io::drive_from_slice(&mut *inc, data, &mut sink, None, 64)
            .map_err(|e| fail(CHECK, format!("provisional sink: {e}")))?;
        if sink.final_through_pass != 0 {
            return Err(fail(
                CHECK,
                format!(
                    "provisional sink: {} final batches came through provide_pass_buffer",
                    sink.final_through_pass
                ),
            ));
        }
        let row = bytes.len() / h.max(1) as usize;
        let mut covered = vec![false; h as usize];
        let mut out = vec![0u8; bytes.len()];
        for (meta, rows, px) in &sink.batches {
            if !meta.is_final {
                continue;
            }
            for r in 0..*rows {
                let y = (meta.y + r) as usize;
                if y >= covered.len() || covered[y] {
                    return Err(fail(
                        CHECK,
                        format!("provisional sink: final row {y} written twice or out of range"),
                    ));
                }
                covered[y] = true;
                out[y * row..(y + 1) * row]
                    .copy_from_slice(&px[r as usize * row..(r as usize + 1) * row]);
            }
        }
        if covered.iter().any(|c| !c) || out != bytes {
            return Err(fail(
                CHECK,
                "provisional sink: final batches do not reproduce the one-shot pixels",
            ));
        }
    }

    // Truncation is a codec error, never a protocol violation or a hang.
    for cut in [data.len() / 4, data.len() / 2] {
        if cut >= data.len() {
            continue;
        }
        let mut inc = new_dec()?;
        let mut sink = zencodec_io::CollectSink::new();
        match zencodec_io::drive_from_slice(&mut *inc, &data[..cut], &mut sink, None, 64) {
            Ok(_) => {
                return Err(fail(
                    CHECK,
                    format!(
                        "truncated to {cut} of {} bytes decoded without error",
                        data.len()
                    ),
                ));
            }
            Err(zencodec_io::DriveError::Step(StepError::Codec(_))) => {}
            Err(e) => {
                return Err(fail(
                    CHECK,
                    format!("truncated to {cut}: expected a codec error, got: {e}"),
                ));
            }
        }
    }
    Ok(())
}
