use crate::fixtures::*;
use std::{borrow::Cow, cell::RefCell, convert::Infallible, rc::Rc};
use zencodec::decode::{
    Decode, DecodeJob, DecodeRowSink, DecoderConfig, Event, IncrementalDecode, Input, Progress,
    Request, StepError, StreamingDecode,
};
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec::metadata_rewrite::{Action as RewriteAction, Request as RewriteRequest};
use zencodec::{ImageInfo, MetadataPolicy, OrientationHint, NoPulse};
use zencodec_io::{CollectSink, DriveOptions, drive_from_read_at, drive_from_slice};
use zenjpeg::JpegDecoderConfig;
use zenjpeg::container::marker;
use zenpixels::PixelDescriptor;

use crate::{policy, policy_disclosure};
#[path = "../../io-validation/src/transport_model.rs"]
#[allow(dead_code)]
mod transport;

#[derive(Debug)]
struct Snapshot {
    supplied: usize,
    orientation: zencodec::Orientation,
    icc: usize,
    xmp: usize,
}
struct Observed<D> {
    inner: D,
    metadata: Vec<Snapshot>,
    row_inputs: Vec<(usize, bool)>,
}
impl<D> Observed<D> {
    fn new(inner: D) -> Self {
        Self {
            inner,
            metadata: Vec::new(),
            row_inputs: Vec::new(),
        }
    }
}
impl<D: IncrementalDecode> IncrementalDecode for Observed<D> {
    type Error = D::Error;
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        stop: &dyn zencodec::Pulse,
    ) -> Result<Progress, StepError<Self::Error>> {
        let progress = self.inner.step(input, sink, stop)?;
        if matches!(progress.event, Event::Metadata(_)) {
            let info = self.inner.info().expect("metadata event has a snapshot");
            let metadata = info.metadata();
            self.metadata.push(Snapshot {
                supplied: input.bytes.len(),
                orientation: info.orientation,
                icc: metadata.icc_profile.as_ref().map_or(0, |v| v.len()),
                xmp: metadata.xmp.as_ref().map_or(0, |v| v.len()),
            });
        }
        if matches!(progress.event, Event::Rows { .. }) {
            self.row_inputs
                .push((input.bytes.len(), input.reaches_end()));
        }
        Ok(progress)
    }
    fn requested(&self) -> &[Request] {
        self.inner.requested()
    }
    fn info(&self) -> Option<&ImageInfo> {
        self.inner.info()
    }
    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.inner.output_descriptor()
    }
}

#[test]
fn encoder_output_timing_depends_on_configuration_and_rows_remain_bit_exact() {
    let raw = raw();
    for (progressive, optimized) in [(false, false), (false, true), (true, true)] {
        let cfg = config(progressive, optimized);
        let oracle_encode = cfg.clone().encode(pixels(&raw)).unwrap();
        for strip in [1, 7, 16] {
            let mut encoder = cfg.clone().job().encoder().unwrap();
            let timing = encoder
                .begin(WIDTH, HEIGHT, PixelDescriptor::RGB8_SRGB)
                .unwrap();
            assert_eq!(timing.streams(), !progressive && !optimized);
            let mut output = Vec::new();
            encoder.take_output(&mut output);
            assert_eq!(output.is_empty(), !timing.streams());
            let mut y = 0;
            while y < HEIGHT {
                let rows = strip.min(HEIGHT - y);
                let view = zenpixels::PixelSlice::new(
                    &raw[y as usize * STRIDE..],
                    WIDTH,
                    rows,
                    STRIDE,
                    PixelDescriptor::RGB8_SRGB,
                )
                .unwrap();
                encoder.push_rows(view).unwrap();
                encoder.take_output(&mut output);
                y += rows;
            }
            if !timing.streams() {
                assert!(output.is_empty());
            }
            output.extend_from_slice(encoder.finish().unwrap().data());
            assert_eq!(
                output,
                oracle_encode.data(),
                "progressive={progressive}, optimized={optimized}, strip={strip}"
            );
            assert_eq!(oracle(&output), oracle(oracle_encode.data()));
        }
    }
}

