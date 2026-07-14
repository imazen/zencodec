//! Behavior tests for `zencodec::CodecSet`, driven by the reference codec.
//!
//! The reference codec registers under `ImageFormat::Pnm` but uses its own
//! `ZCR1` wire format, so magic-byte *detection* tests either use real PNM
//! magic (detection only — the bytes aren't decodable) or the `Zcr` wrapper
//! below, which re-registers the reference decoder under a custom
//! `ImageFormatDefinition` whose `detect` matches the actual wire format —
//! exercising the full detect → decode path end to end.

use std::sync::LazyLock;

use zencodec::prelude::*;
use zencodec::{
    CodecSet, CodecSetError, ImageFormat, ImageFormatDefinition, Metadata, MetadataPolicy,
    ResourceLimits,
};
use zencodec_testkit::{RefError, ReferenceDecoderConfig, ReferenceEncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};

// ===========================================================================
// A custom-format registration of the reference decoder
// ===========================================================================

fn detect_zcr(data: &[u8]) -> bool {
    data.len() >= 4 && &data[..4] == b"ZCR1"
}

static ZCR_FORMAT: ImageFormatDefinition = ImageFormatDefinition::new(
    "zcr-test",
    None,
    "ZCR (testkit reference wire format)",
    "zcr",
    &["zcr"],
    "image/x-zcr-test",
    &["image/x-zcr-test"],
    true,  // alpha
    true,  // animation
    true,  // lossless
    false, // lossy
    4,
    detect_zcr,
);

static ZCR_FORMATS: &[ImageFormat] = &[ImageFormat::Custom(&ZCR_FORMAT)];

/// The reference decoder re-registered under the custom `zcr-test` format.
#[derive(Clone, Debug, Default)]
struct ZcrDecoderConfig;

impl DecoderConfig for ZcrDecoderConfig {
    type Error = RefError;
    type Job<'a> = <ReferenceDecoderConfig as DecoderConfig>::Job<'a>;

    fn formats() -> &'static [ImageFormat] {
        ZCR_FORMATS
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        <ReferenceDecoderConfig as DecoderConfig>::supported_descriptors()
    }
    fn job<'a>(self) -> Self::Job<'a> {
        ReferenceDecoderConfig.job()
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

fn rgb_pixels(bytes: &[u8], width: u32, height: u32) -> PixelSlice<'_> {
    PixelSlice::new(
        bytes,
        width,
        height,
        width as usize * 3,
        PixelDescriptor::RGB8_SRGB,
    )
    .expect("valid pixel slice")
}

const W: u32 = 2;
const H: u32 = 2;
const PIXELS: [u8; 12] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];

fn reference_set() -> CodecSet {
    CodecSet::new()
        .with_decoder(ReferenceDecoderConfig)
        .with_encoder(ReferenceEncoderConfig::new())
}

// ===========================================================================
// Tests
// ===========================================================================

#[test]
fn encode_then_decode_as_roundtrips_pixels() {
    let set = reference_set();

    let encoded = set
        .encode(ImageFormat::Pnm, rgb_pixels(&PIXELS, W, H))
        .expect("encode");
    assert_eq!(encoded.format(), ImageFormat::Pnm);

    let decoded = set
        .decode_as(ImageFormat::Pnm, encoded.data(), &[])
        .expect("decode_as");
    assert_eq!(decoded.width(), W);
    assert_eq!(decoded.height(), H);
    let out = decoded.pixels();
    let mut round = Vec::new();
    for y in 0..out.rows() {
        round.extend_from_slice(out.row(y));
    }
    assert_eq!(round, PIXELS);
}

