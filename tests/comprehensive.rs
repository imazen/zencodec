//! Comprehensive test suite exercising every aspect of zencodec-types.
//!
//! Uses a mock animation codec (mock_anim) that supports all encoder/decoder
//! modes including streaming decode, animation encode/decode, push_rows,
//! and encode_from. This exercises trait paths that the PNM codec can't reach.

mod mock_anim;

use std::borrow::Cow;

use mock_anim::{MockDecoderConfig, MockEncoderConfig};

use zc::decode::{
    Decode, DecodeCapabilities, DecodeJob, DecodeOutput, DecodePolicy, DecoderConfig,
    DynDecoderConfig, FullFrameDecoder, OutputInfo, StreamingDecode,
};
use zc::encode::{
    DynEncoderConfig, EncodeCapabilities, EncodeJob, EncodeOutput, EncodePolicy, Encoder,
    EncoderConfig, FullFrameEncoder,
};
use zc::{
    CodecErrorExt, FullFrame, GainMapMetadata, ImageFormat, ImageInfo, LimitExceeded, Metadata,
    Orientation, OrientationHint, ResourceLimits, ThreadingPolicy, UnsupportedOperation,
};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

// =========================================================================
// Test data helpers
// =========================================================================

fn make_rgb8_buffer(w: u32, h: u32) -> PixelBuffer {
    let bpp = 3;
    let size = w as usize * h as usize * bpp;
    let mut data = vec![0u8; size];
    // Fill with recognizable pattern: row-dependent
    for y in 0..h as usize {
        for x in 0..w as usize {
            let idx = (y * w as usize + x) * bpp;
            data[idx] = (x & 0xFF) as u8;
            data[idx + 1] = (y & 0xFF) as u8;
            data[idx + 2] = ((x + y) & 0xFF) as u8;
        }
    }
    PixelBuffer::from_vec(data, w, h, PixelDescriptor::RGB8_SRGB).unwrap()
}

fn make_rgba8_buffer(w: u32, h: u32) -> PixelBuffer {
    let bpp = 4;
    let size = w as usize * h as usize * bpp;
    let mut data = vec![0u8; size];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let idx = (y * w as usize + x) * bpp;
            data[idx] = (x & 0xFF) as u8;
            data[idx + 1] = (y & 0xFF) as u8;
            data[idx + 2] = ((x + y) & 0xFF) as u8;
            data[idx + 3] = 255;
        }
    }
    PixelBuffer::from_vec(data, w, h, PixelDescriptor::RGBA8_SRGB).unwrap()
}

/// Encode a single frame via the mock codec, return raw mock bytes.
fn encode_single_frame(buf: &PixelBuffer) -> Vec<u8> {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let encoder = job.encoder().unwrap();
    encoder.encode(buf.as_slice()).unwrap().into_vec()
}

/// Encode multiple frames via the mock animation codec.
fn encode_animation(frames: &[(PixelBuffer, u32)]) -> Vec<u8> {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let mut enc = job.full_frame_encoder().unwrap();
    for (buf, dur) in frames {
        enc.push_frame(buf.as_slice(), *dur, None).unwrap();
    }
    enc.finish(None).unwrap().into_vec()
}

// =========================================================================
// 1. EncoderConfig generic tuning methods
// =========================================================================

#[test]
fn encoder_config_quality_effort_lossless_alpha() {
    let config = MockEncoderConfig::new();
    assert_eq!(config.generic_quality(), None);
    assert_eq!(config.generic_effort(), None);
    assert_eq!(config.is_lossless(), None);
    assert_eq!(config.alpha_quality(), None);

    let config = config
        .with_generic_quality(85.0)
        .with_generic_effort(7)
        .with_lossless(true)
        .with_alpha_quality(90.0);

    assert_eq!(config.generic_quality(), Some(85.0));
    assert_eq!(config.generic_effort(), Some(7));
    assert_eq!(config.is_lossless(), Some(true));
    assert_eq!(config.alpha_quality(), Some(90.0));
}

#[test]
fn encoder_config_clone_preserves_tuning() {
    let config = MockEncoderConfig::new().with_generic_quality(50.0);
    let cloned = config.clone();
    assert_eq!(cloned.generic_quality(), Some(50.0));
}

#[test]
fn encoder_config_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MockEncoderConfig>();
}

#[test]
fn decoder_config_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MockDecoderConfig>();
}

// =========================================================================
// 2. Encoder default methods (push_rows, finish, encode_from)
// =========================================================================

#[test]
fn encoder_push_rows_and_finish() {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let mut enc = job.encoder().unwrap();

    let buf = make_rgb8_buffer(4, 8);
    let ps = buf.as_slice();

    // Push 2 rows at a time
    for strip in 0..4u32 {
        let y = strip * 2;
        // Collect 2 rows of data
        let mut strip_data = Vec::new();
        for r in 0..2u32 {
            strip_data.extend_from_slice(ps.row(y + r));
        }
        let strip_ps =
            PixelSlice::new(&strip_data, 4, 2, 4 * 3, PixelDescriptor::RGB8_SRGB).unwrap();
        enc.push_rows(strip_ps).unwrap();
    }

    let output = enc.finish().unwrap();
    assert!(!output.is_empty());

    // Verify roundtrip — decode the mock data back
    let dec_config = MockDecoderConfig;
    let dec_job = dec_config.job();
    let decoder = dec_job.decoder(Cow::Borrowed(output.data()), &[]).unwrap();
    let result = decoder.decode().unwrap();
    assert_eq!(result.width(), 4);
    assert_eq!(result.height(), 8);
}

#[test]
fn encoder_encode_from_pull_model() {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let enc = job.encoder().unwrap();

    assert_eq!(enc.preferred_strip_height(), 4);

    let buf = make_rgb8_buffer(4, 8);
    let row_bytes = 4 * 3;
    let mut rows_pulled = 0u32;

    let ps = buf.as_slice();
    let output = enc
        .encode_from(&mut |y, mut dst| {
            if y >= 8 {
                return 0;
            }
            let rows_to_write = std::cmp::min(4, 8 - y) as usize;
            for r in 0..rows_to_write {
                let src_row = ps.row(y + r as u32);
                dst.row_mut(r as u32)[..row_bytes].copy_from_slice(src_row);
            }
            rows_pulled += rows_to_write as u32;
            rows_to_write
        })
        .unwrap();

    assert_eq!(rows_pulled, 8);
    assert!(!output.is_empty());
}

// =========================================================================
// 3. encode_srgba8 default implementation
// =========================================================================

#[test]
fn encoder_encode_srgba8_opaque() {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let enc = job.encoder().unwrap();

    let mut data = vec![0u8; 4 * 2 * 4]; // 4x2 RGBA8
    for i in (0..data.len()).step_by(4) {
        data[i] = 255; // R
        data[i + 1] = 128; // G
        data[i + 2] = 64; // B
        data[i + 3] = 255; // A
    }

    let output = enc.encode_srgba8(&mut data, true, 4, 2, 4).unwrap();
    assert!(!output.is_empty());
    assert_eq!(output.format(), ImageFormat::Pnm);
}

#[test]
fn encoder_encode_srgba8_with_alpha() {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let enc = job.encoder().unwrap();

    let mut data = vec![128u8; 2 * 2 * 4]; // 2x2 RGBA8
    let output = enc.encode_srgba8(&mut data, false, 2, 2, 2).unwrap();
    assert!(!output.is_empty());
}

