//! The contract through the Tokio drivers.
#![cfg(feature = "tokio")]

mod common;

use common::*;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use zencodec::decode::{
    DecodeCodec, DecodeEvent, DecodePlan, DecodeSpec, DecodeStep, Decoder, Sink,
};
use zencodec::encode::{EncodeCodec, EncodeSpec};
use zencodec::{
    ErrorKind, ImageInfo, Input, InputRequest, Metadata, MetadataKinds, NoPulse, Pulse, Raster,
    Result, SharedPulse, StopReason,
};
use zencodec_fixtures::bias::BiasCodec;
use zencodec_fixtures::rows::RowsCodec;
use zencodec_io::tokio_io::{
    self, AsyncSource, InMemory, Offload, Streamed, decode_buffered, transcode_buffered,
};
use zencodec_io::{Collect, DriveOptions, EncoderSink, RangeRead, encode, transcode};
use zenpixels::PixelDescriptor;

/// A source that yields to the runtime and then hands out a few bytes.
#[derive(Clone)]
struct Trickle {
    bytes: Arc<[u8]>,
    chunk: usize,
    reads: Arc<AtomicUsize>,
    range_calls: Arc<AtomicUsize>,
}

impl Trickle {
    fn new(bytes: &[u8], chunk: usize) -> Self {
        Self {
            bytes: bytes.into(),
            chunk,
            reads: Arc::default(),
            range_calls: Arc::default(),
        }
    }
}

impl AsyncSource for Trickle {
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        tokio::task::yield_now().await;
        self.reads.fetch_add(1, Ordering::SeqCst);
        let from = (offset as usize).min(self.bytes.len());
        let n = out.len().min(self.chunk).min(self.bytes.len() - from);
        out[..n].copy_from_slice(&self.bytes[from..from + n]);
        Ok(n)
    }

    async fn total_len(&mut self) -> io::Result<Option<u64>> {
        Ok(Some(self.bytes.len() as u64))
    }

    async fn read_ranges(&mut self, ranges: &mut [RangeRead]) -> io::Result<()> {
        tokio::task::yield_now().await;
        self.range_calls.fetch_add(1, Ordering::SeqCst);
        for range in ranges {
            let from = (range.offset as usize).min(self.bytes.len());
            let n = range.buf.len().min(self.bytes.len() - from);
            range.buf.truncate(n);
            range.buf.copy_from_slice(&self.bytes[from..from + n]);
        }
        Ok(())
    }
}

fn offloads() -> Vec<Offload> {
    vec![
        Offload::Inline,
        Offload::Blocking,
        Offload::Custom(Arc::new(|job| {
            std::thread::spawn(job);
        })),
    ]
}

fn sample(codec: &dyn EncodeCodec, w: u32, h: u32, seed: u8) -> (Vec<u8>, Vec<u8>) {
    let d = PixelDescriptor::RGB8_SRGB;
    let source = pattern(w, h, d, seed);
    let bytes = encode(
        codec,
        EncodeSpec::new(Raster::packed(w, h, d)),
        slice(&source, w, h, w as usize * 3, d),
        &NoPulse,
    )
    .unwrap();
    (bytes, source)
}

fn to(codec: Arc<dyn EncodeCodec>) -> EncoderSink {
    observed(codec, SharedPulse::default())
}

