//! Shared media frame rendering ([`VideoRenderPlan`]) and capture ([`VideoCapturePlan`])
//! bridging `zencodec_media::frame::Frame` and `zenpixels_convert`.
//!
//! - [`VideoRenderPlan`] implements [`zencodec_media::adapt::RenderFrame`] over
//!   [`reconstruct_planar_into`] / [`reconstruct_planar_f32`] + [`RowPlan`] + [`ColorServices`].
//!   It honors crop origins, chroma siting, bit-packing, and resolved CICP claims, and
//!   refuses missing color claims or unresolved `ChromaSiting::Unknown` unless explicitly
//!   configured via [`PlanarReconstructionOptions::with_unknown_siting_fallback`].
//! - [`VideoCapturePlan`] implements [`zencodec_media::adapt::CaptureFrame`] over
//!   [`PlanarCapturePlan`] + [`ColorServices`] + [`StorageBudget`], accounting every
//!   captured plane allocation against the caller's budget.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::num::NonZeroUsize;

use zencodec::encode::BoxedError;
use zencodec::plane::{
    AnyPlane, FloatEncoding, PlanarLayout, Plane, PlaneEncoding, PlaneRole as MediaPlaneRole,
    PlaneSpec,
};
use zencodec::sample::{ChromaSiting, ChromaSubsampling};
use zencodec::storage::{OwnedStorage, StorageBudget};
use zencodec_media::adapt::{CaptureFrame, RenderFrame};
use zencodec_media::frame::{
    ColorClaim, Frame, FrameGeometry, FrameInterpretation, FrameTiming, OwnedPlane, OwnedPlanes,
    Presentation,
};
use zencodec_media::session::CodecConfig;
use zencodec_media::track::TrackInfo;
use zenpixels::{
    AlphaMode, BitPacking, Cicp, ColorSpace, DiffuseWhite, FloatFacts, PixelBuf, PixelFormat,
    PixelRect, PixelView, PixelViewMut, PlanarView, PlanarViewMut, PlaneFormat,
    PlaneRole as PixelPlaneRole, PlaneView, PlaneViewMut, Primaries, SampleLayout, SampleType,
    SignalRange, Transfer,
};
use zenpixels_convert::{
    CaptureScratch, ColorServices, ConvertError, ConvertPolicy, DepthPolicy, PlanarCaptureOptions,
    PlanarCapturePlan, PlanarCaptureSpec, PlanarReconstructionOptions, RangePolicy, RowPlan,
    reconstruct_planar_f32, reconstruct_planar_into,
};

/// Failure returned by [`VideoRenderPlan`] or [`VideoCapturePlan`].
#[derive(Debug)]
#[non_exhaustive]
pub enum MediaBridgeError {
    /// Frame or track lacks a resolved CICP color claim (sRGB is never silently guessed).
    MissingColorClaim,
    /// Configured `CodecConfig::video.color` conflicts with the capture target CICP signal.
    ColorClaimConflict {
        /// Target sample CICP signal configured on [`VideoCapturePlan`].
        target: Cicp,
        /// Conflicting container/track CICP signal on [`CodecConfig`].
        config: Cicp,
    },
    /// Frame, track, or canvas dimensions do not match.
    GeometryMismatch,
    /// Planar layout, sample encoding, or color space is outside the supported route.
    UnsupportedLayout(&'static str),
    /// Color or planar conversion failed.
    Convert(ConvertError),
    /// Underlying storage or plane error.
    Backend(BoxedError),
}

impl fmt::Display for MediaBridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingColorClaim => f.write_str("video frame has no resolved CICP color claim"),
            Self::ColorClaimConflict { target, config } => write!(
                f,
                "video capture target CICP {target:?} conflicts with codec config color {config:?}"
            ),
            Self::GeometryMismatch => f.write_str("video frame and canvas geometry disagree"),
            Self::UnsupportedLayout(msg) => write!(f, "unsupported planar layout: {msg}"),
            Self::Convert(e) => write!(f, "video color conversion failed: {e}"),
            Self::Backend(e) => write!(f, "video bridge error: {e}"),
        }
    }
}

impl core::error::Error for MediaBridgeError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Convert(e) => Some(e),
            Self::Backend(e) => Some(&**e),
            _ => None,
        }
    }
}

fn box_err(err: MediaBridgeError) -> BoxedError {
    Box::new(err)
}

/// Reusable planar-to-packed video renderer implementing [`RenderFrame`].
pub struct VideoRenderPlan {
    canvas_format: PixelFormat,
    reconstruction: PlanarReconstructionOptions,
    conversion: ConvertPolicy,
    color_services: ColorServices,
    storage: Option<StorageBudget>,
}

impl VideoRenderPlan {
    /// Construct a video renderer targeting packed `canvas_format` with siting-exact
    /// bilinear chroma upsampling and permissive display conversion.
    #[must_use]
    pub fn new(canvas_format: PixelFormat) -> Self {
        Self {
            canvas_format,
            reconstruction: PlanarReconstructionOptions::bilinear(),
            conversion: ConvertPolicy::permissive(),
            color_services: ColorServices::inherit(),
            storage: None,
        }
    }

    /// Configure chroma upsampling and optional `ChromaSiting::Unknown` fallback.
    #[must_use]
    pub const fn with_reconstruction(mut self, options: PlanarReconstructionOptions) -> Self {
        self.reconstruction = options;
        self
    }

    /// Configure color/depth/gamut conversion policy.
    #[must_use]
    pub fn with_conversion(mut self, policy: ConvertPolicy) -> Self {
        self.conversion = policy;
        self
    }

    /// Attach explicit [`ColorServices`] (CMS binding, `StrictNoIcc`, and/or tone mapper).
    #[must_use]
    pub fn with_color_services(mut self, services: ColorServices) -> Self {
        self.color_services = services;
        self
    }

