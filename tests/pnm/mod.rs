//! Minimal PNM (PPM P6 / PGM P5) codec implementing zencodec-types traits.
//!
//! Supports RGB8 and Gray8 only. Used as an integration test to exercise the
//! full Config → Job → Executor pipeline in both concrete and dyn-dispatch modes.
//!
//! Credits: PNM parsing adapted from zenbitmaps (MIT/Apache-2.0/Zlib).

use zc::decode::{Decode, DecodeCapabilities, DecodeJob, DecoderConfig};
use zc::encode::{EncodeCapabilities, EncodeJob, EncodeOutput, Encoder, EncoderConfig};
use zc::{ImageFormat, ImageInfo, MetadataView, ResourceLimits, UnsupportedOperation};

use zc::decode::{DecodeOutput, OutputInfo};
use enough::{Stop, StopReason};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

// =========================================================================
// Error
// =========================================================================

#[derive(Debug)]
pub enum PnmError {
    Unsupported(UnsupportedOperation),
    InvalidData(String),
    Cancelled(StopReason),
    LimitExceeded(zc::LimitExceeded),
}

impl core::fmt::Display for PnmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported(op) => write!(f, "unsupported: {op}"),
            Self::InvalidData(msg) => write!(f, "invalid data: {msg}"),
            Self::Cancelled(r) => write!(f, "cancelled: {r}"),
            Self::LimitExceeded(e) => write!(f, "limit exceeded: {e}"),
        }
    }
}

impl std::error::Error for PnmError {}

impl From<UnsupportedOperation> for PnmError {
    fn from(op: UnsupportedOperation) -> Self {
        Self::Unsupported(op)
    }
}

impl From<StopReason> for PnmError {
    fn from(r: StopReason) -> Self {
        Self::Cancelled(r)
    }
}

impl From<zc::LimitExceeded> for PnmError {
    fn from(e: zc::LimitExceeded) -> Self {
        Self::LimitExceeded(e)
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
pub struct PnmEncodeJob<'a> {
    limits: ResourceLimits,
    stop: Option<&'a dyn Stop>,
    metadata: Option<&'a MetadataView<'a>>,
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
    type Error = PnmError;
    type Job<'a> = PnmEncodeJob<'a>;

    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::GRAY8_SRGB]
    }

    fn capabilities() -> &'static EncodeCapabilities {
        &PNM_ENCODE_CAPS
    }

    fn job(&self) -> PnmEncodeJob<'_> {
        PnmEncodeJob {
            limits: ResourceLimits::none(),
            stop: None,
            metadata: None,
        }
    }
}

impl<'a> EncodeJob<'a> for PnmEncodeJob<'a> {
    type Error = PnmError;
    type Enc = PnmEnc;
    type FrameEnc = (); // no animation

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn with_metadata(mut self, meta: &'a MetadataView<'a>) -> Self {
        self.metadata = Some(meta);
        self
    }

    fn encoder(self) -> Result<PnmEnc, PnmError> {
        let stop: Option<Box<dyn Fn() -> Result<(), StopReason> + Send>> =
            self.stop.map(|s| {
                let _ = s.check();
                Box::new(|| Ok(())) as Box<dyn Fn() -> Result<(), StopReason> + Send>
            });
        Ok(PnmEnc { stop })
    }

    fn frame_encoder(self) -> Result<(), PnmError> {
        Err(UnsupportedOperation::AnimationEncode.into())
    }
}

impl Encoder for PnmEnc {
    type Error = PnmError;

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, PnmError> {
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
            )));
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
pub struct PnmDecodeJob<'a> {
    limits: ResourceLimits,
    stop: Option<&'a dyn Stop>,
}

/// The actual PPM/PGM decoder (data bound at construction).
pub struct PnmDec<'a> {
    data: &'a [u8],
}

/// Stub streaming decoder — PNM doesn't support streaming.
pub struct PnmStreamDec;

impl zc::decode::StreamingDecode for PnmStreamDec {
    type Error = PnmError;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, PnmError> {
        Err(UnsupportedOperation::RowLevelDecode.into())
    }

