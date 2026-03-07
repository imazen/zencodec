//! Integration test exercising the full zencodec-types API via a PNM codec.
//!
//! Tests both concrete (generic) and dyn-dispatch (object-safe) paths.

mod pnm;

use pnm::{PnmDecoderConfig, PnmEncoderConfig};

use zc::decode::{Decode, DecodeJob, DecoderConfig, DynDecoderConfig};
use zc::encode::{EncodeJob, Encoder, EncoderConfig, DynEncoderConfig};
use zc::{ImageFormat, ResourceLimits};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

// =========================================================================
// Test data helpers
// =========================================================================

/// Create a 4x2 RGB8 test image with known pixel values.
fn test_rgb8_pixels() -> PixelBuffer {
    #[rustfmt::skip]
    let data: Vec<u8> = vec![
        // Row 0: red, green, blue, white
        255,   0,   0,
          0, 255,   0,
          0,   0, 255,
        255, 255, 255,
        // Row 1: black, yellow, cyan, magenta
          0,   0,   0,
        255, 255,   0,
          0, 255, 255,
        255,   0, 255,
    ];
    PixelBuffer::from_vec(data, 4, 2, PixelDescriptor::RGB8_SRGB)
        .expect("valid test buffer")
}

/// Create a 3x2 Gray8 test image.
fn test_gray8_pixels() -> PixelBuffer {
    let data: Vec<u8> = vec![0, 128, 255, 64, 192, 32];
    PixelBuffer::from_vec(data, 3, 2, PixelDescriptor::GRAY8_SRGB)
        .expect("valid test buffer")
}

// =========================================================================
// Concrete API tests (generic, no type erasure)
// =========================================================================

#[test]
fn concrete_encode_decode_rgb8_roundtrip() {
    let pixels = test_rgb8_pixels();

    // Encode: Config → Job → Encoder → encode()
    let config = PnmEncoderConfig::new();
    let job = config.job();
    let encoder = job.encoder().expect("encoder creation");
    let output = encoder.encode(pixels.as_slice()).expect("encode");

    assert_eq!(output.format(), ImageFormat::Pnm);
    assert!(!output.is_empty());

    // Verify PPM header
    let encoded = output.data();
    assert!(encoded.starts_with(b"P6\n4 2\n255\n"));

    // Decode: Config → Job → probe + decoder → decode()
    let dec_config = PnmDecoderConfig::new();
    let dec_job = dec_config.job();

    // Probe first
    let info = dec_job.probe(encoded).expect("probe");
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);
    assert_eq!(info.format, ImageFormat::Pnm);

    // Full decode
    let decoder = dec_job.decoder(encoded, &[]).expect("decoder creation");
    let decoded = decoder.decode().expect("decode");

    // Verify roundtrip
    let orig = pixels.as_slice();
    let result = decoded.pixels();
    assert_eq!(orig.width(), result.width());
    assert_eq!(orig.rows(), result.rows());
    assert_eq!(orig.descriptor(), result.descriptor());
    for y in 0..orig.rows() {
        assert_eq!(orig.row(y), result.row(y), "row {y} mismatch");
    }
}

#[test]
fn concrete_encode_decode_gray8_roundtrip() {
    let pixels = test_gray8_pixels();

    let config = PnmEncoderConfig::new();
    let encoder = config.job().encoder().expect("encoder");
    let output = encoder.encode(pixels.as_slice()).expect("encode");

    // Verify PGM header
    let encoded = output.data();
    assert!(encoded.starts_with(b"P5\n3 2\n255\n"));

    let dec_config = PnmDecoderConfig::new();
    let decoder = dec_config.job().decoder(encoded, &[]).expect("decoder");
    let decoded = decoder.decode().expect("decode");

    let orig = pixels.as_slice();
    let result = decoded.pixels();
    assert_eq!(orig.descriptor(), result.descriptor());
    for y in 0..orig.rows() {
        assert_eq!(orig.row(y), result.row(y), "row {y} mismatch");
    }
}

// =========================================================================
// Dyn-dispatch API tests (object-safe, no generics)
// =========================================================================

