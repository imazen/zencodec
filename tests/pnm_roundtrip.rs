//! Integration test exercising the full zencodec-types API via a PNM codec.
//!
//! Tests both concrete (generic) and dyn-dispatch (object-safe) paths.
//! The PNM codec uses `whereat::At<PnmError>` as its error type to validate
//! that location traces survive dyn dispatch and error chain inspection.

mod pnm;

use std::borrow::Cow;

use pnm::{PnmDecoderConfig, PnmEncoderConfig};

use zc::decode::{Decode, DecodeJob, DecoderConfig, DynDecoderConfig};
use zc::encode::{DynEncoderConfig, EncodeJob, Encoder, EncoderConfig};
use zc::{ImageFormat, ResourceLimits, UnsupportedOperation};
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
    PixelBuffer::from_vec(data, 4, 2, PixelDescriptor::RGB8_SRGB).expect("valid test buffer")
}

/// Create a 3x2 Gray8 test image.
fn test_gray8_pixels() -> PixelBuffer {
    let data: Vec<u8> = vec![0, 128, 255, 64, 192, 32];
    PixelBuffer::from_vec(data, 3, 2, PixelDescriptor::GRAY8_SRGB).expect("valid test buffer")
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

    assert_eq!(output.mime_type(), "image/x-portable-anymap");
    assert_eq!(output.extension(), "pnm");

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
    let decoder = dec_job.decoder(Cow::Borrowed(encoded), &[]).expect("decoder creation");
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
    let decoder = dec_config.job().decoder(Cow::Borrowed(encoded), &[]).expect("decoder");
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

    assert_eq!(dyn_dec.formats(), &[ImageFormat::Pnm]);

    let dec_job = dyn_dec.dyn_job();

    // Probe via dyn job
    let info = dec_job.probe(&encoded).expect("dyn probe");
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);

    // Decode via dyn decoder
    let decoder = dec_job.into_decoder(Cow::Borrowed(&encoded), &[]).expect("dyn decoder");
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
        .into_decoder(Cow::Borrowed(&encoded), &[])
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
    let decoder = job.into_decoder(Cow::Borrowed(data), &[])?;
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
    assert_eq!(
        <PnmEncoderConfig as EncoderConfig>::format(),
        ImageFormat::Pnm
    );

    let descs = <PnmEncoderConfig as EncoderConfig>::supported_descriptors();
    assert!(descs.contains(&PixelDescriptor::RGB8_SRGB));
    assert!(descs.contains(&PixelDescriptor::GRAY8_SRGB));

    let caps = <PnmEncoderConfig as EncoderConfig>::capabilities();
    assert!(caps.lossless());
}

#[test]
fn decoder_config_static_methods() {
    assert_eq!(
        <PnmDecoderConfig as DecoderConfig>::formats(),
        &[ImageFormat::Pnm]
    );

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
    let result = job.decoder(Cow::Borrowed(&encoded), &[]);

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
    let result = job.full_frame_encoder();
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
    let result = job.streaming_decoder(Cow::Borrowed(&encoded), &[]);
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
    let result = job.full_frame_decoder(Cow::Borrowed(&encoded), &[]);
    assert!(result.is_err(), "PNM has no animation decode");
}

// =========================================================================
// Error ergonomics: find_cause through dyn dispatch
// =========================================================================

#[test]
fn find_cause_limit_exceeded_through_dyn_decode() {
    let pixels = test_rgb8_pixels();
    let config = PnmEncoderConfig::new();
    let encoded = config
        .job()
        .encoder()
        .unwrap()
        .encode(pixels.as_slice())
        .unwrap()
        .into_vec();

    // Decode with limits that reject this image — through dyn dispatch
    let limits = ResourceLimits::none().with_max_width(2);
    let dec_config = PnmDecoderConfig::new();
    let dyn_dec: &dyn DynDecoderConfig = &dec_config;

    let mut job = dyn_dec.dyn_job();
    job.set_limits(limits);
    let result = job.into_decoder(Cow::Borrowed(&encoded), &[]);

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("should fail with limit exceeded"),
    };

    // The error is BoxedError containing At<PnmError>.
    // At<PnmError>::source() delegates to PnmError::source(),
    // which for #[from] LimitExceeded returns Some(&LimitExceeded).
    // find_cause walks: At<PnmError> → PnmError::source() → LimitExceeded
    let limit = zc::find_cause::<zc::LimitExceeded>(&*err);
    assert!(
        limit.is_some(),
        "find_cause should find LimitExceeded through BoxedError → At<PnmError> chain"
    );
}

