//! Conformance test harness for [`zencodec`] codec implementations.
//!
//! A codec crate adds this as a `dev-dependency` and runs the `check_*`
//! functions against its own [`EncoderConfig`] / [`DecoderConfig`] to verify it
//! honors the shared contract — especially the parts that are easy to get
//! subtly wrong and expensive to ship wrong:
//!
//! - [`check_metadata_no_leak`] — a [`MetadataPolicy`] must never leak what it
//!   discards. The privacy guarantee.
//! - [`check_cross_path_pixel_equivalence`] — every feeding mode (one-shot,
//!   incremental, streaming, push-sink) must produce identical pixels.
//! - [`check_capability_honesty`] — a declared capability works; an undeclared
//!   one cleanly returns [`UnsupportedOperation`](zencodec::UnsupportedOperation).
//!
//! The [`reference`] module ships a faithful codec the harness is validated
//! against, which also serves as a worked example.
//!
//! [`EncoderConfig`]: zencodec::encode::EncoderConfig
//! [`DecoderConfig`]: zencodec::decode::DecoderConfig

use std::borrow::Cow;

use zencodec::CodecErrorExt;
use zencodec::decode::{
    Decode, DecodeJob, DecodeRowSink, DecoderConfig, SinkError, StreamingDecode,
};
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec::exif::Exif;
use zencodec::{Metadata, MetadataFields, MetadataPolicy, Orientation};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

pub mod fixtures;
pub mod reference;

pub use reference::{RefError, ReferenceDecoderConfig, ReferenceEncoderConfig};

// ===========================================================================
// Result types
// ===========================================================================

/// A conformance-check failure, naming the check and a human-readable detail.
#[derive(Debug, Clone)]
pub struct Failure {
    /// The check that failed (e.g. `"metadata_no_leak"`).
    pub check: &'static str,
    /// What went wrong, with enough context to act on.
    pub detail: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.check, self.detail)
    }
}

impl std::error::Error for Failure {}

/// The result of a conformance check.
pub type Conformance = Result<(), Failure>;

fn fail(check: &'static str, detail: impl Into<String>) -> Failure {
    Failure {
        check,
        detail: detail.into(),
    }
}

// ===========================================================================
// Test image
// ===========================================================================

/// A deterministic test image with contiguous (tightly-packed) rows.
pub struct TestImage {
    width: u32,
    height: u32,
    desc: PixelDescriptor,
    data: Vec<u8>,
}

impl TestImage {
    /// An RGBA8 image whose channels vary with position, so any row/column
    /// transposition shows up as a pixel diff.
    pub fn rgba8_gradient(width: u32, height: u32) -> Self {
        Self::gradient(width, height, PixelDescriptor::RGBA8_SRGB, 4)
    }

    /// An RGB8 image with the same gradient pattern.
    pub fn rgb8_gradient(width: u32, height: u32) -> Self {
        Self::gradient(width, height, PixelDescriptor::RGB8_SRGB, 3)
    }

    fn gradient(width: u32, height: u32, desc: PixelDescriptor, bpp: usize) -> Self {
        assert!(width > 0 && height > 0, "test image must be non-empty");
        let mut data = vec![0u8; width as usize * height as usize * bpp];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let p = (y * width as usize + x) * bpp;
                data[p] = (x * 7 + y * 3) as u8; // R
                data[p + 1] = (x * 3 + y * 11) as u8; // G
                data[p + 2] = (x ^ y) as u8; // B
                if bpp == 4 {
                    data[p + 3] = 255 - (x + y) as u8; // A
                }
            }
        }
        Self {
            width,
            height,
            desc,
            data,
        }
    }

    fn row_bytes(&self) -> usize {
        self.width as usize * self.desc.bytes_per_pixel()
    }

    /// Borrow the whole image as a [`PixelSlice`].
    pub fn as_slice(&self) -> PixelSlice<'_> {
        PixelSlice::new(
            &self.data,
            self.width,
            self.height,
            self.row_bytes(),
            self.desc,
        )
        .expect("test image dimensions are valid")
    }

    fn strip(&self, y: u32, h: u32) -> PixelSlice<'_> {
        let rb = self.row_bytes();
        let bytes = &self.data[y as usize * rb..(y as usize + h as usize) * rb];
        PixelSlice::new(bytes, self.width, h, rb, self.desc).expect("strip dimensions are valid")
    }

    fn pixels(&self) -> Pixels {
        grab(self.as_slice())
    }
}

