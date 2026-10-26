//! Private geometry stage: composited stored samples -> oriented canvas region.
//! This does not establish target color, perform blending or resize for display.
use std::{fmt, sync::Arc};
use zencodec::{
    At, CodecError,
    collection::{Cause, Composition},
    enough::Stop,
    image::{
        Auxiliary, AuxiliaryImageOwner, ImageGeometry, ImageOutput, Meaning, OutputError,
        PackedImageOwner, Rect, Samples, ValueSlice,
    },
    storage::{SharedStorage, StorageBudget, StorageError},
};
use zenpixels::{
    ChannelLayout, ColorContext, Orientation, PixelDescriptor, PixelSlice, PixelSliceMut,
    TransferFunction,
};
use zenpixels_convert::orient::apply_orientation_into;

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Composition(Option<Composition>),
    Placement,
    Interpretation,
    MissingStoredRegion {
        required: Rect,
        available: Rect,
    },
    AuxiliaryGrid {
        index: usize,
        dimensions: (usize, usize),
    },
}
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "canvas projection refused: {self:?}")
    }
}
impl std::error::Error for Refusal {}

/// Immediate input coordinates retained as provenance, not current pixel facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionSource {
    pub geometry: ImageGeometry,
    pub orientation: Orientation,
}

struct Plane {
    data: SharedStorage<u64>,
    width: u32,
    rows: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    context: Option<Arc<ColorContext>>,
}
impl Plane {
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
        let pixels = PixelSlice::new(
            bytemuck::cast_slice(self.data.as_slice()),
            self.width,
            self.rows,
            self.stride,
            self.descriptor,
        )
        .map_err(|error| {
            At::wrap(OutputError::Mapping(zencodec::frame::MapError::Backend(
                Box::new(error),
            )))
        })?;
        Ok(match &self.context {
            Some(context) => pixels.with_color_context(context.clone()),
            None => pixels,
        })
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
                    plane.descriptor.channel_type(),
                )
            })
            .transpose()
    }
}
struct Projected {
    color: Plane,
    extras: Arc<Extras>,
}
impl PackedImageOwner for Projected {
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
        self.color.pixels()
    }
    fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        self.extras.auxiliary(index)
    }
    fn retain_auxiliaries(&self) -> Result<Option<Arc<dyn AuxiliaryImageOwner>>, At<OutputError>> {
        Ok(Some(self.extras.clone()))
    }
}

fn check(stop: &dyn Stop) -> Result<(), Cause> {
    stop.check()
        .map_err(|reason| At::wrap(CodecError::new(None, reason.into())).into())
}

/// Geometric projection only. All mapped planes are validated before payload
/// admission. Cropping is in oriented canvas coordinates; unsupported raw views,
/// placement, component codes or unaligned auxiliary grids refuse explicitly.
/// Source decoding/composition has already happened and retains its own costs.
/// Qualification here covers native JXL auxiliary registration and controlled
/// fixtures. Equal dimensions alone do not prove arbitrary auxiliary origins
/// or phase; the shared API still needs that fact before promoting this helper.
pub fn project(
    image: &ImageOutput,
    region: Option<Rect>,
    storage: &StorageBudget,
    stop: &dyn Stop,
) -> Result<ImageOutput, Cause> {
    check(stop)?;
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
    let (ax, ay) = inverse.forward_map(region.x, region.y, canvas_width, canvas_height);
    let (bx, by) = inverse.forward_map(
        region.x + region.width - 1,
        region.y + region.height - 1,
        canvas_width,
        canvas_height,
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
                if dimensions != (available.width as usize, available.height as usize) {
                    return Err(Refusal::AuxiliaryGrid { index, dimensions }.into());
                }
                // Internal byte-permutation view only: the existing orientation
                // kernel moves whole scalar values and does no color arithmetic.
                // The returned owner exposes Values, never grayscale pixels.
                let descriptor = PixelDescriptor::new(
                    channel,
                    ChannelLayout::Gray,
                    None,
                    TransferFunction::Unknown,
                );
                (
                    PixelSlice::new(
                        values.as_strided_bytes(),
                        available.width,
                        available.height,
                        stride,
                        descriptor,
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
        check(stop)?;
        let descriptor = pixels.descriptor();
        // Deliberately padded output exercises the same strided kernel contract
        // for color and every auxiliary sample width, including 64-bit scalars.
        // PixelSlice requires a whole-pixel stride as well as scalar alignment.
        // Round the pixel count to the next multiple of eight: every row is
        // also u64-aligned without splitting RGB or 16-byte pixels in padding.
        let stride = (region.width as usize)
            .checked_add(8)
            .map(|n| n & !7)
            .and_then(|n| n.checked_mul(descriptor.bytes_per_pixel()))
            .ok_or(StorageError::SizeOverflow)?;
        let bytes = stride
            .checked_mul(region.height as usize)
            .ok_or(StorageError::SizeOverflow)?;
        let mut data = storage.filled(bytes / 8, 0xa5a5_a5a5_a5a5_a5a5u64)?;
        let dst = PixelSliceMut::new(
            bytemuck::cast_slice_mut(data.as_mut_slice()),
            region.width,
            region.height,
            stride,
            descriptor,
        )?;
        let selected = pixels.crop_view(
            required.x - available.x,
            required.y - available.y,
            required.width,
            required.height,
        );
        apply_orientation_into(selected, orientation, dst)?;
        check(stop)?;
        planes.push(Plane {
            data: data.freeze(),
            width: region.width,
            rows: region.height,
            stride,
            descriptor,
            context: pixels.color_context().cloned(),
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
    check(stop)?;
    Ok(output)
}

#[cfg(test)]
#[path = "canvas_projection_tests.rs"]
mod tests;
