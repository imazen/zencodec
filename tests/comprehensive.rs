#![allow(deprecated)] // Keep testing the published compatibility surface.
//! Comprehensive test suite exercising every aspect of zencodec.
//!
//! Uses a mock animation codec (mock_anim) that supports all encoder/decoder
//! modes including streaming decode, animation encode/decode, push_rows,
//! and encode_from. This exercises trait paths that the PNM codec can't reach.

mod mock_anim;

use std::borrow::Cow;
use zencodec::helpers::codec_spec;

use mock_anim::{MockDecoderConfig, MockEncoderConfig};

use zencodec::decode::{
    AnimationFrameDecoder, Decode, DecodeCapabilities, DecodeJob, DecodeOutput, DecodePolicy,
    DecoderConfig, DynDecoderConfig, OutputInfo, StreamingDecode,
};
use zencodec::encode::{
    AnimationFrameEncoder, DynEncoderConfig, EncodeCapabilities, EncodeJob, EncodeOutput,
    EncodePolicy, Encoder, EncoderConfig,
};
use zencodec::{
    AnimationFrame, CodecErrorExt, ImageFormat, ImageInfo, ImageSequence, LimitExceeded, Metadata,
    MetadataFields, MetadataPolicy, Orientation, OrientationHint, ResourceLimits, ThreadingPolicy,
    UnsupportedOperation,
};
use zenpixels::{PixelBuf, PixelFormat, PixelView};

// =========================================================================
// Test data helpers
// =========================================================================

fn make_rgb8_buffer(w: u32, h: u32) -> PixelBuf {
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
    PixelBuf::from_bytes(data, w, h, PixelFormat::RGB8_SRGB).unwrap()
}

fn make_rgba8_buffer(w: u32, h: u32) -> PixelBuf {
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
    PixelBuf::from_bytes(data, w, h, PixelFormat::RGBA8_SRGB).unwrap()
}

/// Encode a single frame via the mock codec, return raw mock bytes.
fn encode_single_frame(buf: &PixelBuf) -> Vec<u8> {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let encoder = job.encoder().unwrap();
    encoder.encode(buf.as_view()).unwrap().into_vec()
}

/// Encode multiple frames via the mock animation codec (borrowed buffers).
fn encode_animation_from_slices(frames: &[(&PixelBuf, u32)]) -> Vec<u8> {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let mut enc = job.animation_frame_encoder().unwrap();
    for (buf, dur) in frames {
        enc.push_frame(buf.as_view(), *dur, None).unwrap();
    }
    enc.finish(None).unwrap().into_vec()
}