    /// Bound intermediate render buffers (cropped halo buffers and F32 color-conversion
    /// intermediates) against `storage`.
    #[must_use]
    pub fn with_storage(mut self, storage: StorageBudget) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Optional [`StorageBudget`] governing temporary render intermediates.
    #[must_use]
    pub fn storage(&self) -> Option<&StorageBudget> {
        self.storage.as_ref()
    }

    /// Target packed canvas [`PixelFormat`].
    #[must_use]
    pub fn canvas_format(&self) -> &PixelFormat {
        &self.canvas_format
    }
}

/// Compute the halo-expanded coded luma window `[win_x, win_x + win_w) × [win_y, win_y + win_h)`
/// aligned to subsampling divisors `(hx, vy)` so bilinear chroma upsampling over `crop`
/// retains its 1-chroma-sample border halo on every subsampled axis and produces 0-bit
/// identical results to full-frame reconstruction followed by cropping.
fn halo_window(
    coded_w: u32,
    coded_h: u32,
    crop: zencodec_media::frame::Rect,
    hx: u8,
    vy: u8,
) -> Result<PixelRect, BoxedError> {
    let crop_end_x = crop
        .x
        .checked_add(crop.width)
        .filter(|&end| end <= coded_w && crop.width > 0)
        .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
    let crop_end_y = crop
        .y
        .checked_add(crop.height)
        .filter(|&end| end <= coded_h && crop.height > 0)
        .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;

    let (win_x, win_x_end) = if hx > 1 {
        let h = u32::from(hx);
        let coded_cw = coded_w.div_ceil(h);
        let cx_min = (crop.x / h).saturating_sub(1);
        let cx_max = ((crop_end_x - 1) / h)
            .saturating_add(1)
            .min(coded_cw.saturating_sub(1));
        (
            cx_min.saturating_mul(h),
            (cx_max + 1).saturating_mul(h).min(coded_w),
        )
    } else {
        (crop.x, crop_end_x)
    };

    let (win_y, win_y_end) = if vy > 1 {
        let v = u32::from(vy);
        let coded_ch = coded_h.div_ceil(v);
        let cy_min = (crop.y / v).saturating_sub(1);
        let cy_max = ((crop_end_y - 1) / v)
            .saturating_add(1)
            .min(coded_ch.saturating_sub(1));
        (
            cy_min.saturating_mul(v),
            (cy_max + 1).saturating_mul(v).min(coded_h),
        )
    } else {
        (crop.y, crop_end_y)
    };

    Ok(PixelRect::new(
        win_x,
        win_y,
        win_x_end - win_x,
        win_y_end - win_y,
    ))
}

impl RenderFrame for VideoRenderPlan {
    fn canvas_descriptor(&self, track: &TrackInfo) -> Result<PixelFormat, BoxedError> {
        if track.video_info().is_none() {
            return Err(box_err(MediaBridgeError::UnsupportedLayout(
                "VideoRenderPlan renders video tracks only",
            )));
        }
        Ok(self.canvas_format.clone())
    }

