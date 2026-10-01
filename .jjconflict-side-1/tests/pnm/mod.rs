//! Minimal PNM (PPM P6 / PGM P5) codec implementing zencodec traits.
//!
//! Supports RGB8 and Gray8 only. Used as an integration test to exercise the
//! full Config → Job → Executor pipeline in both concrete and dyn-dispatch modes.
//!
//! Uses `thiserror` + `whereat::At<E>` for error derivation and location
//! tracking, validating that error chains and traces survive dyn dispatch.

use std::borrow::Cow;

use zencodec::decode::{Decode, DecodeCapabilities, DecodeJob, DecoderConfig};
use zencodec::encode::{EncodeCapabilities, EncodeJob, EncodeOutput, Encoder, EncoderConfig};
use zencodec::{
    ImageFormat, ImageInfo, Metadata, ResourceLimits, Unsupported, UnsupportedOperation,
};

use enough::{Stop, StopReason};
use whereat::{At, ErrorAtExt};
use zencodec::decode::{DecodeOutput, OutputInfo};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

// =========================================================================
// Error
// =========================================================================

#[derive(Debug, thiserror::Error)]
pub enum PnmError {
    #[error("unsupported: {0}")]
    Unsupported(#[from] UnsupportedOperation),
    #[error("invalid data: {0}")]
    InvalidData(String),
    #[error("cancelled: {0}")]
    Cancelled(StopReason),
    #[error("limit exceeded: {0}")]
    LimitExceeded(#[from] zencodec::LimitExceeded),
}

/// Manual impl because `StopReason` doesn't implement `Error`,
/// so thiserror's `#[from]` can't be used.
impl From<StopReason> for PnmError {
    fn from(r: StopReason) -> Self {
        PnmError::Cancelled(r)
    }
}

// =========================================================================
// Encode: Config → Job → Encoder
// =========================================================================

/// PNM encoder configuration (PPM P6 / PGM P5).
#[derive(Clone, Debug)]
pub struct PnmEncoderConfig;

impl PnmEncoderConfig {
    pub fn new() -> Self {
        Self
    }
}

/// Per-operation encode job.
pub struct PnmEncodeJob {
    limits: ResourceLimits,
    stop: Option<zencodec::StopToken>,
    metadata: Option<Metadata>,
}

/// The actual PPM/PGM encoder.
pub struct PnmEnc {
    #[allow(dead_code)]
    stop: Option<Box<dyn Fn() -> Result<(), StopReason> + Send>>,
}

static PNM_ENCODE_CAPS: EncodeCapabilities = EncodeCapabilities::new()
    .with_lossless(true)
    .with_native_gray(true);

impl EncoderConfig for PnmEncoderConfig {
    type Error = At<PnmError>;
    type Job = PnmEncodeJob;

    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::GRAY8_SRGB]
    }

    fn capabilities() -> &'static EncodeCapabilities {
        &PNM_ENCODE_CAPS
    }

    fn job(self) -> PnmEncodeJob {
        PnmEncodeJob {
            limits: ResourceLimits::none(),
            stop: None,
            metadata: None,
        }
    }
}

impl EncodeJob for PnmEncodeJob {
    type Error = At<PnmError>;
    type Enc = PnmEnc;
    type AnimationFrameEnc = (); // no animation

    fn with_stop(mut self, stop: zencodec::StopToken) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn with_metadata(mut self, meta: Metadata) -> Self {
        self.metadata = Some(meta);
        self
    }

    fn encoder(self) -> Result<PnmEnc, At<PnmError>> {
        let stop: Option<Box<dyn Fn() -> Result<(), StopReason> + Send>> = self
            .stop
            .map(|s| Box::new(move || s.check()) as Box<dyn Fn() -> Result<(), StopReason> + Send>);
        Ok(PnmEnc { stop })
    }

    fn animation_frame_encoder(self) -> Result<(), At<PnmError>> {
        Err(PnmError::from(UnsupportedOperation::AnimationEncode).start_at())
    }
}

impl Encoder for PnmEnc {
    type Error = At<PnmError>;

