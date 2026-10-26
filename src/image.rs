//! Owned selected-image output with immutable interpretation and borrowed planes.
//!
//! Packed color uses `zenpixels`; auxiliary values and native component codes
//! remain distinct. Retaining source facts does not authorize metadata copying,
//! display conversion, auxiliary rendering or publication under a privacy policy.
//! The collection/discovery adapter is separate from this output contract.
use crate::At;
use crate::frame::{
    Frame, FrameInterpretation, FrameTiming, MappedPlanes, PlaneRole, PlaneSamples,
    PlaneView as CodePlane, Presentation, SampleEncoding,
};
use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
use core::fmt;
use zenpixels::{AlphaMode, ChannelType, Orientation, PixelBuffer, PixelSlice};
type Failure = At<OutputError>;

mod placement;
pub use placement::{Placement, PlacementError, PlacementOverlap};
mod owner;
pub use owner::PackedImageOwner;

/// A rectangle in the selected image's stored coordinates, before orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Horizontal origin.
    pub x: u32,
    /// Vertical origin.
    pub y: u32,
    /// Width in samples.
    pub width: u32,
    /// Height in rows.
    pub height: u32,
}

/// Position of returned samples within the selected image's stored canvas.
/// This does not claim efficient native region reconstruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageGeometry {
    /// Full selected-image width, before orientation.
    pub canvas_width: u32,
    /// Full selected-image height, before orientation.
    pub canvas_height: u32,
    /// Actual returned region; it must be nonempty and wholly inside the canvas.
    pub region: Rect,
}
impl ImageGeometry {
    /// Validate the active extent without clipping or overflowing coordinates.
    pub fn validate(self) -> Result<(), GeometryError> {
        let r = self.region;
        if r.width == 0
            || r.height == 0
            || r.x
                .checked_add(r.width)
                .is_none_or(|x| x > self.canvas_width)
            || r.y
                .checked_add(r.height)
                .is_none_or(|y| y > self.canvas_height)
        {
            Err(GeometryError { geometry: self })
        } else {
            Ok(())
        }
    }
}

/// The requested stored extent is empty or outside its selected canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryError {
    /// The rejected geometry, retaining the extent and canvas for diagnostics.
    pub geometry: ImageGeometry,
}
impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid stored region {:?} in {}x{} canvas",
            self.geometry.region, self.geometry.canvas_width, self.geometry.canvas_height
        )
    }
}
impl core::error::Error for GeometryError {}

#[track_caller]
fn fail(error: OutputError) -> Failure {
    At::wrap(error).at()
}

/// Whether a retained auxiliary plane has already contributed to returned color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorContribution {
    /// The retained auxiliary samples have not been composited into color.
    Separate,
    /// The retained auxiliary samples already contributed to output color.
    Applied,
}

/// Declared auxiliary role and the interpretation needed for its samples.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AuxiliaryKind {
    /// Coverage or opacity, with the source's alpha association.
    Alpha(AlphaMode),
    /// Depth values; source-specific units remain in source details.
    Depth,
    /// Source linear RGBA parameters; not transformed to the output profile.
    SpotColor {
        /// Original linear red, green, blue and solidity parameters.
        linear_rgba: [f32; 4],
        /// Whether this plane has already affected returned color samples.
        contribution: ColorContribution,
    },
    /// Selection coverage, not display alpha.
    SelectionMask,
    /// Black/key channel; this alone does not establish a CMYK transform.
    Black {
        /// False: zero means no ink. True: the maximum nominal value means
        /// no ink (one for normalized float samples), as in native JPEG XL.
        inverted: bool,
    },
    /// A color-filter-array channel, whose source identity is retained.
    Cfa {
        /// Source channel index, not an invented RGB component mapping.
        channel: u32,
    },
    /// Thermal values; source-specific units remain in source details.
    Thermal,
    /// Unqualified native role code, interpreted only with source details.
    Unknown(u32),
}

/// Original source precision, independent of emitted storage precision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourcePrecision {
    /// Original integer sample precision; emitted storage may be floating point.
    Integer {
        /// Number of source bits.
        bits: u32,
        /// Whether the original samples were signed.
        signed: bool,
    },
    /// Original floating-point representation, independent of emitted type.
    Float {
        /// Total source bits per sample.
        bits: u32,
        /// Source exponent bits.
        exponent_bits: u32,
    },
}