    fn render(&mut self, frame: &Frame, canvas: &mut PixelViewMut<'_>) -> Result<(), BoxedError> {
        if canvas.format() != &self.canvas_format {
            return Err(box_err(MediaBridgeError::UnsupportedLayout(
                "canvas format does not match VideoRenderPlan::canvas_format",
            )));
        }
        let interp = frame.interpretation();
        let crop = interp.geometry.crop;
        if crop.width != canvas.width() || crop.height != canvas.height() {
            return Err(box_err(MediaBridgeError::GeometryMismatch));
        }
        let cicp = interp
            .color
            .resolved
            .ok_or_else(|| box_err(MediaBridgeError::MissingColorClaim))?;
        let mapped = frame
            .map()
            .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
        if mapped.plane_count() != interp.layout.planes().len() {
            return Err(box_err(MediaBridgeError::GeometryMismatch));
        }

        let sub = interp.layout.subsampling();
        let (hx_chroma, vy_chroma) = subsampling_divisors(sub);
        let win = halo_window(
            interp.geometry.coded_width,
            interp.geometry.coded_height,
            crop,
            hx_chroma,
            vy_chroma,
        )?;
        let needs_halo_crop = win.x != crop.x
            || win.y != crop.y
            || win.width != crop.width
            || win.height != crop.height;
        let crop_off_x = (crop.x - win.x) as usize;
        let crop_off_y = crop.y - win.y;

        let mut has_alpha = false;
        let mut is_mono = true;
        let mut max_src_sample = SampleType::U8;
        let mut max_code_bits = 8u8;

        // Build zero-copy zenpixels::PlaneView slices covering the halo-expanded coded window.
        let mut raw_planes: Vec<AnyPlane<'_>> = Vec::with_capacity(interp.layout.planes().len());
        let mut plane_meta: Vec<(PlaneFormat, u32, u32, usize, usize, usize)> =
            Vec::with_capacity(interp.layout.planes().len());
        for (idx, spec) in interp.layout.planes().iter().enumerate() {
            let role = media_to_pixel_role(spec.role());
            if role == PixelPlaneRole::Alpha {
                has_alpha = true;
            } else if role != PixelPlaneRole::Y {
                is_mono = false;
            }
            let (hx, vy) = if is_chroma_role(spec.role()) {
                (hx_chroma, vy_chroma)
            } else {
                (1u8, 1u8)
            };
            let (sample, packing) = plane_encoding_to_zenpixels(spec.encoding())?;
            match spec.encoding() {
                PlaneEncoding::Float(_) => {
                    max_src_sample = SampleType::F32;
                    max_code_bits = 32;
                }
                PlaneEncoding::Code(se) => {
                    max_code_bits = max_code_bits.max(se.code_bits());
                    if max_src_sample != SampleType::F32 && se.code_bits() > 8 {
                        max_src_sample = SampleType::U16;
                    }
                }
                _ => {}
            }
            let pf = PlaneFormat::new(role, sample)
                .with_packing(packing)
                .with_subsampling(hx, vy);
            let (ew, eh) = pf
                .cropped_plane_dimensions(win)
                .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
            let comps = usize::from(pf.components);
            let x0 = (win.x / u32::from(hx)) as usize * comps;
            let y0 = (win.y / u32::from(vy)) as usize;
            let plane = mapped
                .plane(idx)
                .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
            raw_planes.push(plane);
            plane_meta.push((pf, ew, eh, x0, y0, comps));
        }

        let mut cropped_planes: Vec<AnyPlane<'_>> = Vec::with_capacity(raw_planes.len());
        for (plane, &(_, ew, eh, x0, y0, comps)) in raw_planes.iter().zip(&plane_meta) {
            let cropped = match plane {
                AnyPlane::U8(p) => AnyPlane::U8(
                    p.sub(x0, y0, (ew as usize) * comps, eh as usize)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
                ),
                AnyPlane::U16(p) => AnyPlane::U16(
                    p.sub(x0, y0, (ew as usize) * comps, eh as usize)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
                ),
                AnyPlane::F32(p) => AnyPlane::F32(
                    p.sub(x0, y0, (ew as usize) * comps, eh as usize)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
                ),
                _ => {
                    return Err(box_err(MediaBridgeError::UnsupportedLayout(
                        "unknown plane sample storage",
                    )));
                }
            };
            cropped_planes.push(cropped);
        }

        let mut views: Vec<PlaneView<'_>> = Vec::with_capacity(cropped_planes.len());
        for (cropped, &(pf, ew, eh, _, _, _)) in cropped_planes.iter().zip(&plane_meta) {
            let (bytes, stride_bytes): (&[u8], usize) = match cropped {
                AnyPlane::U8(sub_p) => (sub_p.strided_samples(), sub_p.stride()),
                AnyPlane::U16(sub_p) => (
                    bytemuck::cast_slice(sub_p.strided_samples()),
                    sub_p.stride() * 2,
                ),
                AnyPlane::F32(sub_p) => (
                    bytemuck::cast_slice(sub_p.strided_samples()),
                    sub_p.stride() * 4,
                ),
                _ => {
                    return Err(box_err(MediaBridgeError::UnsupportedLayout(
                        "unknown plane sample storage",
                    )));
                }
            };
            let pv = PlaneView::new(bytes, ew, eh, stride_bytes, pf)
                .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
            views.push(pv);
        }

        let range = if cicp.full_range {
            SignalRange::Full
        } else {
            SignalRange::Narrow
        };
        let alpha = if has_alpha {
            AlphaMode::Straight
        } else {
            AlphaMode::None
        };
        let transfer = Transfer::from_cicp(cicp.transfer_characteristics)
            .ok_or_else(|| box_err(MediaBridgeError::Convert(ConvertError::UnsupportedRoute)))?;
        let src_color = if is_mono {
            ColorSpace::Gray {
                transfer,
                range,
                alpha,
                white: None,
            }
        } else {
            let primaries = Primaries::from_cicp(cicp.color_primaries).ok_or_else(|| {
                box_err(MediaBridgeError::Convert(ConvertError::UnsupportedRoute))
            })?;
            ColorSpace::Rgb {
                primaries,
                transfer,
                range,
                alpha,
                white: None,
            }
        };

        // Always validate the caller's ConvertPolicy + ColorServices against the true
        // reconstructed source depth, range, alpha, primaries, and transfer BEFORE considering
        // any direct reconstruction shortcut.
        let src_layout = match (is_mono, has_alpha, max_src_sample) {
            (true, false, SampleType::U8) => SampleLayout::GRAY8,
            (true, false, SampleType::U16) => SampleLayout::GRAY16,
            (true, false, _) => SampleLayout::GRAYF32,
            (true, true, SampleType::U8) => SampleLayout::GRAYA8,
            (true, true, SampleType::U16) => SampleLayout::GRAYA16,
            (true, true, _) => SampleLayout::GRAYAF32,
            (false, false, SampleType::U8) => SampleLayout::RGB8,
            (false, false, SampleType::U16) => SampleLayout::RGB16,
            (false, false, _) => SampleLayout::RGBF32,
            (false, true, SampleType::U8) => SampleLayout::RGBA8,
            (false, true, SampleType::U16) => SampleLayout::RGBA16,
            (false, true, _) => SampleLayout::RGBAF32,
        };
        let reconstructed_src_fmt = PixelFormat::new(src_layout, src_color.clone());
        let policy = self
            .conversion
            .clone()
            .with_color_services(&self.color_services);
        let _policy_check = RowPlan::build(&reconstructed_src_fmt, canvas.format(), &policy, None)
            .map_err(|e| box_err(MediaBridgeError::Convert(*e.error())))?;

        let dst_bits = canvas.format().layout.sample.bits();
        let depth_reduces = max_src_sample.is_float() || max_code_bits > dst_bits;
        let quantization_matches_direct = !depth_reduces || policy.depth == DepthPolicy::Round;

        let mut planar = PlanarView::new_with_crop_origin(
            win.width,
            win.height,
            (win.x, win.y),
            interp.layout.siting(),
            &views,
        )
        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?
        .with_color(src_color)
        .with_matrix_coefficients(cicp.matrix())
        .with_range_override(range);
        if has_alpha {
            planar = planar.with_alpha_override(AlphaMode::Straight);
        }

        // Fast zero-intermediate (or halo-only) path when ConvertPolicy has passed and the
        // destination canvas directly matches the reconstructed full-range color space.
        let direct_compatible = quantization_matches_direct
            && match (&planar.color(), &canvas.format().color) {
                (
                    Some(ColorSpace::Rgb {
                        primaries: sp,
                        transfer: st,
                        white: sw,
                        ..
                    }),
                    ColorSpace::Rgb {
                        primaries: dp,
                        transfer: dt,
                        range: SignalRange::Full,
                        alpha: da,
                        white: dw,
                    },
                ) => {
                    sp == dp
                        && st == dt
                        && sw == dw
                        && (!has_alpha || *da == AlphaMode::Straight)
                        && matches!(
                            canvas.format().layout.sample,
                            SampleType::U8 | SampleType::U16 | SampleType::F32
                        )
                }
                (
                    Some(ColorSpace::Gray {
                        transfer: st,
                        white: sw,
                        ..
                    }),
                    ColorSpace::Gray {
                        transfer: dt,
                        range: SignalRange::Full,
                        alpha: da,
                        white: dw,
                    },
                ) => {
                    st == dt
                        && sw == dw
                        && (!has_alpha || *da == AlphaMode::Straight)
                        && matches!(
                            canvas.format().layout.sample,
                            SampleType::U8 | SampleType::U16 | SampleType::F32
                        )
                }
                _ => false,
            };

        if direct_compatible {
            if !needs_halo_crop {
                if reconstruct_planar_into(&planar, canvas, &self.reconstruction).is_ok() {
                    return Ok(());
                }
            } else {
                let bpp = canvas.format().layout.bytes_per_pixel();
                let win_stride = (win.width as usize)
                    .checked_mul(bpp)
                    .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
                let total_bytes = win_stride
                    .checked_mul(win.height as usize)
                    .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
                let mut accounted_buf = match &self.storage {
                    Some(budget) => Some(
                        budget
                            .bytes(total_bytes)
                            .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
                    ),
                    None => None,
                };
                let mut fallback_buf = if accounted_buf.is_none() {
                    Some(
                        PixelBuf::new(win.width, win.height, canvas.format().clone())
                            .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
                    )
                } else {
                    None
                };
                let direct_ok = if let Some(owned) = &mut accounted_buf {
                    let mut view = PixelViewMut::new(
                        owned.as_mut_slice(),
                        win.width,
                        win.height,
                        win_stride,
                        canvas.format(),
                    )
                    .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
                    reconstruct_planar_into(&planar, &mut view, &self.reconstruction).is_ok()
                } else {
                    let pb = fallback_buf.as_mut().expect("allocated above");
                    reconstruct_planar_into(&planar, &mut pb.as_view_mut(), &self.reconstruction)
                        .is_ok()
                };
                if direct_ok {
                    let src_x_byte = crop_off_x * bpp;
                    let copy_bytes = crop.width as usize * bpp;
                    if let Some(owned) = &accounted_buf {
                        let raw = owned.as_slice();
                        for y in 0..crop.height {
                            let sy = (crop_off_y + y) as usize;
                            let start = sy * win_stride + src_x_byte;
                            canvas.row_mut(y)[..copy_bytes]
                                .copy_from_slice(&raw[start..start + copy_bytes]);
                        }
                    } else if let Some(pb) = &fallback_buf {
                        let v = pb.as_view();
                        for y in 0..crop.height {
                            let src_row = v.row(crop_off_y + y);
                            canvas.row_mut(y)[..copy_bytes]
                                .copy_from_slice(&src_row[src_x_byte..src_x_byte + copy_bytes]);
                        }
                    }
                    return Ok(());
                }
            }
        }

        // General color-managed / gamut / transfer / tone-map / dithering path:
        // reconstruct the halo window to full-range F32 (admitted under `self.storage` when set),
        // then run RowPlan over only the visible crop rectangle.
        let f32_channels = if has_alpha { 4usize } else { 3usize };
        let f32_bytes = (win.width as usize)
            .checked_mul(win.height as usize)
            .and_then(|px| px.checked_mul(f32_channels))
            .and_then(|c| c.checked_mul(core::mem::size_of::<f32>()))
            .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
        let _f32_reservation = match &self.storage {
            Some(budget) => Some(
                budget
                    .reserve_backing(f32_bytes, NonZeroUsize::MIN)
                    .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
            ),
            None => None,
        };
        let f32_buf = reconstruct_planar_f32(&planar, &self.reconstruction)
            .map_err(|e| box_err(MediaBridgeError::Convert(*e.error())))?;

        // Policy depth/range/gamut were already enforced above against `reconstructed_src_fmt`.
        // Adapt only the internal F32 intermediate policy so valid integer-to-integer routes
        // (e.g. 8-bit narrow -> 8-bit full under DepthPolicy::Forbid) aren't falsely rejected
        // because the intermediate happens to be F32.
        let mut f32_policy = policy;
        f32_policy.range = RangePolicy::Convert;
        if f32_policy.depth == DepthPolicy::Forbid {
            f32_policy.depth = DepthPolicy::Round;
        }
        if !max_src_sample.is_float() {
            f32_policy.source_facts.float = FloatFacts::UNIT_INTERVAL;
        }
        let row_plan = RowPlan::build(f32_buf.format(), canvas.format(), &f32_policy, None)
            .map_err(|e| box_err(MediaBridgeError::Convert(*e.error())))?;
        let f32_view = f32_buf.as_view();
        let src_bpp = f32_buf.format().layout.bytes_per_pixel();
        let src_x_byte = crop_off_x * src_bpp;
        let src_row_bytes = crop.width as usize * src_bpp;
        let w = canvas.width();
        let h = canvas.height();
        let dst_bpp = canvas.format().layout.bytes_per_pixel();
        let dst_row_bytes = w as usize * dst_bpp;
        // Preserve absolute coded-frame coordinates `(crop.x + x, crop.y + y)` on BOTH axes
        // so spatially-indexed quantization (`DepthPolicy::OrderedDither`, 4×4 Bayer phase)
        // matches full-frame reconstruction followed by cropping with 0-bit exactness.
        let x_phase_lead = if f32_policy.depth == DepthPolicy::OrderedDither {
            (crop.x as usize) & 3
        } else {
            0
        };
        if x_phase_lead == 0 {
            for y in 0..h {
                let src_row = &f32_view.row(crop_off_y + y)[src_x_byte..src_x_byte + src_row_bytes];
                row_plan
                    .convert_row_at(
                        crop.y + y,
                        src_row,
                        &mut canvas.row_mut(y)[..dst_row_bytes],
                        w,
                    )
                    .map_err(|e| box_err(MediaBridgeError::Convert(*e.error())))?;
            }
        } else {
            let padded_w = w
                .checked_add(x_phase_lead as u32)
                .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
            let src_lead_bytes = x_phase_lead * src_bpp;
            let dst_lead_bytes = x_phase_lead * dst_bpp;
            let src_pad_len = src_lead_bytes
                .checked_add(src_row_bytes)
                .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
            let dst_pad_len = dst_lead_bytes
                .checked_add(dst_row_bytes)
                .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
            let pad_total = src_pad_len
                .checked_add(dst_pad_len)
                .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
            let mut accounted_pad = match &self.storage {
                Some(budget) => Some(
                    budget
                        .bytes(pad_total)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
                ),
                None => None,
            };
            let mut unbudgeted_pad = if accounted_pad.is_none() {
                let mut v = Vec::new();
                v.try_reserve_exact(pad_total).map_err(|_| {
                    box_err(MediaBridgeError::Convert(ConvertError::AllocationFailed))
                })?;
                v.resize(pad_total, 0);
                Some(v)
            } else {
                None
            };
            let pad_slice: &mut [u8] = if let Some(owned) = &mut accounted_pad {
                owned.as_mut_slice()
            } else {
                unbudgeted_pad
                    .as_mut()
                    .expect("allocated above")
                    .as_mut_slice()
            };
            let (src_pad, dst_pad) = pad_slice.split_at_mut(src_pad_len);
            for y in 0..h {
                let src_row = &f32_view.row(crop_off_y + y)[src_x_byte..src_x_byte + src_row_bytes];
                src_pad[src_lead_bytes..].copy_from_slice(src_row);
                row_plan
                    .convert_row_at(crop.y + y, src_pad, dst_pad, padded_w)
                    .map_err(|e| box_err(MediaBridgeError::Convert(*e.error())))?;
                canvas.row_mut(y)[..dst_row_bytes].copy_from_slice(&dst_pad[dst_lead_bytes..]);
            }
        }
        drop(f32_buf);
        Ok(())
    }
}

