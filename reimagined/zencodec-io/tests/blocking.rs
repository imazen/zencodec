//! The contract through the blocking drivers.

mod common;

use common::*;
use std::io::Cursor;
use std::sync::Arc;
use zencodec::decode::{
    Accept, Access, DecodeCodec, DecodeEvent, DecodePlan, DecodeSpec, DecodeStep, Decoder, Frames,
    Sink,
};
use zencodec::encode::{
    EncodeCodec, EncodeEvent, EncodeInput, EncodeSpec, FrameMeta, OutputTiming, RowDemand,
    RowSupply, SequenceSpec,
};
use zencodec::{
    Bound, Demand, ErrorKind, Fidelity, FrameDuration, ImageInfo, Input, InputRequest,
    LimitExceeded, LimitKind, LossyTarget, Metadata, MetadataKinds, NoPulse, OutputAccess, Pass,
    Pulse, Raster, ResourceLimits, Result, Sequence, StopReason, Unsupported, find_cause,
};
use zencodec_fixtures::bias::BiasCodec;
use zencodec_fixtures::rle::RleCodec;
use zencodec_fixtures::rows::{RowsCodec, bt601_full, planar_raster, yuv420};
use zencodec_fixtures::{FixtureError, PACKED};
use zencodec_io::{
    Collect, DriveOptions, EncoderSink, Forward, SeekBack, Sequential, decode, decode_read_at,
    decode_slice, encode, encode_sequence, feed_frame, transcode, write_output,
};
use zenpixels::plane::{AnyPlane, PlanarSlice, PlaneBuf};
use zenpixels::{PixelDescriptor, PixelSliceMut};

const SIZES: [(u32, u32); 4] = [(1, 1), (7, 5), (64, 33), (3, 40)];

fn encoded(
    codec: &dyn EncodeCodec,
    w: u32,
    h: u32,
    d: PixelDescriptor,
    seed: u8,
) -> (Vec<u8>, Vec<u8>) {
    let source = pattern(w, h, d, seed);
    let row = w as usize * d.bytes_per_pixel();
    let bytes = encode(
        codec,
        EncodeSpec::new(Raster::packed(w, h, d)),
        slice(&source, w, h, row, d),
        &NoPulse,
    )
    .expect("encode");
    (bytes, source)
}

#[test]
fn every_codec_round_trips_every_layout() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        for d in PACKED {
            for (w, h) in SIZES {
                let (bytes, source) = encoded(&**enc, w, h, d, 1);
                let decoded = decode(&*dec, &bytes, &DecodeSpec::new(), &NoPulse).expect("decode");
                assert_eq!(decoded.info.format, enc.format());
                assert_eq!((decoded.info.width, decoded.info.height), (w, h));
                assert_eq!(decoded.info.lossless, Some(true));
                assert_eq!(decoded.frames.len(), 1);
                assert_eq!(packed(&decoded.frames[0]), source, "{} {w}x{h}", enc.name());
            }
        }
    }
}

#[test]
fn strided_input_encodes_the_same_bytes() {
    for enc in encoders() {
        let (w, h, d) = (9, 6, PixelDescriptor::RGB8_SRGB);
        let (reference, source) = encoded(&*enc, w, h, d, 2);
        let (wide, stride) = padded(&source, w, h, d, 5);
        let bytes = encode(
            &*enc,
            EncodeSpec::new(Raster::packed(w, h, d)),
            slice(&wide, w, h, stride, d),
            &NoPulse,
        )
        .unwrap();
        assert_eq!(bytes, reference, "{}", enc.name());
    }
}

#[test]
fn a_one_byte_source_decodes_what_a_slice_decodes() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, source) = encoded(&**enc, 13, 9, PixelDescriptor::RGBA8_SRGB, 3);
        for (chunk, read_size) in [(1, 1), (5, 3), (4096, 64)] {
            let mut drip = Drip::new(&bytes, chunk);
            let mut collect = Collect::new();
            let opts = DriveOptions::new().with_read_size(read_size);
            decode_read_at(
                dec.decoder(&DecodeSpec::new()).unwrap(),
                &mut drip,
                &mut collect,
                &opts,
                &NoPulse,
            )
            .expect("drip decode");
            assert_eq!(
                packed(&collect.frames()[0]),
                source,
                "{} chunk {chunk}",
                enc.name()
            );
        }
        // A forward-only reader is enough when nothing asks to go back.
        let mut forward = Sequential::new(Cursor::new(&bytes));
        let mut collect = Collect::new();
        decode_read_at(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &mut forward,
            &mut collect,
            &DriveOptions::new().with_read_size(7),
            &NoPulse,
        )
        .expect("sequential decode");
        assert_eq!(packed(&collect.frames()[0]), source);
    }
}

