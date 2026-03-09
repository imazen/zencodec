//! Mock animation codec implementing zencodec-types traits.
//!
//! Supports RGB8 and RGBA8. Implements:
//! - One-shot decode (Decode)
//! - Streaming decode (StreamingDecode)
//! - Full-frame animation decode (FullFrameDecoder)
//! - One-shot encode (Encoder) with push_rows + encode_from
//! - Full-frame animation encode (FullFrameEncoder)
//!
//! Used to exercise trait paths that the PNM codec doesn't cover.

use std::borrow::Cow;

use zc::decode::{
    DecodeCapabilities, DecodeJob, DecodeOutput, DecoderConfig, FullFrameDecoder, OutputInfo,
    StreamingDecode,
};
use zc::encode::{
    EncodeCapabilities, EncodeJob, EncodeOutput, Encoder, EncoderConfig, FullFrameEncoder,
};
use zc::{FullFrame, ImageFormat, ImageInfo, Metadata, ResourceLimits, UnsupportedOperation};

use enough::{Stop, StopReason};
use zc::decode::{DecodeRowSink, SinkError};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice, PixelSliceMut};

// =========================================================================
// Error
// =========================================================================

#[derive(Debug)]
pub enum MockError {
    Unsupported(UnsupportedOperation),
    InvalidData(String),
    Cancelled(StopReason),
    LimitExceeded(zc::LimitExceeded),
    Sink(SinkError),
}

impl std::fmt::Display for MockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(e) => write!(f, "mock: unsupported: {e}"),
            Self::InvalidData(s) => write!(f, "mock: invalid: {s}"),
            Self::Cancelled(r) => write!(f, "mock: cancelled: {r}"),
            Self::LimitExceeded(e) => write!(f, "mock: limit: {e}"),
            Self::Sink(e) => write!(f, "mock: sink: {e}"),
        }
    }
}

impl std::error::Error for MockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unsupported(e) => Some(e),
            Self::LimitExceeded(e) => Some(e),
            Self::Sink(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<UnsupportedOperation> for MockError {
    fn from(e: UnsupportedOperation) -> Self {
        Self::Unsupported(e)
    }
}

impl From<StopReason> for MockError {
    fn from(r: StopReason) -> Self {
        Self::Cancelled(r)
    }
}

impl From<zc::LimitExceeded> for MockError {
    fn from(e: zc::LimitExceeded) -> Self {
        Self::LimitExceeded(e)
    }
}

// =========================================================================
// Wire format: trivial header + raw pixels
// =========================================================================
// Header: "MOCK" (4) + width:u32 LE + height:u32 LE + frame_count:u32 LE
//         + bpp:u8 (3=RGB8, 4=RGBA8) + duration_ms:u32 LE per frame
// Then raw pixel data per frame: width * height * bpp bytes

const HEADER_SIZE: usize = 4 + 4 + 4 + 4 + 1;

fn encode_mock_data(frames: &[(PixelSlice<'_>, u32)]) -> Vec<u8> {
    assert!(!frames.is_empty());
    let (first, _) = &frames[0];
    let w = first.width();
    let h = first.rows();
    let bpp = first.descriptor().bytes_per_pixel();
    let frame_count = frames.len() as u32;

    let mut data = Vec::new();
    data.extend_from_slice(b"MOCK");
    data.extend_from_slice(&w.to_le_bytes());
    data.extend_from_slice(&h.to_le_bytes());
    data.extend_from_slice(&frame_count.to_le_bytes());
    data.push(bpp as u8);

    for (ps, duration_ms) in frames {
        data.extend_from_slice(&duration_ms.to_le_bytes());
        for y in 0..ps.rows() {
            data.extend_from_slice(ps.row(y));
        }
    }
    data
}

fn parse_mock_header(data: &[u8]) -> Result<(u32, u32, u32, u8), MockError> {
    if data.len() < HEADER_SIZE || &data[..4] != b"MOCK" {
        return Err(MockError::InvalidData("bad mock header".into()));
    }
    let w = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let h = u32::from_le_bytes(data[8..12].try_into().unwrap());
    let fc = u32::from_le_bytes(data[12..16].try_into().unwrap());
    let bpp = data[16];
    if w == 0 || h == 0 || fc == 0 || (bpp != 3 && bpp != 4) {
        return Err(MockError::InvalidData("bad dimensions or bpp".into()));
    }
    Ok((w, h, fc, bpp))
}

fn descriptor_for_bpp(bpp: u8) -> PixelDescriptor {
    match bpp {
        3 => PixelDescriptor::RGB8_SRGB,
        4 => PixelDescriptor::RGBA8_SRGB,
        _ => unreachable!(),
    }
}

// =========================================================================
// Decode: Config → Job → Decoder / StreamingDecoder / FullFrameDecoder
// =========================================================================

#[derive(Clone, Debug)]
pub struct MockDecoderConfig;

static MOCK_DECODE_CAPS: DecodeCapabilities = DecodeCapabilities::new()
    .with_cheap_probe(true)
    .with_animation(true)
    .with_row_level(true)
    .with_native_alpha(true)
    .with_cancel(true);

impl DecoderConfig for MockDecoderConfig {
    type Error = MockError;
    type Job<'a> = MockDecodeJob<'a>;

    fn formats() -> &'static [ImageFormat] {
        // Use Pnm as a placeholder since we can't create custom formats easily
        &[ImageFormat::Pnm]
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB]
    }

    fn capabilities() -> &'static DecodeCapabilities {
        &MOCK_DECODE_CAPS
    }

    fn job(&self) -> MockDecodeJob<'_> {
        MockDecodeJob {
            limits: ResourceLimits::none(),
            stop: None,
            policy: None,
            crop: None,
            orientation: None,
            start_frame: None,
            ext: MockDecodeExtensions::default(),
        }
    }
}

