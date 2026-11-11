//! `ZRW1`: raw rows, streamed.
//!
//! ```text
//! "ZRW1"  width:u32le  height:u32le  layout:u8  flags:u8  frames:u32le  loops:u32le
//! then per frame:  duration_ms:u32le  rows
//! ```
//!
//! `layout` 0..=2 is packed gray, RGB, RGBA. Layout 3 is planar 4:2:0, stored
//! two luma rows at a time: the luma rows, then one Cb row, then one Cr row.
//! `flags` bit 0 marks a sequence.
//!
//! The decoder never copies a sample: rows are shown to the sink straight
//! from the input window. The encoder streams, and when it does not know the
//! frame count in advance it patches the header at the end.

use crate::{FixtureError, PACKED, le32, packed_from_byte, packed_of, packed_to_byte};
use std::sync::Arc;
use zencodec::decode::{
    DecodeCodec, DecodeEvent, DecodePlan, DecodeSpec, DecodeStep, Decoder, Delivery, Frames, Sink,
};
use zencodec::encode::{
    EncodeCodec, EncodeEvent, EncodeInput, EncodePlan, EncodeSpec, EncodeStep, Encoder,
    OutputBuffer, OutputTiming, RowDemand,
};
use zencodec::{
    Bound, CodecError, Demand, Error, ErrorKind, Fidelity, FormatInfo, FrameDuration, FrameHeader,
    ImageFormat, ImageInfo, Input, InputRequest, Layout, MetadataKinds, OutputAccess, Pass, Raster,
    ResourceEstimate, Result, Sequence, Unsupported, check, error, error_msg,
};
use zenpixels::plane::{AnyPlane, PlanarLayout, PlanarSlice, PlaneEncoding, PlaneRef, PlaneRole, PlaneSpec};
use zenpixels::sample::{ChromaSiting, ChromaSubsampling, SampleEncoding};
use zenpixels::{ChannelType, Cicp, ColorContext, PixelDescriptor, PixelSlice};

const MAGIC: &[u8; 4] = b"ZRW1";
const HEADER: usize = 22;
const FRAMES_AT: u64 = 14;
const PLANAR: u8 = 3;
const NAME: &str = "zrw";

static INFO: FormatInfo = FormatInfo::new("zrw", "image/x-zrw", "zrw", 4, |b| b.starts_with(MAGIC));

/// The `ZRW1` format.
pub const FORMAT: ImageFormat = ImageFormat::new(&INFO);

/// The planar layout `ZRW1` stores: 8-bit Y, Cb, Cr at 4:2:0.
pub fn yuv420() -> Arc<PlanarLayout> {
    let u8_codes = PlaneEncoding::Code(
        SampleEncoding::new(ChannelType::U8, 8, 0).expect("8 bits fit a byte"),
    );
    Arc::new(
        PlanarLayout::new(
            vec![
                PlaneSpec::new(PlaneRole::Luma, u8_codes),
                PlaneSpec::new(PlaneRole::Cb, u8_codes),
                PlaneSpec::new(PlaneRole::Cr, u8_codes),
            ],
            ChromaSubsampling::Yuv420,
            ChromaSiting::Center,
        )
        .expect("a valid layout"),
    )
}

/// The color context `ZRW1` planar samples carry: BT.601 matrix, full range.
pub fn bt601_full() -> Arc<ColorContext> {
    Arc::new(ColorContext::from_cicp(Cicp::new(1, 13, 6, true)))
}

/// A planar raster as `ZRW1` stores it.
pub fn planar_raster(width: u32, height: u32) -> Raster {
    Raster::planar(width, height, yuv420(), None, bt601_full())
}

/// Bytes of the planar strip that starts at luma row `y`, and its luma rows.
fn strip_shape(width: u32, height: u32, y: u32) -> (usize, u32) {
    let luma_rows = (height - y).min(2);
    let chroma = width.div_ceil(2) as usize;
    (luma_rows as usize * width as usize + 2 * chroma, luma_rows)
}

fn bad(e: FixtureError) -> Error {
    CodecError::from_native(e).at()
}

fn refuse(what: Unsupported) -> Error {
    CodecError::new(ErrorKind::Unsupported(what))
        .with_codec(NAME)
        .at()
}

/// The `ZRW1` codec, both directions.
#[derive(Clone, Copy, Debug, Default)]
pub struct RowsCodec;

// ───────────────────────────── decode ─────────────────────────────