#[test]
fn detect_consults_only_registered_formats() {
    let set = reference_set();

    // Real PNM magic → detected (reference registers under Pnm).
    assert_eq!(set.detect(b"P6\n2 2\n255\n"), Some(ImageFormat::Pnm));
    // JPEG magic is recognizable in general, but no JPEG decoder is registered.
    assert_eq!(set.detect(&[0xFF, 0xD8, 0xFF, 0xE0]), None);

    assert!(set.can_decode(ImageFormat::Pnm));
    assert!(set.can_encode(ImageFormat::Pnm));
    assert!(!set.can_decode(ImageFormat::Jpeg));
    assert!(set.decoder_for(ImageFormat::Pnm).is_some());
    assert!(set.encoder_for(ImageFormat::Pnm).is_some());
}

#[test]
fn custom_format_detect_and_decode_end_to_end() {
    // Encoder emits ZCR1 bytes; the Zcr wrapper's custom format detects them.
    let set = CodecSet::new()
        .with_decoder(ZcrDecoderConfig)
        .with_encoder(ReferenceEncoderConfig::new());

    let encoded = set
        .encode(ImageFormat::Pnm, rgb_pixels(&PIXELS, W, H))
        .expect("encode");

    let detected = set.detect(encoded.data()).expect("detected");
    assert_eq!(detected, ImageFormat::Custom(&ZCR_FORMAT));

    // Full detect → decode path, no format named by the caller.
    let decoded = set.decode(encoded.data()).expect("decode");
    assert_eq!((decoded.width(), decoded.height()), (W, H));

    // probe() goes through the same detection.
    let info = set.probe(encoded.data()).expect("probe");
    assert_eq!((info.width, info.height), (W, H));
}

#[test]
fn shared_static_set_decodes_from_many_threads() {
    static CODECS: LazyLock<CodecSet> = LazyLock::new(|| {
        CodecSet::new()
            .with_decoder(ZcrDecoderConfig)
            .with_encoder(ReferenceEncoderConfig::new())
            .with_limits(ResourceLimits::default())
    });

    let encoded = CODECS
        .encode(ImageFormat::Pnm, rgb_pixels(&PIXELS, W, H))
        .expect("encode");
    let data = encoded.data().to_vec();

    std::thread::scope(|scope| {
        for _ in 0..4 {
            let data = &data;
            scope.spawn(move || {
                let decoded = CODECS.decode(data).expect("decode on worker thread");
                assert_eq!((decoded.width(), decoded.height()), (W, H));
            });
        }
    });
}

#[test]
fn encode_with_fidelity_clones_the_template() {
    use zencodec::encode::Fidelity;

    let set = reference_set();

    // Per-call lossless override; the reference codec records it.
    let out = set
        .encode_with(
            ImageFormat::Pnm,
            Fidelity::Lossless,
            rgb_pixels(&PIXELS, W, H),
        )
        .expect("encode_with");
    let decoded = set
        .decode_as(ImageFormat::Pnm, out.data(), &[])
        .expect("decode");
    assert_eq!((decoded.width(), decoded.height()), (W, H));

    // The registered template is untouched — plain encode still works.
    set.encode(ImageFormat::Pnm, rgb_pixels(&PIXELS, W, H))
        .expect("template encode after encode_with");
}

#[test]
fn encode_job_carries_metadata() {
    let set = reference_set();

    let meta = Metadata::none().with_icc(vec![1, 2, 3, 4]);
    let mut job = set.encode_job(ImageFormat::Pnm).expect("encode_job");
    job.set_metadata_policy(meta, MetadataPolicy::PreserveExact);
    let encoder = job.into_encoder().expect("encoder");
    let out = encoder.encode(rgb_pixels(&PIXELS, W, H)).expect("encode");

    let decoded = set
        .decode_as(ImageFormat::Pnm, out.data(), &[])
        .expect("decode");
    assert_eq!(
        decoded.info().source_color.icc_profile.as_deref(),
        Some(&[1u8, 2, 3, 4][..])
    );
}