// ===========================================================================
// Pixel comparison
// ===========================================================================

/// A decoded image flattened to contiguous rows for byte-exact comparison.
#[derive(PartialEq, Eq)]
struct Pixels {
    width: u32,
    rows: u32,
    desc: PixelDescriptor,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Pixels {{ {}x{} {:?}, {} bytes }}",
            self.width,
            self.rows,
            self.desc,
            self.bytes.len()
        )
    }
}

fn grab(ps: PixelSlice<'_>) -> Pixels {
    let rb = ps.width() as usize * ps.descriptor().bytes_per_pixel();
    let mut bytes = Vec::with_capacity(rb * ps.rows() as usize);
    for y in 0..ps.rows() {
        bytes.extend_from_slice(&ps.row(y)[..rb]);
    }
    Pixels {
        width: ps.width(),
        rows: ps.rows(),
        desc: ps.descriptor(),
        bytes,
    }
}

// ===========================================================================
// Collecting decode sink
// ===========================================================================

/// A [`DecodeRowSink`] that gathers all strips into one contiguous buffer.
#[derive(Default)]
struct CollectSink {
    width: u32,
    rows: u32,
    desc: Option<PixelDescriptor>,
    buf: Vec<u8>,
}

impl DecodeRowSink for CollectSink {
    fn begin(
        &mut self,
        width: u32,
        height: u32,
        descriptor: PixelDescriptor,
    ) -> Result<(), SinkError> {
        self.width = width;
        self.rows = height;
        self.desc = Some(descriptor);
        self.buf = vec![0u8; width as usize * height as usize * descriptor.bytes_per_pixel()];
        Ok(())
    }

    fn provide_next_buffer(
        &mut self,
        y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        let stride = width as usize * descriptor.bytes_per_pixel();
        let end = (y as usize + height as usize) * stride;
        if self.buf.len() < end {
            self.buf.resize(end, 0);
        }
        self.width = width;
        self.desc = Some(descriptor);
        self.rows = self.rows.max(y + height);
        let off = y as usize * stride;
        // Dimensions and span are exact by construction, so this never fails.
        Ok(
            PixelSliceMut::new(&mut self.buf[off..end], width, height, stride, descriptor)
                .expect("collect sink buffer dimensions are valid"),
        )
    }
}

impl CollectSink {
    fn into_pixels(self) -> Result<Pixels, String> {
        let desc = self.desc.ok_or("sink received no buffers")?;
        Ok(Pixels {
            width: self.width,
            rows: self.rows,
            desc,
            bytes: self.buf,
        })
    }
}

// ===========================================================================
// Encode / decode path runners (generic over codec config)
// ===========================================================================

fn enc_oneshot<E>(
    cfg: &E,
    img: &TestImage,
    meta: Metadata,
    policy: MetadataPolicy,
) -> Result<Vec<u8>, String>
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    let enc = cfg
        .clone()
        .job()
        .with_metadata_policy(meta, policy)
        .encoder()
        .map_err(|e| e.to_string())?;
    Ok(enc
        .encode(img.as_slice())
        .map_err(|e| e.to_string())?
        .into_vec())
}

