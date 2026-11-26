//! Worked, runnable examples of the common ways to drive a [`zencodec::CodecSet`].
//!
//! Each `#[test]` is a self-contained example — read them top to bottom as a
//! tutorial. The reference codec (accepts and produces RGB8 + RGBA8) stands in
//! for a real codec.
//!
//! The last two examples show the extra reach a caller gets from
//! `zenpixels-convert`: adapting a foreign pixel format into an encoder's
//! supported set, and converting a decoded buffer into a caller-chosen format.

use std::sync::LazyLock;

use zencodec::encode::Fidelity;
use zencodec::estimate::ComputeEnvironment;
use zencodec::{CodecSet, ColorEmitPolicy, ImageFormat, Metadata, MetadataPolicy};
use zencodec_testkit::{ReferenceEncoderConfig, ReferenceZcrDecoderConfig};
use zenpixels::{PixelFormat, PixelView};

// The reference codec stands in for a real one. Its encoder emits a "ZCR1" wire
// format; `ReferenceZcrDecoderConfig` registers the decoder under a format whose
// `detect` matches those bytes, so the detection-based APIs (`decode`, `probe`,
// `estimate_decode_of`) work end to end below with no custom-format boilerplate.

const W: u32 = 2;
const H: u32 = 2;
const RGB: [u8; 12] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];

/// View tightly-packed RGB8 bytes as a [`PixelView`].
fn rgb8(bytes: &[u8]) -> PixelView<'_> {
    PixelView::new(bytes, W, H, W as usize * 3, &zenpixels::formats::RGB8_SRGB).unwrap()
}

/// One codec set, built once. Real code registers real codecs here.
fn codecs() -> CodecSet {
    CodecSet::new()
        .with_decoder(ReferenceZcrDecoderConfig)
        .with_encoder(ReferenceEncoderConfig::new())
}

// ── The core round trip ─────────────────────────────────────────────────────

#[test]
fn encode_then_decode() {
    let codecs = codecs();

    // Encode: name the output format, hand it the pixels.
    let file = codecs.encode(ImageFormat::Pnm, rgb8(&RGB)).unwrap();

    // Decode: bytes in, pixels + info out. The format is auto-detected.
    let image = codecs.decode(file.data()).unwrap();
    assert_eq!((image.width(), image.height()), (W, H));
}

#[test]
fn probe_without_decoding() {
    let codecs = codecs();
    let file = codecs.encode(ImageFormat::Pnm, rgb8(&RGB)).unwrap();

    // Header parse only — no pixels touched.
    let info = codecs.probe(file.data()).unwrap();
    assert_eq!((info.width, info.height), (W, H));
}

#[test]
fn share_one_set_app_wide() {
    // `CodecSet` is Send + Sync + 'static and every op takes &self, so build it
    // once behind a static and share it everywhere — no locking.
    static CODECS: LazyLock<CodecSet> = LazyLock::new(codecs);

    let file = CODECS.encode(ImageFormat::Pnm, rgb8(&RGB)).unwrap();
    assert!(CODECS.probe(file.data()).is_ok());
}

// ── The one-call transcode ──────────────────────────────────────────────────

#[test]
fn transcode_in_one_call() {
    let codecs = codecs();

    // A source file (here the reference bytes stand in for, say, a JPEG).
    let source = codecs.encode(ImageFormat::Pnm, rgb8(&RGB)).unwrap();

    // decode → carry the source's metadata → re-encode, in one call — the
    // headline proxy operation. `metadata` decides retention (`Web` strips
    // GPS/camera/timestamps/XMP, keeps orientation + rights + color); `color`
    // decides how color *signaling* is emitted (not a pixel CMS conversion).
    let out = codecs
        .transcode(
            source.data(),
            ImageFormat::Pnm,
            Fidelity::Lossless,
            MetadataPolicy::Web,
            ColorEmitPolicy::Balanced,
        )
        .unwrap();

    assert!(codecs.probe(out.data()).is_ok());
}