#[test]
fn animation_roundtrip_through_set() {
    let set = CodecSet::new()
        .with_decoder(ZcrDecoderConfig)
        .with_encoder(ReferenceEncoderConfig::new());

    let frame_a = [255u8; 12];
    let frame_b = [0u8; 12];

    let mut job = set.encode_job(ImageFormat::Pnm).expect("encode_job");
    job.set_loop_count(Some(0));
    let mut enc = job
        .into_animation_frame_encoder()
        .expect("animation encoder");
    enc.push_frame(rgb_pixels(&frame_a, W, H), 100, None)
        .expect("frame a");
    enc.push_frame(rgb_pixels(&frame_b, W, H), 200, None)
        .expect("frame b");
    let out = enc.finish(None).expect("finish");

    let mut dec = set
        .animation_decoder(out.data(), &[])
        .expect("animation decoder");
    assert_eq!(dec.frame_count(), Some(2));
    let first = dec
        .render_next_frame_owned(None)
        .expect("render")
        .expect("frame 0");
    assert_eq!(first.duration_ms(), 100);
    let second = dec
        .render_next_frame_owned(None)
        .expect("render")
        .expect("frame 1");
    assert_eq!(second.duration_ms(), 200);
    assert!(dec.render_next_frame_owned(None).expect("render").is_none());
}

#[test]
fn streaming_decoder_borrowing_the_set() {
    let set = CodecSet::new()
        .with_decoder(ZcrDecoderConfig)
        .with_encoder(ReferenceEncoderConfig::new());

    let encoded = set
        .encode(ImageFormat::Pnm, rgb_pixels(&PIXELS, W, H))
        .expect("encode");

    let mut dec = set
        .streaming_decoder(encoded.data(), &[])
        .expect("streaming decoder");
    let mut rows_seen = 0;
    while let Some((y, strip)) = dec.next_batch().expect("batch") {
        assert_eq!(y, rows_seen);
        rows_seen += strip.rows();
    }
    assert_eq!(rows_seen, H);
}

#[test]
fn missing_codecs_report_typed_errors() {
    let set = CodecSet::new().with_decoder(ReferenceDecoderConfig);

    match set.encode(ImageFormat::Pnm, rgb_pixels(&PIXELS, W, H)) {
        Err(CodecSetError::NoEncoder(ImageFormat::Pnm)) => {}
        other => panic!("expected NoEncoder(Pnm), got {other:?}"),
    }
    match set.decode_as(ImageFormat::Jpeg, b"\xFF\xD8\xFF", &[]) {
        Err(CodecSetError::NoDecoder(ImageFormat::Jpeg)) => {}
        other => panic!("expected NoDecoder(Jpeg), got {other:?}"),
    }

    // A codec failure passes through with the codec's error in the chain.
    match set.decode_as(ImageFormat::Pnm, b"not zcr bytes at all", &[]) {
        Err(CodecSetError::Codec(e)) => {
            let msg = format!("{e}");
            assert!(msg.contains("invalid"), "unexpected codec error: {msg}");
        }
        other => panic!("expected Codec error, got {other:?}"),
    }
}

#[test]
fn cloned_set_is_independent() {
    let base = CodecSet::new().with_decoder(ReferenceDecoderConfig);
    let extended = base.clone().with_encoder(ReferenceEncoderConfig::new());

    assert!(!base.can_encode(ImageFormat::Pnm));
    assert!(extended.can_encode(ImageFormat::Pnm));
    assert!(extended.can_decode(ImageFormat::Pnm));
}

#[test]
fn one_shot_trait_methods_work_directly() {
    // The provided DecoderConfig::decode / probe and EncoderConfig::encode
    // one-shots — no CodecSet, no job plumbing.
    let out = ReferenceEncoderConfig::new()
        .encode(rgb_pixels(&PIXELS, W, H))
        .expect("one-shot encode");

    let info = ReferenceDecoderConfig
        .probe(out.data())
        .expect("one-shot probe");
    assert_eq!((info.width, info.height), (W, H));

    let decoded = ReferenceDecoderConfig
        .decode(out.data())
        .expect("one-shot decode");
    assert_eq!((decoded.width(), decoded.height()), (W, H));
}
