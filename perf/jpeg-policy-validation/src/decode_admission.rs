//! Native counterexample to treating unconfigured output prediction as admission.
use crate::fixtures;
use std::borrow::Cow;
use zencodec::{
    CodecSet,
    decode::{Decode, DecodeJob, DecoderConfig},
    operation::{DecodeRequest, OperationError},
};
use zenjpeg::JpegDecoderConfig;
use zenpixels::PixelDescriptor;

#[test]
fn negotiated_output_must_not_be_rejected_by_native_prediction() {
    for progressive in [false, true] {
        let base = fixtures::base(progressive);
        let mut bytes = base[..2].to_vec();
        bytes.extend(fixtures::descriptive());
        bytes.extend_from_slice(&base[2..]);
        let config = JpegDecoderConfig::new();
        let predicted = config.clone().job().output_info(&bytes).unwrap();
        let requested = PixelDescriptor::RGBA8_SRGB;
        assert_ne!(predicted.native_format, requested);
        let expected = config
            .clone()
            .job()
            .decoder(Cow::Borrowed(&bytes), &[requested])
            .unwrap()
            .decode()
            .unwrap();
        let codecs = CodecSet::new().with_decoder(config);
        let actual = DecodeRequest::new()
            .with_output(requested)
            .decode(&codecs, &bytes)
            .unwrap();
        assert_eq!(actual.pixels().descriptor(), requested);
        assert_eq!(actual.metadata(), expected.metadata());
        assert_eq!(actual.info().orientation, expected.info().orientation);
        assert_eq!(actual.width(), expected.width());
        assert_eq!(actual.height(), expected.height());
        for row in 0..actual.height() {
            assert_eq!(actual.pixels().row(row), expected.pixels().row(row));
        }
    }
}

#[test]
fn required_output_keeps_final_check_when_native_falls_back() {
    let bytes = fixtures::base(false);
    let codecs = CodecSet::new().with_decoder(JpegDecoderConfig::new());
    let requested = PixelDescriptor::RGB16_SRGB;
    let error = DecodeRequest::new()
        .with_output(requested)
        .decode(&codecs, &bytes)
        .unwrap_err();
    assert!(matches!(
        zencodec::find_cause::<OperationError>(&error),
        Some(OperationError::OutputDescriptor { requested: got, actual })
            if *got == requested && *actual != requested
    ));
}

#[test]
fn required_native_layouts_preserve_samples_and_metadata_across_scan_modes() {
    for progressive in [false, true] {
        let base = fixtures::base(progressive);
        let mut bytes = base[..2].to_vec();
        bytes.extend(fixtures::descriptive());
        bytes.extend_from_slice(&base[2..]);
        for requested in [
            PixelDescriptor::RGB8_SRGB,
            PixelDescriptor::RGBA8_SRGB,
            PixelDescriptor::BGRA8_SRGB,
            PixelDescriptor::RGBX8_SRGB,
            PixelDescriptor::BGRX8_SRGB,
            PixelDescriptor::RGBF32_LINEAR,
            PixelDescriptor::RGBAF32_LINEAR,
        ] {
            let config = JpegDecoderConfig::new();
            let expected = config
                .clone()
                .job()
                .decoder(Cow::Borrowed(&bytes), &[requested])
                .unwrap()
                .decode()
                .unwrap();
            assert_eq!(
                expected.pixels().descriptor(),
                requested,
                "native output differs: progressive={progressive}, requested={requested:?}"
            );
            let codecs = CodecSet::new().with_decoder(config);
            let actual = DecodeRequest::new()
                .with_output(requested)
                .decode(&codecs, &bytes)
                .unwrap();
            assert_eq!(actual.pixels().descriptor(), requested);
            assert_eq!(actual.metadata(), expected.metadata());
            assert_eq!(actual.info().orientation, expected.info().orientation);
            assert_eq!(
                (actual.width(), actual.height()),
                (expected.width(), expected.height())
            );
            for row in 0..actual.height() {
                assert_eq!(actual.pixels().row(row), expected.pixels().row(row));
            }
        }
    }
}

#[test]
fn ranked_native_output_does_not_promote_a_later_float_preference() {
    for progressive in [false, true] {
        let bytes = fixtures::base(progressive);
        for preferred in [
            [PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBF32_LINEAR],
            [PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGBAF32_LINEAR],
            [PixelDescriptor::RGBF32_LINEAR, PixelDescriptor::RGB8_SRGB],
            [PixelDescriptor::RGBAF32_LINEAR, PixelDescriptor::RGBA8_SRGB],
        ] {
            let config = JpegDecoderConfig::new();
            let expected = config
                .clone()
                .job()
                .decoder(Cow::Borrowed(&bytes), &preferred[..1])
                .unwrap()
                .decode()
                .unwrap();
            let actual = DecodeRequest::new()
                .preferring_output(preferred.to_vec())
                .decode(&CodecSet::new().with_decoder(config), &bytes)
                .unwrap();
            assert_eq!(
                actual.pixels().descriptor(),
                expected.pixels().descriptor(),
                "progressive={progressive}, preferred={preferred:?}"
            );
            assert_eq!(actual.metadata(), expected.metadata());
            for row in 0..actual.height() {
                assert_eq!(actual.pixels().row(row), expected.pixels().row(row));
            }
        }
    }
}