/// A transcoding sink whose encoder observes `pulse`.
fn observed(codec: Arc<dyn EncodeCodec>, pulse: SharedPulse) -> EncoderSink {
    EncoderSink::new(codec, |frame| EncodeSpec::new(frame.raster.clone()), pulse)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trickling_source_decodes_on_every_executor() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, source) = sample(&**enc, 17, 23, 1);
        for offload in offloads() {
            let mut trickle = Trickle::new(&bytes, 3);
            let opts = DriveOptions::new()
                .with_read_size(5)
                .with_steps_per_batch(2);
            let (decoder, collect) = tokio_io::decode(
                dec.decoder(&DecodeSpec::new()).unwrap(),
                &mut trickle,
                Collect::new(),
                offload.clone(),
                opts,
                SharedPulse::default(),
            )
            .await
            .expect("async decode");
            assert_eq!(
                packed(&collect.frames()[0]),
                source,
                "{} {offload:?}",
                enc.name()
            );
            assert_eq!(decoder.info().unwrap().width, 17);
            assert!(trickle.reads.load(Ordering::SeqCst) > 10);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_async_transcode_writes_what_a_blocking_one_does() {
    for (src_enc, src_dec) in encoders().iter().zip(decoders()) {
        for dst_enc in encoders() {
            let (bytes, _) = sample(&**src_enc, 19, 41, 2);
            let reference = transcode(
                src_dec.decoder(&DecodeSpec::new()).unwrap(),
                &bytes,
                dst_enc.clone(),
                |frame| EncodeSpec::new(frame.raster.clone()),
                &NoPulse,
            )
            .unwrap();
            for offload in offloads() {
                for pipeline in [true, false] {
                    let mut trickle = Trickle::new(&bytes, 64);
                    let mut out = Vec::new();
                    let opts = DriveOptions::new()
                        .with_read_size(64)
                        .with_steps_per_batch(3)
                        .with_pipeline_writes(pipeline);
                    let written = tokio_io::transcode(
                        src_dec.decoder(&DecodeSpec::new()).unwrap(),
                        to(dst_enc.clone()),
                        &mut trickle,
                        &mut out,
                        offload.clone(),
                        opts,
                        SharedPulse::default(),
                    )
                    .await
                    .expect("async transcode");
                    assert_eq!(written, reference.len() as u64);
                    assert_eq!(
                        out,
                        reference,
                        "{} -> {} {offload:?} pipeline={pipeline}",
                        src_enc.name(),
                        dst_enc.name()
                    );
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_reader_of_the_output_only_slows_it() {
    use tokio::io::AsyncReadExt;
    let (bytes, _) = sample(&RowsCodec, 32, 200, 3);
    let reference = transcode(
        RowsCodec.decoder(&DecodeSpec::new()).unwrap(),
        &bytes,
        Arc::new(RowsCodec),
        |frame| EncodeSpec::new(frame.raster.clone()),
        &NoPulse,
    )
    .unwrap();

    // A 64-byte pipe: the writer blocks until the reader takes bytes.
    let (mut tx, mut rx) = tokio::io::duplex(64);
    let reader = tokio::spawn(async move {
        let mut got = Vec::new();
        let mut buf = [0u8; 48];
        loop {
            tokio::task::yield_now().await;
            match rx.read(&mut buf).await.unwrap() {
                0 => return got,
                n => got.extend_from_slice(&buf[..n]),
            }
        }
    });
    // The driver's future is `Send`: it is spawned, not just awaited.
    let writer = tokio::spawn(async move {
        let mut source = Trickle::new(&bytes, 100);
        let written = tokio_io::transcode(
            RowsCodec.decoder(&DecodeSpec::new()).unwrap(),
            to(Arc::new(RowsCodec)),
            &mut source,
            &mut tx,
            Offload::Blocking,
            DriveOptions::new()
                .with_read_size(100)
                .with_steps_per_batch(4),
            SharedPulse::default(),
        )
        .await
        .unwrap();
        drop(tx);
        written
    });
    let written = writer.await.unwrap();
    let got = reader.await.unwrap();
    assert_eq!(written, reference.len() as u64);
    assert_eq!(got, reference);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_buffered_route_is_one_hand_off() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, source) = sample(&**enc, 12, 12, 4);
        let (decoder, collect) = decode_buffered(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            bytes.clone(),
            Collect::new(),
            SharedPulse::default(),
        )
        .await
        .unwrap();
        assert_eq!(packed(&collect.frames()[0]), source);
        assert!(decoder.info().is_some());

        let reference = transcode(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &bytes,
            Arc::new(RowsCodec),
            |frame| EncodeSpec::new(frame.raster.clone()),
            &NoPulse,
        )
        .unwrap();
        let out = transcode_buffered(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            Arc::<[u8]>::from(&bytes[..]),
            to(Arc::new(RowsCodec)),
            SharedPulse::default(),
        )
        .await
        .unwrap();
        assert_eq!(out, reference, "{}", enc.name());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forward_only_stream_is_enough_for_a_sequential_decoder() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, source) = sample(&**enc, 9, 30, 5);
        let mut stream = Streamed::new(&bytes[..]);
        let (_, collect) = tokio_io::decode(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &mut stream,
            Collect::new(),
            Offload::Inline,
            DriveOptions::new().with_read_size(11),
            SharedPulse::default(),
        )
        .await
        .unwrap();
        assert_eq!(packed(&collect.frames()[0]), source, "{}", enc.name());
    }
    // In-memory bytes are a source too.
    let (bytes, source) = sample(&RowsCodec, 9, 30, 5);
    let (_, collect) = tokio_io::decode(
        RowsCodec.decoder(&DecodeSpec::new()).unwrap(),
        &mut InMemory(bytes),
        Collect::new(),
        Offload::Blocking,
        DriveOptions::new(),
        SharedPulse::default(),
    )
    .await
    .unwrap();
    assert_eq!(packed(&collect.frames()[0]), source);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_ends_a_transcode_and_leaves_a_prefix() {
    let (bytes, _) = sample(&RowsCodec, 16, 300, 6);
    let reference = transcode(
        RowsCodec.decoder(&DecodeSpec::new()).unwrap(),
        &bytes,
        Arc::new(RowsCodec),
        |frame| EncodeSpec::new(frame.raster.clone()),
        &NoPulse,
    )
    .unwrap();
    for offload in offloads() {
        let switch = Switch::tripping_at(40);
        let mut source = Trickle::new(&bytes, 200);
        let mut out = Vec::new();
        let err = tokio_io::transcode(
            RowsCodec.decoder(&DecodeSpec::new()).unwrap(),
            observed(Arc::new(RowsCodec), switch.pulse()),
            &mut source,
            &mut out,
            offload.clone(),
            DriveOptions::new()
                .with_read_size(200)
                .with_steps_per_batch(2),
            switch.pulse(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.error().kind(),
            ErrorKind::Stopped(StopReason::Cancelled),
            "{offload:?}"
        );
        assert!(out.len() < reference.len());
        assert_eq!(out, reference[..out.len()], "what was written is a prefix");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_wanted_first_costs_one_range_fetch() {
    let (w, h, d) = (6, 50, PixelDescriptor::RGB8_SRGB);
    let source = pattern(w, h, d, 7);
    let exif = b"verbatim trailer";
    let spec = EncodeSpec::new(Raster::packed(w, h, d)).with_metadata(
        Metadata::none().with_exif(exif.to_vec()),
        MetadataKinds::EXIF,
    );
    let bytes = encode(&BiasCodec, spec, slice(&source, w, h, 18, d), &NoPulse).unwrap();

    let first = DecodeSpec::new().with_metadata_first(MetadataKinds::EXIF);
    let mut trickle = Trickle::new(&bytes, 4);
    let (decoder, collect) = tokio_io::decode(
        BiasCodec.decoder(&first).unwrap(),
        &mut trickle,
        Collect::new(),
        Offload::Blocking,
        DriveOptions::new().with_read_size(4),
        SharedPulse::default(),
    )
    .await
    .unwrap();
    assert_eq!(trickle.range_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        decoder.info().unwrap().metadata.exif.as_deref(),
        Some(&exif[..])
    );
    assert_eq!(packed(&collect.frames()[0]), source);
}

/// Panics on its first step.
struct Panics([InputRequest; 1]);
impl Decoder for Panics {
    fn requested(&self) -> &[InputRequest] {
        &self.0
    }
    fn info(&self) -> Option<&ImageInfo> {
        None
    }
    fn plan(&self) -> Option<&DecodePlan> {
        None
    }
    fn step(&mut self, _: Input<'_>, _: &mut dyn Sink, _: &dyn Pulse) -> Result<DecodeStep> {
        if self.0.len() == 1 {
            panic!("codec bug");
        }
        Ok(DecodeStep::new(0, DecodeEvent::Done))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_codec_that_dies_on_its_worker_is_an_error_not_a_hang() {
    let request = [InputRequest::AtLeast {
        offset: 0,
        min_len: 1,
    }];
    let lossy: Offload = Offload::Custom(Arc::new(drop));
    for offload in [Offload::Blocking, lossy] {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            tokio_io::decode(
                Box::new(Panics(request)),
                &mut InMemory(vec![0u8; 8]),
                Collect::new(),
                offload.clone(),
                DriveOptions::new(),
                SharedPulse::default(),
            ),
        )
        .await
        .expect("no hang");
        let err = result.err().expect("an error");
        assert_eq!(err.error().kind(), ErrorKind::Internal, "{offload:?}");
    }
}

/// Bytes in memory that must never be read: only borrowed.
struct Resident(Vec<u8>);
impl AsyncSource for Resident {
    async fn read_at(&mut self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        panic!("a resident source was read instead of borrowed");
    }
    fn as_slice(&self) -> Option<&[u8]> {
        Some(&self.0)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_inline_driver_borrows_bytes_already_in_memory() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, source) = sample(&**enc, 14, 90, 8);
        let opts = DriveOptions::new().with_steps_per_batch(3);
        let (_, collect) = tokio_io::decode(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &mut Resident(bytes.clone()),
            Collect::new(),
            Offload::Inline,
            opts,
            SharedPulse::default(),
        )
        .await
        .unwrap();
        assert_eq!(packed(&collect.frames()[0]), source, "{}", enc.name());

        let reference = transcode(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &bytes,
            Arc::new(RowsCodec),
            |frame| EncodeSpec::new(frame.raster.clone()),
            &NoPulse,
        )
        .unwrap();
        let mut out = Vec::new();
        let written = tokio_io::transcode(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            to(Arc::new(RowsCodec)),
            &mut Resident(bytes),
            &mut out,
            Offload::Inline,
            opts,
            SharedPulse::default(),
        )
        .await
        .unwrap();
        assert_eq!(written, reference.len() as u64);
        assert_eq!(out, reference, "{}", enc.name());
    }
}