    fn reject(op: UnsupportedOperation) -> At<PnmError> {
        PnmError::from(op).start_at()
    }

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, At<PnmError>> {
        let desc = pixels.descriptor();
        let w = pixels.width();
        let h = pixels.rows();

        let is_rgb = desc.layout() == zenpixels::ChannelLayout::Rgb
            && desc.channel_type() == zenpixels::ChannelType::U8;
        let is_gray = desc.layout() == zenpixels::ChannelLayout::Gray
            && desc.channel_type() == zenpixels::ChannelType::U8;

        if !is_rgb && !is_gray {
            return Err(PnmError::InvalidData(format!(
                "PNM encoder only supports RGB8 and Gray8, got {:?}",
                desc
            ))
            .start_at());
        }

        if is_gray {
            // P5 (PGM)
            let header = format!("P5\n{w} {h}\n255\n");
            let mut out = Vec::with_capacity(header.len() + (w * h) as usize);
            out.extend_from_slice(header.as_bytes());
            for y in 0..h {
                out.extend_from_slice(pixels.row(y));
            }
            Ok(EncodeOutput::new(out, ImageFormat::Pnm))
        } else {
            // P6 (PPM)
            let header = format!("P6\n{w} {h}\n255\n");
            let row_bytes = w as usize * 3;
            let mut out = Vec::with_capacity(header.len() + row_bytes * h as usize);
            out.extend_from_slice(header.as_bytes());
            for y in 0..h {
                let row = pixels.row(y);
                out.extend_from_slice(&row[..row_bytes]);
            }
            Ok(EncodeOutput::new(out, ImageFormat::Pnm))
        }
    }
}

// =========================================================================
// Decode: Config → Job → Decoder
// =========================================================================

/// PNM decoder configuration.
#[derive(Clone, Debug)]
pub struct PnmDecoderConfig;

impl PnmDecoderConfig {
    pub fn new() -> Self {
        Self
    }
}

/// Per-operation decode job.
pub struct PnmDecodeJob {
    limits: ResourceLimits,
    stop: Option<zencodec::StopToken>,
}

/// The actual PPM/PGM decoder (data bound at construction).
pub struct PnmDec<'a> {
    data: Cow<'a, [u8]>,
}

static PNM_DECODE_CAPS: DecodeCapabilities = DecodeCapabilities::new()
    .with_cheap_probe(true)
    .with_native_gray(true);

impl DecoderConfig for PnmDecoderConfig {
    type Error = At<PnmError>;
    type Job<'a> = PnmDecodeJob;

    fn formats() -> &'static [ImageFormat] {
        &[ImageFormat::Pnm]
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::GRAY8_SRGB]
    }

    fn capabilities() -> &'static DecodeCapabilities {
        &PNM_DECODE_CAPS
    }

    fn job<'a>(self) -> Self::Job<'a> {
        PnmDecodeJob {
            limits: ResourceLimits::none(),
            stop: None,
        }
    }
}

impl<'a> DecodeJob<'a> for PnmDecodeJob {
    type Error = At<PnmError>;
    type Dec = PnmDec<'a>;
    type StreamDec = Unsupported<At<PnmError>>;
    type AnimationFrameDec = Unsupported<At<PnmError>>;