#[test]
fn native_owned_result_split_preserves_pixels_metadata_and_all_extras() {
    for progressive in [false, true] {
        let base = fixtures::base(progressive);
        let mut bytes = base[..2].to_vec();
        bytes.extend(fixtures::descriptive());
        bytes.extend_from_slice(&base[2..]);
        let output = JpegDecoderConfig::new()
            .job()
            .decoder(Cow::Borrowed(&bytes), &[PixelDescriptor::RGBA8_SRGB])
            .unwrap()
            .decode()
            .unwrap();
        let metadata = output.metadata();
        let pixels = output.pixels().row(0).as_ptr();
        let source = output.source_encoding_details().unwrap() as *const _ as *const ();
        let extras = output.extras::<zenjpeg::decoder::DecodedExtras>().unwrap() as *const _;
        let count = output.extensions().len();
        let (buffer, details) = output.into_parts();
        assert_eq!(buffer.as_slice().row(0).as_ptr(), pixels);
        assert_eq!(details.info.metadata(), metadata);
        assert_eq!(
            details.source_encoding.as_deref().unwrap() as *const _ as *const (),
            source
        );
        assert_eq!(
            details
                .extensions
                .get::<zenjpeg::decoder::DecodedExtras>()
                .unwrap() as *const _,
            extras
        );
        assert_eq!(details.extensions.len(), count);
        let restored = zencodec::decode::DecodeOutput::from_parts(buffer, details);
        assert_eq!(restored.pixels().row(0).as_ptr(), pixels);
        assert_eq!(restored.metadata(), metadata);
        assert_eq!(
            restored
                .extras::<zenjpeg::decoder::DecodedExtras>()
                .unwrap() as *const _,
            extras
        );
    }
}

/// Private completion transport witness. The native one-shot decode has already
/// happened; this deliberately makes no early-admission or streaming-RAM claim.
struct OwnedResultRows {
    output: Option<zencodec::decode::DecodeOutput>,
    details: Option<zencodec::decode::DecodeDetails>,
}
impl zencodec::decode::IncrementalDecode for OwnedResultRows {
    type Error = std::io::Error;
    fn step(
        &mut self,
        _: zencodec::decode::Input<'_>,
        sink: &mut dyn zencodec::decode::DecodeRowSink,
        _: Option<&dyn zencodec::Pulse>,
    ) -> Result<zencodec::decode::Progress, zencodec::decode::StepError<Self::Error>> {
        use zencodec::decode::{Event, Progress, StepError};
        let output = self.output.take().ok_or(StepError::Finished)?;
        let info = zencodec::decode::OutputInfo::full_decode(
            output.width(),
            output.height(),
            output.descriptor(),
        );
        let (pixels, details) = output.into_parts();
        sink.begin(info.width, info.height, info.native_format)
            .map_err(StepError::Sink)?;
        for y in 0..info.height {
            sink.provide_next_buffer(y, 1, info.width, info.native_format)
                .map_err(StepError::Sink)?
                .row_mut(0)
                .copy_from_slice(pixels.as_slice().row(y));
        }
        sink.finish().map_err(StepError::Sink)?;
        self.details = Some(details);
        Ok(Progress::new(0, Event::Done(info)))
    }
    fn requested(&self) -> &[zencodec::decode::InputRequest] {
        &[]
    }
    fn info(&self) -> Option<&zencodec::ImageInfo> {
        None
    }
    fn take_details(&mut self) -> Option<zencodec::decode::DecodeDetails> {
        self.details.take()
    }
}

#[test]
fn native_owned_details_survive_terminal_erasure_and_row_delivery() {
    use zencodec::decode::{Input, erase_incremental};
    for progressive in [false, true] {
        let bytes = fixtures::base(progressive);
        let output = JpegDecoderConfig::new()
            .job()
            .decoder(Cow::Borrowed(&bytes), &[PixelDescriptor::RGBA8_SRGB])
            .unwrap()
            .decode()
            .unwrap();
        let metadata = output.metadata();
        let source = output.source_encoding_details().unwrap() as *const _ as *const ();
        let expected: Vec<_> = (0..output.height())
            .map(|y| output.pixels().row(y).to_vec())
            .collect();
        let mut decoder = erase_incremental(OwnedResultRows {
            output: Some(output),
            details: None,
        });
        assert!(decoder.take_details().is_none());
        let mut sink = zencodec_io::CollectSink::new();
        decoder.step(Input::empty(), &mut sink, None).unwrap();
        let details = decoder.take_details().unwrap();
        assert_eq!(details.info.metadata(), metadata);
        assert_eq!(
            details.source_encoding.as_deref().unwrap() as *const _ as *const (),
            source
        );
        for (y, row) in expected.iter().enumerate() {
            assert_eq!(sink.as_pixel_slice().unwrap().row(y as u32), row);
        }
        assert!(decoder.take_details().is_none());
    }
}

