//! Decoded frames.
//!
//! A [`Frame`] is an owned, refcounted handle: cheap to clone, valid after
//! the decoder that made it resets or drops, usable as encoder input without
//! a copy. It holds
//!
//! - a [`Picture`]: backend-owned storage (rav1d's refcounted picture, an
//!   owned set of planes, a pool slot) reached through a mapping guard;
//! - an [`Arc<FrameInterpretation>`]: geometry, subsampling, siting, sample
//!   encoding and colour claims, immutable, so a queued frame keeps the
//!   interpretation it was decoded under even after the stream's
//!   configuration changes;
//! - [`FrameTiming`] and [`Presentation`] facts for this one presentation.
//!
//! A frame is a presentation occurrence, not a picture: AV1 show-existing
//! and VP8 invisible-altref replays are [`Frame::represent`] of an earlier
//! frame, sharing its picture with other timing.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use zencodec::time::Timestamp;
use zenpixels::{ChannelType, Cicp};

/// Storage word, significant code bits and their placement within the word.
///
/// Mirrors the unpublished `zenpixels::sample::SampleEncoding` (zenpixels
/// PR 76); this crate switches to that type when zenpixels 0.2.17 publishes,
/// which is also why it stays unpublished until then. A 10-bit sample in a
/// 16-bit word is `new(U16, 10, 0)` (LSB-aligned) or `new(U16, 10, 6)`
/// (MSB-aligned); it is never relabelled as a full-range 16-bit value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SampleEncoding {
    storage: ChannelType,
    code_bits: u8,
    bit_shift: u8,
}

/// An impossible [`SampleEncoding`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SampleEncodingError {
    /// Only 8- and 16-bit unsigned words are supported.
    UnsupportedStorage,
    /// Code bits are zero or wider than the word.
    InvalidBitDepth,
    /// The code does not fit the word at that shift.
    InvalidBitShift,
}

impl fmt::Display for SampleEncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedStorage => "sample storage must be an 8- or 16-bit word",
            Self::InvalidBitDepth => "code bits must be between 1 and the word width",
            Self::InvalidBitShift => "code bits do not fit the word at that shift",
        })
    }
}

impl core::error::Error for SampleEncodingError {}

impl SampleEncoding {
    /// Eight-bit codes in 8-bit words.
    pub const U8: Self = Self {
        storage: ChannelType::U8,
        code_bits: 8,
        bit_shift: 0,
    };

    /// `code_bits` significant bits, shifted left by `bit_shift`, in a
    /// `storage` word.
    ///
    /// # Errors
    ///
    /// A [`SampleEncodingError`] for an unsupported word or a code that does
    /// not fit.
    pub const fn new(
        storage: ChannelType,
        code_bits: u8,
        bit_shift: u8,
    ) -> Result<Self, SampleEncodingError> {
        let width = match storage {
            ChannelType::U8 => 8,
            ChannelType::U16 => 16,
            _ => return Err(SampleEncodingError::UnsupportedStorage),
        };
        if code_bits == 0 || code_bits > width {
            return Err(SampleEncodingError::InvalidBitDepth);
        }
        if bit_shift > width - code_bits {
            return Err(SampleEncodingError::InvalidBitShift);
        }
        Ok(Self {
            storage,
            code_bits,
            bit_shift,
        })
    }

    /// The storage word.
    pub const fn storage(self) -> ChannelType {
        self.storage
    }

    /// Significant bits per code.
    pub const fn code_bits(self) -> u8 {
        self.code_bits
    }

    /// Left shift of the code within the word.
    pub const fn bit_shift(self) -> u8 {
        self.bit_shift
    }

    /// Bytes per stored sample.
    pub const fn bytes_per_sample(self) -> usize {
        match self.storage {
            ChannelType::U8 => 1,
            _ => 2,
        }
    }
}

/// Chroma subsampling of a planar picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Subsampling {
    /// One plane, no chroma.
    Mono,
    /// Chroma halved both ways.
    Yuv420,
    /// Chroma halved horizontally.
    Yuv422,
    /// Full-resolution chroma.
    Yuv444,
}