#[test]
fn dyn_encode_decode_rgb8_roundtrip() {
    let pixels = test_rgb8_pixels();

    // Encode via DynEncoderConfig
    let config = PnmEncoderConfig::new();
    let enc_config: &dyn DynEncoderConfig = &config;

    assert_eq!(enc_config.format(), ImageFormat::Pnm);
    assert!(!enc_config.supported_descriptors().is_empty());

    let enc_job = enc_config.dyn_job();
    let encoder = enc_job.into_encoder().expect("dyn encoder");
    let output = encoder.encode(pixels.as_slice()).expect("dyn encode");

    let encoded = output.into_vec();

    // Decode via DynDecoderConfig
    let dec_config = PnmDecoderConfig::new();
    let dyn_dec: &dyn DynDecoderConfig = &dec_config;

    assert_eq!(dyn_dec.format(), ImageFormat::Pnm);

    let dec_job = dyn_dec.dyn_job();

    // Probe via dyn job
    let info = dec_job.probe(&encoded).expect("dyn probe");
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);

    // Decode via dyn decoder
    let decoder = dec_job.into_decoder(&encoded, &[]).expect("dyn decoder");
    let decoded = decoder.decode().expect("dyn decode");

    let orig = test_rgb8_pixels();
    let result = decoded.pixels();
    assert_eq!(orig.as_slice().width(), result.width());
    assert_eq!(orig.as_slice().rows(), result.rows());
    for y in 0..result.rows() {
        assert_eq!(orig.as_slice().row(y), result.row(y), "row {y} mismatch");
    }
}

#[test]
fn dyn_encode_decode_gray8_roundtrip() {
    let pixels = test_gray8_pixels();

    let enc_config = PnmEncoderConfig::new();
    let enc: &dyn DynEncoderConfig = &enc_config;
    let output = enc
        .dyn_job()
        .into_encoder()
        .expect("dyn encoder")
        .encode(pixels.as_slice())
        .expect("dyn encode");

    let encoded = output.into_vec();

    let dec_config = PnmDecoderConfig::new();
    let dec: &dyn DynDecoderConfig = &dec_config;
    let decoded = dec
        .dyn_job()
        .into_decoder(&encoded, &[])
        .expect("dyn decoder")
        .decode()
        .expect("dyn decode");

    let orig = pixels.as_slice();
    let result = decoded.pixels();
    assert_eq!(orig.descriptor(), result.descriptor());
    for y in 0..orig.rows() {
        assert_eq!(orig.row(y), result.row(y), "row {y} mismatch");
    }
}

// =========================================================================
// Codec-agnostic helper function (demonstrates the dispatch pattern)
// =========================================================================

fn encode_with_any_codec(
    config: &dyn DynEncoderConfig,
    pixels: PixelSlice<'_>,
) -> Result<Vec<u8>, zc::encode::BoxedError> {
    let job = config.dyn_job();
    let encoder = job.into_encoder()?;
    Ok(encoder.encode(pixels)?.into_vec())
}

fn decode_with_any_codec(
    config: &dyn DynDecoderConfig,
    data: &[u8],
) -> Result<PixelBuffer, zc::decode::BoxedError> {
    let job = config.dyn_job();
    let decoder = job.into_decoder(data, &[])?;
    Ok(decoder.decode()?.into_buffer())
}

#[test]
fn codec_agnostic_roundtrip() {
    let pixels = test_rgb8_pixels();

    let enc_config = PnmEncoderConfig::new();
    let encoded =
        encode_with_any_codec(&enc_config, pixels.as_slice()).expect("codec-agnostic encode");

    let dec_config = PnmDecoderConfig::new();
    let decoded = decode_with_any_codec(&dec_config, &encoded).expect("codec-agnostic decode");

    let orig = pixels.as_slice();
    let result = decoded.as_slice();
    for y in 0..orig.rows() {
        assert_eq!(orig.row(y), result.row(y), "row {y} mismatch");
    }
}

// =========================================================================
// Config trait static methods
// =========================================================================

