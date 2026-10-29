//! Heaptrack target for matched native decode paths; no internal timers.
#![forbid(unsafe_code)]
use std::{error::Error, io};
use zencodec::{
    CodecSet,
    decode::*,
    encode::EncoderConfig,
    operation::{DecodeRequest, OperationError, PolicySet},
    storage::StorageBudget,
};
use zencodec_io::{CollectSink, PolicyDecoder, drive_from_slice};
use zenjpeg::{JpegDecoderConfig, JpegEncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};
const WIDTH: u32 = 1024;
const HEIGHT: u32 = 768;
type Failure = Box<dyn Error + Send + Sync>;
struct NoPixels;
impl DecodeRowSink for NoPixels {
    fn begin(&mut self, _: u32, _: u32, _: PixelDescriptor) -> Result<(), SinkError> {
        panic!("required descriptor refusal must precede sink begin")
    }
    fn provide_next_buffer(
        &mut self,
        _: u32,
        _: u32,
        _: u32,
        _: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        panic!("no rows admitted")
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        panic!("no output admitted")
    }
}
fn mismatch(error: &(dyn Error + 'static)) {
    assert!(matches!(
        zencodec::find_cause::<OperationError>(error),
        Some(OperationError::OutputDescriptor {
            requested: PixelDescriptor::RGB16_SRGB,
            actual: PixelDescriptor::RGB8_SRGB
        })
    ));
}
fn checksum(pixels: PixelSlice<'_>) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for y in 0..pixels.rows() {
        for b in pixels.row(y) {
            hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
    }
    hash
}
fn main() -> Result<(), Failure> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: decode_admission MODE baseline|progressive FILE".into());
    }
    let progressive = match args[2].as_str() {
        "baseline" => false,
        "progressive" => true,
        _ => return Err("unknown configuration".into()),
    };
    if args[1] == "fixture" {
        let data: Vec<u8> = (0..WIDTH as usize * HEIGHT as usize * 3)
            .map(|i| ((i / 3 + i / (WIDTH as usize * 3) * 7) % 251) as u8)
            .collect();
        let pixels = PixelSlice::new(
            &data,
            WIDTH,
            HEIGHT,
            WIDTH as usize * 3,
            PixelDescriptor::RGB8_SRGB,
        )?;
        let encoded = JpegEncoderConfig::new()
            .with_progressive(progressive)
            .with_calibrated_quality(87.0)
            .encode(pixels)?;
        std::fs::write(&args[3], encoded.data())?;
        return Ok(());
    }
    let input = std::fs::read(&args[3])?;
    let codecs = CodecSet::new().with_decoder(JpegDecoderConfig::new());
    let refused = args[1].ends_with("refuse");
    // Equal-length mode names keep argument allocation identical in paired profiles.
    let request = if args[1] == "one-shot-plain" {
        DecodeRequest::new()
    } else {
        DecodeRequest::new().with_output(if refused {
            PixelDescriptor::RGB16_SRGB
        } else {
            PixelDescriptor::RGB8_SRGB
        })
    };
    if args[1].starts_with("one-shot-") {
        let result = request.decode(&codecs, &input);
        if refused {
            mismatch(&result.unwrap_err());
            println!("refused-before-return");
        } else {
            let result = result?;
            assert_eq!((result.width(), result.height()), (WIDTH, HEIGHT));
            assert_eq!(result.pixels().descriptor(), PixelDescriptor::RGB8_SRGB);
            println!("pixels {:016x}", checksum(result.pixels()));
        }
        return Ok(());
    }
    let native = request.start_controlled(&codecs, StorageBudget::new(16 << 20, 4))?;
    let mut decoder = PolicyDecoder::new(native, PolicySet::new());
    if args[1] == "controlled-refuse" {
        let error = drive_from_slice(&mut decoder, &input, &mut NoPixels, None, 4096).unwrap_err();
        mismatch(&error);
        assert!(decoder.take_details().is_none());
        println!("refused-before-sink");
        return Ok(());
    }
    let mut sink = CollectSink::new();
    drive_from_slice(&mut decoder, &input, &mut sink, None, 4096)?;
    let details = decoder
        .take_details()
        .ok_or_else(|| io::Error::other("missing final details"))?;
    let pixels = match args[1].as_str() {
        "controlled-own" => sink.into_buffer().unwrap(),
        "controlled-copy" => {
            let pixels = sink.to_buffer().unwrap();
            std::hint::black_box(&sink);
            pixels
        }
        _ => return Err("unknown mode".into()),
    };
    let result = DecodeOutput::from_parts(pixels, details);
    assert_eq!((result.width(), result.height()), (WIDTH, HEIGHT));
    println!("pixels {:016x}", checksum(result.pixels()));
    Ok(())
}
