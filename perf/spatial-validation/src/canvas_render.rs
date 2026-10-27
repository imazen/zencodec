//! Executable candidate for one held-image render operation. Still private:
//! native workspace, planar video and HDR presentation remain separate
//! contracts. Selected decode, color and geometry finish before publication policy.
use std::{fmt, sync::Arc};
use zencodec::{
    At, CodecError,
    collection::{Cause, Collection, ImageId, ImageRequest},
    enough::Stop,
    image::{ImageOutput, OutputError, PackedImageOwner, Rect},
    operation::{
        CheckedCollection, CollectionOperationError, Outcome, PolicyResolver, PolicySet,
        resolve_image,
    },
    storage::{SharedStorage, StorageBudget, StorageError},
};
use zenpixels::{Cicp, ColorContext, ColorOrigin, PixelDescriptor, PixelSlice, PixelSliceMut};
use zenpixels_convert::{ConvertOptions, OutputPlan, OutputProfile, cms::PluggableCms};

/// Current means current samples, not the original file's pre-transform space.
/// Explicit targets never consult opaque source extensions for color authority.
#[derive(Clone)]
pub enum Target {
    Current,
    Named(Cicp),
    Icc(Arc<[u8]>),
}
pub struct Request {
    pub target: Target,
    pub required: PixelDescriptor,
    /// Oriented canvas coordinates. None requests the full oriented canvas.
    pub region: Option<Rect>,
    pub conversion: ConvertOptions,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    ColorPlan,
    Geometry,
    Color,
    Publication,
}
pub struct Failure {
    pub stage: Stage,
    cause: Cause,
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Failure")
            .field("stage", &self.stage)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "selected-image render failed during {:?}", self.stage)
    }
}
impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
fn fail(stage: Stage, cause: impl Into<Cause>) -> At<Failure> {
    At::wrap(Failure {
        stage,
        cause: cause.into(),
    })
}
fn check(stop: &dyn Stop) -> Result<(), Cause> {
    stop.check()
        .map_err(|reason| At::wrap(CodecError::new(None, reason.into())).into())
}

struct Color {
    data: SharedStorage<u64>,
    width: u32,
    rows: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    context: Arc<ColorContext>,
}
impl PackedImageOwner for Color {
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
        PixelSlice::new(
            bytemuck::cast_slice(self.data.as_slice()),
            self.width,
            self.rows,
            self.stride,
            self.descriptor,
        )
        .map(|pixels| pixels.with_color_context(self.context.clone()))
        .map_err(|cause| {
            At::wrap(OutputError::Mapping(zencodec::frame::MapError::Backend(
                Box::new(cause),
            )))
        })
    }
}

/// Request configuration and the admitted destination pool can be reused; a
/// fresh OutputPlan binds each exact held source. Never cache by item/epoch alone.
pub struct Renderer<'a> {
    pub request: Request,
    pub storage: StorageBudget,
    pub cms: Option<&'a dyn PluggableCms>,
}
impl Renderer<'_> {
    /// Consumer convenience over the shared collection operation. The current
    /// candidate reconstructs the full native selection; its display crop does
    /// not promise proportional native work or bypass native dimension limits.
    pub fn render_selected<C: Collection, R: PolicyResolver>(
        &self,
        images: &mut CheckedCollection<C, R>,
        image: &ImageId,
    ) -> Result<Outcome<ImageOutput>, At<CollectionOperationError>> {
        images.decode_with(image, &ImageRequest::new(), |held, stop| {
            self.apply(held, stop)
        })
    }

    pub fn render<R: PolicyResolver + ?Sized>(
        &self,
        image: ImageOutput,
        resolver: &mut R,
        preset: PolicySet,
        stop: &dyn Stop,
    ) -> Result<Outcome<ImageOutput>, At<Failure>> {
        let output = self.apply(image, stop)?;
        resolve_image(output, resolver, preset, stop).map_err(|e| fail(Stage::Publication, e))
    }

    fn apply(&self, image: ImageOutput, stop: &dyn Stop) -> Result<ImageOutput, At<Failure>> {
        check(stop).map_err(|e| fail(Stage::ColorPlan, e))?;
        // Prepare the actual transform before allocating projected samples.
        let mut plan = image
            .with_pixels(|pixels| {
                let target = match &self.request.target {
                    Target::Current => OutputProfile::SameAsOrigin,
                    Target::Named(cicp) => OutputProfile::Named(*cicp),
                    Target::Icc(icc) => OutputProfile::Icc(icc.clone()),
                };
                OutputPlan::new_with_descriptor(
                    pixels.descriptor(),
                    pixels.color_context().cloned(),
                    &ColorOrigin::assumed(),
                    target,
                    self.request.required,
                    &self.request.conversion,
                    self.cms,
                )
            })
            .map_err(|e| fail(Stage::ColorPlan, e))?
            .map_err(|e| fail(Stage::ColorPlan, e))?;
        let projected =
            crate::canvas_projection::project(&image, self.request.region, &self.storage, stop)
                .map_err(|e| fail(Stage::Geometry, e))?;
        // Do not retain native uncompressed color while preparing/converting a
        // replacement. Independently held caller clones remain charged.
        drop(image);
        let unchanged = projected
            .with_pixels(|pixels| {
                plan.borrow_rows(pixels.clone()).map(|borrowed| {
                    borrowed.is_some_and(|view| {
                        view.descriptor() == pixels.descriptor()
                            && view.color_context() == pixels.color_context()
                    })
                })
            })
            .map_err(|e| fail(Stage::Color, e))?
            .map_err(|e| fail(Stage::Color, e))?;
        let output = if unchanged {
            projected
        } else {
            let color = projected
                .with_pixels(|pixels| self.convert(pixels, &mut plan, stop))
                .map_err(|e| fail(Stage::Color, e))?
                .map_err(|e| fail(Stage::Color, e))?;
            projected
                .replace_packed_color(color)
                .map_err(|e| fail(Stage::Color, e))?
        };
        Ok(output)
    }

    fn convert(
        &self,
        source: PixelSlice<'_>,
        plan: &mut OutputPlan,
        stop: &dyn Stop,
    ) -> Result<Color, Cause> {
        let descriptor = plan.output_descriptor();
        let (width, rows) = (source.width(), source.rows());
        let stride = (width as usize)
            .checked_add(8)
            .map(|n| n & !7)
            .and_then(|n| n.checked_mul(descriptor.bytes_per_pixel()))
            .ok_or(StorageError::SizeOverflow)?;
        let bytes = stride
            .checked_mul(rows as usize)
            .ok_or(StorageError::SizeOverflow)?;
        check(stop)?;
        plan.prepare(width)?;
        check(stop)?;
        let mut data = self.storage.filled(bytes / 8, 0xa5a5_a5a5_a5a5_a5a5u64)?;
        let mut context = None;
        // Check cancellation between rows. Partially written storage remains
        // private and drops on failure; no resolver observes a partial image.
        for y in 0..rows {
            check(stop)?;
            let offset = y as usize * stride;
            let destination = PixelSliceMut::new(
                &mut bytemuck::cast_slice_mut(data.as_mut_slice())[offset..offset + stride],
                width,
                1,
                stride,
                descriptor,
            )?;
            let converted = plan.convert_rows(source.crop_view(0, y, width, 1), destination)?;
            context = converted.color_context().cloned();
        }
        check(stop)?;
        Ok(Color {
            data: data.freeze(),
            width,
            rows,
            stride,
            descriptor,
            context: context.ok_or(OutputError::MissingPlane)?,
        })
    }
}

#[path = "canvas_render_tests.rs"]
pub(crate) mod tests;
