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
//! - [`check_color_context_consistency`] — whatever `ColorContext` a decoder
//!   attaches to its output buffers is class-valid (an ICC only rides a layout
//!   its device class describes) and identical across the one-shot, streaming,
//!   and animation (borrowed + owned) paths — strip/scratch buffers are where a
//!   context silently dies. Lenient about *whether* one is attached;
//!   [`check_color_context_attached`] is the strict, opt-in positive direction
//!   (a decoder that read colour back must attach it).
//! - [`check_color_authority_spec`] — the `ColorAuthority` a decoder names in
//!   `source_color` is the one its format's spec assigns for the colour fields it
//!   read back (PNG/JXL/HEIC: CICP outranks ICC; AVIF: MIAF order; the rest:
//!   ICC). [`check_source_color_authority`] is the unit form for a codec's own
//!   fixtures, [`expected_color_authority`] the answer key.
//! - [`check_native_hdr_roundtrip`] — when both ends declare `hdr`, BT.2100 PQ
//!   and HLG survive a `PreserveExact` round trip: the CICP, the HDR envelope
//!   (content light level + mastering display), 16-bit pixels where both ends
//!   are natively 16-bit, and the decoded buffer is *labelled* PQ/HLG rather
//!   than sRGB. Phase 0 of the gain-map/HDR delivery scope.
//! - [`check_gain_map_roundtrip`] — the gain-map encode/decode contract: an
//!   undeclared encoder rejects `with_gain_map_pixels` loudly with
//!   `UnsupportedOperation::GainMapEncode`; a declared one embeds 1- and
//!   3-channel maps (forward and backward direction) that a declared decoder
//!   reports from `probe()`, surfaces only on request (`Components` /
//!   `with_extract_gain_map`), never under the default `BaseOnly`, falls back
//!   honestly on `ReconstructHdr` without `reconstructs_hdr`, and that survive
//!   a decode → `with_gain_map_pixels` transcode. Phases 1, 2 (test matrix) and
//!   4 (the testkit cross-path check) of that scope.
//! - [`check_fidelity_honesty`] — `resolved_target_fidelity()` tells the truth:
//!   a declared `lossless` honours a `Lossless` request byte-exactly, a codec
//!   without `lossy` never reports `Lossy` (it promotes and says so), a codec
//!   with `lossy` never answers a lossy request with `Lossless`, and the legacy
//!   `is_lossless()` agrees. The cross-codec `Fidelity` contract.
//! - [`check_capability_honesty`] — every declared capability works and every
//!   undeclared optional path cleanly returns
//!   [`UnsupportedOperation`]. Both directions for
//!   the structural paths and (where the decoder can observe them) the metadata
//!   channels, so a codec can't claim a feature it lacks *or* hide one it has; see
//!   the fn docs for the exact per-flag scope.
//! - [`check_decode_error_envelope`] / [`assert_uses_codec_error_envelope`] — a
//!   codec's [`ErrorCategory`] and codec name survive
//!   dyn-dispatch type erasure (the `At<CodecError>` envelope contract). Opt-in,
//!   and *not* in [`check_all`]: only for codecs that return the envelope
//!   `type Error` (the testkit's own [`reference`](mod@reference) is a Pattern-A
//!   foil that deliberately fails it).
//! - [`check_decode_truncation_series`] — a truncated (incomplete) input must
//!   categorize as an *incomplete-input* category (never an `Internal` 5xx, an
//!   `OutOfMemory`, an `Io` error, or a caller-fault), and must never panic or
//!   silently decode a truncated header. Enforces the one part of the taxonomy
//!   that is broadly distinguishable. Opt-in, *not* in [`check_all`]; on the
//!   dyn-erased path a Pattern-A codec fails it the same way the envelope check
//!   does (the allowed/denied category policy is documented on
//!   [`check_decode_truncation_series`]).
//!
//! [`check_all`] runs them all with default inputs — the one-call entry point.
//!
//! The [`reference`](mod@reference) module ships a faithful codec (declares and
//! honors every capability) the harness is validated against — it doubles as a
//! worked example; an internal `minimal` codec (every optional capability
//! declared false) validates the false-direction branches.
//!
//! [`EncoderConfig`]: zencodec::encode::EncoderConfig
//! [`DecoderConfig`]: zencodec::decode::DecoderConfig

use std::borrow::Cow;
use std::sync::Arc;

use whereat::At;
use zencodec::CodecErrorExt;
use zencodec::decode::{
    AnimationFrameDecoder, Decode, DecodeJob, DecodeOutput, DecodeRowSink, DecoderConfig,
    DynDecoderConfig, SinkError, StreamingDecode,
};
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, Encoder, EncoderConfig, Fidelity};
use zencodec::exif::Exif;
use zencodec::gainmap::{
    DecodedGainMap, GainMapChannel, GainMapInfo, GainMapParams, GainMapPresence, GainMapRender,
    GainMapSource,
};
use zencodec::{
    Cicp, CodecError, ColorAuthority, ContentLightLevel, ErrorCategory, ImageFormat,
    MasteringDisplay, Metadata, MetadataFields, MetadataPolicy, Orientation, SourceColor,
    UnsupportedOperation,
};
use zenpixels::{
    ColorContext, PixelBuffer, PixelDescriptor, PixelSlice, PixelSliceMut, TransferFunction,
};

pub(crate) mod fixtures;
/// The false-direction exemplar codec — consumed only by this crate's own
/// tests, so it is compiled only with them.
#[cfg(test)]
pub(crate) mod minimal;
pub mod reference;

#[cfg(test)]
pub(crate) use minimal::{MinimalDecoderConfig, MinimalEncoderConfig};
pub use reference::{
    RefError, ReferenceDecoderConfig, ReferenceEncoderConfig, ReferenceZcrDecoderConfig, ZCR_FORMAT,
};

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

    /// A 16-bit RGB gradient carrying `desc`'s colour labelling (a U16 RGB
    /// descriptor such as `RGB16_BT2100_PQ`). Samples span the full 16-bit range
    /// with distinct high and low bytes, so a byte-order bug or an 8-bit
    /// truncation shows up as a pixel diff.
    pub(crate) fn rgb16_gradient(width: u32, height: u32, desc: PixelDescriptor) -> Self {
        assert!(width > 0 && height > 0, "test image must be non-empty");
        assert_eq!(
            desc.bytes_per_pixel(),
            6,
            "rgb16_gradient needs a U16 RGB descriptor"
        );
        let mut data = vec![0u8; width as usize * height as usize * 6];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let p = (y * width as usize + x) * 6;
                let r = (x * 1031 + y * 517) as u16;
                let g = (x * 257 + y * 3079) as u16;
                let b = ((x ^ y) * 4111 + 12345) as u16;
                data[p..p + 2].copy_from_slice(&r.to_ne_bytes());
                data[p + 2..p + 4].copy_from_slice(&g.to_ne_bytes());
                data[p + 4..p + 6].copy_from_slice(&b.to_ne_bytes());
            }
        }
        Self {
            width,
            height,
            desc,
            data,
        }
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

fn encode_animation<E>(cfg: &E, frames: &[TestImage], meta: Metadata) -> Result<Vec<u8>, String>
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    let mut a = cfg
        .clone()
        .job()
        .with_metadata_policy(meta, MetadataPolicy::PreserveExact)
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

    let bytes = encode_animation(&enc, frames, Metadata::none())
        .map_err(|e| fail(CHECK, format!("encode: {e}")))?;
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
/// and **capture timestamps** ([`Exif::has_datetimes`]). Device IDs, camera owner,
/// image identity and UTC offsets are checked separately when their descriptive
/// siblings are retained. An emitted EXIF blob that
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
        (
            "Web + camera/timestamps",
            MetadataPolicy::Custom(
                MetadataPolicy::Web.fields().with_exif(
                    zencodec::ExifPolicy::ATTRIBUTED_ORIENTATION
                        .with_camera(zencodec::Retention::Keep)
                        .with_datetimes(zencodec::Retention::Keep),
                ),
            ),
        ),
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
                // A broad camera/date check is insufficient when descriptive
                // camera tags or timestamps are retained but identifiers are not.
                for (name, present, wanted) in [
                    (
                        "device identifiers",
                        dx.has_device_ids(),
                        want.as_ref().is_some_and(Exif::has_device_ids),
                    ),
                    (
                        "camera owner",
                        dx.has_camera_owner(),
                        want.as_ref().is_some_and(Exif::has_camera_owner),
                    ),
                    (
                        "image identity",
                        dx.has_image_unique_id(),
                        want.as_ref().is_some_and(Exif::has_image_unique_id),
                    ),
                    (
                        "UTC offsets",
                        dx.has_time_offsets(),
                        want.as_ref().is_some_and(Exif::has_time_offsets),
                    ),
                ] {
                    if present && !wanted {
                        return Err(fail(
                            check,
                            format!(
                                "[{policy}] {name} in output EXIF but the policy dropped it (privacy leak)"
                            ),
                        ));
                    }
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

// ===========================================================================
// Native HDR conformance (zencodec#24, Phase 0)
// ===========================================================================

/// The HDR envelope the native-HDR check encodes with — values a dropped or
/// zeroed field can't be mistaken for.
fn hdr_envelope() -> (ContentLightLevel, MasteringDisplay) {
    (
        ContentLightLevel::new(1000, 400),
        MasteringDisplay::HDR10_REFERENCE,
    )
}

/// Container fixed-point tolerance: |Δ| ≤ 2e-5 for sub-unit values (xy
/// chromaticities are stored in 1/50000 units by AVIF/HEIF, luminances in
/// 1/10000), relative 1e-5 above 1.0 (peak luminance).
fn hdr_close(got: f32, want: f32) -> bool {
    let tol = if want.abs() < 1.0 {
        2e-5
    } else {
        want.abs() * 1e-5
    };
    (got - want).abs() <= tol
}

fn mastering_display_matches(got: &MasteringDisplay, want: &MasteringDisplay) -> bool {
    got.primaries_xy
        .iter()
        .zip(&want.primaries_xy)
        .all(|(g, w)| hdr_close(g[0], w[0]) && hdr_close(g[1], w[1]))
        && hdr_close(got.white_point_xy[0], want.white_point_xy[0])
        && hdr_close(got.white_point_xy[1], want.white_point_xy[1])
        && hdr_close(got.max_luminance, want.max_luminance)
        && hdr_close(got.min_luminance, want.min_luminance)
}

/// Native HDR survives a round trip: for BT.2100 **PQ** and **HLG**, the CICP,
/// the HDR envelope (content light level + mastering display), the pixels, and
/// the decoded buffer's HDR *labelling* all come back from a `PreserveExact`
/// encode → decode.
///
/// Skipped (`Ok`) unless **both** ends declare
/// [`hdr`](zencodec::encode::EncodeCapabilities::hdr). For each transfer:
///
/// - encodes `Cicp::BT2100_PQ` / `Cicp::BT2100_HLG` with `ContentLightLevel(1000, 400)`
///   and `MasteringDisplay::HDR10_REFERENCE`, using a 16-bit RGB gradient when both
///   ends declare `native_16bit` (the 8-bit RGB gradient otherwise — the
///   signaling path is exercised either way);
/// - the decoded pixels must equal the input byte-for-byte;
/// - the decoded descriptor must carry the HDR transfer (`Pq` / `Hlg`) — a PQ
///   decode labelled sRGB is exactly the mislabel the descriptor exists to
///   prevent — and on the 16-bit path must equal `RGB16_BT2100_PQ` / `_HLG`;
/// - when both ends declare the `cicp` channel, `Metadata.cicp` must equal the
///   input;
/// - `content_light_level` must survive exactly and `mastering_display` within
///   container fixed-point precision (2e-5 on xy, 1e-5 relative on luminance).
///   `hdr` on both ends means the envelope reaches the encoder and comes back —
///   silently dropping it is the "gain-map → native-HDR transcode loses the
///   envelope" hazard in `docs/correctness-model.md`.
///
/// This is Phase 0 of the gain-map / HDR delivery scope
/// (`docs/gainmap-pipeline-scope-2026-06-08.md`): prove native HDR works with
/// zero new API before any gain-map encode surface is added. Part of
/// [`check_all`].
pub fn check_native_hdr_roundtrip<E, D>(enc: E, dec: D) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "native_hdr_roundtrip";
    let (ec, dc) = (E::capabilities(), D::capabilities());
    if !ec.hdr() || !dc.hdr() {
        return Ok(()); // not applicable to an SDR-only codec
    }
    let sixteen = ec.native_16bit() && dc.native_16bit();
    let (cll, md) = hdr_envelope();
    let cases = [
        (
            "PQ",
            Cicp::BT2100_PQ,
            PixelDescriptor::RGB16_BT2100_PQ,
            TransferFunction::Pq,
        ),
        (
            "HLG",
            Cicp::BT2100_HLG,
            PixelDescriptor::RGB16_BT2100_HLG,
            TransferFunction::Hlg,
        ),
    ];
    for (name, cicp, desc16, transfer) in cases {
        let img = if sixteen {
            TestImage::rgb16_gradient(20, 14, desc16)
        } else {
            TestImage::rgb8_gradient(20, 14)
        };
        let meta = Metadata::none()
            .with_cicp(cicp)
            .with_content_light_level(cll)
            .with_mastering_display(md);
        let bytes = enc_oneshot(&enc, &img, meta, MetadataPolicy::PreserveExact)
            .map_err(|e| fail(CHECK, format!("[{name}] encode: {e}")))?;
        let out =
            dec_output(&dec, &bytes).map_err(|e| fail(CHECK, format!("[{name}] decode: {e}")))?;

        let got_px = grab(out.pixels());
        let want_px = img.pixels();
        if (got_px.width, got_px.rows) != (want_px.width, want_px.rows)
            || got_px.bytes != want_px.bytes
        {
            return Err(fail(
                CHECK,
                format!(
                    "[{name}] decoded {}-bit pixels differ from the input",
                    if sixteen { 16 } else { 8 }
                ),
            ));
        }
        let desc = got_px.desc;
        if desc.transfer() != transfer {
            return Err(fail(
                CHECK,
                format!(
                    "[{name}] the decoded buffer is labelled {:?}, not {transfer:?} — the descriptor \
                     must describe the current pixels; stamp it from the file's CICP",
                    desc.transfer()
                ),
            ));
        }
        if sixteen && desc != desc16 {
            return Err(fail(
                CHECK,
                format!("[{name}] 16-bit decode descriptor is {desc:?}, expected {desc16:?}"),
            ));
        }

        let got = out.metadata();
        if ec.cicp() && dc.cicp() && got.cicp != Some(cicp) {
            return Err(fail(
                CHECK,
                format!(
                    "[{name}] CICP {cicp:?} did not survive PreserveExact (decoded {:?})",
                    got.cicp
                ),
            ));
        }
        match got.content_light_level {
            Some(c) if c == cll => {}
            other => {
                return Err(fail(
                    CHECK,
                    format!(
                        "[{name}] content_light_level {cll:?} did not survive PreserveExact \
                         (decoded {other:?}) — hdr is declared on both ends, so the HDR envelope \
                         must reach the encoder and come back"
                    ),
                ));
            }
        }
        match got.mastering_display {
            Some(m) if mastering_display_matches(&m, &md) => {}
            other => {
                return Err(fail(
                    CHECK,
                    format!(
                        "[{name}] mastering_display {md:?} did not survive PreserveExact within \
                         container precision (decoded {other:?})"
                    ),
                ));
            }
        }
    }
    Ok(())
}

