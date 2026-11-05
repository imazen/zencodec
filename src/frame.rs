//! Immutable native-frame interpretation and timing facts.
//!
//! Interpretation and mapped frame owners are shared by still-image and video
//! operations. Mapping borrows native storage without conversion. Media codec
//! sessions and accounted software allocations stay outside this crate.

pub use crate::frame_storage::{Frame, MapError, MappedPlanes, Picture, PlaneError, Presentation};
use zenpixels::{
    plane::{PlanarLayout, PlaneEncoding},
    sample::SampleEncoding,
};

use crate::time::Timestamp;
use zenpixels::Cicp;

pub use zenpixels::plane::{AnyPlane, PlaneRef, PlaneRole};

/// A rectangle in coded coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Crop {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// The shape of a decoded picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FrameGeometry {
    /// Coded (stored) width.
    pub coded_width: u32,
    /// Coded (stored) height.
    pub coded_height: u32,
    /// Visible region in coded coordinates. Odd origins are kept, so the
    /// chroma phase of a cropped 4:2:0 picture stays known.
    pub crop: Crop,
    /// Intended display size, when the stream says so; pixels are not
    /// resized.
    pub render: Option<(u32, u32)>,
}

impl FrameGeometry {
    /// A `width` x `height` picture, all of it visible.
    pub const fn full(width: u32, height: u32) -> Self {
        Self {
            coded_width: width,
            coded_height: height,
            crop: Crop {
                x: 0,
                y: 0,
                width,
                height,
            },
            render: None,
        }
    }
}

/// Colour claims with their provenance. The bitstream and the container may
/// disagree; `resolved` is what the adapter's format rule chose (AV1 in
/// ISOBMFF: fill and agree rules). Unknown stays unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ColorClaim {
    /// What the bitstream signals.
    pub bitstream: Option<Cicp>,
    /// What the container signals.
    pub container: Option<Cicp>,
    /// The claim the adapter resolved.
    pub resolved: Option<Cicp>,
}

impl ColorClaim {
    /// Claims from both carriers and the resolved one.
    pub const fn new(
        bitstream: Option<Cicp>,
        container: Option<Cicp>,
        resolved: Option<Cicp>,
    ) -> Self {
        Self {
            bitstream,
            container,
            resolved,
        }
    }
}

/// Everything needed to interpret the samples of one frame. Immutable and
/// shared through `Arc`: a configuration change makes a new one, and a frame
/// keeps the one it was decoded under.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameInterpretation {
    /// Picture shape.
    pub geometry: FrameGeometry,
    /// Validated roles, per-plane encodings, subsampling and siting.
    /// Storage owners must agree with this layout; no frame-wide depth is assumed.
    pub layout: PlanarLayout,
    /// Colour claims.
    pub color: ColorClaim,
    /// The codec configuration epoch it was decoded under.
    pub config_epoch: u32,
}

impl FrameInterpretation {
    /// An interpretation with no colour claims, at epoch 0.
    pub fn new(geometry: FrameGeometry, layout: PlanarLayout) -> Self {
        Self {
            geometry,
            layout,
            color: ColorClaim::default(),
            config_epoch: 0,
        }
    }

    /// A common integer encoding only when every plane has exactly that encoding.
    /// Mixed-depth or floating-point layouts return `None`.
    pub fn uniform_code_encoding(&self) -> Option<SampleEncoding> {
        let PlaneEncoding::Code(first) = self.layout.planes().first()?.encoding() else {
            return None;
        };
        self.layout
            .planes()
            .iter()
            .all(|p| p.encoding() == PlaneEncoding::Code(first))
            .then_some(first)
    }

    /// The colour claims.
    pub fn with_color(mut self, color: ColorClaim) -> Self {
        self.color = color;
        self
    }

    /// The configuration epoch.
    pub fn with_config_epoch(mut self, epoch: u32) -> Self {
        self.config_epoch = epoch;
        self
    }
}

/// Timing of one presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct FrameTiming {
    /// Presentation timestamp, when known. Missing stays missing.
    pub pts: Option<Timestamp>,
    /// Duration in ticks of `pts`'s base, when the container carried one.
    pub duration_ticks: Option<u64>,
    /// Ordinal of the packet this presentation came from.
    pub packet_ordinal: Option<u64>,
    /// File offset of that packet.
    pub input_offset: Option<u64>,
}

impl FrameTiming {
    /// Timing with only a presentation timestamp.
    pub const fn at(pts: Timestamp) -> Self {
        Self {
            pts: Some(pts),
            duration_ticks: None,
            packet_ordinal: None,
            input_offset: None,
        }
    }

    /// The recorded duration.
    pub const fn with_duration_ticks(mut self, ticks: u64) -> Self {
        self.duration_ticks = Some(ticks);
        self
    }

    /// The originating packet's ordinal and offset.
    pub const fn with_packet(mut self, ordinal: Option<u64>, offset: Option<u64>) -> Self {
        self.packet_ordinal = ordinal;
        self.input_offset = offset;
        self
    }
}