/// Encode multiple frames via the mock animation codec.
fn encode_animation(frames: &[(PixelBuf, u32)]) -> Vec<u8> {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let mut enc = job.animation_frame_encoder().unwrap();
    for (buf, dur) in frames {
        enc.push_frame(buf.as_view(), *dur, None).unwrap();
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
    let ps = buf.as_view();

    // Push 2 rows at a time
    for strip in 0..4u32 {
        let y = strip * 2;
        // Collect 2 rows of data
        let mut strip_data = Vec::new();
        for r in 0..2u32 {
            strip_data.extend_from_slice(ps.row(y + r));
        }
        let strip_ps =
            PixelView::new(&strip_data, 4, 2, 4 * 3, &zenpixels::formats::RGB8_SRGB).unwrap();
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

    let ps = buf.as_view();
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
// 3. encode_in_place default implementation
// =========================================================================

#[test]
fn encoder_encode_in_place_opaque() {
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

    let view =
        zenpixels::PixelViewMut::new(&mut data, 4, 2, 4 * 4, &zenpixels::formats::RGBA8_SRGB)
            .unwrap();
    let output = enc.encode_in_place(view).unwrap();
    assert!(!output.is_empty());
    assert_eq!(output.format(), ImageFormat::Pnm);
}

#[test]
fn encoder_encode_in_place_with_alpha() {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let enc = job.encoder().unwrap();

    let mut data = vec![128u8; 2 * 2 * 4]; // 2x2 RGBA8
    let view =
        zenpixels::PixelViewMut::new(&mut data, 2, 2, 2 * 4, &zenpixels::formats::RGBA8_SRGB)
            .unwrap();
    let output = enc.encode_in_place(view).unwrap();
    assert!(!output.is_empty());
}

// =========================================================================
// 4. AnimationFrameEncoder for () stub
// =========================================================================

#[test]
fn unit_animation_frame_encoder_rejects() {
    let mut enc = ();
    let buf = make_rgb8_buffer(2, 2);
    let err = enc.push_frame(buf.as_view(), 100, None).unwrap_err();
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
    let mut dec = job
        .animation_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    assert_eq!(dec.frame_count(), Some(2));
    assert_eq!(dec.loop_count(), Some(0));

    // Frame 0
    let f0 = dec.render_next_frame(None).unwrap().unwrap();
    assert_eq!(f0.frame_index(), 0);
    assert_eq!(
        f0.duration(),
        zencodec::animation::FrameDuration::from_millis(100)
    );
    assert_eq!(f0.pixels().width(), 4);
    assert_eq!(f0.pixels().height(), 4);
    // Verify pixel data matches
    for y in 0..4 {
        assert_eq!(f0.pixels().row(y), frame1_check.as_view().row(y));
    }

    // Frame 1
    let f1 = dec.render_next_frame(None).unwrap().unwrap();
    assert_eq!(f1.frame_index(), 1);
    assert_eq!(
        f1.duration(),
        zencodec::animation::FrameDuration::from_millis(200)
    );

    // No more frames
    assert!(dec.render_next_frame(None).unwrap().is_none());
}

#[test]
fn animation_render_next_frame_owned() {
    let frame1 = make_rgb8_buffer(2, 2);
    let data = encode_animation(&[(frame1, 50)]);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job
        .animation_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    let owned = dec.render_next_frame_owned(None).unwrap().unwrap();
    assert_eq!(owned.frame_index(), 0);
    assert_eq!(
        owned.duration(),
        zencodec::animation::FrameDuration::from_millis(50)
    );
    assert_eq!(owned.pixels().width(), 2);

    // Can use OwnedAnimationFrame independently
    let borrowed = owned.as_animation_frame();
    assert_eq!(
        borrowed.duration(),
        zencodec::animation::FrameDuration::from_millis(50)
    );

    let recovered = owned.into_buffer();
    assert_eq!(recovered.width(), 2);
}

#[test]
fn animation_render_to_sink() {
    let frame1 = make_rgb8_buffer(4, 2);
    let data = encode_animation(&[(frame1, 100)]);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job
        .animation_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    struct CollectSink {
        buf: Vec<u8>,
        began: bool,
        finished: bool,
    }

    impl zencodec::decode::DecodeRowSink for CollectSink {
        fn begin(
            &mut self,
            _w: u32,
            _h: u32,
            _desc: PixelFormat,
        ) -> Result<(), zencodec::decode::SinkError> {
            self.began = true;
            Ok(())
        }
        fn provide_next_buffer(
            &mut self,
            strip: zencodec::decode::RowStrip,
            descriptor: PixelFormat,
        ) -> Result<zenpixels::PixelViewMut<'_>, zencodec::decode::SinkError> {
            let (height, width) = (strip.height, strip.width);
            let bpp = descriptor.layout.bytes_per_pixel();
            let stride = width as usize * bpp;
            self.buf.resize(height as usize * stride, 0);
            Ok(zenpixels::PixelViewMut::new_with_format_cow(
                &mut self.buf,
                width,
                height,
                stride,
                zenpixels::FormatCow::from_owned(descriptor),
            )
            .unwrap())
        }
        fn finish(&mut self) -> Result<(), zencodec::decode::SinkError> {
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
        assert_eq!(strip.height(), 1);
        assert_eq!(strip.width(), 4);
        // Verify pixel data matches
        assert_eq!(strip.row(0), buf.as_view().row(y));
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
    job.set_crop_hint(zencodec::Rect::new(0, 0, 4, 2));
    job.set_orientation(OrientationHint::Correct);
    job.set_start_frame_index(0);
    job.try_set_policy(DecodePolicy::permissive()).unwrap();

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
    impl zencodec::decode::DecodeRowSink for TestSink {
        fn provide_next_buffer(
            &mut self,
            strip: zencodec::decode::RowStrip,
            descriptor: PixelFormat,
        ) -> Result<zenpixels::PixelViewMut<'_>, zencodec::decode::SinkError> {
            let (height, width) = (strip.height, strip.width);
            let bpp = descriptor.layout.bytes_per_pixel();
            let stride = width as usize * bpp;
            self.buf.resize(height as usize * stride, 0);
            Ok(zenpixels::PixelViewMut::new_with_format_cow(
                &mut self.buf,
                width,
                height,
                stride,
                zenpixels::FormatCow::from_owned(descriptor),
            )
            .unwrap())
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
fn dyn_decode_animation_frame() {
    let frame1 = make_rgb8_buffer(2, 2);
    let data = encode_animation(&[(frame1, 100)]);

    let config = MockDecoderConfig;
    let dyn_config: &dyn DynDecoderConfig = &config;
    let job = dyn_config.dyn_job();

    let mut dec = job
        .into_animation_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    assert_eq!(dec.frame_count(), Some(1));
    assert_eq!(dec.loop_count(), Some(0));
    assert_eq!(dec.info().width, 2);

    let frame = dec.render_next_frame_owned(None).unwrap().unwrap();
    assert_eq!(frame.frame_index(), 0);
    assert_eq!(
        frame.duration(),
        zencodec::animation::FrameDuration::from_millis(100)
    );

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
    assert!(dyn_config.capabilities().push_rows());
    assert!(dyn_config.capabilities().encode_from());

    let meta = Metadata::none();

    let mut job = dyn_config.dyn_job();
    job.set_limits(ResourceLimits::none());
    job.try_set_policy(EncodePolicy::preserve_all()).unwrap();
    job.try_set_metadata_policy(meta, MetadataPolicy::PreserveExact)
        .unwrap();
    job.set_canvas_size(4, 4);
    job.set_loop_count(Some(0));

    // Into encoder via dyn
    let enc = job.into_encoder().unwrap();
    let buf = make_rgb8_buffer(4, 4);
    let output = enc.encode(buf.as_view()).unwrap();
    assert!(!output.is_empty());
}

#[test]
fn dyn_encode_animation_frame_encoder() {
    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;

    let mut job = dyn_config.dyn_job();
    job.set_loop_count(Some(0));

    let mut enc = job.into_animation_frame_encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    enc.push_frame(buf.as_view(), 100, None).unwrap();
    enc.push_frame(buf.as_view(), 200, None).unwrap();

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
    let ps = buf.as_view();
    for y in 0..4u32 {
        let row_data = ps.row(y);
        let row_ps = PixelView::new(row_data, 4, 1, 4 * 3, &zenpixels::formats::RGB8_SRGB).unwrap();
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
fn dyn_encode_in_place() {
    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;

    let job = dyn_config.dyn_job();
    let enc = job.into_encoder().unwrap();

    let mut data = vec![128u8; 2 * 2 * 4];
    let view =
        zenpixels::PixelViewMut::new(&mut data, 2, 2, 2 * 4, &zenpixels::formats::RGBA8_SRGB)
            .unwrap();
    let output = enc.encode_in_place(view).unwrap();
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
fn concrete_job_dyn_animation_frame_decoder() {
    let frame = make_rgb8_buffer(2, 2);
    let data = encode_animation(&[(frame, 100)]);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job
        .dyn_animation_frame_decoder(Cow::Borrowed(&data), &[])
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
    let output = enc.encode(buf.as_view()).unwrap();
    assert!(!output.is_empty());
}

#[test]
fn concrete_job_dyn_animation_frame_encoder() {
    let config = MockEncoderConfig::new();
    let job = config.job();
    let mut enc = job.dyn_animation_frame_encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    enc.push_frame(buf.as_view(), 100, None).unwrap();
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
    let job = config
        .job()
        .try_with_policy(DecodePolicy::untrusted_minimal())
        .unwrap();
    let dec = job.decoder(Cow::Borrowed(&data), &[]).unwrap();
    let result = dec.decode().unwrap();
    assert_eq!(result.width(), 2);
}

#[test]
fn encode_policy_on_job() {
    let config = MockEncoderConfig::new();
    let job = config
        .job()
        .try_with_policy(EncodePolicy::strip_all())
        .unwrap();
    let enc = job.encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    let output = enc.encode(buf.as_view()).unwrap();
    assert!(!output.is_empty());
}

// =========================================================================
// 12. Metadata roundtrip through Metadata
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

    assert_eq!(meta.icc_profile.as_deref(), Some(icc.as_slice()));
    assert_eq!(meta.exif.as_deref(), Some(exif.as_slice()));
    assert_eq!(meta.xmp.as_deref(), Some(xmp.as_slice()));
    assert!(!meta.is_empty());

    // Pass metadata through the encode job
    let config = MockEncoderConfig::new();
    let job = config
        .job()
        .try_with_metadata_policy(meta, MetadataPolicy::PreserveExact)
        .unwrap();
    let enc = job.encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    let _output = enc.encode(buf.as_view()).unwrap();
    // Mock doesn't embed metadata, but we verified the path compiles and runs
}

/// `with_unfiltered_metadata` / `set_unfiltered_metadata` embed raw metadata
/// without running a policy filter.
#[test]
fn unfiltered_metadata_works() {
    let meta = Metadata::none().with_exif(vec![1, 2, 3, 4]);

    let config = MockEncoderConfig::new();
    let job = config.clone().job().with_unfiltered_metadata(meta.clone());
    let enc = job.encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    assert!(!enc.encode(buf.as_view()).unwrap().is_empty());

    let dyn_config: &dyn DynEncoderConfig = &config;
    let mut job = dyn_config.dyn_job();
    job.set_canvas_size(2, 2);
    job.set_unfiltered_metadata(meta);
    let enc = job.into_encoder().unwrap();
    assert!(!enc.encode(buf.as_view()).unwrap().is_empty());
}

/// `try_with_metadata_policy` filters the record *before* the codec sees it: a
/// discard policy must yield empty metadata, `PreserveExact` must pass bytes
/// through untouched.
#[test]
fn metadata_policy_filters_before_codec() {
    let meta = Metadata::none()
        .with_icc(vec![1u8; 8])
        .with_exif(vec![2u8; 8])
        .with_xmp(b"<x/>".to_vec());

    // Discard-all policy => nothing survives.
    let stripped = meta.filtered(&MetadataPolicy::Custom(MetadataFields::DISCARD_ALL));
    assert!(stripped.is_empty(), "discard policy must strip everything");

    // PreserveExact => byte-identical passthrough.
    let kept = meta.filtered(&MetadataPolicy::PreserveExact);
    assert_eq!(kept.icc_profile.as_deref(), Some([1u8; 8].as_slice()));
    assert_eq!(kept.exif.as_deref(), Some([2u8; 8].as_slice()));
    assert_eq!(kept.xmp.as_deref(), Some(b"<x/>".as_slice()));

    // The job builder accepts the policy and runs end to end.
    let job = MockEncoderConfig::new()
        .job()
        .try_with_metadata_policy(meta, MetadataPolicy::Web)
        .unwrap();
    let enc = job.encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    assert!(!enc.encode(buf.as_view()).unwrap().is_empty());
}

#[test]
fn metadata_clone_roundtrip() {
    let icc = vec![10u8; 16];
    let meta = Metadata::none().with_icc(icc.clone());

    let cloned = meta.clone();
    assert_eq!(cloned.icc_profile.as_deref(), Some(icc.as_slice()));
}

#[test]
fn metadata_from_image_info() {
    let info = ImageInfo::new(10, 10, ImageFormat::Jpeg)
        .with_icc_profile(vec![1, 2, 3])
        .with_exif(vec![4, 5, 6]);

    let meta = info.metadata();
    assert_eq!(meta.icc_profile.as_deref(), Some([1u8, 2, 3].as_slice()));
    assert_eq!(meta.exif.as_deref(), Some([4u8, 5, 6].as_slice()));
    assert!(meta.xmp.is_none());
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
    assert_eq!(output.pixel_format(), &PixelFormat::RGBA8_SRGB);
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
// 15. AnimationFrame with padded stride → to_owned strips padding
// =========================================================================

#[test]
fn animation_frame_to_owned_strips_padding() {
    // Create a buffer with padded stride (pixel-aligned, larger than needed)
    let w = 10u32;
    let h = 2u32;
    let desc = PixelFormat::RGB8_SRGB;
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

    let ps =
        PixelView::new_with_format_cow(&data, w, h, stride, zenpixels::FormatCow::from_owned(desc))
            .unwrap();
    let frame = AnimationFrame::new(ps, 100, 0);
    let owned = frame.try_to_owned_frame().unwrap();

    // Owned should have tight stride (no padding)
    let owned_ps = owned.pixels();
    assert_eq!(owned_ps.stride_bytes(), row_bytes);
    assert_eq!(owned_ps.width(), w);
    assert_eq!(owned_ps.height(), h);

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
    let info = OutputInfo::full_decode(100, 50, PixelFormat::RGB8_SRGB);
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
fn threading_policy_sequential_parallel() {
    let seq = ResourceLimits::none().with_threading(ThreadingPolicy::Sequential);
    assert!(seq.has_any());
    assert!(!seq.threading().is_parallel());

    let par = ResourceLimits::none().with_threading(ThreadingPolicy::Parallel);
    assert!(!par.has_any());
    assert!(par.threading().is_parallel());
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
    let enc = job.into_animation_frame_encoder().unwrap();

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
    let job = config.job().with_pulse(zencodec::SharedPulse::new(
        zencodec::how_far::StopOnly::new(stopper),
    ));
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
    let mut dec = job
        .animation_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    // First frame: no cancel
    dec.render_next_frame(None).unwrap().unwrap();

    // Second frame: cancel
    let stopper = Stopper::cancelled();
    let err = dec
        .render_next_frame(Some(&zencodec::how_far::StopOnly::borrowed(&stopper)))
        .unwrap_err();
    assert!(format!("{err}").contains("cancelled"));
}

#[test]
fn animation_encode_respects_cancellation() {
    use almost_enough::Stopper;

    let config = MockEncoderConfig::new();
    let job = config.job();
    let mut enc = job.animation_frame_encoder().unwrap();

    let buf = make_rgb8_buffer(2, 2);
    enc.push_frame(buf.as_view(), 100, None).unwrap();

    let stopper = Stopper::cancelled();
    let err = enc
        .finish(Some(&zencodec::how_far::StopOnly::borrowed(&stopper)))
        .unwrap_err();
    assert!(format!("{err}").contains("cancelled"));
}

// =========================================================================
// 20. Orientation
// =========================================================================

#[test]
fn orientation_all_variants_exif_roundtrip() {
    for v in 1..=8u8 {
        let o = Orientation::from_exif(v).unwrap();
        assert_eq!(o.to_exif(), v);
    }
}

#[test]
fn orientation_display_dimensions_comprehensive() {
    let cases = [
        (Orientation::Identity, 100, 200, (100, 200)),
        (Orientation::FlipH, 100, 200, (100, 200)),
        (Orientation::Rotate180, 100, 200, (100, 200)),
        (Orientation::FlipV, 100, 200, (100, 200)),
        (Orientation::Transpose, 100, 200, (200, 100)),
        (Orientation::Rotate90, 100, 200, (200, 100)),
        (Orientation::Transverse, 100, 200, (200, 100)),
        (Orientation::Rotate270, 100, 200, (200, 100)),
    ];
    for (orient, w, h, expected) in cases {
        assert_eq!(orient.output_dimensions(w, h), expected, "{orient:?}");
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
// 21. ImageFormat
// =========================================================================

#[test]
fn image_format_detect_all_known() {
    use zencodec::ImageFormatRegistry;
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
        .with_sequence(ImageSequence::Animation {
            frame_count: Some(10),
            loop_count: None,
            random_access: false,
        })
        .with_orientation(Orientation::Rotate90)
        .with_alpha(true)
        .with_icc_profile(vec![1, 2, 3])
        .with_exif(vec![4, 5, 6])
        .with_xmp(vec![7, 8, 9]);

    assert_eq!(info.width, 100);
    assert_eq!(info.height, 200);
    assert_eq!(info.format, ImageFormat::Avif);
    assert_eq!(info.frame_count(), Some(10));
    assert!(info.is_animation());
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
    use zencodec::decode::negotiate_pixel_format;

    // Caller wants sRGB, decoder has unknown-transfer — format match wins
    let preferred = &[PixelFormat::RGBA8_SRGB];
    let available = &[codec_spec(zenpixels::SampleLayout::RGBA8)]; // unknown transfer
    let picked = negotiate_pixel_format(preferred, available);
    // Returns the available entry (unknown transfer), not the preferred
    assert_eq!(picked, Some(codec_spec(zenpixels::SampleLayout::RGBA8)));
}

#[test]
fn negotiate_multiple_preferences_multiple_available() {
    use zencodec::decode::negotiate_pixel_format;

    // Caller prefers: RGBA8_SRGB, then GRAY8_SRGB
    // Available: GRAY8_SRGB, RGB8_SRGB
    // RGBA8 doesn't match anything, so GRAY8 matches second preference
    let preferred = &[PixelFormat::RGBA8_SRGB, PixelFormat::GRAY8_SRGB];
    let available = &[PixelFormat::GRAY8_SRGB, PixelFormat::RGB8_SRGB];
    let picked = negotiate_pixel_format(preferred, available);
    assert_eq!(picked, Some(PixelFormat::GRAY8_SRGB));
}

#[test]
fn best_encode_format_no_match_returns_none() {
    use zencodec::encode::best_encode_format;
    let result = best_encode_format(PixelFormat::GRAY8_SRGB, &[PixelFormat::RGB8_SRGB]);
    assert_eq!(result, None);
}

// =========================================================================
// 25. Capabilities supports() cross-domain
// =========================================================================

#[test]
fn encode_caps_does_not_support_decode_ops() {
    let caps = EncodeCapabilities::new()
        .with_push_rows(true)
        .with_encode_from(true)
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
        .with_streaming(true)
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
    let p = DecodePolicy::untrusted_minimal();
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
    let p = EncodePolicy::strip_all();
    assert!(!p.resolve_icc(true));
    assert!(!p.resolve_exif(true));
    assert!(!p.resolve_xmp(true));
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
fn encode_policy_preserve_all_resolvers() {
    let p = EncodePolicy::preserve_all();
    assert!(p.resolve_icc(false));
    assert!(p.resolve_exif(false));
    assert!(p.resolve_xmp(false));
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
    use zencodec::ImageFormatRegistry;
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
    use zencodec::ImageFormatRegistry;
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
    assert_eq!(result.pixel_format(), &PixelFormat::RGBA8_SRGB);
    assert_eq!(result.width(), 4);
    assert_eq!(result.height(), 4);

    // Verify pixel data roundtrips
    for y in 0..4 {
        assert_eq!(result.pixels().row(y), buf.as_view().row(y));
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
    let mut dec = job
        .animation_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    // Should start at frame 1
    let frame = dec.render_next_frame(None).unwrap().unwrap();
    assert_eq!(frame.frame_index(), 1);
    assert_eq!(
        frame.duration(),
        zencodec::animation::FrameDuration::from_millis(200)
    );

    let frame = dec.render_next_frame(None).unwrap().unwrap();
    assert_eq!(frame.frame_index(), 2);
    assert_eq!(
        frame.duration(),
        zencodec::animation::FrameDuration::from_millis(300)
    );

    assert!(dec.render_next_frame(None).unwrap().is_none());
}

// =========================================================================
// 32. Dyn full frame decoder render_next_frame_to_sink
// =========================================================================

#[test]
fn dyn_animation_frame_decoder_render_to_sink() {
    let frame = make_rgb8_buffer(4, 2);
    let data = encode_animation(&[(frame, 100)]);

    let config = MockDecoderConfig;
    let dyn_config: &dyn DynDecoderConfig = &config;
    let job = dyn_config.dyn_job();

    let mut dec = job
        .into_animation_frame_decoder(Cow::Borrowed(&data), &[])
        .unwrap();

    struct SimpleSink {
        buf: Vec<u8>,
    }
    impl zencodec::decode::DecodeRowSink for SimpleSink {
        fn provide_next_buffer(
            &mut self,
            strip: zencodec::decode::RowStrip,
            descriptor: PixelFormat,
        ) -> Result<zenpixels::PixelViewMut<'_>, zencodec::decode::SinkError> {
            let (height, width) = (strip.height, strip.width);
            let bpp = descriptor.layout.bytes_per_pixel();
            let stride = width as usize * bpp;
            self.buf.resize(height as usize * stride, 0);
            Ok(zenpixels::PixelViewMut::new_with_format_cow(
                &mut self.buf,
                width,
                height,
                stride,
                zenpixels::FormatCow::from_owned(descriptor),
            )
            .unwrap())
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
    let output = encoder.encode(buf.as_view()).unwrap();

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
    let info = OutputInfo::full_decode(100, 200, PixelFormat::RGBA8_SRGB);
    assert_eq!(info.width, 100);
    assert_eq!(info.height, 200);
    assert_eq!(info.native_format, PixelFormat::RGBA8_SRGB);
}

// =========================================================================
// 35. Sink error propagation through push_decoder
// =========================================================================

#[test]
fn push_decoder_sink_error_propagates() {
    let buf = make_rgb8_buffer(4, 2);
    let data = encode_single_frame(&buf);

    struct FailBeginSink;
    impl zencodec::decode::DecodeRowSink for FailBeginSink {
        fn begin(
            &mut self,
            _w: u32,
            _h: u32,
            _desc: PixelFormat,
        ) -> Result<(), zencodec::decode::SinkError> {
            Err("begin failed".into())
        }
        fn provide_next_buffer(
            &mut self,
            _strip: zencodec::decode::RowStrip,
            _d: PixelFormat,
        ) -> Result<zenpixels::PixelViewMut<'_>, zencodec::decode::SinkError> {
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
    impl zencodec::decode::DecodeRowSink for FailFinishSink {
        fn provide_next_buffer(
            &mut self,
            strip: zencodec::decode::RowStrip,
            descriptor: PixelFormat,
        ) -> Result<zenpixels::PixelViewMut<'_>, zencodec::decode::SinkError> {
            let (height, width) = (strip.height, strip.width);
            let bpp = descriptor.layout.bytes_per_pixel();
            let stride = width as usize * bpp;
            self.buf.resize(height as usize * stride, 0);
            Ok(zenpixels::PixelViewMut::new_with_format_cow(
                &mut self.buf,
                width,
                height,
                stride,
                zenpixels::FormatCow::from_owned(descriptor),
            )
            .unwrap())
        }
        fn finish(&mut self) -> Result<(), zencodec::decode::SinkError> {
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
        let job = enc_config.clone().job();
        let enc = job.encoder().unwrap();
        enc.encode(buf1.as_view()).unwrap().into_vec()
    };

    let data2 = {
        let job = enc_config.clone().job();
        let enc = job.encoder().unwrap();
        enc.encode(buf2.as_view()).unwrap().into_vec()
    };

    // Decode both with the same config
    {
        let job = dec_config.clone().job();
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
        .expect_err("should fail with limit exceeded");

    // BoxedError implements CodecErrorExt
    let limit = err.limit_exceeded().unwrap();
    assert!(matches!(limit, LimitExceeded::Width { actual: 2, max: 1 }));
}

// =========================================================================
// 40. Downcast from dyn shims to concrete codec types
// =========================================================================

#[test]
fn downcast_dyn_encoder_config_to_concrete() {
    let config = MockEncoderConfig::new().with_generic_quality(85.0);
    let dyn_config: &dyn DynEncoderConfig = &config;

    // Downcast back to the concrete config type
    let concrete = dyn_config
        .as_any()
        .downcast_ref::<MockEncoderConfig>()
        .expect("should downcast to MockEncoderConfig");
    assert_eq!(concrete.generic_quality(), Some(85.0));
}

#[test]
fn downcast_dyn_decoder_config_to_concrete() {
    let config = MockDecoderConfig;
    let dyn_config: &dyn DynDecoderConfig = &config;

    // Downcast back to the concrete config type
    let concrete = dyn_config
        .as_any()
        .downcast_ref::<MockDecoderConfig>()
        .expect("should downcast to MockDecoderConfig");
    // MockDecoderConfig is a unit struct — just verify the downcast succeeded
    let _ = concrete;
}

#[test]
fn downcast_dyn_encoder_config_wrong_type_returns_none() {
    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;

    // Trying to downcast to the wrong type returns None
    assert!(
        dyn_config
            .as_any()
            .downcast_ref::<MockDecoderConfig>()
            .is_none()
    );
}

#[test]
fn downcast_dyn_animation_frame_encoder_as_any() {
    use mock_anim::MockAnimationFrameEnc;

    let config = MockEncoderConfig::new();
    let job = config.job().with_loop_count(Some(0));
    let mut enc = job.dyn_animation_frame_encoder().unwrap();

    // as_any: verify the concrete type
    assert!(
        enc.as_any()
            .downcast_ref::<MockAnimationFrameEnc>()
            .is_some()
    );
    assert!(
        enc.as_any_mut()
            .downcast_mut::<MockAnimationFrameEnc>()
            .is_some()
    );
}

#[test]
fn downcast_dyn_animation_frame_encoder_into_any() {
    use mock_anim::MockAnimationFrameEnc;

    let config = MockEncoderConfig::new();
    let job = config.job().with_loop_count(Some(0));
    let enc = job.dyn_animation_frame_encoder().unwrap();

    // into_any: consume and recover
    let any_box = enc.into_any();
    let mut concrete: Box<MockAnimationFrameEnc> = any_box
        .downcast()
        .expect("should downcast to MockAnimationFrameEnc");

    // Use the concrete encoder: push a frame and finish
    let buf = make_rgb8_buffer(2, 2);
    concrete.push_frame(buf.as_view(), 100, None).unwrap();
    let output = concrete.finish(None).unwrap();
    assert!(!output.data().is_empty());
}

#[test]
fn downcast_dyn_animation_frame_decoder_as_any() {
    use mock_anim::MockAnimationFrameDec;

    let buf = make_rgb8_buffer(2, 2);
    let data = encode_animation_from_slices(&[(&buf, 100), (&buf, 200)]);

    let config = MockDecoderConfig;
    let job = config.job();
    let mut dec = job
        .dyn_animation_frame_decoder(Cow::Owned(data), &[])
        .unwrap();

    // as_any: verify the concrete type
    assert!(
        dec.as_any()
            .downcast_ref::<MockAnimationFrameDec>()
            .is_some()
    );
    assert!(
        dec.as_any_mut()
            .downcast_mut::<MockAnimationFrameDec>()
            .is_some()
    );

    // Still usable through dyn after as_any inspection
    let frame = dec.render_next_frame_owned(None).unwrap().unwrap();
    assert_eq!(
        frame.duration(),
        zencodec::animation::FrameDuration::from_millis(100)
    );
}

#[test]
fn downcast_dyn_animation_frame_decoder_into_any() {
    use mock_anim::MockAnimationFrameDec;

    let buf = make_rgb8_buffer(2, 2);
    let data = encode_animation_from_slices(&[(&buf, 100)]);

    let config = MockDecoderConfig;
    let job = config.job();
    let dec = job
        .dyn_animation_frame_decoder(Cow::Owned(data), &[])
        .unwrap();

    // into_any: consume and recover
    let any_box = dec.into_any();
    let mut concrete: Box<MockAnimationFrameDec> = any_box
        .downcast()
        .expect("should downcast to MockAnimationFrameDec");

    // Use the concrete decoder directly
    assert_eq!(concrete.frame_count(), Some(1));
    let frame = concrete.render_next_frame(None).unwrap().unwrap();
    assert_eq!(
        frame.duration(),
        zencodec::animation::FrameDuration::from_millis(100)
    );
}

#[test]
fn downcast_dyn_animation_frame_decoder_via_dyn_job() {
    use mock_anim::MockAnimationFrameDec;

    // Full dyn dispatch path: DynDecoderConfig → DynDecodeJob → DynAnimationFrameDecoder → concrete
    let buf = make_rgb8_buffer(2, 2);
    let data = encode_animation_from_slices(&[(&buf, 50)]);

    let config = MockDecoderConfig;
    let dyn_config: &dyn DynDecoderConfig = &config;
    let job = dyn_config.dyn_job();
    let dec = job
        .into_animation_frame_decoder(Cow::Owned(data), &[])
        .unwrap();

    assert!(
        dec.as_any()
            .downcast_ref::<MockAnimationFrameDec>()
            .is_some()
    );
    assert_eq!(dec.frame_count(), Some(1));
}

// =========================================================================
// Extras on output types
// =========================================================================

#[test]
fn encode_output_extras_roundtrip() {
    #[derive(Debug, PartialEq)]
    struct JpegStats {
        actual_quality: f32,
    }

    let mut output = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Jpeg).with_extras(JpegStats {
        actual_quality: 92.5,
    });

    assert_eq!(
        output.extras::<JpegStats>(),
        Some(&JpegStats {
            actual_quality: 92.5
        })
    );
    assert!(output.extras::<u32>().is_none());

    let taken = output.take_extras::<JpegStats>();
    assert_eq!(
        taken,
        Some(JpegStats {
            actual_quality: 92.5
        })
    );
    assert!(output.extras::<JpegStats>().is_none());
}

#[test]
fn encode_output_clone_preserves_extras() {
    let output = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Jpeg).with_extras(42u32);
    assert_eq!(output.extras::<u32>(), Some(&42));

    let cloned = output.clone();
    // Extras are preserved via Arc (cheap ref-count bump)
    assert_eq!(cloned.extras::<u32>(), Some(&42));
    // Data fields are preserved
    assert_eq!(cloned.data(), &[1, 2, 3]);
    assert_eq!(cloned.format(), ImageFormat::Jpeg);
}

#[test]
fn encode_output_eq_ignores_extras() {
    let a = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Jpeg).with_extras(42u32);
    let b = EncodeOutput::new(vec![1, 2, 3], ImageFormat::Jpeg);
    assert_eq!(a, b);
}

#[test]
fn owned_animation_frame_extras_roundtrip() {
    #[derive(Debug, PartialEq)]
    struct FrameMeta {
        is_keyframe: bool,
    }

    let buf = make_rgb8_buffer(2, 2);
    let mut frame = zencodec::OwnedAnimationFrame::new(buf, 100, 0)
        .with_extras(FrameMeta { is_keyframe: true });

    assert_eq!(
        frame.extras::<FrameMeta>(),
        Some(&FrameMeta { is_keyframe: true })
    );
    assert!(frame.extras::<u32>().is_none());

    let taken = frame.take_extras::<FrameMeta>();
    assert_eq!(taken, Some(FrameMeta { is_keyframe: true }));
    assert!(frame.extras::<FrameMeta>().is_none());
}

// =========================================================================
// Job extensions (codec-specific config via dyn dispatch)
// =========================================================================

#[test]
fn encode_job_extensions_concrete() {
    use mock_anim::MockEncodeExtensions;

    let config = MockEncoderConfig::new();
    let mut job = config.job();

    // Extensions accessible on concrete job
    let ext = job.extensions_mut().unwrap();
    let mock_ext = ext.downcast_mut::<MockEncodeExtensions>().unwrap();
    mock_ext.optimize = true;
    mock_ext.custom_tag = Some("test".into());

    // Read back
    let ext = job.extensions().unwrap();
    let mock_ext = ext.downcast_ref::<MockEncodeExtensions>().unwrap();
    assert!(mock_ext.optimize);
    assert_eq!(mock_ext.custom_tag.as_deref(), Some("test"));
}

#[test]
fn encode_job_extensions_through_dyn_job() {
    use mock_anim::MockEncodeExtensions;

    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;
    let mut job = dyn_config.dyn_job();

    // Generic setup
    job.set_limits(ResourceLimits::none());

    // Codec-specific setup via extensions
    let ext = job.extensions_mut().unwrap();
    let mock_ext = ext.downcast_mut::<MockEncodeExtensions>().unwrap();
    mock_ext.optimize = true;

    // Read back through dyn
    let ext = job.extensions().unwrap();
    let mock_ext = ext.downcast_ref::<MockEncodeExtensions>().unwrap();
    assert!(mock_ext.optimize);

    // Still usable for encoding after extensions access
    let enc = job.into_encoder().unwrap();
    let buf = make_rgb8_buffer(2, 2);
    let output = enc.encode(buf.as_view()).unwrap();
    assert!(!output.is_empty());
}

#[test]
fn decode_job_extensions_through_dyn_job() {
    use mock_anim::MockDecodeExtensions;

    let config = MockDecoderConfig;
    let dyn_config: &dyn zencodec::decode::DynDecoderConfig = &config;
    let mut job = dyn_config.dyn_job();

    // Codec-specific setup via extensions
    let ext = job.extensions_mut().unwrap();
    let mock_ext = ext.downcast_mut::<MockDecodeExtensions>().unwrap();
    mock_ext.strict_parsing = true;

    // Read back
    let ext = job.extensions().unwrap();
    let mock_ext = ext.downcast_ref::<MockDecodeExtensions>().unwrap();
    assert!(mock_ext.strict_parsing);

    // Still usable for decoding
    let buf = make_rgb8_buffer(2, 2);
    let data = encode_single_frame(&buf);
    let dec = job.into_decoder(Cow::Owned(data), &[]).unwrap();
    let output = dec.decode().unwrap();
    assert_eq!(output.width(), 2);
}

#[test]
fn extensions_wrong_type_returns_none() {
    let config = MockEncoderConfig::new();
    let dyn_config: &dyn DynEncoderConfig = &config;
    let job = dyn_config.dyn_job();

    // Extensions exist but wrong downcast type returns None
    let ext = job.extensions().unwrap();
    assert!(ext.downcast_ref::<u32>().is_none());
}

#[test]
fn exact_animation_default_rejects_and_dyn_shim_does_not_fall_back_to_milliseconds() {
    use zencodec::animation::FrameDuration;
    let buf = make_rgb8_buffer(2, 2);
    for duration in [
        FrameDuration::new(1001, 30000).unwrap(),
        FrameDuration::from_millis(100),
    ] {
        let mut enc = MockEncoderConfig::new()
            .job()
            .animation_frame_encoder()
            .unwrap();
        let err = enc
            .push_frame_exact(buf.as_view(), duration, None)
            .unwrap_err();
        assert!(matches!(
            err,
            mock_anim::MockError::Unsupported(UnsupportedOperation::AnimationTiming)
        ));
        assert!(enc.finish(None).is_err(), "reject must not accept a frame");
        let mut enc = MockEncoderConfig::new()
            .job()
            .dyn_animation_frame_encoder()
            .unwrap();
        let err = enc
            .push_frame_exact(buf.as_view(), duration, None)
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<mock_anim::MockError>(),
            Some(mock_anim::MockError::Unsupported(
                UnsupportedOperation::AnimationTiming
            ))
        ));
        assert!(
            enc.finish(None).is_err(),
            "dyn shim must not call legacy push_frame"
        );
    }
}

#[test]
fn still_push_reports_the_actual_crop_and_transform() {
    let spec = PixelFormat::RGB8_SRGB;
    let bytes = [1u8, 2, 3, 4, 5, 6]
        .into_iter()
        .flat_map(|id| [id, 0, 0])
        .collect();
    let raster = PixelBuf::from_bytes(bytes, 3, 2, spec.clone()).unwrap();
    let encoded = encode_single_frame(&raster);
    struct Sink {
        data: [u8; 24],
        geometry: Option<(u32, u32, PixelFormat)>,
        done: bool,
    }
    impl zencodec::decode::DecodeRowSink for Sink {
        fn begin(
            &mut self,
            w: u32,
            h: u32,
            spec: PixelFormat,
        ) -> Result<(), zencodec::decode::SinkError> {
            assert!(self.geometry.is_none());
            self.geometry = Some((w, h, spec));
            Ok(())
        }
        fn provide_next_buffer(
            &mut self,
            strip: zencodec::decode::RowStrip,
            spec: PixelFormat,
        ) -> Result<zenpixels::PixelViewMut<'_>, zencodec::decode::SinkError> {
            assert_eq!(strip.y, 0);
            assert_eq!(
                self.geometry,
                Some((strip.width, strip.height, spec.clone()))
            );
            Ok(zenpixels::PixelViewMut::new_with_format_cow(
                &mut self.data,
                strip.width,
                strip.height,
                8,
                zenpixels::FormatCow::from_owned(spec),
            )?)
        }
        fn finish(&mut self) -> Result<(), zencodec::decode::SinkError> {
            self.done = true;
            Ok(())
        }
    }
    for (orientation, crop, size, expected) in [
        (Orientation::Rotate90, None, (2, 3), vec![4, 1, 5, 2, 6, 3]),
        (
            Orientation::FlipH,
            Some([1, 0, 2, 2]),
            (2, 2),
            vec![3, 2, 6, 5],
        ),
    ] {
        for erased in [false, true] {
            let mut sink = Sink {
                data: [0xA7; 24],
                geometry: None,
                done: false,
            };
            let hint = OrientationHint::ExactTransform(orientation);
            let completion = if erased {
                let cfg: &dyn DynDecoderConfig = &MockDecoderConfig;
                let mut job = cfg.dyn_job();
                job.set_orientation(hint);
                if let Some([x, y, w, h]) = crop {
                    job.set_crop_hint(zencodec::Rect::new(x, y, w, h));
                }
                job.push_decode(Cow::Borrowed(&encoded), &mut sink, &[])
                    .unwrap()
            } else {
                let mut job = MockDecoderConfig.job().with_orientation(hint);
                if let Some([x, y, w, h]) = crop {
                    job = job.with_crop_hint(zencodec::Rect::new(x, y, w, h));
                }
                job.push_decoder(Cow::Borrowed(&encoded), &mut sink, &[])
                    .unwrap()
            };
            assert!(sink.done);
            assert_eq!(sink.geometry, Some((size.0, size.1, spec.clone())));
            assert_eq!((completion.width, completion.height), size);
            assert_eq!(completion.native_format, spec);
            assert_eq!(completion.orientation_applied, orientation);
            assert_eq!(completion.crop_applied, crop);
            for (i, expected_row) in expected.as_chunks::<2>().0.iter().enumerate() {
                let row = &sink.data[i * 8..][..8];
                assert_eq!(
                    row,
                    &[expected_row[0], 0, 0, expected_row[1], 0, 0, 0xA7, 0xA7]
                );
            }
        }
    }
}

#[test]
fn rational_animation_completion_and_erased_borrow_preserve_source_frame() {
    use zencodec::{
        animation::FrameDuration,
        decode::{DecodeRowSink, FrameCompletion, SinkError},
        time::{TimeBase, Timestamp},
    };
    struct FrameSink {
        bytes: [u8; 8],
        calls: Vec<&'static str>,
    }
    impl DecodeRowSink for FrameSink {
        fn begin(&mut self, w: u32, h: u32, spec: PixelFormat) -> Result<(), SinkError> {
            assert_eq!((w, h, spec), (2, 1, PixelFormat::RGBA8_SRGB));
            self.calls.push("begin");
            Ok(())
        }
        fn provide_next_buffer(
            &mut self,
            strip: zencodec::decode::RowStrip,
            spec: PixelFormat,
        ) -> Result<zenpixels::PixelViewMut<'_>, SinkError> {
            assert_eq!((strip.y, strip.height, strip.width), (0, 1, 2));
            self.calls.push("rows");
            Ok(zenpixels::PixelViewMut::new_with_format_cow(
                &mut self.bytes,
                strip.width,
                strip.height,
                8,
                zenpixels::FormatCow::from_owned(spec),
            )?)
        }
        fn finish(&mut self) -> Result<(), SinkError> {
            self.calls.push("finish");
            Ok(())
        }
    }
    let spec = PixelFormat::RGBA8_SRGB;
    let expected: [[u8; 8]; 2] = [[101, 12, 43, 0, 93, 255, 7, 17], [6, 5, 4, 3, 2, 1, 0, 255]];
    let buffers =
        expected.map(|bytes| PixelBuf::from_bytes(bytes.to_vec(), 2, 1, spec.clone()).unwrap());
    let input =
        encode_animation_from_slices(&[(&buffers[0], 0xFFFF_FFF0), (&buffers[1], 0xFFFF_FFF1)]);
    let cfg: &dyn DynDecoderConfig = &MockDecoderConfig;
    let mut typed = MockDecoderConfig
        .job()
        .animation_frame_decoder(Cow::Borrowed(&input), &[])
        .unwrap();
    let mut erased = cfg
        .dyn_job()
        .into_animation_frame_decoder(Cow::Borrowed(&input), &[])
        .unwrap();
    let mut borrowed = cfg
        .dyn_job()
        .into_animation_frame_decoder(Cow::Borrowed(&input), &[])
        .unwrap();
    let mut owned = cfg
        .dyn_job()
        .into_animation_frame_decoder(Cow::Borrowed(&input), &[])
        .unwrap();
    let mut sink = FrameSink {
        bytes: [0; 8],
        calls: Vec::new(),
    };
    for (index, ((n, d, tick), bytes)) in [(1, 24, 17), (7, 120, 22)]
        .into_iter()
        .zip(expected)
        .enumerate()
    {
        let facts = FrameCompletion {
            duration: FrameDuration::new(n, d).unwrap(),
            frame_index: index as u32,
            pts: Some(Timestamp::new(tick, TimeBase::new(1, 120).unwrap())),
        };
        let native_pointer = borrowed
            .as_any()
            .downcast_ref::<mock_anim::MockAnimationFrameDec>()
            .unwrap()
            .pixel_address(index);
        let frame = borrowed.render_next_frame(None).unwrap().unwrap();
        assert_eq!(frame.pixels().row(0).as_ptr(), native_pointer);
        assert_eq!(frame.pixels().row(0), bytes);
        assert_eq!(
            (frame.duration(), frame.frame_index(), frame.pts()),
            (facts.duration, facts.frame_index, facts.pts)
        );
        let frame = owned.render_next_frame_owned(None).unwrap().unwrap();
        assert_eq!(frame.pixels().row(0), bytes);
        assert_eq!(
            (frame.duration(), frame.frame_index(), frame.pts()),
            (facts.duration, facts.frame_index, facts.pts)
        );
        for completion in [
            typed
                .render_next_frame_to_sink(None, &mut sink)
                .unwrap()
                .unwrap(),
            erased
                .render_next_frame_to_sink(None, &mut sink)
                .unwrap()
                .unwrap(),
        ] {
            assert_eq!(completion.frame, Some(facts));
            assert_eq!(
                (
                    completion.width,
                    completion.height,
                    completion.native_format
                ),
                (2, 1, spec.clone())
            );
        }
        assert_eq!(
            sink.calls,
            ["begin", "rows", "finish", "begin", "rows", "finish"]
        );
        assert_eq!(sink.bytes, bytes);
        sink.calls.clear();
    }
    assert!(
        typed
            .render_next_frame_to_sink(None, &mut sink)
            .unwrap()
            .is_none()
    );
    assert!(
        erased
            .render_next_frame_to_sink(None, &mut sink)
            .unwrap()
            .is_none()
    );
    assert!(borrowed.render_next_frame(None).unwrap().is_none());
    assert!(owned.render_next_frame_owned(None).unwrap().is_none());
    assert!(sink.calls.is_empty());
}
