#![forbid(unsafe_code)]
#[allow(dead_code)]
#[path = "../../../zencodec-testkit/src/fixtures.rs"]
mod fixtures;
#[path = "../src/policy_native.rs"]
mod native;
#[path = "../src/policy/mod.rs"]
mod policy;
#[path = "../../../docs/examples/policy.rs"]
mod policy_examples;
use policy::*;
use std::{
    convert::Infallible,
    io::{self, Write},
};
use zenbitmaps::{BmpDecoderConfig, QoiDecoderConfig, QoiEncoderConfig};
use zencodec::{
    At, ImageFormat, Metadata, MetadataPolicy, ResourceLimits,
    decode::DecoderConfig,
    encode::{EncodeJob, EncoderConfig},
};
use zenpixels::{PixelDescriptor, PixelSlice};

fn req(effects: Effects) -> Requirements {
    Requirements {
        ram: None,
        disk: Some(0),
        accounting: Accounting::Operation,
        effects,
    }
}
fn fixture() -> (Vec<u8>, u32, u32, usize) {
    let (w, h, stride) = (13, 7, 64);
    let mut bytes = vec![0xa5; stride * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            bytes[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&[
                x as u8,
                y as u8,
                (x + y) as u8,
                127,
            ]);
        }
    }
    (bytes, w, h, stride)
}
fn pixels(f: &(Vec<u8>, u32, u32, usize)) -> PixelSlice<'_> {
    PixelSlice::new(&f.0, f.1, f.2, f.3, PixelDescriptor::RGBA8_SRGB).unwrap()
}

#[test]
fn real_exif_filter_is_checked_by_an_independent_reader() {
    let bytes = fixtures::rich_exif_le();
    let before = exif::Reader::new().read_raw(bytes.clone()).unwrap();
    assert!(
        before
            .get_field(exif::Tag::GPSLatitudeRef, exif::In::PRIMARY)
            .is_some()
    );
    let out = native::exif(
        &mut Engine::new(PolicySet::default(), 3),
        Some(&bytes),
        true,
        req(Effects::RIGHTS.union(Effects::NO_GPS)),
        &mut PolicySet::default(),
    )
    .unwrap()
    .unwrap();
    let after = exif::Reader::new().read_raw(out.to_vec()).unwrap();
    let field = |tag| after.get_field(tag, exif::In::PRIMARY);
    assert!(field(exif::Tag::GPSLatitudeRef).is_none());
    assert_eq!(
        format!("{:?}", field(exif::Tag::Copyright).unwrap().value),
        format!(
            "{:?}",
            before
                .get_field(exif::Tag::Copyright, exif::In::PRIMARY)
                .unwrap()
                .value
        )
    );
    assert_eq!(
        field(exif::Tag::Orientation).unwrap().value.get_uint(0),
        Some(6)
    );
    for tag in [
        exif::Tag::Make,
        exif::Tag::BodySerialNumber,
        exif::Tag::CameraOwnerName,
        exif::Tag::ImageUniqueID,
        exif::Tag::DateTimeOriginal,
    ] {
        assert!(field(tag).is_none(), "{tag:?}");
    }
    assert!(after.fields().all(|f| f.ifd_num == exif::In::PRIMARY));
}

