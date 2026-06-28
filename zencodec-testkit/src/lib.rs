//! Conformance test harness for [`zencodec`] codec implementations.
//!
//! A codec crate adds this as a `dev-dependency` and runs the `check_*`
//! functions against its own [`EncoderConfig`] / [`DecoderConfig`] to verify it
//! honors the shared contract — especially the parts that are easy to get
//! subtly wrong and expensive to ship wrong:
//!
//! - [`check_metadata_no_leak`] — a [`MetadataPolicy`] must never leak what it
//!   discards. The privacy guarantee.
//! - [`check_cross_path_pixel_equivalence`] — every still encode/decode path
//!   (one-shot, `push_rows` + pull `encode_from` encode; one-shot, push-sink,
//!   streaming decode) must produce identical pixels.
//! - [`check_animation_cross_path_equivalence`] — every animation decode path
//!   (borrowed, owned, push-sink) must yield identical frames, matching the input.
//! - [`check_orientation_roundtrip`] — an orientation survives a keeping policy
//!   exactly once (no loss, no double-application).
//! - [`check_capability_honesty`] — every declared capability works and every
//!   undeclared optional path cleanly returns
//!   [`UnsupportedOperation`](zencodec::UnsupportedOperation). Both directions for
//!   the structural paths and (where the decoder can observe them) the metadata
//!   channels, so a codec can't claim a feature it lacks *or* hide one it has; see
//!   the fn docs for the exact per-flag scope.
//! - [`check_decode_error_envelope`] / [`assert_uses_codec_error_envelope`] — a
//!   codec's [`ErrorCategory`](zencodec::ErrorCategory) and codec name survive
//!   dyn-dispatch type erasure (the `At<CodecError>` envelope contract). Opt-in,
//!   and *not* in [`check_all`]: only for codecs that return the envelope
//!   `type Error` (the testkit's own [`reference`](mod@reference) is a Pattern-A
//!   foil that deliberately fails it).
//!
//! [`check_all`] runs them all with default inputs — the one-call entry point.
//!
//! The [`reference`](mod@reference) module ships a faithful codec (declares and
//! honors every capability) the harness is validated against; the [`minimal`] module ships its
//! opposite (declares every optional capability false) so the false-direction
//! branches are validated too. Both double as worked examples.
//!
//! [`EncoderConfig`]: zencodec::encode::EncoderConfig
//! [`DecoderConfig`]: zencodec::decode::DecoderConfig

use std::borrow::Cow;

use whereat::At;
use zencodec::CodecErrorExt;
use zencodec::decode::{
    AnimationFrameDecoder, Decode, DecodeJob, DecodeRowSink, DecoderConfig, DynDecoderConfig,
    SinkError, StreamingDecode,
};
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, Encoder, EncoderConfig};
use zencodec::exif::Exif;
use zencodec::{Cicp, CodecError, Metadata, MetadataFields, MetadataPolicy, Orientation};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

pub mod fixtures;
pub mod minimal;
pub mod reference;

pub use minimal::{MinimalDecoderConfig, MinimalEncoderConfig};
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
        Self::gradient(width, height, PixelDescriptor::RGBA8_SRGB, 4, 0)
    }

    /// An RGB8 image with the same gradient pattern.
    pub fn rgb8_gradient(width: u32, height: u32) -> Self {
        Self::gradient(width, height, PixelDescriptor::RGB8_SRGB, 3, 0)
    }

    /// An RGBA8 gradient offset by `seed`, so distinct same-size frames (for
    /// animation tests) differ in content and a frame-ordering bug is visible.
    pub fn rgba8_gradient_seeded(width: u32, height: u32, seed: u8) -> Self {
        Self::gradient(width, height, PixelDescriptor::RGBA8_SRGB, 4, seed)
    }

    fn gradient(width: u32, height: u32, desc: PixelDescriptor, bpp: usize, seed: u8) -> Self {
        assert!(width > 0 && height > 0, "test image must be non-empty");
        let s = seed as usize;
        let mut data = vec![0u8; width as usize * height as usize * bpp];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let p = (y * width as usize + x) * bpp;
                data[p] = (x * 7 + y * 3 + s) as u8; // R
                data[p + 1] = (x * 3 + y * 11 + s * 2) as u8; // G
                data[p + 2] = ((x ^ y) + s) as u8; // B
                if bpp == 4 {
                    data[p + 3] = 255 - (x + y + s) as u8; // A
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
    grab_ref(&ps)
}

/// Apply an EXIF `orientation` to `p`, producing the image a conformant reader
/// would *display* — orientation is a rendering transform, not a label. Uses
/// zenpixels' canonical [`Orientation::forward_map`] / `output_dimensions`, so
/// the conventions match production exactly.
fn render(p: &Pixels, orientation: Orientation) -> Pixels {
    let bpp = p.desc.bytes_per_pixel();
    let (w, h) = (p.width, p.rows);
    let (ow, oh) = orientation.output_dimensions(w, h);
    let in_rb = w as usize * bpp;
    let out_rb = ow as usize * bpp;
    let mut bytes = vec![0u8; oh as usize * out_rb];
    for sy in 0..h {
        for sx in 0..w {
            let (dx, dy) = orientation.forward_map(sx, sy, w, h);
            let si = sy as usize * in_rb + sx as usize * bpp;
            let di = dy as usize * out_rb + dx as usize * bpp;
            bytes[di..di + bpp].copy_from_slice(&p.bytes[si..si + bpp]);
        }
    }
    Pixels {
        width: ow,
        rows: oh,
        desc: p.desc,
        bytes,
    }
}

/// `grab` for a borrowed slice — e.g. [`AnimationFrame::pixels`], which returns a
/// reference into the decoder's canvas.
fn grab_ref(ps: &PixelSlice<'_>) -> Pixels {
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

    if E::capabilities().encode_from() {
        let ef = run_encode_from(&enc, img)
            .map_err(|e| fail(CHECK, format!("encode_from encode: {e}")))?;
        let (got, _) = dec_oneshot(&dec, &ef)
            .map_err(|e| fail(CHECK, format!("decode encode_from output: {e}")))?;
        if got != want {
            return Err(fail(
                CHECK,
                "encode_from (pull-source) produced different pixels than one-shot encode"
                    .to_string(),
            ));
        }
    }

    Ok(())
}

fn encode_animation<E>(cfg: &E, frames: &[TestImage]) -> Result<Vec<u8>, String>
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    let mut a = cfg
        .clone()
        .job()
        .with_loop_count(Some(0))
        .animation_frame_encoder()
        .map_err(|e| e.to_string())?;
    for (i, f) in frames.iter().enumerate() {
        a.push_frame(f.as_slice(), 40 + i as u32 * 10, None)
            .map_err(|e| e.to_string())?;
    }
    Ok(a.finish(None).map_err(|e| e.to_string())?.into_vec())
}