// =========================================================================
// 4. FullFrameEncoder for () stub
// =========================================================================

#[test]
fn unit_full_frame_encoder_rejects() {
    let mut enc = ();
    let buf = make_rgb8_buffer(2, 2);
    let err = enc.push_frame(buf.as_slice(), 100, None).unwrap_err();
    assert_eq!(err, UnsupportedOperation::AnimationEncode);

    let err = ().finish(None).unwrap_err();
    assert_eq!(err, UnsupportedOperation::AnimationEncode);
}

// =========================================================================
// 5. Animation encode + decode roundtrip
// =========================================================================

#[test]
fn animation_encode_decode_roundtrip() {
    let frame1 = make_rgb8_buffer(4, 4);
    let frame2 = make_rgb8_buffer(4, 4);
    // Make separate copies for the assertion after encoding consumes the slices
    let frame1_check = make_rgb8_buffer(4, 4);

    let data = encode_animation(&[(frame1, 100), (frame2, 200)]);

    // Decode as animation
    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job.full_frame_decoder(Cow::Borrowed(&data), &[]).unwrap();

    assert_eq!(dec.frame_count(), Some(2));
    assert_eq!(dec.loop_count(), Some(0));

    // Frame 0
    let f0 = dec.render_next_frame(None).unwrap().unwrap();
    assert_eq!(f0.frame_index(), 0);
    assert_eq!(f0.duration_ms(), 100);
    assert_eq!(f0.pixels().width(), 4);
    assert_eq!(f0.pixels().rows(), 4);
    // Verify pixel data matches
    for y in 0..4 {
        assert_eq!(f0.pixels().row(y), frame1_check.as_slice().row(y));
    }

    // Frame 1
    let f1 = dec.render_next_frame(None).unwrap().unwrap();
    assert_eq!(f1.frame_index(), 1);
    assert_eq!(f1.duration_ms(), 200);

    // No more frames
    assert!(dec.render_next_frame(None).unwrap().is_none());
}

#[test]
fn animation_render_next_frame_owned() {
    let frame1 = make_rgb8_buffer(2, 2);
    let data = encode_animation(&[(frame1, 50)]);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job.full_frame_decoder(Cow::Borrowed(&data), &[]).unwrap();

    let owned = dec.render_next_frame_owned(None).unwrap().unwrap();
    assert_eq!(owned.frame_index(), 0);
    assert_eq!(owned.duration_ms(), 50);
    assert_eq!(owned.pixels().width(), 2);

    // Can use OwnedFullFrame independently
    let borrowed = owned.as_full_frame();
    assert_eq!(borrowed.duration_ms(), 50);

    let recovered = owned.into_buffer();
    assert_eq!(recovered.width(), 2);
}

#[test]
fn animation_render_to_sink() {
    let frame1 = make_rgb8_buffer(4, 2);
    let data = encode_animation(&[(frame1, 100)]);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job.full_frame_decoder(Cow::Borrowed(&data), &[]).unwrap();

    struct CollectSink {
        buf: Vec<u8>,
        began: bool,
        finished: bool,
    }

    impl zc::decode::DecodeRowSink for CollectSink {
        fn begin(
            &mut self,
            _w: u32,
            _h: u32,
            _desc: PixelDescriptor,
        ) -> Result<(), zc::decode::SinkError> {
            self.began = true;
            Ok(())
        }
        fn provide_next_buffer(
            &mut self,
            _y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<zenpixels::PixelSliceMut<'_>, zc::decode::SinkError> {
            let bpp = descriptor.bytes_per_pixel();
            let stride = width as usize * bpp;
            self.buf.resize(height as usize * stride, 0);
            Ok(
                zenpixels::PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                    .unwrap(),
            )
        }
        fn finish(&mut self) -> Result<(), zc::decode::SinkError> {
            self.finished = true;
            Ok(())
        }
    }

    let mut sink = CollectSink {
        buf: Vec::new(),
        began: false,
        finished: false,
    };
    let info = dec
        .render_next_frame_to_sink(None, &mut sink)
        .unwrap()
        .unwrap();
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);
    assert!(sink.began);
    assert!(sink.finished);
}

// =========================================================================
// 6. Streaming decode
// =========================================================================

#[test]
fn streaming_decode_row_by_row() {
    let buf = make_rgb8_buffer(4, 4);
    let data = encode_single_frame(&buf);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut stream = job.streaming_decoder(Cow::Borrowed(&data), &[]).unwrap();

    let info = stream.info();
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 4);

    let mut rows_received = 0u32;
    while let Some((y, strip)) = stream.next_batch().unwrap() {
        assert_eq!(y, rows_received);
        assert_eq!(strip.rows(), 1);
        assert_eq!(strip.width(), 4);
        // Verify pixel data matches
        assert_eq!(strip.row(0), buf.as_slice().row(y));
        rows_received += 1;
    }
    assert_eq!(rows_received, 4);
}

// =========================================================================
// 7. Dyn dispatch: DynDecodeJob set_ methods
// =========================================================================

#[test]
fn dyn_decode_job_set_methods() {
    let config = MockDecoderConfig;
    let dyn_config: &dyn DynDecoderConfig = &config;

    assert_eq!(dyn_config.formats(), &[ImageFormat::Pnm]);
    assert!(dyn_config.capabilities().animation());
    assert!(dyn_config.capabilities().cheap_probe());

    let buf = make_rgb8_buffer(4, 2);
    let data = encode_single_frame(&buf);

    let mut job = dyn_config.dyn_job();

    // Exercise all set_ methods
    job.set_limits(ResourceLimits::none().with_max_pixels(1_000_000));
    job.set_crop_hint(0, 0, 4, 2);
    job.set_scale_hint(100, 100);
    job.set_orientation(OrientationHint::Correct);
    job.set_start_frame_index(0);
    job.set_policy(DecodePolicy::permissive());

    let info = job.probe(data.as_ref()).unwrap();
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);

    let info = job.probe_full(data.as_ref()).unwrap();
    assert_eq!(info.width, 4);

    let output_info = job.output_info(data.as_ref()).unwrap();
    assert_eq!(output_info.width, 4);

    // Decode via dyn dispatch
    let decoder = job.into_decoder(Cow::Borrowed(&data), &[]).unwrap();
    let result = decoder.decode().unwrap();
    assert_eq!(result.width(), 4);
    assert_eq!(result.height(), 2);
}

#[test]
fn dyn_decode_push_decode() {
    let buf = make_rgb8_buffer(4, 2);
    let data = encode_single_frame(&buf);

    let config = MockDecoderConfig;
    let dyn_config: &dyn DynDecoderConfig = &config;
    let job = dyn_config.dyn_job();

    struct TestSink {
        buf: Vec<u8>,
    }
    impl zc::decode::DecodeRowSink for TestSink {
        fn provide_next_buffer(
            &mut self,
            _y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<zenpixels::PixelSliceMut<'_>, zc::decode::SinkError> {
            let bpp = descriptor.bytes_per_pixel();
            let stride = width as usize * bpp;
            self.buf.resize(height as usize * stride, 0);
            Ok(
                zenpixels::PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                    .unwrap(),
            )
        }
    }

    let mut sink = TestSink { buf: Vec::new() };
    let info = job
        .push_decode(Cow::Borrowed(&data), &mut sink, &[])
        .unwrap();
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);
}