#[test]
fn find_cause_unsupported_through_dyn_decode() {
    let pixels = test_rgb8_pixels();
    let config = PnmEncoderConfig::new();
    let encoded = config
        .job()
        .encoder()
        .unwrap()
        .encode(pixels.as_slice())
        .unwrap()
        .into_vec();

    // Try streaming decode (unsupported) through dyn dispatch
    let dec_config = PnmDecoderConfig::new();
    let dyn_dec: &dyn DynDecoderConfig = &dec_config;

    let job = dyn_dec.dyn_job();
    let result = job.into_streaming_decoder(Cow::Borrowed(&encoded), &[]);

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("streaming decode should fail"),
    };

    let op = zc::find_cause::<UnsupportedOperation>(&*err);
    assert!(
        op.is_some(),
        "find_cause should find UnsupportedOperation through BoxedError → At<PnmError>"
    );
    assert_eq!(op.unwrap(), &UnsupportedOperation::RowLevelDecode);
}

#[test]
fn find_cause_unsupported_through_dyn_encode() {
    let enc_config = PnmEncoderConfig::new();
    let dyn_enc: &dyn DynEncoderConfig = &enc_config;

    let job = dyn_enc.dyn_job();
    let result = job.into_full_frame_encoder();

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("animation encode should fail"),
    };

    let op = zc::find_cause::<UnsupportedOperation>(&*err);
    assert!(
        op.is_some(),
        "find_cause should find UnsupportedOperation through BoxedError → At<PnmError>"
    );
    assert_eq!(op.unwrap(), &UnsupportedOperation::AnimationEncode);
}

#[test]
fn concrete_error_preserves_at_wrapper() {
    // Verify that At<PnmError> is accessible through BoxedError via downcast
    let pixels = test_rgb8_pixels();
    let config = PnmEncoderConfig::new();
    let encoded = config
        .job()
        .encoder()
        .unwrap()
        .encode(pixels.as_slice())
        .unwrap()
        .into_vec();

    // Trigger a LimitExceeded through dyn dispatch
    let limits = ResourceLimits::none().with_max_height(1);
    let dec_config = PnmDecoderConfig::new();
    let dyn_dec: &dyn DynDecoderConfig = &dec_config;

    let mut job = dyn_dec.dyn_job();
    job.set_limits(limits);
    let err = match job.into_decoder(Cow::Borrowed(&encoded), &[]) {
        Err(e) => e,
        Ok(_) => panic!("should fail with limit exceeded"),
    };

    // BoxedError contains At<PnmError> — downcast to access it
    let at_err = err.downcast_ref::<whereat::At<pnm::PnmError>>();
    assert!(
        at_err.is_some(),
        "BoxedError should be downcastable to At<PnmError>"
    );

    // Access the inner PnmError through At::error()
    let pnm_err = at_err.unwrap().error();
    assert!(
        matches!(pnm_err, pnm::PnmError::LimitExceeded(_)),
        "inner error should be the LimitExceeded variant"
    );
}

