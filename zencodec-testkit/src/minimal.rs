//! A deliberately minimal codec: one-shot encode/decode only, drops all
//! metadata, and declares every optional capability *false*.
//!
//! It exists so the testkit can validate the *false-direction* branches of
//! [`check_capability_honesty`](crate::check_capability_honesty) against a
//! known-correct "supports nothing extra" codec — the mirror of the full
//! [`reference`](crate::reference) codec, which exercises the true branches.
//!
//! It also demonstrates the **envelope error pattern**: `type Error =
//! At<CodecError>`. A generic consumer recovers the
//! [`ErrorCategory`](zencodec::ErrorCategory) from a type-erased
//! `Box<dyn Error>` — e.g. the `BoxedError` left after dyn dispatch — by
//! downcasting to the concrete `At<CodecError>`. That is the contrast with
//! [`reference`](crate::reference), whose `type Error = RefError` only
//! classifies on the *typed* path: once erased, all you hold is a `dyn Error`,
//! not a `dyn CategorizedError`, so there is no concrete type to downcast to.
//!
//! Adoption cost is one impl: it shares the reference's wire format and its
//! [`RefError`] kind, bridged by `From<RefError> for At<CodecError>`, so every
//! `?` on a `Result<_, RefError>` auto-wraps into the envelope with no rewrite.
//! Rejected decode modes use the generic [`Unsupported`] stub.

use std::borrow::Cow;

use whereat::At;
use zencodec::decode::{
    Decode, DecodeCapabilities, DecodeJob, DecodeOutput, DecodeRowSink, DecoderConfig, OutputInfo,
    SinkError,
};
use zencodec::encode::{EncodeCapabilities, EncodeJob, EncodeOutput, Encoder, EncoderConfig};
use zencodec::{
    CodecError, ImageFormat, ImageInfo, Metadata, Orientation, ResourceLimits, StopToken,
    Unsupported, UnsupportedOperation,
};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

use crate::reference::{
    Header, RefError, build_info, descriptor_for_bpp, encode_single, frame_pixel_len,
    frame_pixels_offset, parse_header,
};

/// Bridge a decode-sink error into the envelope. `copy_decode_to_sink` wants a
/// `fn` pointer, so this is a named function rather than a closure.
fn wrap_sink(e: SinkError) -> At<CodecError> {
    RefError::Sink(e).into()
}

// Only the always-available capabilities are declared. Everything optional —
// push_rows, encode_from, animation, lossless, stop, icc/exif/xmp/cicp — is
// false, and the codec genuinely supports none of it.
static ENCODE_CAPS: EncodeCapabilities = EncodeCapabilities::new().with_native_alpha(true);
static DECODE_CAPS: DecodeCapabilities = DecodeCapabilities::new()
    .with_cheap_probe(true)
    .with_native_alpha(true);

// ===========================================================================
// Encode
// ===========================================================================

/// Minimal one-shot-only encoder configuration.
#[derive(Clone, Debug, Default)]
pub struct MinimalEncoderConfig;

impl MinimalEncoderConfig {
    /// Construct a fresh config.
    pub fn new() -> Self {
        Self
    }
}

impl EncoderConfig for MinimalEncoderConfig {
    type Error = At<CodecError>;
    type Job = MinEncodeJob;

    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB]
    }
    fn capabilities() -> &'static EncodeCapabilities {
        &ENCODE_CAPS
    }
    fn job(self) -> MinEncodeJob {
        MinEncodeJob {
            orientation: Orientation::Identity,
        }
    }
}

/// Minimal encode job.
pub struct MinEncodeJob {
    orientation: Orientation,
}

impl EncodeJob for MinEncodeJob {
    type Error = At<CodecError>;
    type Enc = MinEnc;
    // `()` is the standard rejection stub for a still-only codec (the shape real
    // codecs use). It also pins the testkit's animation bounds to NOT require
    // `AnimationFrameEnc::Error == E::Error` — `<() as AnimationFrameEncoder>::Error`
    // is `UnsupportedOperation`, not the job's error.
    type AnimationFrameEnc = ();

    fn with_stop(self, _stop: StopToken) -> Self {
        self
    }
    fn with_limits(self, _limits: ResourceLimits) -> Self {
        self
    }
    // Keeps only orientation (display-critical); drops every metadata *channel*
    // (icc/exif/xmp/cicp), which it declares no capability for.
    fn with_metadata(mut self, meta: Metadata) -> Self {
        self.orientation = meta.orientation;
        self
    }
    fn encoder(self) -> Result<MinEnc, At<CodecError>> {
        Ok(MinEnc {
            orientation: self.orientation,
        })
    }
    fn animation_frame_encoder(self) -> Result<(), At<CodecError>> {
        Err(RefError::Unsupported(UnsupportedOperation::AnimationEncode).into())
    }
}