impl Subsampling {
    /// Chroma plane size for a luma size, rounded up; `None` for mono.
    pub const fn chroma_size(self, width: u32, height: u32) -> Option<(u32, u32)> {
        match self {
            Self::Mono => None,
            Self::Yuv420 => Some((width.div_ceil(2), height.div_ceil(2))),
            Self::Yuv422 => Some((width.div_ceil(2), height)),
            Self::Yuv444 => Some((width, height)),
        }
    }
}

/// Where chroma samples sit relative to luma. An axis of its own:
/// `Unknown` is kept, never defaulted to `Center`; a conversion that needs
/// siting refuses `Unknown` on a subsampled axis or takes it from the
/// caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ChromaSiting {
    /// The stream does not say.
    #[default]
    Unknown,
    /// Co-sited with the left luma sample, vertically between rows (MPEG-2
    /// style 4:2:0).
    Left,
    /// Co-sited with the top-left luma sample.
    TopLeft,
    /// Centred between luma samples (JPEG style).
    Center,
}

/// What a plane holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlaneRole {
    /// Y.
    Luma,
    /// Cb (U).
    Cb,
    /// Cr (V).
    Cr,
    /// G (planar RGB, AV1 identity matrix).
    Green,
    /// B.
    Blue,
    /// R.
    Red,
    /// Alpha.
    Alpha,
}

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
    /// Chroma subsampling.
    pub subsampling: Subsampling,
    /// Chroma siting.
    pub siting: ChromaSiting,
    /// Sample storage and code depth.
    pub encoding: SampleEncoding,
    /// What each plane holds, in plane order.
    pub roles: Vec<PlaneRole>,
    /// Colour claims.
    pub color: ColorClaim,
    /// The codec configuration epoch it was decoded under.
    pub config_epoch: u32,
}

impl FrameInterpretation {
    /// An interpretation with no colour claims, at epoch 0.
    pub fn new(
        geometry: FrameGeometry,
        subsampling: Subsampling,
        siting: ChromaSiting,
        encoding: SampleEncoding,
        roles: Vec<PlaneRole>,
    ) -> Self {
        Self {
            geometry,
            subsampling,
            siting,
            encoding,
            roles,
            color: ColorClaim::default(),
            config_epoch: 0,
        }
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

/// Facts about one presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Presentation {
    /// A newly coded, visible key picture (rav1d-safe `is_keyframe`). Not a
    /// random-access promise: see [`RandomAccess`](crate::packet::RandomAccess).
    pub keyframe: bool,
    /// The picture belongs to an earlier presentation (AV1 show-existing).
    pub show_existing: bool,
}

impl Presentation {
    /// A new key picture.
    pub const KEYFRAME: Self = Self {
        keyframe: true,
        show_existing: false,
    };

    /// A replay of an earlier picture.
    pub const SHOW_EXISTING: Self = Self {
        keyframe: false,
        show_existing: true,
    };
}

/// Borrowed samples of one plane, in their storage word.
#[derive(Clone, Copy, Debug)]
pub enum PlaneSamples<'a> {
    /// 8-bit words.
    U8(&'a [u8]),
    /// 16-bit words.
    U16(&'a [u16]),
}

impl PlaneSamples<'_> {
    /// Number of words.
    pub fn len(&self) -> usize {
        match self {
            Self::U8(s) => s.len(),
            Self::U16(s) => s.len(),
        }
    }

    /// Whether there are no words.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The storage word.
    pub fn storage(&self) -> ChannelType {
        match self {
            Self::U8(_) => ChannelType::U8,
            Self::U16(_) => ChannelType::U16,
        }
    }

    fn word(&self, i: usize) -> Option<u16> {
        match self {
            Self::U8(s) => s.get(i).map(|&v| u16::from(v)),
            Self::U16(s) => s.get(i).copied(),
        }
    }
}

/// One plane borrowed from a mapping. The stride is in bytes, as in
/// zenpixels; rows exclude padding. Construction checks the geometry, not
/// the samples.
#[derive(Clone, Copy, Debug)]
pub struct PlaneView<'a> {
    samples: PlaneSamples<'a>,
    width: usize,
    height: usize,
    stride_samples: usize,
    encoding: SampleEncoding,
}