impl DecodeCodec for RowsCodec {
    fn name(&self) -> &'static str {
        NAME
    }

    fn formats(&self) -> &[ImageFormat] {
        &[FORMAT]
    }

    fn decoder(&self, spec: &DecodeSpec) -> Result<Box<dyn Decoder>> {
        if spec.progressive {
            return Err(refuse(Unsupported::Progressive));
        }
        Ok(Box::new(RowsDecoder {
            spec: spec.clone(),
            request: [InputRequest::AtLeast {
                offset: 0,
                min_len: HEADER,
            }],
            pos: 0,
            info: None,
            plan: None,
            head: None,
            state: DecodeState::Header,
        }))
    }
}

#[derive(Clone)]
enum Samples {
    Packed(PixelDescriptor),
    Planar(Arc<PlanarLayout>, Arc<ColorContext>),
}

struct Head {
    width: u32,
    height: u32,
    samples: Samples,
    animated: bool,
    frames: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DecodeState {
    Header,
    FrameStart { frame: u32 },
    Rows { frame: u32, y: u32 },
    AfterFrames,
    Done,
}

struct RowsDecoder {
    spec: DecodeSpec,
    request: [InputRequest; 1],
    /// Absolute offset of the next unread byte.
    pos: u64,
    info: Option<ImageInfo>,
    plan: Option<DecodePlan>,
    head: Option<Head>,
    state: DecodeState,
}

impl RowsDecoder {
    /// Ask for `min_len` bytes at the read position, or fail if the input is over.
    fn need(&mut self, input: &Input<'_>, used: usize, min_len: usize) -> Result<DecodeStep> {
        if input.reaches_end() {
            return Err(CodecError::new(ErrorKind::Truncated).with_codec(NAME).at());
        }
        self.pos += used as u64;
        self.request = [InputRequest::AtLeast {
            offset: self.pos,
            min_len,
        }];
        Ok(DecodeStep::new(used, DecodeEvent::NeedInput))
    }

    fn emit(&mut self, used: usize, event: DecodeEvent) -> DecodeStep {
        self.pos += used as u64;
        self.request = [InputRequest::AtLeast {
            offset: self.pos,
            min_len: 1,
        }];
        DecodeStep::new(used, event)
    }

    fn header(&mut self, input: &Input<'_>) -> Result<DecodeStep> {
        let bytes = input.bytes;
        if bytes.len() >= 4 && !bytes.starts_with(MAGIC) {
            return Err(bad(FixtureError::BadMagic));
        }
        if bytes.len() < HEADER {
            return self.need(input, 0, HEADER);
        }
        let (width, height) = (le32(bytes, 4), le32(bytes, 8));
        let animated = bytes[13] & 1 != 0;
        let frames = le32(bytes, 14);
        if frames == 0 || (!animated && frames != 1) {
            return Err(bad(FixtureError::BadHeader("frame count")));
        }
        self.spec.limits.check_dimensions(width, height)?;
        self.spec.limits.check_frames(frames)?;

        let (samples, native, raster) = if bytes[12] == PLANAR {
            if !self.spec.planar {
                // A real codec would convert; this fixture only shows planes.
                return Err(refuse(Unsupported::Planar));
            }
            let (layout, color) = (yuv420(), bt601_full());
            let raster = Raster::planar(width, height, layout.clone(), None, color.clone());
            (Samples::Planar(layout, color), raster.layout.clone(), raster)
        } else {
            let native = packed_from_byte(bytes[12]).map_err(bad)?;
            let output = self.spec.accept.choose(native, &[]).map_err(refuse)?;
            (
                Samples::Packed(output),
                Layout::Packed(native),
                Raster::packed(width, height, output),
            )
        };
        let sequence = if animated {
            Sequence::Animation {
                frames: Some(frames),
                loops: Some(le32(bytes, 18)),
            }
        } else {
            Sequence::Still
        };
        self.info = Some(
            ImageInfo::new(FORMAT, width, height, native)
                .with_sequence(sequence)
                .with_lossless(true)
                .with_metadata(Default::default(), MetadataKinds::ALL),
        );
        // Rows are shown in place: the decoder owns no working memory.
        self.plan = Some(DecodePlan::new(
            raster,
            Delivery::Strips,
            ResourceEstimate::serial(Bound::AtMost(0)),
        ));
        self.head = Some(Head {
            width,
            height,
            samples,
            animated,
            frames,
        });
        self.state = DecodeState::FrameStart { frame: 0 };
        Ok(self.emit(HEADER, DecodeEvent::Info))
    }
}

impl Decoder for RowsDecoder {
    fn requested(&self) -> &[InputRequest] {
        &self.request
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.info.as_ref()
    }

    fn plan(&self) -> Option<&DecodePlan> {
        self.plan.as_ref()
    }

