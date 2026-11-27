//! Geometry stage: composited stored samples -> oriented canvas region.
//! This does not establish target color, perform blending or resize for display.
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::fmt;
use zencodec::{
    At, CodecError,
    collection::{Cause, Composition},
    image::{
        Auxiliary, AuxiliaryGrid, AuxiliaryImageOwner, ImageGeometry, ImageOutput, Meaning,
        OutputError, PackedImageOwner, Rect, Samples, ValueSlice,
    },
    storage::{SharedStorage, StorageBudget, StorageError},
};
use zenpixels::{
    ChannelOrder, ColorOrigin, FormatCow, HdrMetadata, Orientation, PixelFormat, PixelRect,
    PixelView, PixelViewMut, SampleLayout,
};
use zenpixels_convert::apply_orientation_into;

#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
/// Required geometry or interpretation could not be established.
pub enum Refusal {
    /// Input is not explicitly composited.
    Composition(Option<Composition>),
    /// Input still requires placement or composition.
    Placement,
    /// Required orientation or packed interpretation is unknown or unsupported.
    Interpretation,
    /// The decoded source does not cover all requested samples.
    MissingStoredRegion {
        /// Required source-coordinate rectangle.
        required: Rect,
        /// Actually stored source rectangle.
        available: Rect,
    },
    /// An auxiliary plane lacks qualified alignment with color samples.
    AuxiliaryGrid {
        /// Auxiliary plane index.
        index: usize,
        /// Actual auxiliary dimensions.
        dimensions: (usize, usize),
        /// Adapter-provided alignment evidence.
        registration: AuxiliaryGrid,
    },
}
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "canvas projection refused: {self:?}")
    }
}
impl core::error::Error for Refusal {}

/// Immediate input coordinates retained as provenance, not current pixel facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionSource {
    /// Geometry before projection.
    pub geometry: ImageGeometry,
    /// Orientation consumed by projection.
    pub orientation: Orientation,
}

struct Plane {
    data: SharedStorage<u64>,
    width: u32,
    rows: u32,
    stride: usize,
    format: PixelFormat,
    origin: Option<ColorOrigin>,
    hdr: HdrMetadata,
}
impl Plane {
    fn pixels(&self) -> Result<PixelView<'_>, At<OutputError>> {
        let mut pixels = PixelView::new_with_format_cow(
            bytemuck::cast_slice(self.data.as_slice()),
            self.width,
            self.rows,
            self.stride,
            FormatCow::Borrowed(&self.format),
        )
        .map_err(|error| {
            At::wrap(OutputError::Mapping(zencodec::frame::MapError::Backend(
                Box::new(error),
            )))
        })?
        .with_hdr(self.hdr);
        if let Some(origin) = &self.origin {
            pixels = pixels.with_origin(origin);
        }
        Ok(pixels)
    }
}
struct Extras(Vec<Plane>);
impl AuxiliaryImageOwner for Extras {
    fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        self.0
            .get(index)
            .map(|plane| {
                ValueSlice::new(
                    bytemuck::cast_slice(plane.data.as_slice()),
                    plane.width,
                    plane.rows,
                    plane.stride,
                    plane.format.layout.sample,
                )
                .map(|values| values.with_grid(AuxiliaryGrid::ColorSamples))
            })
            .transpose()
    }
}
struct Projected {
    color: Plane,
    extras: Arc<Extras>,
}
impl PackedImageOwner for Projected {
    fn pixels(&self) -> Result<PixelView<'_>, At<OutputError>> {
        self.color.pixels()
    }
    fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        self.extras.auxiliary(index)
    }
    fn retain_auxiliaries(&self) -> Result<Option<Arc<dyn AuxiliaryImageOwner>>, At<OutputError>> {
        Ok(Some(self.extras.clone()))
    }
}

fn check(pulse: &dyn zencodec::Pulse) -> Result<(), Cause> {
    pulse
        .check()
        .map_err(|reason| At::wrap(CodecError::new(None, reason.into())).into())
}