/// A plane whose storage does not fit its geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlaneError {
    /// The word type disagrees with the sample encoding.
    StorageMismatch,
    /// The byte stride is not a whole number of words.
    MisalignedStride,
    /// The stride is shorter than a row.
    ShortStride,
    /// The storage ends before the last sample.
    ShortBuffer,
    /// The geometry overflows addressable storage.
    GeometryOverflow,
}

impl fmt::Display for PlaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StorageMismatch => "plane storage disagrees with its sample encoding",
            Self::MisalignedStride => "plane byte stride does not hold whole samples",
            Self::ShortStride => "plane stride is shorter than its row",
            Self::ShortBuffer => "plane storage does not reach the last sample",
            Self::GeometryOverflow => "plane geometry overflows addressable storage",
        })
    }
}

impl core::error::Error for PlaneError {}

impl<'a> PlaneView<'a> {
    /// A `width` x `height` plane over `samples` with rows `stride_bytes`
    /// apart.
    ///
    /// # Errors
    ///
    /// A [`PlaneError`] when the storage does not fit the geometry.
    pub fn new(
        samples: PlaneSamples<'a>,
        width: usize,
        height: usize,
        stride_bytes: usize,
        encoding: SampleEncoding,
    ) -> Result<Self, PlaneError> {
        if samples.storage() != encoding.storage() {
            return Err(PlaneError::StorageMismatch);
        }
        let bps = encoding.bytes_per_sample();
        if !stride_bytes.is_multiple_of(bps) {
            return Err(PlaneError::MisalignedStride);
        }
        let stride_samples = stride_bytes / bps;
        if width != 0 && height != 0 {
            if stride_samples < width {
                return Err(PlaneError::ShortStride);
            }
            let needed = (height - 1)
                .checked_mul(stride_samples)
                .and_then(|o| o.checked_add(width))
                .ok_or(PlaneError::GeometryOverflow)?;
            if needed > samples.len() {
                return Err(PlaneError::ShortBuffer);
            }
        }
        Ok(Self {
            samples,
            width,
            height,
            stride_samples,
            encoding,
        })
    }

    /// Width in samples.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Distance between rows, in bytes.
    pub fn stride_bytes(&self) -> usize {
        self.stride_samples * self.encoding.bytes_per_sample()
    }

    /// The sample encoding.
    pub fn encoding(&self) -> SampleEncoding {
        self.encoding
    }

    /// Row `y`, without padding.
    pub fn row(&self, y: usize) -> Option<PlaneSamples<'a>> {
        if y >= self.height {
            return None;
        }
        let start = y * self.stride_samples;
        let end = start + self.width;
        Some(match self.samples {
            PlaneSamples::U8(s) => PlaneSamples::U8(&s[start..end]),
            PlaneSamples::U16(s) => PlaneSamples::U16(&s[start..end]),
        })
    }

    /// The code value at (x, y): the stored word shifted and masked per the
    /// encoding.
    pub fn code(&self, x: usize, y: usize) -> Option<u16> {
        if x >= self.width {
            return None;
        }
        let w = self.row(y)?.word(x)?;
        let mask = ((1u32 << self.encoding.code_bits()) - 1) as u16;
        Some((w >> self.encoding.bit_shift()) & mask)
    }
}

/// A picture's planes, mapped. Views borrow the mapping, which borrows the
/// picture, so storage stays put while it is read (rav1d-safe: "a mapped
/// plane owns a borrow guard").
pub trait MappedPlanes {
    /// Number of planes.
    fn plane_count(&self) -> usize;

    /// Plane `index`.
    fn plane(&self, index: usize) -> Option<PlaneView<'_>>;
}

/// A picture could not be mapped.
#[derive(Debug)]
#[non_exhaustive]
pub enum MapError {
    /// The storage is not readable on the CPU.
    Unavailable,
    /// The backend refused, saying why.
    Backend(String),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("picture cannot be mapped"),
            Self::Backend(s) => write!(f, "backend mapping failed: {s}"),
        }
    }
}

impl core::error::Error for MapError {}

/// Backend-owned pixel storage: rav1d's refcounted picture, an owned set of
/// planes, a pool slot.
pub trait Picture: Send + Sync {
    /// Map the planes for reading.
    ///
    /// # Errors
    ///
    /// A [`MapError`] when the storage cannot be read on the CPU.
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError>;
}