#[test]
fn whereat_trace_has_location() {
    // Verify that At<PnmError> captures source location via start_at()
    let dec_config = PnmDecoderConfig::new();
    let job = dec_config.job();
    let err = job.probe(b"not a pnm").expect_err("should fail");

    // The error is At<PnmError> with at least one location frame
    assert!(
        err.frame_count() > 0,
        "At<PnmError> from start_at() should have at least one location frame"
    );

    // Check that the trace includes a file path (from #[track_caller])
    let debug_str = format!("{:?}", err);
    assert!(
        debug_str.contains("pnm"),
        "Debug output should contain source file reference: {debug_str}"
    );
}

#[test]
fn find_cause_returns_none_for_absent_type() {
    let dec_config = PnmDecoderConfig::new();
    let dyn_dec: &dyn DynDecoderConfig = &dec_config;

    let job = dyn_dec.dyn_job();
    let err = job.probe(b"not a pnm").expect_err("should fail");

    // InvalidData doesn't have LimitExceeded in its source chain
    assert!(
        zc::find_cause::<zc::LimitExceeded>(&*err).is_none(),
        "find_cause should return None when cause type is absent"
    );
}

// =========================================================================
// Sink exploration tests
//
// These tests exercise DecodeRowSink in various use cases to surface
// design constraints and gaps in the current trait.
// =========================================================================

use zc::decode::{DecodeRowSink, SinkError};
use zenpixels::PixelSliceMut;

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

fn lcm(a: usize, b: usize) -> usize {
    a / gcd(a, b) * b
}

/// Use case: decode_into alternative.
///
/// A pre-allocated buffer sink knows its dimensions and format upfront.
/// The caller allocates the buffer before decode starts, and the sink
/// just hands out slices into it.
///
/// This works well with the current trait — the sink validates that the
/// codec's descriptor matches what was pre-allocated.
#[test]
fn sink_preallocated_buffer() {
    // Encode a test image
    let pixels = test_rgb8_pixels(); // 4x2 RGB8
    let config = PnmEncoderConfig::new();
    let encoded = config.job().encoder().unwrap().encode(pixels.as_slice()).unwrap();

    // Pre-allocate a buffer for decode output
    let width = 4u32;
    let height = 2u32;
    let desc = PixelDescriptor::RGB8_SRGB;
    let bpp = desc.bytes_per_pixel();
    let stride = width as usize * bpp;
    let mut buffer = vec![0u8; stride * height as usize];

    struct PreallocSink<'a> {
        buf: &'a mut [u8],
        expected_desc: PixelDescriptor,
        expected_width: u32,
        total_height: u32,
        stride: usize,
    }

    impl DecodeRowSink for PreallocSink<'_> {
        fn demand(
            &mut self,
            y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            // Validate format matches what we pre-allocated for
            if descriptor != self.expected_desc {
                return Err(format!(
                    "format mismatch: expected {:?}, got {:?}",
                    self.expected_desc, descriptor
                ).into());
            }
            if width != self.expected_width {
                return Err(format!(
                    "width mismatch: expected {}, got {}",
                    self.expected_width, width
                ).into());
            }
            if y + height > self.total_height {
                return Err("strip exceeds buffer bounds".into());
            }

            let start = y as usize * self.stride;
            let bpp = descriptor.bytes_per_pixel();
            let row_bytes = width as usize * bpp;
            let end = start + (height as usize - 1) * self.stride + row_bytes;
            Ok(PixelSliceMut::new(
                &mut self.buf[start..end],
                width,
                height,
                self.stride,
                descriptor,
            ).expect("valid slice"))
        }
    }

    let mut sink = PreallocSink {
        buf: &mut buffer,
        expected_desc: desc,
        expected_width: width,
        total_height: height,
        stride,
    };

    // Decode via push_decoder
    let dec_config = PnmDecoderConfig::new();
    let job = dec_config.job();
    let info = job.push_decoder(Cow::Borrowed(encoded.data()), &mut sink, &[])
        .expect("push_decoder");

    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);
    assert_eq!(info.native_format, PixelDescriptor::RGB8_SRGB);

    // Verify pixel data matches original
    let orig = pixels.as_slice();
    for y in 0..height {
        let orig_row = orig.row(y);
        let start = y as usize * stride;
        let end = start + stride;
        assert_eq!(&buffer[start..end], orig_row, "row {y} mismatch");
    }
}