#[test]
fn every_codec_transcodes_to_every_codec() {
    for (src_enc, src_dec) in encoders().iter().zip(decoders()) {
        for (dst_enc, dst_dec) in encoders().iter().zip(decoders()) {
            let (bytes, source) = encoded(&**src_enc, 21, 37, PixelDescriptor::RGB8_SRGB, 4);
            let out = transcode(
                src_dec.decoder(&DecodeSpec::new()).unwrap(),
                &bytes,
                dst_enc.clone(),
                |frame| EncodeSpec::new(frame.raster.clone()),
                &NoPulse,
            )
            .expect("transcode");
            let decoded = decode(&*dst_dec, &out, &DecodeSpec::new(), &NoPulse).unwrap();
            assert_eq!(
                packed(&decoded.frames[0]),
                source,
                "{} -> {}",
                src_enc.name(),
                dst_enc.name()
            );
            // Row by row or all at once, the destination file is the same.
            let (direct, _) = encoded(&**dst_enc, 21, 37, PixelDescriptor::RGB8_SRGB, 4);
            assert_eq!(out, direct);
        }
    }
}

#[test]
fn short_loans_change_nothing() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, source) = encoded(&**enc, 11, 19, PixelDescriptor::GRAY8_SRGB, 5);
        let mut sink = Recording {
            max_loan: 1,
            ..Recording::default()
        };
        decode_slice(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &bytes,
            &mut sink,
            &NoPulse,
        )
        .unwrap();
        assert_eq!(packed(&sink.inner.frames()[0]), source, "{}", enc.name());
    }
}

fn frames(n: u8, w: u32, h: u32, d: PixelDescriptor) -> Vec<Vec<u8>> {
    (0..n).map(|i| pattern(w, h, d, 10 + i)).collect()
}

#[test]
fn sequences_round_trip_with_and_without_a_declared_count() {
    let (w, h, d) = (6, 4, PixelDescriptor::RGBA8_SRGB);
    let row = w as usize * 4;
    let sources = frames(3, w, h, d);
    let mut files = Vec::new();
    for sequence in [
        SequenceSpec::new().with_frames(3).with_loops(2),
        SequenceSpec::new().with_loops(2),
    ] {
        let spec = EncodeSpec::new(Raster::packed(w, h, d)).with_sequence(sequence);
        let bytes = encode_sequence(
            &RowsCodec,
            spec,
            sources.iter().enumerate().map(|(i, s)| {
                (
                    slice(s, w, h, row, d),
                    FrameMeta::shown_for(FrameDuration::from_millis(40 * (i as u32 + 1))),
                )
            }),
            &NoPulse,
        )
        .expect("encode sequence");
        files.push(bytes);
    }
    // The unknown count was patched into the header before the bytes left.
    assert_eq!(files[0], files[1]);

    let all = decode(
        &RowsCodec,
        &files[0],
        &DecodeSpec::new().with_frames(Frames::All),
        &NoPulse,
    )
    .unwrap();
    assert_eq!(
        all.info.sequence,
        Sequence::Animation {
            frames: Some(3),
            loops: Some(2)
        }
    );
    assert_eq!(all.frames.len(), 3);
    for (i, frame) in all.frames.iter().enumerate() {
        assert_eq!(frame.header.index, i as u32);
        assert_eq!(
            frame.header.duration,
            Some(FrameDuration::from_millis(40 * (i as u32 + 1)))
        );
        assert_eq!(packed(frame), sources[i]);
    }
    let first = decode(&RowsCodec, &files[0], &DecodeSpec::new(), &NoPulse).unwrap();
    assert_eq!(first.frames.len(), 1);

    // Frame by frame through the transcoding sink.
    let out = transcode(
        RowsCodec
            .decoder(&DecodeSpec::new().with_frames(Frames::All))
            .unwrap(),
        &files[0],
        Arc::new(RowsCodec),
        |frame| {
            EncodeSpec::new(frame.raster.clone()).with_sequence(SequenceSpec::new().with_loops(2))
        },
        &NoPulse,
    )
    .unwrap();
    assert_eq!(out, files[0]);
}

/// Drive a sequence encode, writing to `destination` after every step.
fn stream_sequence(
    spec: EncodeSpec,
    sources: &[Vec<u8>],
    destination: &mut dyn zencodec_io::Destination,
) -> (OutputTiming, Vec<u64>) {
    let (w, h, d) = (
        spec.raster.width,
        spec.raster.height,
        spec.raster.descriptor().unwrap(),
    );
    let mut encoder = RowsCodec
        .encoder(&spec.with_supply(RowSupply::Whole))
        .unwrap();
    let timing = encoder.plan().output;
    let mut sizes = Vec::new();
    let mut event = encoder.step(EncodeInput::None, &NoPulse).unwrap().event;
    for source in sources {
        assert_eq!(event, EncodeEvent::NeedFrame);
        event = encoder
            .step(EncodeInput::Frame(FrameMeta::default()), &NoPulse)
            .unwrap()
            .event;
        let frame = slice(source, w, h, w as usize * d.bytes_per_pixel(), d);
        event = feed_frame(&mut *encoder, &frame, event, &NoPulse).unwrap();
        sizes.push(write_output(encoder.output(), destination).unwrap());
    }
    assert_eq!(
        encoder.step(EncodeInput::End, &NoPulse).unwrap().event,
        EncodeEvent::Done
    );
    sizes.push(write_output(encoder.output(), destination).unwrap());
    destination.flush().unwrap();
    (timing, sizes)
}

