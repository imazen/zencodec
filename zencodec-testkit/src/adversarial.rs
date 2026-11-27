//! Explicit acceptance oracles. These checks never infer correctness from file
//! length, capability flags, or agreement between two implementations alone.
use super::*;
use zencodec::animation::FrameDuration;
use zencodec::encode::DynEncoderConfig;

/// A specific error that constitutes an expected refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedError {
    Unsupported(UnsupportedOperation),
    Category(ErrorCategory),
}
impl ExpectedError {
    /// Match through the error chain, including erased errors. Unknown causes
    /// fail rather than satisfying an arbitrary expected rejection.
    pub fn matches(self, error: &(dyn std::error::Error + 'static)) -> bool {
        match self {
            Self::Unsupported(op) => error.unsupported_operation() == Some(&op),
            Self::Category(category) => error.error_category() == Some(category),
        }
    }
}

/// Caller-specified expected result at a structural prefix boundary.
pub enum PrefixOutcome<'a> {
    /// Complete/recoverable payload, with an independently specified pixel oracle.
    Accept(PixelView<'a>),
    Reject(ExpectedError),
}

/// A meaningful prefix boundary, not a fraction of the total file length.
pub struct PrefixCase<'a> {
    pub end: usize,
    pub outcome: PrefixOutcome<'a>,
}