/// Use case: format-constrained sink.
///
/// The sink only accepts RGBA8 but the codec produces RGB8.
/// With the current API, the sink discovers the mismatch at the first
/// demand() call and must reject — there's no upfront negotiation.
///
/// This test demonstrates the gap: the sink can't tell the decoder
/// "please give me RGBA8 instead" at demand() time. The `preferred`
/// parameter on decoder() is the only negotiation point, but it's
/// on the decoder side, not the sink side.
#[test]
fn sink_format_mismatch_rejected() {
    let pixels = test_rgb8_pixels();
    let config = PnmEncoderConfig::new();
    let _encoded = config.job().encoder().unwrap().encode(pixels.as_slice()).unwrap();

    struct Rgba8OnlySink {
        buf: Vec<u8>,
    }

    impl DecodeRowSink for Rgba8OnlySink {
        fn demand(
            &mut self,
            _y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            // This sink only accepts RGBA8
            if descriptor != PixelDescriptor::RGBA8_SRGB {
                return Err(format!(
                    "sink requires RGBA8, got {:?}",
                    descriptor
                ).into());
            }
            let bpp = descriptor.bytes_per_pixel();
            let stride = width as usize * bpp;
            let needed = height as usize * stride;
            self.buf.resize(needed, 0);
            Ok(PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                .expect("valid slice"))
        }
    }

    let mut sink = Rgba8OnlySink { buf: Vec::new() };

    // PNM produces RGB8, sink wants RGBA8.
    // push_decoder's default impl decodes first, then pushes to sink.
    // The sink rejects at demand() time — but decode already happened.
    //
    // Note: push_decoder's default uses FullFrameDec::wrap_sink_error,
    // which panics for PNM (Unsupported stub). We test via manual
    // simulation instead, which is more representative of how a real
    // codec with streaming decode would hit this.
    let pixels_decoded = test_rgb8_pixels();
    let ps = pixels_decoded.as_slice();
    let desc = ps.descriptor();

    let result = sink.demand(0, ps.rows(), ps.width(), desc);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("RGBA8") || err_msg.contains("sink requires"), "{err_msg}");
}

/// Use case: row-processing pipeline sink.
///
/// The sink processes each strip as it arrives (e.g., color conversion,
/// downsampling, or writing to a file). The current API implicitly
/// signals completion of a strip by calling demand() again, which works
/// but means the sink can only process on the *next* demand() call.
/// The last strip's completion is implicit when push_decoder returns.
///
/// This test verifies the implicit-completion pattern works and shows
/// where an explicit rows_written() signal would help.
#[test]
fn sink_row_processing_pipeline() {
    let pixels = test_rgb8_pixels(); // 4x2 RGB8
    let config = PnmEncoderConfig::new();
    let encoded = config.job().encoder().unwrap().encode(pixels.as_slice()).unwrap();

    struct ProcessingSink {
        strip_buf: Vec<u8>,
        // Accumulates processed output: sum of each row's R channel values
        row_sums: Vec<u32>,
        pending_strip: Option<(u32, u32, u32)>, // (y, height, width) of previous strip
    }

    impl ProcessingSink {
        fn process_pending(&mut self) {
            if let Some((y, height, width)) = self.pending_strip.take() {
                // Process the strip that was just written
                let bpp = 3; // RGB8
                let stride = width as usize * bpp;
                for row_idx in 0..height {
                    let row_start = row_idx as usize * stride;
                    let mut sum = 0u32;
                    for x in 0..width as usize {
                        sum += self.strip_buf[row_start + x * bpp] as u32; // R channel
                    }
                    self.row_sums.push(sum);
                    let _ = y; // would use y + row_idx for positioning
                }
            }
        }
    }

    impl DecodeRowSink for ProcessingSink {
        fn demand(
            &mut self,
            y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            // Process the *previous* strip before handing out the buffer
            // for the next one. This is the implicit completion signal.
            self.process_pending();

            self.pending_strip = Some((y, height, width));
            let bpp = descriptor.bytes_per_pixel();
            let stride = width as usize * bpp;
            let needed = height as usize * stride;
            self.strip_buf.resize(needed, 0);
            Ok(PixelSliceMut::new(&mut self.strip_buf, width, height, stride, descriptor)
                .expect("valid slice"))
        }
    }

    let mut sink = ProcessingSink {
        strip_buf: Vec::new(),
        row_sums: Vec::new(),
        pending_strip: None,
    };

    let dec_config = PnmDecoderConfig::new();
    dec_config.job().push_decoder(
        Cow::Borrowed(encoded.data()),
        &mut sink,
        &[],
    ).expect("push_decoder");

    // Process the last strip — this is the gap. The caller must know
    // to call process_pending() after push_decoder returns. An explicit
    // finish() method on the sink would solve this.
    sink.process_pending();

    // Verify we processed all rows
    assert_eq!(sink.row_sums.len(), 2, "should have processed 2 rows");

    // Row 0: R values are 255, 0, 0, 255 → sum = 510
    assert_eq!(sink.row_sums[0], 510);
    // Row 1: R values are 0, 255, 0, 255 → sum = 510
    assert_eq!(sink.row_sums[1], 510);
}