    fn step(&mut self, input: Input<'_>, sink: &mut dyn Sink) -> Result<DecodeStep> {
        if input.offset != self.pos {
            return Err(error_msg(ErrorKind::Protocol, "input is not at the read position"));
        }
        let bytes = input.bytes;
        match self.state {
            DecodeState::Header => self.header(&input),
            DecodeState::FrameStart { frame } => {
                if bytes.len() < 4 {
                    return self.need(&input, 0, 4);
                }
                let head = self.head.as_ref().expect("set with the header");
                let raster = self.plan.as_ref().expect("set with the header").raster.clone();
                let duration = head
                    .animated
                    .then(|| FrameDuration::from_millis(le32(bytes, 0)));
                check(&self.spec.pulse)?;
                sink.begin(&FrameHeader::in_sequence(raster, frame, duration))?;
                self.state = DecodeState::Rows { frame, y: 0 };
                Ok(self.emit(4, DecodeEvent::Rows))
            }
            DecodeState::Rows { frame, y } => {
                let head = self.head.as_ref().expect("set with the header");
                let (width, height) = (head.width, head.height);
                let (used, rows) = match &head.samples {
                    Samples::Packed(descriptor) => {
                        let row_bytes = width as usize * descriptor.bytes_per_pixel();
                        let rows = if row_bytes == 0 {
                            height - y
                        } else {
                            ((bytes.len() / row_bytes) as u64).min(u64::from(height - y)) as u32
                        };
                        if rows == 0 && y < height {
                            return self.need(&input, 0, row_bytes);
                        }
                        let used = rows as usize * row_bytes;
                        if rows > 0 {
                            let strip =
                                PixelSlice::new(&bytes[..used], width, rows, row_bytes, *descriptor)
                                    .map_err(|_| error(ErrorKind::Internal))?;
                            sink.rows(y, strip, Pass::Final)?;
                        }
                        (used, rows)
                    }
                    Samples::Planar(layout, color) => {
                        let (strip_bytes, luma_rows) = strip_shape(width, height, y);
                        if bytes.len() < strip_bytes {
                            return self.need(&input, 0, strip_bytes);
                        }
                        let (w, chroma) = (width as usize, width.div_ceil(2) as usize);
                        let luma_len = luma_rows as usize * w;
                        let plane = |from: usize, len: usize, width: usize, rows: usize| {
                            PlaneRef::new(&bytes[from..from + len], width, rows, width)
                                .map(AnyPlane::U8)
                                .map_err(|_| error(ErrorKind::Internal))
                        };
                        let planes = [
                            plane(0, luma_len, w, luma_rows as usize)?,
                            plane(luma_len, chroma, chroma, 1)?,
                            plane(luma_len + chroma, chroma, chroma, 1)?,
                        ];
                        let strip = PlanarSlice::new(&planes, layout, width, luma_rows, color, None)
                            .map_err(|_| error(ErrorKind::Internal))?;
                        sink.planes(y, strip)?;
                        (strip_bytes, luma_rows)
                    }
                };
                check(&self.spec.pulse)?;
                let y = y + rows;
                if y < height {
                    self.state = DecodeState::Rows { frame, y };
                    return Ok(self.emit(used, DecodeEvent::Rows));
                }
                sink.end()?;
                let more = self.spec.frames == Frames::All && frame + 1 < head.frames;
                self.state = if more {
                    DecodeState::FrameStart { frame: frame + 1 }
                } else {
                    DecodeState::AfterFrames
                };
                Ok(self.emit(used, DecodeEvent::FrameDone))
            }
            DecodeState::AfterFrames => {
                self.state = DecodeState::Done;
                Ok(DecodeStep::new(0, DecodeEvent::Done))
            }
            DecodeState::Done => Err(error_msg(ErrorKind::Protocol, "step after the decode ended")),
        }
    }
}

// ───────────────────────────── encode ─────────────────────────────

impl EncodeCodec for RowsCodec {
    fn name(&self) -> &'static str {
        NAME
    }

    fn format(&self) -> ImageFormat {
        FORMAT
    }

