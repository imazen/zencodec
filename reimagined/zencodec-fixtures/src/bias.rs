//! `ZTP1`: every sample stored relative to the image's smallest sample.
//!
//! ```text
//! "ZTP1"  width:u32le  height:u32le  layout:u8  bias:u8  exif_offset:u64le  exif_len:u32le
//! rows (each byte = sample - bias)    then the EXIF block, after the pixels
//! ```
//!
//! The bias is not known until every sample has been seen, so the encoder
//! needs two passes over the rows. How it gets them depends on what the
//! caller offered, and the plan says which:
//!
//! | supply | plan | rows held by the encoder |
//! |---|---|---|
//! | `Whole` | `RowDemand::Whole` | none: it scans and writes the caller's slice |
//! | `Replay(2+)` | `RowDemand::Replay(2)` | none: it asks for the rows twice |
//! | `Once` | `RowDemand::Strips` | the whole frame, and its estimate says so |
//!
//! The decoder writes into rows the sink lends. Its EXIF sits after the
//! pixels; a caller that needs it first gets it through a range read.

use crate::{FixtureError, le32, le64, packed_from_byte, packed_of, packed_to_byte};
use zencodec::decode::{
    Access, DecodeCodec, DecodeEvent, DecodePlan, DecodeSpec, DecodeStep, Decoder, Delivery, Sink,
};
use zencodec::encode::{
    EncodeCodec, EncodeEvent, EncodeInput, EncodePlan, EncodeSpec, EncodeStep, Encoder,
    OutputBuffer, OutputTiming, RowDemand, RowSupply,
};
use zencodec::{
    Bound, ByteRange, CodecError, Demand, Error, ErrorKind, Fidelity, FormatInfo, FrameHeader,
    ImageFormat, ImageInfo, Input, InputRequest, Layout, MetadataKinds, Pass, Raster,
    ResourceEstimate, Result, Unsupported, check, copy_rows, error, error_msg,
};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

const MAGIC: &[u8; 4] = b"ZTP1";
const HEADER: usize = 26;
const NAME: &str = "ztp";

static INFO: FormatInfo = FormatInfo::new("ztp", "image/x-ztp", "ztp", 4, |b| b.starts_with(MAGIC));

/// The `ZTP1` format.
pub const FORMAT: ImageFormat = ImageFormat::new(&INFO);

fn bad(e: FixtureError) -> Error {
    CodecError::from_native(e).at()
}

fn refuse(what: Unsupported) -> Error {
    CodecError::new(ErrorKind::Unsupported(what))
        .with_codec(NAME)
        .at()
}

fn truncated() -> Error {
    CodecError::new(ErrorKind::Truncated).with_codec(NAME).at()
}

/// The `ZTP1` codec, both directions.
#[derive(Clone, Copy, Debug, Default)]
pub struct BiasCodec;

// ───────────────────────────── decode ─────────────────────────────

