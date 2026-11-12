//! Optional observation preserves the unobserved and borrowed-observer paths.

mod common;

use common::{packed, pattern, slice};
use std::sync::atomic::{AtomicU64, Ordering};
use zencodec::decode::{DecodeCodec, DecodePlan, DecodeSpec, DecodeStep, Decoder, Sink};
use zencodec::{ImageInfo, Input, InputRequest, Pulse, Result, Stop, StopReason};
use zencodec_fixtures::rows::RowsCodec;
use zencodec_io::{Collect, decode_slice, encode};
use zenpixels::PixelDescriptor;

struct Observer<'a>(&'a AtomicU64);

impl Stop for Observer<'_> {
    fn check(&self) -> std::result::Result<(), StopReason> {
        Ok(())
    }
}

impl how_far::Report for Observer<'_> {
    fn advance(&self, units: u64) {
        self.0.fetch_add(units, Ordering::SeqCst);
    }
}

impl Pulse for Observer<'_> {
    fn split(
        &self,
        _: how_far::Execution,
        _: &[how_far::PhaseSpec<'_>],
    ) -> std::result::Result<Vec<how_far::Child<'_>>, how_far::PlanError> {
        Err(how_far::PlanError::Unsupported)
    }
}

struct Reporting(Box<dyn Decoder>);

impl Decoder for Reporting {
    fn requested(&self) -> &[InputRequest] {
        self.0.requested()
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.0.info()
    }

    fn plan(&self) -> Option<&DecodePlan> {
        self.0.plan()
    }

    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn Sink,
        pulse: Option<&dyn Pulse>,
    ) -> Result<DecodeStep> {
        let step = self.0.step(input, sink, pulse)?;
        if let Some(pulse) = pulse {
            pulse.advance(1);
        }
        Ok(step)
    }
}

#[test]
fn none_and_a_borrowed_unshareable_reporting_pulse_preserve_pixels() {
    let descriptor = PixelDescriptor::RGB8_SRGB;
    let source = pattern(17, 29, descriptor, 23);
    let pixels = slice(&source, 17, 29, 17 * 3, descriptor);
    let bytes = encode(
        &RowsCodec,
        zencodec::encode::EncodeSpec::for_pixels(&pixels),
        pixels,
        None,
    )
    .unwrap();
    let reports = AtomicU64::new(0);
    let observer = Observer(&reports);
    assert!(
        observer.share().is_err(),
        "this observer cannot escape its borrow"
    );
    for pulse in [None, Some(&observer as &dyn Pulse)] {
        let decoder = Reporting(RowsCodec.decoder(&DecodeSpec::new()).unwrap());
        let mut collect = Collect::new();
        decode_slice(Box::new(decoder), &bytes, &mut collect, pulse).unwrap();
        assert_eq!(packed(&collect.frames()[0]), source);
        if pulse.is_none() {
            assert_eq!(reports.load(Ordering::SeqCst), 0);
        }
    }
    assert!(
        reports.load(Ordering::SeqCst) > 0,
        "progress reached the borrowed observer"
    );
}

#[cfg(feature = "tokio")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_during_a_short_read_prevents_another_source_call() {
    use common::Switch;
    use zencodec_io::tokio_io::{self, AsyncSource, Offload};

    struct StopsOnRead<'a> {
        bytes: &'a [u8],
        switch: Switch,
        reads: usize,
    }

    impl AsyncSource for StopsOnRead<'_> {
        async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            let at = (offset as usize).min(self.bytes.len());
            let count = out.len().min(self.bytes.len() - at).min(1);
            out[..count].copy_from_slice(&self.bytes[at..at + count]);
            self.switch.stop();
            Ok(count)
        }
    }

    let descriptor = PixelDescriptor::RGB8_SRGB;
    let source = pattern(17, 29, descriptor, 23);
    let pixels = slice(&source, 17, 29, 17 * 3, descriptor);
    let bytes = encode(
        &RowsCodec,
        zencodec::encode::EncodeSpec::for_pixels(&pixels),
        pixels,
        None,
    )
    .unwrap();
    for offload in [Offload::Inline, Offload::InPlace, Offload::Blocking] {
        let switch = Switch::default();
        let mut input = StopsOnRead {
            bytes: &bytes,
            switch: switch.clone(),
            reads: 0,
        };
        let error = tokio_io::decode(
            RowsCodec.decoder(&DecodeSpec::new()).unwrap(),
            &mut input,
            Collect::new(),
            offload,
            zencodec_io::DriveOptions::new(),
            Some(switch.pulse()),
        )
        .await
        .err()
        .expect("a source-signalled stop must end the operation");
        assert_eq!(
            error.error().kind(),
            zencodec::ErrorKind::Stopped(StopReason::Cancelled)
        );
        assert_eq!(input.reads, 1, "no second source call after cancellation");
    }
}