pub struct MockDecodeJob<'a> {
    limits: ResourceLimits,
    stop: Option<&'a dyn Stop>,
    policy: Option<zc::decode::DecodePolicy>,
    crop: Option<(u32, u32, u32, u32)>,
    orientation: Option<zc::OrientationHint>,
    start_frame: Option<u32>,
    pub ext: MockDecodeExtensions,
}

impl<'a> DecodeJob<'a> for MockDecodeJob<'a> {
    type Error = MockError;
    type Dec = MockDec<'a>;
    type StreamDec = MockStreamDec<'a>;
    type FullFrameDec = MockFullFrameDec;

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn with_policy(mut self, policy: zc::decode::DecodePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    fn with_crop_hint(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.crop = Some((x, y, width, height));
        self
    }

    fn with_orientation(mut self, hint: zc::OrientationHint) -> Self {
        self.orientation = Some(hint);
        self
    }

    fn with_start_frame_index(mut self, index: u32) -> Self {
        self.start_frame = Some(index);
        self
    }

    fn extensions(&self) -> Option<&dyn std::any::Any> {
        Some(&self.ext)
    }

    fn extensions_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut self.ext)
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, MockError> {
        let (w, h, fc, _bpp) = parse_mock_header(data)?;
        Ok(ImageInfo::new(w, h, ImageFormat::Pnm)
            .with_frame_count(fc)
            .with_animation(fc > 1))
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, MockError> {
        let (w, h, _fc, bpp) = parse_mock_header(data)?;
        Ok(OutputInfo::full_decode(w, h, descriptor_for_bpp(bpp)))
    }

    fn decoder(
        self,
        data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<MockDec<'a>, MockError> {
        let (w, h, _, _) = parse_mock_header(&data)?;
        self.limits.check_dimensions(w, h)?;
        if let Some(stop) = self.stop {
            stop.check()?;
        }
        Ok(MockDec { data })
    }

    fn push_decoder(
        self,
        data: Cow<'a, [u8]>,
        sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, MockError> {
        zc::helpers::copy_decode_to_sink(self, data, sink, preferred, MockError::Sink)
    }

    fn streaming_decoder(
        self,
        data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<MockStreamDec<'a>, MockError> {
        let (w, h, _, bpp) = parse_mock_header(&data)?;
        self.limits.check_dimensions(w, h)?;
        Ok(MockStreamDec {
            data,
            current_row: 0,
            width: w,
            height: h,
            bpp,
        })
    }

    fn full_frame_decoder(
        self,
        data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<MockFullFrameDec, MockError> {
        let (w, h, fc, bpp) = parse_mock_header(&data)?;
        self.limits.check_dimensions(w, h)?;
        let owned = data.into_owned();
        Ok(MockFullFrameDec {
            data: owned,
            width: w,
            height: h,
            frame_count: fc,
            bpp,
            current_frame: self.start_frame.unwrap_or(0),
        })
    }
}

// --- One-shot decoder ---

#[derive(Debug)]
pub struct MockDec<'a> {
    data: Cow<'a, [u8]>,
}

impl<'a> zc::decode::Decode for MockDec<'a> {
    type Error = MockError;

    fn decode(self) -> Result<DecodeOutput, MockError> {
        let (w, h, _fc, bpp) = parse_mock_header(&self.data)?;
        let desc = descriptor_for_bpp(bpp);
        let frame_size = w as usize * h as usize * bpp as usize;
        let data_start = HEADER_SIZE + 4; // skip first frame's duration_ms
        if self.data.len() < data_start + frame_size {
            return Err(MockError::InvalidData("truncated frame data".into()));
        }
        let pixels = &self.data[data_start..data_start + frame_size];
        let buf = PixelBuffer::from_vec(pixels.to_vec(), w, h, desc)
            .map_err(|e| MockError::InvalidData(format!("buffer: {e}")))?;
        let info = ImageInfo::new(w, h, ImageFormat::Pnm);
        Ok(DecodeOutput::new(buf, info))
    }
}

// --- Streaming decoder (yields 1 row at a time) ---

pub struct MockStreamDec<'a> {
    data: Cow<'a, [u8]>,
    current_row: u32,
    width: u32,
    height: u32,
    bpp: u8,
}