/// Use case: completion-aware sink (flush to output).
///
/// The sink writes decoded rows to an output buffer and needs to
/// finalize when decode completes. Currently, the sink has no
/// way to know decode is done except by the fact that push_decoder()
/// returned.
///
/// This test shows a pattern where the sink accumulates data and
/// the caller must explicitly finalize after decode.
#[test]
fn sink_completion_aware() {
    let pixels = test_gray8_pixels(); // 3x2 Gray8
    let config = PnmEncoderConfig::new();
    let encoded = config.job().encoder().unwrap().encode(pixels.as_slice()).unwrap();

    struct AccumulatingSink {
        output: PixelBuffer,
        rows_received: u32,
    }

    impl AccumulatingSink {
        fn new(width: u32, height: u32, desc: PixelDescriptor) -> Self {
            Self {
                output: PixelBuffer::new(width, height, desc),
                rows_received: 0,
            }
        }

        fn is_complete(&self) -> bool {
            self.rows_received == self.output.height()
        }
    }

    impl DecodeRowSink for AccumulatingSink {
        fn demand(
            &mut self,
            y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            if descriptor != self.output.descriptor() {
                return Err("format mismatch".into());
            }
            if width != self.output.width() {
                return Err("width mismatch".into());
            }
            // Track how many rows we've received
            self.rows_received = y + height;

            // Return a view into the pre-allocated buffer at the right offset
            Ok(self.output.rows_mut(y, height))
        }
    }

    // First: probe to learn dimensions and format
    let dec_config = PnmDecoderConfig::new();
    let job = dec_config.job();
    let out_info = job.output_info(encoded.data()).expect("output_info");

    let mut sink = AccumulatingSink::new(out_info.width, out_info.height, out_info.native_format);

    let dec_config2 = PnmDecoderConfig::new();
    dec_config2.job().push_decoder(
        Cow::Borrowed(encoded.data()),
        &mut sink,
        &[],
    ).expect("push_decoder");

    // After push_decoder returns, the sink should be complete
    assert!(sink.is_complete(), "sink should have received all rows");
    assert_eq!(sink.rows_received, 2);

    // Verify the accumulated output matches original
    let orig = pixels.as_slice();
    let result = sink.output.as_slice();
    for y in 0..orig.rows() {
        assert_eq!(orig.row(y), result.row(y), "row {y} mismatch");
    }
}

