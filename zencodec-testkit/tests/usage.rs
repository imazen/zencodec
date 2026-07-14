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
use zenpixels::{PixelDescriptor, PixelSlice};

// The reference codec stands in for a real one. Its encoder emits a "ZCR1" wire
// format; `ReferenceZcrDecoderConfig` registers the decoder under a format whose
// `detect` matches those bytes, so the detection-based APIs (`decode`, `probe`,
// `estimate_decode_of`) work end to end below with no custom-format boilerplate.

const W: u32 = 2;
const H: u32 = 2;
const RGB: [u8; 12] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];

/// View tightly-packed RGB8 bytes as a [`PixelSlice`].
fn rgb8(bytes: &[u8]) -> PixelSlice<'_> {
    PixelSlice::new(bytes, W, H, W as usize * 3, PixelDescriptor::RGB8_SRGB).unwrap()
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
        .decode_preferring(file.data(), &[PixelDescriptor::RGBA8_SRGB])
        .unwrap();
    let _got = image.pixels().descriptor();
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
    use zenpixels_convert::adapt::adapt_for_encode;

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

    // `adapt_for_encode` picks the best supported target and converts into it
    // (it would borrow, zero-copy, if BGRA8 were already supported).
    let adapted = adapt_for_encode(
        &bgra,
        PixelDescriptor::BGRA8_SRGB,
        W,
        H,
        W as usize * 4,
        supported,
    )
    .unwrap();

    let bytes: &[u8] = &adapted.data;
    let stride = adapted.width as usize * adapted.descriptor.bytes_per_pixel();
    let slice = PixelSlice::new(
        bytes,
        adapted.width,
        adapted.rows,
        stride,
        adapted.descriptor,
    )
    .unwrap();
    let file = codecs.encode(ImageFormat::Pnm, slice).unwrap();

    // Round-trips: decode back and red sits where RGB expects it.
    let image = codecs.decode(file.data()).unwrap();
    assert_eq!(image.pixels().row(0)[0], 10);
}

#[test]
fn convert_a_decoded_image_to_another_format() {
    use zenpixels_convert::PixelBufferConvertExt;

    let codecs = codecs();
    let file = codecs.encode(ImageFormat::Pnm, rgb8(&RGB)).unwrap();

    // Decode gives the codec's native format; convert the owned buffer to
    // whatever a downstream consumer wants (here BGRA8, e.g. for a GPU upload).
    let image = codecs.decode(file.data()).unwrap();
    let bgra = image
        .into_buffer()
        .convert_to(PixelDescriptor::BGRA8_SRGB)
        .unwrap();
    assert_eq!(bgra.as_slice().descriptor(), PixelDescriptor::BGRA8_SRGB);
}