#[test]
fn native_rewrite_preserves_scans_and_orientation_while_removing_late_private_carriers() {
    for progressive in [false, true] {
        for placement in [
            Placement::Header,
            Placement::AfterFirstScan,
            Placement::BeforeEoi,
            Placement::SplitIcc,
        ] {
            let mut source = fixture(progressive, placement);
            source.extend_from_slice(PRIVATE); // A trailer is another disclosure surface.
            let request = RewriteRequest::new(&source);
            let plan = zenjpeg::metadata::rewrite(&request).unwrap();
            let output = plan.to_vec().unwrap();
            assert_eq!(scans(&source), scans(&output));
            assert_eq!(oracle(&source), oracle(&output));
            assert!(!output.windows(PRIVATE.len()).any(|v| v == PRIVATE));
            assert!(
                plan.report()
                    .iter()
                    .any(|r| r.action() == RewriteAction::OpaqueIcc)
            );
            let exif = exif::Reader::new()
                .read_from_container(&mut std::io::Cursor::new(&output))
                .unwrap();
            assert_eq!(
                exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                    .unwrap()
                    .value
                    .get_uint(0),
                Some(6)
            );
            assert!(
                exif.get_field(exif::Tag::Artist, exif::In::PRIMARY)
                    .is_none()
            );
            let parts: Vec<_> = marker::iter(&output)
                .filter(|s| s.payload.starts_with(b"ICC_PROFILE\0"))
                .collect();
            let combined: Vec<_> = parts
                .iter()
                .flat_map(|s| s.payload[14..].iter().copied())
                .collect();
            assert_eq!(combined, profile());
            println!(
                "JPEG_REWRITE progressive={progressive} placement={placement:?} input={} output={} scans={} opaque_icc=true",
                source.len(),
                output.len(),
                scans(&output).len()
            );
        }
    }
}

#[test]
fn late_metadata_exercises_whole_file_incremental_pixels_and_snapshot_completeness() {
    for progressive in [false, true] {
        for placement in [
            Placement::Header,
            Placement::AfterFirstScan,
            Placement::BeforeEoi,
            Placement::SplitIcc,
        ] {
            let source = fixture(progressive, placement);
            for orient in [OrientationHint::Preserve, OrientationHint::Correct] {
                let buffered = JpegDecoderConfig::new()
                    .job()
                    .with_orientation(orient)
                    .decoder(Cow::Borrowed(&source), &[])
                    .unwrap()
                    .decode()
                    .unwrap();
                let mut observed = Observed::new(
                    JpegDecoderConfig::new()
                        .job()
                        .with_orientation(orient)
                        .dyn_incremental_decoder(&[])
                        .unwrap(),
                );
                let mut sink = CollectSink::new();
                let (output, _) =
                    drive_from_slice(&mut observed, &source, &mut sink, None, 7).unwrap();
                assert!(sink.finished());
                assert_eq!(
                    (output.width, output.height),
                    (buffered.width(), buffered.height())
                );
                let pixels = buffered.pixels();
                for y in 0..pixels.rows() {
                    assert_eq!(
                        sink.as_pixel_slice().unwrap().row(y),
                        pixels.row(y),
                        "{progressive}/{placement:?}/{orient:?}/row{y}"
                    );
                }
                assert!(!observed.metadata.is_empty());
                assert!(
                    observed
                        .row_inputs
                        .iter()
                        .all(|(n, eof)| *n == source.len() && *eof)
                );
                let header = &observed.metadata[0];
                let final_metadata = buffered.info().metadata();
                // Observation, not a desired assertion that metadata must stay
                // incomplete. The report reveals discrepancies in current main.
                println!(
                    "JPEG_DISCOVERY progressive={progressive} placement={placement:?} orient={orient:?} file={} header_at={} metadata_events={} header_orientation={:?} header_icc={} header_xmp={} decoded_orientation={:?} decoded_icc={} decoded_xmp={} row_calls={}",
                    source.len(),
                    header.supplied,
                    observed.metadata.len(),
                    header.orientation,
                    header.icc,
                    header.xmp,
                    buffered.info().orientation,
                    final_metadata.icc_profile.as_ref().map_or(0, |v| v.len()),
                    final_metadata.xmp.as_ref().map_or(0, |v| v.len()),
                    observed.row_inputs.len()
                );
            }
        }
    }
}

