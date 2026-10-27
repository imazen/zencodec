//! Native scheduling qualification. These observations are not allocation bounds.
use crate::fixtures;
use std::io::{self, Write};
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zenpixels::PixelSlice;

#[derive(Clone, Copy, Debug)]
enum Schedule {
    FinishFirst,
    EachStrip,
    PartialDrain,
}

fn run<E: Encoder>(
    mut encoder: E,
    pixels: &PixelSlice<'_>,
    schedule: Schedule,
) -> (Vec<u8>, bool, usize) {
    let timing = encoder
        .begin(pixels.width(), pixels.rows(), pixels.descriptor())
        .unwrap();
    let mut output = Vec::new();
    let mut observations = 0;
    let mut drain = |encoder: &mut E| {
        let pending = encoder.pending_output().to_vec();
        // A paused consumer must be able to inspect output repeatedly without
        // advancing input, codec state, or the pending byte range.
        assert_eq!(encoder.pending_output(), pending);
        assert_eq!(encoder.pending_output(), pending);
        if !pending.is_empty() {
            observations += 1;
        }
        match schedule {
            Schedule::FinishFirst => {}
            Schedule::EachStrip => {
                output.extend_from_slice(&pending);
                encoder.consume_output(pending.len());
                assert!(encoder.pending_output().is_empty());
            }
            Schedule::PartialDrain => {
                let mut consumed = 0;
                while consumed < pending.len() {
                    let n = (pending.len() - consumed).min(7);
                    output.extend_from_slice(&encoder.pending_output()[..n]);
                    encoder.consume_output(n);
                    consumed += n;
                    assert_eq!(encoder.pending_output(), &pending[consumed..]);
                }
            }
        }
    };
    drain(&mut encoder);
    for y in 0..pixels.rows() {
        encoder.push_rows(pixels.sub_rows(y, 1)).unwrap();
        drain(&mut encoder);
    }
    let rest = encoder.finish().unwrap();
    output.extend_from_slice(rest.data());
    (output, timing.streams(), observations)
}

#[test]
fn native_jpeg_output_scheduling_preserves_exact_bytes_and_configuration() {
    let raw = fixtures::raw();
    let full = fixtures::pixels(&raw);
    for progressive in [false, true] {
        for optimized in [false, true] {
            // Tiny height refuses a misleading header-only streaming claim.
            for height in [8, fixtures::HEIGHT] {
                let pixels = full.sub_rows(0, height);
                let config = fixtures::config(progressive, optimized);
                let (expected, streams, _) = run(
                    config.clone().job().encoder().unwrap(),
                    &pixels,
                    Schedule::FinishFirst,
                );
                assert_eq!(
                    streams,
                    !progressive && !optimized && height == fixtures::HEIGHT
                );
                let decoded = fixtures::oracle(&expected);
                for schedule in [Schedule::EachStrip, Schedule::PartialDrain] {
                    let (actual, actual_streams, pending) =
                        run(config.clone().job().encoder().unwrap(), &pixels, schedule);
                    assert_eq!(actual_streams, streams);
                    assert_eq!(
                        actual, expected,
                        "{progressive}/{optimized}/{height}/{schedule:?}"
                    );
                    assert_eq!(fixtures::oracle(&actual), decoded);
                    assert_eq!(pending > 0, streams);
                }
                // Shared convenience must also preserve the requested codec
                // settings and produce identical bytes despite different batches.
                let mut writer = ShortWriter(Vec::new());
                let report = zencodec_io::encode_to_writer(
                    config.clone().job().encoder().unwrap(),
                    pixels,
                    &mut writer,
                )
                .unwrap();
                assert_eq!(report.timing.streams(), streams);
                assert_eq!(report.bytes as usize, writer.0.len());
                assert_eq!(writer.0, expected);
            }
        }
    }
}

struct ShortWriter(Vec<u8>);
impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = bytes.len().min(3);
        self.0.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// This is the executing adapter witness for the shared schedule resolver.