#[test]
fn an_unknown_frame_count_is_patched_or_held_never_wrong() {
    let (w, h, d) = (5, 3, PixelDescriptor::RGB8_SRGB);
    let sources = frames(4, w, h, d);
    let spec = EncodeSpec::new(Raster::packed(w, h, d)).with_sequence(SequenceSpec::new());
    let reference = encode_sequence(
        &RowsCodec,
        spec.clone(),
        sources
            .iter()
            .map(|s| (slice(s, w, h, w as usize * 3, d), FrameMeta::default())),
        &NoPulse,
    )
    .unwrap();

    // A destination that can seek back gets bytes as they are made, then a patch.
    let mut seekable = SeekBack::new(Cursor::new(Vec::new())).unwrap();
    let (timing, sizes) = stream_sequence(
        spec.clone().with_output(OutputAccess::Patchable),
        &sources,
        &mut seekable,
    );
    assert_eq!(timing, OutputTiming::Streaming { lag_rows: 0 });
    assert!(
        sizes[..4].iter().all(|n| *n > 0),
        "bytes left after every frame: {sizes:?}"
    );
    assert_eq!(seekable.into_inner().into_inner(), reference);

    // A destination that cannot gets nothing until the count is known.
    let mut forward = Forward(Vec::new());
    let (timing, sizes) = stream_sequence(spec, &sources, &mut forward);
    assert_eq!(timing, OutputTiming::AtFinish);
    assert_eq!(&sizes[..4], &[0, 0, 0, 0]);
    assert_eq!(forward.0, reference);
}

fn planar_source(w: u32, h: u32) -> [PlaneBuf<u8>; 3] {
    let (cw, ch) = (w.div_ceil(2) as usize, h.div_ceil(2) as usize);
    let plane = |w: usize, h: usize, seed: usize| {
        let samples = (0..w * h)
            .map(|i| ((i * 5 + seed * 37) % 251) as u8)
            .collect();
        PlaneBuf::new(samples, w, h, w).unwrap()
    };
    [
        plane(w as usize, h as usize, 1),
        plane(cw, ch, 2),
        plane(cw, ch, 3),
    ]
}

fn encode_planar(source: &[PlaneBuf<u8>; 3], w: u32, h: u32, strip: u32) -> Vec<u8> {
    let (layout, color) = (yuv420(), bt601_full());
    let mut encoder = RowsCodec
        .encoder(&EncodeSpec::new(planar_raster(w, h)))
        .unwrap();
    assert_eq!(encoder.plan().strip_rows, 2);
    let mut event = encoder.step(EncodeInput::None, &NoPulse).unwrap().event;
    while let EncodeEvent::NeedRows { y, .. } = event {
        let rows = strip.min(h - y);
        let (cy, crows) = ((y / 2) as usize, rows.div_ceil(2) as usize);
        let cw = w.div_ceil(2) as usize;
        let planes = [
            AnyPlane::U8(
                source[0]
                    .sub(0, y as usize, w as usize, rows as usize)
                    .unwrap(),
            ),
            AnyPlane::U8(source[1].sub(0, cy, cw, crows).unwrap()),
            AnyPlane::U8(source[2].sub(0, cy, cw, crows).unwrap()),
        ];
        let strip = PlanarSlice::new(&planes, &layout, w, rows, &color, None).unwrap();
        let step = encoder.step(EncodeInput::Planes(strip), &NoPulse).unwrap();
        assert_eq!(step.rows, rows);
        event = step.event;
    }
    assert_eq!(event, EncodeEvent::Done);
    encoder.output().take()
}

#[test]
fn planar_strips_pass_through_without_becoming_rgb() {
    for (w, h) in [(5, 5), (8, 6), (1, 1)] {
        let source = planar_source(w, h);
        let bytes = encode_planar(&source, w, h, 2);
        assert_eq!(
            bytes,
            encode_planar(&source, w, h, 4),
            "strip height is not in the file"
        );

        let decoded = decode(
            &RowsCodec,
            &bytes,
            &DecodeSpec::new().with_planar(true),
            &NoPulse,
        )
        .unwrap();
        assert!(decoded.info.native.is_planar());
        let planar = decoded.frames[0].pixels.planar().expect("planar frame");
        for (got, want) in planar.planes.iter().zip(&source) {
            let AnyPlane::U8(got) = got.view() else {
                panic!("8-bit planes")
            };
            assert_eq!(got.strided_samples(), want.strided_samples());
        }

        // A sink that did not ask for planes is refused, not handed RGB it did
        // not expect.
        let refused = decode(&RowsCodec, &bytes, &DecodeSpec::new(), &NoPulse).unwrap_err();
        assert_eq!(
            refused.error().kind(),
            ErrorKind::Unsupported(Unsupported::Planar)
        );

        // Decoder planes go straight to the encoder.
        let out = transcode(
            RowsCodec
                .decoder(&DecodeSpec::new().with_planar(true))
                .unwrap(),
            &bytes,
            Arc::new(RowsCodec),
            |frame| EncodeSpec::new(frame.raster.clone()),
            &NoPulse,
        )
        .unwrap();
        assert_eq!(out, bytes);
    }
}