#[test]
fn encoder_config_static_methods() {
    assert_eq!(<PnmEncoderConfig as EncoderConfig>::format(), ImageFormat::Pnm);

    let descs = <PnmEncoderConfig as EncoderConfig>::supported_descriptors();
    assert!(descs.contains(&PixelDescriptor::RGB8_SRGB));
    assert!(descs.contains(&PixelDescriptor::GRAY8_SRGB));

    let caps = <PnmEncoderConfig as EncoderConfig>::capabilities();
    assert!(caps.lossless());
}

#[test]
fn decoder_config_static_methods() {
    assert_eq!(<PnmDecoderConfig as DecoderConfig>::format(), ImageFormat::Pnm);

    let descs = <PnmDecoderConfig as DecoderConfig>::supported_descriptors();
    assert!(descs.contains(&PixelDescriptor::RGB8_SRGB));
    assert!(descs.contains(&PixelDescriptor::GRAY8_SRGB));

    let caps = <PnmDecoderConfig as DecoderConfig>::capabilities();
    assert!(caps.cheap_probe());
}

// =========================================================================
// Probing and output_info
// =========================================================================

#[test]
fn probe_and_output_info() {
    let pixels = test_rgb8_pixels();
    let config = PnmEncoderConfig::new();
    let encoded = config
        .job()
        .encoder()
        .unwrap()
        .encode(pixels.as_slice())
        .unwrap()
        .into_vec();

    let dec_config = PnmDecoderConfig::new();
    let job = dec_config.job();

    let info = job.probe(&encoded).unwrap();
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);

    let out_info = job.output_info(&encoded).unwrap();
    assert_eq!(out_info.width, 4);
    assert_eq!(out_info.height, 2);
    assert_eq!(out_info.native_format, PixelDescriptor::RGB8_SRGB);
}

// =========================================================================
// Resource limits
// =========================================================================

#[test]
fn decode_respects_dimension_limits() {
    let pixels = test_rgb8_pixels();
    let config = PnmEncoderConfig::new();
    let encoded = config
        .job()
        .encoder()
        .unwrap()
        .encode(pixels.as_slice())
        .unwrap()
        .into_vec();

    // Set limits that reject 4x2 images
    let limits = ResourceLimits::none().with_max_width(2);
    let dec_config = PnmDecoderConfig::new();
    let job = dec_config.job().with_limits(limits);
    let result = job.decoder(&encoded, &[]);

    assert!(result.is_err(), "should reject image exceeding width limit");
}

// =========================================================================
// Error cases
// =========================================================================

#[test]
fn decode_invalid_data() {
    let dec_config = PnmDecoderConfig::new();
    let job = dec_config.job();
    let result = job.probe(b"not a pnm file");
    assert!(result.is_err());
}

#[test]
fn decode_truncated() {
    let dec_config = PnmDecoderConfig::new();
    let result = dec_config.job().probe(b"P6");
    assert!(result.is_err());
}

#[test]
fn unsupported_animation_encode() {
    let config = PnmEncoderConfig::new();
    let job = config.job();
    let result = job.frame_encoder();
    assert!(result.is_err(), "PNM has no animation support");
}

#[test]
fn unsupported_streaming_decode() {
    let pixels = test_rgb8_pixels();
    let config = PnmEncoderConfig::new();
    let encoded = config
        .job()
        .encoder()
        .unwrap()
        .encode(pixels.as_slice())
        .unwrap()
        .into_vec();

    let dec_config = PnmDecoderConfig::new();
    let job = dec_config.job();
    let result = job.streaming_decoder(&encoded, &[]);
    assert!(result.is_err(), "PNM has no streaming decode");
}

#[test]
fn unsupported_animation_decode() {
    let pixels = test_rgb8_pixels();
    let config = PnmEncoderConfig::new();
    let encoded = config
        .job()
        .encoder()
        .unwrap()
        .encode(pixels.as_slice())
        .unwrap()
        .into_vec();

    let dec_config = PnmDecoderConfig::new();
    let job = dec_config.job();
    let result = job.frame_decoder(&encoded, &[]);
    assert!(result.is_err(), "PNM has no animation decode");
}
