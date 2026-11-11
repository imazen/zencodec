//! The short path for codec authors: bytes in, pixels out, and the reverse.
//!
//! A codec that works on a whole file and a whole image implements
//! [`WholeImageDecode`] or [`WholeImageEncode`] (two or three functions) and
//! wraps itself in [`WholeImage`]. The wrapper supplies the state machine,
//! the gates, format negotiation, cancellation checks and the requirement
//! refusals, so a simple codec is a full citizen of the contract without
//! learning it. It claims nothing the codec did not state: memory is
//! unbounded unless the codec bounds it, and no metadata is embedded unless
//! the codec says it embeds it.

use crate::bytes::{Input, InputRequest, OutputAccess, OutputBuffer};
use crate::decode::{
    DecodeCodec, DecodeEvent, DecodePlan, DecodeSpec, DecodeStep, Decoder, Delivery, Sink,
};
use crate::encode::{
    EncodeCodec, EncodeEvent, EncodeInput, EncodePlan, EncodeSpec, EncodeStep, Encoder, RowDemand,
    RowSupply,
};
use crate::error::{CodecError, ErrorKind, Result, Unsupported, check, error, error_msg};
use crate::format::ImageFormat;
use crate::info::{ImageInfo, MetadataKinds};
use crate::limits::{Bound, ResourceEstimate};
use crate::quality::{Demand, Fidelity};
use crate::raster::{FrameHeader, Raster, copy_rows};
use alloc::boxed::Box;
use alloc::vec::Vec;
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

/// A decoder that needs the whole file and produces the whole image.
pub trait WholeImageDecode: Clone + Send + Sync + 'static {
    /// The codec's name.
    fn name(&self) -> &'static str;

    /// The formats it reads.
    fn formats(&self) -> &[ImageFormat];

    /// Read the header from the start of the file. `Ok(None)` asks for more
    /// bytes. `info.native` must be a packed layout.
    fn probe(&self, bytes: &[u8]) -> Result<Option<ImageInfo>>;

    /// Formats `decode` can produce besides the native one.
    fn convertible(&self, info: &ImageInfo) -> &[PixelDescriptor] {
        let _ = info;
        &[]
    }

    /// The most memory `decode` will own, when the codec can say.
    fn memory_bound(&self, info: &ImageInfo) -> Bound {
        let _ = info;
        Bound::Unknown
    }

    /// Decode the complete file into `output`.
    fn decode(
        &self,
        bytes: &[u8],
        info: &ImageInfo,
        output: PixelDescriptor,
    ) -> Result<PixelBuffer>;
}

/// An encoder that takes the whole image and returns the whole file.
pub trait WholeImageEncode: Clone + Send + Sync + 'static {
    /// The codec's name.
    fn name(&self) -> &'static str;

    /// The format it writes.
    fn format(&self) -> ImageFormat;

    /// The packed formats it encodes.
    fn accepts(&self) -> &[PixelDescriptor];

    /// The fidelity it would deliver for a requested one, or `None` when it
    /// cannot honor it. The default suits a lossless-only codec.
    fn fidelity(&self, want: &Fidelity) -> Option<Fidelity> {
        want.is_lossless().then_some(Fidelity::Lossless)
    }

    /// The fidelity it delivers when the caller does not ask.
    fn default_fidelity(&self) -> Fidelity {
        Fidelity::Lossless
    }

    /// The metadata kinds it writes when given them.
    fn embeds(&self) -> MetadataKinds {
        MetadataKinds::NONE
    }

    /// Encode a complete image. `spec.fidelity` has been settled to `fidelity`.
    fn encode(
        &self,
        pixels: PixelSlice<'_>,
        fidelity: Fidelity,
        spec: &EncodeSpec,
    ) -> Result<Vec<u8>>;
}

/// Makes a [`WholeImageDecode`] or [`WholeImageEncode`] a contract codec.
#[derive(Clone, Debug)]
pub struct WholeImage<C>(pub C);

// ───────────────────────────── decode ─────────────────────────────

impl<C: WholeImageDecode> DecodeCodec for WholeImage<C> {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn formats(&self) -> &[ImageFormat] {
        self.0.formats()
    }

    fn decoder(&self, spec: &DecodeSpec) -> Result<Box<dyn Decoder>> {
        Ok(Box::new(WholeDecoder {
            codec: self.0.clone(),
            spec: spec.clone(),
            request: [InputRequest::AtLeast {
                offset: 0,
                min_len: 64,
            }],
            state: DecodeState::Header,
        }))
    }
}

enum DecodeState {
    Header,
    Body {
        info: ImageInfo,
        plan: DecodePlan,
        output: PixelDescriptor,
    },
    Done {
        info: ImageInfo,
        plan: DecodePlan,
    },
    Failed,
}