/// Step a decoder over a whole input until `Info`.
fn step_to_info(decoder: &mut dyn Decoder, bytes: &[u8], sink: &mut dyn Sink) -> Result<()> {
    let mut input = Input::whole(bytes);
    loop {
        let step = decoder.step(input, sink, &NoPulse)?;
        match step.event {
            DecodeEvent::Info => return Ok(()),
            DecodeEvent::NeedInput => match decoder.requested()[0] {
                InputRequest::Range(r) => {
                    input = Input::new(r.offset, &bytes[r.offset as usize..r.end() as usize])
                        .with_total_len(Some(bytes.len() as u64));
                }
                InputRequest::AtLeast { offset, .. } => {
                    input = Input::new(offset, &bytes[offset as usize..])
                        .with_total_len(Some(bytes.len() as u64));
                }
                _ => input = Input::whole(bytes),
            },
            other => panic!("{other:?} before Info"),
        }
    }
}

#[test]
fn info_arrives_before_anything_reaches_the_sink() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, _) = encoded(&**enc, 16, 16, PixelDescriptor::RGB8_SRGB, 6);
        let mut decoder = dec.decoder(&DecodeSpec::new()).unwrap();
        assert!(decoder.info().is_none() && decoder.plan().is_none());
        let mut sink = Recording::default();
        step_to_info(&mut *decoder, &bytes, &mut sink).unwrap();
        assert!(sink.log.is_empty(), "{}: {:?}", enc.name(), sink.log);
        let (info, plan) = (decoder.info().unwrap(), decoder.plan().unwrap());
        assert_eq!((info.width, info.height), (16, 16));
        assert_eq!((plan.raster.width, plan.raster.height), (16, 16));
        // Declining is dropping. Nothing was begun, so nothing is left half done.
        drop(decoder);
        assert!(sink.log.is_empty());
    }
}

#[test]
fn gates_refuse_before_work_and_say_why() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, _) = encoded(&**enc, 20, 20, PixelDescriptor::RGB8_SRGB, 7);
        let spec = DecodeSpec::new().with_limits(ResourceLimits::none().with_max_pixels(399));
        let mut sink = Recording::default();
        let err = decode_slice(dec.decoder(&spec).unwrap(), &bytes, &mut sink, &NoPulse)
            .err()
            .unwrap();
        assert_eq!(
            err.error().kind(),
            ErrorKind::Limit(LimitKind::Pixels),
            "{}",
            enc.name()
        );
        let numbers = err
            .error()
            .native_as::<LimitExceeded>()
            .expect("the numbers");
        assert_eq!((numbers.limit, numbers.actual), (399, Some(400)));
        assert!(sink.log.is_empty());

        // The same gate, on the encode side, refuses before any row.
        let spec = EncodeSpec::new(Raster::packed(20, 20, PixelDescriptor::RGB8_SRGB))
            .with_limits(ResourceLimits::none().with_max_dimensions(19, 100));
        let err = enc.encoder(&spec).err().expect("refused");
        assert_eq!(err.error().kind(), ErrorKind::Limit(LimitKind::Width));
    }
}

#[test]
fn an_exact_output_size_is_gated_before_any_row() {
    let raster = Raster::packed(10, 10, PixelDescriptor::RGB8_SRGB);
    let exact = 22 + 4 + 300;
    let tight_gate = |bytes| {
        EncodeSpec::new(raster.clone())
            .with_limits(ResourceLimits::none().with_max_output_bytes(bytes))
    };
    let encoder = RowsCodec.encoder(&tight_gate(exact)).unwrap();
    assert_eq!(encoder.plan().estimate.output_bytes, Bound::AtMost(exact));
    let err = RowsCodec.encoder(&tight_gate(exact - 1)).err().unwrap();
    assert_eq!(err.error().kind(), ErrorKind::Limit(LimitKind::OutputBytes));
}

