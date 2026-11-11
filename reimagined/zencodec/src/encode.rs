//! Encoding: the decoder's mirror image.
//!
//! ```text
//! EncodeCodec::prepare(&EncodeSpec) ──► Box<dyn Encoder>   (plan() is ready)
//!
//!        ┌──────────── EncodeInput (rows the driver holds) ◄── NeedRows { pass, y }
//!        ▼
//!   Encoder::step ──► EncodeEvent: NeedRows │ NeedFrame │ Done
//!        │
//!        └──► output(): bytes the driver takes between steps
//! ```
//!
//! A decoder names the bytes it wants and is lent a window; an encoder names
//! the rows it wants and is lent a window. Because the demand is a value and
//! not a callback, pushing rows, pulling rows, replaying a source for a second
//! pass, and encoding a whole buffer are one loop: answer `NeedRows` with rows
//! from wherever they are. A host that must *await* its rows (an async
//! pipeline, a foreign runtime) answers when it has them; nothing blocks.
//!
//! The rules every encoder keeps:
//!
//! 1. **`prepare` decides everything.** It settles each [`Demand`] against
//!    what the codec can do, refuses unmet requirements with the requirement
//!    named, checks the gates, and returns an encoder whose
//!    [`plan`](Encoder::plan) states what will happen. It sees no pixels.
//! 2. **`step(EncodeInput::None)` is always allowed** and reports what the
//!    encoder needs. A still starts at `NeedRows { pass: 0, y: 0 }`; a
//!    sequence starts at `NeedFrame`.
//! 3. **Rows answer the last `NeedRows`**: they start at its `y`, in its pass.
//!    [`EncodeStep::rows`] says how many were consumed. An encoder consumes
//!    every row it is given when their count is a multiple of
//!    [`EncodePlan::strip_rows`] or reaches the frame's last row; otherwise
//!    it may leave a remainder for the driver to present again.
//! 4. **The last step does the finishing.** There is no separate finish
//!    call, so a driver that offloads steps offloads finalization too.
//! 5. **Output is the encoder's**: it fills [`output`](Encoder::output) and
//!    the driver takes from it. Draining changes memory, never the bytes.
//! 6. **An error is final**, and bytes already taken are a prefix of a file
//!    that will not be finished.

use crate::bytes::OutputAccess;
use crate::error::Result;
use crate::format::ImageFormat;
use crate::info::{Metadata, MetadataKinds};
use crate::limits::{ResourceEstimate, ResourceLimits, ResourceTargets};
use crate::quality::{Demand, Effort, Fidelity};
use crate::raster::{FrameDuration, Raster};
use alloc::boxed::Box;
use how_far::SharedPulse;
use zenpixels::plane::PlanarSlice;
use zenpixels::{PixelSlice, PixelSliceMut};

pub use crate::bytes::{OutputBuffer, Patch};

/// An encoder for one format, configured and ready to share.
///
/// Codec-specific options live on the implementing type; everything a caller
/// can ask of any codec is in the [`EncodeSpec`].
pub trait EncodeCodec: Send + Sync + 'static {
    /// The codec's name, e.g. `"zenjpeg"`.
    fn name(&self) -> &'static str;

    /// The format it writes.
    fn format(&self) -> ImageFormat;

    /// Admit a request or refuse it, before any pixel moves.
    fn prepare(&self, spec: &EncodeSpec) -> Result<Box<dyn Encoder>>;
}

/// How the caller can present rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowSupply {
    /// Each row once, in order, in strips. A decoder feeding an encoder.
    #[default]
    Once,
    /// Strips, and any pass can be presented again, up to this many passes.
    Replay(u32),
    /// The whole frame is in memory and can be presented as one slice, any
    /// number of times.
    Whole,
}

/// A sequence to encode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SequenceSpec {
    /// Frame count, when known before the first frame.
    pub frames: Option<u32>,
    /// Repeat count; `Some(0)` loops forever.
    pub loops: Option<u32>,
}