impl<'a> StreamingDecode for MockStreamDec<'a> {
    type Error = MockError;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, MockError> {
        if self.current_row >= self.height {
            return Ok(None);
        }
        let row_bytes = self.width as usize * self.bpp as usize;
        let data_start = HEADER_SIZE + 4; // skip first frame duration
        let offset = data_start + self.current_row as usize * row_bytes;
        if offset + row_bytes > self.data.len() {
            return Err(MockError::InvalidData("truncated streaming data".into()));
        }
        let desc = descriptor_for_bpp(self.bpp);
        let row_data = &self.data[offset..offset + row_bytes];
        let ps = PixelSlice::new(row_data, self.width, 1, row_bytes, desc)
            .map_err(|e| MockError::InvalidData(format!("slice: {e}")))?;
        let y = self.current_row;
        self.current_row += 1;
        Ok(Some((y, ps)))
    }

    fn info(&self) -> &ImageInfo {
        // leak for simplicity in test code
        let info = ImageInfo::new(self.width, self.height, ImageFormat::Pnm);
        Box::leak(Box::new(info))
    }
}

// --- Full-frame animation decoder ---

pub struct MockFullFrameDec {
    data: Vec<u8>,
    width: u32,
    height: u32,
    frame_count: u32,
    bpp: u8,
    current_frame: u32,
}

impl FullFrameDecoder for MockFullFrameDec {
    type Error = MockError;

    fn wrap_sink_error(err: SinkError) -> MockError {
        MockError::Sink(err)
    }

    fn info(&self) -> &ImageInfo {
        Box::leak(Box::new(
            ImageInfo::new(self.width, self.height, ImageFormat::Pnm)
                .with_frame_count(self.frame_count)
                .with_animation(true),
        ))
    }

    fn frame_count(&self) -> Option<u32> {
        Some(self.frame_count)
    }

    fn loop_count(&self) -> Option<u32> {
        Some(0) // infinite loop
    }

    fn render_next_frame(
        &mut self,
        stop: Option<&dyn Stop>,
    ) -> Result<Option<FullFrame<'_>>, MockError> {
        if let Some(s) = stop {
            s.check()?;
        }
        if self.current_frame >= self.frame_count {
            return Ok(None);
        }

        let frame_pixels = self.width as usize * self.height as usize * self.bpp as usize;
        let frame_data_size = 4 + frame_pixels; // duration + pixels
        let frame_offset = HEADER_SIZE + self.current_frame as usize * frame_data_size;

        if frame_offset + frame_data_size > self.data.len() {
            return Err(MockError::InvalidData("truncated animation data".into()));
        }

        let duration_ms = u32::from_le_bytes(
            self.data[frame_offset..frame_offset + 4]
                .try_into()
                .unwrap(),
        );
        let pixel_start = frame_offset + 4;
        let pixel_end = pixel_start + frame_pixels;
        let desc = descriptor_for_bpp(self.bpp);
        let row_bytes = self.width as usize * self.bpp as usize;

        let ps = PixelSlice::new(
            &self.data[pixel_start..pixel_end],
            self.width,
            self.height,
            row_bytes,
            desc,
        )
        .map_err(|e| MockError::InvalidData(format!("frame slice: {e}")))?;

        let frame = FullFrame::new(ps, duration_ms, self.current_frame);
        self.current_frame += 1;
        Ok(Some(frame))
    }

    fn render_next_frame_to_sink(
        &mut self,
        stop: Option<&dyn Stop>,
        sink: &mut dyn DecodeRowSink,
    ) -> Result<Option<OutputInfo>, MockError> {
        zc::helpers::copy_frame_to_sink(self, stop, sink)
    }
}

// =========================================================================
// Extension types (for testing extensions() / extensions_mut())
// =========================================================================