#[test]
fn a_memory_gate_is_a_promise() {
    let raster = Raster::packed(32, 32, PixelDescriptor::GRAY8_SRGB);
    let gated = |supply| {
        EncodeSpec::new(raster.clone())
            .with_supply(supply)
            .with_limits(ResourceLimits::none().with_max_memory_bytes(1000))
    };
    // Rows offered once must be held: 1024 bytes, over the gate.
    let err = BiasCodec.encoder(&gated(RowSupply::Once)).err().unwrap();
    assert_eq!(err.error().kind(), ErrorKind::Limit(LimitKind::Memory));
    // Offered twice, nothing is held.
    for supply in [RowSupply::Replay(2), RowSupply::Whole] {
        let encoder = BiasCodec.encoder(&gated(supply)).unwrap();
        assert_eq!(encoder.plan().estimate.memory_bytes, Bound::AtMost(0));
    }
    // The simple adapter cannot bound its codec, so under a gate it refuses
    // instead of guessing.
    let err = RleCodec::codec()
        .encoder(&gated(RowSupply::Whole))
        .err()
        .unwrap();
    assert_eq!(err.error().kind(), ErrorKind::Limit(LimitKind::Memory));
    assert_eq!(
        err.error().native_as::<LimitExceeded>().unwrap().actual,
        None
    );

    // Decoding: the RLE decoder states its bound, so the gate can admit it.
    let (bytes, _) = encoded(&RleCodec::codec(), 32, 32, PixelDescriptor::GRAY8_SRGB, 8);
    let within = DecodeSpec::new().with_limits(ResourceLimits::none().with_max_memory_bytes(1024));
    assert!(decode(&RleCodec::codec(), &bytes, &within, &NoPulse).is_ok());
    let over = DecodeSpec::new().with_limits(ResourceLimits::none().with_max_memory_bytes(1023));
    let err = decode(&RleCodec::codec(), &bytes, &over, &NoPulse).unwrap_err();
    assert_eq!(err.error().kind(), ErrorKind::Limit(LimitKind::Memory));
}

#[test]
fn requirements_are_refused_and_preferences_are_reported() {
    let lossy = Fidelity::Lossy(LossyTarget::ApproxSsim2(80.0));
    for enc in encoders() {
        let raster = Raster::packed(4, 4, PixelDescriptor::RGB8_SRGB);
        let require = EncodeSpec::new(raster.clone()).with_fidelity(Demand::Require(lossy));
        let err = enc.encoder(&require).err().expect("refused");
        assert_eq!(
            err.error().kind(),
            ErrorKind::Unsupported(Unsupported::Fidelity)
        );
        assert_eq!(err.error().codec(), Some(enc.name()));

        let prefer = EncodeSpec::new(raster.clone()).with_fidelity(Demand::Prefer(lossy));
        assert_eq!(
            enc.encoder(&prefer).unwrap().plan().fidelity,
            Fidelity::Lossless
        );

        let icc = Metadata::none().with_icc(vec![1, 2, 3]);
        let required =
            EncodeSpec::new(raster.clone()).with_metadata(icc.clone(), MetadataKinds::ICC);
        let err = enc.encoder(&required).err().expect("refused");
        assert_eq!(
            err.error().kind(),
            ErrorKind::Unsupported(Unsupported::Metadata)
        );
        // Not required: admitted, and the plan says it will not be written.
        let offered = EncodeSpec::new(raster).with_metadata(icc, MetadataKinds::NONE);
        assert_eq!(
            enc.encoder(&offered).unwrap().plan().embedded,
            MetadataKinds::NONE
        );
    }

    // A decoder that cannot produce a required format refuses when it learns
    // what the file holds, before any pixel.
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, _) = encoded(&**enc, 4, 4, PixelDescriptor::RGB8_SRGB, 9);
        let spec = DecodeSpec::new().with_accept(Accept::require(&[PixelDescriptor::RGBA8_SRGB]));
        let mut sink = Recording::default();
        let err = decode_slice(dec.decoder(&spec).unwrap(), &bytes, &mut sink, &NoPulse)
            .err()
            .unwrap();
        assert_eq!(
            err.error().kind(),
            ErrorKind::Unsupported(Unsupported::PixelFormat)
        );
        assert!(sink.log.is_empty());
        // A preference falls back to the native format.
        let spec = DecodeSpec::new().with_accept(Accept::prefer(&[PixelDescriptor::RGBA8_SRGB]));
        let decoded = decode(&*dec, &bytes, &spec, &NoPulse).unwrap();
        assert_eq!(
            decoded.frames[0].header.raster.descriptor(),
            Some(PixelDescriptor::RGB8_SRGB)
        );
    }
}