#[test]
fn dyn_decode_streaming() {
    let buf = make_rgb8_buffer(2, 3);
    let data = encode_single_frame(&buf);

    let config = MockDecoderConfig;
    let dyn_config: &dyn DynDecoderConfig = &config;
    let job = dyn_config.dyn_job();

    let mut stream = job
        .into_streaming_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    let info = stream.info();
    assert_eq!(info.width, 2);

    let mut count = 0;
    while let Some((_, strip)) = stream.next_batch().unwrap() {
        assert_eq!(strip.width(), 2);
        count += 1;
    }
    assert_eq!(count, 3);
}

#[test]
fn dyn_decode_full_frame() {
    let frame1 = make_rgb8_buffer(2, 2);
    let data = encode_animation(&[(frame1, 100)]);

    let config = MockDecoderConfig;
    let dyn_config: &dyn DynDecoderConfig = &config;
    let job = dyn_config.dyn_job();

    let mut dec = job
        .into_full_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    assert_eq!(dec.frame_count(), Some(1));
    assert_eq!(dec.loop_count(), Some(0));
    assert_eq!(dec.info().width, 2);

    let frame = dec.render_next_frame_owned(None).unwrap().unwrap();
    assert_eq!(frame.frame_index(), 0);
    assert_eq!(frame.duration_ms(), 100);

    assert!(dec.render_next_frame_owned(None).unwrap().is_none());
}

// =========================================================================
// 8. Dyn dispatch: DynEncodeJob set_ methods
// =========================================================================

#[test]
fn dyn_encode_job_set_methods() {
    let config = MockEncoderConfig::new().with_generic_quality(80.0);
    let dyn_config: &dyn DynEncoderConfig = &config;

    assert_eq!(dyn_config.format(), ImageFormat::Pnm);
    assert!(dyn_config.capabilities().animation());
    assert!(dyn_config.capabilities().row_level());
    assert!(dyn_config.capabilities().pull());

    let meta = Metadata::none();
    let view = meta.as_view();

    let mut job = dyn_config.dyn_job();
    job.set_limits(ResourceLimits::none());
    job.set_policy(EncodePolicy::permissive());
    job.set_metadata(&view);
    job.set_canvas_size(4, 4);
    job.set_loop_count(Some(0));

    // Into encoder via dyn
    let enc = job.into_encoder().unwrap();
    let buf = make_rgb8_buffer(4, 4);
    let output = enc.encode(buf.as_slice()).unwrap();
    assert!(!output.is_empty());
}

#[test]
fn dyn_encode_full_frame_encoder() {
    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;

    let mut job = dyn_config.dyn_job();
    job.set_loop_count(Some(0));

    let mut enc = job.into_full_frame_encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    enc.push_frame(buf.as_slice(), 100, None).unwrap();
    enc.push_frame(buf.as_slice(), 200, None).unwrap();

    let output = enc.finish(None).unwrap();
    assert!(!output.is_empty());
}

#[test]
fn dyn_encode_push_rows_and_finish() {
    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;

    let job = dyn_config.dyn_job();
    let mut enc = job.into_encoder().unwrap();
    assert_eq!(enc.preferred_strip_height(), 4);

    let buf = make_rgb8_buffer(4, 4);
    let ps = buf.as_slice();
    for y in 0..4u32 {
        let row_data = ps.row(y);
        let row_ps = PixelSlice::new(row_data, 4, 1, 4 * 3, PixelDescriptor::RGB8_SRGB).unwrap();
        enc.push_rows(row_ps).unwrap();
    }
    let output = enc.finish().unwrap();
    assert!(!output.is_empty());
}

#[test]
fn dyn_encode_encode_from() {
    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;

    let job = dyn_config.dyn_job();
    let enc = job.into_encoder().unwrap();

    let mut rows_pulled = 0u32;
    let output = enc
        .encode_from(&mut |y, mut dst| {
            if y >= 4 {
                return 0;
            }
            // Write 1 row
            dst.row_mut(0).fill(y as u8);
            rows_pulled += 1;
            1
        })
        .unwrap();

    assert_eq!(rows_pulled, 4);
    assert!(!output.is_empty());
}

#[test]
fn dyn_encode_srgba8() {
    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;

    let job = dyn_config.dyn_job();
    let enc = job.into_encoder().unwrap();

    let mut data = vec![128u8; 2 * 2 * 4];
    let output = enc.encode_srgba8(&mut data, false, 2, 2, 2).unwrap();
    assert!(!output.is_empty());
}

// =========================================================================
// 9. DecodeJob convenience dyn_ methods (on concrete job)
// =========================================================================

#[test]
fn concrete_job_dyn_decoder() {
    let buf = make_rgb8_buffer(2, 2);
    let data = encode_single_frame(&buf);

    let config = MockDecoderConfig;
    let job = config.job();
    let dec = job.dyn_decoder(Cow::Borrowed(&data), &[]).unwrap();
    let result = dec.decode().unwrap();
    assert_eq!(result.width(), 2);
}

#[test]
fn concrete_job_dyn_streaming_decoder() {
    let buf = make_rgb8_buffer(2, 2);
    let data = encode_single_frame(&buf);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job
        .dyn_streaming_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    let mut count = 0;
    while dec.next_batch().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 2);
}

#[test]
fn concrete_job_dyn_full_frame_decoder() {
    let frame = make_rgb8_buffer(2, 2);
    let data = encode_animation(&[(frame, 100)]);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job
        .dyn_full_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    let f = dec.render_next_frame_owned(None).unwrap().unwrap();
    assert_eq!(f.frame_index(), 0);
    assert!(dec.render_next_frame_owned(None).unwrap().is_none());
}

// =========================================================================
// 10. EncodeJob convenience dyn_ methods
// =========================================================================

#[test]
fn concrete_job_dyn_encoder() {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let enc = job.dyn_encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    let output = enc.encode(buf.as_slice()).unwrap();
    assert!(!output.is_empty());
}

#[test]
fn concrete_job_dyn_full_frame_encoder() {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let mut enc = job.dyn_full_frame_encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    enc.push_frame(buf.as_slice(), 100, None).unwrap();
    let output = enc.finish(None).unwrap();
    assert!(!output.is_empty());
}

// =========================================================================
// 11. Policy integration
// =========================================================================

#[test]
fn decode_policy_on_job() {
    let buf = make_rgb8_buffer(2, 2);
    let data = encode_single_frame(&buf);

    let config = MockDecoderConfig;
    let job = config.job().with_policy(DecodePolicy::strict());
    let dec = job.decoder(Cow::Borrowed(&data), &[]).unwrap();
    let result = dec.decode().unwrap();
    assert_eq!(result.width(), 2);
}

#[test]
fn encode_policy_on_job() {
    let config = MockEncoderConfig::new();
    let job = config.job().with_policy(EncodePolicy::strict());
    let enc = job.encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    let output = enc.encode(buf.as_slice()).unwrap();
    assert!(!output.is_empty());
}

// =========================================================================
// 12. Metadata roundtrip through MetadataView
// =========================================================================