struct WholeDecoder<C> {
    codec: C,
    spec: DecodeSpec,
    request: [InputRequest; 1],
    state: DecodeState,
}

impl<C: WholeImageDecode> WholeDecoder<C> {
    fn header(&mut self, input: Input<'_>) -> Result<DecodeStep> {
        if input.offset != 0 {
            return Err(error_msg(
                ErrorKind::Protocol,
                "header input must start at 0",
            ));
        }
        let Some(info) = self.codec.probe(input.bytes)? else {
            if input.reaches_end() {
                return Err(CodecError::new(ErrorKind::Truncated)
                    .with_codec(self.codec.name())
                    .at());
            }
            // Ask for twice as much as was not enough.
            self.request = [InputRequest::AtLeast {
                offset: 0,
                min_len: input.bytes.len().max(32) * 2,
            }];
            return Ok(DecodeStep::new(0, DecodeEvent::NeedInput));
        };
        let Some(native) = info.native.descriptor() else {
            return Err(error_msg(
                ErrorKind::Internal,
                "a whole-image decoder reports a packed native layout",
            ));
        };
        self.spec.limits.check_dimensions(info.width, info.height)?;
        let estimate = ResourceEstimate::serial(self.codec.memory_bound(&info));
        self.spec.limits.check_memory(estimate.memory_bytes)?;
        let output = self
            .spec
            .accept
            .choose(native, self.codec.convertible(&info))
            .map_err(|u| {
                CodecError::new(ErrorKind::Unsupported(u))
                    .with_codec(self.codec.name())
                    .at()
            })?;
        let plan = DecodePlan::new(
            Raster::packed(info.width, info.height, output),
            Delivery::WholeFrame,
            estimate,
        );
        self.request = [InputRequest::All];
        self.state = DecodeState::Body { info, plan, output };
        Ok(DecodeStep::new(0, DecodeEvent::Info))
    }
}

impl<C: WholeImageDecode> Decoder for WholeDecoder<C> {
    fn requested(&self) -> &[InputRequest] {
        &self.request
    }

    fn info(&self) -> Option<&ImageInfo> {
        match &self.state {
            DecodeState::Body { info, .. } | DecodeState::Done { info, .. } => Some(info),
            _ => None,
        }
    }

    fn plan(&self) -> Option<&DecodePlan> {
        match &self.state {
            DecodeState::Body { plan, .. } | DecodeState::Done { plan, .. } => Some(plan),
            _ => None,
        }
    }

    fn step(&mut self, input: Input<'_>, sink: &mut dyn Sink) -> Result<DecodeStep> {
        match core::mem::replace(&mut self.state, DecodeState::Failed) {
            DecodeState::Header => {
                self.state = DecodeState::Header;
                let step = self.header(input);
                if step.is_err() {
                    self.state = DecodeState::Failed;
                }
                step
            }
            DecodeState::Body { info, plan, output } => {
                if !InputRequest::All.is_met_by(&input) {
                    self.state = DecodeState::Body { info, plan, output };
                    return Ok(DecodeStep::new(0, DecodeEvent::NeedInput));
                }
                check(&self.spec.pulse)?;
                let pixels = self.codec.decode(input.bytes, &info, output)?;
                if pixels.descriptor() != output
                    || pixels.width() != info.width
                    || pixels.height() != info.height
                {
                    return Err(error_msg(
                        ErrorKind::Internal,
                        "decode returned a different image than probe described",
                    ));
                }
                check(&self.spec.pulse)?;
                sink.begin(&FrameHeader::still(plan.raster.clone()))?;
                sink.frame(pixels)?;
                sink.end()?;
                self.state = DecodeState::Done { info, plan };
                Ok(DecodeStep::new(input.bytes.len(), DecodeEvent::Done))
            }
            DecodeState::Done { .. } | DecodeState::Failed => Err(error_msg(
                ErrorKind::Protocol,
                "step after the decode ended",
            )),
        }
    }
}

// ───────────────────────────── encode ─────────────────────────────

impl<C: WholeImageEncode> EncodeCodec for WholeImage<C> {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn format(&self) -> ImageFormat {
        self.0.format()
    }