/// Use case: SIMD-aligned decode_into with stride padding.
///
/// A real imaging pipeline often needs SIMD-aligned rows. The sink
/// can provide buffers with padded stride, and the codec writes only
/// the pixel data portion via row_mut(). Padding bytes are untouched.
#[test]
fn sink_simd_aligned_decode_into() {
    let pixels = test_rgb8_pixels(); // 4x2 RGB8
    let config = PnmEncoderConfig::new();
    let encoded = config.job().encoder().unwrap().encode(pixels.as_slice()).unwrap();

    struct AlignedDecodeIntoSink {
        buf: Vec<u8>,
        width: u32,
        height: u32,
        stride: usize,
        desc: PixelDescriptor,
    }

    impl AlignedDecodeIntoSink {
        fn new(width: u32, height: u32, desc: PixelDescriptor) -> Self {
            let bpp = desc.bytes_per_pixel();
            let row_bytes = width as usize * bpp;
            // Align to next multiple of 64 that is also a multiple of bpp
            // (PixelSliceMut requires stride to be pixel-aligned)
            let align = lcm(64, bpp);
            let stride = row_bytes.div_ceil(align) * align;
            let total = if height > 0 {
                (height as usize - 1) * stride + row_bytes
            } else {
                0
            };
            Self {
                buf: vec![0xAA; total], // fill with sentinel
                width,
                height,
                stride,
                desc,
            }
        }
    }

    impl DecodeRowSink for AlignedDecodeIntoSink {
        fn demand(
            &mut self,
            y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            if descriptor != self.desc || width != self.width {
                return Err("format/width mismatch".into());
            }
            if y + height > self.height {
                return Err("out of bounds".into());
            }

            let bpp = descriptor.bytes_per_pixel();
            let row_bytes = width as usize * bpp;
            let start = y as usize * self.stride;
            let end = start + (height as usize - 1) * self.stride + row_bytes;

            Ok(PixelSliceMut::new(
                &mut self.buf[start..end],
                width,
                height,
                self.stride,
                descriptor,
            ).expect("valid"))
        }
    }

    let dec_config = PnmDecoderConfig::new();
    let job = dec_config.job();
    let out_info = job.output_info(encoded.data()).unwrap();

    let mut sink = AlignedDecodeIntoSink::new(
        out_info.width,
        out_info.height,
        out_info.native_format,
    );

    let dec_config2 = PnmDecoderConfig::new();
    dec_config2.job().push_decoder(
        Cow::Borrowed(encoded.data()),
        &mut sink,
        &[],
    ).expect("push_decoder");

    // Verify stride is 64-byte aligned AND pixel-aligned
    // For RGB8 (bpp=3), lcm(64,3)=192
    assert_eq!(sink.stride % 64, 0, "stride should be 64-byte aligned");
    assert_eq!(sink.stride % 3, 0, "stride should be pixel-aligned for RGB8");
    assert_eq!(sink.stride, 192);
    // Verify row bytes are 12 (4 pixels × 3 bytes)
    let row_bytes = 4 * 3;
    assert_eq!(row_bytes, 12);

    // Verify pixel data matches, reading with stride
    let orig = pixels.as_slice();
    for y in 0..2u32 {
        let start = y as usize * sink.stride;
        let orig_row = orig.row(y);
        assert_eq!(
            &sink.buf[start..start + row_bytes],
            orig_row,
            "row {y} pixel data mismatch"
        );
    }
}