#[test]
fn metadata_roundtrip_via_encode_job() {
    let icc = vec![1u8, 2, 3, 4];
    let exif = vec![5u8, 6, 7, 8];
    let xmp = b"<xmp>test</xmp>".to_vec();

    let meta = Metadata::none()
        .with_icc(icc.clone())
        .with_exif(exif.clone())
        .with_xmp(xmp.clone());

    let view = meta.as_view();
    assert_eq!(view.icc_profile, Some(icc.as_slice()));
    assert_eq!(view.exif, Some(exif.as_slice()));
    assert_eq!(view.xmp, Some(xmp.as_slice()));
    assert!(!view.is_empty());

    // Pass metadata through the encode job
    let config = MockEncoderConfig::new();
    let job = config.job().with_metadata(&view);
    let enc = job.encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    let _output = enc.encode(buf.as_slice()).unwrap();
    // Mock doesn't embed metadata, but we verified the path compiles and runs
}

#[test]
fn metadata_owned_from_view() {
    let icc = vec![10u8; 16];
    let meta = Metadata::none().with_icc(icc.clone());
    let view = meta.as_view();

    let owned = Metadata::from(view);
    assert_eq!(owned.as_view().icc_profile, Some(icc.as_slice()));
}

#[test]
fn metadata_from_image_info() {
    let info = ImageInfo::new(10, 10, ImageFormat::Jpeg)
        .with_icc_profile(vec![1, 2, 3])
        .with_exif(vec![4, 5, 6]);

    let view = info.metadata();
    assert_eq!(view.icc_profile, Some([1u8, 2, 3].as_slice()));
    assert_eq!(view.exif, Some([4u8, 5, 6].as_slice()));
    assert!(view.xmp.is_none());
}

// =========================================================================
// 13. DecodeOutput extras and accessors
// =========================================================================

#[test]
fn decode_output_extras_type_mismatch() {
    let buf = make_rgb8_buffer(2, 2);
    let info = ImageInfo::new(2, 2, ImageFormat::Png);
    let output = DecodeOutput::new(buf, info).with_extras(42u32);

    // Wrong type returns None
    assert!(output.extras::<String>().is_none());
    assert!(output.extras::<u64>().is_none());
    assert_eq!(output.extras::<u32>(), Some(&42));
}

#[test]
fn decode_output_take_extras_consumes() {
    let buf = make_rgb8_buffer(2, 2);
    let info = ImageInfo::new(2, 2, ImageFormat::Png);
    let mut output = DecodeOutput::new(buf, info).with_extras("hello".to_string());

    let taken = output.take_extras::<String>();
    assert_eq!(taken, Some("hello".to_string()));

    // Second take returns None
    assert!(output.take_extras::<String>().is_none());
}

#[test]
fn decode_output_into_buffer() {
    let buf = make_rgba8_buffer(3, 3);
    let info = ImageInfo::new(3, 3, ImageFormat::Avif);
    let output = DecodeOutput::new(buf, info);

    assert!(output.has_alpha());
    assert_eq!(output.descriptor(), PixelDescriptor::RGBA8_SRGB);
    assert_eq!(output.format(), ImageFormat::Avif);

    let recovered = output.into_buffer();
    assert_eq!(recovered.width(), 3);
    assert_eq!(recovered.height(), 3);
}

#[test]
fn decode_output_debug() {
    let buf = make_rgb8_buffer(2, 2);
    let info = ImageInfo::new(2, 2, ImageFormat::Jpeg);
    let output = DecodeOutput::new(buf, info);
    let s = format!("{:?}", output);
    assert!(s.contains("DecodeOutput"));
}

// =========================================================================
// 14. EncodeOutput accessors and AsRef
// =========================================================================

#[test]
fn encode_output_as_ref() {
    let output = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Png);
    let slice: &[u8] = output.as_ref();
    assert_eq!(slice, &[1, 2, 3]);
}

#[test]
fn encode_output_empty() {
    let output = EncodeOutput::new(vec![], ImageFormat::Png);
    assert!(output.is_empty());
    assert_eq!(output.len(), 0);
}

// =========================================================================
// 15. FullFrame with padded stride → to_owned strips padding
// =========================================================================

#[test]
fn full_frame_to_owned_strips_padding() {
    // Create a buffer with padded stride (pixel-aligned, larger than needed)
    let w = 10u32;
    let h = 2u32;
    let desc = PixelDescriptor::RGB8_SRGB;
    let bpp = 3;
    let row_bytes = w as usize * bpp;
    let stride = 66; // padded (must be multiple of bpp=3): 22 pixels * 3 = 66
    let total = (h as usize - 1) * stride + row_bytes;
    let mut data = vec![0u8; total];

    // Fill with known pattern
    for y in 0..h as usize {
        for x in 0..row_bytes {
            data[y * stride + x] = (y * 100 + x) as u8;
        }
    }

    let ps = PixelSlice::new(&data, w, h, stride, desc).unwrap();
    let frame = FullFrame::new(ps, 100, 0);
    let owned = frame.to_owned_frame();

    // Owned should have tight stride (no padding)
    let owned_ps = owned.pixels();
    assert_eq!(owned_ps.stride(), row_bytes);
    assert_eq!(owned_ps.width(), w);
    assert_eq!(owned_ps.rows(), h);

    // Verify pixel data survived
    for y in 0..h {
        for x in 0..row_bytes {
            assert_eq!(
                owned_ps.row(y)[x],
                (y as usize * 100 + x) as u8,
                "mismatch at y={y}, x={x}"
            );
        }
    }
}

// =========================================================================
// 16. ResourceLimits edge cases
// =========================================================================

#[test]
fn limits_exactly_at_boundary_pass() {
    let limits = ResourceLimits::none()
        .with_max_width(100)
        .with_max_height(100)
        .with_max_pixels(10_000);
    assert!(limits.check_dimensions(100, 100).is_ok());
}

#[test]
fn limits_one_over_boundary_fail() {
    let limits = ResourceLimits::none().with_max_width(100);
    assert!(limits.check_dimensions(101, 1).is_err());
    assert!(limits.check_dimensions(100, 1).is_ok());
}

#[test]
fn limits_zero_dimensions_pass() {
    let limits = ResourceLimits::none().with_max_pixels(0);
    // 0x0 = 0 pixels, which does NOT exceed 0
    assert!(limits.check_dimensions(0, 0).is_ok());
    // 1x1 = 1 pixel, which exceeds 0
    assert!(limits.check_dimensions(1, 1).is_err());
}

#[test]
fn limits_large_dimensions_overflow_safe() {
    // max u32 * max u32 should not overflow — it uses u64 multiplication
    let limits = ResourceLimits::none().with_max_pixels(u64::MAX);
    assert!(limits.check_dimensions(u32::MAX, u32::MAX).is_ok());
}

#[test]
fn limits_check_output_info() {
    let limits = ResourceLimits::none().with_max_width(50);
    let info = OutputInfo::full_decode(100, 50, PixelDescriptor::RGB8_SRGB);
    let err = limits.check_output_info(&info).unwrap_err();
    assert!(matches!(
        err,
        LimitExceeded::Width {
            actual: 100,
            max: 50
        }
    ));
}