impl SequenceSpec {
    /// A sequence of unknown length.
    pub const fn new() -> Self {
        Self {
            frames: None,
            loops: None,
        }
    }

    /// State the frame count.
    pub const fn with_frames(mut self, frames: u32) -> Self {
        self.frames = Some(frames);
        self
    }

    /// State the repeat count.
    pub const fn with_loops(mut self, loops: u32) -> Self {
        self.loops = Some(loops);
        self
    }
}

/// Everything a caller can ask of any encoder. Owns its data, so it can move
/// to a worker thread or across a foreign-function boundary.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct EncodeSpec {
    /// The samples that will be supplied: size, layout and meaning.
    pub raster: Raster,
    /// `Some` to encode a sequence of frames of that raster.
    pub sequence: Option<SequenceSpec>,
    /// How closely the output must match.
    pub fidelity: Demand<Fidelity>,
    /// How hard to work.
    pub effort: Demand<Effort>,
    /// Exactly the metadata to embed.
    pub metadata: Metadata,
    /// The kinds that must be embedded; the rest are embedded when the format
    /// can carry them. [`EncodePlan::embedded`] says which were.
    pub require_metadata: MetadataKinds,
    /// How the caller can present rows.
    pub supply: RowSupply,
    /// Whether the destination can be patched.
    pub output: OutputAccess,
    /// Gates.
    pub limits: ResourceLimits,
    /// Targets.
    pub targets: ResourceTargets,
    /// Progress and cancellation, shared with whoever started the work.
    pub pulse: SharedPulse,
}

impl EncodeSpec {
    /// A still of this raster, codec defaults, rows offered once.
    pub fn new(raster: Raster) -> Self {
        Self {
            raster,
            sequence: None,
            fidelity: Demand::Default,
            effort: Demand::Default,
            metadata: Metadata::default(),
            require_metadata: MetadataKinds::NONE,
            supply: RowSupply::Once,
            output: OutputAccess::Forward,
            limits: ResourceLimits::none(),
            targets: ResourceTargets::auto(),
            pulse: SharedPulse::default(),
        }
    }

    /// Encode a sequence.
    pub fn with_sequence(mut self, sequence: SequenceSpec) -> Self {
        self.sequence = Some(sequence);
        self
    }

    /// Ask for or insist on a fidelity.
    pub fn with_fidelity(mut self, fidelity: Demand<Fidelity>) -> Self {
        self.fidelity = fidelity;
        self
    }

    /// Ask for or insist on an effort.
    pub fn with_effort(mut self, effort: Demand<Effort>) -> Self {
        self.effort = effort;
        self
    }

    /// Embed this metadata, insisting on `required`.
    pub fn with_metadata(mut self, metadata: Metadata, required: MetadataKinds) -> Self {
        self.metadata = metadata;
        self.require_metadata = required;
        self
    }

    /// State how rows can be presented.
    pub fn with_supply(mut self, supply: RowSupply) -> Self {
        self.supply = supply;
        self
    }

    /// State whether the destination can be patched.
    pub fn with_output(mut self, output: OutputAccess) -> Self {
        self.output = output;
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

    /// Share progress and cancellation.
    pub fn with_pulse(mut self, pulse: SharedPulse) -> Self {
        self.pulse = pulse;
        self
    }
}

/// How an encoder wants its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowDemand {
    /// Strips, each row once.
    Strips,
    /// The whole frame in one slice. Only planned when the supply is
    /// [`RowSupply::Whole`].
    Whole,
    /// Strips, over this many passes. Only planned within a
    /// [`RowSupply::Replay`] or [`RowSupply::Whole`] offer.
    Replay(u32),
}

/// When an encoder's bytes become available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputTiming {
    /// As rows arrive, trailing them by at most this many rows.
    Streaming {
        /// Rows pushed before their bytes appear.
        lag_rows: u32,
    },
    /// All at the end.
    AtFinish,
}