    fn encoder(&self, spec: &EncodeSpec) -> Result<Box<dyn Encoder>> {
        let (width, height) = (spec.raster.width, spec.raster.height);
        let (layout_byte, frame_bytes, strip_rows) = match &spec.raster.layout {
            Layout::Planar { planes, alpha: None } if **planes == *yuv420() => {
                let chroma = u64::from(width.div_ceil(2)) * u64::from(height.div_ceil(2));
                (PLANAR, u64::from(width) * u64::from(height) + 2 * chroma, 2)
            }
            Layout::Planar { .. } => return Err(refuse(Unsupported::Planar)),
            layout => {
                let descriptor = packed_of(layout).ok_or_else(|| refuse(Unsupported::PixelFormat))?;
                let bpp = descriptor.bytes_per_pixel() as u64;
                let byte = packed_to_byte(descriptor).expect("checked by packed_of");
                (byte, u64::from(width) * u64::from(height) * bpp, 1)
            }
        };
        let fidelity = spec
            .fidelity
            .resolve(Fidelity::Lossless, |want| {
                want.is_lossless().then_some(Fidelity::Lossless)
            })
            .map_err(|_| refuse(Unsupported::Fidelity))?;
        if matches!(spec.effort, Demand::Require(_)) {
            return Err(refuse(Unsupported::Effort));
        }
        if !spec.require_metadata.is_empty() {
            return Err(refuse(Unsupported::Metadata));
        }
        spec.limits.check_dimensions(width, height)?;

        let known_frames = match spec.sequence {
            None => Some(1),
            Some(sequence) => sequence.frames,
        };
        // The size is exact, so the output gate is settled before any row moves.
        let total = known_frames.map(|n| HEADER as u64 + u64::from(n) * (4 + frame_bytes));
        if let Some(total) = total {
            spec.limits.check_output_bytes(total)?;
        }
        if let Some(n) = known_frames {
            spec.limits.check_frames(n)?;
        }
        // An unknown frame count is patched into the header at the end. A
        // destination that cannot be patched gets nothing until then.
        let hold = known_frames.is_none() && spec.output == OutputAccess::Forward;
        let memory = if hold {
            Bound::Unknown
        } else {
            Bound::AtMost(0)
        };
        spec.limits.check_memory(memory)?;

        let mut output = OutputBuffer::new(spec.output, spec.limits.max_output_bytes);
        if hold {
            output.hold();
        }
        let plan = EncodePlan::new(
            fidelity,
            ResourceEstimate::serial(memory)
                .with_output_bytes(total.map_or(Bound::Unknown, Bound::AtMost)),
        )
        .with_input(RowDemand::Strips)
        .with_strip_rows(strip_rows)
        .with_output(if hold {
            OutputTiming::AtFinish
        } else {
            OutputTiming::Streaming { lag_rows: 0 }
        });

        Ok(Box::new(RowsEncoder {
            spec: spec.clone(),
            plan,
            output,
            layout_byte,
            known_frames,
            written: 0,
            state: EncodeState::Start,
        }))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EncodeState {
    Start,
    BetweenFrames,
    Rows { y: u32 },
    Done,
}

struct RowsEncoder {
    spec: EncodeSpec,
    plan: EncodePlan,
    output: OutputBuffer,
    layout_byte: u8,
    known_frames: Option<u32>,
    written: u32,
    state: EncodeState,
}

impl RowsEncoder {
    fn animated(&self) -> bool {
        self.spec.sequence.is_some()
    }

    fn start(&mut self) -> Result<()> {
        let mut header = [0u8; HEADER];
        header[..4].copy_from_slice(MAGIC);
        header[4..8].copy_from_slice(&self.spec.raster.width.to_le_bytes());
        header[8..12].copy_from_slice(&self.spec.raster.height.to_le_bytes());
        header[12] = self.layout_byte;
        header[13] = u8::from(self.animated());
        header[14..18].copy_from_slice(&self.known_frames.unwrap_or(0).to_le_bytes());
        let loops = self.spec.sequence.and_then(|s| s.loops).unwrap_or(0);
        header[18..22].copy_from_slice(&loops.to_le_bytes());
        self.output.write(&header)?;
        if self.animated() {
            self.state = EncodeState::BetweenFrames;
        } else {
            self.output.write(&0u32.to_le_bytes())?;
            self.state = EncodeState::Rows { y: 0 };
        }
        Ok(())
    }

    fn need(&self) -> EncodeEvent {
        match self.state {
            EncodeState::Start => unreachable!("started before reporting"),
            EncodeState::BetweenFrames => EncodeEvent::NeedFrame,
            EncodeState::Rows { y } => EncodeEvent::NeedRows { pass: 0, y },
            EncodeState::Done => EncodeEvent::Done,
        }
    }

    /// Advance past `rows` rows, closing the frame at its last row.
    fn advance(&mut self, y: u32, rows: u32) -> Result<EncodeStep> {
        check(&self.spec.pulse)?;
        let y = y + rows;
        if y < self.spec.raster.height {
            self.state = EncodeState::Rows { y };
        } else {
            self.written += 1;
            if self.animated() {
                self.state = EncodeState::BetweenFrames;
            } else {
                self.output.release();
                self.state = EncodeState::Done;
            }
        }
        Ok(EncodeStep::new(rows, self.need()))
    }

    fn packed_rows(&mut self, y: u32, rows: PixelSlice<'_>) -> Result<EncodeStep> {
        let raster = &self.spec.raster;
        if Some(rows.descriptor()) != raster.descriptor()
            || rows.width() != raster.width
            || rows.rows() > raster.height - y
        {
            return Err(error_msg(ErrorKind::Protocol, "rows do not match the spec"));
        }
        let row_bytes = raster.row_bytes().ok_or_else(|| error(ErrorKind::Internal))?;
        for row in 0..rows.rows() {
            self.output.write(&rows.row(row)[..row_bytes])?;
        }
        self.advance(y, rows.rows())
    }

    fn planar_rows(&mut self, y: u32, strip: PlanarSlice<'_>) -> Result<EncodeStep> {
        let raster = &self.spec.raster;
        let Layout::Planar { planes: layout, .. } = &raster.layout else {
            return Err(error_msg(ErrorKind::Protocol, "this encode takes packed rows"));
        };
        let given = strip.height();
        if *strip.layout() != **layout
            || strip.width() != raster.width
            || given > raster.height - y
        {
            return Err(error_msg(ErrorKind::Protocol, "planes do not match the spec"));
        }
        // A strip is consumed whole when it is a multiple of two luma rows or
        // reaches the last row. Otherwise the odd row waits for its pair.
        let last = y + given == raster.height;
        let take = if last { given } else { given & !1 };
        let [AnyPlane::U8(luma), AnyPlane::U8(cb), AnyPlane::U8(cr)] = strip.planes() else {
            return Err(error(ErrorKind::Internal));
        };
        let mut row = 0;
        while row < take {
            let luma_rows = (take - row).min(2);
            for r in row..row + luma_rows {
                self.output
                    .write(luma.row(r as usize).ok_or_else(|| error(ErrorKind::Internal))?)?;
            }
            let chroma_row = (row / 2) as usize;
            self.output
                .write(cb.row(chroma_row).ok_or_else(|| error(ErrorKind::Internal))?)?;
            self.output
                .write(cr.row(chroma_row).ok_or_else(|| error(ErrorKind::Internal))?)?;
            row += luma_rows;
        }
        self.advance(y, take)
    }
}

impl Encoder for RowsEncoder {
    fn plan(&self) -> &EncodePlan {
        &self.plan
    }

    fn step(&mut self, input: EncodeInput<'_>) -> Result<EncodeStep> {
        if self.state == EncodeState::Start {
            self.start()?;
        }
        match (self.state, input) {
            (_, EncodeInput::None) => Ok(EncodeStep::new(0, self.need())),
            (EncodeState::BetweenFrames, EncodeInput::Frame(meta)) => {
                self.spec.limits.check_frames(self.written + 1)?;
                if self.known_frames.is_some_and(|n| self.written >= n) {
                    return Err(error_msg(ErrorKind::Protocol, "more frames than declared"));
                }
                let millis = meta.duration.and_then(|d| d.as_millis()).unwrap_or(0);
                self.output.write(&millis.to_le_bytes())?;
                self.state = EncodeState::Rows { y: 0 };
                Ok(EncodeStep::new(0, self.need()))
            }
            (EncodeState::BetweenFrames, EncodeInput::End) => {
                match self.known_frames {
                    Some(n) if n != self.written => {
                        return Err(error_msg(ErrorKind::Protocol, "fewer frames than declared"));
                    }
                    Some(_) => {}
                    None if self.written == 0 => {
                        return Err(error_msg(ErrorKind::Protocol, "a sequence needs a frame"));
                    }
                    None => self.output.patch(FRAMES_AT, &self.written.to_le_bytes())?,
                }
                self.output.release();
                self.state = EncodeState::Done;
                Ok(EncodeStep::new(0, EncodeEvent::Done))
            }
            (EncodeState::Rows { y }, EncodeInput::Planes(strip)) => self.planar_rows(y, strip),
            (EncodeState::Rows { y }, input) => match input.packed() {
                Some(rows) => self.packed_rows(y, rows),
                None => Err(error_msg(ErrorKind::Protocol, "rows were needed")),
            },
            _ => Err(error_msg(ErrorKind::Protocol, "input does not answer the last event")),
        }
    }

    fn output(&mut self) -> &mut OutputBuffer {
        &mut self.output
    }
}

/// The packed formats `ZRW1` stores.
pub const ACCEPTS: [PixelDescriptor; 3] = PACKED;
