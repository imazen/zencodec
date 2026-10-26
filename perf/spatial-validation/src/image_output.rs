//! Private output trial: retain owned native storage behind a borrowed mapping.
//! Packed color, auxiliary values and integer component codes stay distinct.
use super::image_items::ImageGeometry;
use std::{fmt, sync::Arc};
use zencodec::frame::{
    Frame, FrameInterpretation, FrameTiming, MappedPlanes, PlaneRole, PlaneSamples,
    PlaneView as CodePlane, Presentation, SampleEncoding,
};
use zenpixels::{AlphaMode, ChannelType, Orientation, PixelBuffer, PixelSlice};
type Failure = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorContribution {
    /// The retained auxiliary samples have not been composited into color.
    Separate,
    /// The retained auxiliary samples already contributed to output color.
    Applied,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AuxiliaryKind {
    Alpha(AlphaMode),
    Depth,
    /// Source linear RGBA parameters; not transformed to the output profile.
    SpotColor {
        linear_rgba: [f32; 4],
        contribution: ColorContribution,
    },
    SelectionMask,
    Black,
    Cfa {
        channel: u32,
    },
    Thermal,
    Unknown(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourcePrecision {
    Integer { bits: u32, signed: bool },
    Float { bits: u32, exponent_bits: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Auxiliary {
    pub kind: AuxiliaryKind,
    pub name: String,
    /// Source facts, not the representation/geometry of the returned plane.
    pub source_precision: SourcePrecision,
    pub source_dim_shift: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Meaning {
    Color,
    Component(PlaneRole),
    Auxiliary(Auxiliary),
}

#[derive(Debug)]
pub struct Values {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    stride: usize,
    channel: ChannelType,
}
impl Values {
    pub fn as_strided_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn new(
        bytes: Vec<u8>,
        width: u32,
        height: u32,
        stride: usize,
        channel: ChannelType,
    ) -> Result<Self, Failure> {
        let row = (width as usize)
            .checked_mul(channel.byte_size())
            .ok_or(OutputError::Geometry)?;
        let needed = (height as usize)
            .checked_sub(1)
            .and_then(|y| y.checked_mul(stride))
            .and_then(|n| n.checked_add(row))
            .ok_or(OutputError::Geometry)?;
        if width == 0 || channel.byte_size() == 0 || stride < row || bytes.len() < needed {
            return Err(OutputError::Geometry.into());
        }
        Ok(Self {
            bytes,
            width,
            height,
            stride,
            channel,
        })
    }
}

pub enum Samples<'a> {
    Pixels(PixelSlice<'a>),
    Values(&'a Values),
    Codes(CodePlane<'a>),
}
pub enum Row<'a> {
    Bytes(&'a [u8]),
    Words16(&'a [u16]),
}
impl Samples<'_> {
    pub fn dimensions(&self) -> (usize, usize) {
        match self {
            Self::Pixels(p) => (p.width() as usize, p.rows() as usize),
            Self::Values(p) => (p.width as usize, p.height as usize),
            Self::Codes(p) => (p.width(), p.height()),
        }
    }
    pub fn channel_type(&self) -> ChannelType {
        match self {
            Self::Pixels(p) => p.descriptor().channel_type(),
            Self::Values(p) => p.channel,
            Self::Codes(p) => p.encoding().storage(),
        }
    }
    pub fn code_encoding(&self) -> Option<SampleEncoding> {
        match self {
            Self::Codes(p) => Some(p.encoding()),
            _ => None,
        }
    }
    pub fn stride_bytes(&self) -> usize {
        match self {
            Self::Pixels(p) => p.stride(),
            Self::Values(p) => p.stride,
            Self::Codes(p) => p.stride_bytes(),
        }
    }
    pub fn row(&self, y: usize) -> Option<Row<'_>> {
        if y >= self.dimensions().1 {
            return None;
        }
        Some(match self {
            Self::Pixels(p) => Row::Bytes(p.row(y as u32)),
            Self::Values(p) => Row::Bytes(
                &p.bytes[y * p.stride..y * p.stride + p.width as usize * p.channel.byte_size()],
            ),
            Self::Codes(p) => match p.row(y)? {
                PlaneSamples::U8(v) => Row::Bytes(v),
                PlaneSamples::U16(v) => Row::Words16(v),
            },
        })
    }
}

pub struct Plane<'a> {
    pub meaning: &'a Meaning,
    pub samples: Samples<'a>,
}

pub trait MappedImage {
    fn plane_count(&self) -> usize;
    fn plane(&self, index: usize) -> Result<Option<Plane<'_>>, Failure>;
}
pub trait ImageStorage: Send + Sync {
    fn map(&self) -> Result<Box<dyn MappedImage + '_>, Failure>;
}

/// Frozen together with the storage. Callers can inspect or clone these facts,
/// but cannot relabel an existing image by editing the clone.
#[derive(Clone, Debug)]
pub struct ImageInterpretation {
    pub geometry: ImageGeometry,
    /// None means that this adapter does not establish an orientation contract.
    pub orientation: Option<Orientation>,
    /// Retains native component roles, code depth, siting, crop, color and epoch.
    pub signal: Option<Arc<FrameInterpretation>>,
    pub timing: Option<FrameTiming>,
    pub presentation: Option<Presentation>,
    /// Unmodified source facts, not instructions for interpreting transformed
    /// output and not permission to copy opaque metadata into a derivative.
    pub source_details: zencodec::Extensions,
}
impl ImageInterpretation {
    pub fn packed(geometry: ImageGeometry, orientation: Orientation) -> Self {
        Self {
            geometry,
            orientation: Some(orientation),
            signal: None,
            timing: None,
            presentation: None,
            source_details: zencodec::Extensions::new(),
        }
    }
}

#[derive(Clone)]
pub struct ImageOutput {
    storage: Arc<dyn ImageStorage>,
    interpretation: Arc<ImageInterpretation>,
}
impl fmt::Debug for ImageOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageOutput")
            .field("interpretation", &self.interpretation)
            .finish_non_exhaustive()
    }
}
impl ImageOutput {
    pub fn packed(
        pixels: PixelBuffer,
        extras: Vec<(Auxiliary, Values)>,
        interpretation: ImageInterpretation,
    ) -> Result<Self, Failure> {
        let g = interpretation.geometry;
        super::image_items::RegionRequest::Stored(g.region)
            .resolve(g.canvas_width, g.canvas_height)?;
        if (pixels.width(), pixels.height()) != (g.region.width, g.region.height) {
            return Err(OutputError::Geometry.into());
        }
        if interpretation.orientation.is_none()
            || interpretation.signal.is_some()
            || interpretation.presentation.is_some()
        {
            return Err(OutputError::Interpretation.into());
        }
        Ok(Self {
            storage: Arc::new(Packed {
                pixels,
                extras: extras
                    .into_iter()
                    .map(|(role, plane)| (Meaning::Auxiliary(role), plane))
                    .collect(),
            }),
            interpretation: Arc::new(interpretation),
        })
    }
    pub fn from_frame(frame: Frame) -> Self {
        let signal = frame.interpretation().clone();
        let g = signal.geometry;
        let interpretation = ImageInterpretation {
            geometry: ImageGeometry {
                canvas_width: g.coded_width,
                canvas_height: g.coded_height,
                region: super::image_items::Rect {
                    x: 0,
                    y: 0,
                    width: g.coded_width,
                    height: g.coded_height,
                },
            },
            orientation: None,
            timing: Some(frame.timing()),
            presentation: Some(frame.presentation()),
            signal: Some(signal.clone()),
            source_details: zencodec::Extensions::new(),
        };
        Self {
            storage: Arc::new(Components {
                roles: signal
                    .roles
                    .iter()
                    .copied()
                    .map(Meaning::Component)
                    .collect(),
                frame,
            }),
            interpretation: Arc::new(interpretation),
        }
    }
    pub fn interpretation(&self) -> &ImageInterpretation {
        &self.interpretation
    }
    pub fn map(&self) -> Result<Box<dyn MappedImage + '_>, Failure> {
        self.storage.map()
    }
    pub fn with_pixels<T>(&self, consume: impl FnOnce(PixelSlice<'_>) -> T) -> Result<T, Failure> {
        let mapped = self.map()?;
        let plane = mapped.plane(0)?.ok_or(OutputError::MissingPlane)?;
        match plane.samples {
            Samples::Pixels(pixels) if *plane.meaning == Meaning::Color => Ok(consume(pixels)),
            _ => Err(OutputError::NotPackedColor.into()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum OutputError {
    Geometry,
    Interpretation,
    MissingPlane,
    NotPackedColor,
}
impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Geometry => "invalid sample plane geometry",
            Self::Interpretation => "packed samples have an incompatible interpretation",
            Self::MissingPlane => "mapped storage does not supply a declared plane",
            Self::NotPackedColor => "samples are not qualified packed color pixels",
        })
    }
}
impl std::error::Error for OutputError {}

struct Packed {
    pixels: PixelBuffer,
    extras: Vec<(Meaning, Values)>,
}
struct PackedMap<'a>(&'a Packed);
impl ImageStorage for Packed {
    fn map(&self) -> Result<Box<dyn MappedImage + '_>, Failure> {
        Ok(Box::new(PackedMap(self)))
    }
}
impl MappedImage for PackedMap<'_> {
    fn plane_count(&self) -> usize {
        1 + self.0.extras.len()
    }
    fn plane(&self, index: usize) -> Result<Option<Plane<'_>>, Failure> {
        if index == 0 {
            return Ok(Some(Plane {
                meaning: &Meaning::Color,
                samples: Samples::Pixels(self.0.pixels.as_slice()),
            }));
        }
        Ok(self
            .0
            .extras
            .get(index - 1)
            .map(|(meaning, samples)| Plane {
                meaning,
                samples: Samples::Values(samples),
            }))
    }
}

struct Components {
    frame: Frame,
    roles: Vec<Meaning>,
}
struct ComponentMap<'a> {
    native: Box<dyn MappedPlanes + 'a>,
    roles: &'a [Meaning],
}
impl ImageStorage for Components {
    fn map(&self) -> Result<Box<dyn MappedImage + '_>, Failure> {
        let native = self.frame.map()?;
        if native.plane_count() != self.roles.len() {
            return Err(OutputError::MissingPlane.into());
        }
        Ok(Box::new(ComponentMap {
            native,
            roles: &self.roles,
        }))
    }
}
impl MappedImage for ComponentMap<'_> {
    fn plane_count(&self) -> usize {
        self.roles.len()
    }
    fn plane(&self, index: usize) -> Result<Option<Plane<'_>>, Failure> {
        let Some(meaning) = self.roles.get(index) else {
            return Ok(None);
        };
        let samples = self.native.plane(index).ok_or(OutputError::MissingPlane)?;
        Ok(Some(Plane {
            meaning,
            samples: Samples::Codes(samples),
        }))
    }
}