#[test]
fn malformed_exact_privacy_conflicts_and_missing_carriers_do_not_silently_downgrade() {
    let valid = fixtures::rich_exif_le();
    let invalid = b"II*\0\xff\xff\xff\xff";
    let conflict = Effects::EXACT_METADATA.union(Effects::NO_GPS);
    for bytes in [valid.as_slice(), invalid] {
        let mut called = false;
        let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            called = true;
            Ok(Decision::UsePreset)
        };
        assert!(
            native::exif(
                &mut Engine::new(PolicySet::default(), 3),
                Some(bytes),
                true,
                req(conflict),
                &mut resolver
            )
            .is_err()
        );
        assert!(!called);
        let out = native::exif(
            &mut Engine::new(PolicySet::default(), 3),
            Some(bytes),
            true,
            req(Effects::EXACT_METADATA),
            &mut PolicySet::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(out.as_ref(), bytes);
        let out = native::exif(
            &mut Engine::new(PolicySet::default(), 3),
            Some(bytes),
            false,
            req(Effects::NO_GPS),
            &mut PolicySet::default(),
        )
        .unwrap();
        assert!(out.is_none());
    }
    assert!(
        native::exif(
            &mut Engine::new(PolicySet::default(), 3),
            Some(&valid),
            false,
            req(Effects::RIGHTS),
            &mut PolicySet::default()
        )
        .is_err()
    );
    // Keeping absent rights is vacuous. A required PRESENCE would be a separate intent.
    assert!(
        native::exif(
            &mut Engine::new(PolicySet::default(), 3),
            None,
            false,
            req(Effects::RIGHTS.union(Effects::EXACT_METADATA)),
            &mut PolicySet::default()
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn native_setter_is_best_effort_but_checked_retention_refuses_before_encoding() {
    let fixture = fixture();
    let metadata = Metadata::none().with_copyright("Required attribution");
    let legacy = QoiEncoderConfig::new()
        .job()
        .with_metadata_policy(metadata.clone(), MetadataPolicy::PreserveExact)
        .encode(pixels(&fixture))
        .unwrap();
    let decoded = QoiDecoderConfig::new().decode(legacy.data()).unwrap();
    assert!(decoded.metadata().exif.is_none());
    for format in [ImageFormat::Qoi, ImageFormat::Bmp] {
        for erased in [false, true] {
            let mut output = Vec::new();
            let mut count = 0;
            let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                count += 1;
                Ok(Decision::UsePreset)
            };
            let error = native::encode(
                native::Request {
                    preset: PolicySet::default(),
                    format,
                    erased,
                    requirements: req(Effects::RIGHTS),
                    limits: ResourceLimits::none(),
                },
                pixels(&fixture),
                &metadata,
                &mut resolver,
                &mut output,
            )
            .unwrap_err();
            assert_eq!(error.error().report.stage, native::Stage::Policy);
            assert!(output.is_empty());
            assert_eq!(count, 0);
            assert!(zencodec::find_cause::<Failure<Infallible>>(&error).is_some());
        }
    }
}

#[test]
fn typed_and_registered_operations_preserve_strided_pixels_and_call_order() {
    let fixture = fixture();
    let metadata = Metadata::none().with_copyright("Optional attribution");
    for format in [ImageFormat::Qoi, ImageFormat::Bmp] {
        let mut outputs = Vec::new();
        for erased in [false, true] {
            let mut questions = Vec::new();
            let mut resolver = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
                questions.push(match q {
                    Question::Metadata { .. } => Kind::Metadata,
                    Question::Execution { .. } => Kind::Execution,
                    Question::Fidelity { .. } => Kind::Fidelity,
                    Question::Media { .. } => Kind::Media,
                    Question::Unknown { code } => Kind::Unknown(code),
                });
                Ok(Decision::UsePreset)
            };
            let mut output = Vec::new();
            let report = native::encode(
                native::Request {
                    preset: PolicySet::default(),
                    format,
                    erased,
                    requirements: req(Effects::LOSSLESS.union(Effects::NO_GPS)),
                    limits: ResourceLimits::none(),
                },
                pixels(&fixture),
                &metadata,
                &mut resolver,
                &mut output,
            )
            .unwrap();
            assert_eq!(report.accepted_bytes, output.len() as u64);
            assert!(report.output_complete);
            assert_eq!(questions, [Kind::Metadata, Kind::Fidelity, Kind::Execution]);
            let decoded = match format {
                ImageFormat::Qoi => QoiDecoderConfig::new().decode(&output).unwrap(),
                ImageFormat::Bmp => BmpDecoderConfig::new().decode(&output).unwrap(),
                _ => unreachable!(),
            };
            for y in 0..fixture.2 {
                assert_eq!(decoded.pixels().row(y), pixels(&fixture).row(y));
            }
            outputs.push(output);
        }
        assert_eq!(outputs[0], outputs[1]);
    }
}

#[derive(Debug)]
struct WriterBroke;
impl std::fmt::Display for WriterBroke {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("fixture writer failed")
    }
}
impl std::error::Error for WriterBroke {}
struct Writer {
    bytes: Vec<u8>,
    limit: usize,
    chunk: usize,
    interrupted: bool,
    zero: bool,
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if self.zero {
            return Ok(0);
        }
        if self.bytes.len() == self.limit {
            return Err(io::Error::other(WriterBroke));
        }
        let n = bytes
            .len()
            .min(self.chunk)
            .min(self.limit - self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn output_errors_keep_progress_and_native_causes_without_reencoding() {
    let fixture = fixture();
    for erased in [false, true] {
        for zero in [false, true] {
            let mut writer = Writer {
                bytes: vec![],
                limit: 11,
                chunk: 3,
                interrupted: false,
                zero,
            };
            let mut decisions = 0;
            let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                decisions += 1;
                Ok(Decision::UsePreset)
            };
            let error = native::encode(
                native::Request {
                    preset: PolicySet::default(),
                    format: ImageFormat::Qoi,
                    erased,
                    requirements: req(Effects::LOSSLESS),
                    limits: ResourceLimits::none(),
                },
                pixels(&fixture),
                &Metadata::none(),
                &mut resolver,
                &mut writer,
            )
            .unwrap_err();
            assert_eq!(error.error().report.stage, native::Stage::Output);
            assert_eq!(
                error.error().report.accepted_bytes,
                if zero { 0 } else { 11 }
            );
            assert!(!error.error().report.output_complete);
            let selected = &error.error().report.decisions;
            assert_eq!(selected.metadata, Some(MetadataAction::KeepExact));
            assert_eq!(
                selected.fidelity,
                Some(zencodec::encode::Fidelity::Lossless)
            );
            assert_eq!(selected.execution, Some(Execution::Buffered));
            assert!(
                selected
                    .records
                    .iter()
                    .all(|r| r.unwrap().code == Code::Selected)
            );
            assert_eq!(decisions, 3);
            let io = zencodec::find_cause::<io::Error>(&error).unwrap();
            if zero {
                assert_eq!(io.kind(), io::ErrorKind::WriteZero);
            } else {
                assert!(io.get_ref().unwrap().is::<WriterBroke>());
            }
        }
    }
    let mut writer = Writer {
        bytes: vec![],
        limit: usize::MAX,
        chunk: 1,
        interrupted: false,
        zero: false,
    };
    native::encode(
        native::Request {
            preset: PolicySet::default(),
            format: ImageFormat::Qoi,
            erased: false,
            requirements: req(Effects::LOSSLESS),
            limits: ResourceLimits::none(),
        },
        pixels(&fixture),
        &Metadata::none(),
        &mut PolicySet::default(),
        &mut writer,
    )
    .unwrap();
    assert_eq!(
        writer.bytes,
        QoiEncoderConfig::new()
            .encode(pixels(&fixture))
            .unwrap()
            .data()
    );
}

#[test]
fn one_stored_request_keeps_profile_requirements_and_its_configured_fallback() {
    let fixture = fixture();
    let profile = Profile {
        requirements: req(Effects::LOSSLESS),
        preset: PolicySet {
            metadata: MetadataAction::DropDescriptive,
            ..PolicySet::default()
        },
    };
    for format in [ImageFormat::Qoi, ImageFormat::Bmp] {
        let mut outputs = Vec::new();
        for erased in [false, true] {
            for custom in [false, true] {
                let mut request = native::Request::new(format, profile);
                if erased {
                    request = request.via_registry();
                }
                let mut output = Vec::new();
                // Captured application state need not be Send. Each operation
                // borrows its own callback; no shared callback mutex is required.
                let calls = std::rc::Rc::new(std::cell::Cell::new(0));
                let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                    calls.set(calls.get() + 1);
                    Ok(Decision::UsePreset)
                };
                let report = if custom {
                    request.encode_with(
                        pixels(&fixture),
                        &Metadata::none(),
                        &mut resolver,
                        &mut output,
                    )
                } else {
                    // EXAMPLE: policy_one_call
                    // ANCHOR: policy_one_call
                    request.encode(pixels(&fixture), &Metadata::none(), &mut output)
                    // END: policy_one_call
                    // END EXAMPLE: policy_one_call
                }
                .unwrap();
                assert_eq!(calls.get(), if custom { 3 } else { 0 });
                assert_eq!(
                    report.decisions.metadata,
                    Some(MetadataAction::DropDescriptive)
                );
                for r in report.decisions.records.into_iter().flatten() {
                    assert_eq!(r.code, Code::Selected);
                    assert_eq!(
                        r.route,
                        if custom {
                            Route::Preset
                        } else {
                            Route::Resolver
                        }
                    );
                }
                outputs.push(output);
            }
            // A callback cannot remove the stored web profile's rights requirement.
            let mut request = native::Request::new(format, Profile::web_exif());
            if erased {
                request = request.via_registry();
            }
            let mut called = false;
            let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
                called = true;
                Ok(Decision::UsePreset)
            };
            let mut output = Vec::new();
            let error = request
                .encode_with(
                    pixels(&fixture),
                    &Metadata::none().with_copyright("Required"),
                    &mut resolver,
                    &mut output,
                )
                .unwrap_err();
            assert!(!called);
            assert!(output.is_empty());
            let failure = zencodec::find_cause::<Failure<Infallible>>(&error).unwrap();
            assert_eq!(failure.code(), Code::NoAlternative);
            assert_eq!(
                failure.details().unwrap().requirements,
                Profile::web_exif().requirements
            );
            assert_eq!(
                error.error().report.decisions.records[0].unwrap().code,
                Code::NoAlternative
            );
        }
        assert!(outputs.iter().all(|output| output == &outputs[0]));
    }
}