/// Use case: dyn dispatch with push_decoder.
///
/// The sink is used through &mut dyn DecodeRowSink, and the decoder
/// is created through the dyn dispatch path. This validates that
/// the sink works correctly across type erasure boundaries.
#[test]
fn sink_through_dyn_dispatch() {
    let pixels = test_rgb8_pixels();
    let config = PnmEncoderConfig::new();
    let encoded = config.job().encoder().unwrap().encode(pixels.as_slice()).unwrap();

    struct CollectSink {
        buf: Vec<u8>,
        desc: Option<PixelDescriptor>,
        dimensions: Option<(u32, u32)>,
    }

    impl DecodeRowSink for CollectSink {
        fn demand(
            &mut self,
            y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            // Record what the codec told us (first call)
            if self.desc.is_none() {
                self.desc = Some(descriptor);
            }
            // Track total dimensions
            self.dimensions = Some((width, y + height));

            let bpp = descriptor.bytes_per_pixel();
            let stride = width as usize * bpp;
            let needed = height as usize * stride;
            self.buf.resize(needed, 0);
            Ok(PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                .expect("valid"))
        }
    }

    let mut sink = CollectSink {
        buf: Vec::new(),
        desc: None,
        dimensions: None,
    };

    // Use the concrete push_decoder path (since DynDecodeJob doesn't
    // have push_decoder yet — that's another gap to note)
    let dec_config = PnmDecoderConfig::new();
    dec_config.job().push_decoder(
        Cow::Borrowed(encoded.data()),
        &mut sink as &mut dyn DecodeRowSink,
        &[],
    ).expect("push_decoder");

    assert_eq!(sink.desc, Some(PixelDescriptor::RGB8_SRGB));
    assert_eq!(sink.dimensions, Some((4, 2)));
}

/// Use case: sink that discovers format from the first demand() call.
///
/// When the caller doesn't know the output format upfront (e.g., the
/// codec might produce RGB8 or Gray8 depending on the input), the
/// sink must defer allocation until the first demand() call reveals
/// the format.
///
/// This works with the current API but means the sink can't validate
/// format compatibility before decode begins. With a negotiation step,
/// the sink could declare acceptable formats upfront.
#[test]
fn sink_deferred_allocation() {
    let pixels = test_gray8_pixels(); // Gray8 — sink doesn't know this in advance
    let config = PnmEncoderConfig::new();
    let encoded = config.job().encoder().unwrap().encode(pixels.as_slice()).unwrap();

    struct DeferredSink {
        buf: Option<PixelBuffer>,
    }

    impl DecodeRowSink for DeferredSink {
        fn demand(
            &mut self,
            y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            // Allocate on first call, when we learn the format
            if self.buf.is_none() {
                // Problem: we don't know total height here. We only know
                // the current strip height. For push_decoder's default impl,
                // the entire image comes in one strip, but a streaming codec
                // might give us 8-row strips. We'd need output_info() or
                // probe() results to pre-allocate correctly.
                //
                // Workaround: grow the buffer on each demand(). But that
                // means we can't return a view into a pre-sized buffer.
                // For now, assume single-strip (push_decoder default).
                let total_h = y + height; // only correct if single-strip!
                self.buf = Some(PixelBuffer::new(width, total_h, descriptor));
            }

            let buf = self.buf.as_mut().unwrap();
            Ok(buf.rows_mut(y, height))
        }
    }

    let mut sink = DeferredSink { buf: None };

    let dec_config = PnmDecoderConfig::new();
    dec_config.job().push_decoder(
        Cow::Borrowed(encoded.data()),
        &mut sink,
        &[],
    ).expect("push_decoder");

    let buf = sink.buf.expect("should have allocated");
    assert_eq!(buf.width(), 3);
    assert_eq!(buf.height(), 2);
    assert_eq!(buf.descriptor(), PixelDescriptor::GRAY8_SRGB);

    // Verify data
    let orig = pixels.as_slice();
    let result = buf.as_slice();
    for y in 0..orig.rows() {
        assert_eq!(orig.row(y), result.row(y), "row {y} mismatch");
    }
}