struct CachedCapture {
    src_format: PixelFormat,
    size: (u32, u32),
    epoch: u32,
    container_color: Option<Cicp>,
    plan: PlanarCapturePlan,
    interp: Arc<FrameInterpretation>,
}

/// Reusable packed-to-planar video capturer implementing [`CaptureFrame`], backed by
/// [`PlanarCapturePlan`] and [`StorageBudget`].
///
/// Both captured plane owners and the reusable downsampling scratch buffer are allocated
/// through and accounted against the caller's [`StorageBudget`].
pub struct VideoCapturePlan {
    planes: Box<[PlaneFormat]>,
    chroma_siting: ChromaSiting,
    signal: Cicp,
    alpha: AlphaMode,
    diffuse_white: Option<DiffuseWhite>,
    options: PlanarCaptureOptions,
    conversion: ConvertPolicy,
    color_services: ColorServices,
    storage: StorageBudget,
    scratch: Option<OwnedStorage<f64>>,
    cached: Option<CachedCapture>,
}

impl VideoCapturePlan {
    /// Construct a video capturer targeting the given planar storage layout, chroma siting,
    /// and coded CICP signal under `storage`.
    #[must_use]
    pub fn new(
        planes: impl Into<Box<[PlaneFormat]>>,
        chroma_siting: ChromaSiting,
        signal: Cicp,
        storage: StorageBudget,
    ) -> Self {
        Self {
            planes: planes.into(),
            chroma_siting,
            signal,
            alpha: AlphaMode::Straight,
            diffuse_white: None,
            options: PlanarCaptureOptions::siting_aware(),
            conversion: ConvertPolicy::permissive(),
            color_services: ColorServices::inherit(),
            storage,
            scratch: None,
            cached: None,
        }
    }