fn enc_push_rows<E>(cfg: &E, img: &TestImage) -> Result<Vec<u8>, String>
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    let mut enc = cfg
        .clone()
        .job()
        .with_metadata_policy(Metadata::none(), MetadataPolicy::PreserveExact)
        .encoder()
        .map_err(|e| e.to_string())?;
    let strip = enc.preferred_strip_height().max(1);
    let mut y = 0;
    while y < img.height {
        let h = strip.min(img.height - y);
        enc.push_rows(img.strip(y, h)).map_err(|e| e.to_string())?;
        y += h;
    }
    Ok(enc.finish().map_err(|e| e.to_string())?.into_vec())
}

fn dec_oneshot<D: DecoderConfig>(cfg: &D, bytes: &[u8]) -> Result<(Pixels, Metadata), String> {
    let out = cfg
        .clone()
        .job()
        .decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    Ok((grab(out.pixels()), out.metadata()))
}

fn dec_streaming<D: DecoderConfig>(cfg: &D, bytes: &[u8]) -> Result<Pixels, String> {
    let mut sd = cfg
        .clone()
        .job()
        .streaming_decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| e.to_string())?;
    let mut width = 0;
    let mut rows = 0;
    let mut desc = None;
    let mut bytes_out = Vec::new();
    while let Some((_, strip)) = sd.next_batch().map_err(|e| e.to_string())? {
        let p = grab(strip);
        if desc.is_none() {
            width = p.width;
            desc = Some(p.desc);
        }
        rows += p.rows;
        bytes_out.extend_from_slice(&p.bytes);
    }
    Ok(Pixels {
        width,
        rows,
        desc: desc.ok_or("streaming decoder yielded no strips")?,
        bytes: bytes_out,
    })
}

fn dec_push<D: DecoderConfig>(cfg: &D, bytes: &[u8]) -> Result<Pixels, String> {
    let mut sink = CollectSink::default();
    cfg.clone()
        .job()
        .push_decoder(Cow::Borrowed(bytes), &mut sink, &[])
        .map_err(|e| e.to_string())?;
    sink.into_pixels()
}

// ===========================================================================
// Conformance checks
// ===========================================================================

/// A round trip through one-shot encode → one-shot decode reproduces the input
/// pixels exactly. The smallest sanity check; a failure here means nothing else
/// is trustworthy.
pub fn check_pixel_roundtrip<E, D>(enc: E, dec: D, img: &TestImage) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "pixel_roundtrip";
    let bytes = enc_oneshot(&enc, img, Metadata::none(), MetadataPolicy::PreserveExact)
        .map_err(|e| fail(CHECK, format!("encode: {e}")))?;
    let (got, _) = dec_oneshot(&dec, &bytes).map_err(|e| fail(CHECK, format!("decode: {e}")))?;
    if got != img.pixels() {
        return Err(fail(
            CHECK,
            format!(
                "decoded pixels differ from the {}x{} input",
                img.width, img.height
            ),
        ));
    }
    Ok(())
}

/// Every advertised feeding mode produces identical pixels.
///
/// Encode paths: one-shot, plus incremental `push_rows` when the encoder's
/// capabilities advertise it. Decode paths: one-shot, push-sink, plus streaming
/// when advertised. All decoded results must equal each other *and* the input.
pub fn check_cross_path_pixel_equivalence<E, D>(enc: E, dec: D, img: &TestImage) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "cross_path_pixel_equivalence";
    let want = img.pixels();

    // --- decode paths over a canonical one-shot encode ---
    let canonical = enc_oneshot(&enc, img, Metadata::none(), MetadataPolicy::PreserveExact)
        .map_err(|e| fail(CHECK, format!("canonical encode: {e}")))?;

    let mut decoded: Vec<(&str, Pixels)> = Vec::new();
    decoded.push((
        "decode",
        dec_oneshot(&dec, &canonical)
            .map_err(|e| fail(CHECK, format!("one-shot decode: {e}")))?
            .0,
    ));
    decoded.push((
        "push_decoder",
        dec_push(&dec, &canonical).map_err(|e| fail(CHECK, format!("push decode: {e}")))?,
    ));
    if D::capabilities().streaming() {
        decoded.push((
            "streaming",
            dec_streaming(&dec, &canonical)
                .map_err(|e| fail(CHECK, format!("streaming decode: {e}")))?,
        ));
    }

    for (name, px) in &decoded {
        if *px != want {
            return Err(fail(
                CHECK,
                format!("decode path `{name}` diverged from the input image"),
            ));
        }
    }

    // --- encode paths must all decode back to the input ---
    if E::capabilities().push_rows() {
        let pr =
            enc_push_rows(&enc, img).map_err(|e| fail(CHECK, format!("push_rows encode: {e}")))?;
        let (got, _) = dec_oneshot(&dec, &pr)
            .map_err(|e| fail(CHECK, format!("decode push_rows output: {e}")))?;
        if got != want {
            return Err(fail(
                CHECK,
                "push_rows encode produced different pixels than one-shot encode".to_string(),
            ));
        }
    }

    Ok(())
}