/// Mock encode extensions — codec-specific knobs accessible through dyn dispatch.
#[derive(Debug, Default)]
pub struct MockEncodeExtensions {
    pub optimize: bool,
    pub custom_tag: Option<String>,
}

/// Mock decode extensions.
#[derive(Debug, Default)]
pub struct MockDecodeExtensions {
    pub strict_parsing: bool,
}

// =========================================================================
// Encode: Config → Job → Encoder / FullFrameEncoder
// =========================================================================

#[derive(Clone, Debug)]
pub struct MockEncoderConfig {
    quality: Option<f32>,
    effort: Option<i32>,
    lossless: Option<bool>,
    alpha_quality: Option<f32>,
}

impl MockEncoderConfig {
    pub fn new() -> Self {
        Self {
            quality: None,
            effort: None,
            lossless: None,
            alpha_quality: None,
        }
    }
}

static MOCK_ENCODE_CAPS: EncodeCapabilities = EncodeCapabilities::new()
    .with_lossless(true)
    .with_lossy(true)
    .with_native_alpha(true)
    .with_animation(true)
    .with_row_level(true)
    .with_pull(true)
    .with_cancel(true)
    .with_icc(true)
    .with_exif(true)
    .with_effort_range(0, 10)
    .with_quality_range(0.0, 100.0);

impl EncoderConfig for MockEncoderConfig {
    type Error = MockError;
    type Job<'a> = MockEncodeJob<'a>;

    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &[PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGBA8_SRGB]
    }

    fn capabilities() -> &'static EncodeCapabilities {
        &MOCK_ENCODE_CAPS
    }

    fn with_generic_quality(mut self, quality: f32) -> Self {
        self.quality = Some(quality);
        self
    }

    fn with_generic_effort(mut self, effort: i32) -> Self {
        self.effort = Some(effort);
        self
    }

    fn with_lossless(mut self, lossless: bool) -> Self {
        self.lossless = Some(lossless);
        self
    }

    fn with_alpha_quality(mut self, quality: f32) -> Self {
        self.alpha_quality = Some(quality);
        self
    }

    fn generic_quality(&self) -> Option<f32> {
        self.quality
    }

    fn generic_effort(&self) -> Option<i32> {
        self.effort
    }

    fn is_lossless(&self) -> Option<bool> {
        self.lossless
    }

    fn alpha_quality(&self) -> Option<f32> {
        self.alpha_quality
    }

    fn job(&self) -> MockEncodeJob<'_> {
        MockEncodeJob {
            limits: ResourceLimits::none(),
            stop: None,
            metadata: None,
            canvas_size: None,
            loop_count: None,
            policy: None,
            ext: MockEncodeExtensions::default(),
        }
    }
}

pub struct MockEncodeJob<'a> {
    limits: ResourceLimits,
    stop: Option<&'a dyn Stop>,
    metadata: Option<Metadata>,
    canvas_size: Option<(u32, u32)>,
    loop_count: Option<Option<u32>>,
    policy: Option<zc::encode::EncodePolicy>,
    pub ext: MockEncodeExtensions,
}

impl<'a> EncodeJob<'a> for MockEncodeJob<'a> {
    type Error = MockError;
    type Enc = MockEnc;
    type FullFrameEnc = MockFullFrameEnc;

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn with_policy(mut self, policy: zc::encode::EncodePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    fn with_metadata(mut self, meta: &Metadata) -> Self {
        self.metadata = Some(meta.clone());
        self
    }

    fn with_canvas_size(mut self, width: u32, height: u32) -> Self {
        self.canvas_size = Some((width, height));
        self
    }

    fn with_loop_count(mut self, count: Option<u32>) -> Self {
        self.loop_count = Some(count);
        self
    }

    fn extensions(&self) -> Option<&dyn std::any::Any> {
        Some(&self.ext)
    }

    fn extensions_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut self.ext)
    }

    fn encoder(self) -> Result<MockEnc, MockError> {
        Ok(MockEnc {
            accumulated: Vec::new(),
            width: None,
            height: None,
            desc: None,
        })
    }

    fn full_frame_encoder(self) -> Result<MockFullFrameEnc, MockError> {
        Ok(MockFullFrameEnc {
            frames: Vec::new(),
            loop_count: self.loop_count.flatten(),
        })
    }
}

// --- Single-image encoder (supports all three paths) ---

pub struct MockEnc {
    accumulated: Vec<u8>,
    width: Option<u32>,
    height: Option<u32>,
    desc: Option<PixelDescriptor>,
}

impl Encoder for MockEnc {
    type Error = MockError;

    fn reject(op: UnsupportedOperation) -> MockError {
        MockError::Unsupported(op)
    }