    /// Set chroma downsampling options and optional unknown-siting fallback.
    #[must_use]
    pub fn with_options(mut self, options: PlanarCaptureOptions) -> Self {
        self.options = options;
        self.cached = None;
        self
    }

    /// Set alpha mode for an optional `PlaneRole::Alpha` destination plane.
    #[must_use]
    pub fn with_alpha(mut self, alpha: AlphaMode) -> Self {
        self.alpha = alpha;
        self.cached = None;
        self
    }

    /// Set explicit diffuse-white anchor.
    #[must_use]
    pub fn with_diffuse_white(mut self, white: DiffuseWhite) -> Self {
        self.diffuse_white = Some(white);
        self.cached = None;
        self
    }

    /// Set conversion policy (`DepthPolicy`, `GamutPolicy`, `RangePolicy`, `AlphaPolicy`).
    #[must_use]
    pub fn with_conversion(mut self, policy: ConvertPolicy) -> Self {
        self.conversion = policy;
        self.cached = None;
        self
    }

    /// Attach explicit [`ColorServices`].
    #[must_use]
    pub fn with_color_services(mut self, services: ColorServices) -> Self {
        self.color_services = services;
        self.cached = None;
        self
    }

    /// Exact scratch bytes required in [`StorageBudget`] to capture frames matching
    /// `spec` and `config` (for upfront route admission).
    pub fn required_scratch_bytes(
        &self,
        spec: &PixelFormat,
        config: &CodecConfig,
    ) -> Result<usize, BoxedError> {
        let video = self.validated_video_config(config)?;
        let (plan, _) = self.build_plan_for(
            spec,
            video.coded_width,
            video.coded_height,
            config.epoch,
            video.color,
        )?;
        Ok(plan.required_scratch_bytes())
    }