/// A retention policy never leaks what it discards.
///
/// Encodes the image with rich metadata (GPS + thumbnail + camera + copyright +
/// XMP + ICC) under several policies, decodes, and asserts the output metadata
/// is a *subset* of what the policy keeps. A subset (not equality) check, so a
/// codec that supports fewer channels still passes — it can drop more, never
/// add back. Anything the policy dropped reappearing in the output is a leak.
pub fn check_metadata_no_leak<E, D>(enc: E, dec: D, img: &TestImage) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "metadata_no_leak";
    let rich = Metadata::none()
        .with_exif(fixtures::rich_exif_le())
        .with_xmp(fixtures::sample_xmp())
        .with_icc(fixtures::sample_icc());

    let policies = [
        ("Web", MetadataPolicy::Web),
        ("ColorAndRotation", MetadataPolicy::ColorAndRotation),
        ("PreserveExact", MetadataPolicy::PreserveExact),
        (
            "Custom(DISCARD_ALL)",
            MetadataPolicy::Custom(MetadataFields::DISCARD_ALL),
        ),
    ];

    for (name, policy) in policies {
        let expected = rich.clone().filtered(&policy);
        let bytes = enc_oneshot(&enc, img, rich.clone(), policy)
            .map_err(|e| fail(CHECK, format!("[{name}] encode: {e}")))?;
        let (_, decoded) =
            dec_oneshot(&dec, &bytes).map_err(|e| fail(CHECK, format!("[{name}] decode: {e}")))?;
        assert_no_leak(CHECK, name, &decoded, &expected)?;
    }
    Ok(())
}