#[test]
fn limit_exceeded_all_variants_display() {
    let cases: Vec<(LimitExceeded, &str)> = vec![
        (
            LimitExceeded::Width {
                actual: 5000,
                max: 4096,
            },
            "width 5000 exceeds limit 4096",
        ),
        (
            LimitExceeded::Height {
                actual: 3000,
                max: 2048,
            },
            "height 3000 exceeds limit 2048",
        ),
        (
            LimitExceeded::Pixels {
                actual: 100,
                max: 50,
            },
            "pixel count 100 exceeds limit 50",
        ),
        (
            LimitExceeded::Memory {
                actual: 200,
                max: 100,
            },
            "memory 200 bytes exceeds limit 100",
        ),
        (
            LimitExceeded::InputSize {
                actual: 200,
                max: 100,
            },
            "input size 200 bytes exceeds limit 100",
        ),
        (
            LimitExceeded::OutputSize {
                actual: 200,
                max: 100,
            },
            "output size 200 bytes exceeds limit 100",
        ),
        (
            LimitExceeded::Frames {
                actual: 200,
                max: 100,
            },
            "frame count 200 exceeds limit 100",
        ),
        (
            LimitExceeded::Duration {
                actual: 60000,
                max: 30000,
            },
            "duration 60000ms exceeds limit 30000ms",
        ),
    ];

    for (err, expected) in cases {
        assert_eq!(format!("{err}"), expected, "for {err:?}");
    }
}

#[test]
fn limit_exceeded_clone_eq() {
    let a = LimitExceeded::Pixels {
        actual: 100,
        max: 50,
    };
    let b = a.clone();
    assert_eq!(a, b);
}

// =========================================================================
// 17. ThreadingPolicy
// =========================================================================

#[test]
fn threading_policy_limit_or_any() {
    let limits = ResourceLimits::none().with_threading(ThreadingPolicy::LimitOrAny {
        preferred_max_threads: 8,
    });
    assert!(limits.has_any());
    assert_eq!(
        limits.threading(),
        ThreadingPolicy::LimitOrAny {
            preferred_max_threads: 8
        }
    );
}

// =========================================================================
// 18. Error chain walking through mock codec
// =========================================================================

#[test]
fn error_chain_limit_exceeded_through_mock() {
    let config = MockDecoderConfig;
    let job = config
        .job()
        .with_limits(ResourceLimits::none().with_max_width(2));

    let buf = make_rgb8_buffer(4, 4);
    let data = encode_single_frame(&buf);

    let err = job.decoder(Cow::Borrowed(&data), &[]).unwrap_err();
    let limit = err.limit_exceeded().unwrap();
    assert!(matches!(limit, LimitExceeded::Width { actual: 4, max: 2 }));
}

#[test]
fn error_chain_unsupported_through_dyn() {
    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;

    let job = dyn_config.dyn_job();
    let enc = job.into_full_frame_encoder().unwrap();

    // Finish without pushing frames
    let err = enc.finish(None).unwrap_err();
    // The error comes through BoxedError — verify it's inspectable
    assert!(err.to_string().contains("no frames"));
}

// =========================================================================
// 19. Cooperative cancellation
// =========================================================================

#[test]
fn decode_respects_cancellation() {
    use almost_enough::Stopper;

    let buf = make_rgb8_buffer(4, 4);
    let data = encode_single_frame(&buf);

    let stopper = Stopper::cancelled();

    let config = MockDecoderConfig;
    let job = config.job().with_stop(&stopper);
    let err = job.decoder(Cow::Borrowed(&data), &[]).unwrap_err();
    assert!(format!("{err}").contains("cancelled"));
}

#[test]
fn animation_decode_respects_per_frame_cancellation() {
    use almost_enough::Stopper;

    let frame1 = make_rgb8_buffer(2, 2);
    let frame2 = make_rgb8_buffer(2, 2);
    let data = encode_animation(&[(frame1, 100), (frame2, 200)]);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job.full_frame_decoder(Cow::Borrowed(&data), &[]).unwrap();

    // First frame: no cancel
    dec.render_next_frame(None).unwrap().unwrap();

    // Second frame: cancel
    let stopper = Stopper::cancelled();
    let err = dec.render_next_frame(Some(&stopper)).unwrap_err();
    assert!(format!("{err}").contains("cancelled"));
}

#[test]
fn animation_encode_respects_cancellation() {
    use almost_enough::Stopper;

    let config = MockEncoderConfig::new();
    let job = config.job();
    let mut enc = job.full_frame_encoder().unwrap();

    let buf = make_rgb8_buffer(2, 2);
    enc.push_frame(buf.as_slice(), 100, None).unwrap();

    let stopper = Stopper::cancelled();
    let err = enc.finish(Some(&stopper)).unwrap_err();
    assert!(format!("{err}").contains("cancelled"));
}

// =========================================================================
// 20. Orientation
// =========================================================================

#[test]
fn orientation_all_variants_exif_roundtrip() {
    for v in 1..=8u16 {
        let o = Orientation::from_exif(v);
        assert_eq!(o.exif_value(), v);
    }
}

#[test]
fn orientation_display_dimensions_comprehensive() {
    let cases = [
        (Orientation::Normal, 100, 200, (100, 200)),
        (Orientation::FlipHorizontal, 100, 200, (100, 200)),
        (Orientation::Rotate180, 100, 200, (100, 200)),
        (Orientation::FlipVertical, 100, 200, (100, 200)),
        (Orientation::Transpose, 100, 200, (200, 100)),
        (Orientation::Rotate90, 100, 200, (200, 100)),
        (Orientation::Transverse, 100, 200, (200, 100)),
        (Orientation::Rotate270, 100, 200, (200, 100)),
    ];
    for (orient, w, h, expected) in cases {
        assert_eq!(orient.display_dimensions(w, h), expected, "{orient:?}");
    }
}

#[test]
fn orientation_hint_all_variants_eq() {
    assert_eq!(OrientationHint::Preserve, OrientationHint::Preserve);
    assert_eq!(OrientationHint::Correct, OrientationHint::Correct);
    assert_ne!(OrientationHint::Preserve, OrientationHint::Correct);

    let ct = OrientationHint::CorrectAndTransform(Orientation::Rotate90);
    let ct2 = OrientationHint::CorrectAndTransform(Orientation::Rotate90);
    assert_eq!(ct, ct2);

    let et = OrientationHint::ExactTransform(Orientation::Rotate180);
    let et2 = OrientationHint::ExactTransform(Orientation::Rotate270);
    assert_ne!(et, et2);
}

// =========================================================================
// 21. GainMapMetadata
// =========================================================================

#[test]
fn gain_map_as_extras() {
    let meta = GainMapMetadata {
        gain_map_max: [3.0; 3],
        hdr_capacity_max: 3.0,
        ..GainMapMetadata::default()
    };

    let buf = make_rgb8_buffer(2, 2);
    let info = ImageInfo::new(2, 2, ImageFormat::Jpeg);
    let output = DecodeOutput::new(buf, info).with_extras(meta);

    let recovered = output.extras::<GainMapMetadata>().unwrap();
    assert_eq!(recovered.gain_map_max, [3.0; 3]);
    assert!(recovered.is_uniform());
}

// =========================================================================
// 22. ImageFormat
// =========================================================================