    /// Scratch bytes currently retained inside this capturer and charged against its
    /// [`StorageBudget`].
    #[must_use]
    pub fn retained_scratch_bytes(&self) -> usize {
        self.scratch
            .as_ref()
            .map_or(0, |s| s.len() * core::mem::size_of::<f64>())
    }

    /// Drop any retained capture scratch buffer, releasing its [`StorageBudget`] charge.
    pub fn clear_scratch(&mut self) {
        self.scratch = None;
    }

    fn validated_video_config(
        &self,
        config: &CodecConfig,
    ) -> Result<zencodec_media::track::VideoTrackInfo, BoxedError> {
        let video = config
            .video
            .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
        if let Some(cfg_color) = video.color
            && cfg_color != self.signal
        {
            return Err(box_err(MediaBridgeError::ColorClaimConflict {
                target: self.signal,
                config: cfg_color,
            }));
        }
        Ok(video)
    }

    fn build_plan_for(
        &self,
        src_format: &PixelFormat,
        width: u32,
        height: u32,
        epoch: u32,
        container_color: Option<Cicp>,
    ) -> Result<(PlanarCapturePlan, Arc<FrameInterpretation>), BoxedError> {
        let mut spec = PlanarCaptureSpec::full_frame(
            width,
            height,
            self.planes.clone(),
            self.chroma_siting,
            self.signal,
        )
        .with_alpha(self.alpha);
        if let Some(w) = self.diffuse_white {
            spec = spec.with_diffuse_white(w);
        }
        let policy = self
            .conversion
            .clone()
            .with_color_services(&self.color_services);
        let plan =
            PlanarCapturePlan::build_with_options(src_format, spec, &policy, self.options, None)
                .map_err(|e| box_err(MediaBridgeError::Convert(*e.error())))?;
        let layout = build_media_planar_layout(&self.planes, plan.resolved_siting())?;
        // Never invent a bitstream color claim prior to encode: preserve any independently
        // sourced container color claim (`video.color`) and record `self.signal` as the
        // resolved sample color actually captured into the planes.
        let interp = Arc::new(
            FrameInterpretation::new(FrameGeometry::full(width, height), layout)
                .with_color(ColorClaim::new(None, container_color, Some(self.signal)))
                .with_config_epoch(epoch),
        );
        Ok((plan, interp))
    }
}

enum StagingPlane {
    U8 {
        buf: OwnedStorage<u8>,
        width_samples: usize,
        height: usize,
        stride_samples: usize,
        pf: PlaneFormat,
        enc: PlaneEncoding,
    },
    U16 {
        buf: OwnedStorage<u16>,
        width_samples: usize,
        height: usize,
        stride_samples: usize,
        pf: PlaneFormat,
        enc: PlaneEncoding,
    },
    F32 {
        buf: OwnedStorage<f32>,
        width_samples: usize,
        height: usize,
        stride_samples: usize,
        pf: PlaneFormat,
        enc: PlaneEncoding,
    },
}

impl CaptureFrame for VideoCapturePlan {
    fn validate_input(&self, spec: &PixelFormat, config: &CodecConfig) -> Result<(), BoxedError> {
        let video = self.validated_video_config(config)?;
        let _ = self.build_plan_for(
            spec,
            video.coded_width,
            video.coded_height,
            config.epoch,
            video.color,
        )?;
        Ok(())
    }