// ===========================================================================
// Gain-map encode / decode conformance (zencodec#24, Phases 1/2/4)
// ===========================================================================

/// One cell of the gain-map test matrix.
#[derive(Clone, Copy)]
struct GainMapCase {
    name: &'static str,
    channels: u8,
    /// ISO 21496-1 backward direction: HDR base + SDR gain map.
    backward: bool,
}

/// The matrix the scope audit asked for: 1-channel (the UltraHDR norm),
/// 3-channel (iOS 18 mainstream), and a backward-direction file (Android 16
/// ships HDR-base + SDR-gain-map).
const GAIN_MAP_CASES: [GainMapCase; 3] = [
    GainMapCase {
        name: "1ch forward",
        channels: 1,
        backward: false,
    },
    GainMapCase {
        name: "3ch forward",
        channels: 3,
        backward: false,
    },
    GainMapCase {
        name: "1ch backward",
        channels: 1,
        backward: true,
    },
];

/// Gain-map dimensions: deliberately *not* the base's (20×14) — sub-resolution
/// maps are the ecosystem norm, and a codec that assumes base geometry breaks.
const GAIN_MAP_W: u32 = 10;
const GAIN_MAP_H: u32 = 7;

/// The ISO 21496-1 parameters every case carries. All values are dyadic
/// (exactly representable in the wire format's fractions), so the decoded
/// params must equal the input *exactly* — no tolerance hides a mis-serialized
/// field. Per-channel values differ for the 3-channel case so a channel swap
/// is visible.
fn gain_map_params(case: GainMapCase) -> GainMapParams {
    let ch = |i: usize| GainMapChannel {
        min: -1.0 + i as f64 * 0.25,
        max: 2.0 + i as f64 * 0.5,
        gamma: 1.0 + i as f64 * 0.5,
        base_offset: 1.0 / 64.0,
        alternate_offset: 1.0 / 32.0,
    };
    let mut p = GainMapParams::default();
    p.channels = if case.channels == 1 {
        [ch(0); 3]
    } else {
        [ch(0), ch(1), ch(2)]
    };
    // Forward: SDR base (0 stops), HDR alternate (+2 stops). Backward flips the
    // roles — and sets the authoritative flag, not just the headrooms.
    if case.backward {
        p.base_hdr_headroom = 2.0;
        p.alternate_hdr_headroom = 0.0;
        p.backward_direction = true;
    } else {
        p.base_hdr_headroom = 0.0;
        p.alternate_hdr_headroom = 2.0;
    }
    p
}

/// Deterministic 8-bit gain-map pixels: a smooth ramp per channel (a gain map
/// is a low-frequency control signal), offset per channel so channel order is
/// observable.
fn gain_map_pixels(case: GainMapCase) -> Pixels {
    let c = case.channels as usize;
    let mut bytes = vec![0u8; GAIN_MAP_W as usize * GAIN_MAP_H as usize * c];
    for y in 0..GAIN_MAP_H as usize {
        for x in 0..GAIN_MAP_W as usize {
            for k in 0..c {
                bytes[(y * GAIN_MAP_W as usize + x) * c + k] =
                    (16 + x * 13 + y * 17 + k * 40) as u8;
            }
        }
    }
    Pixels {
        width: GAIN_MAP_W,
        rows: GAIN_MAP_H,
        desc: if c == 1 {
            PixelDescriptor::GRAY8
        } else {
            PixelDescriptor::RGB8
        },
        bytes,
    }
}

/// The alternate rendition's colour: PQ, as an UltraHDR/AVIF map usually says.
const GAIN_MAP_ALTERNATE_CICP: Cicp = Cicp::BT2100_PQ;

/// Build the encode-input fixture for one case. Built fresh each time
/// ([`DecodedGainMap`] is consumed by `with_gain_map_pixels`).
fn gain_map_fixture(case: GainMapCase) -> DecodedGainMap {
    let px = gain_map_pixels(case);
    let buf = PixelBuffer::from_vec(px.bytes, px.width, px.rows, px.desc)
        .expect("gain-map fixture dimensions are valid");
    let info = GainMapInfo::new(gain_map_params(case), GAIN_MAP_W, GAIN_MAP_H, case.channels)
        .with_alternate_cicp(GAIN_MAP_ALTERNATE_CICP);
    DecodedGainMap::new(buf, info)
}

/// A format that is not the encoder's own, for the mismatched-`format`
/// rejection on `with_gain_map_encoded`.
fn foreign_format<E: EncoderConfig>() -> ImageFormat {
    [ImageFormat::Bmp, ImageFormat::Pnm, ImageFormat::Qoi]
        .into_iter()
        .find(|f| *f != E::format())
        .expect("three candidates cannot all equal one format")
}

/// Encode `img` with `gm` attached via `with_gain_map_pixels` (one-shot path).
fn enc_with_gain_map<E>(cfg: &E, img: &TestImage, gm: DecodedGainMap) -> Result<Vec<u8>, String>
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    let enc = cfg
        .clone()
        .job()
        .with_metadata_policy(Metadata::none(), MetadataPolicy::PreserveExact)
        .with_gain_map_pixels(gm)
        .map_err(|e| format!("with_gain_map_pixels: {e}"))?
        .encoder()
        .map_err(|e| e.to_string())?;
    Ok(enc
        .encode(img.as_slice())
        .map_err(|e| e.to_string())?
        .into_vec())
}

/// Encode `img` with `gm` attached, through the incremental `push_rows` +
/// `finish` path — a separate code path in real codecs, where a job-level
/// setting is easy to leave behind.
fn enc_push_rows_with_gain_map<E>(
    cfg: &E,
    img: &TestImage,
    gm: DecodedGainMap,
) -> Result<Vec<u8>, String>
where
    E: EncoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    let mut enc = cfg
        .clone()
        .job()
        .with_metadata_policy(Metadata::none(), MetadataPolicy::PreserveExact)
        .with_gain_map_pixels(gm)
        .map_err(|e| format!("with_gain_map_pixels: {e}"))?
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

/// Decode with a specific [`GainMapRender`] intent.
fn dec_render<D: DecoderConfig>(
    cfg: &D,
    bytes: &[u8],
    render: GainMapRender,
) -> Result<DecodeOutput, D::Error> {
    cfg.clone()
        .job()
        .with_gain_map_render(render)
        .decoder(Cow::Borrowed(bytes), &[])?
        .decode()
}

/// What the check compares of a surfaced gain map.
struct GainMapObserved {
    pixels: Pixels,
    info: GainMapInfo,
}

impl GainMapObserved {
    fn of(gm: &DecodedGainMap) -> Self {
        Self {
            pixels: grab(gm.pixels.as_slice()),
            info: gm.metadata.clone(),
        }
    }
}

/// Mean absolute sample error between two same-geometry 8-bit maps.
fn mean_abs_error(a: &[u8], b: &[u8]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return f64::INFINITY;
    }
    let sum: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    sum as f64 / a.len() as f64
}

/// Sanity bound for a *lossy* gain-map codec, in 8-bit sample units: the
/// smooth 10×7 ramp fixture re-encoded at the codec's default must come back
/// within this mean absolute error. This is a "not blank, not garbled, not
/// another channel" guard, **not** a fidelity claim — how well a lossy codec
/// preserves a gain map is that codec's own rate–distortion test (see the
/// q90 knee note on `with_gain_map_pixels`).
const LOSSY_GAIN_MAP_MAE_BOUND: f64 = 24.0;

/// Compare a surfaced gain map with what was embedded: geometry, channel
/// count, params (exactly), the alternate CICP, and the pixels — byte-exact
/// when the encoder is lossless, else within [`LOSSY_GAIN_MAP_MAE_BOUND`].
fn compare_gain_map(
    got: &GainMapObserved,
    want_px: &Pixels,
    want_info: &GainMapInfo,
    lossless: bool,
    what: &str,
) -> Result<(), String> {
    if (got.pixels.width, got.pixels.rows) != (want_px.width, want_px.rows) {
        return Err(format!(
            "{what}: gain-map geometry is {}x{}, expected {}x{}",
            got.pixels.width, got.pixels.rows, want_px.width, want_px.rows
        ));
    }
    let got_ch = got.pixels.desc.channels() as u8;
    if got_ch != want_info.channels {
        return Err(format!(
            "{what}: gain map has {got_ch} channels ({:?}), expected {}",
            got.pixels.desc, want_info.channels
        ));
    }
    if got.info.width != want_px.width || got.info.height != want_px.rows {
        return Err(format!(
            "{what}: GainMapInfo says {}x{} but the pixels are {}x{}",
            got.info.width, got.info.height, want_px.width, want_px.rows
        ));
    }
    if got.info.channels != want_info.channels {
        return Err(format!(
            "{what}: GainMapInfo.channels is {}, expected {}",
            got.info.channels, want_info.channels
        ));
    }
    if got.info.params != want_info.params {
        return Err(format!(
            "{what}: ISO 21496-1 params did not survive exactly (all fixture values are \
             dyadic, so this is a serialization/parse defect, not rounding):\n  got  {:?}\n  want {:?}",
            got.info.params, want_info.params
        ));
    }
    if got.info.alternate_cicp != want_info.alternate_cicp {
        return Err(format!(
            "{what}: alternate_cicp is {:?}, expected {:?}",
            got.info.alternate_cicp, want_info.alternate_cicp
        ));
    }
    if got.pixels.desc.bytes_per_pixel() != want_px.desc.bytes_per_pixel() {
        return Err(format!(
            "{what}: gain map came back as {:?}, expected an 8-bit {}-channel layout",
            got.pixels.desc, want_info.channels
        ));
    }
    if lossless {
        if got.pixels.bytes != want_px.bytes {
            return Err(format!(
                "{what}: gain-map pixels differ from the input on a lossless encoder"
            ));
        }
    } else {
        let mae = mean_abs_error(&got.pixels.bytes, &want_px.bytes);
        if mae > LOSSY_GAIN_MAP_MAE_BOUND {
            return Err(format!(
                "{what}: gain-map pixels came back with mean abs error {mae:.1} (bound \
                 {LOSSY_GAIN_MAP_MAE_BOUND}) — blank, garbled, or the wrong map"
            ));
        }
    }
    Ok(())
}