#[test]
fn the_codecs_own_error_survives_erasure() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (mut bytes, _) = encoded(&**enc, 8, 8, PixelDescriptor::RGB8_SRGB, 11);
        let truncated = &bytes[..bytes.len() - 5];
        let err = decode(&*dec, truncated, &DecodeSpec::new(), &NoPulse).unwrap_err();
        assert_eq!(err.error().kind(), ErrorKind::Truncated, "{}", enc.name());

        bytes[12] = 9; // a layout byte no fixture defines
        let err = decode(&*dec, &bytes, &DecodeSpec::new(), &NoPulse).unwrap_err();
        assert_eq!(err.error().kind(), ErrorKind::Malformed);
        assert_eq!(
            err.error().native_as::<FixtureError>(),
            Some(&FixtureError::BadLayout(9))
        );
        // A caller holding only `dyn Error` reaches it too.
        let erased: Box<dyn std::error::Error + Send + Sync> = Box::new(err);
        assert_eq!(
            find_cause::<FixtureError>(&*erased),
            Some(&FixtureError::BadLayout(9))
        );

        // The same cause comes back through a transcode.
        let err = transcode(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &bytes,
            Arc::new(RowsCodec),
            |frame| EncodeSpec::new(frame.raster.clone()),
            &NoPulse,
        )
        .unwrap_err();
        assert_eq!(
            err.error().native_as::<FixtureError>(),
            Some(&FixtureError::BadLayout(9))
        );
    }
}

#[derive(Debug, PartialEq)]
struct DiskFull(u32);
impl std::fmt::Display for DiskFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "disk full at row {}", self.0)
    }
}
impl std::error::Error for DiskFull {}

/// Fails the first sink call that carries rows.
struct Failing(Collect);
impl Failing {
    fn fail<T>(y: u32) -> Result<T> {
        Err(zencodec::CodecError::new(ErrorKind::Sink)
            .with_native(DiskFull(y))
            .at())
    }
}
impl Sink for Failing {
    fn begin(&mut self, frame: &zencodec::FrameHeader) -> Result<()> {
        self.0.begin(frame)
    }
    fn lend(&mut self, y: u32, _: u32, _: Pass) -> Result<PixelSliceMut<'_>> {
        Self::fail(y)
    }
    fn frame(&mut self, _: zenpixels::PixelBuffer) -> Result<()> {
        Self::fail(0)
    }
}

#[test]
fn a_sink_failure_ends_the_decode_with_the_sinks_own_error() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, _) = encoded(&**enc, 8, 8, PixelDescriptor::RGB8_SRGB, 12);
        let mut sink = Failing(Collect::new());
        let err = decode_slice(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &bytes,
            &mut sink,
            &NoPulse,
        )
        .err()
        .unwrap();
        assert_eq!(err.error().kind(), ErrorKind::Sink, "{}", enc.name());
        assert_eq!(err.error().native_as::<DiskFull>(), Some(&DiskFull(0)));
    }
}

#[test]
fn a_stop_before_the_first_row_reaches_no_sink() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, _) = encoded(&**enc, 8, 8, PixelDescriptor::RGB8_SRGB, 13);
        let switch = Switch::default();
        switch.stop();
        let mut sink = Recording::default();
        // A pulse that borrows the switch and cannot be shared is enough
        // for work that stays on this thread.
        let pulse = switch.borrowed();
        let decoder = dec.decoder(&DecodeSpec::new()).unwrap();
        let err = decode_slice(decoder, &bytes, &mut sink, &pulse)
            .err()
            .unwrap();
        assert_eq!(
            err.error().kind(),
            ErrorKind::Stopped(StopReason::Cancelled),
            "{}",
            enc.name()
        );
        assert_eq!(err.error().stop_reason(), Some(StopReason::Cancelled));
        assert!(sink.log.is_empty(), "{}: {:?}", enc.name(), sink.log);

        let source = pattern(8, 8, PixelDescriptor::RGB8_SRGB, 13);
        let spec = EncodeSpec::new(Raster::packed(8, 8, PixelDescriptor::RGB8_SRGB));
        let err = encode(
            &**enc,
            spec,
            slice(&source, 8, 8, 24, PixelDescriptor::RGB8_SRGB),
            &pulse,
        )
        .unwrap_err();
        assert_eq!(
            err.error().kind(),
            ErrorKind::Stopped(StopReason::Cancelled)
        );
    }
}

#[test]
fn a_stop_mid_decode_is_seen_within_a_strip() {
    let (bytes, _) = encoded(&BiasCodec, 4, 64, PixelDescriptor::GRAY8_SRGB, 14);
    // The driver looks at the pulse around every step and the decoder looks
    // within each one, so the twelfth look falls a few one-row strips in.
    let switch = Switch::tripping_at(12);
    let mut sink = Recording {
        max_loan: 1,
        ..Recording::default()
    };
    let decoder = BiasCodec.decoder(&DecodeSpec::new()).unwrap();
    let err = decode_slice(decoder, &bytes, &mut sink, &switch.borrowed())
        .err()
        .unwrap();
    assert_eq!(
        err.error().kind(),
        ErrorKind::Stopped(StopReason::Cancelled)
    );
    let lends = sink.log.iter().filter(|c| **c == "lend").count();
    assert!((1..=6).contains(&lends), "stopped after {lends} of 64 strips");
}