#[test]
fn late_callback_error_keeps_earlier_selections_and_the_original_trace() {
    let fixture = fixture();
    let mut calls = 0;
    let mut resolver = |question: Question<'_>| -> Result<Decision, At<WriterBroke>> {
        calls += 1;
        if matches!(question, Question::Fidelity { .. }) {
            return Err(At::wrap(WriterBroke).at_named("application fidelity rule"));
        }
        Ok(Decision::UsePreset)
    };
    let mut output = Vec::new();
    let error = native::Request::new(ImageFormat::Qoi, Profile::exact_exif())
        .via_registry()
        .encode_with(
            pixels(&fixture),
            &Metadata::none(),
            &mut resolver,
            &mut output,
        )
        .unwrap_err();
    assert!(output.is_empty());
    assert_eq!(calls, 2);
    assert!(zencodec::find_cause::<WriterBroke>(&error).is_some());
    assert!(format!("{error:?}").contains("application fidelity rule"));
    let decisions = &error.error().report.decisions;
    assert_eq!(decisions.metadata, Some(MetadataAction::KeepExact));
    assert!(decisions.fidelity.is_none());
    assert!(decisions.execution.is_none());
    assert_eq!(decisions.records[0].unwrap().code, Code::Selected);
    assert_eq!(decisions.records[1].unwrap().code, Code::Callback);
    assert!(decisions.records[2].is_none());
}