    fn with_stop(mut self, stop: zencodec::StopToken) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, At<PnmError>> {
        let (w, h, _is_gray) = parse_pnm_header(data).map_err(|e| e.start_at())?;
        let info = ImageInfo::new(w, h, ImageFormat::Pnm);
        Ok(info)
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, At<PnmError>> {
        let (w, h, is_gray) = parse_pnm_header(data).map_err(|e| e.start_at())?;
        let desc = if is_gray {
            PixelDescriptor::GRAY8_SRGB
        } else {
            PixelDescriptor::RGB8_SRGB
        };
        Ok(OutputInfo::full_decode(w, h, desc))
    }

    fn decoder(
        self,
        data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<PnmDec<'a>, At<PnmError>> {
        let (w, h, _) = parse_pnm_header(&data).map_err(|e| e.start_at())?;
        self.limits
            .check_dimensions(w, h)
            .map_err(|e| PnmError::from(e).start_at())?;
        Ok(PnmDec { data })
    }

    fn push_decoder(
        self,
        data: Cow<'a, [u8]>,
        sink: &mut dyn zencodec::decode::DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, At<PnmError>> {
        zencodec::helpers::copy_decode_to_sink(self, data, sink, preferred, |e| {
            PnmError::InvalidData(e.to_string()).start_at()
        })
    }

    fn streaming_decoder(
        self,
        _data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<Unsupported<At<PnmError>>, At<PnmError>> {
        Err(PnmError::from(UnsupportedOperation::RowLevelDecode).start_at())
    }

    fn animation_frame_decoder(
        self,
        _data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<Unsupported<At<PnmError>>, At<PnmError>> {
        Err(PnmError::from(UnsupportedOperation::AnimationDecode).start_at())
    }
}

impl<'a> Decode for PnmDec<'a> {
    type Error = At<PnmError>;

    fn decode(self) -> Result<DecodeOutput, At<PnmError>> {
        let (w, h, is_gray) = parse_pnm_header(&self.data).map_err(|e| e.start_at())?;
        let data_offset = find_data_offset(&self.data).map_err(|e| e.start_at())?;
        let pixel_data = &self.data[data_offset..];

        if is_gray {
            let expected = w as usize * h as usize;
            if pixel_data.len() < expected {
                return Err(PnmError::InvalidData("truncated pixel data".into()).start_at());
            }
            let desc = PixelDescriptor::GRAY8_SRGB;
            let buf = PixelBuffer::from_vec(pixel_data[..expected].to_vec(), w, h, desc)
                .map_err(|e| PnmError::InvalidData(format!("buffer error: {e}")).start_at())?;
            let info = ImageInfo::new(w, h, ImageFormat::Pnm);
            Ok(DecodeOutput::new(buf, info))
        } else {
            let expected = w as usize * h as usize * 3;
            if pixel_data.len() < expected {
                return Err(PnmError::InvalidData("truncated pixel data".into()).start_at());
            }
            let desc = PixelDescriptor::RGB8_SRGB;
            let buf = PixelBuffer::from_vec(pixel_data[..expected].to_vec(), w, h, desc)
                .map_err(|e| PnmError::InvalidData(format!("buffer error: {e}")).start_at())?;
            let info = ImageInfo::new(w, h, ImageFormat::Pnm);
            Ok(DecodeOutput::new(buf, info))
        }
    }
}

// =========================================================================
// PNM header parsing (P5/P6 only, simplified)
// =========================================================================

/// Parse PNM header, returns (width, height, is_gray).
fn parse_pnm_header(data: &[u8]) -> Result<(u32, u32, bool), PnmError> {
    if data.len() < 3 {
        return Err(PnmError::InvalidData("too short".into()));
    }
    let is_gray = match &data[..2] {
        b"P5" => true,
        b"P6" => false,
        _ => return Err(PnmError::InvalidData("not P5/P6 PNM".into())),
    };
    let mut pos = 2;
    pos = skip_ws_comments(data, pos)?;
    let (width, new_pos) = parse_u32_at(data, pos)?;
    pos = skip_ws_comments(data, new_pos)?;
    let (height, new_pos) = parse_u32_at(data, pos)?;
    pos = skip_ws_comments(data, new_pos)?;
    let (maxval, _) = parse_u32_at(data, pos)?;
    if width == 0 || height == 0 {
        return Err(PnmError::InvalidData("zero dimension".into()));
    }
    if maxval != 255 {
        return Err(PnmError::InvalidData(format!(
            "only maxval=255 supported, got {maxval}"
        )));
    }
    Ok((width, height, is_gray))
}

/// Find the byte offset where pixel data begins.
fn find_data_offset(data: &[u8]) -> Result<usize, PnmError> {
    let mut pos = 2;
    // Skip three tokens (width, height, maxval) + the single whitespace after maxval
    for _ in 0..3 {
        pos = skip_ws_comments(data, pos)?;
        while pos < data.len() && data[pos].is_ascii_digit() {
            pos += 1;
        }
    }
    if pos >= data.len() {
        return Err(PnmError::InvalidData("truncated header".into()));
    }
    Ok(pos + 1)
}

fn skip_ws_comments(data: &[u8], mut pos: usize) -> Result<usize, PnmError> {
    loop {
        if pos >= data.len() {
            return Err(PnmError::InvalidData("unexpected EOF in header".into()));
        }
        match data[pos] {
            b' ' | b'\t' | b'\n' | b'\r' => pos += 1,
            b'#' => {
                while pos < data.len() && data[pos] != b'\n' {
                    pos += 1;
                }
                if pos < data.len() {
                    pos += 1;
                }
            }
            _ => return Ok(pos),
        }
    }
}

fn parse_u32_at(data: &[u8], pos: usize) -> Result<(u32, usize), PnmError> {
    let mut end = pos;
    let max_end = core::cmp::min(pos + 11, data.len());
    while end < max_end && data[end].is_ascii_digit() {
        end += 1;
    }
    if end == pos {
        return Err(PnmError::InvalidData("expected number".into()));
    }
    let s = core::str::from_utf8(&data[pos..end])
        .map_err(|_| PnmError::InvalidData("non-UTF8".into()))?;
    let val: u32 = s
        .parse()
        .map_err(|_| PnmError::InvalidData(format!("number too large: {s}")))?;
    Ok((val, end))
}