#[test]
fn image_format_detect_all_known() {
    use zc::ImageFormatRegistry;
    let reg = ImageFormatRegistry::common();

    // JPEG
    assert_eq!(reg.detect(&[0xFF, 0xD8, 0xFF]), Some(ImageFormat::Jpeg));

    // PNG
    assert_eq!(
        reg.detect(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
        Some(ImageFormat::Png)
    );

    // GIF87a
    assert_eq!(reg.detect(b"GIF87a\x00\x00"), Some(ImageFormat::Gif));

    // GIF89a
    assert_eq!(reg.detect(b"GIF89a\x00\x00"), Some(ImageFormat::Gif));

    // WebP
    assert_eq!(
        reg.detect(b"RIFF\x00\x00\x00\x00WEBP"),
        Some(ImageFormat::WebP)
    );

    // BMP
    assert_eq!(reg.detect(b"BM\x00\x00"), Some(ImageFormat::Bmp));

    // Unknown
    assert_eq!(reg.detect(&[0, 0, 0, 0]), None);

    // Empty
    assert_eq!(reg.detect(&[]), None);
}

#[test]
fn image_format_extension_mime() {
    assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
    assert_eq!(ImageFormat::Jpeg.mime_type(), "image/jpeg");

    assert_eq!(ImageFormat::Png.extension(), "png");
    assert_eq!(ImageFormat::Png.mime_type(), "image/png");

    assert_eq!(ImageFormat::Gif.extension(), "gif");
    assert_eq!(ImageFormat::Gif.mime_type(), "image/gif");

    assert_eq!(ImageFormat::WebP.extension(), "webp");
    assert_eq!(ImageFormat::WebP.mime_type(), "image/webp");
}

#[test]
fn image_format_display() {
    assert_eq!(format!("{}", ImageFormat::Jpeg), "JPEG");
    assert_eq!(format!("{}", ImageFormat::Png), "PNG");
    assert_eq!(format!("{}", ImageFormat::Avif), "AVIF");
}

#[test]
fn image_format_eq_hash_copy() {
    let a = ImageFormat::Jpeg;
    let b = a; // Copy
    assert_eq!(a, b);

    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(ImageFormat::Jpeg);
    set.insert(ImageFormat::Png);
    set.insert(ImageFormat::Jpeg);
    assert_eq!(set.len(), 2);
}

// =========================================================================
// 23. ImageInfo builders and accessors
// =========================================================================

#[test]
fn image_info_comprehensive_builder() {
    let info = ImageInfo::new(100, 200, ImageFormat::Avif)
        .with_frame_count(10)
        .with_animation(true)
        .with_orientation(Orientation::Rotate90)
        .with_alpha(true)
        .with_icc_profile(vec![1, 2, 3])
        .with_exif(vec![4, 5, 6])
        .with_xmp(vec![7, 8, 9]);

    assert_eq!(info.width, 100);
    assert_eq!(info.height, 200);
    assert_eq!(info.format, ImageFormat::Avif);
    assert_eq!(info.frame_count, Some(10));
    assert!(info.has_animation);
    assert_eq!(info.orientation, Orientation::Rotate90);
    assert!(info.has_alpha);
    assert_eq!(
        info.source_color.icc_profile.as_deref(),
        Some([1u8, 2, 3].as_slice())
    );
    assert_eq!(
        info.embedded_metadata.exif.as_deref(),
        Some([4u8, 5, 6].as_slice())
    );
    assert_eq!(
        info.embedded_metadata.xmp.as_deref(),
        Some([7u8, 8, 9].as_slice())
    );

    // Display dimensions accounts for orientation
    assert_eq!(info.display_width(), 200);
    assert_eq!(info.display_height(), 100);
}

// =========================================================================
// 24. Negotiation edge cases
// =========================================================================

#[test]
fn negotiate_same_format_different_metadata() {
    use zc::decode::negotiate_pixel_format;

    // Caller wants sRGB, decoder has unknown-transfer — format match wins
    let preferred = &[PixelDescriptor::RGBA8_SRGB];
    let available = &[PixelDescriptor::RGBA8]; // unknown transfer
    let picked = negotiate_pixel_format(preferred, available);
    // Returns the available entry (unknown transfer), not the preferred
    assert_eq!(picked, PixelDescriptor::RGBA8);
}

#[test]
fn negotiate_multiple_preferences_multiple_available() {
    use zc::decode::negotiate_pixel_format;

    // Caller prefers: RGBA8_SRGB, then GRAY8_SRGB
    // Available: GRAY8_SRGB, RGB8_SRGB
    // RGBA8 doesn't match anything, so GRAY8 matches second preference
    let preferred = &[PixelDescriptor::RGBA8_SRGB, PixelDescriptor::GRAY8_SRGB];
    let available = &[PixelDescriptor::GRAY8_SRGB, PixelDescriptor::RGB8_SRGB];
    let picked = negotiate_pixel_format(preferred, available);
    assert_eq!(picked, PixelDescriptor::GRAY8_SRGB);
}

#[test]
fn best_encode_format_no_match_returns_none() {
    use zc::encode::best_encode_format;
    let result = best_encode_format(PixelDescriptor::GRAY8_SRGB, &[PixelDescriptor::RGB8_SRGB]);
    assert_eq!(result, None);
}

// =========================================================================
// 25. Capabilities supports() cross-domain
// =========================================================================

#[test]
fn encode_caps_does_not_support_decode_ops() {
    let caps = EncodeCapabilities::new()
        .with_row_level(true)
        .with_pull(true)
        .with_animation(true);

    // Encode ops: supported
    assert!(caps.supports(UnsupportedOperation::RowLevelEncode));
    assert!(caps.supports(UnsupportedOperation::PullEncode));
    assert!(caps.supports(UnsupportedOperation::AnimationEncode));

    // Decode ops: never supported by encode caps
    assert!(!caps.supports(UnsupportedOperation::DecodeInto));
    assert!(!caps.supports(UnsupportedOperation::RowLevelDecode));
    assert!(!caps.supports(UnsupportedOperation::AnimationDecode));
    assert!(!caps.supports(UnsupportedOperation::PixelFormat));
}

#[test]
fn decode_caps_does_not_support_encode_ops() {
    let caps = DecodeCapabilities::new()
        .with_decode_into(true)
        .with_row_level(true)
        .with_animation(true);

    // Decode ops: supported
    assert!(caps.supports(UnsupportedOperation::DecodeInto));
    assert!(caps.supports(UnsupportedOperation::RowLevelDecode));
    assert!(caps.supports(UnsupportedOperation::AnimationDecode));

    // Encode ops: never supported by decode caps
    assert!(!caps.supports(UnsupportedOperation::RowLevelEncode));
    assert!(!caps.supports(UnsupportedOperation::PullEncode));
    assert!(!caps.supports(UnsupportedOperation::AnimationEncode));
    assert!(!caps.supports(UnsupportedOperation::PixelFormat));
}

// =========================================================================
// 26. Capabilities debug output
// =========================================================================

#[test]
fn capabilities_debug_has_all_fields() {
    let enc_caps = EncodeCapabilities::new()
        .with_icc(true)
        .with_animation(true)
        .with_effort_range(0, 10)
        .with_quality_range(0.0, 100.0);
    let s = format!("{enc_caps:?}");
    assert!(s.contains("icc: true"));
    assert!(s.contains("animation: true"));
    assert!(s.contains("effort_range"));
    assert!(s.contains("quality_range"));

    let dec_caps = DecodeCapabilities::new()
        .with_icc(true)
        .with_cheap_probe(true)
        .with_enforces_max_input_bytes(true);
    let s = format!("{dec_caps:?}");
    assert!(s.contains("icc: true"));
    assert!(s.contains("cheap_probe: true"));
    assert!(s.contains("enforces_max_input_bytes: true"));
}

// =========================================================================
// 27. Policy const construction and resolve
// =========================================================================

#[test]
fn decode_policy_all_resolvers() {
    let p = DecodePolicy::strict();
    assert!(!p.resolve_icc(true));
    assert!(!p.resolve_exif(true));
    assert!(!p.resolve_xmp(true));
    assert!(!p.resolve_progressive(true));
    assert!(!p.resolve_animation(true));
    assert!(!p.resolve_truncated(true));
    assert!(p.resolve_strict(false));
}

#[test]
fn encode_policy_all_resolvers() {
    let p = EncodePolicy::strict();
    assert!(!p.resolve_icc(true));
    assert!(!p.resolve_exif(true));
    assert!(!p.resolve_xmp(true));
    assert!(!p.resolve_animation(true));
    assert!(p.resolve_deterministic(false));
}

#[test]
fn decode_policy_permissive_all_resolvers() {
    let p = DecodePolicy::permissive();
    assert!(p.resolve_icc(false));
    assert!(p.resolve_exif(false));
    assert!(p.resolve_xmp(false));
    assert!(p.resolve_progressive(false));
    assert!(p.resolve_animation(false));
    assert!(p.resolve_truncated(false));
    assert!(!p.resolve_strict(true));
}

#[test]
fn encode_policy_permissive_all_resolvers() {
    let p = EncodePolicy::permissive();
    assert!(p.resolve_icc(false));
    assert!(p.resolve_exif(false));
    assert!(p.resolve_xmp(false));
    assert!(p.resolve_animation(false));
    assert!(!p.resolve_deterministic(true));
}

// =========================================================================
// 28. UnsupportedOperation
// =========================================================================

#[test]
fn unsupported_operation_all_names() {
    let ops = [
        (UnsupportedOperation::RowLevelEncode, "row_level_encode"),
        (UnsupportedOperation::PullEncode, "pull_encode"),
        (UnsupportedOperation::AnimationEncode, "animation_encode"),
        (UnsupportedOperation::DecodeInto, "decode_into"),
        (UnsupportedOperation::RowLevelDecode, "row_level_decode"),
        (UnsupportedOperation::AnimationDecode, "animation_decode"),
        (UnsupportedOperation::PixelFormat, "pixel_format"),
    ];
    for (op, name) in ops {
        assert_eq!(op.name(), name, "{op:?}");
        assert_eq!(
            format!("{op}"),
            format!("unsupported operation: {name}"),
            "{op:?}"
        );
    }
}

#[test]
fn unsupported_operation_error_has_no_source() {
    use std::error::Error;
    let op = UnsupportedOperation::PixelFormat;
    assert!(op.source().is_none());
}

// =========================================================================
// 29. ImageFormat from_extension and from_mime_type
// =========================================================================

#[test]
fn image_format_from_extension() {
    use zc::ImageFormatRegistry;
    let reg = ImageFormatRegistry::common();

    assert_eq!(reg.from_extension("jpg"), Some(ImageFormat::Jpeg));
    assert_eq!(reg.from_extension("jpeg"), Some(ImageFormat::Jpeg));
    assert_eq!(reg.from_extension("JPG"), Some(ImageFormat::Jpeg));
    assert_eq!(reg.from_extension("png"), Some(ImageFormat::Png));
    assert_eq!(reg.from_extension("gif"), Some(ImageFormat::Gif));
    assert_eq!(reg.from_extension("webp"), Some(ImageFormat::WebP));
    assert_eq!(reg.from_extension("avif"), Some(ImageFormat::Avif));
    assert_eq!(reg.from_extension("bmp"), Some(ImageFormat::Bmp));
    assert_eq!(reg.from_extension("jxl"), Some(ImageFormat::Jxl));
    assert_eq!(reg.from_extension("xyz"), None);
}

#[test]
fn image_format_from_mime_type() {
    use zc::ImageFormatRegistry;
    let reg = ImageFormatRegistry::common();

    assert_eq!(reg.from_mime_type("image/jpeg"), Some(ImageFormat::Jpeg));
    assert_eq!(reg.from_mime_type("image/png"), Some(ImageFormat::Png));
    assert_eq!(reg.from_mime_type("image/gif"), Some(ImageFormat::Gif));
    assert_eq!(reg.from_mime_type("image/webp"), Some(ImageFormat::WebP));
    assert_eq!(reg.from_mime_type("image/avif"), Some(ImageFormat::Avif));
    assert_eq!(reg.from_mime_type("text/plain"), None);
}

// =========================================================================
// 30. RGBA8 encode/decode roundtrip (exercises alpha path)
// =========================================================================

#[test]
fn rgba8_roundtrip() {
    let buf = make_rgba8_buffer(4, 4);
    let data = encode_single_frame(&buf);

    let config = MockDecoderConfig;
    let job = config.job();
    let dec = job.decoder(Cow::Borrowed(&data), &[]).unwrap();
    let result = dec.decode().unwrap();

    assert!(result.has_alpha());
    assert_eq!(result.descriptor(), PixelDescriptor::RGBA8_SRGB);
    assert_eq!(result.width(), 4);
    assert_eq!(result.height(), 4);

    // Verify pixel data roundtrips
    for y in 0..4 {
        assert_eq!(result.pixels().row(y), buf.as_slice().row(y));
    }
}

// =========================================================================
// 31. Animation with start_frame_index
// =========================================================================

#[test]
fn animation_start_frame_index() {
    let f0 = make_rgb8_buffer(2, 2);
    let f1 = make_rgb8_buffer(2, 2);
    let f2 = make_rgb8_buffer(2, 2);
    let data = encode_animation(&[(f0, 100), (f1, 200), (f2, 300)]);

    let config = MockDecoderConfig;
    let job = config.job().with_start_frame_index(1);
    let mut dec = job.full_frame_decoder(Cow::Borrowed(&data), &[]).unwrap();

    // Should start at frame 1
    let frame = dec.render_next_frame(None).unwrap().unwrap();
    assert_eq!(frame.frame_index(), 1);
    assert_eq!(frame.duration_ms(), 200);

    let frame = dec.render_next_frame(None).unwrap().unwrap();
    assert_eq!(frame.frame_index(), 2);
    assert_eq!(frame.duration_ms(), 300);

    assert!(dec.render_next_frame(None).unwrap().is_none());
}

// =========================================================================
// 32. Dyn full frame decoder render_next_frame_to_sink
// =========================================================================

#[test]
fn dyn_full_frame_decoder_render_to_sink() {
    let frame = make_rgb8_buffer(4, 2);
    let data = encode_animation(&[(frame, 100)]);

    let config = MockDecoderConfig;
    let dyn_config: &dyn DynDecoderConfig = &config;
    let job = dyn_config.dyn_job();

    let mut dec = job
        .into_full_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    struct SimpleSink {
        buf: Vec<u8>,
    }
    impl zc::decode::DecodeRowSink for SimpleSink {
        fn provide_next_buffer(
            &mut self,
            _y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<zenpixels::PixelSliceMut<'_>, zc::decode::SinkError> {
            let bpp = descriptor.bytes_per_pixel();
            let stride = width as usize * bpp;
            self.buf.resize(height as usize * stride, 0);
            Ok(
                zenpixels::PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                    .unwrap(),
            )
        }
    }

    let mut sink = SimpleSink { buf: Vec::new() };
    let info = dec
        .render_next_frame_to_sink(None, &mut sink)
        .unwrap()
        .unwrap();
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);

    assert!(
        dec.render_next_frame_to_sink(None, &mut sink)
            .unwrap()
            .is_none()
    );
}