#[test]
fn native_row_reader_uses_post_scan_orientation_and_agrees_with_buffered_pixels() {
    for progressive in [false, true] {
        let source = fixture(progressive, Placement::BeforeEoi);
        for orient in [OrientationHint::Preserve, OrientationHint::Correct] {
            let buffered = JpegDecoderConfig::new()
                .job()
                .with_orientation(orient)
                .decoder(Cow::Borrowed(&source), &[])
                .unwrap()
                .decode()
                .unwrap();
            let mut stream = JpegDecoderConfig::new()
                .job()
                .with_orientation(orient)
                .streaming_decoder(Cow::Borrowed(&source), &[])
                .unwrap();
            let mut row = 0;
            while let Some((y_start, pixels)) = stream.next_batch().unwrap() {
                assert_eq!(y_start, row);
                for y in 0..pixels.rows() {
                    assert_eq!(pixels.row(y), buffered.pixels().row(row + y));
                }
                row += pixels.rows();
            }
            assert_eq!(row, buffered.height());
        }
    }
}

#[test]
fn missing_xmp_service_and_incomplete_icc_cannot_become_clean_metadata() {
    let source = fixture(true, Placement::BeforeEoi);
    let request = RewriteRequest::new(&source).with_xmp_editing(true);
    assert!(zenjpeg::metadata::rewrite(&request).is_err());
    let exact = RewriteRequest::new(&source).with_policy(MetadataPolicy::PreserveExact);
    let retained = zenjpeg::metadata::rewrite(&exact)
        .unwrap()
        .to_vec()
        .unwrap();
    assert!(retained.windows(PRIVATE.len()).any(|v| v == PRIVATE));
    for count in [0, 2, 3] {
        let jpeg = base(false);
        let mut incomplete = jpeg[..2].to_vec();
        incomplete.extend(icc_part(&profile(), 1, count));
        incomplete.extend_from_slice(&jpeg[2..]);
        let request = RewriteRequest::new(&incomplete);
        assert!(
            zenjpeg::metadata::rewrite(&request).is_err(),
            "ICC count={count}"
        );
    }
}

#[test]
fn required_privacy_checks_the_native_plan_report_before_copying_any_bytes() {
    use policy::*;
    use policy_disclosure::{Obligation, Prepared, Privacy, TranscodeRoute};
    let source = fixture(true, Placement::BeforeEoi);
    let request = RewriteRequest::new(&source);
    let plan = zenjpeg::metadata::rewrite(&request).unwrap();
    let opaque = plan
        .report()
        .iter()
        .find(|e| e.action() == RewriteAction::OpaqueIcc)
        .unwrap();
    let mut engine = Engine::new(PolicySet::default(), 8);
    let mut callback = |q: Question<'_>| -> Result<Decision, zencodec::At<Infallible>> {
        Ok(Decision::choose(q.choices().unwrap().first().unwrap()))
    };
    let evidence = Prepared::opaque(
        &source[opaque.source_range()],
        Carrier::Icc,
        Obligation::UnknownCritical,
        Privacy::OrientationOnlyV1,
        TranscodeRoute::Rewrite,
    )
    .resolve(&mut engine, Context::default(), &mut callback, &NoPulse);
    assert!(evidence.is_err());
    let mut writer = Vec::new();
    if evidence.is_ok() {
        for chunk in plan.chunks() {
            writer.extend_from_slice(chunk);
        }
    }
    assert!(writer.is_empty());
    // A valid metadata-only rewrite still contains uninspected ICC. A strict
    // request refuses before copying the plan; removing the ICC would change
    // the declared rendering contract and is not an offered action.
}

