//! `zencodec::helpers::WholeFileIncremental` over the reference codec:
//! early header, pixels identical to one-shot through every driver,
//! truncation as a codec error, cancellation.

use std::borrow::Cow;
use std::io::Cursor;

use zencodec::decode::{
    Decode, DecodeJob, DecoderConfig, Event, IncrementalDecode, Input, MetadataUpdate, StepError,
    erase_incremental,
};
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec::helpers::WholeFileIncremental;
use zencodec_io::{CollectSink, DriveError, drive_from_slice, pump_read_to_sink};
use zencodec_testkit::TestImage;
use zencodec_testkit::reference::{ReferenceDecoderConfig, ReferenceEncoderConfig};

fn file() -> Vec<u8> {
    let img = TestImage::rgba8_gradient(37, 21);
    ReferenceEncoderConfig::new()
        .job()
        .encoder()
        .unwrap()
        .encode(img.as_slice())
        .unwrap()
        .into_vec()
}

fn one_shot(data: &[u8]) -> Vec<u8> {
    let out = ReferenceDecoderConfig
        .job()
        .decoder(Cow::Borrowed(data), &[])
        .unwrap()
        .decode()
        .unwrap();
    let s = out.pixels();
    let row = s.width() as usize * s.descriptor().bytes_per_pixel();
    (0..s.rows())
        .flat_map(|y| s.row(y)[..row].to_vec())
        .collect()
}

fn decoder() -> Box<zencodec::decode::DynIncrementalDecoder> {
    erase_incremental(WholeFileIncremental::new(ReferenceDecoderConfig.job(), &[]))
}

#[test]
fn pixels_match_one_shot_at_every_window() {
    let data = file();
    let want = one_shot(&data);
    for window in [1usize, 7, 64, data.len()] {
        let mut dec = decoder();
        let mut sink = CollectSink::new();
        let (info, _) = drive_from_slice(&mut *dec, &data, &mut sink, None, window).unwrap();
        assert_eq!((info.width, info.height), (37, 21));
        assert_eq!(sink.bytes(), &want[..], "window {window}");
        assert!(sink.finished());
    }
}

#[test]
fn header_is_reported_before_the_body_arrives() {
    let data = file();
    let mut dec = decoder();
    let mut sink = CollectSink::new();
    let unstoppable = &zencodec::NoPulse;
    // Offer a prefix that holds the header but not the pixels.
    let prefix = &data[..data.len() / 2];
    let mut saw_header = false;
    for _ in 0..8 {
        let p = dec
            .step(Input::new(0, prefix), &mut sink, Some(&unstoppable))
            .unwrap();
        if p.event == Event::Metadata(MetadataUpdate::HeaderReady) {
            saw_header = true;
            break;
        }
        assert_eq!(p.event, Event::Need);
    }
    assert!(saw_header, "probe should succeed on half the file");
    assert_eq!(dec.info().map(|i| (i.width, i.height)), Some((37, 21)));
    assert_eq!(sink.rows_written(), 0, "no rows before the whole file");
}

#[test]
fn a_stream_is_read_to_its_end() {
    let data = file();
    let mut dec = decoder();
    let mut sink = CollectSink::new();
    pump_read_to_sink(Cursor::new(&data), &mut *dec, &mut sink, None, 512, 1 << 20).unwrap();
    assert_eq!(sink.bytes(), &one_shot(&data)[..]);
}

#[test]
fn truncation_is_a_codec_error() {
    let data = file();
    for cut in [0usize, 3, 40, data.len() / 2, data.len() - 1] {
        let mut dec = decoder();
        let mut sink = CollectSink::new();
        let err = drive_from_slice(&mut *dec, &data[..cut], &mut sink, None, 64).unwrap_err();
        assert!(
            matches!(err, DriveError::Step(StepError::Codec(_))),
            "cut {cut}: {err:?}"
        );
    }
}

#[test]
fn cancellation_is_terminal() {
    let data = file();
    let stopper = Flag::default();
    let mut dec = decoder();
    let mut sink = CollectSink::new();
    let _ = dec
        .step(
            Input::empty(),
            &mut sink,
            Some(&zencodec::how_far::StopOnly::borrowed(&stopper)),
        )
        .unwrap();
    stopper.0.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        dec.step(
            Input::new(0, &data).with_total_len(data.len() as u64),
            &mut sink,
            Some(&zencodec::how_far::StopOnly::borrowed(&stopper))
        ),
        Err(StepError::Cancelled(_))
    ));
    assert!(matches!(
        dec.step(Input::empty(), &mut sink, Some(&zencodec::NoPulse)),
        Err(StepError::Finished)
    ));
}

#[derive(Default)]
struct Flag(std::sync::atomic::AtomicBool);

impl enough::Stop for Flag {
    fn check(&self) -> Result<(), enough::StopReason> {
        if self.0.load(std::sync::atomic::Ordering::SeqCst) {
            Err(enough::StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[test]
fn the_testkit_contract_holds() {
    let data = file();
    zencodec_testkit::check_incremental_decoder(
        || Ok(decoder()),
        |bytes| {
            ReferenceDecoderConfig
                .job()
                .decoder(Cow::Borrowed(bytes), &[])
                .and_then(|d| d.decode())
                .map_err(|e| e.to_string())
        },
        &data,
    )
    .unwrap();
}