#[test]
fn documented_consumer_calls_preserve_success_decline_and_callback_error() {
    let fixture = fixture();
    let mut expected = Vec::new();
    policy_examples::derivative(pixels(&fixture), &Metadata::none(), &mut expected).unwrap();
    let mut output = Vec::new();
    policy_examples::account_derivative(
        pixels(&fixture),
        &Metadata::none(),
        &mut output,
        Some(true),
    )
    .unwrap();
    assert_eq!(output, expected);
    for may_publish in [Some(false), None] {
        let mut output = Vec::new();
        let error = policy_examples::account_derivative(
            pixels(&fixture),
            &Metadata::none(),
            &mut output,
            may_publish,
        )
        .unwrap_err();
        assert!(output.is_empty());
        let failure =
            zencodec::find_cause::<Failure<policy_examples::AccountRuleUnavailable>>(&error)
                .unwrap();
        assert_eq!(
            failure.code(),
            if may_publish.is_none() {
                Code::Callback
            } else {
                Code::Declined
            }
        );
        assert_eq!(
            zencodec::find_cause::<policy_examples::AccountRuleUnavailable>(&error).is_some(),
            may_publish.is_none()
        );
    }
    let mut output = Vec::new();
    let error = policy_examples::derivative(
        pixels(&fixture),
        &Metadata::none().with_copyright("Required rights"),
        &mut output,
    )
    .unwrap_err();
    assert_eq!(
        zencodec::find_cause::<Failure<Infallible>>(&error)
            .unwrap()
            .code(),
        Code::NoAlternative
    );
    assert!(output.is_empty());
}