fn decode_anim_borrowed<D: DecoderConfig>(cfg: &D, bytes: &[u8]) -> Result<Vec<Pixels>, String> {
    let mut d = cfg
        .clone()
        .job()
        .animation_frame_decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    // The borrowed frame is invalidated by the next call, so copy before looping.
    while let Some(frame) = d.render_next_frame(None).map_err(|e| e.to_string())? {
        out.push(grab_ref(frame.pixels()));
    }
    Ok(out)
}

fn decode_anim_owned<D: DecoderConfig>(cfg: &D, bytes: &[u8]) -> Result<Vec<Pixels>, String> {
    let mut d = cfg
        .clone()
        .job()
        .animation_frame_decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(frame) = d.render_next_frame_owned(None).map_err(|e| e.to_string())? {
        out.push(grab(frame.pixels()));
    }
    Ok(out)
}

fn decode_anim_sink<D: DecoderConfig>(cfg: &D, bytes: &[u8]) -> Result<Vec<Pixels>, String> {
    let mut d = cfg
        .clone()
        .job()
        .animation_frame_decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    loop {
        let mut sink = CollectSink::default();
        match d
            .render_next_frame_to_sink(None, &mut sink)
            .map_err(|e| e.to_string())?
        {
            Some(_info) => out.push(sink.into_pixels()?),
            None => break,
        }
    }
    Ok(out)
}

/// Every animation decode path yields identical frames, matching the input.
///
/// Encodes the frames via the animation encoder, then decodes the result three
/// ways — [`render_next_frame`](zencodec::decode::AnimationFrameDecoder::render_next_frame)
/// (borrowed canvas), `render_next_frame_owned`, and `render_next_frame_to_sink`
/// (push model) — and asserts all three produce the same frame count and the same
/// per-frame pixels as the input. The borrowed path is the usual source of bugs:
/// its frame aliases the decoder's canvas and is invalidated by the next call, so
/// a codec that composites in place can leak the wrong frame.
///
/// Skipped (returns `Ok`) when either end does not advertise animation.
pub fn check_animation_cross_path_equivalence<E, D>(
    enc: E,
    dec: D,
    frames: &[TestImage],
) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    const CHECK: &str = "animation_cross_path_equivalence";
    if !E::capabilities().animation() || !D::capabilities().animation() {
        return Ok(()); // not applicable to a still-only codec
    }
    if frames.is_empty() {
        return Err(fail(CHECK, "no frames supplied"));
    }

    let bytes = encode_animation(&enc, frames).map_err(|e| fail(CHECK, format!("encode: {e}")))?;
    let want: Vec<Pixels> = frames.iter().map(|f| f.pixels()).collect();

    let paths = [
        ("render_next_frame", decode_anim_borrowed(&dec, &bytes)),
        ("render_next_frame_owned", decode_anim_owned(&dec, &bytes)),
        ("render_next_frame_to_sink", decode_anim_sink(&dec, &bytes)),
    ];
    for (name, res) in paths {
        let got = res.map_err(|e| fail(CHECK, format!("{name}: {e}")))?;
        if got.len() != want.len() {
            return Err(fail(
                CHECK,
                format!(
                    "{name} produced {} frames, expected {}",
                    got.len(),
                    want.len()
                ),
            ));
        }
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            if g != w {
                return Err(fail(
                    CHECK,
                    format!("{name} frame {i} pixels differ from the input"),
                ));
            }
        }
    }
    Ok(())
}