// =========================================================================
// 33. Codec-agnostic function exercising the full pipeline
// =========================================================================

/// This function knows nothing about the concrete codec — pure dyn dispatch.
fn codec_agnostic_roundtrip(enc_config: &dyn DynEncoderConfig, dec_config: &dyn DynDecoderConfig) {
    // Encode
    let buf = make_rgb8_buffer(4, 2);
    let job = enc_config.dyn_job();
    let encoder = job.into_encoder().unwrap();
    let output = encoder.encode(buf.as_slice()).unwrap();

    // Decode
    let job = dec_config.dyn_job();
    let info = job.probe(output.data()).unwrap();
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 2);

    let decoder = job.into_decoder(Cow::Borrowed(output.data()), &[]).unwrap();
    let result = decoder.decode().unwrap();
    assert_eq!(result.width(), 4);
    assert_eq!(result.height(), 2);
}

#[test]
fn codec_agnostic_pipeline() {
    let enc = MockEncoderConfig::new();
    let dec = MockDecoderConfig;
    codec_agnostic_roundtrip(&enc, &dec);
}

// =========================================================================
// 34. OutputInfo
// =========================================================================

#[test]
fn output_info_full_decode() {
    let info = OutputInfo::full_decode(100, 200, PixelDescriptor::RGBA8_SRGB);
    assert_eq!(info.width, 100);
    assert_eq!(info.height, 200);
    assert_eq!(info.native_format, PixelDescriptor::RGBA8_SRGB);
}