/// The gain-map encode/decode contract (`docs/gainmap-pipeline-scope-2026-06-08.md`
/// Phases 1, 2 and 4, tracked in zencodec#24), for every declared/undeclared
/// combination of [`EncodeCapabilities::gain_map`](zencodec::encode::EncodeCapabilities::gain_map)
/// and [`DecodeCapabilities::gain_map`](zencodec::decode::DecodeCapabilities::gain_map):
///
/// **Encoder undeclared** ⇒ `with_gain_map_pixels` and `with_gain_map_encoded`
/// must fail with [`UnsupportedOperation::GainMapEncode`] (the trait default) —
/// a dropped gain map is lost HDR, so silence is the one forbidden outcome.
///
/// **Encoder declared** ⇒ for each case of the test matrix (1-channel forward,
/// 3-channel forward, 1-channel backward-direction — sub-resolution 10×7 maps
/// on a 20×14 RGB8 base, ISO 21496-1 params with only dyadic values, PQ
/// alternate CICP):
///
/// - `with_gain_map_pixels` accepts the map and the base still encodes;
///   `with_gain_map_encoded` with a *foreign* `format` is rejected (the
///   documented mismatch error — any error, it is the codec's own).
/// - **Decoder declared** ⇒ `probe()` reports the map (`gain_map` presence not
///   `Absent`, and when `Available` its geometry/channels/params match;
///   `supplements.gain_map` set); the default `BaseOnly` decode carries **no**
///   [`DecodedGainMap`] extra (opt-in only) and its base pixels equal the input
///   when the encoder is lossless; `Components` surfaces a `DecodedGainMap`
///   whose geometry, channels, params (exact), alternate CICP and pixels
///   (byte-exact when lossless, else within a mean-abs-error sanity bound)
///   match; `with_extract_gain_map(true)` yields the identical map;
///   `ReconstructHdr { None }` without `reconstructs_hdr` either surfaces
///   the components with the base still labelled SDR, or fails with
///   `UnsupportedOperation` — never an SDR buffer labelled PQ/HLG; with
///   `reconstructs_hdr` it must succeed with an HDR-labelled buffer and the
///   envelope (`mastering_display` + `content_light_level`) on `source_color`.
/// - **Transcode (Phase 4)**: the `Components` map is fed back through a fresh
///   `with_gain_map_pixels`, decoded again, and must match the first
///   generation (metadata exactly; pixels byte-exact when lossless).
/// - **`push_rows` path** (when the encoder declares it): the same map
///   attached to an incremental `push_rows` + `finish` encode surfaces
///   identically — the streaming encoder is a separate code path in real
///   codecs, and a job-level setting is easy to leave behind there.
/// - **Decoder undeclared** ⇒ a `Components` decode of the same file either
///   decodes the base with no `DecodedGainMap` extra (a hidden capability is
///   a lie) or fails with `UnsupportedOperation`.
///
/// Skipped (`Ok`) only for the undeclared/undeclared pair once the loud
/// rejection is verified. All violations across the matrix are collected and
/// reported together. Not covered: the own-format `with_gain_map_encoded`
/// fast path (the payload shape is container-specific — a bare AV1 OBU vs a
/// whole JPEG — so a codec tests it with its own fixture), 10/12-bit maps, and
/// the positive `reconstructs_hdr` branch against the testkit's own codecs
/// (the reference declines to reconstruct, as zencodec carries no HDR math).
/// Part of [`check_all`].
pub fn check_gain_map_roundtrip<E, D>(enc: E, dec: D) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "gain_map_roundtrip";
    let (ec, dc) = (E::capabilities(), D::capabilities());
    let mut v: Vec<String> = Vec::new();

    // --- undeclared encoder: loud rejection, both entry points ---
    if !ec.gain_map() {
        let case = GAIN_MAP_CASES[0];
        match enc
            .clone()
            .job()
            .with_gain_map_pixels(gain_map_fixture(case))
        {
            Ok(_) => v.push(
                "encoder does not declare gain_map, yet with_gain_map_pixels succeeded (hidden \
                 capability, or a silently dropped gain map)"
                    .into(),
            ),
            Err(e) if e.unsupported_operation() == Some(&UnsupportedOperation::GainMapEncode) => {}
            Err(e) => v.push(format!(
                "encoder does not declare gain_map: with_gain_map_pixels must fail with \
                 UnsupportedOperation::GainMapEncode, got: {e}"
            )),
        }
        let src = GainMapSource::new(
            vec![0u8; 16],
            E::format(),
            GainMapInfo::new(gain_map_params(case), GAIN_MAP_W, GAIN_MAP_H, 1),
        );
        match enc.clone().job().with_gain_map_encoded(src) {
            Ok(_) => v.push(
                "encoder does not declare gain_map, yet with_gain_map_encoded succeeded".into(),
            ),
            Err(e) if e.unsupported_operation() == Some(&UnsupportedOperation::GainMapEncode) => {}
            Err(e) => v.push(format!(
                "encoder does not declare gain_map: with_gain_map_encoded must fail with \
                 UnsupportedOperation::GainMapEncode, got: {e}"
            )),
        }
        return if v.is_empty() {
            Ok(())
        } else {
            Err(fail(CHECK, v.join("; ")))
        };
    }

    // --- declared encoder ---
    let base = TestImage::rgb8_gradient(20, 14);
    let lossless = enc.is_lossless() == Some(true);

    // Foreign-format encoded map: the documented mismatch rejection.
    let foreign = GainMapSource::new(
        vec![0u8; 16],
        foreign_format::<E>(),
        GainMapInfo::new(
            gain_map_params(GAIN_MAP_CASES[0]),
            GAIN_MAP_W,
            GAIN_MAP_H,
            1,
        ),
    );
    if enc.clone().job().with_gain_map_encoded(foreign).is_ok() {
        v.push(format!(
            "with_gain_map_encoded accepted a gain map in a foreign format ({:?}) — the \
             contract is to reject a format mismatch so the caller decodes to pixels",
            foreign_format::<E>()
        ));
    }

    for case in GAIN_MAP_CASES {
        let name = case.name;
        let want_px = gain_map_pixels(case);
        let want_info = gain_map_fixture(case).metadata;

        let bytes = match enc_with_gain_map(&enc, &base, gain_map_fixture(case)) {
            Ok(b) => b,
            Err(e) => {
                v.push(format!("[{name}] encode with gain map: {e}"));
                continue;
            }
        };

        if !dc.gain_map() {
            // A decoder without the capability must not surface one anyway.
            match dec_render(&dec, &bytes, GainMapRender::Components) {
                Ok(out) if out.extras::<DecodedGainMap>().is_some() => v.push(format!(
                    "[{name}] decoder does not declare gain_map, yet Components surfaced a \
                     DecodedGainMap (hidden capability)"
                )),
                Ok(_) => {}
                Err(e) if e.unsupported_operation().is_some() => {}
                Err(e) => v.push(format!(
                    "[{name}] decoder does not declare gain_map: a Components decode must \
                     decode the base or fail with UnsupportedOperation, got: {e}"
                )),
            }
            continue;
        }

        // probe(): the map is reported.
        match dec.clone().job().probe(&bytes) {
            Err(e) => v.push(format!("[{name}] probe: {e}")),
            Ok(info) => {
                if !info.supplements.gain_map {
                    v.push(format!(
                        "[{name}] probe: supplements.gain_map is false on a file with a gain map"
                    ));
                }
                match &info.gain_map {
                    GainMapPresence::Absent => v.push(format!(
                        "[{name}] probe: gain_map presence is Absent on a file with a gain map"
                    )),
                    GainMapPresence::Available(i) => {
                        if (i.width, i.height, i.channels)
                            != (GAIN_MAP_W, GAIN_MAP_H, case.channels)
                        {
                            v.push(format!(
                                "[{name}] probe: GainMapInfo is {}x{}x{}, expected {}x{}x{}",
                                i.width,
                                i.height,
                                i.channels,
                                GAIN_MAP_W,
                                GAIN_MAP_H,
                                case.channels
                            ));
                        }
                        if i.params != want_info.params {
                            v.push(format!(
                                "[{name}] probe: ISO 21496-1 params differ from the input:\n  got  {:?}\n  want {:?}",
                                i.params, want_info.params
                            ));
                        }
                    }
                    _ => {} // Unknown is honest for a cheap probe
                }
            }
        }

        // Default render: base only, no surfaced map.
        match dec_render(&dec, &bytes, GainMapRender::BaseOnly) {
            Err(e) => v.push(format!("[{name}] BaseOnly decode: {e}")),
            Ok(out) => {
                if out.extras::<DecodedGainMap>().is_some() {
                    v.push(format!(
                        "[{name}] BaseOnly (the default) surfaced a DecodedGainMap — gain-map \
                         decode is opt-in"
                    ));
                }
                let px = grab(out.pixels());
                if (px.width, px.rows) != (base.width, base.height) {
                    v.push(format!(
                        "[{name}] BaseOnly decoded {}x{}, expected the {}x{} base",
                        px.width, px.rows, base.width, base.height
                    ));
                } else if lossless && px.bytes != base.pixels().bytes {
                    v.push(format!(
                        "[{name}] BaseOnly base pixels differ from the input on a lossless \
                         encoder — attaching a gain map must not disturb the base"
                    ));
                }
            }
        }

        // Components: the map is surfaced and matches.
        let first = match dec_render(&dec, &bytes, GainMapRender::Components) {
            Err(e) => {
                v.push(format!("[{name}] Components decode: {e}"));
                None
            }
            Ok(mut out) => match out.take_extras::<DecodedGainMap>() {
                None => {
                    v.push(format!(
                        "[{name}] decoder declares gain_map, but Components surfaced no \
                         DecodedGainMap"
                    ));
                    None
                }
                Some(gm) => {
                    let obs = GainMapObserved::of(&gm);
                    if let Err(e) =
                        compare_gain_map(&obs, &want_px, &want_info, lossless, "Components")
                    {
                        v.push(format!("[{name}] {e}"));
                    }
                    Some((gm, obs))
                }
            },
        };

        // with_extract_gain_map(true) is the same request as Components.
        if let Some((_, first_obs)) = &first {
            match dec
                .clone()
                .job()
                .with_extract_gain_map(true)
                .decoder(Cow::Borrowed(&bytes), &[])
                .and_then(|d| d.decode())
            {
                Err(e) => v.push(format!("[{name}] with_extract_gain_map(true) decode: {e}")),
                Ok(out) => match out.extras::<DecodedGainMap>() {
                    None => v.push(format!(
                        "[{name}] with_extract_gain_map(true) surfaced no DecodedGainMap, but \
                         Components did — the two must be equivalent"
                    )),
                    Some(gm) => {
                        let obs = GainMapObserved::of(gm);
                        if obs.pixels != first_obs.pixels || obs.info != first_obs.info {
                            v.push(format!(
                                "[{name}] with_extract_gain_map(true) and Components surfaced \
                                 different gain maps"
                            ));
                        }
                    }
                },
            }
        }

        // ReconstructHdr: honoured only with reconstructs_hdr; otherwise an
        // honest fallback, never an SDR buffer wearing an HDR label.
        let recon = GainMapRender::ReconstructHdr {
            target_headroom: None,
        };
        match dec_render(&dec, &bytes, recon) {
            Ok(out) => {
                let desc = out.pixels().descriptor();
                let hdr_labelled = matches!(
                    desc.transfer(),
                    TransferFunction::Pq | TransferFunction::Hlg | TransferFunction::Linear
                );
                if dc.reconstructs_hdr() {
                    if !hdr_labelled {
                        v.push(format!(
                            "[{name}] reconstructs_hdr is declared, but ReconstructHdr produced a \
                             buffer labelled {:?} — the output must be an HDR pixel format",
                            desc.transfer()
                        ));
                    }
                    let sc = &out.info().source_color;
                    if sc.mastering_display.is_none() || sc.content_light_level.is_none() {
                        v.push(format!(
                            "[{name}] reconstructs_hdr: the reconstructed output must carry the \
                             luminance envelope (mastering_display + content_light_level) on \
                             source_color — without it a native-HDR transcode loses it"
                        ));
                    }
                } else if matches!(
                    desc.transfer(),
                    TransferFunction::Pq | TransferFunction::Hlg
                ) {
                    v.push(format!(
                        "[{name}] reconstructs_hdr is NOT declared, yet ReconstructHdr returned a \
                         buffer labelled {:?} — an SDR base must not be relabelled HDR; surface \
                         Components or fail with UnsupportedOperation",
                        desc.transfer()
                    ));
                }
            }
            Err(e) if dc.reconstructs_hdr() => v.push(format!(
                "[{name}] reconstructs_hdr is declared, but ReconstructHdr failed: {e}"
            )),
            Err(e) if e.unsupported_operation().is_some() => {}
            Err(e) => v.push(format!(
                "[{name}] ReconstructHdr without reconstructs_hdr must surface Components or \
                 fail with UnsupportedOperation, got: {e}"
            )),
        }

        // Phase 4: the surfaced map transcodes through with_gain_map_pixels.
        if let Some((gm, first_obs)) = first {
            let base2 = TestImage::rgb8_gradient(20, 14);
            match enc_with_gain_map(&enc, &base2, gm).and_then(|b| {
                dec_render(&dec, &b, GainMapRender::Components).map_err(|e| e.to_string())
            }) {
                Err(e) => v.push(format!(
                    "[{name}] transcode (decode → with_gain_map_pixels): {e}"
                )),
                Ok(out) => match out.extras::<DecodedGainMap>() {
                    None => v.push(format!(
                        "[{name}] transcode: the re-encoded file surfaced no DecodedGainMap"
                    )),
                    Some(gm2) => {
                        let obs2 = GainMapObserved::of(gm2);
                        if let Err(e) = compare_gain_map(
                            &obs2,
                            &first_obs.pixels,
                            &first_obs.info,
                            lossless,
                            "transcode generation 2",
                        ) {
                            v.push(format!("[{name}] {e}"));
                        }
                    }
                },
            }
        }

        // push_rows: the streaming encoder carries the map too.
        if ec.push_rows() {
            match enc_push_rows_with_gain_map(&enc, &base, gain_map_fixture(case)).and_then(|b| {
                dec_render(&dec, &b, GainMapRender::Components).map_err(|e| e.to_string())
            }) {
                Err(e) => v.push(format!("[{name}] push_rows encode with gain map: {e}")),
                Ok(out) => match out.extras::<DecodedGainMap>() {
                    None => v.push(format!(
                        "[{name}] push_rows: the incrementally encoded file surfaced no \
                         DecodedGainMap — the gain map was lost on the push_rows path"
                    )),
                    Some(gm) => {
                        if let Err(e) = compare_gain_map(
                            &GainMapObserved::of(gm),
                            &want_px,
                            &want_info,
                            lossless,
                            "push_rows",
                        ) {
                            v.push(format!("[{name}] {e}"));
                        }
                    }
                },
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
// Buffer colour-context conformance (zencodec#25)
// ===========================================================================

/// The colour every context check encodes with: an RGB-class ICC (the test
/// images are RGB/RGBA, so the profile is class-valid for the decoded layout)
/// plus sRGB CICP. `SourceColor`'s default authority is ICC, so a faithful
/// decoder's drop-dupe context carries the ICC alone.
fn color_context_metadata() -> Metadata {
    Metadata::none()
        .with_icc(fixtures::sample_icc())
        .with_cicp(Cicp::SRGB)
}

fn dec_output<D: DecoderConfig>(cfg: &D, bytes: &[u8]) -> Result<DecodeOutput, String> {
    cfg.clone()
        .job()
        .decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())
}

fn describe_ctx(ctx: Option<&ColorContext>) -> String {
    match ctx {
        None => "no ColorContext".into(),
        Some(c) => format!(
            "ColorContext {{ icc: {}, cicp: {:?} }}",
            c.icc
                .as_ref()
                .map_or("none".to_string(), |i| format!("{} bytes", i.len())),
            c.cicp
        ),
    }
}

fn same_ctx(a: Option<&Arc<ColorContext>>, b: Option<&Arc<ColorContext>>) -> bool {
    a.map(|x| &**x) == b.map(|x| &**x)
}

/// The two rules every *attached* context must satisfy, for one emitted
/// buffer or slice. `None` (nothing attached) is always acceptable here.
///
/// 1. **Non-empty** — a context with neither ICC nor CICP is noise; attach
///    nothing instead.
/// 2. **Class gate** — an ICC rides a buffer only when its device class (header
///    bytes 16..20) matches the buffer's colour model: `RGB ` ↔ Rgb/Rgba/Bgra,
///    `GRAY` ↔ Gray/GrayAlpha, `CMYK` ↔ Cmyk. Crosswise pairing is invalid
///    signaling (libpng rejects it); an unreadable class is not valid for any
///    layout.
fn validate_buffer_context(
    ctx: Option<&ColorContext>,
    desc: PixelDescriptor,
) -> Result<(), String> {
    let Some(ctx) = ctx else {
        return Ok(());
    };
    if ctx.icc.is_none() && ctx.cicp.is_none() {
        return Err(
            "an attached ColorContext carries neither ICC nor CICP — attach nothing rather \
             than an empty description"
                .into(),
        );
    }
    if let Some(icc) = &ctx.icc {
        let model = desc.color_model();
        match zenpixels::icc::profile_color_space(icc) {
            Some(class) if class == model => {}
            Some(class) => {
                return Err(format!(
                    "an ICC of device class {class:?} is attached to a {model:?} buffer ({desc:?}) — \
                     an ICC only rides a layout its class describes (RGB ↔ Rgb/Rgba/Bgra, GRAY ↔ \
                     Gray/GrayAlpha); derive its CICP and carry that alone, or emit a layout the \
                     profile describes"
                ));
            }
            None => {
                return Err(format!(
                    "an ICC whose device class (header bytes 16..20) is unreadable is attached to a \
                     {model:?} buffer — it is not class-valid for any layout; do not attach it"
                ));
            }
        }
    }
    Ok(())
}

/// Whatever [`ColorContext`] a decoder attaches to its output pixels is
/// class-valid and the same on every decode path.
///
/// The decoded-buffer convention (zencodec `docs/IMPLEMENTING.md`, "Colour on the
/// decoded buffer"): a decoder SHOULD attach `SourceColor::to_color_context()` to
/// the buffers it emits, class-gated so an ICC only rides a layout its device
/// class describes. This check enforces the two invariants that hold whether or
/// not a codec has adopted the convention yet:
///
/// - **Class-valid and non-empty** — see the rules on the ICC device class in the
///   module docs; an attached context must describe *these* pixels.
/// - **Path equivalence** — the one-shot buffer, every streaming strip (when
///   streaming is advertised), and every animation frame on both the borrowed and
///   the owned path (when animation is advertised) carry *identical* contexts. Strip
///   and scratch buffers are rebuilt per batch and are where a context silently
///   dies; a probe-era vs frame-era CICP divergence between paths shows up here too.
///
/// It does **not** require a context to be attached — a codec that attaches
/// nothing passes (the pixels stay described by `ImageInfo.source_color`). Run
/// [`check_color_context_attached`] for the strict positive direction once the
/// codec has adopted the convention. Part of [`check_all`].
pub fn check_color_context_consistency<E, D>(enc: E, dec: D, img: &TestImage) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
    <E::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder,
{
    const CHECK: &str = "color_context_consistency";
    let meta = color_context_metadata();
    let bytes = enc_oneshot(&enc, img, meta.clone(), MetadataPolicy::PreserveExact)
        .map_err(|e| fail(CHECK, format!("encode: {e}")))?;
    let out = dec_output(&dec, &bytes).map_err(|e| fail(CHECK, format!("decode: {e}")))?;
    let desc = out.pixels().descriptor();
    let oneshot = out.into_buffer().color_context().cloned();
    validate_buffer_context(oneshot.as_deref(), desc)
        .map_err(|e| fail(CHECK, format!("one-shot decode: {e}")))?;

    if D::capabilities().streaming() {
        let mut sd = dec
            .clone()
            .job()
            .streaming_decoder(Cow::Borrowed(&bytes), &[])
            .map_err(|e| fail(CHECK, format!("streaming decoder: {e}")))?;
        while let Some((y, strip)) = sd
            .next_batch()
            .map_err(|e| fail(CHECK, format!("streaming next_batch: {e}")))?
        {
            validate_buffer_context(strip.color_context().map(|c| &**c), strip.descriptor())
                .map_err(|e| fail(CHECK, format!("streaming strip at y={y}: {e}")))?;
            if !same_ctx(strip.color_context(), oneshot.as_ref()) {
                return Err(fail(
                    CHECK,
                    format!(
                        "streaming strip at y={y} carries {} but the one-shot decode carries {} — \
                         strip/scratch buffers must re-attach the context at emission",
                        describe_ctx(strip.color_context().map(|c| &**c)),
                        describe_ctx(oneshot.as_deref())
                    ),
                ));
            }
        }
    }

    if E::capabilities().animation() && D::capabilities().animation() {
        let frames = [
            TestImage::rgba8_gradient_seeded(img.width, img.height, 0),
            TestImage::rgba8_gradient_seeded(img.width, img.height, 90),
        ];
        let anim = encode_animation(&enc, &frames, meta)
            .map_err(|e| fail(CHECK, format!("animation encode: {e}")))?;
        let primary = dec_output(&dec, &anim)
            .map_err(|e| fail(CHECK, format!("one-shot decode of the animation: {e}")))?;
        let pdesc = primary.pixels().descriptor();
        let primary_ctx = primary.into_buffer().color_context().cloned();
        validate_buffer_context(primary_ctx.as_deref(), pdesc)
            .map_err(|e| fail(CHECK, format!("one-shot decode of the animation: {e}")))?;

        let mismatch = |path: &str, i: u32, got: Option<&Arc<ColorContext>>| {
            fail(
                CHECK,
                format!(
                    "{path} frame {i} carries {} but the one-shot decode of the same file carries {} — \
                     every frame must carry the context its pixels are described by",
                    describe_ctx(got.map(|c| &**c)),
                    describe_ctx(primary_ctx.as_deref())
                ),
            )
        };

        let mut d = dec
            .clone()
            .job()
            .animation_frame_decoder(Cow::Borrowed(&anim), &[])
            .map_err(|e| fail(CHECK, format!("animation decoder: {e}")))?;
        while let Some(frame) = d
            .render_next_frame(None)
            .map_err(|e| fail(CHECK, format!("render_next_frame: {e}")))?
        {
            let i = frame.frame_index();
            let px = frame.pixels();
            validate_buffer_context(px.color_context().map(|c| &**c), px.descriptor())
                .map_err(|e| fail(CHECK, format!("render_next_frame frame {i}: {e}")))?;
            if !same_ctx(px.color_context(), primary_ctx.as_ref()) {
                return Err(mismatch("render_next_frame", i, px.color_context()));
            }
        }

        let mut d = dec
            .clone()
            .job()
            .animation_frame_decoder(Cow::Borrowed(&anim), &[])
            .map_err(|e| fail(CHECK, format!("animation decoder: {e}")))?;
        while let Some(frame) = d
            .render_next_frame_owned(None)
            .map_err(|e| fail(CHECK, format!("render_next_frame_owned: {e}")))?
        {
            let i = frame.frame_index();
            let px = frame.pixels();
            validate_buffer_context(px.color_context().map(|c| &**c), px.descriptor())
                .map_err(|e| fail(CHECK, format!("render_next_frame_owned frame {i}: {e}")))?;
            if !same_ctx(px.color_context(), primary_ctx.as_ref()) {
                return Err(mismatch("render_next_frame_owned", i, px.color_context()));
            }
        }
    }
    Ok(())
}

/// A decoder that read colour back attaches it to the output buffer — the
/// strict, positive direction of the decoded-buffer convention.
///
/// Encodes with an RGB-class ICC + sRGB CICP under `PreserveExact`, decodes, and
/// looks at what the decoder reported in `ImageInfo.source_color`. If it read
/// back neither ICC nor CICP there is nothing to attach and the check passes
/// (whether it *should* have read them is [`check_capability_honesty`]'s job).
/// Otherwise the output buffer must carry a [`ColorContext`], and it must carry
/// the **authoritative** field: under [`ColorAuthority::Icc`] with a class-valid
/// profile, the ICC bytes ride as-is (rank 1 of the convention); under
/// [`ColorAuthority::Cicp`], the CICP. A class-invalid ICC only has to be
/// replaced by *some* description (the derived-CICP fallback), which the
/// class-gate rules in [`check_color_context_consistency`] verify. When streaming
/// is advertised, the first strip must carry a context too.
///
/// Opt-in — **not** part of [`check_all`] — because the convention is a SHOULD
/// that codecs adopt one at a time; run it once yours does.
pub fn check_color_context_attached<E, D>(enc: E, dec: D, img: &TestImage) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "color_context_attached";
    let bytes = enc_oneshot(
        &enc,
        img,
        color_context_metadata(),
        MetadataPolicy::PreserveExact,
    )
    .map_err(|e| fail(CHECK, format!("encode: {e}")))?;
    let out = dec_output(&dec, &bytes).map_err(|e| fail(CHECK, format!("decode: {e}")))?;
    let src = out.info().source_color.clone();
    let desc = out.pixels().descriptor();
    let ctx = out.into_buffer().color_context().cloned();

    if src.icc_profile.is_none() && src.cicp.is_none() {
        return Ok(()); // the decoder read no colour back — nothing to attach
    }
    let Some(ctx) = ctx else {
        return Err(fail(
            CHECK,
            format!(
                "the decoder read colour back into ImageInfo.source_color (icc: {}, cicp: {:?}) \
                 but attached no ColorContext to the output buffer — attach \
                 SourceColor::to_color_context(), class-gated, after the descriptor is final",
                src.icc_profile
                    .as_ref()
                    .map_or("none".to_string(), |i| format!("{} bytes", i.len())),
                src.cicp
            ),
        ));
    };
    validate_buffer_context(Some(&ctx), desc)
        .map_err(|e| fail(CHECK, format!("one-shot decode: {e}")))?;

    let src_icc_class_valid = src
        .icc_profile
        .as_deref()
        .is_some_and(|icc| zenpixels::icc::profile_color_space(icc) == Some(desc.color_model()));
    match src.color_authority {
        ColorAuthority::Icc if src_icc_class_valid => {
            if ctx.icc.as_deref() != src.icc_profile.as_deref() {
                return Err(fail(
                    CHECK,
                    format!(
                        "ICC is the authority and the source profile is class-valid for the {:?} \
                         output, so the buffer context must carry those ICC bytes as-is; it carries {}",
                        desc.color_model(),
                        describe_ctx(Some(&ctx))
                    ),
                ));
            }
        }
        ColorAuthority::Cicp if src.cicp.is_some() => {
            if ctx.cicp != src.cicp {
                return Err(fail(
                    CHECK,
                    format!(
                        "CICP is the authority ({:?}) but the buffer context carries {}",
                        src.cicp,
                        describe_ctx(Some(&ctx))
                    ),
                ));
            }
        }
        // Authority field absent, or an ICC the output layout can't carry: any
        // non-empty, class-valid description (already validated) is acceptable.
        ColorAuthority::Icc | ColorAuthority::Cicp => {}
    }

    if D::capabilities().streaming() {
        let mut sd = dec
            .clone()
            .job()
            .streaming_decoder(Cow::Borrowed(&bytes), &[])
            .map_err(|e| fail(CHECK, format!("streaming decoder: {e}")))?;
        let first = sd
            .next_batch()
            .map_err(|e| fail(CHECK, format!("streaming next_batch: {e}")))?;
        if let Some((y, strip)) = first
            && strip.color_context().is_none()
        {
            return Err(fail(
                CHECK,
                format!(
                    "the one-shot buffer carries a ColorContext but the streaming strip at y={y} \
                     carries none — re-attach the context on every emitted strip"
                ),
            ));
        }
    }
    Ok(())
}

// ===========================================================================
// Colour authority — the per-format spec table (issue #11, Table 4)
// ===========================================================================

/// The [`ColorAuthority`] a format's specification assigns, given which colour
/// fields the decoder actually read back.
///
/// This is the shared answer key behind [`check_source_color_authority`] — the
/// "what does the spec say" half of the audit in zencodec issue #11 (Table 4),
/// so every codec crate asserts the same rule instead of re-deriving it:
///
/// | format | rule |
/// |---|---|
/// | PNG | `cICP` outranks `iCCP` (PNG 3rd ed.): `Cicp` when CICP was read, else `Icc` |
/// | JXL | codestream colour encoding outranks an embedded ICC: `Cicp` when read, else `Icc` |
/// | HEIC | `nclx` is the primary colour authority (ISO 23008-12): `Cicp` when read, else `Icc` |
/// | AVIF | MIAF order: an ICC `colr` outranks `nclx` — `Icc` when an ICC was read, else `Cicp` when CICP was read, else `Icc` |
/// | Radiance HDR | scene-linear BT.709 has no ICC carrier: `Cicp` when the codec expressed it as CICP, else `Icc` |
/// | JPEG, WebP, GIF, TIFF, BMP, ICO, PNM, Farbfeld, QOI, TGA, DNG, RAW, PDF | ICC (or an sRGB assumption): always `Icc` |
///
/// Returns `None` where no rule is recorded — [`Custom`](ImageFormat::Custom),
/// [`Unknown`](ImageFormat::Unknown), and the formats the audit did not cover
/// (EXR, JPEG 2000, SVG) — so a check over those passes rather than inventing a
/// rule. `Cicp` is never expected when no CICP was read (the enum's own
/// invariant: codecs only set `Cicp` when `cicp` is populated).
pub fn expected_color_authority(
    format: ImageFormat,
    has_cicp: bool,
    has_icc: bool,
) -> Option<ColorAuthority> {
    use ImageFormat::*;
    let cicp_first = if has_cicp {
        ColorAuthority::Cicp
    } else {
        ColorAuthority::Icc
    };
    Some(match format {
        Png | Jxl | Heic | Hdr => cicp_first,
        Avif => {
            if has_icc {
                ColorAuthority::Icc
            } else {
                cicp_first
            }
        }
        Jpeg | WebP | Gif | Tiff | Bmp | Ico | Pnm | Farbfeld | Qoi | Tga | Dng | Raw | Pdf => {
            ColorAuthority::Icc
        }
        // EXR / JPEG 2000 / SVG were not covered by the audit; `Custom` /
        // `Unknown` (and any future `#[non_exhaustive]` variant) have no rule.
        _ => return None,
    })
}

/// A decoded [`SourceColor`] names the authority its format's spec assigns.
///
/// The unit form of the check — run it on the `source_color` a decoder produced
/// for a known input. The expected authority comes from
/// [`expected_color_authority`] applied to the fields the decoder *read back*
/// (so a decoder that reads no CICP is not expected to name it). Passes when the
/// format has no recorded rule.
///
/// This is the "shared test helper" the issue #11 audit asked for: the
/// authority-mismatch class of bug (HEIC naming `Icc` for an `nclx`-only file,
/// so the CICP was dropped by `SourceColor::to_color_context()` and the pixels
/// fell back to sRGB) is invisible to a pixel round trip and only shows up
/// downstream in the CMS.
pub fn check_source_color_authority(format: ImageFormat, sc: &SourceColor) -> Conformance {
    const CHECK: &str = "source_color_authority";
    let Some(want) = expected_color_authority(format, sc.cicp.is_some(), sc.icc_profile.is_some())
    else {
        return Ok(());
    };
    if sc.color_authority == want {
        return Ok(());
    }
    Err(fail(
        CHECK,
        format!(
            "{format:?} read cicp: {}, icc: {} — the format spec makes {want:?} authoritative, \
             but source_color.color_authority is {:?}",
            sc.cicp.map_or("none".to_string(), |c| format!("{c:?}")),
            sc.icc_profile
                .as_ref()
                .map_or("none".to_string(), |i| format!("{} bytes", i.len())),
            sc.color_authority
        ),
    ))
}

/// Decoded colour authority follows the format spec on every metadata mix.
///
/// Encodes `img` with an ICC only, a CICP only, both, and neither (each
/// channel only when the encoder declares it), then runs
/// [`check_source_color_authority`] on the `source_color` of the one-shot
/// decode *and* of `probe()`, under the encoder's declared
/// [`format()`](EncoderConfig::format). Passes trivially for formats without a
/// recorded rule (see [`expected_color_authority`]).
pub fn check_color_authority_spec<E, D>(enc: E, dec: D, img: &TestImage) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "color_authority_spec";
    let ec = E::capabilities();
    let format = E::format();
    let mut mixes: Vec<(&str, Metadata)> = vec![("no colour metadata", Metadata::none())];
    if ec.icc() {
        mixes.push((
            "icc only",
            Metadata::none().with_icc(fixtures::sample_icc()),
        ));
    }
    if ec.cicp() {
        mixes.push(("cicp only", Metadata::none().with_cicp(Cicp::SRGB)));
    }
    if ec.icc() && ec.cicp() {
        mixes.push((
            "icc + cicp",
            Metadata::none()
                .with_icc(fixtures::sample_icc())
                .with_cicp(Cicp::SRGB),
        ));
    }
    let mut v: Vec<String> = Vec::new();
    for (name, meta) in mixes {
        let bytes = match enc_oneshot(&enc, img, meta, MetadataPolicy::PreserveExact) {
            Ok(b) => b,
            Err(e) => {
                v.push(format!("{name}: encode failed: {e}"));
                continue;
            }
        };
        match dec_output(&dec, &bytes) {
            Err(e) => v.push(format!("{name}: decode failed: {e}")),
            Ok(out) => {
                if let Err(f) = check_source_color_authority(format, &out.info().source_color) {
                    v.push(format!("{name}, decode: {}", f.detail));
                }
            }
        }
        match dec.clone().job().probe(&bytes) {
            Err(e) => v.push(format!("{name}: probe failed: {e}")),
            Ok(info) => {
                if let Err(f) = check_source_color_authority(format, &info.source_color) {
                    v.push(format!("{name}, probe: {}", f.detail));
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
/// [`UnsupportedOperation`] — a codec can't claim a
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
/// `native_16bit`, `native_f32`, `enforces_max_pixels` / `enforces_max_memory`,
/// the CICP-carrier flags, and the `effort` / `quality` / `threads` ranges),
/// whose honesty needs format-specific fixtures a generic harness can't supply.
/// `hdr` and `gain_map` have their own checks ([`check_native_hdr_roundtrip`],
/// [`check_gain_map_roundtrip`]).
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
// Fidelity honesty (issue #26 — the per-codec `Fidelity` contract)
// ===========================================================================

/// The resolved [`Fidelity`] report is honest against the declared capabilities
/// and against the pixels that actually come back.
///
/// [`with_fidelity`](EncoderConfig::with_fidelity) is best-effort and
/// infallible, so the whole cross-codec contract rests on
/// [`resolved_target_fidelity`](EncoderConfig::resolved_target_fidelity) telling
/// the truth. For each request — `Lossless`, then every [`LossyTarget`](zencodec::encode::LossyTarget) arm
/// (`CodecSpecificQuality` at the middle of the declared `quality_range`,
/// `ApproxSsim2`, `ApproxButteraugli`, `ApproxZensimB`), each applied on top of a
/// prior `Lossless` request so a stale setting cannot leak through — the
/// check asserts:
///
/// - **`lossless` declared** ⇒ a `Lossless` request resolves to
///   `Some(Lossless)` **and** the decoded pixels are byte-identical to the
///   input. Undeclared ⇒ it must not resolve to `Some(Lossless)` (a codec
///   cannot claim a mode it does not declare).
/// - **`lossy` declared** ⇒ a lossy request never resolves to `Lossless` (a
///   codec with a lossy mode must use it when asked), and — when it also
///   declares a `quality_range` — resolves to `Some(Lossy(_))` rather than
///   `None`. Undeclared ⇒ it must not resolve to `Some(Lossy(_))`: the honest
///   outcomes are promotion to `Some(Lossless)` (as the
///   [`reference`](mod@reference) codec does) or `None`.
/// - Every request still encodes and decodes, and whenever the codec *reports*
///   `Lossless` the pixels are exact — whatever was asked for.
/// - The legacy getter agrees: when both `is_lossless()` and the resolved
///   fidelity are `Some`, `is_lossless()` equals `resolved.is_lossless()`.
///
/// All violations are collected and reported together. Not covered: whether a
/// metric target was *hit* (needs the metric), and the deferred
/// `LosslessMode`/near-lossless arm (not in the shipped enum).
pub fn check_fidelity_honesty<E, D>(enc: E, dec: D, img: &TestImage) -> Conformance
where
    E: EncoderConfig,
    D: DecoderConfig,
    <E::Job as EncodeJob>::Enc: Encoder<Error = E::Error>,
{
    const CHECK: &str = "fidelity_honesty";
    let ec = E::capabilities();
    let mut v: Vec<String> = Vec::new();

    // Round-trip a configured encoder; on success, report whether the pixels
    // came back exact.
    let exact = |cfg: &E| -> Result<bool, String> {
        let bytes = enc_oneshot(cfg, img, Metadata::none(), MetadataPolicy::PreserveExact)?;
        let (px, _) = dec_oneshot(&dec, &bytes)?;
        Ok(px == img.pixels())
    };
    let legacy_agrees = |name: &str, cfg: &E, resolved: Option<Fidelity>, v: &mut Vec<String>| {
        if let (Some(legacy), Some(f)) = (cfg.is_lossless(), resolved)
            && legacy != f.is_lossless()
        {
            v.push(format!(
                "{name}: is_lossless() = Some({legacy}) disagrees with resolved_target_fidelity() = {f:?}"
            ));
        }
    };

    // --- Lossless ---
    let ll = enc.clone().with_fidelity(Fidelity::Lossless);
    let r = ll.resolved_target_fidelity();
    if ec.lossless() {
        if r != Some(Fidelity::Lossless) {
            v.push(format!(
                "Lossless: `lossless` is declared, but the request resolved to {r:?} (expected Some(Lossless))"
            ));
        }
    } else if r == Some(Fidelity::Lossless) {
        v.push(
            "Lossless: `lossless` is NOT declared, yet the request resolved to Some(Lossless)"
                .into(),
        );
    }
    legacy_agrees("Lossless", &ll, r, &mut v);
    match exact(&ll) {
        Err(e) => v.push(format!("Lossless: round trip failed: {e}")),
        Ok(false) if r == Some(Fidelity::Lossless) => v.push(
            "Lossless: resolved to Lossless but the decoded pixels differ from the input".into(),
        ),
        Ok(_) => {}
    }

    // --- every lossy arm, applied after a Lossless request (last write wins) ---
    let q = ec.quality_range().map_or(75.0, |[lo, hi]| (lo + hi) / 2.0);
    let requests = [
        ("Lossy(CodecSpecificQuality)", Fidelity::codec_quality(q)),
        ("Lossy(ApproxSsim2)", Fidelity::ssim2(80.0)),
        ("Lossy(ApproxButteraugli)", Fidelity::butteraugli(1.5)),
        ("Lossy(ApproxZensimB)", Fidelity::zensim_b(80.0)),
    ];
    for (name, req) in requests {
        let cfg = enc
            .clone()
            .with_fidelity(Fidelity::Lossless)
            .with_fidelity(req);
        let r = cfg.resolved_target_fidelity();
        if ec.lossy() {
            match r {
                Some(Fidelity::Lossy(_)) => {}
                Some(Fidelity::Lossless) => v.push(format!(
                    "{name}: `lossy` is declared, but the request resolved to Lossless (a prior Lossless request leaked, or lossy is not honored)"
                )),
                None if ec.quality_range().is_some() => v.push(format!(
                    "{name}: `lossy` + a quality_range are declared, but the request resolved to None (report the target it mapped to)"
                )),
                _ => {}
            }
        } else if let Some(Fidelity::Lossy(t)) = r {
            v.push(format!(
                "{name}: `lossy` is NOT declared, yet the request resolved to Lossy({t:?}) — promote to Lossless (and report it) or report None"
            ));
        }
        legacy_agrees(name, &cfg, r, &mut v);
        match exact(&cfg) {
            Err(e) => v.push(format!("{name}: round trip failed: {e}")),
            Ok(false) if r == Some(Fidelity::Lossless) => v.push(format!(
                "{name}: resolved to Lossless but the decoded pixels differ from the input"
            )),
            Ok(_) => {}
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
/// [`ErrorCategory`] and codec name are gone.
/// `At<CodecError>` is one concrete type, so it survives any erasure by a
/// downcast. [`check_decode_error_envelope`] is the runtime companion that proves
/// the category actually *flows* through erasure.
///
/// It bounds the config, job, and leaf executor on both sides. The optional stub
/// associated types (`AnimationFrameEnc`, `StreamDec`, `AnimationFrameDec`) are
/// intentionally *not* bound: a still-only codec legitimately uses `()` /
/// [`Unsupported`](zencodec::Unsupported), whose `Error` is
/// [`UnsupportedOperation`], not the envelope.
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

/// A codec's [`ErrorCategory`] **and** originating codec
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

// ===========================================================================
// EOF / truncation-series conformance
// ===========================================================================

/// Whether `cat` is an acceptable category for a truncated / incomplete input.
///
/// TRUE for the whole image-bytes-origin cluster — [`ErrorCategory::Image`] with any
/// [`ImageError`](zencodec::ImageError): [`UnexpectedEof`](zencodec::ImageError::UnexpectedEof)
/// (the ideal), [`Malformed`](zencodec::ImageError::Malformed), or
/// [`Unsupported`](zencodec::ImageError::Unsupported). A truncated prefix legitimately
/// reads as *client-supplied incomplete data* (an HTTP 4xx): this early it may look
/// cut-short, corrupt, or unrecognizable, and a codec that can't yet tell those apart
/// is still safe as long as it stays inside the `Image` arm.
///
/// FALSE for every other origin, each of which MISATTRIBUTES a client-side truncation:
/// [`Internal`](ErrorCategory::Internal) (a codec bug / 5xx),
/// [`Resource`](ErrorCategory::Resource) (OOM from an unvalidated length in truncated
/// data, or a limit), [`Io`](ErrorCategory::Io) (there is no I/O on an in-memory slice),
/// the caller-fault [`Request`](ErrorCategory::Request) set, the operation
/// [`Stopped`](ErrorCategory::Stopped) set, and [`Policy`](ErrorCategory::Policy).
pub(crate) fn is_incomplete_input_category(cat: ErrorCategory) -> bool {
    // Default-DENY: `ErrorCategory` is `#[non_exhaustive]`, so any *future* variant
    // falls through to `false` and is conservatively flagged for review rather than
    // silently accepted as a valid truncation category. The image-bytes-origin arm
    // (`Image(_)`) IS exactly the incomplete-input-tolerable set — a truncated prefix
    // is always a client-supplied-data fault, never a codec bug or caller-request fault.
    matches!(cat, ErrorCategory::Image(_))
}

/// Deterministic series of truncation lengths spanning a `len`-byte bitstream: the
/// small absolute sizes `{0,1,2,3,4,8,16}` (header region) unioned with the
/// fractions `{1/8,1/4,3/8,1/2,5/8,3/4,7/8, len-1}` of `len`. Deduped, sorted
/// ascending, and clamped to `n < len` (a full-length "truncation" is the original
/// image, not a truncation). `0` — the empty input — is a legitimate truncation.
fn truncation_lengths(len: usize) -> Vec<usize> {
    let mut lens: Vec<usize> = vec![0, 1, 2, 3, 4, 8, 16];
    for (num, den) in [(1, 8), (1, 4), (3, 8), (1, 2), (5, 8), (3, 4), (7, 8)] {
        lens.push(len * num / den);
    }
    lens.push(len.saturating_sub(1));
    lens.retain(|&n| n < len);
    lens.sort_unstable();
    lens.dedup();
    lens
}

/// A truncated (incomplete) input is categorized as *incomplete client data* —
/// never mis-attributed as an internal bug, an OOM, an I/O error, or a caller
/// fault — and never panics or silently decodes.
///
/// This enforces the one part of the [`ErrorCategory`] taxonomy that IS broadly
/// distinguishable: whatever a codec's exact error, cutting a known-good image
/// short must land in the *incomplete-input* set
/// ([`UnexpectedEof`](zencodec::ImageError::UnexpectedEof)
/// is ideal, but the rest of the image-bytes-origin [`Image`](ErrorCategory::Image) arm
/// ([`Malformed`](zencodec::ImageError::Malformed) / [`Unsupported`](zencodec::ImageError::Unsupported))
/// is tolerated because a truncated prefix can genuinely look malformed). It catches the real bug class
/// where a codec reads a length field out of truncated data and OOMs, or funnels a
/// truncation into [`Internal`](ErrorCategory::Internal) — a 5xx for a 4xx-class
/// client error.
///
/// `valid` is a KNOWN-GOOD encoded image the codec supplies (at least a few bytes).
/// The check builds a deterministic series of prefixes and, for each, runs a **full
/// decode through the dyn-erased boundary** —
/// [`DynDecodeJob::push_decode`](zencodec::decode::DynDecodeJob::push_decode) into a
/// throwaway sink. That is the dyn full-decode path, so the pixel **body** is
/// actually read and *body* truncation is exercised, not just the header a `probe`
/// would parse. Each decode is wrapped in [`catch_unwind`](std::panic::catch_unwind)
/// so a codec panic on truncated input becomes a named failure, not a process abort.
/// Every offending offset is collected and reported together.
///
/// Per offset `n` of `len`:
/// - **panic** → failure (truncated input must never panic).
/// - **`Ok`** (decode succeeded) when `n <= len/4` (header/early region) → failure:
///   a truncated valid image must not silently decode. For `n > len/4` a success is
///   *tolerated* (trailing-marker / already-complete-payload leniency).
/// - **`Err`** → the erased [`ErrorCategory`] is read back: an incomplete-input
///   category passes; any other category fails, naming the offset. If the category
///   did not survive erasure at all, the codec is **Pattern A** (native-enum
///   `type Error`, not `At<CodecError>`) — reported once, exactly as
///   [`check_decode_error_envelope`] reports it.
///
/// Like the envelope checks, this is **opt-in and NOT part of [`check_all`]**: the
/// testkit's own [`reference`](mod@reference) codec is a Pattern-A foil that fails
/// it on the erased path, while the internal `minimal` envelope exemplar passes (its
/// truncations classify as `MalformedImage`).
pub fn check_decode_truncation_series<D>(dec: D, valid: &[u8]) -> Conformance
where
    D: DecoderConfig + 'static,
{
    const CHECK: &str = "decode_truncation_series";
    let len = valid.len();
    if len < 2 {
        return Err(fail(
            CHECK,
            "supply a real encoded image of at least a few bytes",
        ));
    }

    let mut violations: Vec<String> = Vec::new();
    let mut reported_pattern_a = false;

    for n in truncation_lengths(len) {
        let truncated = &valid[..n];
        // Drive a FULL decode through the dyn-erased boundary into a throwaway sink.
        // `push_decode` reads the pixel body (via the copy-to-sink helper), so this
        // exercises *body* truncation, not just the header a `probe` parses.
        // `catch_unwind` turns a codec panic into a reported failure naming the
        // offset; `AssertUnwindSafe` is sound because the torn sink is discarded.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let dyn_cfg: &dyn DynDecoderConfig = &dec;
            let mut sink = CollectSink::default();
            dyn_cfg
                .dyn_job()
                .push_decode(Cow::Borrowed(truncated), &mut sink, &[])
        }));

        match outcome {
            Err(_panic) => violations.push(format!(
                "decode panicked on truncation at {n}/{len} bytes — truncated input must never panic"
            )),
            Ok(Ok(_output)) => {
                if n <= len / 4 {
                    violations.push(format!(
                        "decode SUCCEEDED on a truncated header ({n} of {len} bytes) — a truncated \
                         valid image must not silently decode (pixels are sacred)"
                    ));
                }
                // n > len/4: trailing-marker / already-complete-payload leniency — tolerated.
            }
            Ok(Err(erased)) => match erased.error_category() {
                None => {
                    // Pattern A: the category is unrecoverable once erased. Report it
                    // once (the same loss `check_decode_error_envelope` catches) —
                    // every subsequent Err is None too, so don't spam per-offset.
                    if !reported_pattern_a {
                        reported_pattern_a = true;
                        violations.push(format!(
                            "ErrorCategory did not survive dyn-dispatch erasure: the decoder's \
                             `type Error` is not `At<CodecError>` (Pattern A — a native error enum \
                             erases to a bare `dyn Error`, which cannot be downcast to recover the \
                             category). Switch the zencodec trait impls to `type Error = \
                             At<CodecError>`. First erased error (truncation at {n}/{len} bytes) \
                             was: {erased}"
                        ));
                    }
                }
                Some(cat) => {
                    if !is_incomplete_input_category(cat) {
                        violations.push(format!(
                            "truncation at {n}/{len} bytes categorized as {cat:?}; a truncated input \
                             is incomplete client data and must map to an incomplete-input category \
                             (UnexpectedEof/MalformedImage/UnsupportedImageType/UnsupportedImageFeature), \
                             never {cat:?} — that misattributes it (e.g. Internal reads as a codec \
                             bug/5xx; OutOfMemory means the codec allocated from an unvalidated length \
                             in truncated data)"
                        ));
                    }
                    // incomplete-input category: OK (kept silent on success).
                }
            },
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(fail(CHECK, violations.join("; ")))
    }
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
    check_fidelity_honesty(enc.clone(), dec.clone(), &img)?;
    check_color_context_consistency(enc.clone(), dec.clone(), &img)?;
    check_color_authority_spec(enc.clone(), dec.clone(), &img)?;
    check_native_hdr_roundtrip(enc.clone(), dec.clone())?;
    check_gain_map_roundtrip(enc.clone(), dec.clone())?;
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
        assert!(x.has_device_ids() && x.has_camera_owner() && x.has_image_unique_id());
        assert!(x.has_datetimes() && x.has_time_offsets());
        assert_eq!(x.copyright().as_deref(), Some("(C) 2026 Test"));
    }

    #[test]
    fn metadata_check_catches_identifiers_alongside_retained_camera_tags() {
        use zencodec::{ExifPolicy, Retention};
        let rich = Metadata::none().with_exif(fixtures::rich_exif_le());
        let base = ExifPolicy::ATTRIBUTED_ORIENTATION
            .with_camera(Retention::Keep)
            .with_datetimes(Retention::Keep);
        let policy = |exif| MetadataPolicy::Custom(MetadataPolicy::Web.fields().with_exif(exif));
        let expected = rich.filtered(&policy(base));
        for leaky in [
            base.with_device_ids(Retention::Keep),
            base.with_camera_owner(Retention::Keep),
            base.with_image_unique_id(Retention::Keep),
            base.with_time_offsets(Retention::Keep),
        ] {
            let actual = rich.filtered(&policy(leaky));
            assert!(assert_no_leak("mutation", "camera without IDs", &actual, &expected).is_err());
        }
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

    /// The reference declares `gain_map` on both ends and honours the whole
    /// matrix (1ch/3ch/backward, opt-in surfacing, honest `ReconstructHdr`
    /// fallback, the Phase 4 transcode).
    #[test]
    fn reference_gain_map_roundtrip() {
        let (e, d) = ref_codecs();
        check_gain_map_roundtrip(e, d).unwrap();
    }

    /// The minimal codec declares no gain-map support: the trait defaults must
    /// reject both entry points with `GainMapEncode`, which is all the check
    /// asks of an undeclared encoder.
    #[test]
    fn minimal_gain_map_roundtrip() {
        check_gain_map_roundtrip(MinimalEncoderConfig::new(), MinimalDecoderConfig::new()).unwrap();
    }

    /// The fixture's ISO 21496-1 values are all dyadic, so the wire format
    /// must reproduce them exactly — the check's exact-params assertion rests
    /// on this (a non-representable fixture value would fail every codec).
    #[test]
    fn gain_map_fixture_params_are_wire_exact() {
        use zencodec::gainmap::{Iso21496Format, parse_iso21496_fmt, serialize_iso21496_fmt};
        for case in GAIN_MAP_CASES {
            let p = gain_map_params(case);
            for fmt in [Iso21496Format::JxlJhgm, Iso21496Format::AvifTmap] {
                let back = parse_iso21496_fmt(&serialize_iso21496_fmt(&p, fmt), fmt).unwrap();
                assert_eq!(back, p, "{} via {fmt:?}", case.name);
            }
        }
    }

    /// The reference's own-format `with_gain_map_encoded` fast path (not
    /// exercised by the generic check — the payload shape is container-specific):
    /// a single-frame RGB8 reference image is byte-carried as a 3-channel map,
    /// and any other format is the documented mismatch rejection.
    #[test]
    fn reference_gain_map_encoded_own_format() {
        let case = GAIN_MAP_CASES[1]; // 3ch
        let fixture = gain_map_fixture(case);
        // Encode the gain-map pixels as a standalone reference image...
        let gm_img = ReferenceEncoderConfig::new()
            .job()
            .encoder()
            .unwrap()
            .encode(fixture.pixels.as_slice())
            .unwrap()
            .into_vec();
        let src = GainMapSource::new(gm_img, ImageFormat::Pnm, fixture.metadata.clone());
        let base = TestImage::rgb8_gradient(20, 14);
        let bytes = ReferenceEncoderConfig::new()
            .job()
            .with_gain_map_encoded(src)
            .unwrap()
            .encoder()
            .unwrap()
            .encode(base.as_slice())
            .unwrap()
            .into_vec();
        let out = dec_render(&ReferenceDecoderConfig, &bytes, GainMapRender::Components).unwrap();
        let gm = out.extras::<DecodedGainMap>().expect("surfaced");
        compare_gain_map(
            &GainMapObserved::of(gm),
            &gain_map_pixels(case),
            &fixture.metadata,
            true,
            "encoded own-format",
        )
        .unwrap();

        // ...and a foreign format is rejected with the codec's own error.
        let bad = GainMapSource::new(
            vec![0; 8],
            ImageFormat::Jpeg,
            gain_map_fixture(case).metadata,
        );
        let err = ReferenceEncoderConfig::new()
            .job()
            .with_gain_map_encoded(bad)
            .err()
            .expect("foreign format rejected");
        assert!(matches!(err, RefError::Invalid(_)), "{err}");
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
        assert_eq!(
            err.category(),
            ErrorCategory::Stopped(zencodec::enough::StopReason::Cancelled)
        );
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
        assert_eq!(
            erased.error_category(),
            Some(ErrorCategory::Image(zencodec::ImageError::Malformed))
        );
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
        assert_eq!(
            err.error().category(),
            ErrorCategory::Image(zencodec::ImageError::Malformed)
        );
        assert_eq!(
            err.error().codec(),
            Some(crate::reference::MINIMAL_CODEC_NAME)
        );
        // Same category via the generic Option recovery:
        assert_eq!(
            err.error_category(),
            Some(ErrorCategory::Image(zencodec::ImageError::Malformed))
        );
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

    /// A lossless-only codec: `Lossless` is exact, every lossy arm promotes to
    /// `Lossless` and reports it, and the legacy getter agrees.
    #[test]
    fn reference_fidelity_honesty() {
        let (e, d) = ref_codecs();
        check_fidelity_honesty(e, d, &TestImage::rgba8_gradient(12, 9)).unwrap();
        let cfg = ReferenceEncoderConfig::new().with_fidelity(Fidelity::ssim2(70.0));
        assert_eq!(cfg.resolved_target_fidelity(), Some(Fidelity::Lossless));
        assert_eq!(cfg.is_lossless(), Some(true));
    }

    /// No `lossless`, no `lossy`, no quality dial: every request resolves to
    /// `None` and the pixels still round-trip.
    #[test]
    fn minimal_fidelity_honesty() {
        check_fidelity_honesty(
            MinimalEncoderConfig::new(),
            MinimalDecoderConfig::new(),
            &TestImage::rgba8_gradient(12, 9),
        )
        .unwrap();
        assert_eq!(
            MinimalEncoderConfig::new()
                .with_fidelity(Fidelity::Lossless)
                .resolved_target_fidelity(),
            None
        );
    }

    #[test]
    fn minimal_color_authority_spec() {
        check_color_authority_spec(
            MinimalEncoderConfig::new(),
            MinimalDecoderConfig::new(),
            &TestImage::rgba8_gradient(12, 9),
        )
        .unwrap();
    }

    /// The reference (PNM-labelled) codec reads ICC and CICP back and leaves the
    /// authority at the format's `Icc` default on every metadata mix.
    #[test]
    fn reference_color_authority_spec() {
        let (e, d) = ref_codecs();
        check_color_authority_spec(e, d, &TestImage::rgba8_gradient(12, 9)).unwrap();
    }

    /// The answer key encodes issue #11's Table 4 verbatim.
    #[test]
    fn expected_color_authority_table() {
        use ColorAuthority::{Cicp as C, Icc as I};
        use ImageFormat::*;
        let e = expected_color_authority;
        // CICP outranks ICC.
        for f in [Png, Jxl, Heic, Hdr] {
            assert_eq!(e(f, true, true), Some(C), "{f:?}");
            assert_eq!(e(f, true, false), Some(C), "{f:?}");
            assert_eq!(e(f, false, true), Some(I), "{f:?}");
            assert_eq!(e(f, false, false), Some(I), "{f:?}");
        }
        // MIAF: ICC colr outranks nclx.
        assert_eq!(e(Avif, true, true), Some(I));
        assert_eq!(e(Avif, false, true), Some(I));
        assert_eq!(e(Avif, true, false), Some(C));
        assert_eq!(e(Avif, false, false), Some(I));
        // ICC-only (or sRGB-assumed) formats never name CICP.
        for f in [
            Jpeg, WebP, Gif, Tiff, Bmp, Ico, Pnm, Farbfeld, Qoi, Tga, Dng, Raw, Pdf,
        ] {
            for (c, i) in [(true, true), (true, false), (false, true), (false, false)] {
                assert_eq!(e(f, c, i), Some(I), "{f:?} cicp={c} icc={i}");
            }
        }
        // No recorded rule.
        for f in [Exr, Jp2, Svg, Unknown] {
            assert_eq!(e(f, true, true), None, "{f:?}");
        }
    }

    /// The HEIC bug the audit opened with: an `nclx`-only file whose decoder
    /// left the authority at the `Icc` default. `to_color_context()` then drops
    /// the CICP and the pixels fall back to sRGB — invisible to a pixel round
    /// trip, caught here.
    #[test]
    fn source_color_authority_catches_nclx_only_default_authority() {
        let sc = SourceColor::default().with_cicp(Cicp::BT2100_PQ);
        let f = check_source_color_authority(ImageFormat::Heic, &sc).unwrap_err();
        assert_eq!(f.check, "source_color_authority");
        assert!(f.detail.contains("Cicp"), "{f}");
        assert!(
            check_source_color_authority(
                ImageFormat::Heic,
                &sc.with_color_authority(ColorAuthority::Cicp)
            )
            .is_ok()
        );
    }

    /// AVIF: an ICC `colr` box outranks `nclx` (MIAF), so naming CICP with an ICC
    /// present is the mismatch there — and a JPEG may never name CICP at all.
    #[test]
    fn source_color_authority_icc_first_formats() {
        let both = SourceColor::default()
            .with_cicp(Cicp::SRGB)
            .with_icc_profile(fixtures::sample_icc());
        assert!(check_source_color_authority(ImageFormat::Avif, &both).is_ok());
        let wrong = both.clone().with_color_authority(ColorAuthority::Cicp);
        assert!(check_source_color_authority(ImageFormat::Avif, &wrong).is_err());
        assert!(check_source_color_authority(ImageFormat::Jpeg, &wrong).is_err());
        // A format with no recorded rule never fails.
        assert!(check_source_color_authority(ImageFormat::Exr, &wrong).is_ok());
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

    // ---- EOF / truncation-series conformance ----

    /// The allowed/denied policy of [`is_incomplete_input_category`] over the
    /// origin-first [`ErrorCategory`] taxonomy: the entire image-bytes-origin
    /// [`Image`](ErrorCategory::Image) arm passes; every other origin — including the
    /// misattribution traps [`Resource`](ErrorCategory::Resource) (OOM / limits),
    /// [`Internal`](ErrorCategory::Internal), and [`Io`](ErrorCategory::Io) — is denied.
    #[test]
    fn incomplete_input_category_policy() {
        use zencodec::enough::StopReason;
        use zencodec::{
            CodecIoKind, ErrorCategory as E, ImageError, InternalKind, InvalidKind, LimitKind,
            PolicyKind, RequestError, ResourceError, UnsupportedImageKind, UnsupportedOperation,
        };

        // The whole `Image(_)` arm is allowed (UnexpectedEof ideal; Malformed and both
        // Unsupported kinds tolerated because a truncated prefix can look that way).
        for c in [
            E::Image(ImageError::UnexpectedEof),
            E::Image(ImageError::Malformed),
            E::Image(ImageError::Unsupported(UnsupportedImageKind::Type)),
            E::Image(ImageError::Unsupported(UnsupportedImageKind::Feature)),
        ] {
            assert!(
                is_incomplete_input_category(c),
                "{c:?} must be an allowed truncation category"
            );
        }

        // Every non-`Image` origin is denied (representative payload per arm).
        for c in [
            E::Request(RequestError::Unsupported(UnsupportedOperation::PixelFormat)),
            E::Request(RequestError::Unsupported(
                UnsupportedOperation::AnimationEncode,
            )),
            E::Request(RequestError::CmsRequired),
            E::Request(RequestError::Invalid(InvalidKind::Parameters)),
            E::Request(RequestError::Invalid(InvalidKind::Buffer)),
            E::Request(RequestError::Invalid(InvalidKind::State)),
            E::Policy(PolicyKind::Decode),
            E::Policy(PolicyKind::Encode),
            E::Stopped(StopReason::Cancelled),
            E::Stopped(StopReason::TimedOut),
            E::Resource(ResourceError::Limits(LimitKind::Pixels)),
            E::Resource(ResourceError::OutOfMemory),
            E::Io(CodecIoKind::opaque()),
            E::Internal(InternalKind::Bug),
            E::Internal(InternalKind::Dependency),
        ] {
            assert!(
                !is_incomplete_input_category(c),
                "{c:?} misattributes a client-side truncation and must be denied"
            );
        }
    }

    /// The series generator: small header sizes + fractions all present, every value
    /// `< len`, strictly increasing (sorted + deduped), and correct for tiny `len`.
    #[test]
    fn truncation_series_generator() {
        let len = 279usize; // a realistic small reference-image length
        let s = truncation_lengths(len);

        assert!(
            s.windows(2).all(|w| w[0] < w[1]),
            "strictly increasing (sorted + deduped): {s:?}"
        );
        assert!(s.iter().all(|&n| n < len), "every value < len: {s:?}");
        for n in [0usize, 1, 2, 3, 4, 8, 16] {
            assert!(s.contains(&n), "small header size {n} present: {s:?}");
        }
        for (num, den) in [(1, 8), (1, 4), (3, 8), (1, 2), (5, 8), (3, 4), (7, 8)] {
            assert!(
                s.contains(&(len * num / den)),
                "fraction {num}/{den} present: {s:?}"
            );
        }
        assert!(s.contains(&(len - 1)), "len-1 present: {s:?}");

        // Tiny lengths: everything clamps to `< len`, deduped, no underflow panic.
        assert_eq!(truncation_lengths(5), vec![0, 1, 2, 3, 4]);
        assert_eq!(truncation_lengths(2), vec![0, 1]);
    }

    /// End-to-end foil: `reference` is Pattern A (`type Error = RefError`). Its
    /// truncations classify fine on the typed path, but the category is
    /// unrecoverable once the dyn boundary erases it to `BoxedError`, so every
    /// truncated offset yields a `None` category and the check FAILs — naming the
    /// Pattern-A cause exactly once (not once per offset), the same limitation
    /// [`check_decode_error_envelope`] catches.
    #[test]
    fn reference_pattern_a_fails_truncation_series() {
        let (e, d) = ref_codecs();
        let img = TestImage::rgba8_gradient(9, 6);
        let valid = enc_oneshot(&e, &img, Metadata::none(), MetadataPolicy::PreserveExact).unwrap();
        assert!(valid.len() > 16, "reference image should be non-trivial");

        let err = check_decode_truncation_series(d, &valid)
            .expect_err("Pattern A must fail the truncation-series check on the erased path");
        assert_eq!(err.check, "decode_truncation_series");
        assert!(
            err.detail.contains("Pattern A"),
            "the failure should name the Pattern-A cause: {}",
            err.detail
        );
        assert_eq!(
            err.detail.matches("Pattern A").count(),
            1,
            "the Pattern-A limitation is reported once, not per truncated offset: {}",
            err.detail
        );
    }

    /// End-to-end positive: `minimal` is Pattern B (`type Error = At<CodecError>`).
    /// Every truncation classifies as `MalformedImage` — an allowed incomplete-input
    /// category that survives dyn erasure — with no panic and no silent decode of a
    /// truncated header, so the check PASSES.
    #[test]
    fn minimal_passes_truncation_series() {
        let img = TestImage::rgba8_gradient(9, 6);
        let valid = enc_oneshot(
            &MinimalEncoderConfig::new(),
            &img,
            Metadata::none(),
            MetadataPolicy::PreserveExact,
        )
        .unwrap();
        check_decode_truncation_series(MinimalDecoderConfig::new(), &valid).unwrap();
    }

    /// A too-short `valid` is rejected up front — the check needs a real encoded
    /// image to truncate, not a stray byte.
    #[test]
    fn truncation_series_rejects_tiny_valid() {
        let err = check_decode_truncation_series(MinimalDecoderConfig::new(), &[0x00])
            .expect_err("a 1-byte `valid` cannot be a real encoded image");
        assert_eq!(err.check, "decode_truncation_series");
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

    // ---- native HDR conformance (zencodec#24, Phase 0) ----

    /// The reference declares `hdr` + `native_16bit` on both ends and stores
    /// CLLI/MDCV + 16-bit samples raw, stamping the descriptor from CICP, so PQ
    /// and HLG round-trip in full.
    #[test]
    fn reference_native_hdr_roundtrip() {
        let (e, d) = ref_codecs();
        check_native_hdr_roundtrip(e, d).unwrap();
    }

    /// An SDR-only codec (no `hdr`) is out of scope, not a failure.
    #[test]
    fn minimal_native_hdr_roundtrip_skipped() {
        check_native_hdr_roundtrip(MinimalEncoderConfig::new(), MinimalDecoderConfig::new())
            .unwrap();
    }

    /// The 16-bit HDR decode is labelled exactly `RGB16_BT2100_PQ` — the
    /// stamped descriptor equals the constant, so downstream `==` checks on the
    /// canonical descriptors hold.
    #[test]
    fn reference_hdr_decode_is_labelled_pq() {
        let (e, d) = ref_codecs();
        let img = TestImage::rgb16_gradient(6, 5, PixelDescriptor::RGB16_BT2100_PQ);
        let meta = Metadata::none().with_cicp(Cicp::BT2100_PQ);
        let bytes = enc_oneshot(&e, &img, meta, MetadataPolicy::PreserveExact).unwrap();
        let out = dec_output(&d, &bytes).unwrap();
        assert_eq!(out.pixels().descriptor(), PixelDescriptor::RGB16_BT2100_PQ);
        assert_eq!(grab(out.pixels()), img.pixels());
    }

    /// Tolerance is container fixed-point, not slack: a 1/50000 xy step passes,
    /// a real drift fails, and luminance is relative.
    #[test]
    fn mastering_display_tolerance_is_fixed_point_tight() {
        let want = MasteringDisplay::HDR10_REFERENCE;
        let mut quantized = want;
        quantized.primaries_xy[0][0] += 1e-5; // within one 1/50000 step
        quantized.max_luminance = 10000.05; // 5e-6 relative
        assert!(mastering_display_matches(&quantized, &want));
        let mut drifted = want;
        drifted.primaries_xy[0][0] += 1e-3;
        assert!(!mastering_display_matches(&drifted, &want));
        let mut dark = want;
        dark.min_luminance = 0.0;
        assert!(
            !mastering_display_matches(&dark, &want),
            "a zeroed min luminance (0.0001 → 0) is a drop, not precision"
        );
    }

    // ---- buffer colour-context conformance (zencodec#25) ----

    /// The reference attaches a class-gated context on every path, so both the
    /// lenient consistency check and the strict attached check pass.
    #[test]
    fn reference_color_context_consistency() {
        let (e, d) = ref_codecs();
        check_color_context_consistency(e, d, &TestImage::rgba8_gradient(11, 9)).unwrap();
    }

    #[test]
    fn reference_color_context_attached() {
        let (e, d) = ref_codecs();
        check_color_context_attached(e, d, &TestImage::rgb8_gradient(11, 9)).unwrap();
    }

    /// The minimal codec drops every colour channel, so it attaches nothing and
    /// reads nothing back: the lenient check passes (nothing to validate) and the
    /// strict check passes (nothing to attach).
    #[test]
    fn minimal_color_context_checks_pass_without_color() {
        let img = TestImage::rgba8_gradient(8, 6);
        check_color_context_consistency(
            MinimalEncoderConfig::new(),
            MinimalDecoderConfig::new(),
            &img,
        )
        .unwrap();
        check_color_context_attached(
            MinimalEncoderConfig::new(),
            MinimalDecoderConfig::new(),
            &img,
        )
        .unwrap();
    }

    /// The class gate has teeth: an ICC rides only a layout its device class
    /// describes, an unreadable class rides nothing, and an empty context is
    /// rejected — while class-matching and CICP-only contexts pass.
    #[test]
    fn buffer_context_class_gate() {
        use zenpixels::PixelDescriptor as P;
        let rgb_icc = ColorContext::from_icc(fixtures::sample_icc_with_class(b"RGB "));
        let gray_icc = ColorContext::from_icc(fixtures::sample_icc_with_class(b"GRAY"));
        let junk_icc = ColorContext::from_icc(fixtures::sample_icc_with_class(b"????"));
        let cicp_only = ColorContext::from_cicp(Cicp::BT2100_PQ);

        assert!(validate_buffer_context(None, P::RGB8_SRGB).is_ok());
        assert!(validate_buffer_context(Some(&rgb_icc), P::RGB8_SRGB).is_ok());
        assert!(validate_buffer_context(Some(&rgb_icc), P::RGBA8_SRGB).is_ok());
        assert!(validate_buffer_context(Some(&gray_icc), P::GRAY8_SRGB).is_ok());
        assert!(validate_buffer_context(Some(&cicp_only), P::GRAY8_SRGB).is_ok());

        let e = validate_buffer_context(Some(&gray_icc), P::RGB8_SRGB).unwrap_err();
        assert!(e.contains("device class Gray"), "{e}");
        let e = validate_buffer_context(Some(&rgb_icc), P::GRAY8_SRGB).unwrap_err();
        assert!(e.contains("device class Rgb"), "{e}");
        let e = validate_buffer_context(Some(&junk_icc), P::RGB8_SRGB).unwrap_err();
        assert!(e.contains("unreadable"), "{e}");
        let e = validate_buffer_context(Some(&ColorContext::default()), P::RGB8_SRGB).unwrap_err();
        assert!(e.contains("neither ICC nor CICP"), "{e}");
    }

    /// The reference's worked-example gate: a class-matching ICC rides as-is
    /// (drop-dupe: the CICP is dropped under ICC authority); a class-mismatched
    /// one is replaced by a CICP-only description (the signaled CICP, since the
    /// synthetic fixture has no derivable CICP); nothing describable → `None`.
    #[test]
    fn reference_class_gate_ranks_the_fallbacks() {
        use zencodec::decode::SourceColor;
        use zenpixels::ColorModel;

        let rgb = Arc::<[u8]>::from(fixtures::sample_icc_with_class(b"RGB "));
        let gray = Arc::<[u8]>::from(fixtures::sample_icc_with_class(b"GRAY"));

        let both = SourceColor::default()
            .with_icc_profile(rgb.clone())
            .with_cicp(Cicp::SRGB);
        let ctx = crate::reference::class_gated_context(&both, ColorModel::Rgb).expect("attached");
        assert_eq!(
            ctx.icc.as_deref(),
            Some(&*rgb),
            "class-matching ICC rides as-is"
        );
        assert_eq!(ctx.cicp, None, "drop-dupe: ICC is the authority");

        let mismatched = SourceColor::default()
            .with_icc_profile(gray.clone())
            .with_cicp(Cicp::SRGB);
        let ctx = crate::reference::class_gated_context(&mismatched, ColorModel::Rgb)
            .expect("falls back to a CICP-only description");
        assert_eq!(
            ctx.icc, None,
            "a GRAY-class profile never rides an RGB buffer"
        );
        assert_eq!(ctx.cicp, Some(Cicp::SRGB));

        let underivable = SourceColor::default().with_icc_profile(gray);
        assert!(
            crate::reference::class_gated_context(&underivable, ColorModel::Rgb).is_none(),
            "nothing describable → no context (not an empty one)"
        );
        assert!(
            crate::reference::class_gated_context(&SourceColor::default(), ColorModel::Rgb)
                .is_none()
        );
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
            zencodec::ErrorCategory::Image(zencodec::ImageError::Malformed),
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