#[test]
fn native_controlled_completion_retains_descriptive_metadata_and_extras() {
    use zencodec::decode::IncrementalDecode;
    use zencodec::operation::PolicySet;
    use zencodec::storage::StorageBudget;
    use zencodec_io::{CollectSink, PolicyDecoder, drive_from_slice};
    for progressive in [false, true] {
        let base = fixtures::base(progressive);
        let mut bytes = base[..2].to_vec();
        bytes.extend(fixtures::descriptive());
        bytes.extend_from_slice(&base[2..]);
        let config = JpegDecoderConfig::new();
        let direct = config
            .clone()
            .job()
            .decoder(Cow::Borrowed(&bytes), &[PixelDescriptor::RGBA8_SRGB])
            .unwrap()
            .decode()
            .unwrap();
        let codecs = CodecSet::new().with_decoder(config);
        let mut factory_calls = 0;
        let mut session = PolicyDecoder::try_new(
            || {
                factory_calls += 1;
                DecodeRequest::new()
                    .with_output(PixelDescriptor::RGBA8_SRGB)
                    .start_controlled(&codecs, StorageBudget::new(1 << 20, 4))
            },
            PolicySet::new(),
            None,
        )
        .unwrap();
        assert_eq!(factory_calls, 1);
        let mut sink = CollectSink::new();
        assert!(session.take_details().is_none());
        drive_from_slice(&mut session, &bytes, &mut sink, None, 7).unwrap();
        let details = session.take_details().expect("real native completion");
        assert!(session.take_details().is_none());
        assert_eq!(details.info.metadata(), direct.metadata());
        assert_eq!(details.info.orientation, direct.info().orientation);
        assert!(details.source_encoding.is_some());
        let actual = details
            .extensions
            .get::<zenjpeg::decoder::DecodedExtras>()
            .unwrap();
        let expected = direct.extras::<zenjpeg::decoder::DecodedExtras>().unwrap();
        assert_eq!(actual.segments().len(), expected.segments().len());
        for (a, b) in actual.segments().iter().zip(expected.segments()) {
            assert_eq!(
                (a.marker, a.segment_type, &a.data),
                (b.marker, b.segment_type, &b.data)
            );
        }
        let storage = sink.bytes().as_ptr();
        let output = zencodec::decode::DecodeOutput::from_parts(
            sink.into_buffer().expect("driver finished the sink"),
            details,
        );
        assert_eq!(output.pixels().as_strided_bytes().as_ptr(), storage);
        assert_eq!(output.metadata(), direct.metadata());
        assert_eq!(output.pixels().descriptor(), direct.pixels().descriptor());
        for y in 0..direct.height() {
            assert_eq!(output.pixels().row(y), direct.pixels().row(y));
        }
    }
}

#[test]
fn required_profile_tagged_layout_is_negotiated_without_losing_channels() {
    for progressive in [false, true] {
        for placement in [
            fixtures::Placement::Header,
            fixtures::Placement::BeforeEoi,
            fixtures::Placement::SplitIcc,
        ] {
            let bytes = fixtures::fixture(progressive, placement);
            for preference in [
                PixelDescriptor::RGBA8_SRGB,
                PixelDescriptor::BGRA8_SRGB,
                PixelDescriptor::RGBX8_SRGB,
                PixelDescriptor::BGRX8_SRGB,
                PixelDescriptor::RGBAF32_LINEAR,
            ] {
                let expected = JpegDecoderConfig::new()
                    .job()
                    .decoder(Cow::Borrowed(&bytes), &[preference])
                    .unwrap()
                    .decode()
                    .unwrap();
                assert_eq!(
                    expected.metadata().icc_profile.as_deref(),
                    Some(fixtures::profile().as_slice()),
                    "native output must retain the complete profile at {placement:?}",
                );
                let required = expected.pixels().descriptor();
                assert_ne!(
                    required, preference,
                    "fixture must change color interpretation"
                );
                let codecs = CodecSet::new().with_decoder(JpegDecoderConfig::new());
                let actual = DecodeRequest::new().with_output(required)
                    .decode(&codecs, &bytes)
                    .unwrap_or_else(|e| panic!("progressive={progressive}, placement={placement:?}, preference={preference:?}, required={required:?}: {e}"));
                assert_eq!(actual.pixels().descriptor(), required);
                assert_eq!(actual.metadata(), expected.metadata());
                for row in 0..actual.height() {
                    assert_eq!(actual.pixels().row(row), expected.pixels().row(row));
                }
            }
        }
    }
}
