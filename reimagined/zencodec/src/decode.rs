//! Decoding: one state machine.
//!
//! ```text
//! DecodeCodec::decoder(&DecodeSpec) ──► Box<dyn Decoder>
//!
//!        ┌──────────── Input (bytes the driver holds) ◄── requested()
//!        ▼
//!   Decoder::step ──► DecodeEvent: NeedInput │ Info │ Rows │ FrameDone │ Done
//!        │
//!        └──► Sink: begin, then lend / rows / frame / planes, then end
//! ```
//!
//! One-shot, streaming, push, pull, animation and range-driven decoding are
//! this loop under different drivers. The rules every decoder keeps:
//!
//! 1. **[`decoder`](DecodeCodec::decoder) is cheap.** It validates the spec and allocates nothing
//!    proportional to an image; it has seen no input.
//! 2. **`step` never waits.** It consumes a prefix of the window it is lent
//!    and returns. [`requested`](Decoder::requested) always names the window
//!    the next step needs, and the driver lends the window that answers its
//!    first entry before every step. [`NeedInput`](DecodeEvent::NeedInput)
//!    says the window lent was not enough.
//! 3. **[`Info`](DecodeEvent::Info) comes first and once**, before any sink
//!    call and before any allocation proportional to the image. From then on
//!    [`info`](Decoder::info) and [`plan`](Decoder::plan) are `Some`.
//!    Stepping again is acceptance; dropping the decoder is refusal. Gates in
//!    the spec are checked by the decoder at this point too.
//! 4. **Per frame**: `begin`, rows covering the frame, `end`, then
//!    [`FrameDone`](DecodeEvent::FrameDone). After the last frame,
//!    [`Done`](DecodeEvent::Done).
//! 5. **An error is final.** A decoder is not stepped after one.
//! 6. **Progress and cancellation** arrive with every step as an optional borrowed
//!    [`Pulse`]. `None` disables both. A decoder checks it around expensive work and reports a stop
//!    as [`ErrorKind::Stopped`]. It keeps no reference to the pulse between
//!    steps; a decoder with threads of its own that outlive a step asks for
//!    [`Pulse::share`] and refuses when the pulse cannot be shared.

use crate::bytes::{Input, InputRequest};
use crate::error::{ErrorKind, Result, Unsupported, error_msg, unsupported};
use crate::format::ImageFormat;
use crate::info::{ImageInfo, MetadataKinds};
use crate::limits::{ResourceEstimate, ResourceLimits, ResourceTargets};
use crate::raster::{FrameHeader, Pass, Raster, copy_rows};
use alloc::boxed::Box;
use how_far::Pulse;
use zenpixels::plane::PlanarSlice;
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice, PixelSliceMut};

/// A decoder for one or more formats, configured and ready to share.
///
/// Codec-specific options live on the implementing type; everything a caller
/// can ask of any codec is in the [`DecodeSpec`].
pub trait DecodeCodec: Send + Sync + 'static {
    /// The codec's name, e.g. `"zenjpeg"`.
    fn name(&self) -> &'static str;

    /// The formats it reads.
    fn formats(&self) -> &[ImageFormat];

    /// Start a decode. Refuses, with the unmet requirement named, when the
    /// spec asks for something this codec can never do.
    fn decoder(&self, spec: &DecodeSpec) -> Result<Box<dyn Decoder>>;
}

/// The pixel formats a caller will take, in order of preference.
#[derive(Clone, Copy, Debug)]
pub struct Accept {
    list: [PixelDescriptor; Self::MAX],
    len: u8,
    required: bool,
}

impl Accept {
    /// Most formats a list can hold.
    pub const MAX: usize = 8;

    /// Whatever the decoder produces most directly.
    pub const fn native() -> Self {
        Self {
            list: [PixelDescriptor::RGBA8_SRGB; Self::MAX],
            len: 0,
            required: false,
        }
    }

    /// The first of these the decoder can produce without loss, else its
    /// native format.
    ///
    /// # Panics
    ///
    /// If `formats` holds more than [`MAX`](Self::MAX) entries.
    pub const fn prefer(formats: &[PixelDescriptor]) -> Self {
        Self::from_list(formats, false)
    }

    /// The first of these the decoder can produce, else refusal.
    ///
    /// # Panics
    ///
    /// If `formats` is empty or holds more than [`MAX`](Self::MAX) entries.
    pub const fn require(formats: &[PixelDescriptor]) -> Self {
        assert!(!formats.is_empty(), "a requirement needs a format");
        Self::from_list(formats, true)
    }