/// What a prepared encoder will do.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct EncodePlan {
    /// How it wants rows.
    pub input: RowDemand,
    /// The strip height it consumes whole; 1 when any count will do.
    pub strip_rows: u32,
    /// When its bytes appear.
    pub output: OutputTiming,
    /// The fidelity it will deliver, settled from the spec's demand.
    pub fidelity: Fidelity,
    /// The metadata kinds it will embed.
    pub embedded: MetadataKinds,
    /// What it expects to use.
    pub estimate: ResourceEstimate,
}

impl EncodePlan {
    /// A plan that takes strips once and holds its output to the end.
    pub fn new(fidelity: Fidelity, estimate: ResourceEstimate) -> Self {
        Self {
            input: RowDemand::Strips,
            strip_rows: 1,
            output: OutputTiming::AtFinish,
            fidelity,
            embedded: MetadataKinds::NONE,
            estimate,
        }
    }

    /// State the row demand.
    pub fn with_input(mut self, input: RowDemand) -> Self {
        self.input = input;
        self
    }

    /// State the strip height.
    pub fn with_strip_rows(mut self, strip_rows: u32) -> Self {
        self.strip_rows = strip_rows.max(1);
        self
    }

    /// State the output timing.
    pub fn with_output(mut self, output: OutputTiming) -> Self {
        self.output = output;
        self
    }

    /// State the embedded metadata.
    pub fn with_embedded(mut self, embedded: MetadataKinds) -> Self {
        self.embedded = embedded;
        self
    }
}

/// What a frame of a sequence adds to the spec's raster.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameMeta {
    /// How long the frame is shown.
    pub duration: Option<FrameDuration>,
}

impl FrameMeta {
    /// A frame shown for `duration`.
    pub const fn shown_for(duration: FrameDuration) -> Self {
        Self {
            duration: Some(duration),
        }
    }
}

/// What the driver hands an encoder for one step.
#[derive(Debug)]
#[non_exhaustive]
pub enum EncodeInput<'a> {
    /// Nothing new: report the current need, or continue work in hand.
    None,
    /// Packed rows answering the last `NeedRows`.
    Rows(PixelSlice<'a>),
    /// Packed rows the encoder may overwrite while it works. The caller does
    /// not rely on their contents afterwards.
    Scratch(PixelSliceMut<'a>),
    /// A planar strip answering the last `NeedRows`.
    Planes(PlanarSlice<'a>),
    /// The next frame of a sequence begins; answers `NeedFrame`.
    Frame(FrameMeta),
    /// The sequence is complete; answers `NeedFrame`.
    End,
}

impl EncodeInput<'_> {
    /// The packed rows, whether or not the encoder may overwrite them.
    pub fn packed(&self) -> Option<PixelSlice<'_>> {
        match self {
            Self::Rows(rows) => Some(rows.sub_rows(0, rows.rows())),
            Self::Scratch(rows) => Some(rows.as_pixel_slice()),
            _ => None,
        }
    }
}

/// What an encoder needs next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodeEvent {
    /// Rows of the current frame, starting at `y`, for this pass.
    NeedRows {
        /// The pass, from zero.
        pass: u32,
        /// The first row wanted.
        y: u32,
    },
    /// The next frame, or the end of the sequence.
    NeedFrame,
    /// The output is complete.
    Done,
}

/// The result of one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeStep {
    /// Rows consumed from the front of the input.
    pub rows: u32,
    /// What the encoder needs next.
    pub event: EncodeEvent,
}

impl EncodeStep {
    /// A step.
    pub const fn new(rows: u32, event: EncodeEvent) -> Self {
        Self { rows, event }
    }
}

/// An encode in progress. See the [module rules](self).
pub trait Encoder: Send + 'static {
    /// What this encode will do. Fixed at `prepare`.
    fn plan(&self) -> &EncodePlan;

    /// Take some rows and do some work.
    fn step(&mut self, input: EncodeInput<'_>) -> Result<EncodeStep>;

    /// The encoded bytes not yet taken.
    fn output(&mut self) -> &mut OutputBuffer;
}