    fn capture(
        &mut self,
        pixels: &PixelView<'_>,
        config: &CodecConfig,
        timing: FrameTiming,
    ) -> Result<Frame, BoxedError> {
        let video = self.validated_video_config(config)?;
        let (w, h) = (pixels.width(), pixels.height());
        if (video.coded_width, video.coded_height) != (w, h) {
            return Err(box_err(MediaBridgeError::GeometryMismatch));
        }

        let needs_rebuild = match &self.cached {
            Some(c) => {
                &c.src_format != pixels.format()
                    || c.size != (w, h)
                    || c.epoch != config.epoch
                    || c.container_color != video.color
            }
            None => true,
        };
        if needs_rebuild {
            let (plan, interp) =
                self.build_plan_for(pixels.format(), w, h, config.epoch, video.color)?;
            self.cached = Some(CachedCapture {
                src_format: pixels.format().clone(),
                size: (w, h),
                epoch: config.epoch,
                container_color: video.color,
                plan,
                interp,
            });
        }

        let cached = self.cached.as_ref().expect("populated above");
        let (plan, interp) = (&cached.plan, &cached.interp);
        let vis = PixelRect::new(0, 0, w, h);
        let needed_f64s = plan.required_scratch_f64s();
        if needed_f64s > 0
            && self
                .scratch
                .as_ref()
                .is_none_or(|buf| buf.len() < needed_f64s)
        {
            self.scratch = None;
            let buf = self
                .storage
                .filled(needed_f64s, 0.0f64)
                .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
            self.scratch = Some(buf);
        }

        // Allocate each plane through `self.storage` so every frame obeys StorageBudget.
        let mut staging: Vec<StagingPlane> = Vec::with_capacity(self.planes.len());
        for &pf in self.planes.iter() {
            let (pw, ph) = pf
                .cropped_plane_dimensions(vis)
                .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
            let comps = usize::from(pf.components);
            let width_samples = (pw as usize)
                .checked_mul(comps)
                .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
            let height = ph as usize;
            let stride_samples = width_samples;
            let total_samples = stride_samples
                .checked_mul(height)
                .ok_or_else(|| box_err(MediaBridgeError::GeometryMismatch))?;
            let enc = plane_format_to_media_encoding(pf)?;
            match pf.sample {
                SampleType::U8 => {
                    let buf = self
                        .storage
                        .filled(total_samples, 0u8)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
                    staging.push(StagingPlane::U8 {
                        buf,
                        width_samples,
                        height,
                        stride_samples,
                        pf,
                        enc,
                    });
                }
                SampleType::U16 => {
                    let buf = self
                        .storage
                        .filled(total_samples, 0u16)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
                    staging.push(StagingPlane::U16 {
                        buf,
                        width_samples,
                        height,
                        stride_samples,
                        pf,
                        enc,
                    });
                }
                SampleType::F32 => {
                    let buf = self
                        .storage
                        .filled(total_samples, 0.0f32)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
                    staging.push(StagingPlane::F32 {
                        buf,
                        width_samples,
                        height,
                        stride_samples,
                        pf,
                        enc,
                    });
                }
                _ => {
                    return Err(box_err(MediaBridgeError::UnsupportedLayout(
                        "VideoCapturePlan supports U8, U16, and F32 destination planes",
                    )));
                }
            }
        }

        {
            let mut plane_views: Vec<PlaneViewMut<'_>> = Vec::with_capacity(staging.len());
            for sp in &mut staging {
                match sp {
                    StagingPlane::U8 {
                        buf,
                        width_samples,
                        height,
                        stride_samples,
                        pf,
                        ..
                    } => {
                        let pw = (*width_samples / usize::from(pf.components)) as u32;
                        plane_views.push(
                            PlaneViewMut::new(
                                buf.as_mut_slice(),
                                pw,
                                *height as u32,
                                *stride_samples,
                                *pf,
                            )
                            .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
                        );
                    }
                    StagingPlane::U16 {
                        buf,
                        width_samples,
                        height,
                        stride_samples,
                        pf,
                        ..
                    } => {
                        let pw = (*width_samples / usize::from(pf.components)) as u32;
                        plane_views.push(
                            PlaneViewMut::new(
                                bytemuck::cast_slice_mut(buf.as_mut_slice()),
                                pw,
                                *height as u32,
                                *stride_samples * 2,
                                *pf,
                            )
                            .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
                        );
                    }
                    StagingPlane::F32 {
                        buf,
                        width_samples,
                        height,
                        stride_samples,
                        pf,
                        ..
                    } => {
                        let pw = (*width_samples / usize::from(pf.components)) as u32;
                        plane_views.push(
                            PlaneViewMut::new(
                                bytemuck::cast_slice_mut(buf.as_mut_slice()),
                                pw,
                                *height as u32,
                                *stride_samples * 4,
                                *pf,
                            )
                            .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?,
                        );
                    }
                }
            }
            let mut dst_view = PlanarViewMut::new_with_crop_origin(
                w,
                h,
                (0, 0),
                plan.resolved_siting(),
                plane_views,
            )
            .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
            let scratch_slice: &mut [f64] = match &mut self.scratch {
                Some(buf) => buf.as_mut_slice(),
                None => &mut [],
            };
            let mut scratch = CaptureScratch::from_f64_slice(scratch_slice);
            plan.execute(pixels.clone(), &mut dst_view, &mut scratch)
                .map_err(|e| box_err(MediaBridgeError::Convert(*e.error())))?;
        }

        let mut encoded_planes: Vec<(OwnedPlane, PlaneEncoding)> =
            Vec::with_capacity(staging.len());
        for sp in staging {
            match sp {
                StagingPlane::U8 {
                    buf,
                    width_samples,
                    height,
                    stride_samples,
                    enc,
                    ..
                } => {
                    let plane = Plane::new(buf.freeze(), width_samples, height, stride_samples)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
                    encoded_planes.push((OwnedPlane::AccountedU8(plane), enc));
                }
                StagingPlane::U16 {
                    buf,
                    width_samples,
                    height,
                    stride_samples,
                    enc,
                    ..
                } => {
                    let plane = Plane::new(buf.freeze(), width_samples, height, stride_samples)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
                    encoded_planes.push((OwnedPlane::AccountedU16(plane), enc));
                }
                StagingPlane::F32 {
                    buf,
                    width_samples,
                    height,
                    stride_samples,
                    enc,
                    ..
                } => {
                    let plane = Plane::new(buf.freeze(), width_samples, height, stride_samples)
                        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
                    encoded_planes.push((OwnedPlane::AccountedF32(plane), enc));
                }
            }
        }
        let picture = OwnedPlanes::from_encoded_planes(encoded_planes)
            .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))?;
        Ok(Frame::new(
            Arc::new(picture),
            Arc::clone(interp),
            timing,
            Presentation::default(),
        ))
    }
}

fn subsampling_divisors(sub: ChromaSubsampling) -> (u8, u8) {
    match sub {
        ChromaSubsampling::Mono | ChromaSubsampling::Yuv444 => (1, 1),
        ChromaSubsampling::Yuv422 => (2, 1),
        ChromaSubsampling::Yuv420 => (2, 2),
        _ => (1, 1),
    }
}