#[test]
fn real_jpeg_requests_run_over_the_reused_imi_model_without_pixel_changes() {
    use zencodec::source::SliceReadAt;
    for progressive in [false, true] {
        let bytes = fixture(progressive, Placement::SplitIcc);
        let expected = JpegDecoderConfig::new().decode(&bytes).unwrap();
        for cache in [0, 131072] {
            let ledger = Rc::new(RefCell::new(transport::Ledger::default()));
            let config = transport::Config {
                ranges: true,
                cache_bytes: cache,
                latency_ms: 160.0,
                bytes_per_ms: 12500.0,
                concurrency: 8,
                merge_gap: 4096,
                max_range: 131072,
                part_size: 0,
                shared_cache: true,
            };
            let mut source = transport::Transport::new(
                SliceReadAt::new(&bytes),
                "jpeg".into(),
                config,
                ledger.clone(),
            )
            .unwrap();
            let mut decoder = Observed::new(
                JpegDecoderConfig::new()
                    .job()
                    .dyn_incremental_decoder(&[])
                    .unwrap(),
            );
            let mut sink = CollectSink::new();
            drive_from_read_at(
                &mut decoder,
                &mut source,
                &mut sink,
                None,
                DriveOptions::new(0, 7),
            )
            .unwrap();
            assert_eq!(
                (sink.width(), sink.height()),
                (expected.width(), expected.height())
            );
            for y in 0..sink.height() {
                assert_eq!(
                    sink.as_pixel_slice().unwrap().row(y),
                    expected.pixels().row(y)
                );
            }
            let data = ledger.borrow();
            println!(
                "JPEG_TRANSPORT progressive={progressive} cache={cache} file={} fetched={} rounds={} simulated_ms={:.5} header_at={} header_icc={}",
                bytes.len(),
                data.transferred_bytes,
                data.rounds,
                data.now_ms,
                decoder.metadata[0].supplied,
                decoder.metadata[0].icc
            );
            assert!(source.retained() <= cache);
        }
    }
}

#[test]
fn inventory_and_rendering_scope_are_distinct_and_full_probe_is_observed() {
    use zenjpeg::container::metadata::JpegMetadata;
    for progressive in [false, true] {
        for placement in [
            Placement::Header,
            Placement::AfterFirstScan,
            Placement::BeforeEoi,
            Placement::SplitIcc,
        ] {
            let bytes = fixture(progressive, placement);
            let inventory = JpegMetadata::parse(&bytes).unwrap();
            let inspected = inventory.icc_profile();
            let independent = oracle_icc(&bytes);
            match placement {
                Placement::Header => assert_eq!(inspected.unwrap().unwrap().as_ref(), profile()),
                Placement::AfterFirstScan | Placement::BeforeEoi => {
                    assert!(inspected.unwrap().is_none())
                }
                Placement::SplitIcc => assert!(inspected.is_err()),
            }
            // djpeg calls ICC extraction after start_decompress, which can
            // consume all progressive scans. Observe this independently of the
            // header-only accessor, without certifying either as universal scope.
            println!(
                "JPEG_ORACLE progressive={progressive} placement={placement:?} bytes={} status={} diagnostic={:?}",
                independent.bytes.len(),
                independent.status,
                independent.diagnostic
            );
            if independent.bytes.is_empty() {
                assert!(independent.diagnostic.contains("ICC"));
                assert!([0, 2].contains(&independent.status));
            } else {
                assert_eq!(independent.bytes, profile());
                assert_eq!(independent.status, 0);
            }
            let cfg = JpegDecoderConfig::new();
            let header = cfg.probe_header(&bytes).unwrap();
            let full = cfg.probe_full_metadata(&bytes).unwrap();
            let decoded = cfg.decode(&bytes).unwrap();
            let icc_len =
                |info: &ImageInfo| info.metadata().icc_profile.as_ref().map_or(0, |v| v.len());
            println!(
                "JPEG_SCOPE progressive={progressive} placement={placement:?} header_icc={} full_probe_icc={} decoded_icc={} inventory_items={} independent_icc={}",
                icc_len(&header),
                icc_len(&full),
                icc_len(decoded.info()),
                inventory.items().len(),
                independent.bytes.len()
            );
        }
    }
}