/// Source auxiliary interpretation retained with its emitted values.
#[derive(Clone, Debug, PartialEq)]
pub struct Auxiliary {
    /// Declared role and qualified rendering contribution.
    pub kind: AuxiliaryKind,
    /// Unfiltered source name. May contain private data; not a safe log label.
    pub name: String,
    /// Source facts, not the representation/geometry of the returned plane.
    pub source_precision: SourcePrecision,
    /// Source power-of-two reduction; emitted dimensions are independently mapped.
    pub source_dim_shift: u32,
}

/// Meaning of one mapped plane; auxiliary values are not grayscale color.
#[derive(Clone, Debug, PartialEq)]
pub enum Meaning {
    /// Packed color whose descriptor/context travels with its pixels.
    Color,
    /// Native component role interpreted by the enclosing frame signal.
    Component(PlaneRole),
    /// Auxiliary values with no implied grayscale color interpretation.
    Auxiliary(Auxiliary),
}

/// Owned strided scalar values without implied color or normalization.
#[derive(Debug)]
pub struct Values {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    stride: usize,
    channel: ChannelType,
}

/// Borrowed strided scalar values, without implied color or normalization.
/// The backing owner may be native, ordinary or admitted storage.
#[derive(Clone, Copy, Debug)]
pub struct ValueSlice<'a> {
    bytes: &'a [u8],
    width: u32,
    height: u32,
    stride: usize,
    channel: ChannelType,
}
impl<'a> ValueSlice<'a> {
    /// Check a nonempty scalar plane's active extent. Padding is preserved;
    /// samples remain in their native numerical representation.
    pub fn new(
        bytes: &'a [u8],
        width: u32,
        height: u32,
        stride: usize,
        channel: ChannelType,
    ) -> Result<Self, At<OutputError>> {
        let row = (width as usize)
            .checked_mul(channel.byte_size())
            .ok_or_else(|| fail(OutputError::Geometry))?;
        let needed = (height as usize)
            .checked_sub(1)
            .and_then(|y| y.checked_mul(stride))
            .and_then(|n| n.checked_add(row))
            .ok_or_else(|| fail(OutputError::Geometry))?;
        if width == 0 || channel.byte_size() == 0 || stride < row || bytes.len() < needed {
            return Err(fail(OutputError::Geometry));
        }
        Ok(Self {
            bytes,
            width,
            height,
            stride,
            channel,
        })
    }
    /// Borrow storage including inter-row padding, without copying it.
    pub fn as_strided_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}
impl Values {
    /// Borrow storage including inter-row padding.
    pub fn as_strided_bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Borrow the values with their validated layout.
    pub fn as_slice(&self) -> ValueSlice<'_> {
        ValueSlice {
            bytes: &self.bytes,
            width: self.width,
            height: self.height,
            stride: self.stride,
            channel: self.channel,
        }
    }
    /// Adopt one nonempty scalar plane, checking stride and active extent.
    /// Bytes retain their native numerical representation. No sample conversion
    /// or allocation is performed.
    pub fn new(
        bytes: Vec<u8>,
        width: u32,
        height: u32,
        stride: usize,
        channel: ChannelType,
    ) -> Result<Self, At<OutputError>> {
        ValueSlice::new(&bytes, width, height, stride, channel)?;
        Ok(Self {
            bytes,
            width,
            height,
            stride,
            channel,
        })
    }
}