    fn info(&self) -> &ImageInfo {
        unreachable!("streaming decode not supported")
    }
}

/// Stub frame decoder — PNM doesn't support animation.
pub struct PnmFrameDec;

impl zc::decode::FrameDecode for PnmFrameDec {
    type Error = PnmError;

    fn next_frame(&mut self) -> Result<Option<zc::decode::DecodeFrame>, PnmError> {
        Err(UnsupportedOperation::AnimationDecode.into())
    }
}

static PNM_DECODE_CAPS: DecodeCapabilities = DecodeCapabilities::new()
    .with_cheap_probe(true)
    .with_native_gray(true);

impl DecoderConfig for PnmDecoderConfig {
    type Error = PnmError;
    type Job<'a> = PnmDecodeJob<'a>;

    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::GRAY8_SRGB]
    }

    fn capabilities() -> &'static DecodeCapabilities {
        &PNM_DECODE_CAPS
    }

    fn job(&self) -> PnmDecodeJob<'_> {
        PnmDecodeJob {
            limits: ResourceLimits::none(),
            stop: None,
        }
    }
}

impl<'a> DecodeJob<'a> for PnmDecodeJob<'a> {
    type Error = PnmError;
    type Dec = PnmDec<'a>;
    type StreamDec = PnmStreamDec;
    type FrameDec = PnmFrameDec;

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, PnmError> {
        let (w, h, _is_gray) = parse_pnm_header(data)?;
        let info = ImageInfo::new(w, h, ImageFormat::Pnm).with_frame_count(1);
        Ok(info)
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, PnmError> {
        let (w, h, is_gray) = parse_pnm_header(data)?;
        let desc = if is_gray {
            PixelDescriptor::GRAY8_SRGB
        } else {
            PixelDescriptor::RGB8_SRGB
        };
        Ok(OutputInfo::full_decode(w, h, desc))
    }

    fn decoder(
        self,
        data: &'a [u8],
        _preferred: &[PixelDescriptor],
    ) -> Result<PnmDec<'a>, PnmError> {
        let (w, h, _) = parse_pnm_header(data)?;
        self.limits.check_dimensions(w, h)?;
        Ok(PnmDec { data })
    }

    fn streaming_decoder(
        self,
        _data: &'a [u8],
        _preferred: &[PixelDescriptor],
    ) -> Result<PnmStreamDec, PnmError> {
        Err(UnsupportedOperation::RowLevelDecode.into())
    }

    fn frame_decoder(
        self,
        _data: &'a [u8],
        _preferred: &[PixelDescriptor],
    ) -> Result<PnmFrameDec, PnmError> {
        Err(UnsupportedOperation::AnimationDecode.into())
    }
}

impl<'a> Decode for PnmDec<'a> {
    type Error = PnmError;

    fn decode(self) -> Result<DecodeOutput, PnmError> {
        let (w, h, is_gray) = parse_pnm_header(self.data)?;
        let data_offset = find_data_offset(self.data)?;
        let pixel_data = &self.data[data_offset..];

        if is_gray {
            let expected = w as usize * h as usize;
            if pixel_data.len() < expected {
                return Err(PnmError::InvalidData("truncated pixel data".into()));
            }
            let desc = PixelDescriptor::GRAY8_SRGB;
            let buf = PixelBuffer::from_vec(pixel_data[..expected].to_vec(), w, h, desc)
                .map_err(|e| PnmError::InvalidData(format!("buffer error: {e}")))?;
            let info = ImageInfo::new(w, h, ImageFormat::Pnm);
            Ok(DecodeOutput::new(buf, info))
        } else {
            let expected = w as usize * h as usize * 3;
            if pixel_data.len() < expected {
                return Err(PnmError::InvalidData("truncated pixel data".into()));
            }
            let desc = PixelDescriptor::RGB8_SRGB;
            let buf = PixelBuffer::from_vec(pixel_data[..expected].to_vec(), w, h, desc)
                .map_err(|e| PnmError::InvalidData(format!("buffer error: {e}")))?;
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