// ── Encode / decode controls ────────────────────────────────────────────────

#[test]
#[allow(deprecated)] // Retain the established best-effort metadata compatibility path.
fn encode_with_fidelity_and_metadata() {
    let codecs = codecs();

    // A per-call fidelity override, without disturbing the registered template.
    let file = codecs
        .encode_with(ImageFormat::Pnm, Fidelity::Lossless, rgb8(&RGB))
        .unwrap();
    assert!(codecs.probe(file.data()).is_ok());

    // Metadata (ICC/EXIF/XMP) rides along via the job. Configure it, then
    // `encode` straight off the job — no `into_encoder()` step.
    let mut job = codecs.encode_job(ImageFormat::Pnm).unwrap();
    job.set_metadata_policy(
        Metadata::none().with_icc(vec![1, 2, 3, 4]),
        MetadataPolicy::PreserveExact,
    );
    let with_icc = job.encode(rgb8(&RGB)).unwrap();
    let info = codecs.probe(with_icc.data()).unwrap();
    assert!(info.source_color.icc_profile.is_some());
}

#[test]
fn decode_requesting_a_pixel_format() {
    let codecs = codecs();
    let file = codecs.encode(ImageFormat::Pnm, rgb8(&RGB)).unwrap();

    // Ask the decoder to produce RGBA8. A decoder that can output several
    // formats honors the preference; one that can't (like the reference)
    // returns its native format — so read the descriptor back, don't assume it.
    // When you truly need a format the decoder can't make, convert the output
    // (see `convert_a_decoded_image_to_another_format`).
    let image = codecs
        .decode_preferring(file.data(), &[PixelFormat::RGBA8_SRGB])
        .unwrap();
    let _got = image.pixels().format().clone();
    assert_eq!((image.width(), image.height()), (W, H));
}

#[test]
fn stream_decode_by_strip() {
    let codecs = codecs();
    let file = codecs.encode(ImageFormat::Pnm, rgb8(&RGB)).unwrap();

    // Pull strips instead of materializing the whole image.
    let mut dec = codecs.streaming_decoder(file.data(), &[]).unwrap();
    let mut rows = 0;
    while let Some((_y, strip)) = dec.next_batch().unwrap() {
        rows += strip.rows();
    }
    assert_eq!(rows, H);
}

// ── Estimate before you commit ──────────────────────────────────────────────

#[test]
fn estimate_before_encoding() {
    let codecs = codecs();

    // `estimate_encode_of` reads dims + format off the pixel slice you already
    // hold — no `ImageCharacteristics` to build by hand. `host()` detects this
    // machine (cores + SIMD tier; std only).
    let est = codecs
        .estimate_encode_of(ImageFormat::Pnm, rgb8(&RGB), &ComputeEnvironment::host())
        .unwrap();

    // The reference codec ships no cost model, so this is `unknown()`; a real
    // codec fills in peak-memory / wall-time. The point is the one-call shape.
    let _ = est;
}

#[test]
fn estimate_before_decoding() {
    let codecs = codecs();
    let file = codecs.encode(ImageFormat::Pnm, rgb8(&RGB)).unwrap();

    // `estimate_decode_of` probes the bytes for dims + format, then estimates.
    // `conservative()` is the no_std-friendly single-core baseline.
    let est = codecs
        .estimate_decode_of(file.data(), &ComputeEnvironment::conservative())
        .unwrap();
    let _ = est;
}

// ── With zenpixels-convert on hand ──────────────────────────────────────────