/// Borrowed samples tied to the mapping and their declared interpretation.
pub enum Samples<'a> {
    /// Packed color with native stride and color metadata.
    Pixels(PixelSlice<'a>),
    /// Strided auxiliary scalar values.
    Values(ValueSlice<'a>),
    /// Native component words with significant code depth and placement.
    Codes(CodePlane<'a>),
}
/// Active row storage, excluding padding. Read its type from the plane.
pub enum Row<'a> {
    /// Bytes of packed pixels or values, or eight-bit component words.
    Bytes(&'a [u8]),
    /// Native-endian sixteen-bit component words; retain their code encoding.
    Words16(&'a [u16]),
}
impl Samples<'_> {
    /// Width and height of this emitted plane.
    pub fn dimensions(&self) -> (usize, usize) {
        match self {
            Self::Pixels(p) => (p.width() as usize, p.rows() as usize),
            Self::Values(p) => (p.width as usize, p.height as usize),
            Self::Codes(p) => (p.width(), p.height()),
        }
    }
    /// Storage sample type, not original precision or code depth.
    pub fn channel_type(&self) -> ChannelType {
        match self {
            Self::Pixels(p) => p.descriptor().channel_type(),
            Self::Values(p) => p.channel,
            Self::Codes(p) => p.encoding().storage(),
        }
    }
    /// Significant code bits for component words; absent for pixels/values.
    pub fn code_encoding(&self) -> Option<SampleEncoding> {
        match self {
            Self::Codes(p) => Some(p.encoding()),
            _ => None,
        }
    }
    /// Distance between row starts in bytes.
    pub fn stride_bytes(&self) -> usize {
        match self {
            Self::Pixels(p) => p.stride(),
            Self::Values(p) => p.stride,
            Self::Codes(p) => p.stride_bytes(),
        }
    }
    /// Active row, or `None` past the plane height.
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

/// A mapped plane whose interpretation and samples share the owner's lifetime.
pub struct Plane<'a> {
    /// Qualified interpretation of this plane.
    pub meaning: &'a Meaning,
    /// Samples borrowed through the native mapping.
    pub samples: Samples<'a>,
}

/// Borrowed mapping guard. Plane views cannot outlive this mapping.
pub trait MappedImage {
    /// Number of declared emitted planes.
    fn plane_count(&self) -> usize;
    /// Borrow a declared plane. `None` means an out-of-range index; a missing
    /// declared native plane is an error.
    fn plane(&self, index: usize) -> Result<Option<Plane<'_>>, At<OutputError>>;
}
pub(crate) struct SampleFacts {
    pub packed: Option<zenpixels::PixelDescriptor>,
    pub planes: usize,
    pub auxiliaries: usize,
    pub applied_spot_colors: usize,
}
trait ImageStorage: Send + Sync {
    fn facts(&self) -> SampleFacts;
    fn map(&self) -> Result<Box<dyn MappedImage + '_>, Failure>;
}

/// Frozen together with the storage. Callers can inspect or clone these facts,
/// but cannot relabel an existing image by editing the clone.
#[derive(Clone, Debug)]
pub struct ImageInterpretation {
    /// Returned stored extent within the selected canvas.
    pub geometry: ImageGeometry,
    /// Placement on a separate presentation canvas, before orientation.
    /// None means this adapter establishes no separate placement contract.
    /// This does not describe blending or permit discarding off-canvas samples.
    pub placement: Option<Placement>,
    /// None means that this adapter does not establish an orientation contract.
    pub orientation: Option<Orientation>,
    /// Retains native component roles, code depth, siting, crop, color and epoch.
    pub signal: Option<Arc<FrameInterpretation>>,
    /// Presentation timing, when established; never inferred from item order.
    pub timing: Option<FrameTiming>,
    /// Native temporal presentation facts, if supplied by a frame adapter.
    pub presentation: Option<Presentation>,
    /// Unmodified source facts, not instructions for interpreting transformed
    /// output and not permission to copy opaque metadata into a derivative.
    pub source_details: crate::Extensions,
}
impl ImageInterpretation {
    /// Describe stored packed pixels without native component-code claims.
    pub fn packed(geometry: ImageGeometry, orientation: Orientation) -> Self {
        Self {
            geometry,
            placement: None,
            orientation: Some(orientation),
            signal: None,
            timing: None,
            presentation: None,
            source_details: crate::Extensions::new(),
        }
    }
}

/// Owned native output. Clones retain the same storage and frozen facts.
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
    pub(crate) fn sample_facts(&self) -> SampleFacts {
        self.storage.facts()
    }
    /// Adapter constructor: adopt packed color plus explicitly interpreted
    /// auxiliary planes. Validate the selected region and compatible facts.
    /// Auxiliary planes retain their own dimensions; no resampling is implied.
    pub fn packed(
        pixels: PixelBuffer,
        extras: Vec<(Auxiliary, Values)>,
        interpretation: ImageInterpretation,
    ) -> Result<Self, At<OutputError>> {
        let g = interpretation.geometry;
        g.validate().map_err(|e| fail(OutputError::Region(e)))?;
        if let Some(placement) = interpretation.placement {
            placement
                .visible(g)
                .map_err(|e| fail(OutputError::Placement(e)))?;
        }
        if (pixels.width(), pixels.height()) != (g.region.width, g.region.height) {
            return Err(fail(OutputError::Geometry));
        }
        if interpretation.orientation.is_none()
            || interpretation.signal.is_some()
            || interpretation.presentation.is_some()
        {
            return Err(fail(OutputError::Interpretation));
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
    /// Retain an existing native component frame without copying samples.
    /// The frame adapter remains responsible for its interpretation claims;
    /// mapping checks the active region, role count, sample encoding and plane
    /// geometry. It does not parse or certify color profiles.
    pub fn from_frame(frame: Frame) -> Self {
        let signal = frame.interpretation().clone();
        let g = signal.geometry;
        let interpretation = ImageInterpretation {
            geometry: ImageGeometry {
                canvas_width: g.coded_width,
                canvas_height: g.coded_height,
                region: Rect {
                    x: 0,
                    y: 0,
                    width: g.coded_width,
                    height: g.coded_height,
                },
            },
            orientation: None,
            placement: None,
            timing: Some(frame.timing()),
            presentation: Some(frame.presentation()),
            signal: Some(signal.clone()),
            source_details: crate::Extensions::new(),
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
    /// Frozen facts belonging to these samples, not a mutable relabeling API.
    pub fn interpretation(&self) -> &ImageInterpretation {
        &self.interpretation
    }
    /// Borrow the output storage. Native mapping failures preserve their cause.
    pub fn map(&self) -> Result<Box<dyn MappedImage + '_>, At<OutputError>> {
        self.storage.map()
    }
    /// Packed-color convenience. Component codes require explicit conversion
    /// and refuse this path. The borrowed pixels cannot outlive their mapping.
    pub fn with_pixels<T>(
        &self,
        consume: impl FnOnce(PixelSlice<'_>) -> T,
    ) -> Result<T, At<OutputError>> {
        let mapped = self.map()?;
        let plane = mapped
            .plane(0)?
            .ok_or_else(|| fail(OutputError::MissingPlane))?;
        match plane.samples {
            Samples::Pixels(pixels) if *plane.meaning == Meaning::Color => Ok(consume(pixels)),
            _ => Err(fail(OutputError::NotPackedColor)),
        }
    }
}

/// Output storage and declared interpretation cannot satisfy the operation.
/// Backend mapping causes remain available through `Error::source`.
/// Default formatting does not print arbitrary backend payloads.
#[non_exhaustive]
pub enum OutputError {
    /// The returned region is empty or outside the selected canvas.
    Region(GeometryError),
    /// The native mapping failed, retaining its original cause.
    Mapping(crate::frame::MapError),
    /// The active sample extent does not fit its storage or declared region.
    Geometry,
    /// Packed pixels were given incompatible component or orientation facts.
    Interpretation,
    /// A native mapping omitted a declared plane.
    MissingPlane,
    /// A mapped component disagrees with the held interpretation.
    Component {
        /// Plane index in the held interpretation.
        index: usize,
        /// Width and height required by the role and subsampling.
        expected_dimensions: (usize, usize),
        /// Actual dimensions supplied by the native mapping.
        actual_dimensions: (usize, usize),
        /// Code encoding captured with the frame.
        expected_encoding: SampleEncoding,
        /// Code encoding supplied by the mapping.
        actual_encoding: SampleEncoding,
    },
    /// The packed-color convenience was requested for component codes.
    NotPackedColor,
    /// Separate presentation placement is invalid for these local samples.
    Placement(PlacementError),
}
impl fmt::Debug for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Region(e) => f.debug_tuple("Region").field(e).finish(),
            Self::Mapping(_) => f
                .debug_tuple("Mapping")
                .field(&"backend error; inspect source")
                .finish(),
            Self::Geometry => f.write_str("Geometry"),
            Self::Interpretation => f.write_str("Interpretation"),
            Self::MissingPlane => f.write_str("MissingPlane"),
            Self::Component {
                index,
                expected_dimensions,
                actual_dimensions,
                expected_encoding,
                actual_encoding,
            } => f
                .debug_struct("Component")
                .field("index", index)
                .field("expected_dimensions", expected_dimensions)
                .field("actual_dimensions", actual_dimensions)
                .field("expected_encoding", expected_encoding)
                .field("actual_encoding", actual_encoding)
                .finish(),
            Self::NotPackedColor => f.write_str("NotPackedColor"),
            Self::Placement(e) => f.debug_tuple("Placement").field(e).finish(),
        }
    }
}
impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Region(e) => return e.fmt(f),
            Self::Mapping(_) => return f.write_str("native image mapping failed"),
            Self::Placement(e) => return e.fmt(f),
            _ => {}
        }
        f.write_str(match self {
            Self::Region(_) | Self::Mapping(_) | Self::Placement(_) => unreachable!(),
            Self::Geometry => "invalid sample plane geometry",
            Self::Interpretation => "packed samples have an incompatible interpretation",
            Self::MissingPlane => "mapped storage does not supply a declared plane",
            Self::Component { .. } => "mapped component disagrees with its held interpretation",
            Self::NotPackedColor => "samples are not qualified packed color pixels",
        })
    }
}
impl core::error::Error for OutputError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Region(error) => Some(error),
            Self::Mapping(error) => Some(error),
            Self::Placement(error) => Some(error),
            _ => None,
        }
    }
}

