//! A deliberately minimal codec: one-shot encode/decode only, drops all
//! metadata, and declares every optional capability *false*.
//!
//! Expressed directly on [`zencodec::advanced::StillEncoderConfig`] and
//! [`zencodec::advanced::StillDecoderConfig`] so the conformance harness
//! simultaneously validates the *false-direction* branches of
//! [`check_capability_honesty`](crate::check_capability_honesty) AND proves
//! the simplified authoring adapters satisfy the full codec contract.

use std::borrow::Cow;

use whereat::At;
use zencodec::advanced::{StillDecoder, StillDecoderConfig, StillEncoder, StillEncoderConfig};
use zencodec::decode::{DecodeCapabilities, DecodeOutput, OutputInfo, SinkError};
use zencodec::encode::{EncodeCapabilities, EncodeOutput};
use zencodec::{
    CodecError, ImageFormat, ImageInfo, Metadata, Orientation, ResourceLimits, SharedPulse,
    UnsupportedOperation,
};
use zenpixels::{PixelBuf, PixelFormat, PixelView};

use crate::reference::{
    Header, RefError, build_info, descriptor_for_bpp, encode_single, frame_pixel_len,
    frame_pixels_offset, parse_header,
};

// Only the always-available capabilities are declared. Everything optional —
// push_rows, encode_from, animation, lossless, stop, icc/exif/xmp/cicp — is
// false, and the codec genuinely supports none of it.
static ENCODE_CAPS: EncodeCapabilities = EncodeCapabilities::new().with_native_alpha(true);
static DECODE_CAPS: DecodeCapabilities = DecodeCapabilities::new()
    .with_cheap_probe(true)
    .with_native_alpha(true);

static SUPPORTED_FORMATS: &[PixelFormat] = &[PixelFormat::RGB8_SRGB, PixelFormat::RGBA8_SRGB];

/// Minimal one-shot-only still encoder.
#[derive(Clone, Debug, Default)]
pub struct MinimalEncoder;

impl StillEncoder for MinimalEncoder {
    type Error = At<CodecError>;

    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }

    fn supported_pixel_formats() -> &'static [PixelFormat] {
        SUPPORTED_FORMATS
    }

    fn capabilities() -> &'static EncodeCapabilities {
        &ENCODE_CAPS
    }

    fn reject(op: UnsupportedOperation) -> At<CodecError> {
        RefError::Unsupported(op).into()
    }

    fn encode_still(
        self,
        pixels: PixelView<'_>,
        metadata: Option<Metadata>,
        _limits: ResourceLimits,
        _pulse: SharedPulse,
    ) -> Result<EncodeOutput, At<CodecError>> {
        // Keeps only orientation (display-critical); drops every metadata
        // channel (icc/exif/xmp/cicp), which it declares no capability for.
        let orientation = metadata.map_or(Orientation::Identity, |m| m.orientation);
        let meta = Metadata::none().with_orientation(orientation);
        Ok(EncodeOutput::new(
            encode_single(pixels, &meta, None)?,
            ImageFormat::Pnm,
        ))
    }
}

/// Minimal one-shot-only encoder configuration backed by [`StillEncoderConfig`].
pub type MinimalEncoderConfig = StillEncoderConfig<MinimalEncoder>;

/// Minimal one-shot-only still decoder.
#[derive(Clone, Debug, Default)]
pub struct MinimalDecoder;

impl StillDecoder for MinimalDecoder {
    type Error = At<CodecError>;

    fn formats() -> &'static [ImageFormat] {
        &[ImageFormat::Pnm]
    }

    fn supported_pixel_formats() -> &'static [PixelFormat] {
        SUPPORTED_FORMATS
    }

    fn capabilities() -> &'static DecodeCapabilities {
        &DECODE_CAPS
    }

    fn reject(op: UnsupportedOperation) -> At<CodecError> {
        RefError::Unsupported(op).into()
    }

    fn wrap_sink_error(err: SinkError) -> At<CodecError> {
        RefError::Sink(err).into()
    }

    fn probe_still(
        &self,
        data: &[u8],
        _limits: &ResourceLimits,
    ) -> Result<ImageInfo, At<CodecError>> {
        Ok(build_info(&parse_header(data)?))
    }

    fn output_info_still(
        &self,
        data: &[u8],
        _limits: &ResourceLimits,
    ) -> Result<OutputInfo, At<CodecError>> {
        let h = parse_header(data)?;
        Ok(OutputInfo::full_decode(
            h.width,
            h.height,
            descriptor_for_bpp(h.bpp)?,
        ))
    }

    fn decode_still(
        self,
        data: Cow<'_, [u8]>,
        _preferred: &[PixelFormat],
        _limits: ResourceLimits,
        _pulse: SharedPulse,
    ) -> Result<DecodeOutput, At<CodecError>> {
        let h: Header = parse_header(&data)?;
        let desc = descriptor_for_bpp(h.bpp)?;
        let start = frame_pixels_offset(&h, 0);
        let len = frame_pixel_len(&h);
        let pixels = data
            .get(start..start + len)
            .ok_or_else(|| RefError::Invalid("truncated pixels".into()))?;
        let buf = PixelBuf::from_bytes(pixels.to_vec(), h.width, h.height, desc)
            .map_err(|e| RefError::Invalid(format!("buffer: {e}")))?;
        Ok(DecodeOutput::new(buf, build_info(&h)))
    }
}

/// Minimal one-shot-only decoder configuration backed by [`StillDecoderConfig`].
pub type MinimalDecoderConfig = StillDecoderConfig<MinimalDecoder>;