impl DecodeCodec for BiasCodec {
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
        Ok(Box::new(BiasDecoder {
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

#[derive(Clone, Copy)]
struct Head {
    width: u32,
    height: u32,
    descriptor: PixelDescriptor,
    bias: u8,
    exif: ByteRange,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DecodeState {
    Header,
    /// Waiting for the EXIF range the caller wanted before pixels.
    ExifFirst,
    Rows {
        y: u32,
        begun: bool,
    },
    /// After the last row: the EXIF block, if it was not read first.
    Trailer,
    AfterFrame,
    Done,
}

struct BiasDecoder {
    spec: DecodeSpec,
    request: [InputRequest; 1],
    /// Absolute offset of the next unread sequential byte.
    pos: u64,
    info: Option<ImageInfo>,
    plan: Option<DecodePlan>,
    head: Option<Head>,
    state: DecodeState,
}

impl BiasDecoder {
    fn need(&mut self, input: &Input<'_>, used: usize, min_len: usize) -> Result<DecodeStep> {
        if input.reaches_end() {
            return Err(truncated());
        }
        self.pos += used as u64;
        self.request = [InputRequest::AtLeast {
            offset: self.pos,
            min_len,
        }];
        Ok(DecodeStep::new(used, DecodeEvent::NeedInput))
    }

    /// Move the read position and say so: `requested` is always current.
    fn advance(&mut self, used: usize) {
        self.pos += used as u64;
        self.request = [InputRequest::AtLeast {
            offset: self.pos,
            min_len: 1,
        }];
    }

    fn sequential(&self, input: &Input<'_>) -> Result<()> {
        if input.offset == self.pos {
            Ok(())
        } else {
            Err(error_msg(
                ErrorKind::Protocol,
                "input is not at the read position",
            ))
        }
    }

    fn set_exif(&mut self, exif: Option<&[u8]>) {
        let info = self.info.as_mut().expect("set with the header");
        info.metadata.exif = exif.map(Into::into);
        info.settled = MetadataKinds::ALL;
    }

    fn header(&mut self, input: &Input<'_>) -> Result<DecodeStep> {
        self.sequential(input)?;
        let bytes = input.bytes;
        if bytes.len() >= 4 && !bytes.starts_with(MAGIC) {
            return Err(bad(FixtureError::BadMagic));
        }
        if bytes.len() < HEADER {
            return self.need(input, 0, HEADER);
        }
        let (width, height) = (le32(bytes, 4), le32(bytes, 8));
        let native = packed_from_byte(bytes[12]).map_err(bad)?;
        let exif = ByteRange::new(le64(bytes, 14), u64::from(le32(bytes, 22)));
        self.spec.limits.check_dimensions(width, height)?;
        let descriptor = self.spec.accept.choose(native, &[]).map_err(refuse)?;
        let data_end = HEADER as u64
            + u64::from(width) * u64::from(height) * descriptor.bytes_per_pixel() as u64;
        if exif.length > 0 && exif.offset != data_end {
            return Err(bad(FixtureError::BadHeader(
                "EXIF does not follow the pixels",
            )));
        }

        let early = exif.length > 0 && self.spec.metadata_first.contains(MetadataKinds::EXIF);
        // Everything but EXIF is settled now: the format carries nothing else.
        let settled = if exif.length == 0 {
            MetadataKinds::ALL
        } else {
            MetadataKinds::ALL.difference(MetadataKinds::EXIF)
        };
        self.info = Some(
            ImageInfo::new(FORMAT, width, height, Layout::Packed(native))
                .with_lossless(true)
                .with_metadata(Default::default(), settled),
        );
        // Samples are written straight into lent rows.
        self.plan = Some(
            DecodePlan::new(
                Raster::packed(width, height, descriptor),
                Delivery::Strips,
                ResourceEstimate::serial(Bound::AtMost(0)),
            )
            .with_access(if early {
                Access::Random
            } else {
                Access::Sequential
            }),
        );
        self.head = Some(Head {
            width,
            height,
            descriptor,
            bias: bytes[13],
            exif,
        });
        self.advance(HEADER);
        if early {
            // Info waits until the EXIF is in hand: the caller asked for that.
            self.state = DecodeState::ExifFirst;
            self.request = [InputRequest::Range(exif)];
            return Ok(DecodeStep::new(HEADER, DecodeEvent::NeedInput));
        }
        self.state = DecodeState::Rows { y: 0, begun: false };
        Ok(DecodeStep::new(HEADER, DecodeEvent::Info))
    }
}

impl Decoder for BiasDecoder {
    fn requested(&self) -> &[InputRequest] {
        &self.request
    }

    fn info(&self) -> Option<&ImageInfo> {
        // Not before `Info`, even though the header has been parsed.
        match self.state {
            DecodeState::Header | DecodeState::ExifFirst => None,
            _ => self.info.as_ref(),
        }
    }

    fn plan(&self) -> Option<&DecodePlan> {
        match self.state {
            DecodeState::Header | DecodeState::ExifFirst => None,
            _ => self.plan.as_ref(),
        }
    }

    fn step(&mut self, input: Input<'_>, sink: &mut dyn Sink) -> Result<DecodeStep> {
        match self.state {
            DecodeState::Header => self.header(&input),
            DecodeState::ExifFirst => {
                let exif = self.head.expect("set with the header").exif;
                if !InputRequest::Range(exif).is_met_by(&input) {
                    return Ok(DecodeStep::new(0, DecodeEvent::NeedInput));
                }
                let start = (exif.offset - input.offset) as usize;
                let Some(block) = input.bytes.get(start..start + exif.length as usize) else {
                    return Err(truncated());
                };
                self.set_exif(Some(block));
                self.advance(0);
                self.state = DecodeState::Rows { y: 0, begun: false };
                // A range read is a look, not a read: nothing is consumed.
                Ok(DecodeStep::new(0, DecodeEvent::Info))
            }
            DecodeState::Rows { y, begun } => {
                self.sequential(&input)?;
                let head = self.head.expect("set with the header");
                if !begun {
                    check(&self.spec.pulse)?;
                    let raster = self
                        .plan
                        .as_ref()
                        .expect("set with the header")
                        .raster
                        .clone();
                    sink.begin(&FrameHeader::still(raster))?;
                    self.state = DecodeState::Rows { y, begun: true };
                }
                let row_bytes = head.width as usize * head.descriptor.bytes_per_pixel();
                let remaining = head.height - y;
                // A zero-width image has rows but no bytes.
                let available = input
                    .bytes
                    .len()
                    .checked_div(row_bytes)
                    .map_or(remaining, |n| (n as u64).min(remaining.into()) as u32);
                if available == 0 && remaining > 0 {
                    return self.need(&input, 0, row_bytes);
                }
                let mut rows = 0;
                if available > 0 {
                    // One loan per step: the sink may lend fewer rows than asked.
                    let mut lent = sink.lend(y, available, Pass::Final)?;
                    if lent.width() != head.width || lent.descriptor() != head.descriptor {
                        return Err(error_msg(
                            ErrorKind::Sink,
                            "sink lent rows of another shape",
                        ));
                    }
                    rows = lent.rows().min(available);
                    if rows == 0 {
                        return Err(error_msg(ErrorKind::Sink, "sink lent no rows"));
                    }
                    for r in 0..rows {
                        let from = &input.bytes[r as usize * row_bytes..][..row_bytes];
                        for (to, stored) in lent.row_mut(r)[..row_bytes].iter_mut().zip(from) {
                            *to = stored.wrapping_add(head.bias);
                        }
                    }
                }
                check(&self.spec.pulse)?;
                let used = rows as usize * row_bytes;
                self.advance(used);
                self.state = if y + rows == head.height {
                    DecodeState::Trailer
                } else {
                    DecodeState::Rows {
                        y: y + rows,
                        begun: true,
                    }
                };
                Ok(DecodeStep::new(used, DecodeEvent::Rows))
            }
            DecodeState::Trailer => {
                self.sequential(&input)?;
                let exif = self.head.expect("set with the header").exif;
                let pending = self
                    .info
                    .as_ref()
                    .is_some_and(|i| !i.settled.contains(MetadataKinds::EXIF));
                let mut used = 0;
                if pending {
                    let len = exif.length as usize;
                    if input.bytes.len() < len {
                        return self.need(&input, 0, len);
                    }
                    self.set_exif(Some(&input.bytes[..len]));
                    used = len;
                    self.advance(used);
                }
                sink.end()?;
                self.state = DecodeState::AfterFrame;
                Ok(DecodeStep::new(used, DecodeEvent::FrameDone))
            }
            DecodeState::AfterFrame => {
                self.state = DecodeState::Done;
                Ok(DecodeStep::new(0, DecodeEvent::Done))
            }
            DecodeState::Done => Err(error_msg(
                ErrorKind::Protocol,
                "step after the decode ended",
            )),
        }
    }
}

// ───────────────────────────── encode ─────────────────────────────

impl EncodeCodec for BiasCodec {
    fn name(&self) -> &'static str {
        NAME
    }

    fn format(&self) -> ImageFormat {
        FORMAT
    }

    fn encoder(&self, spec: &EncodeSpec) -> Result<Box<dyn Encoder>> {
        if spec.sequence.is_some() {
            return Err(refuse(Unsupported::Animation));
        }
        let descriptor = match &spec.raster.layout {
            Layout::Planar { .. } => return Err(refuse(Unsupported::Planar)),
            layout => packed_of(layout).ok_or_else(|| refuse(Unsupported::PixelFormat))?,
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
        // EXIF is the one block this format carries.
        let embedded = spec.metadata.kinds().intersection(MetadataKinds::EXIF);
        if !embedded.contains(spec.require_metadata) {
            return Err(refuse(Unsupported::Metadata));
        }
        let (width, height) = (spec.raster.width, spec.raster.height);
        spec.limits.check_dimensions(width, height)?;
        let frame_bytes =
            u64::from(width) * u64::from(height) * descriptor.bytes_per_pixel() as u64;
        let exif_len = spec.metadata.exif.as_ref().map_or(0, |e| e.len() as u64);
        if exif_len > u64::from(u32::MAX) {
            return Err(refuse(Unsupported::Metadata));
        }
        let total = HEADER as u64 + frame_bytes + exif_len;
        spec.limits.check_output_bytes(total)?;

        let (input, mode, held) = match spec.supply {
            RowSupply::Whole => (RowDemand::Whole, Mode::Whole, 0),
            RowSupply::Replay(passes) if passes >= 2 => (RowDemand::Replay(2), Mode::Replay, 0),
            // One look at each row is not enough, so the rows are kept.
            _ => (RowDemand::Strips, Mode::Buffer(None), frame_bytes),
        };
        spec.limits.check_memory(Bound::AtMost(held))?;
        let plan = EncodePlan::new(
            fidelity,
            ResourceEstimate::serial(Bound::AtMost(held)).with_output_bytes(Bound::AtMost(total)),
        )
        .with_input(input)
        .with_embedded(embedded)
        .with_output(if matches!(mode, Mode::Replay) {
            OutputTiming::Streaming { lag_rows: 0 }
        } else {
            OutputTiming::AtFinish
        });

        Ok(Box::new(BiasEncoder {
            spec: spec.clone(),
            plan,
            output: OutputBuffer::new(spec.output, spec.limits.max_output_bytes),
            descriptor,
            frame_bytes,
            mode,
            pass: 0,
            y: 0,
            min: u8::MAX,
            scratch: Vec::new(),
            done: false,
        }))
    }
}

enum Mode {
    Whole,
    Replay,
    Buffer(Option<PixelBuffer>),
}

struct BiasEncoder {
    spec: EncodeSpec,
    plan: EncodePlan,
    output: OutputBuffer,
    descriptor: PixelDescriptor,
    frame_bytes: u64,
    mode: Mode,
    pass: u32,
    y: u32,
    min: u8,
    scratch: Vec<u8>,
    done: bool,
}

impl BiasEncoder {
    fn row_bytes(&self) -> usize {
        self.spec.raster.width as usize * self.descriptor.bytes_per_pixel()
    }

    fn scan(&mut self, rows: &PixelSlice<'_>) {
        let row_bytes = self.row_bytes();
        for y in 0..rows.rows() {
            for &sample in &rows.row(y)[..row_bytes] {
                self.min = self.min.min(sample);
            }
        }
    }

    fn write_header(&mut self) -> Result<()> {
        // An image with no samples has no minimum.
        if self.frame_bytes == 0 {
            self.min = 0;
        }
        let exif_len = self.spec.metadata.exif.as_ref().map_or(0, |e| e.len());
        let mut header = [0u8; HEADER];
        header[..4].copy_from_slice(MAGIC);
        header[4..8].copy_from_slice(&self.spec.raster.width.to_le_bytes());
        header[8..12].copy_from_slice(&self.spec.raster.height.to_le_bytes());
        header[12] = packed_to_byte(self.descriptor).ok_or_else(|| error(ErrorKind::Internal))?;
        header[13] = self.min;
        header[14..22].copy_from_slice(&(HEADER as u64 + self.frame_bytes).to_le_bytes());
        header[22..26].copy_from_slice(&(exif_len as u32).to_le_bytes());
        self.output.write(&header)
    }

    fn write_rows(&mut self, rows: &PixelSlice<'_>) -> Result<()> {
        let row_bytes = self.row_bytes();
        for y in 0..rows.rows() {
            self.scratch.clear();
            self.scratch
                .extend(rows.row(y)[..row_bytes].iter().map(|s| s - self.min));
            self.output.write(&self.scratch)?;
        }
        check(&self.spec.pulse)
    }

    fn write_trailer(&mut self) -> Result<()> {
        if let Some(exif) = self.spec.metadata.exif.clone() {
            self.output.write(&exif)?;
        }
        self.done = true;
        Ok(())
    }

    /// Both passes over rows that are all in hand.
    fn encode_whole(&mut self, rows: &PixelSlice<'_>) -> Result<()> {
        self.scan(rows);
        check(&self.spec.pulse)?;
        self.write_header()?;
        self.write_rows(rows)?;
        self.write_trailer()
    }
}

impl Encoder for BiasEncoder {
    fn plan(&self) -> &EncodePlan {
        &self.plan
    }

    fn step(&mut self, input: EncodeInput<'_>) -> Result<EncodeStep> {
        let height = self.spec.raster.height;
        let need = |this: &Self| {
            if this.done {
                EncodeEvent::Done
            } else {
                EncodeEvent::NeedRows {
                    pass: this.pass,
                    y: this.y,
                }
            }
        };
        let Some(rows) = input.packed() else {
            return match input {
                EncodeInput::None => Ok(EncodeStep::new(0, need(self))),
                _ => Err(error_msg(
                    ErrorKind::Protocol,
                    "this encoder takes packed rows",
                )),
            };
        };
        if self.done
            || rows.width() != self.spec.raster.width
            || rows.descriptor() != self.descriptor
            || rows.rows() > height - self.y
        {
            return Err(error_msg(ErrorKind::Protocol, "rows do not match the spec"));
        }
        let given = rows.rows();
        match &mut self.mode {
            Mode::Whole => {
                if given != height {
                    return Err(error_msg(
                        ErrorKind::Protocol,
                        "a whole-frame encoder takes the frame in one slice",
                    ));
                }
                self.encode_whole(&rows)?;
            }
            Mode::Replay => {
                if self.pass == 0 {
                    self.scan(&rows);
                } else {
                    self.write_rows(&rows)?;
                }
                self.y += given;
                if self.y == height {
                    if self.pass == 0 {
                        self.write_header()?;
                        self.pass = 1;
                        self.y = 0;
                    }
                    // A frame with no rows has nothing to replay.
                    if self.pass == 1 && (self.y == height) {
                        self.write_trailer()?;
                    }
                }
            }
            Mode::Buffer(held) => {
                if held.is_none() {
                    let buffer =
                        PixelBuffer::try_new(self.spec.raster.width, height, self.descriptor)
                            .map_err(|_| error(ErrorKind::OutOfMemory))?;
                    *held = Some(buffer);
                }
                let buffer = held.as_mut().expect("allocated above");
                copy_rows(&rows, &mut buffer.rows_mut(self.y, given))?;
                self.y += given;
                if self.y == height {
                    let buffer = held.take().expect("allocated above");
                    self.encode_whole(&buffer.as_slice())?;
                }
            }
        }
        Ok(EncodeStep::new(given, need(self)))
    }

    fn output(&mut self) -> &mut OutputBuffer {
        &mut self.output
    }
}