    fn preferred_strip_height(&self) -> u32 {
        4
    }

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, MockError> {
        let frame = (pixels, 0u32);
        let data = encode_mock_data(&[frame]);
        Ok(EncodeOutput::new(data, ImageFormat::Pnm))
    }

    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), MockError> {
        if self.width.is_none() {
            self.width = Some(rows.width());
            self.height = Some(0);
            self.desc = Some(rows.descriptor());
        }
        let h = self.height.as_mut().unwrap();
        for y in 0..rows.rows() {
            self.accumulated.extend_from_slice(rows.row(y));
            *h += 1;
        }
        Ok(())
    }

    fn finish(self) -> Result<EncodeOutput, MockError> {
        let w = self
            .width
            .ok_or(MockError::InvalidData("no rows pushed".into()))?;
        let h = self.height.unwrap();
        let desc = self.desc.unwrap();
        let buf = PixelBuffer::from_vec(self.accumulated, w, h, desc)
            .map_err(|e| MockError::InvalidData(format!("buffer: {e}")))?;
        let ps = buf.as_slice();
        let frame = (ps, 0u32);
        let data = encode_mock_data(&[frame]);
        Ok(EncodeOutput::new(data, ImageFormat::Pnm))
    }

    fn encode_from(
        self,
        source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, MockError> {
        // Pull rows one at a time using a fixed 4-row strip
        let strip_height = 4u32;
        let mut all_data = Vec::new();
        let mut y = 0u32;
        let mut total_h = 0u32;
        let w = 4u32; // fixed for simplicity
        let desc = PixelDescriptor::RGB8_SRGB;
        let bpp = desc.bytes_per_pixel();
        let row_bytes = w as usize * bpp;

        loop {
            let mut strip_buf = vec![0u8; strip_height as usize * row_bytes];
            let rows_written = {
                let ps = PixelSliceMut::new(&mut strip_buf, w, strip_height, row_bytes, desc)
                    .map_err(|e| MockError::InvalidData(format!("pull buf: {e}")))?;
                source(y, ps)
            };
            if rows_written == 0 {
                break;
            }
            all_data.extend_from_slice(&strip_buf[..rows_written * row_bytes]);
            total_h += rows_written as u32;
            y += rows_written as u32;
        }

        if total_h == 0 {
            return Err(MockError::InvalidData("no rows from source".into()));
        }
        let buf = PixelBuffer::from_vec(all_data, w, total_h, desc)
            .map_err(|e| MockError::InvalidData(format!("buffer: {e}")))?;
        let ps = buf.as_slice();
        let data = encode_mock_data(&[(ps, 0)]);
        Ok(EncodeOutput::new(data, ImageFormat::Pnm))
    }
}

// --- Full-frame animation encoder ---

pub struct MockFullFrameEnc {
    frames: Vec<(Vec<u8>, u32, u32, u32, PixelDescriptor)>, // data, w, h, duration, desc
    #[allow(dead_code)]
    loop_count: Option<u32>,
}

impl FullFrameEncoder for MockFullFrameEnc {
    type Error = MockError;

    fn reject(op: UnsupportedOperation) -> MockError {
        MockError::Unsupported(op)
    }

    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        stop: Option<&dyn Stop>,
    ) -> Result<(), MockError> {
        if let Some(s) = stop {
            s.check()?;
        }
        let w = pixels.width();
        let h = pixels.rows();
        let desc = pixels.descriptor();
        let mut data = Vec::with_capacity(w as usize * h as usize * desc.bytes_per_pixel());
        for y in 0..h {
            data.extend_from_slice(pixels.row(y));
        }
        self.frames.push((data, w, h, duration_ms, desc));
        Ok(())
    }

    fn finish(self, stop: Option<&dyn Stop>) -> Result<EncodeOutput, MockError> {
        if let Some(s) = stop {
            s.check()?;
        }
        if self.frames.is_empty() {
            return Err(MockError::InvalidData("no frames".into()));
        }

        // Build slices for encode_mock_data
        let bufs: Vec<PixelBuffer> = self
            .frames
            .iter()
            .map(|(data, w, h, _, desc)| {
                PixelBuffer::from_vec(data.clone(), *w, *h, *desc).unwrap()
            })
            .collect();
        let frame_refs: Vec<(PixelSlice<'_>, u32)> = bufs
            .iter()
            .zip(self.frames.iter())
            .map(|(buf, (_, _, _, dur, _))| (buf.as_slice(), *dur))
            .collect();

        let data = encode_mock_data(&frame_refs);
        Ok(EncodeOutput::new(data, ImageFormat::Pnm))
    }
}