// Source, encoder and destination stay held across selection; no re-preparation.
fn execute_prepared<E: Encoder, R: zencodec::operation::PolicyResolver>(
    mut encoder: E,
    mut resolver: R,
    source_calls: &std::cell::Cell<u32>,
    writer_calls: &std::cell::Cell<u32>,
    writer: &mut impl Write,
) -> Result<(u64, zencodec::operation::DecisionRecord), Box<dyn std::error::Error>> {
    use zencodec::operation::{
        OutputSchedule, OutputScheduleFacts, OutputScheduleOffer, OutputSource, PolicySet,
        resolve_output_schedule,
    };
    use zenpixels::PixelDescriptor;
    let timing = encoder.begin(
        fixtures::WIDTH,
        fixtures::HEIGHT,
        PixelDescriptor::RGB8_SRGB,
    )?;
    let mut offers = vec![OutputScheduleOffer::new(OutputSchedule::FinishFirst)];
    if timing.streams() {
        offers.push(OutputScheduleOffer::new(OutputSchedule::Live));
    }
    let (index, record) = resolve_output_schedule(
        OutputScheduleFacts::new(timing, OutputSource::PullRows),
        &offers,
        &mut resolver,
        PolicySet::new(),
        &zencodec::enough::Unstoppable,
    )?;
    let schedule = offers[index].schedule;
    let mut emitted = 0u64;
    let mut drain = |encoder: &mut E| -> io::Result<()> {
        if schedule == OutputSchedule::Live && !encoder.pending_output().is_empty() {
            writer_calls.set(writer_calls.get() + 1);
            writer.write_all(encoder.pending_output())?;
            let len = encoder.pending_output().len();
            encoder.consume_output(len);
            emitted += len as u64;
        }
        Ok(())
    };
    drain(&mut encoder)?;
    // Own only one strided source row. No borrowed complete raster enters this
    // route. Native retained memory is still unknown, so no bound is claimed.
    let mut row = vec![0xa5; fixtures::STRIDE];
    let mut random = 0x12345678u32;
    for _ in 0..fixtures::HEIGHT {
        source_calls.set(source_calls.get() + 1);
        for byte in &mut row[..fixtures::WIDTH as usize * 3] {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            *byte = random as u8;
        }
        encoder.push_rows(PixelSlice::new(
            &row,
            fixtures::WIDTH,
            1,
            fixtures::STRIDE,
            PixelDescriptor::RGB8_SRGB,
        )?)?;
        drain(&mut encoder)?;
    }
    drop(row);
    let rest = encoder.finish()?;
    if !rest.is_empty() {
        writer_calls.set(writer_calls.get() + 1);
        writer.write_all(rest.data())?;
        emitted += rest.len() as u64;
    }
    Ok((emitted, record))
}

#[test]
fn native_jpeg_prepared_schedule_policy_runs_before_source_or_destination_effects() {
    use std::cell::Cell;
    use zencodec::operation::{OutputSchedule, PolicySet, Selection};
    let raw = fixtures::raw();
    for optimized in [false, true] {
        for schedule in [OutputSchedule::FinishFirst, OutputSchedule::Live] {
            for bound in [None, Some(0)] {
                let source_calls = Cell::new(0);
                let writer_calls = Cell::new(0);
                let mut policy = PolicySet::new().require_output_schedule(schedule);
                if let Some(bytes) = bound {
                    policy = policy.with_output_wait_limit(bytes);
                }
                let config = fixtures::config(false, optimized);
                let mut output = ShortWriter(Vec::new());
                let result = execute_prepared(
                    config.clone().job().encoder().unwrap(),
                    policy,
                    &source_calls,
                    &writer_calls,
                    &mut output,
                );
                if bound.is_some() || (optimized && schedule == OutputSchedule::Live) {
                    let error = result.unwrap_err();
                    assert!(
                        zencodec::find_cause::<zencodec::operation::PolicyFailure>(error.as_ref())
                            .is_some()
                    );
                    assert_eq!(source_calls.get(), 0);
                    assert_eq!(writer_calls.get(), 0);
                    assert!(output.0.is_empty());
                } else {
                    let (emitted, record) = result.unwrap();
                    let bytes = output.0;
                    assert_eq!(emitted as usize, bytes.len());
                    assert!(
                        matches!(record.selected, Some(Selection::OutputSchedule(o)) if o.schedule == schedule && o.wait_retained_upper_bound.is_none())
                    );
                    assert_eq!(source_calls.get(), fixtures::HEIGHT);
                    let (reference, _, _) = run(
                        config.job().encoder().unwrap(),
                        &fixtures::pixels(&raw),
                        Schedule::FinishFirst,
                    );
                    assert_eq!(bytes, reference);
                    assert_eq!(fixtures::oracle(&bytes), fixtures::oracle(&reference));
                }
            }
        }
    }
}
