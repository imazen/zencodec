use alloc::{boxed::Box, sync::Arc};
use core::fmt;
use zencodec::{
    At, CodecError,
    collection::{Cause, Collection, ImageId, ImageRequest},
    image::{ImageOutput, OutputError, PackedImageOwner, Rect},
    operation::{
        CheckedCollection, CollectionOperationError, OperationPolicy, Outcome, PolicySet,
        resolve_image,
    },
    storage::{SharedStorage, StorageBudget, StorageError},
};
use zenpixels::{
    Cicp, ColorOrigin, ColorSpace, FormatCow, HdrMetadata, PixelFormat, PixelView, Primaries,
    RenderingIntent, SignalRange, Transfer,
};
use zenpixels_convert::{
    CmsBackend, CmsBinding, CmsChoice, CmsQuote, ColorServices, ConvertError, ConvertPolicy,
    RowPlan,
};

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
    /// Required packed pixel format, including layout and alpha interpretation.
    pub required: PixelFormat,
    /// Oriented canvas coordinates. None requests the full oriented canvas.
    pub region: Option<Rect>,
    /// Explicit conversion behavior, including clipping and channel loss.
    pub conversion: ConvertPolicy,
}
impl Default for RenderRequest {
    fn default() -> Self {
        Self {
            target: ColorTarget::Named(Cicp::new(1, 13, 0, true)),
            required: PixelFormat::RGBA8_SRGB,
            region: None,
            conversion: ConvertPolicy::permissive(),
        }
    }
}
impl RenderRequest {
    /// Require a pixel format in the current sample color space.
    pub fn new(required: PixelFormat) -> Self {
        Self {
            target: ColorTarget::Current,
            required,
            region: None,
            conversion: ConvertPolicy::lossless(),
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
    pub fn with_conversion(mut self, conversion: ConvertPolicy) -> Self {
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
fn check(pulse: &dyn zencodec::Pulse) -> Result<(), Cause> {
    pulse
        .check()
        .map_err(|reason| At::wrap(CodecError::new(None, reason.into())).into())
}

struct Color {
    data: SharedStorage<u64>,
    width: u32,
    rows: u32,
    stride: usize,
    format: PixelFormat,
    origin: Option<ColorOrigin>,
    hdr: HdrMetadata,
}
impl PackedImageOwner for Color {
    fn pixels(&self) -> Result<PixelView<'_>, At<OutputError>> {
        let mut pixels = PixelView::new_with_format_cow(
            bytemuck::cast_slice(self.data.as_slice()),
            self.width,
            self.rows,
            self.stride,
            FormatCow::Borrowed(&self.format),
        )
        .map_err(|cause| {
            At::wrap(OutputError::Mapping(zencodec::frame::MapError::Backend(
                Box::new(cause),
            )))
        })?
        .with_hdr(self.hdr);
        if let Some(origin) = &self.origin {
            pixels = pixels.with_origin(origin);
        }
        Ok(pixels)
    }
}

/// RenderRequest configuration and the admitted destination pool can be reused; a
/// fresh RowPlan binds each exact held source. Never cache by item/epoch alone.
pub struct Renderer {
    /// Reusable requested presentation.
    pub request: RenderRequest,
    /// Managed output budget; excludes native codec and CMS workspace.
    pub storage: StorageBudget,
    /// Optional legacy CMS handle. Prefer [`color_services`](Self::color_services) for
    /// named CMS bindings, strict no-ICC mode, and shared tone mapping.
    pub cms: Option<Arc<dyn CmsBackend>>,
    /// Shared color-management and tone-mapping service bundle.
    pub color_services: ColorServices,
}
impl Renderer {
    /// Render to sRGB RGBA8 over the full oriented canvas by default.
    /// Color conversion rounds depth reduction and clips to the display range.
    /// Use an explicit request for higher precision or extended-range output.
    pub fn new(storage: StorageBudget) -> Self {
        Self {
            request: RenderRequest::default(),
            storage,
            cms: None,
            color_services: ColorServices::inherit(),
        }
    }
    /// Replace the reusable output requirements.
    pub fn with_request(mut self, request: RenderRequest) -> Self {
        self.request = request;
        self
    }
    /// Supply a CMS for profile conversions that require one.
    pub fn with_cms(mut self, cms: Arc<dyn CmsBackend>) -> Self {
        let binding = CmsBinding::new("custom.cms", "unknown", cms.clone(), CmsQuote::new());
        self.color_services = self.color_services.with_cms(CmsChoice::Use(binding));
        self.cms = Some(cms);
        self
    }
    /// Supply explicit [`ColorServices`] (named CMS binding, `StrictNoIcc`, and/or tone mapper).
    pub fn with_color_services(mut self, services: ColorServices) -> Self {
        self.cms = services.cms.binding().map(|b| b.backend().clone());
        self.color_services = services;
        self
    }

    /// Render the first top-level image in discovery order. Returns `None` only
    /// when discovery succeeds with no images. Discovery, decoding, rendering
    /// and policy errors propagate; unsupported images are never skipped.
    /// Each call starts discovery anew; use a cursor and `render_selected` to
    /// traverse a collection. This does not choose a thumbnail or child image.
    pub fn render_first<C: Collection, R: OperationPolicy>(
        &self,
        images: &mut CheckedCollection<C, R>,
    ) -> Result<Option<Outcome<ImageOutput>>, At<CollectionOperationError>> {
        let mut cursor = images.images()?;
        images
            .next(&mut cursor)?
            .map(|id| self.render_selected(images, &id))
            .transpose()
    }

    /// Consumer convenience over the shared collection operation. The current
    /// candidate reconstructs the full native selection; its display crop does
    /// not promise proportional native work or bypass native dimension limits.
    pub fn render_selected<C: Collection, R: OperationPolicy>(
        &self,
        images: &mut CheckedCollection<C, R>,
        image: &ImageId,
    ) -> Result<Outcome<ImageOutput>, At<CollectionOperationError>> {
        images.decode_with(image, &ImageRequest::new(), |held, pulse| {
            self.apply(held, Some(pulse))
        })
    }

    /// Render an already decoded image, then publish once through policy.
    /// `None` adds no per-call control; policy and storage checks still apply.
    pub fn render<R: OperationPolicy + ?Sized>(
        &self,
        image: ImageOutput,
        policy: &mut R,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Outcome<ImageOutput>, At<RenderFailure>> {
        self.admit(policy, pulse)?;
        let output = self.apply(image, pulse)?;
        resolve_image(output, policy, PolicySet::new(), pulse)
            .map_err(|e| fail(RenderStage::Publication, e))
    }

    fn admit<R: OperationPolicy + ?Sized>(
        &self,
        policy: &R,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), At<RenderFailure>> {
        check(pulse.unwrap_or(&zencodec::NoPulse)).map_err(|e| fail(RenderStage::ColorPlan, e))?;
        // Managed output storage is not a bound on conversion/CMS preparation.
        policy
            .requirements()
            .unwrap_or_default()
            .check_workspace(None)
            .map_err(|e| fail(RenderStage::ColorPlan, e))
    }

    fn resolve_target_format(&self, source: &PixelFormat) -> Result<PixelFormat, Cause> {
        let alpha = self.request.required.color.alpha();
        let layout = self.request.required.layout;
        let color = match &self.request.target {
            ColorTarget::Current => source.color.clone().with_alpha(alpha),
            ColorTarget::Named(cicp) => {
                if cicp.matrix_coefficients != 0 {
                    return Err(Box::new(At::wrap(ConvertError::UnsupportedRoute)));
                }
                let transfer = Transfer::from_cicp(cicp.transfer_characteristics)
                    .ok_or_else(|| Box::new(At::wrap(ConvertError::UnsupportedRoute)))?;
                let range = if cicp.full_range {
                    SignalRange::Full
                } else {
                    SignalRange::Narrow
                };
                if layout.order.is_gray_family() {
                    ColorSpace::Gray {
                        transfer,
                        alpha,
                        range,
                        white: None,
                    }
                } else {
                    let primaries = Primaries::from_cicp(cicp.color_primaries)
                        .ok_or_else(|| Box::new(At::wrap(ConvertError::UnsupportedRoute)))?;
                    ColorSpace::Rgb {
                        primaries,
                        transfer,
                        alpha,
                        range,
                        white: None,
                    }
                }
            }
            ColorTarget::Icc(profile) => ColorSpace::Icc {
                profile: profile.clone(),
                color_channels: if layout.order.is_gray_family() {
                    1
                } else if layout.order.is_cmyk_family() {
                    4
                } else {
                    3
                },
                alpha,
                intent: RenderingIntent::RelativeColorimetric,
            },
        };
        let format = PixelFormat::new(layout, color);
        format
            .validate()
            .map_err(|_| Box::new(At::wrap(ConvertError::UnsupportedRoute)) as Cause)?;
        Ok(format)
    }

    fn apply(
        &self,
        image: ImageOutput,
        pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<ImageOutput, At<RenderFailure>> {
        let control = pulse.unwrap_or(&zencodec::NoPulse);
        check(control).map_err(|e| fail(RenderStage::ColorPlan, e))?;
        let mut policy = self
            .request
            .conversion
            .clone()
            .with_color_services(&self.color_services);
        if matches!(self.color_services.cms, CmsChoice::Inherit)
            && let Some(cms) = &self.cms
        {
            policy = policy.with_cms(cms.clone());
        }
        // Prepare the actual transform before allocating projected samples.
        let (target_format, plan) = image
            .with_pixels(|pixels| -> Result<(PixelFormat, RowPlan), Cause> {
                let target_format = self.resolve_target_format(pixels.format())?;
                let plan = RowPlan::build(pixels.format(), &target_format, &policy, None)?;
                Ok((target_format, plan))
            })
            .map_err(|e| fail(RenderStage::ColorPlan, e))?
            .map_err(|e| fail(RenderStage::ColorPlan, e))?;
        let projected = crate::geometry::project(&image, self.request.region, &self.storage, pulse)
            .map_err(|e| fail(RenderStage::Geometry, e))?;
        // Do not retain native uncompressed color while preparing/converting a
        // replacement. Independently held caller clones remain charged.
        drop(image);
        let unchanged = projected
            .with_pixels(|pixels| plan.is_identity() && pixels.format() == &target_format)
            .map_err(|e| fail(RenderStage::Color, e))?;
        let output = if unchanged {
            projected
        } else {
            let color = projected
                .with_pixels(|pixels| self.convert(pixels, &target_format, &plan, control))
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
        source: PixelView<'_>,
        target_format: &PixelFormat,
        plan: &RowPlan,
        pulse: &dyn zencodec::Pulse,
    ) -> Result<Color, Cause> {
        let (width, rows) = (source.width(), source.height());
        let bpp = target_format.layout.bytes_per_pixel();
        let row_bytes = (width as usize)
            .checked_mul(bpp)
            .ok_or(StorageError::SizeOverflow)?;
        let stride = (width as usize)
            .checked_add(8)
            .map(|n| n & !7)
            .and_then(|n| n.checked_mul(bpp))
            .ok_or(StorageError::SizeOverflow)?;
        let bytes = stride
            .checked_mul(rows as usize)
            .ok_or(StorageError::SizeOverflow)?;
        check(pulse)?;
        let mut data = self.storage.filled(bytes / 8, 0xa5a5_a5a5_a5a5_a5a5u64)?;
        // Check cancellation between rows. Partially written storage remains
        // private and drops on failure; no resolver observes a partial image.
        for y in 0..rows {
            check(pulse)?;
            let offset = y as usize * stride;
            let dst_row =
                &mut bytemuck::cast_slice_mut(data.as_mut_slice())[offset..offset + row_bytes];
            plan.convert_row_at(y, source.row(y), dst_row, width)?;
        }
        check(pulse)?;
        let hdr = if target_format.can_represent_hdr() {
            source.hdr()
        } else {
            HdrMetadata::default()
        };
        Ok(Color {
            data: data.freeze(),
            width,
            rows,
            stride,
            format: target_format.clone(),
            origin: source.origin().cloned(),
            hdr,
        })
    }
}