#[test]
fn encode_from_a_foreign_pixel_format() {
    use zenpixels::{ColorOrigin, PeakSource};
    use zenpixels_convert::{ConvertPolicy, Negotiate, OutputPlan, OutputSignaling};

    let codecs = codecs();

    // The caller holds a BGRA8 framebuffer — a format the encoder does NOT list.
    // 2x2 BGRA: the RGB colors above, byte-swapped, opaque.
    let bgra: [u8; 16] = [
        30, 20, 10, 255, 60, 50, 40, 255, 90, 80, 70, 255, 120, 110, 100, 255,
    ];

    // What does this encoder accept? Ask the registry.
    let supported = codecs
        .encoder_for(ImageFormat::Pnm)
        .unwrap()
        .supported_descriptors();

    let source =
        PixelView::new(&bgra, W, H, W as usize * 4, &zenpixels::formats::BGRA8_SRGB).unwrap();
    let policy = ConvertPolicy::permissive();
    let target = Negotiate::select(source.format(), supported, &policy).unwrap();
    let plan = OutputPlan::build(
        source.format(),
        &target.format,
        &policy,
        PeakSource::Measured,
        OutputSignaling::Strip,
    )
    .unwrap();
    let adapted = plan.execute_view(source, &ColorOrigin::assumed()).unwrap();

    let file = codecs
        .encode(ImageFormat::Pnm, adapted.pixels.as_view())
        .unwrap();

    // Round-trips: decode back and red sits where RGB expects it.
    let image = codecs.decode(file.data()).unwrap();
    assert_eq!(image.pixels().row(0)[0], 10);
}

#[test]
fn convert_a_decoded_image_to_another_format() {
    use zenpixels::PeakSource;
    use zenpixels_convert::{ConvertPolicy, ImagePlan};

    let codecs = codecs();
    let file = codecs.encode(ImageFormat::Pnm, rgb8(&RGB)).unwrap();

    // Decode gives the codec's native format; convert the owned buffer to
    // whatever a downstream consumer wants (here BGRA8, e.g. for a GPU upload).
    let image = codecs.decode(file.data()).unwrap();
    let target = PixelFormat::BGRA8_SRGB;
    let conversion = ImagePlan::build(
        image.buffer().format(),
        &target,
        &ConvertPolicy::lossless(),
        PeakSource::Measured,
    )
    .unwrap();
    let bgra = conversion.execute(image.buffer()).unwrap();
    assert_eq!(bgra.format(), &target);
}

#[test]
fn ranked_alpha_corrected_animation_and_exact_begin_admission() {
    use zencodec::decode::FrameCompletion;
    use zencodec::{OrientationHint, UnsupportedOperation};
    use zencodec_testkit::{
        DisplayFrameCase, ExpectedError, ReferenceDecoderConfig, ReferenceEncoderConfig,
        check_animation_begin_refusal, check_animation_delivery, check_decode_preferences,
    };
    use zenpixels::{PixelFormat, PixelView, Primaries};

    let rgba = PixelFormat::RGBA8_SRGB;
    let raw = [
        10, 11, 12, 0, 20, 21, 22, 64, 30, 31, 32, 255, 40, 41, 42, 255, 50, 51, 52, 99, 60, 61,
        62, 5,
    ];
    let source = PixelView::new(&raw, 3, 2, 12, &rgba).unwrap();
    let alternate = [
        9, 8, 7, 77, 6, 5, 4, 0, 3, 2, 1, 22, 1, 2, 3, 99, 4, 5, 6, 33, 7, 8, 9, 0,
    ];
    // Independent ZCR1 fixture. The fixed header carries dimensions, frame
    // count, layout, EXIF orientation 6, and optional metadata flags.
    let mut file = b"ZCR1".to_vec();
    for value in [3u32, 2, 2] {
        file.extend_from_slice(&value.to_le_bytes());
    }
    file.extend_from_slice(&[4, 6, 0]);
    for (samples, ms) in [(&raw, 40u32), (&alternate, 70)] {
        file.extend_from_slice(&ms.to_le_bytes());
        file.extend_from_slice(samples);
    }
    let decoder = ReferenceDecoderConfig;
    let prefs = [
        PixelFormat::GRAY8_SRGB,
        rgba.clone(),
        PixelFormat::RGB8_SRGB,
    ];
    check_decode_preferences(decoder.clone(), &file, &prefs, source.clone()).unwrap();
    let displayed = [
        [
            40, 41, 42, 255, 10, 11, 12, 0, 50, 51, 52, 99, 20, 21, 22, 64, 60, 61, 62, 5, 30, 31,
            32, 255,
        ],
        [
            1, 2, 3, 99, 9, 8, 7, 77, 4, 5, 6, 33, 6, 5, 4, 0, 7, 8, 9, 0, 3, 2, 1, 22,
        ],
    ];
    let cases: Vec<_> = displayed
        .iter()
        .zip([40, 70])
        .enumerate()
        .map(|(i, (samples, ms))| DisplayFrameCase {
            pixels: PixelView::new(samples, 2, 3, 8, &rgba).unwrap(),
            timing: FrameCompletion {
                duration: zencodec::animation::FrameDuration::from_millis(ms),
                frame_index: i as u32,
                pts: None,
            },
        })
        .collect();
    check_animation_delivery(
        decoder.clone(),
        &file,
        &prefs,
        OrientationHint::Correct,
        Some(0),
        &cases,
    )
    .unwrap();
    assert!(
        check_animation_delivery(
            decoder,
            &file,
            &prefs,
            OrientationHint::Preserve,
            Some(0),
            &cases
        )
        .is_err()
    );

    let p3_spec = PixelFormat::RGBA8_SRGB.with_primaries(Primaries::DisplayP3);
    let p3 = PixelView::new(&raw, 3, 2, 12, &p3_spec).unwrap();
    check_animation_begin_refusal(
        ReferenceEncoderConfig::new(),
        source,
        p3,
        ExpectedError::Unsupported(UnsupportedOperation::PixelFormat),
    )
    .unwrap();
}

