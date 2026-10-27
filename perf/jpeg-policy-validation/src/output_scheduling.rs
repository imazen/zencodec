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