    const fn from_list(formats: &[PixelDescriptor], required: bool) -> Self {
        assert!(formats.len() <= Self::MAX, "too many formats");
        let mut list = [PixelDescriptor::RGBA8_SRGB; Self::MAX];
        let mut i = 0;
        while i < formats.len() {
            list[i] = formats[i];
            i += 1;
        }
        Self {
            list,
            len: formats.len() as u8,
            required,
        }
    }

    /// The ranked formats.
    pub fn formats(&self) -> &[PixelDescriptor] {
        &self.list[..self.len as usize]
    }

    /// Whether a format outside the list is refused.
    pub const fn is_required(&self) -> bool {
        self.required
    }

    /// Settle on an output format.
    ///
    /// `native` is what the decoder produces most directly and `convertible`
    /// what else it can produce without loss.
    pub fn choose(
        &self,
        native: PixelDescriptor,
        convertible: &[PixelDescriptor],
    ) -> Result<PixelDescriptor, Unsupported> {
        for want in self.formats() {
            if *want == native || convertible.contains(want) {
                return Ok(*want);
            }
        }
        if self.required {
            Err(Unsupported::PixelFormat)
        } else {
            Ok(native)
        }
    }
}

impl Default for Accept {
    fn default() -> Self {
        Self::native()
    }
}

/// Which frames to decode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Frames {
    /// The first frame only.
    #[default]
    First,
    /// Every frame, in order.
    All,
}

/// Everything a caller can ask of any decoder. Plain owned data, so it can
/// move to a worker thread or across a foreign-function boundary.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct DecodeSpec {
    /// Packed formats the caller will take.
    pub accept: Accept,
    /// Whether the sink takes planar strips when the file is planar.
    pub planar: bool,
    /// Which frames.
    pub frames: Frames,
    /// Whether the sink wants provisional passes.
    pub progressive: bool,
    /// Metadata that must be settled in [`ImageInfo`] at
    /// [`Info`](DecodeEvent::Info), before any pixel leaves. A decoder that
    /// would have to read past the pixels asks for those bytes first, or
    /// refuses.
    pub metadata_first: MetadataKinds,
    /// Gates.
    pub limits: ResourceLimits,
    /// Targets.
    pub targets: ResourceTargets,
}

impl DecodeSpec {
    /// Native format, first frame, no gates.
    pub fn new() -> Self {
        Self::default()
    }

    /// State the packed formats the caller will take.
    pub fn with_accept(mut self, accept: Accept) -> Self {
        self.accept = accept;
        self
    }

    /// Take planar strips when the file is planar.
    pub fn with_planar(mut self, planar: bool) -> Self {
        self.planar = planar;
        self
    }

    /// Choose the frames.
    pub fn with_frames(mut self, frames: Frames) -> Self {
        self.frames = frames;
        self
    }

    /// Ask for provisional passes.
    pub fn with_progressive(mut self, progressive: bool) -> Self {
        self.progressive = progressive;
        self
    }

    /// Require these metadata kinds to be settled before pixels.
    pub fn with_metadata_first(mut self, kinds: MetadataKinds) -> Self {
        self.metadata_first = kinds;
        self
    }

    /// Set the gates.
    pub fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Set the targets.
    pub fn with_targets(mut self, targets: ResourceTargets) -> Self {
        self.targets = targets;
        self
    }
}

/// How a decoder reads its input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Access {
    /// Front to back.
    #[default]
    Sequential,
    /// It will ask for ranges out of order.
    Random,
}

/// How a decoder hands over rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Delivery {
    /// In strips, as input arrives.
    Strips,
    /// One complete frame at a time.
    WholeFrame,
}

/// What a decoder will do, known once the header is read.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DecodePlan {
    /// The samples each frame will arrive as. This is the settled output,
    /// after format negotiation; the file's own layout is in [`ImageInfo`].
    pub raster: Raster,
    /// Strips or whole frames.
    pub delivery: Delivery,
    /// Sequential or random input.
    pub access: Access,
    /// What it expects to use.
    pub estimate: ResourceEstimate,
}

impl DecodePlan {
    /// A plan.
    pub fn new(raster: Raster, delivery: Delivery, estimate: ResourceEstimate) -> Self {
        Self {
            raster,
            delivery,
            access: Access::Sequential,
            estimate,
        }
    }

    /// State the input access.
    pub fn with_access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }
}