struct Packed {
    pixels: PixelBuffer,
    extras: Vec<(Meaning, Values)>,
}
struct PackedMap<'a>(&'a Packed);
impl ImageStorage for Packed {
    fn facts(&self) -> SampleFacts {
        SampleFacts {
            packed: Some(self.pixels.descriptor()),
            planes: 1 + self.extras.len(),
            auxiliaries: self.extras.len(),
            applied_spot_colors: self
                .extras
                .iter()
                .filter(|(meaning, _)| {
                    matches!(
                        meaning,
                        Meaning::Auxiliary(Auxiliary {
                            kind: AuxiliaryKind::SpotColor {
                                contribution: ColorContribution::Applied,
                                ..
                            },
                            ..
                        })
                    )
                })
                .count(),
        }
    }
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
                samples: Samples::Values(samples.as_slice()),
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
    signal: &'a FrameInterpretation,
}
impl ImageStorage for Components {
    fn facts(&self) -> SampleFacts {
        SampleFacts {
            packed: None,
            planes: self.roles.len(),
            auxiliaries: 0,
            applied_spot_colors: 0,
        }
    }
    fn map(&self) -> Result<Box<dyn MappedImage + '_>, Failure> {
        let signal = self.frame.interpretation();
        let g = signal.geometry;
        ImageGeometry {
            canvas_width: g.coded_width,
            canvas_height: g.coded_height,
            region: Rect {
                x: g.crop.x,
                y: g.crop.y,
                width: g.crop.width,
                height: g.crop.height,
            },
        }
        .validate()
        .map_err(|e| fail(OutputError::Region(e)))?;
        if signal.roles.is_empty()
            || signal
                .roles
                .iter()
                .enumerate()
                .any(|(i, role)| signal.roles[..i].contains(role))
            || g.render.is_some_and(|(w, h)| w == 0 || h == 0)
        {
            return Err(fail(OutputError::Interpretation));
        }
        let native = self
            .frame
            .map()
            .map_err(|e| fail(OutputError::Mapping(e)))?;
        if native.plane_count() != self.roles.len() {
            return Err(fail(OutputError::MissingPlane));
        }
        Ok(Box::new(ComponentMap {
            native,
            roles: &self.roles,
            signal,
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
        let samples = self
            .native
            .plane(index)
            .ok_or_else(|| fail(OutputError::MissingPlane))?;
        let g = self.signal.geometry;
        let role = self.signal.roles[index];
        let (w, h) = match role {
            PlaneRole::Cb | PlaneRole::Cr => self
                .signal
                .subsampling
                .chroma_size(g.coded_width, g.coded_height)
                .ok_or_else(|| fail(OutputError::Interpretation))?,
            _ => (g.coded_width, g.coded_height),
        };
        let expected_dimensions = (w as usize, h as usize);
        let actual_dimensions = (samples.width(), samples.height());
        if samples.encoding() != self.signal.encoding || actual_dimensions != expected_dimensions {
            return Err(fail(OutputError::Component {
                index,
                expected_dimensions,
                actual_dimensions,
                expected_encoding: self.signal.encoding,
                actual_encoding: samples.encoding(),
            }));
        }
        Ok(Some(Plane {
            meaning,
            samples: Samples::Codes(samples),
        }))
    }
}