/// Minimal one-shot encoder. `push_rows` / `finish` / `encode_from` fall through
/// to the trait defaults, which reject with `UnsupportedOperation`.
pub struct MinEnc {
    orientation: Orientation,
}

impl Encoder for MinEnc {
    type Error = At<CodecError>;

    fn reject(op: UnsupportedOperation) -> At<CodecError> {
        RefError::Unsupported(op).into()
    }

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, At<CodecError>> {
        // Orientation only — no metadata channels.
        let meta = Metadata::none().with_orientation(self.orientation);
        Ok(EncodeOutput::new(
            encode_single(pixels, &meta),
            ImageFormat::Pnm,
        ))
    }
}

// ===========================================================================
// Decode
// ===========================================================================

/// Minimal one-shot-only decoder configuration.
#[derive(Clone, Debug, Default)]
pub struct MinimalDecoderConfig;

impl MinimalDecoderConfig {
    /// Construct a fresh config.
    pub fn new() -> Self {
        Self
    }
}

impl DecoderConfig for MinimalDecoderConfig {
    type Error = At<CodecError>;
    type Job<'a> = MinDecodeJob;

    fn formats() -> &'static [ImageFormat] {
        &[ImageFormat::Pnm]
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB]
    }
    fn capabilities() -> &'static DecodeCapabilities {
        &DECODE_CAPS
    }
    fn job<'a>(self) -> Self::Job<'a> {
        MinDecodeJob
    }
}

/// Minimal decode job.
pub struct MinDecodeJob;

impl<'a> DecodeJob<'a> for MinDecodeJob {
    type Error = At<CodecError>;
    type Dec = MinDec<'a>;
    // Streaming / animation are declared false and rejected before any stub is
    // built, so the generic `Unsupported` stub (Error = the job's error) suffices.
    type StreamDec = Unsupported<At<CodecError>>;
    type AnimationFrameDec = Unsupported<At<CodecError>>;

    fn with_stop(self, _stop: StopToken) -> Self {
        self
    }
    fn with_limits(self, _limits: ResourceLimits) -> Self {
        self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, At<CodecError>> {
        Ok(build_info(&parse_header(data)?))
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, At<CodecError>> {
        let h = parse_header(data)?;
        Ok(OutputInfo::full_decode(
            h.width,
            h.height,
            descriptor_for_bpp(h.bpp)?,
        ))
    }

    fn decoder(
        self,
        data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<MinDec<'a>, At<CodecError>> {
        parse_header(&data)?;
        Ok(MinDec { data })
    }

    fn push_decoder(
        self,
        data: Cow<'a, [u8]>,
        sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, At<CodecError>> {
        zencodec::helpers::copy_decode_to_sink(self, data, sink, preferred, wrap_sink)
    }

    fn streaming_decoder(
        self,
        _data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<Unsupported<At<CodecError>>, At<CodecError>> {
        Err(RefError::Unsupported(UnsupportedOperation::RowLevelDecode).into())
    }

    fn animation_frame_decoder(
        self,
        _data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<Unsupported<At<CodecError>>, At<CodecError>> {
        Err(RefError::Unsupported(UnsupportedOperation::AnimationDecode).into())
    }
}

/// Minimal one-shot decoder.
#[derive(Debug)]
pub struct MinDec<'a> {
    data: Cow<'a, [u8]>,
}

impl Decode for MinDec<'_> {
    type Error = At<CodecError>;

    fn decode(self) -> Result<DecodeOutput, At<CodecError>> {
        let h: Header = parse_header(&self.data)?;
        let desc = descriptor_for_bpp(h.bpp)?;
        let start = frame_pixels_offset(&h, 0);
        let len = frame_pixel_len(&h);
        let pixels = self
            .data
            .get(start..start + len)
            .ok_or_else(|| RefError::Invalid("truncated pixels".into()))?;
        let buf = PixelBuffer::from_vec(pixels.to_vec(), h.width, h.height, desc)
            .map_err(|e| RefError::Invalid(format!("buffer: {e}")))?;
        Ok(DecodeOutput::new(buf, build_info(&h)))
    }
}
