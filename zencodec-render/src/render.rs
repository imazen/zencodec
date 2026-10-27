use alloc::{boxed::Box, sync::Arc};
use core::fmt;
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
#[non_exhaustive]
pub enum ColorTarget {
    /// Preserve the current sample color space, not original-file provenance.
    Current,
    /// Convert to this named color space.
    Named(Cicp),
    /// Convert to this ICC profile, requiring a suitable supplied CMS.
    Icc(Arc<[u8]>),
}
/// Required display output. No unsupported setting is silently ignored.
pub struct RenderRequest {
    /// Required output color space.
    pub target: ColorTarget,
    /// Required packed pixel descriptor, including alpha interpretation.
    pub required: PixelDescriptor,
    /// Oriented canvas coordinates. None requests the full oriented canvas.
    pub region: Option<Rect>,
    /// Explicit conversion behavior, including clipping and channel loss.
    pub conversion: ConvertOptions,
}
impl Default for RenderRequest {
    fn default() -> Self {
        Self {
            target: ColorTarget::Named(Cicp::new(1, 13, 0, true)),
            required: PixelDescriptor::RGBA8_SRGB,
            region: None,
            conversion: ConvertOptions::permissive(),
        }
    }
}
impl RenderRequest {
    /// Require a descriptor in the current sample color space.
    pub fn new(required: PixelDescriptor) -> Self {
        Self {
            target: ColorTarget::Current,
            required,
            region: None,
            conversion: ConvertOptions::retain_channels(),
        }
    }
    /// Set an explicit output color space.
    pub fn with_target(mut self, target: ColorTarget) -> Self {
        self.target = target;
        self
    }
    /// Select an oriented-canvas rectangle; None means the complete canvas.
    pub fn with_region(mut self, region: Option<Rect>) -> Self {
        self.region = region;
        self
    }
    /// Set explicit conversion behavior.
    pub fn with_conversion(mut self, conversion: ConvertOptions) -> Self {
        self.conversion = conversion;
        self
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// Stage at which rendering failed.
pub enum RenderStage {
    /// Prepare a transform for the held source.
    ColorPlan,
    /// Apply orientation and display-coordinate crop.
    Geometry,
    /// Execute the prepared color conversion.
    Color,
    /// Ask the operation policy before returning output.
    Publication,
}
/// Rendering failure retaining the concrete original cause.
pub struct RenderFailure {
    /// Stage that failed.
    pub stage: RenderStage,
    cause: Cause,
}
impl fmt::Debug for RenderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderFailure")
            .field("stage", &self.stage)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for RenderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "selected-image render failed during {:?}", self.stage)
    }
}
impl core::error::Error for RenderFailure {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
fn fail(stage: RenderStage, cause: impl Into<Cause>) -> At<RenderFailure> {
    At::wrap(RenderFailure {
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

/// RenderRequest configuration and the admitted destination pool can be reused; a
/// fresh OutputPlan binds each exact held source. Never cache by item/epoch alone.
pub struct Renderer<'a> {
    /// Reusable requested presentation.
    pub request: RenderRequest,
    /// Managed output budget; excludes native codec and CMS workspace.
    pub storage: StorageBudget,
    /// Optional color-management implementation. Unsupported conversions fail.
    pub cms: Option<&'a dyn PluggableCms>,
}
impl<'a> Renderer<'a> {
    /// Render to sRGB RGBA8 over the full oriented canvas by default.
    /// Color conversion rounds depth reduction and clips to the display range.
    /// Use an explicit request for higher precision or extended-range output.
    pub fn new(storage: StorageBudget) -> Self {
        Self {
            request: RenderRequest::default(),
            storage,
            cms: None,
        }
    }
    /// Replace the reusable output requirements.
    pub fn with_request(mut self, request: RenderRequest) -> Self {
        self.request = request;
        self
    }
    /// Supply a CMS for profile conversions that require one.
    pub fn with_cms(mut self, cms: &'a dyn PluggableCms) -> Self {
        self.cms = Some(cms);
        self
    }

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

    /// Render an already decoded image, then publish once through policy.
    pub fn render<R: PolicyResolver + ?Sized>(
        &self,
        image: ImageOutput,
        resolver: &mut R,
        preset: PolicySet,
        stop: &dyn Stop,
    ) -> Result<Outcome<ImageOutput>, At<RenderFailure>> {
        let output = self.apply(image, stop)?;
        resolve_image(output, resolver, preset, stop).map_err(|e| fail(RenderStage::Publication, e))
    }

    fn apply(&self, image: ImageOutput, stop: &dyn Stop) -> Result<ImageOutput, At<RenderFailure>> {
        check(stop).map_err(|e| fail(RenderStage::ColorPlan, e))?;
        // Prepare the actual transform before allocating projected samples.
        let mut plan = image
            .with_pixels(|pixels| {
                let target = match &self.request.target {
                    ColorTarget::Current => OutputProfile::SameAsOrigin,
                    ColorTarget::Named(cicp) => OutputProfile::Named(*cicp),
                    ColorTarget::Icc(icc) => OutputProfile::Icc(icc.clone()),
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
            .map_err(|e| fail(RenderStage::ColorPlan, e))?
            .map_err(|e| fail(RenderStage::ColorPlan, e))?;
        let projected = crate::geometry::project(&image, self.request.region, &self.storage, stop)
            .map_err(|e| fail(RenderStage::Geometry, e))?;
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
            .map_err(|e| fail(RenderStage::Color, e))?
            .map_err(|e| fail(RenderStage::Color, e))?;
        let output = if unchanged {
            projected
        } else {
            let color = projected
                .with_pixels(|pixels| self.convert(pixels, &mut plan, stop))
                .map_err(|e| fail(RenderStage::Color, e))?
                .map_err(|e| fail(RenderStage::Color, e))?;
            projected
                .replace_packed_color(color)
                .map_err(|e| fail(RenderStage::Color, e))?
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