/// Use case: multi-strip streaming sink.
///
/// Simulates what a real streaming codec (like JPEG with MCU strips)
/// would do: multiple demand() calls, each for a subset of rows.
/// Tests that the sink correctly handles incremental strips being
/// written into a pre-allocated buffer.
#[test]
fn sink_multi_strip_simulation() {
    // Create a larger test image to exercise multi-strip
    let width = 8u32;
    let height = 24u32;
    let desc = PixelDescriptor::RGB8_SRGB;
    let bpp = desc.bytes_per_pixel();
    let mut data = vec![0u8; width as usize * height as usize * bpp];
    // Fill with row-dependent pattern
    for y in 0..height {
        for x in 0..width {
            let idx = (y as usize * width as usize + x as usize) * bpp;
            data[idx] = y as u8;       // R = row index
            data[idx + 1] = x as u8;   // G = col index
            data[idx + 2] = 128;       // B = constant
        }
    }
    let source = PixelBuffer::from_vec(data, width, height, desc).unwrap();

    struct MultiStripSink {
        output: PixelBuffer,
        strips_received: Vec<(u32, u32)>, // (y, height) of each strip
    }

    impl DecodeRowSink for MultiStripSink {
        fn demand(
            &mut self,
            y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            if width != self.output.width() || descriptor != self.output.descriptor() {
                return Err("mismatch".into());
            }
            self.strips_received.push((y, height));
            Ok(self.output.rows_mut(y, height))
        }
    }

    let mut sink = MultiStripSink {
        output: PixelBuffer::new(width, height, desc),
        strips_received: Vec::new(),
    };

    // Simulate a codec pushing 8-row strips (like JPEG MCU height)
    let strip_height = 8u32;
    let src = source.as_slice();
    for strip_y in (0..height).step_by(strip_height as usize) {
        let h = strip_height.min(height - strip_y);
        let mut dst = sink.demand(strip_y, h, width, desc).unwrap();
        for row in 0..h {
            dst.row_mut(row).copy_from_slice(src.row(strip_y + row));
        }
    }

    assert_eq!(sink.strips_received.len(), 3);
    assert_eq!(sink.strips_received[0], (0, 8));
    assert_eq!(sink.strips_received[1], (8, 8));
    assert_eq!(sink.strips_received[2], (16, 8));

    // Verify all data
    let result = sink.output.as_slice();
    for y in 0..height {
        assert_eq!(src.row(y), result.row(y), "row {y} mismatch");
    }
}

/// Use case: early abort from sink.
///
/// The sink processes rows and decides to abort partway through
/// (e.g., a cancelled request, or the sink detects the image
/// isn't what it expected). The codec should stop and propagate.
#[test]
fn sink_early_abort() {
    // Build a test image
    let width = 4u32;
    let height = 16u32;
    let desc = PixelDescriptor::RGB8_SRGB;
    let data = vec![128u8; width as usize * height as usize * 3];
    let source = PixelBuffer::from_vec(data, width, height, desc).unwrap();

    struct AbortAfterNSink {
        buf: Vec<u8>,
        strips_before_abort: u32,
        strips_seen: u32,
    }

    impl DecodeRowSink for AbortAfterNSink {
        fn demand(
            &mut self,
            _y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<PixelSliceMut<'_>, SinkError> {
            self.strips_seen += 1;
            if self.strips_seen > self.strips_before_abort {
                return Err("abort: seen enough".into());
            }
            let bpp = descriptor.bytes_per_pixel();
            let stride = width as usize * bpp;
            let needed = height as usize * stride;
            self.buf.resize(needed, 0);
            Ok(PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                .expect("valid"))
        }
    }

    let mut sink = AbortAfterNSink {
        buf: Vec::new(),
        strips_before_abort: 1,
        strips_seen: 0,
    };

    // Simulate codec pushing strips — should abort on second demand()
    let strip_h = 8u32;
    let src = source.as_slice();
    let mut aborted = false;
    for strip_y in (0..height).step_by(strip_h as usize) {
        let h = strip_h.min(height - strip_y);
        match sink.demand(strip_y, h, width, desc) {
            Ok(mut dst) => {
                for row in 0..h {
                    dst.row_mut(row).copy_from_slice(src.row(strip_y + row));
                }
            }
            Err(_) => {
                aborted = true;
                break;
            }
        }
    }

    assert!(aborted, "sink should have aborted");
    assert_eq!(sink.strips_seen, 2); // first succeeds, second aborts
}