/// Exercise explicit complete-payload/truncation boundaries through one-shot and
/// sink dispatch. Successful recovery must produce the expected pixels. The caller
/// supplies strict/compatibility configuration separately. No prefix is silently
/// omitted; out-of-range fixture boundaries are errors.
pub fn check_decode_prefix_cases<D: DecoderConfig + 'static>(
    decoder: D,
    bytes: &[u8],
    cases: &[PrefixCase<'_>],
) -> Conformance {
    const CHECK: &str = "structural decode prefixes";
    if cases.is_empty() {
        return Err(fail(CHECK, "supply explicit prefix cases"));
    }
    for case in cases {
        let input = bytes
            .get(..case.end)
            .ok_or_else(|| fail(CHECK, "prefix exceeds fixture"))?;
        for sink_path in [false, true] {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let job = decoder.dyn_job();
                if sink_path {
                    let mut sink = CollectSink::default();
                    job.push_decode(Cow::Borrowed(input), &mut sink, &[])?;
                    Ok(sink.into_pixels().map_err(|e| fail(CHECK, e)))
                } else {
                    let output = job.into_decoder(Cow::Borrowed(input), &[])?.decode()?;
                    Ok(Ok(grab(output.pixels())))
                }
            }));
            let result: Result<Result<Pixels, Failure>, zencodec::decode::BoxedError> = result
                .map_err(|_| fail(CHECK, format!("panic at {} (sink={sink_path})", case.end)))?;
            match (&case.outcome, result) {
                (PrefixOutcome::Accept(expected), Ok(pixels)) => {
                    if pixels? != grab_ref(expected) {
                        return Err(fail(CHECK, format!("wrong pixels at {}", case.end)));
                    }
                }
                (PrefixOutcome::Reject(expected), Err(error))
                    if expected.matches(error.as_ref()) => {}
                (_, result) => {
                    return Err(fail(
                        CHECK,
                        format!(
                            "unexpected result at {} (sink={sink_path}): {result:?}",
                            case.end
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// One frame submission and its independent encoded result. Accepted frames have
/// an expected native decode representation; this can differ from input for an
/// explicitly lossy encode. Use distinct pixels to expose frame reordering.
pub struct TimingCase<'a> {
    pub pixels: PixelView<'a>,
    pub requested: FrameDuration,
    pub expected: Result<(FrameDuration, PixelView<'a>), ExpectedError>,
}

/// Strict counterpart to `check_animation_timing`: requires typed errors and
/// verifies independently specified frame pixels, as well as duration and index.
/// Accepted frames on both sides of a rejection expose timing/state corruption.
pub fn check_animation_cases<E, D>(
    enc: E,
    dec: D,
    exact: bool,
    cases: &[TimingCase<'_>],
) -> Conformance
where
    E: EncoderConfig + 'static,
    D: DecoderConfig + 'static,
    <E::Job as EncodeJob>::Enc: Encoder + Send,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    const CHECK: &str = "explicit animation cases";
    let accepted: Vec<_> = cases
        .iter()
        .filter_map(|c| c.expected.as_ref().ok())
        .collect();
    if accepted.len() < 2 {
        return Err(fail(CHECK, "supply at least two accepted frames"));
    }
    // Both paths are erased here; the blanket shim invokes the generic primitive.
    // Existing check_animation_timing separately verifies generic/erased parity.
    let mut encoder = enc
        .dyn_job()
        .into_animation_frame_encoder()
        .map_err(|e| fail(CHECK, e.to_string()))?;
    for (index, case) in cases.iter().enumerate() {
        if exact
            && case
                .expected
                .as_ref()
                .is_ok_and(|(duration, _)| *duration != case.requested)
        {
            return Err(fail(CHECK, "exact fixture expects rounding"));
        }
        let pixels = case.pixels.clone();
        let result = if exact {
            encoder
                .push_frame_exact(pixels, case.requested, None)
                .map(|()| case.requested)
        } else {
            encoder.push_frame_timed(pixels, case.requested, None)
        };
        match (&case.expected, result) {
            (Ok((duration, _)), Ok(actual)) if *duration == actual => (),
            (Err(expected), Err(error)) if expected.matches(error.as_ref()) => (),
            (_, result) => {
                return Err(fail(
                    CHECK,
                    format!("frame {index}: unexpected admission {result:?}"),
                ));
            }
        }
    }
    let output = encoder
        .finish(None)
        .map_err(|e| fail(CHECK, e.to_string()))?;
    let durations: Vec<_> = accepted.iter().map(|(duration, _)| *duration).collect();
    check_animation_durations(dec.clone(), output.data(), &durations)?;
    let mut decoder = dec
        .dyn_job()
        .into_animation_frame_decoder(Cow::Borrowed(output.data()), &[])
        .map_err(|e| fail(CHECK, e.to_string()))?;
    for (index, (_, expected)) in accepted.iter().enumerate() {
        let frame = decoder
            .render_next_frame_owned(None)
            .map_err(|e| fail(CHECK, e.to_string()))?
            .ok_or_else(|| fail(CHECK, "missing frame"))?;
        if grab(frame.pixels()) != grab_ref(expected) {
            return Err(fail(
                CHECK,
                format!("frame {index}: pixels differ from independent oracle"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_rejection_does_not_accept_unrelated_errors() {
        let expected = ExpectedError::Unsupported(UnsupportedOperation::AnimationTiming);
        assert!(expected.matches(&UnsupportedOperation::AnimationTiming));
        assert!(!expected.matches(&UnsupportedOperation::AnimationEncode));
        assert!(!expected.matches(&std::io::Error::other("internal failure")));
    }

    #[test]
    fn distinct_frame_oracles_detect_reordering_and_wrong_pixels() {
        let a = TestImage::rgba8_gradient_seeded(4, 3, 0);
        let b = TestImage::rgba8_gradient_seeded(4, 3, 37);
        let ms = FrameDuration::from_millis;
        let cases = [
            TimingCase {
                pixels: a.as_slice(),
                requested: ms(40),
                expected: Ok((ms(40), a.as_slice())),
            },
            TimingCase {
                pixels: b.as_slice(),
                requested: ms(60),
                expected: Ok((ms(60), b.as_slice())),
            },
        ];
        check_animation_cases(
            ReferenceEncoderConfig::new(),
            ReferenceDecoderConfig,
            true,
            &cases,
        )
        .unwrap();
        let wrong = [
            TimingCase {
                pixels: a.as_slice(),
                requested: ms(40),
                expected: Ok((ms(40), b.as_slice())),
            },
            TimingCase {
                pixels: b.as_slice(),
                requested: ms(60),
                expected: Ok((ms(60), a.as_slice())),
            },
        ];
        assert!(
            check_animation_cases(
                ReferenceEncoderConfig::new(),
                ReferenceDecoderConfig,
                true,
                &wrong
            )
            .is_err()
        );
    }

    #[test]
    fn structural_prefixes_allow_complete_payload_before_a_large_trailer() {
        let image = TestImage::rgb8_gradient(3, 2);
        let bytes = MinimalEncoderConfig::default()
            .encode(image.as_slice())
            .unwrap();
        let complete = bytes.data().len();
        let mut trailing = bytes.data().to_vec();
        trailing.resize(complete * 8, 0);
        let cases = [
            PrefixCase {
                end: 0,
                outcome: PrefixOutcome::Reject(ExpectedError::Category(ErrorCategory::Image(
                    zencodec::ImageError::Malformed,
                ))),
            },
            PrefixCase {
                end: complete,
                outcome: PrefixOutcome::Accept(image.as_slice()),
            },
        ];
        check_decode_prefix_cases(MinimalDecoderConfig::default(), &trailing, &cases).unwrap();
        // No file-length heuristic: required payload loss is rejected even near EOF.
        let cases = [PrefixCase {
            end: complete - 1,
            outcome: PrefixOutcome::Reject(ExpectedError::Category(ErrorCategory::Image(
                zencodec::ImageError::Malformed,
            ))),
        }];
        check_decode_prefix_cases(MinimalDecoderConfig::default(), bytes.data(), &cases).unwrap();
        let wrong = TestImage::rgb8_gradient(4, 2);
        let cases = [PrefixCase {
            end: complete,
            outcome: PrefixOutcome::Accept(wrong.as_slice()),
        }];
        assert!(
            check_decode_prefix_cases(MinimalDecoderConfig::default(), bytes.data(), &cases)
                .is_err()
        );
        let cases = [PrefixCase {
            end: complete - 1,
            outcome: PrefixOutcome::Accept(image.as_slice()),
        }];
        assert!(
            check_decode_prefix_cases(MinimalDecoderConfig::default(), bytes.data(), &cases)
                .is_err()
        );
    }
}

/// Run ranked decode preferences against independently supplied pixels.
///
/// The expected view includes the actual output spec and its alpha samples.
pub fn check_decode_preferences<D: DecoderConfig + 'static>(
    decoder: D,
    bytes: &[u8],
    preferred: &[PixelFormat],
    expected: PixelView<'_>,
) -> Conformance {
    const CHECK: &str = "ranked decode preferences";
    if preferred.is_empty() {
        return Err(fail(CHECK, "supply a nonempty ordered preference list"));
    }
    let want = grab_ref(&expected);
    let verify = |route, actual| {
        if actual == want {
            Ok(())
        } else {
            Err(fail(
                CHECK,
                format!("{route} selected different pixels or interpretation"),
            ))
        }
    };
    let typed = decoder
        .clone()
        .job()
        .decoder(Cow::Borrowed(bytes), preferred)
        .and_then(|decode| decode.decode())
        .map_err(|error| fail(CHECK, error.to_string()))?;
    verify("typed decode", grab(typed.pixels()))?;
    let erased = decoder
        .dyn_job()
        .into_decoder(Cow::Borrowed(bytes), preferred)
        .and_then(|decode| decode.decode())
        .map_err(|error| fail(CHECK, error.to_string()))?;
    verify("erased decode", grab(erased.pixels()))?;

    for erased in [false, true] {
        let mut sink = CollectSink::default();
        let result = if erased {
            decoder
                .dyn_job()
                .push_decode(Cow::Borrowed(bytes), &mut sink, preferred)
                .map_err(|error| error.to_string())
        } else {
            decoder
                .clone()
                .job()
                .push_decoder(Cow::Borrowed(bytes), &mut sink, preferred)
                .map_err(|error| error.to_string())
        };
        let completion = result.map_err(|error| fail(CHECK, error))?;
        sink.verify_completion(&completion)
            .map_err(|error| fail(CHECK, error))?;
        verify(
            "push decode",
            sink.into_pixels().map_err(|error| fail(CHECK, error))?,
        )?;
    }

    if D::capabilities().streaming() {
        let mut typed = decoder
            .clone()
            .job()
            .streaming_decoder(Cow::Borrowed(bytes), preferred)
            .map_err(|error| fail(CHECK, error.to_string()))?;
        let mut erased = decoder
            .dyn_job()
            .into_streaming_decoder(Cow::Borrowed(bytes), preferred)
            .map_err(|error| fail(CHECK, error.to_string()))?;
        for erase in [false, true] {
            let mut next = 0u32;
            loop {
                let batch = if erase {
                    erased
                        .next_batch()
                        .map_err(|error| fail(CHECK, error.to_string()))?
                } else {
                    typed
                        .next_batch()
                        .map_err(|error| fail(CHECK, error.to_string()))?
                };
                let Some((y, rows)) = batch else { break };
                if y != next || rows.height() == 0 {
                    return Err(fail(CHECK, "unexpected pull coordinates"));
                }
                let span = expected
                    .sub_rows(y, rows.height())
                    .map_err(|error| fail(CHECK, error.to_string()))?;
                if grab_ref(&rows) != grab_ref(&span) {
                    return Err(fail(
                        CHECK,
                        format!("pull pixels or spec differ from reference at row {y}"),
                    ));
                }
                next += rows.height();
            }
            if next != expected.height() {
                return Err(fail(CHECK, "pull decode ended with incomplete rows"));
            }
        }
    }
    Ok(())
}

/// Independently known display pixels and timing for one animation frame.
pub struct DisplayFrameCase<'a> {
    /// Exact active pixels, interpretation and provenance.
    pub pixels: PixelView<'a>,
    /// Source duration and optional PTS, with the zero-based display index.
    pub timing: zencodec::decode::FrameCompletion,
}

/// Check typed and erased animation, including corrected display orientation,
/// ranked native format selection, independent timing and exact sink effects.
pub fn check_animation_delivery<D: DecoderConfig + 'static>(
    decoder: D,
    bytes: &[u8],
    preferred: &[PixelFormat],
    orientation: zencodec::OrientationHint,
    expected_loop_count: Option<u32>,
    cases: &[DisplayFrameCase<'_>],
) -> Conformance {
    use zencodec::decode::RowMetadata;
    const CHECK: &str = "explicit animation delivery";
    if cases.is_empty() {
        return Err(fail(CHECK, "supply at least one displayed frame"));
    }
    let wrap = |error: String| fail(CHECK, error);
    for mode in 0..6 {
        let erase = mode >= 3;
        let route = mode % 3;
        let mut job = decoder.dyn_job();
        job.set_orientation(orientation);
        let mut erased = if erase {
            Some(
                job.into_animation_frame_decoder(Cow::Borrowed(bytes), preferred)
                    .map_err(|error| wrap(error.to_string()))?,
            )
        } else {
            None
        };
        let mut typed = if erase {
            None
        } else {
            Some(
                decoder
                    .clone()
                    .job()
                    .with_orientation(orientation)
                    .animation_frame_decoder(Cow::Borrowed(bytes), preferred)
                    .map_err(|error| wrap(error.to_string()))?,
            )
        };
        let (count, loops) = if let Some(decoder) = &erased {
            (decoder.frame_count(), decoder.loop_count())
        } else {
            let decoder = typed.as_ref().unwrap();
            (decoder.frame_count(), decoder.loop_count())
        };
        if count.is_some_and(|n| n as usize != cases.len()) || loops != expected_loop_count {
            return Err(fail(
                CHECK,
                format!("route {mode} reported frame count {count:?}, loop count {loops:?}"),
            ));
        }
        for index in 0..=cases.len() {
            let expected = cases.get(index);
            let mut sink = CollectSink::default();
            let observed = match route {
                0 => {
                    let frame = if let Some(decoder) = &mut erased {
                        decoder
                            .render_next_frame(None)
                            .map_err(|error| error.to_string())
                    } else {
                        typed
                            .as_mut()
                            .unwrap()
                            .render_next_frame(None)
                            .map_err(|error| error.to_string())
                    }
                    .map_err(wrap)?;
                    frame.map(|frame| AnimationObservation {
                        pixels: grab_ref(frame.pixels()),
                        metadata: RowMetadata::from_view(frame.pixels()),
                        timing: zencodec::decode::FrameCompletion {
                            duration: frame.duration(),
                            frame_index: frame.frame_index(),
                            pts: frame.pts(),
                        },
                    })
                }
                1 => {
                    let frame = if let Some(decoder) = &mut erased {
                        decoder
                            .render_next_frame_owned(None)
                            .map_err(|error| error.to_string())
                    } else {
                        typed
                            .as_mut()
                            .unwrap()
                            .render_next_frame_owned(None)
                            .map_err(|error| error.to_string())
                    }
                    .map_err(wrap)?;
                    frame.map(|frame| {
                        let view = frame.pixels();
                        AnimationObservation {
                            pixels: grab_ref(&view),
                            metadata: RowMetadata::from_view(&view),
                            timing: zencodec::decode::FrameCompletion {
                                duration: frame.duration(),
                                frame_index: frame.frame_index(),
                                pts: frame.pts(),
                            },
                        }
                    })
                }
                _ => {
                    let completion = if let Some(decoder) = &mut erased {
                        decoder
                            .render_next_frame_to_sink(None, &mut sink)
                            .map_err(|error| error.to_string())
                    } else {
                        typed
                            .as_mut()
                            .unwrap()
                            .render_next_frame_to_sink(None, &mut sink)
                            .map_err(|error| error.to_string())
                    }
                    .map_err(wrap)?;
                    match completion {
                        Some(info) => {
                            sink.verify_completion(&info).map_err(wrap)?;
                            Some(AnimationObservation {
                                timing: info.frame.ok_or_else(|| {
                                    fail(CHECK, "missing frame completion timing")
                                })?,
                                pixels: sink.pixels().map_err(wrap)?,
                                metadata: sink.metadata.unwrap_or_default(),
                            })
                        }
                        None if sink.desc.is_none() => None,
                        None => return Err(fail(CHECK, "end of animation had sink effects")),
                    }
                }
            };
            match (expected, observed) {
                (None, None) => {}
                (Some(want), Some(got))
                    if got.timing == want.timing
                        && got.pixels == grab_ref(&want.pixels)
                        && got.metadata == RowMetadata::from_view(&want.pixels) => {}
                _ => {
                    return Err(fail(
                        CHECK,
                        format!(
                            "route {mode} did not deliver frame or EOF at index {index} as specified"
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Verify rejection of an exact unsupported spec before header publication.
/// The encoder must remain usable with the accepted spec after the refusal.
pub fn check_animation_begin_refusal<E>(
    encoder: E,
    accepted: PixelView<'_>,
    rejected: PixelView<'_>,
    expected: ExpectedError,
) -> Conformance
where
    E: EncoderConfig + 'static,
    <E::Job as EncodeJob>::Enc: Encoder + Send,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    const CHECK: &str = "animation begin admission";
    let (width, height) = (accepted.width(), accepted.height());
    if (rejected.width(), rejected.height()) != (width, height)
        || accepted.format() == rejected.format()
    {
        return Err(fail(
            CHECK,
            "supply two different specs with identical dimensions",
        ));
    }
    let mut typed = encoder
        .clone()
        .job()
        .animation_frame_encoder()
        .map_err(|error| fail(CHECK, error.to_string()))?;
    let mut erased = encoder
        .dyn_job()
        .into_animation_frame_encoder()
        .map_err(|error| fail(CHECK, error.to_string()))?;
    let reject = rejected.format().clone();
    let typed_failure = typed
        .begin(width, height, reject.clone(), Some(1))
        .err()
        .filter(|error| expected.matches(error));
    let erased_failure = erased
        .begin(width, height, reject, Some(1))
        .err()
        .filter(|error| expected.matches(error.as_ref()));
    if typed_failure.is_none()
        || erased_failure.is_none()
        || !typed.pending_output().is_empty()
        || !erased.pending_output().is_empty()
    {
        return Err(fail(
            CHECK,
            "unsupported begin accepted, returned the wrong error or published bytes",
        ));
    }
    typed
        .begin(width, height, accepted.format().clone(), Some(1))
        .map_err(|error| fail(CHECK, error.to_string()))?;
    erased
        .begin(width, height, accepted.format().clone(), Some(1))
        .map_err(|error| fail(CHECK, error.to_string()))?;
    let typed_len = typed.pending_output().len();
    let erased_len = erased.pending_output().len();
    if !typed
        .push_frame(rejected.clone(), 50, None)
        .is_err_and(|error| expected.matches(&error))
        || !erased
            .push_frame(rejected, 50, None)
            .is_err_and(|error| expected.matches(error.as_ref()))
        || typed.pending_output().len() != typed_len
        || erased.pending_output().len() != erased_len
    {
        return Err(fail(
            CHECK,
            "an incompatible frame was accepted or changed published output",
        ));
    }
    let typed = typed
        .push_frame(accepted.clone(), 50, None)
        .and_then(|()| typed.finish(None))
        .map_err(|error| fail(CHECK, error.to_string()))?;
    erased
        .push_frame(accepted, 50, None)
        .map_err(|error| fail(CHECK, error.to_string()))?;
    let erased = erased
        .finish(None)
        .map_err(|error| fail(CHECK, error.to_string()))?;
    if typed.data() != erased.data() {
        return Err(fail(
            CHECK,
            "typed and erased bytes differ after refusal and reuse",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod prefix_tests {
    use super::*;

    #[test]
    fn explicit_header_and_required_payload_offsets_allow_an_independent_trailer() {
        let mut bytes = b"ZCR1".to_vec();
        for n in [1u32, 1, 1] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        bytes.extend_from_slice(&[3, 1, 0, 0, 0, 0, 0, 49, 78, 128]);
        let raw = [49, 78, 128];
        let expected = PixelView::new(&raw, 1, 1, 3, &zenpixels::formats::RGB8_SRGB).unwrap();
        bytes.extend_from_slice(&[0xE3; 128]);
        let incomplete =
            ExpectedError::Category(ErrorCategory::Image(zencodec::ImageError::Malformed));
        check_decode_prefix_cases(
            crate::MinimalDecoderConfig::default(),
            &bytes,
            &[
                PrefixCase {
                    end: 18,
                    outcome: PrefixOutcome::Reject(incomplete),
                },
                PrefixCase {
                    end: 25,
                    outcome: PrefixOutcome::Reject(incomplete),
                },
                PrefixCase {
                    end: 26,
                    outcome: PrefixOutcome::Accept(expected.clone()),
                },
                PrefixCase {
                    end: bytes.len(),
                    outcome: PrefixOutcome::Accept(expected),
                },
            ],
        )
        .unwrap();
    }
}
