//! A deliberately minimal codec: one-shot encode/decode only, drops all
//! metadata, and declares every optional capability *false*.
//!
//! It exists so the testkit can validate the *false-direction* branches of
//! [`check_capability_honesty`](crate::check_capability_honesty) against a
//! known-correct "supports nothing extra" codec — the mirror of the full
//! [`reference`](crate::reference) codec, which exercises the true branches. It
//! shares the reference's wire format and error type, and reuses the reference's
//! executor types for the associated types it never constructs (it rejects those
//! paths before building them).

use std::borrow::Cow;

use zencodec::decode::{
    Decode, DecodeCapabilities, DecodeJob, DecodeOutput, DecodeRowSink, DecoderConfig, OutputInfo,
};
use zencodec::encode::{EncodeCapabilities, EncodeJob, EncodeOutput, Encoder, EncoderConfig};
use zencodec::{ImageFormat, ImageInfo, Metadata, ResourceLimits, StopToken, UnsupportedOperation};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

use crate::reference::{
    Header, RefAnimDec, RefAnimEnc, RefError, RefStreamDec, build_info, descriptor_for_bpp,
    encode_single, frame_pixel_len, frame_pixels_offset, parse_header,
};

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
    type Error = RefError;
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
        MinEncodeJob
    }
}

/// Minimal encode job.
pub struct MinEncodeJob;

impl EncodeJob for MinEncodeJob {
    type Error = RefError;
    type Enc = MinEnc;
    // Never constructed — animation is declared false and rejected below.
    type AnimationFrameEnc = RefAnimEnc;

    fn with_stop(self, _stop: StopToken) -> Self {
        self
    }
    fn with_limits(self, _limits: ResourceLimits) -> Self {
        self
    }
    // Drops metadata: the minimal codec declares no metadata channels.
    fn with_metadata(self, _meta: Metadata) -> Self {
        self
    }
    fn encoder(self) -> Result<MinEnc, RefError> {
        Ok(MinEnc)
    }
    fn animation_frame_encoder(self) -> Result<RefAnimEnc, RefError> {
        Err(RefError::Unsupported(UnsupportedOperation::AnimationEncode))
    }
}

/// Minimal one-shot encoder. `push_rows` / `finish` / `encode_from` fall through
/// to the trait defaults, which reject with `UnsupportedOperation`.
pub struct MinEnc;

impl Encoder for MinEnc {
    type Error = RefError;

    fn reject(op: UnsupportedOperation) -> RefError {
        RefError::Unsupported(op)
    }

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, RefError> {
        // Encode with empty metadata — the minimal codec embeds none.
        Ok(EncodeOutput::new(
            encode_single(pixels, &Metadata::none()),
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
    type Error = RefError;
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
    type Error = RefError;
    type Dec = MinDec<'a>;
    // Never constructed — streaming/animation are declared false and rejected.
    type StreamDec = RefStreamDec<'a>;
    type AnimationFrameDec = RefAnimDec;

    fn with_stop(self, _stop: StopToken) -> Self {
        self
    }
    fn with_limits(self, _limits: ResourceLimits) -> Self {
        self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, RefError> {
        Ok(build_info(&parse_header(data)?))
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, RefError> {
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
    ) -> Result<MinDec<'a>, RefError> {
        parse_header(&data)?;
        Ok(MinDec { data })
    }

    fn push_decoder(
        self,
        data: Cow<'a, [u8]>,
        sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, RefError> {
        zencodec::helpers::copy_decode_to_sink(self, data, sink, preferred, RefError::Sink)
    }

    fn streaming_decoder(
        self,
        _data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<RefStreamDec<'a>, RefError> {
        Err(RefError::Unsupported(UnsupportedOperation::RowLevelDecode))
    }

    fn animation_frame_decoder(
        self,
        _data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<RefAnimDec, RefError> {
        Err(RefError::Unsupported(UnsupportedOperation::AnimationDecode))
    }
}

/// Minimal one-shot decoder.
#[derive(Debug)]
pub struct MinDec<'a> {
    data: Cow<'a, [u8]>,
}

impl Decode for MinDec<'_> {
    type Error = RefError;

    fn decode(self) -> Result<DecodeOutput, RefError> {
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