/// Geometric projection only. All mapped planes are validated before payload
/// admission. Cropping is in oriented canvas coordinates; unsupported raw views,
/// placement, component codes or unaligned auxiliary grids refuse explicitly.
/// Source decoding/composition has already happened and retains its own costs.
/// Auxiliary registration must be explicitly established by its adapter;
/// dimensions alone do not authorize projection. Other grids require their own
/// geometry representation and resampling mechanism before they can be offered.
/// `None` adds no per-call control; geometry and storage checks still apply.
pub fn project(
    image: &ImageOutput,
    region: Option<Rect>,
    storage: &StorageBudget,
    pulse: Option<&dyn zencodec::Pulse>,
) -> Result<ImageOutput, Cause> {
    let pulse = pulse.unwrap_or(&zencodec::NoPulse);
    check(pulse)?;
    let source = image.interpretation();
    if source.composition != Some(Composition::Composited) {
        return Err(Refusal::Composition(source.composition).into());
    }
    if source.placement.is_some() {
        return Err(Refusal::Placement.into());
    }
    if source.signal.is_some() || source.presentation.is_some() {
        return Err(Refusal::Interpretation.into());
    }
    let orientation = source.orientation.ok_or(Refusal::Interpretation)?;
    source.geometry.validate()?;
    let (canvas_width, canvas_height) =
        orientation.output_dimensions(source.geometry.canvas_width, source.geometry.canvas_height);
    let region = region.unwrap_or(Rect {
        x: 0,
        y: 0,
        width: canvas_width,
        height: canvas_height,
    });
    let geometry = ImageGeometry {
        canvas_width,
        canvas_height,
        region,
    };
    geometry.validate()?;
    // Inverse-map opposite corners of a validated rectangle. D4 transforms
    // preserve rectangular support; this borrows only the needed stored rows.
    let inverse = orientation.inverse();
    let (ax, ay) =
        inverse.map_pixel_coordinates((region.x, region.y), (canvas_width, canvas_height));
    let (bx, by) = inverse.map_pixel_coordinates(
        (region.x + region.width - 1, region.y + region.height - 1),
        (canvas_width, canvas_height),
    );
    let required = Rect {
        x: ax.min(bx),
        y: ay.min(by),
        width: ax.abs_diff(bx) + 1,
        height: ay.abs_diff(by) + 1,
    };
    let available = source.geometry.region;
    if required.x < available.x
        || required.y < available.y
        || required.x + required.width > available.x + available.width
        || required.y + required.height > available.y + available.height
    {
        return Err(Refusal::MissingStoredRegion {
            required,
            available,
        }
        .into());
    }
    let mapped = image.map()?;
    let mut views = Vec::new();
    views.try_reserve_exact(mapped.plane_count())?;
    for index in 0..mapped.plane_count() {
        let plane = mapped.plane(index)?.ok_or(OutputError::MissingPlane)?;
        let dimensions = plane.samples.dimensions();
        let channel = plane.samples.channel_type();
        let stride = plane.samples.stride_bytes();
        let (pixels, auxiliary) = match (index, plane.meaning, plane.samples) {
            (0, Meaning::Color, Samples::Pixels(pixels)) => (pixels, None),
            (_, Meaning::Auxiliary(role), Samples::Values(values)) if index > 0 => {
                if values.grid() != AuxiliaryGrid::ColorSamples
                    || dimensions != (available.width as usize, available.height as usize)
                {
                    return Err(Refusal::AuxiliaryGrid {
                        index,
                        dimensions,
                        registration: values.grid(),
                    }
                    .into());
                }
                // Internal byte-permutation view only: the existing orientation
                // kernel moves whole scalar values and does no color arithmetic.
                // The returned owner exposes Values, never grayscale pixels.
                let format =
                    zencodec::helpers::codec_spec(SampleLayout::new(channel, ChannelOrder::Gray));
                (
                    PixelView::new_with_format_cow(
                        values.as_strided_bytes(),
                        available.width,
                        available.height,
                        stride,
                        FormatCow::from_owned(format),
                    )?,
                    Some(role.clone()),
                )
            }
            _ => return Err(Refusal::Interpretation.into()),
        };
        views.push((pixels, auxiliary));
    }
    if views.is_empty() {
        return Err(OutputError::MissingPlane.into());
    }
    let mut planes = Vec::new();
    let mut roles: Vec<Auxiliary> = Vec::new();
    planes.try_reserve_exact(views.len())?;
    roles.try_reserve_exact(views.len() - 1)?;
    for (pixels, auxiliary) in views {
        check(pulse)?;
        let format = pixels.format().clone();
        // Deliberately padded output exercises the same strided kernel contract
        // for color and every auxiliary sample width, including 64-bit scalars.
        // PixelViewMut requires a whole-pixel stride as well as scalar alignment.
        // Round the pixel count to the next multiple of eight: every row is
        // also u64-aligned without splitting RGB or 16-byte pixels in padding.
        let stride = (region.width as usize)
            .checked_add(8)
            .map(|n| n & !7)
            .and_then(|n| n.checked_mul(format.layout.bytes_per_pixel()))
            .ok_or(StorageError::SizeOverflow)?;
        let bytes = stride
            .checked_mul(region.height as usize)
            .ok_or(StorageError::SizeOverflow)?;
        let mut data = storage.filled(bytes / 8, 0xa5a5_a5a5_a5a5_a5a5u64)?;
        let mut dst = PixelViewMut::new_with_format_cow(
            bytemuck::cast_slice_mut(data.as_mut_slice()),
            region.width,
            region.height,
            stride,
            FormatCow::Borrowed(&format),
        )?;
        let selected = pixels.crop(PixelRect {
            x: required.x - available.x,
            y: required.y - available.y,
            width: required.width,
            height: required.height,
        })?;
        apply_orientation_into(&selected, orientation, &mut dst)?;
        check(pulse)?;
        planes.push(Plane {
            data: data.freeze(),
            width: region.width,
            rows: region.height,
            stride,
            format,
            origin: pixels.origin().cloned(),
            hdr: pixels.hdr(),
        });
        if let Some(role) = auxiliary {
            roles.push(role);
        }
    }
    let color = planes.remove(0);
    let mut interpretation = source.clone();
    interpretation.geometry = geometry;
    interpretation.orientation = Some(Orientation::Identity);
    interpretation.source_details.insert(ProjectionSource {
        geometry: source.geometry,
        orientation,
    });
    let output = ImageOutput::from_packed_owner(
        Projected {
            color,
            extras: Arc::new(Extras(planes)),
        },
        roles,
        interpretation,
    )?;
    check(pulse)?;
    Ok(output)
}
