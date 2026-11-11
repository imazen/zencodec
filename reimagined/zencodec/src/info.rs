//! What a file says about itself.
//!
//! [`ImageInfo`] and [`Metadata`] record what the *source* carried. What the
//! decoded samples *are* travels separately, on the frame's
//! [`Raster`](crate::Raster). The two are kept apart on purpose: a decoder
//! that converts on the way out changes the second and never the first.

use crate::format::ImageFormat;
use crate::raster::Layout;
use alloc::sync::Arc;
use zenpixels::hdr::{ContentLightLevel, MasteringDisplay};
use zenpixels::{Cicp, Orientation};

/// A set of metadata kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MetadataKinds(u8);

impl MetadataKinds {
    /// No kinds.
    pub const NONE: Self = Self(0);
    /// An ICC profile.
    pub const ICC: Self = Self(1);
    /// EXIF.
    pub const EXIF: Self = Self(1 << 1);
    /// XMP.
    pub const XMP: Self = Self(1 << 2);
    /// CICP code points.
    pub const CICP: Self = Self(1 << 3);
    /// Content light level and mastering display.
    pub const HDR: Self = Self(1 << 4);
    /// Every kind.
    pub const ALL: Self = Self(0b1_1111);

    /// Both sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The kinds in both.
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The kinds in `self` that are not in `other`.
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether every kind in `other` is in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Metadata blocks, verbatim. Decoders fill it from the file; encoders embed it.
///
/// The codec does not filter, prune or reconcile these: the caller hands an
/// encoder exactly what to write. Parsing and policy live in `zenmeta`.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Metadata {
    /// ICC profile bytes.
    pub icc: Option<Arc<[u8]>>,
    /// EXIF, starting at the TIFF header.
    pub exif: Option<Arc<[u8]>>,
    /// XMP packet.
    pub xmp: Option<Arc<[u8]>>,
    /// CICP code points.
    pub cicp: Option<Cicp>,
    /// Content light level.
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering display.
    pub mastering_display: Option<MasteringDisplay>,
}

impl Metadata {
    /// No metadata.
    pub fn none() -> Self {
        Self::default()
    }

    /// Carry an ICC profile.
    pub fn with_icc(mut self, icc: impl Into<Arc<[u8]>>) -> Self {
        self.icc = Some(icc.into());
        self
    }

    /// Carry EXIF.
    pub fn with_exif(mut self, exif: impl Into<Arc<[u8]>>) -> Self {
        self.exif = Some(exif.into());
        self
    }

    /// Carry XMP.
    pub fn with_xmp(mut self, xmp: impl Into<Arc<[u8]>>) -> Self {
        self.xmp = Some(xmp.into());
        self
    }

    /// Carry CICP code points.
    pub fn with_cicp(mut self, cicp: Cicp) -> Self {
        self.cicp = Some(cicp);
        self
    }

    /// The kinds present.
    pub fn kinds(&self) -> MetadataKinds {
        let mut kinds = MetadataKinds::NONE;
        if self.icc.is_some() {
            kinds = kinds.union(MetadataKinds::ICC);
        }
        if self.exif.is_some() {
            kinds = kinds.union(MetadataKinds::EXIF);
        }
        if self.xmp.is_some() {
            kinds = kinds.union(MetadataKinds::XMP);
        }
        if self.cicp.is_some() {
            kinds = kinds.union(MetadataKinds::CICP);
        }
        if self.content_light_level.is_some() || self.mastering_display.is_some() {
            kinds = kinds.union(MetadataKinds::HDR);
        }
        kinds
    }
}

/// One image or several.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Sequence {
    /// One image.
    #[default]
    Still,
    /// Frames shown in order.
    Animation {
        /// Frame count, when the file states it.
        frames: Option<u32>,
        /// Repeat count; `Some(0)` loops forever.
        loops: Option<u32>,
    },
}

/// What a file's header establishes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ImageInfo {
    /// The format.
    pub format: ImageFormat,
    /// Stored width, before orientation.
    pub width: u32,
    /// Stored height, before orientation.
    pub height: u32,
    /// How the file stores its samples.
    pub native: Layout,
    /// One image or several.
    pub sequence: Sequence,
    /// The orientation the file asks for; decoders do not apply it.
    pub orientation: Orientation,
    /// Whether the file's coding is lossless, when it says.
    pub lossless: Option<bool>,
    /// The metadata found so far.
    pub metadata: Metadata,
    /// The kinds for which `metadata` is final: present-and-read, or known absent.
    pub settled: MetadataKinds,
}

impl ImageInfo {
    /// A still image with no metadata read yet.
    pub fn new(format: ImageFormat, width: u32, height: u32, native: Layout) -> Self {
        Self {
            format,
            width,
            height,
            native,
            sequence: Sequence::Still,
            orientation: Orientation::default(),
            lossless: None,
            metadata: Metadata::default(),
            settled: MetadataKinds::NONE,
        }
    }

    /// State the sequence.
    pub fn with_sequence(mut self, sequence: Sequence) -> Self {
        self.sequence = sequence;
        self
    }

    /// State the orientation.
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// State whether the coding is lossless.
    pub fn with_lossless(mut self, lossless: bool) -> Self {
        self.lossless = Some(lossless);
        self
    }

    /// Record metadata and the kinds it settles.
    pub fn with_metadata(mut self, metadata: Metadata, settled: MetadataKinds) -> Self {
        self.metadata = metadata;
        self.settled = settled;
        self
    }
}