#[test]
fn independent_metadata_inspection_rejects_substitution_and_missing_authority() {
    use zencodec::{Cicp, ColorAuthority, Metadata, SourceColor};
    use zencodec_testkit::{check_metadata_inspector, check_source_color_fixture};
    use zenpixels_convert::icc_profiles::SynthesizedIcc;

    let expected = Metadata::none().with_cicp(Cicp::DISPLAY_P3);
    let inspect = |raw: &[u8]| -> Result<Metadata, &str> {
        let [b'c', b'I', b'C', b'P', p, t, m, 1] = raw else {
            return Err("incomplete carrier");
        };
        Ok(Metadata::none().with_cicp(Cicp::new(*p, *t, *m, true)))
    };
    let accepted = [b'c', b'I', b'C', b'P', 12, 13, 0, 1];
    check_metadata_inspector(&accepted, &expected, inspect).unwrap();
    assert!(check_metadata_inspector(&accepted[..7], &expected, inspect).is_err());
    let mut substitution = accepted;
    substitution[4] = 1;
    assert!(check_metadata_inspector(&substitution, &expected, inspect).is_err());

    let SynthesizedIcc::Profile(profile) = SynthesizedIcc::for_cicp(Cicp::DISPLAY_P3) else {
        panic!("expected an RGB ICC fixture");
    };
    let pq = Cicp::BT2100_PQ_RGB;
    let conflict = SourceColor::default()
        .with_icc_profile(profile.as_ref())
        .with_cicp(pq);
    check_source_color_fixture(
        &conflict,
        Some(pq),
        Some(profile.as_ref()),
        ColorAuthority::Icc,
    )
    .unwrap();
    assert!(!conflict.has_hdr_transfer());
    assert!(conflict.any_tag_has_hdr_transfer());
    let wrong = conflict.with_color_authority(ColorAuthority::Cicp);
    assert!(
        check_source_color_fixture(
            &wrong,
            Some(pq),
            Some(profile.as_ref()),
            ColorAuthority::Icc
        )
        .is_err()
    );
    assert!(
        check_source_color_fixture(
            &SourceColor::default(),
            Some(pq),
            Some(profile.as_ref()),
            ColorAuthority::Icc
        )
        .is_err()
    );
}