fn with_exif(exif: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (w, h, d) = (6, 50, PixelDescriptor::RGB8_SRGB);
    let source = pattern(w, h, d, 15);
    let spec = EncodeSpec::new(Raster::packed(w, h, d)).with_metadata(
        Metadata::none().with_exif(exif.to_vec()),
        MetadataKinds::EXIF,
    );
    let mut encoder = BiasCodec
        .encoder(&spec.clone().with_supply(RowSupply::Whole))
        .unwrap();
    assert_eq!(encoder.plan().embedded, MetadataKinds::EXIF);
    drop(encoder.step(EncodeInput::None, &NoPulse));
    (
        encode(&BiasCodec, spec, slice(&source, w, h, 18, d), &NoPulse).unwrap(),
        source,
    )
}

#[test]
fn metadata_after_the_pixels_is_late_unless_asked_for_first() {
    let exif = b"II*\0 not really exif, but verbatim";
    let (bytes, source) = with_exif(exif);

    // By default the decoder reads front to back: at Info the EXIF is not
    // settled, and at Done it is there.
    let mut decoder = BiasCodec.decoder(&DecodeSpec::new()).unwrap();
    let mut sink = Recording::default();
    step_to_info(&mut *decoder, &bytes, &mut sink).unwrap();
    let info = decoder.info().unwrap();
    assert!(!info.settled.contains(MetadataKinds::EXIF) && info.metadata.exif.is_none());
    assert_eq!(decoder.plan().unwrap().access, Access::Sequential);
    let decoded = decode(&BiasCodec, &bytes, &DecodeSpec::new(), &NoPulse).unwrap();
    assert_eq!(decoded.info.metadata.exif.as_deref(), Some(&exif[..]));
    assert_eq!(decoded.info.settled, MetadataKinds::ALL);

    // Asked for first, it is in hand at Info, before any pixel, and the plan
    // admits to reading out of order.
    let first = DecodeSpec::new().with_metadata_first(MetadataKinds::EXIF);
    let mut decoder = BiasCodec.decoder(&first).unwrap();
    let mut sink = Recording::default();
    step_to_info(&mut *decoder, &bytes, &mut sink).unwrap();
    assert!(sink.log.is_empty());
    assert_eq!(
        decoder.info().unwrap().metadata.exif.as_deref(),
        Some(&exif[..])
    );
    assert_eq!(decoder.plan().unwrap().access, Access::Random);

    // The in-memory driver answers the same request with a sub-slice.
    let decoded = decode(&BiasCodec, &bytes, &first, &NoPulse).unwrap();
    assert_eq!(decoded.info.metadata.exif.as_deref(), Some(&exif[..]));
    assert_eq!(packed(&decoded.frames[0]), source);

    // Over a source that reads a byte at a time, that costs one range fetch.
    let mut drip = Drip::new(&bytes, 1);
    let mut collect = Collect::new();
    let decoder = decode_read_at(
        BiasCodec.decoder(&first).unwrap(),
        &mut drip,
        &mut collect,
        &DriveOptions::new().with_read_size(1),
        &NoPulse,
    )
    .unwrap();
    assert_eq!(drip.range_calls, 1);
    assert_eq!(
        decoder.info().unwrap().metadata.exif.as_deref(),
        Some(&exif[..])
    );
    assert_eq!(packed(&collect.frames()[0]), source);
}

#[test]
fn one_encoder_three_ways_to_see_the_rows_twice() {
    let (w, h, d) = (9, 12, PixelDescriptor::RGB8_SRGB);
    let source = pattern(w, h, d, 16);
    let frame = slice(&source, w, h, 27, d);
    let raster = Raster::packed(w, h, d);
    let mut files = Vec::new();
    for (supply, demand, held) in [
        (RowSupply::Whole, RowDemand::Whole, 0),
        (RowSupply::Replay(2), RowDemand::Replay(2), 0),
        (RowSupply::Once, RowDemand::Strips, 9 * 12 * 3),
    ] {
        let mut encoder = BiasCodec
            .encoder(&EncodeSpec::new(raster.clone()).with_supply(supply))
            .unwrap();
        assert_eq!(encoder.plan().input, demand);
        assert_eq!(encoder.plan().estimate.memory_bytes, Bound::AtMost(held));
        // The same answer-the-request loop for all three, five rows at a time
        // unless the encoder asked for the frame whole.
        let mut event = encoder.step(EncodeInput::None, &NoPulse).unwrap().event;
        let mut passes = 0;
        while let EncodeEvent::NeedRows { pass, y } = event {
            passes = passes.max(pass + 1);
            let rows = if demand == RowDemand::Whole {
                h
            } else {
                5.min(h - y)
            };
            let step = encoder
                .step(EncodeInput::Rows(frame.sub_rows(y, rows)), &NoPulse)
                .unwrap();
            assert_eq!(step.rows, rows);
            event = step.event;
        }
        assert_eq!(event, EncodeEvent::Done);
        assert_eq!(passes, if demand == RowDemand::Replay(2) { 2 } else { 1 });
        files.push(encoder.output().take());
    }
    assert_eq!(files[0], files[1]);
    assert_eq!(files[0], files[2]);
}