fn media_to_pixel_role(role: MediaPlaneRole) -> PixelPlaneRole {
    match role {
        MediaPlaneRole::Luma => PixelPlaneRole::Y,
        MediaPlaneRole::Cb => PixelPlaneRole::Cb,
        MediaPlaneRole::Cr => PixelPlaneRole::Cr,
        MediaPlaneRole::CbCr => PixelPlaneRole::CbCr,
        MediaPlaneRole::CrCb => PixelPlaneRole::CrCb,
        MediaPlaneRole::Red => PixelPlaneRole::R,
        MediaPlaneRole::Green => PixelPlaneRole::G,
        MediaPlaneRole::Blue => PixelPlaneRole::B,
        MediaPlaneRole::Alpha => PixelPlaneRole::Alpha,
        _ => PixelPlaneRole::Y,
    }
}

fn pixel_to_media_role(role: PixelPlaneRole) -> Result<MediaPlaneRole, BoxedError> {
    match role {
        PixelPlaneRole::Y => Ok(MediaPlaneRole::Luma),
        PixelPlaneRole::Cb => Ok(MediaPlaneRole::Cb),
        PixelPlaneRole::Cr => Ok(MediaPlaneRole::Cr),
        PixelPlaneRole::CbCr => Ok(MediaPlaneRole::CbCr),
        PixelPlaneRole::CrCb => Ok(MediaPlaneRole::CrCb),
        PixelPlaneRole::R => Ok(MediaPlaneRole::Red),
        PixelPlaneRole::G => Ok(MediaPlaneRole::Green),
        PixelPlaneRole::B => Ok(MediaPlaneRole::Blue),
        PixelPlaneRole::Alpha => Ok(MediaPlaneRole::Alpha),
        _ => Err(box_err(MediaBridgeError::UnsupportedLayout(
            "plane role has no media PlanarLayout equivalent",
        ))),
    }
}

fn is_chroma_role(role: MediaPlaneRole) -> bool {
    matches!(
        role,
        MediaPlaneRole::Cb | MediaPlaneRole::Cr | MediaPlaneRole::CbCr | MediaPlaneRole::CrCb
    )
}

fn plane_encoding_to_zenpixels(enc: PlaneEncoding) -> Result<(SampleType, BitPacking), BoxedError> {
    match enc {
        PlaneEncoding::Code(se) => {
            let st = se.storage();
            let bits = se.code_bits();
            let shift = se.bit_shift();
            let total = st.bits();
            let packing = if shift == 0 && bits == total {
                BitPacking::Full
            } else if shift == 0 {
                BitPacking::Lsb { bits }
            } else if shift + bits == total {
                BitPacking::Msb { bits }
            } else {
                return Err(box_err(MediaBridgeError::UnsupportedLayout(
                    "mid-word bit shift not representable in BitPacking",
                )));
            };
            Ok((st, packing))
        }
        PlaneEncoding::Float(fe) => {
            if fe != FloatEncoding::NOMINAL {
                return Err(box_err(MediaBridgeError::UnsupportedLayout(
                    "non-nominal FloatEncoding scale/offset not supported in PlanarView",
                )));
            }
            Ok((SampleType::F32, BitPacking::Full))
        }
        _ => Err(box_err(MediaBridgeError::UnsupportedLayout(
            "unknown PlaneEncoding variant",
        ))),
    }
}

fn plane_format_to_media_encoding(pf: PlaneFormat) -> Result<PlaneEncoding, BoxedError> {
    if pf.sample == SampleType::F32 {
        return Ok(PlaneEncoding::Float(FloatEncoding::NOMINAL));
    }
    let se = pf.sample_encoding().ok_or_else(|| {
        box_err(MediaBridgeError::UnsupportedLayout(
            "destination plane has no uniform SampleEncoding",
        ))
    })?;
    Ok(PlaneEncoding::Code(se))
}

fn build_media_planar_layout(
    planes: &[PlaneFormat],
    resolved_siting: ChromaSiting,
) -> Result<PlanarLayout, BoxedError> {
    let mut specs = Vec::with_capacity(planes.len());
    let mut chroma_sub: Option<(u8, u8)> = None;
    let mut has_chroma = false;
    let mut is_rgb = false;
    for &pf in planes {
        let role = pixel_to_media_role(pf.role)?;
        if matches!(
            role,
            MediaPlaneRole::Red | MediaPlaneRole::Green | MediaPlaneRole::Blue
        ) {
            is_rgb = true;
        }
        if is_chroma_role(role) {
            has_chroma = true;
            if let Some(prev) = chroma_sub {
                if prev != pf.subsampling {
                    return Err(box_err(MediaBridgeError::UnsupportedLayout(
                        "mixed chroma subsampling across planes",
                    )));
                }
            } else {
                chroma_sub = Some(pf.subsampling);
            }
        }
        let enc = plane_format_to_media_encoding(pf)?;
        specs.push(PlaneSpec::new(role, enc));
    }
    let subsampling = if is_rgb {
        ChromaSubsampling::Yuv444
    } else if !has_chroma {
        ChromaSubsampling::Mono
    } else {
        match chroma_sub.unwrap_or((1, 1)) {
            (1, 1) => ChromaSubsampling::Yuv444,
            (2, 1) => ChromaSubsampling::Yuv422,
            (2, 2) => ChromaSubsampling::Yuv420,
            _ => {
                return Err(box_err(MediaBridgeError::UnsupportedLayout(
                    "chroma subsampling outside Mono/4:4:4/4:2:2/4:2:0",
                )));
            }
        }
    };
    PlanarLayout::new(specs, subsampling, resolved_siting)
        .map_err(|e| box_err(MediaBridgeError::Backend(Box::new(e))))
}

#[allow(dead_code)]
fn _assert_layout_import(_: SampleLayout) {}