    fn encoder(&self, spec: &EncodeSpec) -> Result<Box<dyn Encoder>> {
        let refuse = |what| {
            CodecError::new(ErrorKind::Unsupported(what))
                .with_codec(self.0.name())
                .at()
        };
        if spec.sequence.is_some() {
            return Err(refuse(Unsupported::Animation));
        }
        let Some(descriptor) = spec.raster.descriptor() else {
            return Err(refuse(Unsupported::Planar));
        };
        if !self.0.accepts().contains(&descriptor) {
            return Err(refuse(Unsupported::PixelFormat));
        }
        if matches!(spec.effort, Demand::Require(_)) {
            return Err(refuse(Unsupported::Effort));
        }
        let fidelity = spec
            .fidelity
            .resolve(self.0.default_fidelity(), |want| self.0.fidelity(want))
            .map_err(|_| refuse(Unsupported::Fidelity))?;
        let embedded = spec.metadata.kinds().intersection(self.0.embeds());
        if !embedded.contains(spec.require_metadata) {
            return Err(refuse(Unsupported::Metadata));
        }
        spec.limits
            .check_dimensions(spec.raster.width, spec.raster.height)?;

        // With the whole frame on offer the encoder borrows it; otherwise it
        // must hold the rows itself, and says so.
        let whole = spec.supply == RowSupply::Whole;
        let held = if whole {
            0
        } else {
            let row = spec
                .raster
                .row_bytes()
                .ok_or_else(|| refuse(Unsupported::Dimensions))?;
            (row as u64).saturating_mul(spec.raster.height.into())
        };
        // The codec's own working memory is not bounded by this trait.
        let memory = if spec.limits.max_memory_bytes.is_some() {
            Bound::Unknown
        } else {
            Bound::AtMost(held)
        };
        spec.limits.check_memory(memory)?;
        let plan = EncodePlan::new(fidelity, ResourceEstimate::serial(memory))
            .with_input(if whole {
                RowDemand::Whole
            } else {
                RowDemand::Strips
            })
            .with_embedded(embedded);

        let mut output = OutputBuffer::new(OutputAccess::Forward, spec.limits.max_output_bytes);
        output.hold();
        Ok(Box::new(WholeEncoder {
            codec: self.0.clone(),
            spec: spec.clone(),
            plan,
            descriptor,
            held: None,
            y: 0,
            done: false,
            output,
        }))
    }
}

struct WholeEncoder<C> {
    codec: C,
    spec: EncodeSpec,
    plan: EncodePlan,
    descriptor: PixelDescriptor,
    held: Option<PixelBuffer>,
    y: u32,
    done: bool,
    output: OutputBuffer,
}

impl<C: WholeImageEncode> WholeEncoder<C> {
    fn finish(&mut self, pixels: PixelSlice<'_>) -> Result<()> {
        check(&self.spec.pulse)?;
        let bytes = self.codec.encode(pixels, self.plan.fidelity, &self.spec)?;
        self.output.write_owned(bytes)?;
        self.output.release();
        self.done = true;
        Ok(())
    }
}

impl<C: WholeImageEncode> Encoder for WholeEncoder<C> {
    fn plan(&self) -> &EncodePlan {
        &self.plan
    }

    fn step(&mut self, input: EncodeInput<'_>) -> Result<EncodeStep> {
        if self.done {
            return match input {
                EncodeInput::None => Ok(EncodeStep::new(0, EncodeEvent::Done)),
                _ => Err(error_msg(ErrorKind::Protocol, "rows after the image ended")),
            };
        }
        let (width, height) = (self.spec.raster.width, self.spec.raster.height);
        let Some(rows) = input.packed() else {
            return match input {
                EncodeInput::None => Ok(EncodeStep::new(
                    0,
                    EncodeEvent::NeedRows { pass: 0, y: self.y },
                )),
                _ => Err(error_msg(
                    ErrorKind::Protocol,
                    "this encoder takes packed rows",
                )),
            };
        };
        if rows.width() != width
            || rows.descriptor() != self.descriptor
            || rows.rows() > height - self.y
        {
            return Err(error_msg(ErrorKind::Protocol, "rows do not match the spec"));
        }
        let given = rows.rows();
        if self.plan.input == RowDemand::Whole {
            if given != height {
                return Err(error_msg(
                    ErrorKind::Protocol,
                    "a whole-frame encoder takes the frame in one slice",
                ));
            }
            self.finish(rows)?;
            return Ok(EncodeStep::new(given, EncodeEvent::Done));
        }
        if self.held.is_none() {
            let buffer = PixelBuffer::try_new(width, height, self.descriptor)
                .map_err(|_| error(ErrorKind::OutOfMemory))?;
            self.held = Some(buffer);
        }
        let held = self.held.as_mut().expect("allocated above");
        copy_rows(&rows, &mut held.rows_mut(self.y, given))?;
        self.y += given;
        if self.y < height {
            return Ok(EncodeStep::new(
                given,
                EncodeEvent::NeedRows { pass: 0, y: self.y },
            ));
        }
        let held = self.held.take().expect("allocated above");
        self.finish(held.as_slice())?;
        Ok(EncodeStep::new(given, EncodeEvent::Done))
    }

    fn output(&mut self) -> &mut OutputBuffer {
        &mut self.output
    }
}