#[test]
fn complete_reversed_header_icc_reassembles_but_a_missing_part_never_passes_inspection() {
    use zenjpeg::container::metadata::JpegMetadata;
    let jpeg = base(true);
    let icc = profile();
    let mid = icc.len() / 2;
    let first = icc_part(&icc[..mid], 1, 2);
    let second = icc_part(&icc[mid..], 2, 2);
    for (name, chunks) in [
        ("reversed", [second, first.clone()].concat()),
        ("missing", first),
    ] {
        let bytes = [&jpeg[..2], &chunks, &jpeg[2..]].concat();
        let inventory = JpegMetadata::parse(&bytes).unwrap();
        let inspected = inventory.icc_profile();
        let independent = oracle_icc(&bytes);
        if name == "reversed" {
            assert_eq!(inspected.unwrap().unwrap().as_ref(), icc);
            assert_eq!(independent.bytes, icc);
            assert_eq!(independent.status, 0);
        } else {
            assert!(inspected.is_err());
            assert!(independent.bytes.is_empty());
            assert_eq!(independent.status, 2);
            assert!(independent.diagnostic.contains("bad ICC marker"));
        }
        let probe = JpegDecoderConfig::new().probe_full_metadata(&bytes);
        println!(
            "JPEG_CHUNKS case={name} full_probe={:?}",
            probe.map(|i| i.metadata().icc_profile.as_ref().map_or(0, |v| v.len()))
        );
    }
}

#[test]
fn unknown_length_short_reads_do_not_publish_rows_before_full_input() {
    use std::io::Read;
    struct Short<'a> {
        rest: &'a [u8],
        max: usize,
    }
    impl Read for Short<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = out.len().min(self.max).min(self.rest.len());
            out[..n].copy_from_slice(&self.rest[..n]);
            self.rest = &self.rest[n..];
            Ok(n)
        }
    }
    for progressive in [false, true] {
        let bytes = fixture(progressive, Placement::BeforeEoi);
        let expected = JpegDecoderConfig::new().decode(&bytes).unwrap();
        for chunk in [1, 7, 4096] {
            let mut decoder = Observed::new(
                JpegDecoderConfig::new()
                    .job()
                    .dyn_incremental_decoder(&[])
                    .unwrap(),
            );
            let mut sink = CollectSink::new();
            zencodec_io::pump_read_to_sink(
                Short {
                    rest: &bytes,
                    max: chunk,
                },
                &mut decoder,
                &mut sink,
                None,
                7,
                32768,
            )
            .unwrap();
            assert!(sink.finished());
            for y in 0..sink.height() {
                assert_eq!(
                    sink.as_pixel_slice().unwrap().row(y),
                    expected.pixels().row(y)
                );
            }
            assert!(
                decoder
                    .row_inputs
                    .iter()
                    .all(|(n, eof)| *n == bytes.len() && *eof)
            );
            println!(
                "JPEG_SHORT progressive={progressive} chunk={chunk} file={} header_at={} metadata_events={} header_icc={} rows_at={}",
                bytes.len(),
                decoder.metadata[0].supplied,
                decoder.metadata.len(),
                decoder.metadata[0].icc,
                decoder.row_inputs[0].0
            );
        }
    }
}

#[test]
fn guide_executes_native_rewrite_and_checks_icc_before_publication() {
    let mut callback_calls = 0;
    let mut resolver =
        |_: policy::Question<'_>| -> Result<policy::Decision, zencodec::At<Infallible>> {
            callback_calls += 1;
            Ok(policy::Decision::UsePreset)
        };
    for progressive in [false, true] {
        let source = fixture(progressive, Placement::BeforeEoi);
        let ordinary = crate::guide::derivative(&source).unwrap();
        assert_eq!(scans(&ordinary), scans(&source));
        assert_eq!(oracle(&ordinary), oracle(&source));
        let mut writer = Vec::new();
        let error =
            crate::guide::require_inspected_icc(&source, &mut writer, &mut resolver).unwrap_err();
        assert!(
            error
                .downcast_ref::<zencodec::At<policy::Failure<Infallible>>>()
                .is_some()
        );
        assert!(writer.is_empty());

        let error = crate::guide::checked_derivative(&source, &mut writer).unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::guide::UninspectedIcc>()
                .is_some()
        );
        assert!(writer.is_empty());

        let clean = base(progressive);
        crate::guide::require_inspected_icc(&clean, &mut writer, &mut resolver).unwrap();
        assert_eq!(scans(&writer), scans(&clean));
        assert_eq!(oracle(&writer), oracle(&clean));
        let mut checked = Vec::new();
        crate::guide::checked_derivative(&clean, &mut checked).unwrap();
        assert_eq!(scans(&checked), scans(&clean));
        assert_eq!(oracle(&checked), oracle(&clean));
    }
    // Impossible requests are rejected by admission, not sent to application
    // code to negotiate away a hard requirement.
    assert_eq!(callback_calls, 0);
}