// =========================================================================
// 35. DecodeCost and EncodeCost
// =========================================================================

#[test]
fn decode_cost_limits_check() {
    use zc::decode::DecodeCost;

    let cost = DecodeCost::new(100, 50, None);

    let limits = ResourceLimits::none()
        .with_max_pixels(100)
        .with_max_memory(200);
    assert!(limits.check_decode_cost(&cost).is_ok());

    // peak_memory=None falls back to output_bytes
    let limits = ResourceLimits::none().with_max_memory(50);
    let err = limits.check_decode_cost(&cost).unwrap_err();
    assert!(matches!(
        err,
        LimitExceeded::Memory {
            actual: 100,
            max: 50
        }
    ));
}

#[test]
fn encode_cost_limits_check() {
    use zc::encode::EncodeCost;

    let cost = EncodeCost::new(100, 50, Some(80));

    let limits = ResourceLimits::none()
        .with_max_pixels(100)
        .with_max_memory(100);
    assert!(limits.check_encode_cost(&cost).is_ok());

    let limits = ResourceLimits::none().with_max_memory(50);
    let err = limits.check_encode_cost(&cost).unwrap_err();
    assert!(matches!(
        err,
        LimitExceeded::Memory {
            actual: 80,
            max: 50
        }
    ));
}

// =========================================================================
// 36. Sink error propagation through push_decoder_via_full_decode
// =========================================================================

#[test]
fn push_decoder_sink_error_propagates() {
    let buf = make_rgb8_buffer(4, 2);
    let data = encode_single_frame(&buf);

    struct FailBeginSink;
    impl zc::decode::DecodeRowSink for FailBeginSink {
        fn begin(
            &mut self,
            _w: u32,
            _h: u32,
            _desc: PixelDescriptor,
        ) -> Result<(), zc::decode::SinkError> {
            Err("begin failed".into())
        }
        fn provide_next_buffer(
            &mut self,
            _y: u32,
            _h: u32,
            _w: u32,
            _d: PixelDescriptor,
        ) -> Result<zenpixels::PixelSliceMut<'_>, zc::decode::SinkError> {
            unreachable!()
        }
    }

    let config = MockDecoderConfig;
    let job = config.job();
    let err = job
        .push_decoder(Cow::Borrowed(&data), &mut FailBeginSink, &[])
        .unwrap_err();
    assert!(format!("{err}").contains("begin failed"));
}

#[test]
fn push_decoder_sink_finish_error_propagates() {
    let buf = make_rgb8_buffer(4, 2);
    let data = encode_single_frame(&buf);

    struct FailFinishSink {
        buf: Vec<u8>,
    }
    impl zc::decode::DecodeRowSink for FailFinishSink {
        fn provide_next_buffer(
            &mut self,
            _y: u32,
            height: u32,
            width: u32,
            descriptor: PixelDescriptor,
        ) -> Result<zenpixels::PixelSliceMut<'_>, zc::decode::SinkError> {
            let bpp = descriptor.bytes_per_pixel();
            let stride = width as usize * bpp;
            self.buf.resize(height as usize * stride, 0);
            Ok(
                zenpixels::PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor)
                    .unwrap(),
            )
        }
        fn finish(&mut self) -> Result<(), zc::decode::SinkError> {
            Err("finish failed".into())
        }
    }

    let config = MockDecoderConfig;
    let job = config.job();
    let err = job
        .push_decoder(
            Cow::Borrowed(&data),
            &mut FailFinishSink { buf: Vec::new() },
            &[],
        )
        .unwrap_err();
    assert!(format!("{err}").contains("finish failed"));
}

// =========================================================================
// 37. Decode with Cow::Owned (zero-copy ownership transfer)
// =========================================================================

#[test]
fn decode_with_cow_owned() {
    let buf = make_rgb8_buffer(2, 2);
    let data = encode_single_frame(&buf);

    let config = MockDecoderConfig;
    let job = config.job();
    // Pass owned data — exercises the Cow::Owned path
    let dec = job.decoder(Cow::Owned(data), &[]).unwrap();
    let result = dec.decode().unwrap();
    assert_eq!(result.width(), 2);
}

// =========================================================================
// 38. Multiple dyn jobs from same config (config is reusable)
// =========================================================================

#[test]
fn config_reusable_multiple_jobs() {
    let enc_config = MockEncoderConfig::new().with_generic_quality(90.0);
    let dec_config = MockDecoderConfig;

    // Encode two different images with the same config
    let buf1 = make_rgb8_buffer(2, 2);
    let buf2 = make_rgb8_buffer(4, 4);

    let data1 = {
        let job = enc_config.job();
        let enc = job.encoder().unwrap();
        enc.encode(buf1.as_slice()).unwrap().into_vec()
    };

    let data2 = {
        let job = enc_config.job();
        let enc = job.encoder().unwrap();
        enc.encode(buf2.as_slice()).unwrap().into_vec()
    };

    // Decode both with the same config
    {
        let job = dec_config.job();
        let dec = job.decoder(Cow::Borrowed(&data1), &[]).unwrap();
        assert_eq!(dec.decode().unwrap().width(), 2);
    }
    {
        let job = dec_config.job();
        let dec = job.decoder(Cow::Borrowed(&data2), &[]).unwrap();
        assert_eq!(dec.decode().unwrap().width(), 4);
    }
}

// =========================================================================
// 39. Dyn dispatch error inspection (CodecErrorExt on BoxedError)
// =========================================================================

#[test]
fn boxed_error_codec_error_ext() {
    let config = MockDecoderConfig;

    let buf = make_rgb8_buffer(2, 2);
    let data = encode_single_frame(&buf);

    // Use the dyn path — error comes out as BoxedError
    let dyn_config: &dyn DynDecoderConfig = &config;
    let mut dyn_job = dyn_config.dyn_job();
    dyn_job.set_limits(ResourceLimits::none().with_max_width(1));

    let err = dyn_job
        .into_decoder(Cow::Borrowed(&data), &[])
        .err()
        .expect("should fail with limit exceeded");

    // BoxedError implements CodecErrorExt
    let limit = err.limit_exceeded().unwrap();
    assert!(matches!(limit, LimitExceeded::Width { actual: 2, max: 1 }));
}