/// Asks forever for input it already holds.
struct Stuck([InputRequest; 1]);
impl Decoder for Stuck {
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
        Ok(DecodeStep::new(0, DecodeEvent::NeedInput))
    }
}

#[test]
fn drivers_end_a_decoder_that_makes_no_progress() {
    let request = [InputRequest::AtLeast {
        offset: 0,
        min_len: 4,
    }];
    let bytes = [0u8; 16];
    let err = decode_slice(
        Box::new(Stuck(request)),
        &bytes,
        &mut Collect::new(),
        &NoPulse,
    )
    .err()
    .unwrap();
    assert_eq!(err.error().kind(), ErrorKind::Protocol);
    let err = decode_read_at(
        Box::new(Stuck(request)),
        &mut &bytes[..],
        &mut Collect::new(),
        &DriveOptions::new(),
        &NoPulse,
    )
    .err()
    .unwrap();
    assert_eq!(err.error().kind(), ErrorKind::Protocol);
}

#[test]
fn the_transcoding_sink_stages_at_most_one_strip() {
    // The bias decoder writes into lent rows; the sink lends from one strip.
    let (bytes, source) = encoded(&BiasCodec, 5, 100, PixelDescriptor::RGB8_SRGB, 17);
    let mut sink = EncoderSink::new(
        Arc::new(RowsCodec),
        |frame| EncodeSpec::new(frame.raster.clone()),
        &NoPulse,
    );
    decode_slice(
        BiasCodec.decoder(&DecodeSpec::new()).unwrap(),
        &bytes,
        &mut sink,
        &NoPulse,
    )
    .unwrap();
    sink.finish().unwrap();
    assert_eq!(sink.plan().unwrap().estimate.memory_bytes, Bound::AtMost(0));
    let out = sink.take_output();
    let decoded = decode(&RowsCodec, &out, &DecodeSpec::new(), &NoPulse).unwrap();
    assert_eq!(packed(&decoded.frames[0]), source);
}

/// Bytes in memory that must never be read: only borrowed.
struct Resident<'a>(&'a [u8]);
impl zencodec_io::ReadAt for Resident<'_> {
    fn read_at(&mut self, _: u64, _: &mut [u8]) -> std::io::Result<usize> {
        panic!("a resident source was read instead of borrowed");
    }
    fn as_slice(&self) -> Option<&[u8]> {
        Some(self.0)
    }
}

#[test]
fn a_source_already_in_memory_is_borrowed_not_read() {
    for (enc, dec) in encoders().iter().zip(decoders()) {
        let (bytes, source) = encoded(&**enc, 30, 30, PixelDescriptor::RGB8_SRGB, 18);
        let mut collect = Collect::new();
        decode_read_at(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &mut Resident(&bytes),
            &mut collect,
            &DriveOptions::new(),
            &NoPulse,
        )
        .unwrap();
        assert_eq!(packed(&collect.frames()[0]), source, "{}", enc.name());
        // A plain slice is resident too.
        let mut collect = Collect::new();
        decode_read_at(
            dec.decoder(&DecodeSpec::new()).unwrap(),
            &mut &bytes[..],
            &mut collect,
            &DriveOptions::new().with_max_input_bytes(1),
            &NoPulse,
        )
        .expect("nothing is held, so the input gate has nothing to refuse");
        assert_eq!(packed(&collect.frames()[0]), source);
    }
}

#[test]
fn a_streaming_window_stays_as_small_as_the_decoder_lets_it() {
    // Rows are consumed as they arrive, so a gate far below the file size
    // is enough: the window is reused, not grown.
    let (bytes, source) = encoded(&RowsCodec, 16, 400, PixelDescriptor::RGB8_SRGB, 19);
    assert!(bytes.len() > 19_000);
    let mut collect = Collect::new();
    let opts = DriveOptions::new()
        .with_read_size(256)
        .with_max_input_bytes(512);
    decode_read_at(
        RowsCodec.decoder(&DecodeSpec::new()).unwrap(),
        &mut Drip::new(&bytes, 256),
        &mut collect,
        &opts,
        &NoPulse,
    )
    .expect("a 512-byte window decodes a 19 KB file");
    assert_eq!(packed(&collect.frames()[0]), source);

    // A decoder that needs the whole file cannot fit, and is told so.
    let (bytes, _) = encoded(&RleCodec::codec(), 16, 400, PixelDescriptor::RGB8_SRGB, 19);
    let err = decode_read_at(
        RleCodec::codec().decoder(&DecodeSpec::new()).unwrap(),
        &mut Drip::new(&bytes, 256),
        &mut Collect::new(),
        &opts,
        &NoPulse,
    )
    .err()
    .unwrap();
    assert_eq!(err.error().kind(), ErrorKind::Limit(LimitKind::InputBytes));
}