/// A retention policy never leaks what it discards.
///
/// Encodes the image with rich metadata (GPS + thumbnail + camera + copyright +
/// XMP + ICC + CICP) under several policies, decodes, and asserts the output
/// metadata is a *subset* of what the policy keeps. A subset (not equality)
/// check, so a codec that supports fewer channels still passes — it can drop
/// more, never add back. Anything the policy dropped reappearing in the output
/// is a leak.
///
/// What's directly asserted on the decoded output: ICC, XMP, CICP, HDR
/// (content-light-level / mastering-display), and the EXIF sub-categories the
/// public [`Exif`] API can introspect — GPS, thumbnail, rights (copyright/artist),
/// **camera/device identity** ([`Exif::has_camera`], Make/Model/MakerNote/serials/…)
/// and **capture timestamps** ([`Exif::has_datetimes`]). An emitted EXIF blob that
/// fails to re-parse is also a failure (a mangled blob can hide raw GPS/camera
/// bytes a lenient reader scrapes, and its drops can't be verified).
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
        .with_icc(fixtures::sample_icc())
        .with_cicp(Cicp::SRGB);

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
    if decoded.cicp.is_some() && expected.cicp.is_none() {
        return Err(fail(
            check,
            format!("[{policy}] CICP color signaling in output but the policy dropped it"),
        ));
    }
    if decoded.content_light_level.is_some() && expected.content_light_level.is_none() {
        return Err(fail(
            check,
            format!("[{policy}] HDR content-light-level in output but the policy dropped it"),
        ));
    }
    if decoded.mastering_display.is_some() && expected.mastering_display.is_none() {
        return Err(fail(
            check,
            format!("[{policy}] HDR mastering-display in output but the policy dropped it"),
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
                let want_camera = want.as_ref().is_some_and(Exif::has_camera);
                if dx.has_camera() && !want_camera {
                    return Err(fail(
                        check,
                        format!(
                            "[{policy}] camera-identity tags (Make/Model/MakerNote/serial/…) in output EXIF but the policy dropped them (privacy leak)"
                        ),
                    ));
                }
                let want_datetimes = want.as_ref().is_some_and(Exif::has_datetimes);
                if dx.has_datetimes() && !want_datetimes {
                    return Err(fail(
                        check,
                        format!(
                            "[{policy}] capture-timestamp tags in output EXIF but the policy dropped them (privacy leak)"
                        ),
                    ));
                }
            } else {
                // The output carries an EXIF blob the policy meant to keep (in part),
                // but it does not parse. A mangled blob can still embed raw GPS /
                // camera bytes a lenient reader scrapes, and the drops can't be
                // verified — treat an unparseable emitted blob as a failure.
                return Err(fail(
                    check,
                    format!(
                        "[{policy}] output EXIF is unparseable — cannot verify the policy's drops, and it may hide raw GPS/camera bytes"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// The *displayed* image is preserved through a policy that keeps orientation.
///
/// EXIF orientation is a rendering transform the reader applies, not a label, so
/// the invariant is on the displayed pixels — `render(pixels, orientation)` — not
/// the stored buffer. For each non-identity orientation under each keeping policy
/// (`Web`, `ColorAndRotation`, `PreserveExact`), this asserts
/// `render(decoded_pixels, decoded_orientation) == render(input, requested)`.
///
/// That one invariant is correct for every valid storage strategy and catches
/// every failure mode:
/// - **Carry** (stored pixels as-authored + tag) — passes.
/// - **Bake** (rotated pixels + `Identity` tag, the only option for a
///   metadata-free format) — also passes; the displayed image is identical.
/// - **Double-application** (rotated pixels *and* the tag) — caught: applying the
///   tag to already-rotated pixels rotates twice, so the render differs.
/// - **Loss** (tag dropped to `Identity`, pixels untouched) — caught: the render
///   is the un-rotated image.
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
    let input = img.pixels();
    for ori in orientations {
        // What a reader should display for an image authored with this orientation.
        let want = render(&input, ori);
        for (name, policy) in policies {
            let meta = Metadata::none().with_orientation(ori);
            let bytes = enc_oneshot(&enc, img, meta, policy)
                .map_err(|e| fail(CHECK, format!("[{name}] encode {ori:?}: {e}")))?;
            let (px, decoded) = dec_oneshot(&dec, &bytes)
                .map_err(|e| fail(CHECK, format!("[{name}] decode {ori:?}: {e}")))?;
            if render(&px, decoded.orientation) != want {
                return Err(fail(
                    CHECK,
                    format!(
                        "[{name}] orientation {ori:?}: displayed image not preserved (decoded orientation = {:?}; \
                         a reader applying the tag would show loss, or a double-rotation if the codec also baked it)",
                        decoded.orientation
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Classify one structural capability: declared support must match observed
/// behavior. Declared + works = fine; declared + failed = lying (missing impl);
/// undeclared + worked = lying (hidden support); undeclared + failed with
/// anything other than `UnsupportedOperation` = wrong error for an absent
/// feature.
fn classify<T, Er>(name: &str, declared: bool, res: Result<T, Er>, v: &mut Vec<String>)
where
    Er: std::error::Error + 'static,
{
    match (declared, res) {
        (true, Ok(_)) => {}
        (true, Err(e)) => v.push(format!(
            "{name}: declared supported, but the operation failed: {e}"
        )),
        (false, Ok(_)) => v.push(format!(
            "{name}: not declared, but the operation succeeded (hidden capability)"
        )),
        (false, Err(e)) => {
            if e.unsupported_operation().is_none() {
                v.push(format!(
                    "{name}: not declared; expected UnsupportedOperation when used, got a different error: {e}"
                ));
            }
        }
    }
}

fn run_push_rows<E>(cfg: &E, img: &TestImage) -> Result<Vec<u8>, E::Error>
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    let mut e = cfg
        .clone()
        .job()
        .with_metadata_policy(Metadata::none(), MetadataPolicy::PreserveExact)
        .encoder()?;
    let strip = e.preferred_strip_height().max(1);
    let mut y = 0;
    while y < img.height {
        let h = strip.min(img.height - y);
        e.push_rows(img.strip(y, h))?;
        y += h;
    }
    Ok(e.finish()?.into_vec())
}

fn run_encode_from<E>(cfg: &E, img: &TestImage) -> Result<Vec<u8>, E::Error>
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    let e = cfg
        .clone()
        .job()
        .with_metadata_policy(Metadata::none(), MetadataPolicy::PreserveExact)
        .encoder()?;
    let rb = img.row_bytes();
    let mut next = 0u32;
    let mut src = |_y: u32, mut buf: PixelSliceMut<'_>| -> usize {
        if next >= img.height {
            return 0;
        }
        let want = buf.rows().min(img.height - next);
        for r in 0..want {
            let s = (next + r) as usize * rb;
            let dst = buf.row_mut(r);
            let n = dst.len().min(rb);
            dst[..n].copy_from_slice(&img.data[s..s + n]);
        }
        next += want;
        want as usize
    };
    Ok(e.encode_from(&mut src)?.into_vec())
}

/// Capability-honesty for the animation encode path, handled separately from
/// [`classify`] because the frame encoder's `Error` can differ from the codec's
/// (`type AnimationFrameEnc = ()` has `Error = UnsupportedOperation`, not the
/// codec's error). `animation_frame_encoder()` returns the *job's* error
/// (`E::Error`, inspectable for `UnsupportedOperation`); the per-frame error is
/// only stringified, so no `Error = E::Error` unification is required.
fn check_animation_encode_honesty<E>(cfg: &E, img: &TestImage, declared: bool, v: &mut Vec<String>)
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    match cfg
        .clone()
        .job()
        .with_loop_count(Some(0))
        .animation_frame_encoder()
    {
        Ok(mut a) => {
            // The encoder was created, so the codec supports animation.
            let frames: Result<(), String> = (|| {
                a.push_frame(img.as_slice(), 100, None)
                    .map_err(|e| e.to_string())?;
                a.push_frame(img.as_slice(), 100, None)
                    .map_err(|e| e.to_string())?;
                a.finish(None).map_err(|e| e.to_string())?;
                Ok(())
            })();
            match (declared, frames) {
                (true, Ok(())) => {}
                (true, Err(e)) => v.push(format!("encode animation: declared supported, but the operation failed: {e}")),
                (false, _) => v.push(
                    "encode animation: not declared, but animation_frame_encoder() succeeded (hidden capability)".into(),
                ),
            }
        }
        Err(e) => {
            if declared {
                v.push(format!("encode animation: declared supported, but animation_frame_encoder() failed: {e}"));
            } else if e.unsupported_operation().is_none() {
                v.push(format!(
                    "encode animation: not declared; expected UnsupportedOperation when used, got a different error: {e}"
                ));
            }
        }
    }
}

fn run_streaming<D: DecoderConfig>(cfg: &D, bytes: &[u8]) -> Result<(), D::Error> {
    let mut sd = cfg
        .clone()
        .job()
        .streaming_decoder(Cow::Borrowed(bytes), &[])?;
    while sd.next_batch()?.is_some() {}
    Ok(())
}

fn run_animation_decode<D: DecoderConfig>(cfg: &D, bytes: &[u8]) -> Result<(), D::Error> {
    let mut ad = cfg
        .clone()
        .job()
        .animation_frame_decoder(Cow::Borrowed(bytes), &[])?;
    while ad.render_next_frame(None)?.is_some() {}
    Ok(())
}

/// Declared capabilities match real behavior.
///
/// For the encode paths (`push_rows`, `encode_from`, animation), the decode paths
/// (streaming, animation), the `lossless` knob, and `cheap_probe`, **both
/// directions** are checked: every declared capability is exercised, and every
/// *undeclared* optional path must decline with
/// [`UnsupportedOperation`](zencodec::UnsupportedOperation) — a codec can't claim a
/// feature it lacks *or* hide one it has. The metadata channels
/// (`icc`/`exif`/`xmp`/`cicp`) are checked bidirectionally **where the decoder can
/// observe them**: a declared channel must survive a `PreserveExact` round trip, and
/// an undeclared one must *not* (a hidden write capability); the
/// encoder-writes-but-decoder-doesn't-read quadrant isn't observable through decode.
/// `native_alpha` is forward-only — a declared RGBA8 round trip must preserve alpha;
/// the no-alpha direction isn't cleanly assertable (a codec may legitimately reject
/// or flatten RGBA input).
///
/// All violations are collected and reported together, so one run names every
/// dishonest flag.
///
/// Not covered: cooperative cancellation (`stop`) — whether a codec honors a
/// triggered token is timing-dependent on small inputs and can't be asserted
/// reliably here; the `lossy` flag, whose effect isn't observable from the
/// bitstream alone; and the pixel-format / resource / tuning flags (`native_gray`,
/// `native_16bit`, `native_f32`, `hdr`, `gain_map`, `enforces_max_pixels` /
/// `enforces_max_memory`, the CICP-carrier flags, and the `effort` / `quality` /
/// `threads` ranges), whose honesty needs format-specific fixtures a generic
/// harness can't supply.
pub fn check_capability_honesty<E, D>(enc: E, dec: D, img: &TestImage) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    const CHECK: &str = "capability_honesty";
    let ec = E::capabilities();
    let dc = D::capabilities();
    let mut v: Vec<String> = Vec::new();

    // --- structural encode paths (both directions) ---
    classify(
        "encode push_rows",
        ec.push_rows(),
        run_push_rows(&enc, img),
        &mut v,
    );
    classify(
        "encode encode_from",
        ec.encode_from(),
        run_encode_from(&enc, img),
        &mut v,
    );
    check_animation_encode_honesty(&enc, img, ec.animation(), &mut v);

    // --- lossless config-knob honesty ---
    // Declared => with_lossless(true) must surface via is_lossless(); undeclared
    // => the no-op default leaves is_lossless() == None.
    let toggled = enc.clone().with_lossless(true).is_lossless();
    match (ec.lossless(), toggled) {
        (true, Some(true)) => {}
        (true, other) => v.push(format!(
            "encode lossless: declared, but with_lossless(true) gives is_lossless() = {other:?} (expected Some(true))"
        )),
        (false, None) => {}
        (false, other) => v.push(format!(
            "encode lossless: not declared, but with_lossless(true) gives is_lossless() = {other:?} (expected None)"
        )),
    }

    // --- a canonical encode for the decode-side checks ---
    match enc_oneshot(&enc, img, Metadata::none(), MetadataPolicy::PreserveExact) {
        Err(e) => v.push(format!(
            "could not produce a canonical encode for decode checks: {e}"
        )),
        Ok(canonical) => {
            classify(
                "decode streaming",
                dc.streaming(),
                run_streaming(&dec, &canonical),
                &mut v,
            );
            classify(
                "decode animation",
                dc.animation(),
                run_animation_decode(&dec, &canonical),
                &mut v,
            );
            if dc.cheap_probe()
                && let Err(e) = dec.clone().job().probe(&canonical)
            {
                v.push(format!(
                    "decode cheap_probe: declared, but probe() failed: {e}"
                ));
            }
        }
    }

    // --- metadata-channel honesty (bidirectional): for each channel the decoder
    //     can read back, a declared encoder channel must survive a PreserveExact
    //     round trip, AND an *undeclared* encoder channel must NOT (a codec can't
    //     hide a write capability it claims not to have). The "encoder writes,
    //     decoder doesn't read" quadrant isn't observable through decode, so it's
    //     left to the codec's own tests. ---
    let rich = Metadata::none()
        .with_icc(fixtures::sample_icc())
        .with_exif(fixtures::rich_exif_le())
        .with_xmp(fixtures::sample_xmp())
        .with_cicp(Cicp::SRGB);
    match enc_oneshot(&enc, img, rich, MetadataPolicy::PreserveExact)
        .and_then(|b| dec_oneshot(&dec, &b))
    {
        Err(e) => v.push(format!("metadata-channel round trip failed: {e}")),
        Ok((_, meta)) => {
            // (channel name, encoder declares write, decoder declares read, survived)
            let channels = [
                ("icc", ec.icc(), dc.icc(), meta.icc_profile.is_some()),
                ("exif", ec.exif(), dc.exif(), meta.exif.is_some()),
                ("xmp", ec.xmp(), dc.xmp(), meta.xmp.is_some()),
                ("cicp", ec.cicp(), dc.cicp(), meta.cicp.is_some()),
            ];
            for (name, enc_writes, dec_reads, survived) in channels {
                if !dec_reads {
                    continue; // not observable through this decoder
                }
                if enc_writes && !survived {
                    v.push(format!(
                        "{name}: declared by encoder+decoder, but did not survive a PreserveExact round trip"
                    ));
                } else if !enc_writes && survived {
                    v.push(format!(
                        "{name}: encoder declared it does NOT support this channel, yet it survived a round trip (hidden capability)"
                    ));
                }
            }
        }
    }

    // --- native_alpha honesty: RGBA8 alpha survives when both ends claim it ---
    if ec.native_alpha() && dc.native_alpha() {
        let rgba = TestImage::rgba8_gradient(img.width.max(2), img.height.max(2));
        match enc_oneshot(&enc, &rgba, Metadata::none(), MetadataPolicy::PreserveExact)
            .and_then(|b| dec_oneshot(&dec, &b))
        {
            Err(e) => v.push(format!(
                "native_alpha: declared, but an RGBA8 round trip failed: {e}"
            )),
            Ok((px, _)) => {
                if px != rgba.pixels() {
                    v.push("native_alpha: declared, but RGBA8 pixels (alpha included) did not round-trip".into());
                }
            }
        }
    }

    if v.is_empty() {
        Ok(())
    } else {
        Err(fail(CHECK, v.join("; ")))
    }
}

// ===========================================================================
// Error-envelope conformance (the `At<CodecError>` Pattern-B contract)
// ===========================================================================

/// Statically assert a codec returns the shared **`At<CodecError>` envelope**
/// from every encode/decode trait boundary — the Pattern-B error contract.
///
/// A zero-cost compile-time gate: it takes no arguments and runs no code. A codec
/// invokes it once — `assert_uses_codec_error_envelope::<MyEncoderConfig, MyDecoderConfig>()`
/// — and the bounds below make it a **compile error** for any codec whose
/// `type Error` is its own native enum instead of `At<CodecError>` (Pattern A).
///
/// Why it matters: a native-enum `type Error` classifies only on the *typed*
/// path. The moment it is erased — the `BoxedError` every `Dyn*` dispatch method
/// produces, an `anyhow::Error`, a mapped wrapper — all you hold is a `dyn Error`,
/// and you cannot downcast that to a `dyn CategorizedError`; the
/// [`ErrorCategory`](zencodec::ErrorCategory) and codec name are gone.
/// `At<CodecError>` is one concrete type, so it survives any erasure by a
/// downcast. [`check_decode_error_envelope`] is the runtime companion that proves
/// the category actually *flows* through erasure.
///
/// It bounds the config, job, and leaf executor on both sides. The optional stub
/// associated types (`AnimationFrameEnc`, `StreamDec`, `AnimationFrameDec`) are
/// intentionally *not* bound: a still-only codec legitimately uses `()` /
/// [`Unsupported`](zencodec::Unsupported), whose `Error` is
/// [`UnsupportedOperation`](zencodec::UnsupportedOperation), not the envelope.
///
/// Not part of [`check_all`] — the testkit's own [`reference`](mod@reference)
/// codec is a deliberate Pattern-A foil, so this is opt-in for codecs that have
/// adopted the envelope.
pub fn assert_uses_codec_error_envelope<E, D>()
where
    E: EncoderConfig<Error = At<CodecError>>,
    E::Job: EncodeJob<Error = At<CodecError>>,
    <E::Job as EncodeJob>::Enc: Encoder<Error = At<CodecError>>,
    D: DecoderConfig<Error = At<CodecError>>,
    for<'a> D::Job<'a>: DecodeJob<'a, Error = At<CodecError>>,
    for<'a> <D::Job<'a> as DecodeJob<'a>>::Dec: Decode<Error = At<CodecError>>,
{
}

/// A codec's [`ErrorCategory`](zencodec::ErrorCategory) **and** originating codec
/// name survive dyn-dispatch type erasure — the runtime half of the Pattern-B
/// contract.
///
/// Drives the decoder through the dyn boundary (`&dyn DynDecoderConfig` →
/// [`dyn_job`](zencodec::decode::DynDecoderConfig::dyn_job) → `probe`) on input
/// the codec rejects, so the typed `At<CodecError>` is erased to the `BoxedError`
/// a generic pipeline actually holds. It then recovers the envelope from that
/// `Box<dyn Error>` and asserts both
/// [`error_category`](zencodec::CodecErrorExt::error_category) and the
/// [`codec`](zencodec::CodecError::codec) name come back.
///
/// A Pattern-A codec (native-enum `type Error`) **fails** here: its category may
/// exist on the typed value, but it is unrecoverable once erased — which is the
/// whole point of the envelope, and what this check exists to catch.
/// [`assert_uses_codec_error_envelope`] is the compile-time companion; this proves
/// the category genuinely propagates. Not part of [`check_all`] (see that note).
///
/// `malformed` must be bytes this codec rejects — any non-decodable input, a short
/// garbage buffer is usually enough. If the codec *accepts* them, the check says so
/// rather than passing silently.
pub fn check_decode_error_envelope<D>(dec: D, malformed: &[u8]) -> Conformance
where
    D: DecoderConfig + 'static,
{
    const CHECK: &str = "decode_error_envelope";
    let dyn_cfg: &dyn DynDecoderConfig = &dec;
    let erased = match dyn_cfg.dyn_job().probe(malformed) {
        Err(e) => e,
        Ok(_) => {
            return Err(fail(
                CHECK,
                "probe() accepted the supplied `malformed` input — pass bytes this codec rejects, \
                 so an error is actually produced to inspect",
            ));
        }
    };
    if erased.error_category().is_none() {
        return Err(fail(
            CHECK,
            format!(
                "ErrorCategory did not survive dyn-dispatch erasure: the decoder's `type Error` is \
                 not `At<CodecError>` (Pattern A — a native error enum erases to a bare `dyn Error`, \
                 which cannot be downcast to recover the category). Switch the zencodec trait impls \
                 to `type Error = At<CodecError>`. Erased error was: {erased}"
            ),
        ));
    }
    if erased.codec_error().and_then(CodecError::codec).is_none() {
        return Err(fail(
            CHECK,
            "the CodecError envelope survived erasure but carries no codec name — make the native \
             error's `CategorizedError::codec_name()` return `Some(\"<codec>\")`",
        ));
    }
    Ok(())
}

/// Run every conformance check with sensible default inputs, returning the first
/// failure. The one-call entry point for a codec's test suite; for control over
/// image sizes or animation frames, call the individual `check_*` functions.
pub fn check_all<E, D>(enc: E, dec: D) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    let img = TestImage::rgba8_gradient(40, 24);
    check_pixel_roundtrip(enc.clone(), dec.clone(), &img)?;
    check_cross_path_pixel_equivalence(enc.clone(), dec.clone(), &img)?;
    check_orientation_roundtrip(enc.clone(), dec.clone(), &img)?;
    check_metadata_no_leak(enc.clone(), dec.clone(), &img)?;
    check_capability_honesty(enc.clone(), dec.clone(), &img)?;
    let frames = [
        TestImage::rgba8_gradient_seeded(24, 16, 0),
        TestImage::rgba8_gradient_seeded(24, 16, 60),
        TestImage::rgba8_gradient_seeded(24, 16, 120),
    ];
    check_animation_cross_path_equivalence(enc, dec, &frames)?;
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

    /// The full reference declares (and honors) every capability, so the
    /// true-direction branches must all pass.
    #[test]
    fn reference_capability_honesty() {
        let (e, d) = ref_codecs();
        check_capability_honesty(e, d, &TestImage::rgba8_gradient(12, 9)).unwrap();
    }

    /// A real reference-codec operation cancelled via its `Stop` token surfaces
    /// an error whose [`ErrorCategory`] is `Cancelled` — classified through the
    /// codec's opt-in [`CategorizedError`] impl, no concrete-enum match needed
    /// (issue #99). This is what lets a server map a cancelled request to HTTP
    /// 499 instead of treating it as malformed input.
    #[test]
    fn reference_cancellation_is_classifiable() {
        use enough::{Stop, StopReason};
        use zencodec::{CategorizedError, CodecErrorExt, ErrorCategory};

        // A token already in the stopped state — the codec's first check fires.
        struct Cancelled;
        impl Stop for Cancelled {
            fn check(&self) -> Result<(), StopReason> {
                Err(StopReason::Cancelled)
            }
        }

        let img = TestImage::rgba8_gradient(8, 8);
        let mut a = ReferenceEncoderConfig::new()
            .job()
            .animation_frame_encoder()
            .expect("animation encoder");
        let err = a
            .push_frame(img.as_slice(), 40, Some(&Cancelled))
            .expect_err("a fired stop token must cancel the push");

        // Classify it the way a generic consumer would, with no knowledge of RefError:
        assert_eq!(err.category(), ErrorCategory::Cancelled);
        // ...and it must NOT be mistaken for a limit or an unsupported operation.
        assert!(err.limit_exceeded().is_none());
        assert!(err.unsupported_operation().is_none());
    }

    /// The envelope pattern (`type Error = At<CodecError>`, the `minimal` codec)
    /// lets a generic consumer recover the [`ErrorCategory`] *after dyn dispatch
    /// erases the concrete error to `BoxedError`* — the case typed-only
    /// classification (issue #99) can't reach, because the erased value is a
    /// `dyn Error`, not a `dyn CategorizedError`. This is what the envelope buys.
    #[test]
    fn minimal_envelope_category_survives_dyn_erasure() {
        use zencodec::decode::DynDecoderConfig;
        use zencodec::{CodecError, CodecErrorExt, ErrorCategory};

        // Drive the minimal codec entirely through the dyn surface; its
        // `At<CodecError>` is erased to `Box<dyn Error>` by the shim.
        let cfg = MinimalDecoderConfig::new();
        let dyn_min: &dyn DynDecoderConfig = &cfg;
        let erased = dyn_min
            .dyn_job()
            .probe(b"not a ZCR1 header")
            .expect_err("malformed header must fail");
        // A consumer holding only `Box<dyn Error>` recovers the category — and
        // the originating codec name, so it can tell codecs apart generically.
        assert_eq!(erased.error_category(), Some(ErrorCategory::MalformedImage));
        assert_eq!(
            erased.codec_error().and_then(CodecError::codec),
            Some(crate::reference::MINIMAL_CODEC_NAME)
        );

        // Contrast: the reference codec (`type Error = RefError`) classifies fine
        // on the *typed* path, but once dyn dispatch erases it there is no shared
        // concrete type to downcast to — the gap the envelope closes.
        let ref_cfg = ReferenceDecoderConfig;
        let dyn_ref: &dyn DynDecoderConfig = &ref_cfg;
        let erased_ref = dyn_ref
            .dyn_job()
            .probe(b"not a ZCR1 header")
            .expect_err("malformed header must fail");
        assert_eq!(erased_ref.error_category(), None);
        assert!(erased_ref.codec_error().is_none());
    }

    /// On the typed path, `At<CodecError>` answers the category two ways — the
    /// total inherent `category()` on the envelope and the `Option` recovery —
    /// and carries a location trace (the `From` bridge starts it).
    #[test]
    fn minimal_envelope_typed_path_and_trace() {
        use std::borrow::Cow;
        use zencodec::decode::{DecodeJob, DecoderConfig};
        use zencodec::{CodecErrorExt, ErrorCategory};

        let err = MinimalDecoderConfig::new()
            .job()
            .decoder(Cow::Borrowed(b"not a ZCR1 header"), &[])
            .expect_err("malformed header must fail");
        // Total category + codec name via the concrete envelope:
        assert_eq!(err.error().category(), ErrorCategory::MalformedImage);
        assert_eq!(
            err.error().codec(),
            Some(crate::reference::MINIMAL_CODEC_NAME)
        );
        // Same category via the generic Option recovery:
        assert_eq!(err.error_category(), Some(ErrorCategory::MalformedImage));
        // The trace was started by the bridge's `.start_at()`.
        let dbg = format!("{err:?}");
        assert!(dbg.contains("at "), "expected a trace frame: {dbg}");
    }

    /// The minimal codec declares every optional capability *false* and rejects
    /// those paths, so the false-direction branches must all pass.
    #[test]
    fn minimal_capability_honesty() {
        check_capability_honesty(
            MinimalEncoderConfig::new(),
            MinimalDecoderConfig::new(),
            &TestImage::rgba8_gradient(12, 9),
        )
        .unwrap();
    }

    /// The minimal codec still round-trips pixels one-shot and cleanly declines
    /// every optional path with `UnsupportedOperation`.
    #[test]
    fn minimal_one_shot_roundtrip_and_pixels() {
        let (e, d) = (MinimalEncoderConfig::new(), MinimalDecoderConfig::new());
        check_pixel_roundtrip(e, d, &TestImage::rgb8_gradient(10, 7)).unwrap();
    }

    // ---- error-envelope conformance (the `At<CodecError>` Pattern-B contract) ----

    /// The envelope exemplar (`minimal`, `type Error = At<CodecError>`) carries its
    /// category AND codec name through dyn-dispatch erasure to `BoxedError`.
    #[test]
    fn minimal_decode_error_envelope_survives_erasure() {
        // 16 bytes of garbage: fails `parse_header` (short / bad magic) →
        // RefError::Invalid → At<CodecError>{MalformedImage, "zencodec-testkit/minimal"}.
        check_decode_error_envelope(MinimalDecoderConfig::new(), &[0xABu8; 16]).unwrap();
    }

    /// The negative case proves the check has teeth. `reference` is Pattern A
    /// (`type Error = RefError`); its RefError *is* `CategorizedError`, but that
    /// category is unrecoverable once erased to `BoxedError`, so the check must
    /// FAIL — exactly the loss the envelope prevents. (Same underlying RefError as
    /// `minimal` above: the only difference is the envelope `type Error`.)
    #[test]
    fn reference_pattern_a_fails_the_envelope_check() {
        let err = check_decode_error_envelope(ReferenceDecoderConfig, &[0xABu8; 16])
            .expect_err("Pattern A must fail the envelope-survival check");
        assert_eq!(err.check, "decode_error_envelope");
        assert!(
            err.detail.contains("Pattern A"),
            "the failure should name the Pattern-A cause: {}",
            err.detail
        );
    }

    /// The compile-time gate accepts the envelope codec. A Pattern-A codec here
    /// would fail to *compile* (`type Error` ≠ `At<CodecError>`); that direction
    /// can't live in a normal test, so the runtime check above covers it.
    #[test]
    fn minimal_satisfies_the_static_envelope_assertion() {
        assert_uses_codec_error_envelope::<MinimalEncoderConfig, MinimalDecoderConfig>();
    }

    /// The classifier underpinning the honesty check must flag both kinds of lie
    /// (declared-but-broken, and works-but-undeclared) while accepting an honest
    /// decline (undeclared + `UnsupportedOperation`).
    #[test]
    fn detector_catches_a_lie() {
        // declared = true, but the operation failed → lie (missing impl).
        let mut v = Vec::new();
        classify(
            "x",
            true,
            Err::<(), _>(RefError::Invalid("boom".into())),
            &mut v,
        );
        assert_eq!(v.len(), 1, "declared + failed must be flagged");

        // declared = false, but the operation succeeded → lie (hidden capability).
        let mut v = Vec::new();
        classify("y", false, Ok::<(), RefError>(()), &mut v);
        assert_eq!(v.len(), 1, "undeclared + worked must be flagged");

        // declared = false, and it declined with UnsupportedOperation → honest.
        let mut v = Vec::new();
        let declined = Err::<(), _>(RefError::Unsupported(
            zencodec::UnsupportedOperation::RowLevelEncode,
        ));
        classify("z", false, declined, &mut v);
        assert!(v.is_empty(), "undeclared + UnsupportedOperation is honest");
    }

    #[test]
    fn reference_orientation_roundtrip() {
        let (e, d) = ref_codecs();
        // Non-square so an axis-swap bug (Rotate90/270/transpose) shows as a
        // dimension or pixel mismatch.
        check_orientation_roundtrip(e, d, &TestImage::rgba8_gradient(9, 6)).unwrap();
    }

    /// `render` must match EXIF semantics: identity is a no-op, Rotate90 swaps
    /// axes, and self-inverse transforms applied twice return the original.
    #[test]
    fn render_matches_orientation_semantics() {
        let p = TestImage::rgba8_gradient(3, 2).pixels();
        assert_eq!(render(&p, Orientation::Identity), p, "identity is a no-op");

        let r90 = render(&p, Orientation::Rotate90);
        assert_eq!((r90.width, r90.rows), (2, 3), "Rotate90 swaps axes");

        // Self-inverse transforms applied twice are the identity.
        let r180 = render(&p, Orientation::Rotate180);
        assert_eq!(
            render(&r180, Orientation::Rotate180),
            p,
            "Rotate180∘Rotate180 == id"
        );
        let fh = render(&p, Orientation::FlipH);
        assert_eq!(render(&fh, Orientation::FlipH), p, "FlipH∘FlipH == id");
        let fv = render(&p, Orientation::FlipV);
        assert_eq!(render(&fv, Orientation::FlipV), p, "FlipV∘FlipV == id");
    }

    #[test]
    fn reference_animation_cross_path() {
        let (e, d) = ref_codecs();
        // Distinct frames, so a frame-ordering or canvas-aliasing bug is visible.
        let frames = [
            TestImage::rgba8_gradient_seeded(10, 8, 0),
            TestImage::rgba8_gradient_seeded(10, 8, 50),
            TestImage::rgba8_gradient_seeded(10, 8, 130),
        ];
        check_animation_cross_path_equivalence(e, d, &frames).unwrap();
    }

    #[test]
    fn minimal_animation_cross_path_skipped() {
        // Minimal declares animation=false, so the check is not applicable and passes.
        let frames = [TestImage::rgba8_gradient_seeded(8, 8, 0)];
        check_animation_cross_path_equivalence(
            MinimalEncoderConfig::new(),
            MinimalDecoderConfig::new(),
            &frames,
        )
        .unwrap();
    }

    #[test]
    fn reference_check_all() {
        let (e, d) = ref_codecs();
        check_all(e, d).unwrap();
    }

    #[test]
    fn minimal_check_all() {
        check_all(MinimalEncoderConfig::new(), MinimalDecoderConfig::new()).unwrap();
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

    // ---- whereat trace: lines preserved up the stack + crate boundaries ----

    /// A whereat trace preserves every `.at()` hop's file:line as an error climbs
    /// the stack — and through the `BoxedError` erasure a dyn pipeline performs —
    /// so a diagnostic can point at every layer, not just the last. Each frame is
    /// attributable to its source file (hence its crate: `Location::file()` embeds
    /// the crate directory), so the boundary between codec-internal frames and the
    /// caller's is visible in the trace.
    #[test]
    fn error_trace_preserves_lines_all_the_way_up() {
        use std::borrow::Cow;
        use zencodec::decode::{DecodeJob, DecoderConfig};

        // A real codec error, located inside the codec module (the `?` site in
        // minimal.rs) by the bridge's track-caller `start_at` — exactly one frame.
        let origin: At<CodecError> = MinimalDecoderConfig::new()
            .job()
            .decoder(Cow::Borrowed(b"not a ZCR1 header"), &[])
            .expect_err("malformed header must fail");
        assert_eq!(
            origin.frame_count(),
            1,
            "the codec locates its error exactly once"
        );
        let f0 = origin
            .frames()
            .next()
            .and_then(|f| f.location())
            .expect("origin frame has a location");
        let origin_line = f0.line();
        assert!(
            f0.file().contains("minimal.rs"),
            "origin frame is attributed to the codec module, not the caller: {}",
            f0.file()
        );

        // Climb the stack: each `.at()` is a distinct source line — the layers an
        // error crosses on the way up (codec boundary → pipeline → app).
        let l1 = line!() + 1;
        let hop1 = origin.at();
        let l2 = line!() + 1;
        let climbed = hop1.at();

        // Every hop is its own frame, oldest-first, none collapsed or lost.
        let locs: Vec<(String, u32)> = climbed
            .frames()
            .filter_map(|f| f.location().map(|l| (l.file().to_string(), l.line())))
            .collect();
        assert_eq!(locs.len(), 3, "origin + 2 hops = 3 frames, none lost");
        assert_eq!(
            locs[0].1, origin_line,
            "origin line preserved at the bottom of the trace"
        );
        assert!(locs[0].0.contains("minimal.rs"));
        assert_eq!(
            (locs[1].0.contains("lib.rs"), locs[1].1),
            (true, l1),
            "hop 1 file+line preserved"
        );
        assert_eq!(
            (locs[2].0.contains("lib.rs"), locs[2].1),
            (true, l2),
            "hop 2 file+line preserved"
        );
        assert_ne!(
            locs[0].0, locs[1].0,
            "codec-origin and caller frames are distinguishable by file (the crate-boundary signal)"
        );

        // The whole trace survives dyn-dispatch erasure to Box<dyn Error> and back:
        // no frame, no line is lost when the concrete type is hidden.
        let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(climbed);
        let recovered = boxed
            .downcast_ref::<At<CodecError>>()
            .expect("downcast the erased envelope");
        let after: Vec<(String, u32)> = recovered
            .frames()
            .filter_map(|f| f.location().map(|l| (l.file().to_string(), l.line())))
            .collect();
        assert_eq!(
            after, locs,
            "every frame + line survives the BoxedError round-trip"
        );
    }

    /// whereat records crate boundaries when an error crosses them
    /// ([`At::at_crate`](whereat::At::at_crate) — the mechanism
    /// `whereat::define_at_crate_info!()` wires into zencodec and every codec). The
    /// rendered trace shows the crate transition, so a reader can see *which crate*
    /// each leg of the propagation happened in.
    #[test]
    fn error_trace_marks_crate_boundaries() {
        use whereat::AtCrateInfo;

        // Two crates the error notionally crosses (codec → app).
        static CODEC_CRATE: AtCrateInfo = AtCrateInfo::builder().name("demo-codec").build();
        static APP_CRATE: AtCrateInfo = AtCrateInfo::builder().name("demo-app").build();

        let err = At::wrap(CodecError::new(
            Some("demo-codec"),
            zencodec::ErrorCategory::MalformedImage,
        ))
        .at_crate(&CODEC_CRATE)
        .at()
        .at_crate(&APP_CRATE)
        .at();

        // Both boundaries are recorded as `AtContext::Crate` markers; the
        // full-trace display walks them and names each crate at the transition.
        let full = err.full_trace().to_string();
        assert!(
            full.contains("demo-codec") && full.contains("demo-app"),
            "both crate boundaries should be recorded + visible in the full trace:\n{full}"
        );
    }
}
