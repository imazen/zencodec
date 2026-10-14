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
    Accept(PixelSlice<'a>),
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
    pub pixels: PixelSlice<'a>,
    pub requested: FrameDuration,
    pub expected: Result<(FrameDuration, PixelSlice<'a>), ExpectedError>,
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
        let pixels = case.pixels.sub_rows(0, case.pixels.rows());
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
        let bytes = MinimalEncoderConfig::new()
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
        check_decode_prefix_cases(MinimalDecoderConfig::new(), &trailing, &cases).unwrap();
        // No file-length heuristic: required payload loss is rejected even near EOF.
        let cases = [PrefixCase {
            end: complete - 1,
            outcome: PrefixOutcome::Reject(ExpectedError::Category(ErrorCategory::Image(
                zencodec::ImageError::Malformed,
            ))),
        }];
        check_decode_prefix_cases(MinimalDecoderConfig::new(), bytes.data(), &cases).unwrap();
        let wrong = TestImage::rgb8_gradient(4, 2);
        let cases = [PrefixCase {
            end: complete,
            outcome: PrefixOutcome::Accept(wrong.as_slice()),
        }];
        assert!(
            check_decode_prefix_cases(MinimalDecoderConfig::new(), bytes.data(), &cases).is_err()
        );
        let cases = [PrefixCase {
            end: complete - 1,
            outcome: PrefixOutcome::Accept(image.as_slice()),
        }];
        assert!(
            check_decode_prefix_cases(MinimalDecoderConfig::new(), bytes.data(), &cases).is_err()
        );
    }
}