/// What a step did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeEvent {
    /// The window lent was not enough; see [`Decoder::requested`].
    NeedInput,
    /// The header is read: [`Decoder::info`] and [`Decoder::plan`] are set and
    /// nothing has reached the sink.
    Info,
    /// The sink was called (a frame began, or rows arrived) and there is more
    /// to do with the input in hand. A driver may yield here.
    Rows,
    /// A frame is complete.
    FrameDone,
    /// Everything asked for is decoded.
    Done,
}

/// The result of one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeStep {
    /// Bytes consumed from the front of the input.
    pub consumed: usize,
    /// What happened.
    pub event: DecodeEvent,
}

impl DecodeStep {
    /// A step.
    pub const fn new(consumed: usize, event: DecodeEvent) -> Self {
        Self { consumed, event }
    }
}

/// A decode in progress. See the [module rules](self).
pub trait Decoder: Send + 'static {
    /// The bytes the next step needs: always current, from the start. The
    /// first entry is the window to lend; later entries forecast ranges a
    /// driver may fetch with it.
    fn requested(&self) -> &[InputRequest];

    /// What the file says about itself; `Some` from [`Info`](DecodeEvent::Info)
    /// on, and complete at [`Done`](DecodeEvent::Done).
    fn info(&self) -> Option<&ImageInfo>;

    /// What this decode will do; `Some` from [`Info`](DecodeEvent::Info) on.
    fn plan(&self) -> Option<&DecodePlan>;

    /// Do some work with the input in hand, observed by `pulse`.
    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn Sink,
        pulse: Option<&dyn Pulse>,
    ) -> Result<DecodeStep>;
}

/// Where decoded rows go.
///
/// Three ways to hand over packed rows, so that no decoder copies to fit the
/// interface. A sink implements [`lend`](Self::lend); the other two default
/// to it and are worth overriding when the sink can use rows in place.
///
/// - **`lend`**: the sink owns the destination and the decoder writes into
///   it. Right for decoders that produce rows as they go.
/// - **`rows`**: the decoder already holds finished rows and shows them.
///   Right when rows sit in the decoder's or the driver's memory.
/// - **`frame`**: the decoder gives away a finished frame. Right for
///   decoders that can only produce the whole image.
///
/// Planar strips arrive through [`planes`](Self::planes), borrowed from the
/// decoder, and only when the spec said the sink takes them.
pub trait Sink {
    /// A frame is about to arrive. Refuse here, before any row moves.
    fn begin(&mut self, frame: &FrameHeader) -> Result<()>;

    /// Lend a buffer for rows `y..y + rows`. The buffer holds between one and
    /// `rows` rows of the frame's width and format; the decoder asks again
    /// for the rest. Rows lent are complete when the sink is next called.
    fn lend(&mut self, y: u32, rows: u32, pass: Pass) -> Result<PixelSliceMut<'_>>;

    /// Take rows the decoder holds, starting at row `y`.
    fn rows(&mut self, y: u32, rows: PixelSlice<'_>, pass: Pass) -> Result<()> {
        let mut done = 0;
        while done < rows.rows() {
            let mut lent = self.lend(y + done, rows.rows() - done, pass)?;
            let n = lent.rows().min(rows.rows() - done);
            if n == 0 {
                return Err(error_msg(ErrorKind::Sink, "sink lent no rows"));
            }
            copy_rows(&rows.sub_rows(done, n), &mut lent)?;
            done += n;
        }
        Ok(())
    }

    /// Take a complete frame, and its allocation.
    fn frame(&mut self, pixels: PixelBuffer) -> Result<()> {
        self.rows(0, pixels.as_slice(), Pass::Final)
    }

    /// Take a planar strip whose first luma row is `y`.
    fn planes(&mut self, y: u32, strip: PlanarSlice<'_>) -> Result<()> {
        let _ = (y, strip);
        Err(unsupported(Unsupported::Planar))
    }

    /// The frame is complete.
    fn end(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<S: Sink + ?Sized> Sink for &mut S {
    fn begin(&mut self, frame: &FrameHeader) -> Result<()> {
        (**self).begin(frame)
    }
    fn lend(&mut self, y: u32, rows: u32, pass: Pass) -> Result<PixelSliceMut<'_>> {
        (**self).lend(y, rows, pass)
    }
    fn rows(&mut self, y: u32, rows: PixelSlice<'_>, pass: Pass) -> Result<()> {
        (**self).rows(y, rows, pass)
    }
    fn frame(&mut self, pixels: PixelBuffer) -> Result<()> {
        (**self).frame(pixels)
    }
    fn planes(&mut self, y: u32, strip: PlanarSlice<'_>) -> Result<()> {
        (**self).planes(y, strip)
    }
    fn end(&mut self) -> Result<()> {
        (**self).end()
    }
}