#[test]
fn native_limit_failure_stays_a_codec_error_and_unknown_memory_refuses_before_callback() {
    let fixture = fixture();
    for erased in [false, true] {
        let mut output = Vec::new();
        let error = native::encode(
            native::Request {
                preset: PolicySet::default(),
                format: ImageFormat::Qoi,
                erased,
                requirements: req(Effects::LOSSLESS),
                limits: ResourceLimits::none().with_max_pixels(1),
            },
            pixels(&fixture),
            &Metadata::none(),
            &mut PolicySet::default(),
            &mut output,
        )
        .unwrap_err();
        assert_eq!(error.error().report.stage, native::Stage::Codec);
        assert!(zencodec::find_cause::<zencodec::CodecError>(&error).is_some());
        assert!(output.is_empty());
    }
    let mut called = false;
    let mut resolver = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        called = true;
        Ok(Decision::UsePreset)
    };
    assert!(
        native::encode(
            native::Request {
                preset: PolicySet::default(),
                format: ImageFormat::Qoi,
                erased: false,
                requirements: Requirements {
                    ram: Some(100000),
                    ..req(Effects::LOSSLESS)
                },
                limits: ResourceLimits::none()
            },
            pixels(&fixture),
            &Metadata::none(),
            &mut resolver,
            &mut Vec::new()
        )
        .is_err()
    );
    assert!(!called);
}

#[test]
fn a_named_profile_installs_requirements_even_when_a_custom_callback_delegates() {
    let invalid = b"II*\0\xff\xff\xff\xff";
    let profile = Profile::web_exif();
    let mut fallback =
        |_: Question<'_>| -> Result<Decision, At<Infallible>> { Ok(Decision::UsePreset) };
    assert!(
        native::exif(
            &mut Engine::new(profile.preset, 3),
            Some(invalid),
            true,
            profile.requirements,
            &mut fallback
        )
        .is_err()
    );
    let exact = Profile::exact_exif();
    assert_eq!(
        native::exif(
            &mut Engine::new(exact.preset, 3),
            Some(invalid),
            true,
            exact.requirements,
            &mut fallback
        )
        .unwrap()
        .unwrap()
        .as_ref(),
        invalid
    );
    let bounded = exact.with_memory_cap(1024);
    assert!(
        native::exif(
            &mut Engine::new(bounded.preset, 3),
            Some(invalid),
            true,
            bounded.requirements,
            &mut fallback
        )
        .is_err()
    );
}

#[test]
fn still_requests_reject_requirements_from_other_domains_instead_of_ignoring_them() {
    let fixture = fixture();
    let mut called = false;
    let mut callback = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
        called = true;
        Ok(Decision::UsePreset)
    };
    let result = native::encode(
        native::Request {
            preset: PolicySet::default(),
            format: ImageFormat::Qoi,
            erased: false,
            requirements: req(Effects::KEEP_TRACK),
            limits: ResourceLimits::none(),
        },
        pixels(&fixture),
        &Metadata::none(),
        &mut callback,
        &mut Vec::new(),
    );
    assert_eq!(
        result.unwrap_err().error().report.stage,
        native::Stage::Configuration
    );
    assert!(!called);
}