fn assert_no_leak(
    check: &'static str,
    policy: &str,
    decoded: &Metadata,
    expected: &Metadata,
) -> Conformance {
    if decoded.icc_profile.is_some() && expected.icc_profile.is_none() {
        return Err(fail(
            check,
            format!("[{policy}] ICC profile in output but the policy dropped it"),
        ));
    }
    if decoded.xmp.is_some() && expected.xmp.is_none() {
        return Err(fail(
            check,
            format!("[{policy}] XMP in output but the policy dropped it"),
        ));
    }

    match &decoded.exif {
        None => {} // nothing embedded is always safe
        Some(d) => {
            if expected.exif.is_none() {
                return Err(fail(
                    check,
                    format!("[{policy}] EXIF in output but the policy dropped the whole blob"),
                ));
            }
            let want = expected.exif.as_deref().and_then(Exif::parse);
            if let Some(dx) = Exif::parse(d.as_ref()) {
                let want_gps = want.as_ref().is_some_and(Exif::has_gps);
                if dx.has_gps() && !want_gps {
                    return Err(fail(
                        check,
                        format!(
                            "[{policy}] GPS data in output EXIF but the policy dropped it (privacy leak)"
                        ),
                    ));
                }
                let want_thumb = want.as_ref().is_some_and(Exif::has_thumbnail);
                if dx.has_thumbnail() && !want_thumb {
                    return Err(fail(
                        check,
                        format!(
                            "[{policy}] thumbnail in output EXIF but the policy dropped it (privacy leak)"
                        ),
                    ));
                }
                let want_rights = want
                    .as_ref()
                    .is_some_and(|w| w.copyright().is_some() || w.artist().is_some());
                if (dx.copyright().is_some() || dx.artist().is_some()) && !want_rights {
                    return Err(fail(
                        check,
                        format!(
                            "[{policy}] rights tags in output EXIF but the policy dropped them"
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// An orientation set on the metadata survives a policy that keeps it, exactly
/// once.
///
/// `Web`, `ColorAndRotation`, and `PreserveExact` all keep orientation (it is
/// display-correctness, not privacy). This encodes each non-identity orientation
/// under each, decodes, and asserts the decoded orientation matches — catching
/// both *loss* (reset to identity) and the *double-application* hazard where a
/// codec bakes the rotation into pixels and also re-emits the tag.
pub fn check_orientation_roundtrip<E, D>(enc: E, dec: D, img: &TestImage) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "orientation_roundtrip";
    let orientations = [
        Orientation::Rotate90,
        Orientation::Rotate180,
        Orientation::Rotate270,
        Orientation::FlipH,
        Orientation::FlipV,
    ];
    let policies = [
        ("Web", MetadataPolicy::Web),
        ("ColorAndRotation", MetadataPolicy::ColorAndRotation),
        ("PreserveExact", MetadataPolicy::PreserveExact),
    ];
    for ori in orientations {
        for (name, policy) in policies {
            let meta = Metadata::none().with_orientation(ori);
            let bytes = enc_oneshot(&enc, img, meta, policy)
                .map_err(|e| fail(CHECK, format!("[{name}] encode {ori:?}: {e}")))?;
            let (_, decoded) = dec_oneshot(&dec, &bytes)
                .map_err(|e| fail(CHECK, format!("[{name}] decode {ori:?}: {e}")))?;
            if decoded.orientation != ori {
                return Err(fail(
                    CHECK,
                    format!(
                        "[{name}] orientation {ori:?} survived as {:?}",
                        decoded.orientation
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Declared capabilities match real behavior.
///
/// For now: the encoder's pull path (`encode_from`) must succeed iff
/// [`EncodeCapabilities::encode_from`](zencodec::EncodeCapabilities::encode_from)
/// is set; when unset, the call must fail with an
/// [`UnsupportedOperation`](zencodec::UnsupportedOperation) in its cause chain
/// (not a panic, not a silent success).
pub fn check_capability_honesty<E, D>(enc: E, _dec: D) -> Conformance
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "capability_honesty";
    if !E::capabilities().encode_from() {
        let encoder = enc
            .clone()
            .job()
            .with_metadata_policy(Metadata::none(), MetadataPolicy::PreserveExact)
            .encoder()
            .map_err(|e| fail(CHECK, format!("build encoder: {e}")))?;
        // Source that immediately signals end-of-image; an honest decline should
        // reject before consuming it.
        let mut src = |_y: u32, _buf: PixelSliceMut<'_>| 0usize;
        match encoder.encode_from(&mut src) {
            Ok(_) => {
                return Err(fail(
                    CHECK,
                    "encode_from succeeded although capabilities advertise it is unsupported",
                ));
            }
            Err(e) => {
                if e.unsupported_operation().is_none() {
                    return Err(fail(
                        CHECK,
                        format!(
                            "encode_from is unsupported but the error lacks UnsupportedOperation in its chain: {e}"
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ref_codecs() -> (ReferenceEncoderConfig, ReferenceDecoderConfig) {
        (ReferenceEncoderConfig::new(), ReferenceDecoderConfig)
    }

    #[test]
    fn fixture_exif_is_realistic() {
        // Guard the hand-laid TIFF offsets: if any are wrong, this fails loudly
        // rather than letting the no-leak check run against a malformed blob.
        let blob = fixtures::rich_exif_le();
        let x = Exif::parse(&blob).expect("fixture EXIF parses");
        assert!(x.has_gps(), "fixture must contain GPS");
        assert!(x.has_thumbnail(), "fixture must contain a thumbnail");
        assert_eq!(x.copyright().as_deref(), Some("(C) 2026 Test"));
    }

    #[test]
    fn reference_pixel_roundtrip_rgba8() {
        let (e, d) = ref_codecs();
        check_pixel_roundtrip(e, d, &TestImage::rgba8_gradient(37, 19)).unwrap();
    }

    #[test]
    fn reference_pixel_roundtrip_rgb8() {
        let (e, d) = ref_codecs();
        check_pixel_roundtrip(e, d, &TestImage::rgb8_gradient(16, 16)).unwrap();
    }

    #[test]
    fn reference_cross_path_equivalence() {
        let (e, d) = ref_codecs();
        check_cross_path_pixel_equivalence(e, d, &TestImage::rgba8_gradient(40, 23)).unwrap();
    }

    #[test]
    fn reference_metadata_no_leak() {
        let (e, d) = ref_codecs();
        check_metadata_no_leak(e, d, &TestImage::rgba8_gradient(8, 8)).unwrap();
    }

    #[test]
    fn reference_capability_honesty() {
        let (e, d) = ref_codecs();
        check_capability_honesty(e, d).unwrap();
    }

    #[test]
    fn reference_orientation_roundtrip() {
        let (e, d) = ref_codecs();
        check_orientation_roundtrip(e, d, &TestImage::rgba8_gradient(8, 8)).unwrap();
    }

    /// The reference round-trips metadata faithfully, so under PreserveExact the
    /// decoded EXIF must still carry GPS + thumbnail + copyright (positive
    /// direction the generic no-leak check intentionally doesn't assert).
    #[test]
    fn reference_preserve_exact_keeps_everything() {
        let (e, d) = ref_codecs();
        let img = TestImage::rgba8_gradient(8, 8);
        let rich = Metadata::none()
            .with_exif(fixtures::rich_exif_le())
            .with_xmp(fixtures::sample_xmp())
            .with_icc(fixtures::sample_icc());
        let bytes = enc_oneshot(&e, &img, rich, MetadataPolicy::PreserveExact).unwrap();
        let (_, meta) = dec_oneshot(&d, &bytes).unwrap();
        let x = Exif::parse(meta.exif.as_deref().expect("exif kept")).expect("parses");
        assert!(x.has_gps() && x.has_thumbnail());
        assert_eq!(x.copyright().as_deref(), Some("(C) 2026 Test"));
        assert!(meta.xmp.is_some(), "xmp kept");
        assert!(meta.icc_profile.is_some(), "icc kept");
    }

    /// Web strips GPS/thumbnail/XMP but keeps rights — verified on the faithful
    /// reference, where decoded == filtered.
    #[test]
    fn reference_web_strips_privacy_keeps_rights() {
        let (e, d) = ref_codecs();
        let img = TestImage::rgba8_gradient(8, 8);
        let rich = Metadata::none()
            .with_exif(fixtures::rich_exif_le())
            .with_xmp(fixtures::sample_xmp());
        let bytes = enc_oneshot(&e, &img, rich, MetadataPolicy::Web).unwrap();
        let (_, meta) = dec_oneshot(&d, &bytes).unwrap();
        let x = Exif::parse(meta.exif.as_deref().expect("exif kept")).expect("parses");
        assert!(!x.has_gps(), "Web must strip GPS");
        assert!(!x.has_thumbnail(), "Web must strip the thumbnail");
        assert_eq!(
            x.copyright().as_deref(),
            Some("(C) 2026 Test"),
            "Web keeps rights"
        );
        assert!(meta.xmp.is_none(), "Web strips XMP");
    }
}