/// One presentation of a decoded picture. Cheap to clone; outlives the
/// decoder; may share its picture with another frame.
#[derive(Clone)]
pub struct Frame {
    picture: Arc<dyn Picture>,
    interpretation: Arc<FrameInterpretation>,
    timing: FrameTiming,
    presentation: Presentation,
}

impl Frame {
    /// A frame over `picture`.
    pub fn new(
        picture: Arc<dyn Picture>,
        interpretation: Arc<FrameInterpretation>,
        timing: FrameTiming,
        presentation: Presentation,
    ) -> Self {
        Self {
            picture,
            interpretation,
            timing,
            presentation,
        }
    }

    /// The same picture and interpretation presented again with other
    /// timing (AV1 show-existing, VP8 altref replay).
    pub fn represent(&self, timing: FrameTiming, presentation: Presentation) -> Self {
        Self {
            picture: Arc::clone(&self.picture),
            interpretation: Arc::clone(&self.interpretation),
            timing,
            presentation,
        }
    }

    /// How to read the samples.
    pub fn interpretation(&self) -> &Arc<FrameInterpretation> {
        &self.interpretation
    }

    /// This presentation's timing.
    pub fn timing(&self) -> FrameTiming {
        self.timing
    }

    /// Facts about this presentation.
    pub fn presentation(&self) -> Presentation {
        self.presentation
    }

    /// Map the planes for reading.
    ///
    /// # Errors
    ///
    /// A [`MapError`] when the storage cannot be read on the CPU.
    pub fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        self.picture.map()
    }

    /// Whether both frames present the same picture storage.
    pub fn shares_picture(&self, other: &Frame) -> bool {
        Arc::ptr_eq(&self.picture, &other.picture)
    }

    /// The backing picture.
    pub fn picture(&self) -> &Arc<dyn Picture> {
        &self.picture
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("timing", &self.timing)
            .field("presentation", &self.presentation)
            .field("epoch", &self.interpretation.config_epoch)
            .finish_non_exhaustive()
    }
}

/// Words of one owned plane.
#[derive(Clone, Debug)]
pub enum PlaneData {
    /// 8-bit words.
    U8(Vec<u8>),
    /// 16-bit words.
    U16(Vec<u16>),
}

/// One owned plane.
#[derive(Clone, Debug)]
pub struct OwnedPlane {
    /// The words.
    pub data: PlaneData,
    /// Width in samples.
    pub width: usize,
    /// Height in rows.
    pub height: usize,
    /// Distance between rows, in bytes.
    pub stride_bytes: usize,
}

/// Owned planes in their native words: the [`Picture`] of a software
/// decoder.
#[derive(Clone, Debug)]
pub struct OwnedPlanes {
    planes: Vec<OwnedPlane>,
    encoding: SampleEncoding,
}

impl OwnedPlanes {
    /// Planes sharing one sample encoding.
    ///
    /// # Errors
    ///
    /// A [`PlaneError`] when a plane's storage does not fit its geometry.
    pub fn new(planes: Vec<OwnedPlane>, encoding: SampleEncoding) -> Result<Self, PlaneError> {
        let me = Self { planes, encoding };
        for i in 0..me.planes.len() {
            me.view(i)?;
        }
        Ok(me)
    }

    fn view(&self, i: usize) -> Result<PlaneView<'_>, PlaneError> {
        let p = &self.planes[i];
        let samples = match &p.data {
            PlaneData::U8(v) => PlaneSamples::U8(v),
            PlaneData::U16(v) => PlaneSamples::U16(v),
        };
        PlaneView::new(samples, p.width, p.height, p.stride_bytes, self.encoding)
    }
}

struct OwnedMapped<'a>(&'a OwnedPlanes);

impl MappedPlanes for OwnedMapped<'_> {
    fn plane_count(&self) -> usize {
        self.0.planes.len()
    }

    fn plane(&self, index: usize) -> Option<PlaneView<'_>> {
        if index >= self.0.planes.len() {
            return None;
        }
        self.0.view(index).ok()
    }
}

impl Picture for OwnedPlanes {
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        Ok(Box::new(OwnedMapped(self)))
    }
}
