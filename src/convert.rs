//! Pixel format conversion for [`PixelSlice`].
//!
//! Supports lossless and well-defined conversions between pixel formats:
//! - **Depth**: U8 ↔ U16 (scale by ×257 / rounded ÷65536)
//! - **Add alpha**: Gray→GrayAlpha, Rgb→Rgba (opaque alpha)
//! - **Drop alpha**: GrayAlpha→Gray, Rgba→Rgb
//! - **Gray→RGB**: broadcast `v → (v, v, v)` via [`GrayExpand`]
//! - Any combination of the above in a single pass
//!
//! RGB→Gray is **not** supported (requires explicit luma coefficients).

use alloc::sync::Arc;

use crate::buffer::{
    AlphaMode, ChannelLayout, ChannelType, PixelBuffer, PixelDescriptor, PixelSlice,
};

/// How to expand grayscale channels to RGB.
///
/// Used by [`PixelSlice::convert()`] when the source layout is grayscale
/// and the target layout is RGB or RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GrayExpand {
    /// Channel broadcast: `v → (v, v, v)`. Lossless.
    Broadcast,
}

/// Policy for alpha channel removal. Required when converting
/// from a layout with alpha to one without.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AlphaPolicy {
    /// Discard only if every pixel is fully opaque. Returns error otherwise.
    DiscardIfOpaque,
    /// Discard unconditionally. Caller acknowledges data loss.
    DiscardUnchecked,
    /// Composite onto solid background (values in source range, 0–255 for U8).
    CompositeOnto {
        /// Red background value.
        r: u8,
        /// Green background value.
        g: u8,
        /// Blue background value.
        b: u8,
    },
    /// Return error rather than dropping alpha.
    Forbid,
}

/// Policy for bit depth reduction (U16→U8, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DepthPolicy {
    /// Round to nearest value.
    Round,
    /// Truncate (floor). Faster, biased toward lower values.
    Truncate,
    /// Return error rather than reducing depth.
    Forbid,
}

/// Explicit options for pixel format conversion. All lossy
/// operations require a policy choice — no silent defaults.
#[derive(Clone, Copy, Debug)]
pub struct ConvertOptions {
    /// How to expand grayscale to RGB.
    pub gray_expand: GrayExpand,
    /// How to handle alpha removal.
    pub alpha_policy: AlphaPolicy,
    /// How to handle depth reduction.
    pub depth_policy: DepthPolicy,
}

/// Error from [`PixelSlice::convert()`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConvertError {
    /// RGB-to-grayscale conversion requires explicit luma coefficients
    /// and is not supported by the built-in converter.
    RgbToGray,
    /// Source or target uses an unsupported channel type (F32, F16, I16).
    UnsupportedChannelType,
    /// Cross-layout conversion involving Bgra is not supported.
    UnsupportedLayout,
    /// Alpha channel is not fully opaque and [`AlphaPolicy::DiscardIfOpaque`] was set.
    AlphaNotOpaque,
    /// Depth reduction was requested but [`DepthPolicy::Forbid`] was set.
    DepthReductionForbidden,
    /// Alpha removal was requested but [`AlphaPolicy::Forbid`] was set.
    AlphaRemovalForbidden,
    /// Buffer allocation failed.
    AllocationFailed,
}

impl core::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::RgbToGray => write!(f, "RGB-to-grayscale requires explicit luma coefficients"),
            Self::UnsupportedChannelType => {
                write!(f, "unsupported channel type for conversion (only U8/U16)")
            }
            Self::UnsupportedLayout => {
                write!(f, "cross-layout Bgra conversion not supported")
            }
            Self::AlphaNotOpaque => {
                write!(f, "alpha channel is not fully opaque")
            }
            Self::DepthReductionForbidden => {
                write!(f, "depth reduction forbidden by policy")
            }
            Self::AlphaRemovalForbidden => {
                write!(f, "alpha removal forbidden by policy")
            }
            Self::AllocationFailed => {
                write!(f, "buffer allocation failed")
            }
        }
    }
}

fn validate_conversion(
    src_ty: ChannelType,
    src_layout: ChannelLayout,
    dst_ty: ChannelType,
    dst_layout: ChannelLayout,
) -> Result<(), ConvertError> {
    // Only U8 and U16 depths supported
    if !matches!(src_ty, ChannelType::U8 | ChannelType::U16)
        || !matches!(dst_ty, ChannelType::U8 | ChannelType::U16)
    {
        return Err(ConvertError::UnsupportedChannelType);
    }
    // Bgra needs swizzle for cross-layout conversion — not supported
    if src_layout != dst_layout
        && (matches!(src_layout, ChannelLayout::Bgra) || matches!(dst_layout, ChannelLayout::Bgra))
    {
        return Err(ConvertError::UnsupportedLayout);
    }
    // RGB → Gray requires explicit luma coefficients
    let src_is_rgb = matches!(
        src_layout,
        ChannelLayout::Rgb | ChannelLayout::Rgba | ChannelLayout::Bgra
    );
    let dst_is_gray = matches!(dst_layout, ChannelLayout::Gray | ChannelLayout::GrayAlpha);
    if src_is_rgb && dst_is_gray {
        return Err(ConvertError::RgbToGray);
    }
    Ok(())
}

// ── Channel I/O helpers ─────────────────────────────────────────────────

/// Read one channel from `src` at byte `offset` as a raw u16.
/// For U8: 0–255. For U16: 0–65535.
#[inline(always)]
fn read_ch(src: &[u8], offset: usize, ty: ChannelType) -> u16 {
    match ty {
        ChannelType::U8 => src[offset] as u16,
        _ => u16::from_ne_bytes([src[offset], src[offset + 1]]),
    }
}

/// Write one channel, converting depth between source and destination ranges.
#[inline(always)]
fn write_ch(dst: &mut [u8], offset: usize, v: u16, src_ty: ChannelType, dst_ty: ChannelType) {
    match (src_ty, dst_ty) {
        (ChannelType::U8, ChannelType::U8) => dst[offset] = v as u8,
        (ChannelType::U8, ChannelType::U16) => {
            let wide = v * 257;
            dst[offset..offset + 2].copy_from_slice(&wide.to_ne_bytes());
        }
        (ChannelType::U16, ChannelType::U8) => {
            // Correct rounding: (v * 255 + 32768) / 65536
            // This is the exact inverse of U8→U16 scaling (×257).
            dst[offset] = ((v as u32 * 255 + 32768) >> 16) as u8;
        }
        _ => {
            // U16→U16 (and any other same-depth)
            dst[offset..offset + 2].copy_from_slice(&v.to_ne_bytes());
        }
    }
}

/// Maximum channel value for a depth.
#[inline(always)]
fn max_value(ty: ChannelType) -> u16 {
    match ty {
        ChannelType::U8 => 255,
        _ => 65535,
    }
}

// ── Per-pixel conversion ────────────────────────────────────────────────

/// Read a source pixel as (c0, c1, c2, alpha) in the source depth range.
///
/// For gray sources, c0/c1/c2 are all the gray value (broadcast).
/// Alpha is set to max if the source has no alpha channel.
#[inline(always)]
fn read_rgba(
    src: &[u8],
    offset: usize,
    ty: ChannelType,
    layout: ChannelLayout,
    cs: usize,
    _expand: GrayExpand,
) -> (u16, u16, u16, u16) {
    let amax = max_value(ty);
    match layout {
        ChannelLayout::Gray => {
            let v = read_ch(src, offset, ty);
            (v, v, v, amax)
        }
        ChannelLayout::GrayAlpha => {
            let v = read_ch(src, offset, ty);
            let a = read_ch(src, offset + cs, ty);
            (v, v, v, a)
        }
        ChannelLayout::Rgb => {
            let r = read_ch(src, offset, ty);
            let g = read_ch(src, offset + cs, ty);
            let b = read_ch(src, offset + 2 * cs, ty);
            (r, g, b, amax)
        }
        // Rgba and Bgra: read 4 channels positionally
        _ => {
            let c0 = read_ch(src, offset, ty);
            let c1 = read_ch(src, offset + cs, ty);
            let c2 = read_ch(src, offset + 2 * cs, ty);
            let c3 = read_ch(src, offset + 3 * cs, ty);
            (c0, c1, c2, c3)
        }
    }
}

/// Write a pixel to the destination buffer with depth conversion.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn write_pixel(
    dst: &mut [u8],
    offset: usize,
    c0: u16,
    c1: u16,
    c2: u16,
    a: u16,
    src_ty: ChannelType,
    dst_ty: ChannelType,
    dst_layout: ChannelLayout,
    dcs: usize,
) {
    match dst_layout {
        ChannelLayout::Gray => {
            write_ch(dst, offset, c0, src_ty, dst_ty);
        }
        ChannelLayout::GrayAlpha => {
            write_ch(dst, offset, c0, src_ty, dst_ty);
            write_ch(dst, offset + dcs, a, src_ty, dst_ty);
        }
        ChannelLayout::Rgb => {
            write_ch(dst, offset, c0, src_ty, dst_ty);
            write_ch(dst, offset + dcs, c1, src_ty, dst_ty);
            write_ch(dst, offset + 2 * dcs, c2, src_ty, dst_ty);
        }
        // Rgba and Bgra: write 4 channels positionally
        _ => {
            write_ch(dst, offset, c0, src_ty, dst_ty);
            write_ch(dst, offset + dcs, c1, src_ty, dst_ty);
            write_ch(dst, offset + 2 * dcs, c2, src_ty, dst_ty);
            write_ch(dst, offset + 3 * dcs, a, src_ty, dst_ty);
        }
    }
}

/// Convert one row of pixels between formats.
fn convert_row(
    src: &[u8],
    dst: &mut [u8],
    width: usize,
    src_desc: &PixelDescriptor,
    dst_desc: &PixelDescriptor,
    gray_expand: GrayExpand,
) {
    let src_ty = src_desc.channel_type;
    let dst_ty = dst_desc.channel_type;
    let src_bpp = src_desc.bytes_per_pixel();
    let dst_bpp = dst_desc.bytes_per_pixel();
    let src_cs = src_ty.byte_size();
    let dst_cs = dst_ty.byte_size();

    for x in 0..width {
        let si = x * src_bpp;
        let di = x * dst_bpp;
        let (c0, c1, c2, a) = read_rgba(src, si, src_ty, src_desc.layout, src_cs, gray_expand);
        write_pixel(
            dst,
            di,
            c0,
            c1,
            c2,
            a,
            src_ty,
            dst_ty,
            dst_desc.layout,
            dst_cs,
        );
    }
}

// ── Alpha channel scanning ─────────────────────────────────────────────

/// Check if all alpha values in the source are fully opaque.
fn is_fully_opaque(src: &[u8], width: usize, height: usize, desc: &PixelDescriptor) -> bool {
    if !desc.layout.has_alpha() {
        return true;
    }
    let bpp = desc.bytes_per_pixel();
    let cs = desc.channel_type.byte_size();
    let alpha_offset = (desc.layout.channels() - 1) * cs;
    let max = max_value(desc.channel_type);
    for y in 0..height {
        for x in 0..width {
            let off = (y * width + x) * bpp + alpha_offset;
            if read_ch(src, off, desc.channel_type) != max {
                return false;
            }
        }
    }
    true
}

// ── Policy validation ─────────────────────────────────────────────────

/// Check that the requested conversion is allowed by the given policies.
fn validate_policies(
    src_desc: &PixelDescriptor,
    dst_layout: ChannelLayout,
    dst_ty: ChannelType,
    options: &ConvertOptions,
) -> Result<(), ConvertError> {
    // Alpha removal check
    let drops_alpha = src_desc.layout.has_alpha() && !dst_layout.has_alpha();
    if drops_alpha && options.alpha_policy == AlphaPolicy::Forbid {
        return Err(ConvertError::AlphaRemovalForbidden);
    }

    // Depth reduction check
    let reduces_depth = src_desc.channel_type == ChannelType::U16 && dst_ty == ChannelType::U8;
    if reduces_depth && options.depth_policy == DepthPolicy::Forbid {
        return Err(ConvertError::DepthReductionForbidden);
    }

    Ok(())
}

// ── PixelSlice conversion methods ───────────────────────────────────────

impl<P> PixelSlice<'_, P> {
    /// Convert pixel data to a different format in a single pass.
    ///
    /// Supports depth conversion (U8 ↔ U16), adding/dropping alpha,
    /// and grayscale-to-RGB expansion. RGB-to-grayscale is not supported.
    ///
    /// Returns a new tightly-packed [`PixelBuffer`] with the target format.
    /// **Allocates** a new buffer.
    /// Color metadata (transfer function, primaries, working space, color
    /// context) is preserved from the source.
    ///
    /// # Errors
    ///
    /// Returns [`ConvertError`] if the conversion is not supported.
    pub fn convert(
        &self,
        target_layout: ChannelLayout,
        target_depth: ChannelType,
        gray_expand: GrayExpand,
    ) -> Result<PixelBuffer, ConvertError> {
        let src_desc = self.descriptor();
        validate_conversion(
            src_desc.channel_type,
            src_desc.layout,
            target_depth,
            target_layout,
        )?;

        self.convert_inner(target_layout, target_depth, gray_expand)
    }

    /// Convert with explicit policies for all lossy operations. **Allocates**.
    ///
    /// Unlike [`convert()`](Self::convert), this method enforces policies on
    /// alpha removal and depth reduction, returning errors when forbidden.
    ///
    /// # Errors
    ///
    /// Returns [`ConvertError`] if:
    /// - The conversion is structurally unsupported (same as `convert()`)
    /// - Alpha removal is forbidden or pixels are not opaque with `DiscardIfOpaque`
    /// - Depth reduction is forbidden
    pub fn convert_explicit(
        &self,
        target_layout: ChannelLayout,
        target_depth: ChannelType,
        options: ConvertOptions,
    ) -> Result<PixelBuffer, ConvertError> {
        let src_desc = self.descriptor();
        validate_conversion(
            src_desc.channel_type,
            src_desc.layout,
            target_depth,
            target_layout,
        )?;
        validate_policies(&src_desc, target_layout, target_depth, &options)?;

        // Runtime opacity check for DiscardIfOpaque
        let drops_alpha = src_desc.layout.has_alpha() && !target_layout.has_alpha();
        if drops_alpha && options.alpha_policy == AlphaPolicy::DiscardIfOpaque {
            let w = self.width() as usize;
            let h = self.rows() as usize;
            // For strided buffers we need to check row by row
            let mut opaque = true;
            for y in 0..h as u32 {
                let row = self.row(y);
                if !is_fully_opaque(row, w, 1, &src_desc) {
                    opaque = false;
                    break;
                }
            }
            if !opaque {
                return Err(ConvertError::AlphaNotOpaque);
            }
        }

        self.convert_inner(target_layout, target_depth, options.gray_expand)
    }

    /// Add alpha channel (lossless). **Allocates** a new `PixelBuffer`.
    ///
    /// - Gray → GrayAlpha (opaque alpha)
    /// - Rgb → Rgba (opaque alpha)
    /// - Already has alpha → identity copy
    ///
    /// # Errors
    ///
    /// Returns [`ConvertError`] if the source uses an unsupported channel type.
    pub fn try_add_alpha(&self) -> Result<PixelBuffer, ConvertError> {
        let desc = self.descriptor();
        let target = match desc.layout {
            ChannelLayout::Gray => ChannelLayout::GrayAlpha,
            ChannelLayout::Rgb => ChannelLayout::Rgba,
            other => other,
        };
        self.convert(target, desc.channel_type, GrayExpand::Broadcast)
    }

    /// Widen to U16 depth (lossless, ×257). **Allocates** a new `PixelBuffer`.
    ///
    /// No-op copy if already U16.
    ///
    /// # Errors
    ///
    /// Returns [`ConvertError`] if the source uses an unsupported channel type.
    pub fn try_widen_to_u16(&self) -> Result<PixelBuffer, ConvertError> {
        let desc = self.descriptor();
        self.convert(desc.layout, ChannelType::U16, GrayExpand::Broadcast)
    }

    /// Narrow to U8 depth (lossy, rounded). **Allocates** a new `PixelBuffer`.
    ///
    /// U16 values are rounded: `(v * 255 + 32768) >> 16`.
    /// No-op copy if already U8.
    ///
    /// # Errors
    ///
    /// Returns [`ConvertError`] if the source uses an unsupported channel type.
    pub fn try_narrow_to_u8(&self, depth: DepthPolicy) -> Result<PixelBuffer, ConvertError> {
        let desc = self.descriptor();
        let reduces = desc.channel_type == ChannelType::U16;
        if reduces && depth == DepthPolicy::Forbid {
            return Err(ConvertError::DepthReductionForbidden);
        }
        self.convert(desc.layout, ChannelType::U8, GrayExpand::Broadcast)
    }

    /// Add an alpha channel. No-op copy if already has alpha.
    ///
    /// - Gray → GrayAlpha (opaque)
    /// - Rgb → Rgba (opaque)
    /// - GrayAlpha / Rgba / Bgra → identity copy
    ///
    /// # Panics
    ///
    /// Panics if the source uses an unsupported channel type (F32, F16, I16).
    #[deprecated(note = "use try_add_alpha() which returns Result")]
    pub fn to_with_alpha(&self) -> PixelBuffer {
        self.try_add_alpha()
            .expect("to_with_alpha: add-alpha conversion should not fail")
    }

    /// Widen to U16 depth. No-op copy if already U16.
    ///
    /// U8 values are scaled by ×257 (0→0, 128→32896, 255→65535).
    ///
    /// # Panics
    ///
    /// Panics if the source uses an unsupported channel type (F32, F16, I16).
    #[deprecated(note = "use try_widen_to_u16() which returns Result")]
    pub fn to_u16(&self) -> PixelBuffer {
        self.try_widen_to_u16()
            .expect("to_u16: depth conversion should not fail")
    }

    /// Narrow to U8 depth. No-op copy if already U8.
    ///
    /// U16 values are rounded: `(v * 255 + 32768) >> 16` (0→0, 32896→128, 65535→255).
    ///
    /// # Panics
    ///
    /// Panics if the source uses an unsupported channel type (F32, F16, I16).
    #[deprecated(note = "use try_narrow_to_u8() which returns Result")]
    pub fn to_u8(&self) -> PixelBuffer {
        self.try_narrow_to_u8(DepthPolicy::Round)
            .expect("to_u8: depth conversion should not fail")
    }

    // ── Internal ────────────────────────────────────────────────────────

    fn convert_inner(
        &self,
        target_layout: ChannelLayout,
        target_depth: ChannelType,
        gray_expand: GrayExpand,
    ) -> Result<PixelBuffer, ConvertError> {
        let src_desc = self.descriptor();
        let w = self.width() as usize;
        let h = self.rows() as usize;

        // Build target descriptor, preserving color metadata
        let dst_desc = PixelDescriptor {
            channel_type: target_depth,
            layout: target_layout,
            transfer: src_desc.transfer,
            primaries: src_desc.primaries,
            signal_range: src_desc.signal_range,
            alpha: if target_layout.has_alpha() {
                if src_desc.layout.has_alpha() {
                    src_desc.alpha
                } else {
                    Some(AlphaMode::Straight)
                }
            } else {
                None
            },
        };

        // Allocate output buffer (handles alignment internally)
        let mut buf = PixelBuffer::new(self.width(), self.rows(), dst_desc)
            .with_working_space(self.working_space());
        if let Some(ctx) = self.color_context() {
            buf = buf.with_color_context(Arc::clone(ctx));
        }

        // Write pixel data
        if h > 0 && w > 0 {
            let is_identity =
                src_desc.channel_type == target_depth && src_desc.layout == target_layout;
            let mut dst = buf.as_slice_mut();
            for y in 0..h as u32 {
                let src_row = self.row(y);
                let dst_row = dst.row_mut(y);
                if is_identity {
                    dst_row.copy_from_slice(src_row);
                } else {
                    convert_row(src_row, dst_row, w, &src_desc, &dst_desc, gray_expand);
                }
            }
        }

        Ok(buf)
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    fn make_slice(data: &[u8], width: u32, rows: u32, desc: PixelDescriptor) -> PixelSlice<'_> {
        let stride = desc.bytes_per_pixel() * width as usize;
        PixelSlice::new(data, width, rows, stride, desc).unwrap()
    }

    #[test]
    fn identity_rgb8() {
        let data = [1, 2, 3, 4, 5, 6];
        let s = make_slice(&data, 2, 1, PixelDescriptor::RGB8);
        let buf = s
            .convert(ChannelLayout::Rgb, ChannelType::U8, GrayExpand::Broadcast)
            .unwrap();
        assert_eq!(buf.as_contiguous_bytes().unwrap(), &data);
    }

    #[test]
    fn identity_rgba16() {
        let data: Vec<u8> = [100u16, 200, 300, 400]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let s = make_slice(&data, 1, 1, PixelDescriptor::RGBA16);
        let buf = s
            .convert(ChannelLayout::Rgba, ChannelType::U16, GrayExpand::Broadcast)
            .unwrap();
        assert_eq!(buf.as_contiguous_bytes().unwrap(), &data[..]);
    }

    #[test]
    fn u8_to_u16_gray() {
        let data = [100, 200];
        let s = make_slice(&data, 2, 1, PixelDescriptor::GRAY8);
        let buf = s.to_u16();
        let bytes = buf.as_contiguous_bytes().unwrap();
        let expected: Vec<u8> = [100u16 * 257, 200u16 * 257]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        assert_eq!(bytes, &expected[..]);
    }

    #[test]
    fn u16_to_u8_gray() {
        let data: Vec<u8> = [32896u16, 65535]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let s = make_slice(&data, 2, 1, PixelDescriptor::GRAY16);
        let buf = s.to_u8();
        let bytes = buf.as_contiguous_bytes().unwrap();
        assert_eq!(bytes, &[128, 255]);
    }

    #[test]
    fn rgb_to_rgba_add_alpha() {
        let data = [10, 20, 30, 40, 50, 60];
        let s = make_slice(&data, 2, 1, PixelDescriptor::RGB8);
        let buf = s.to_with_alpha();
        let bytes = buf.as_contiguous_bytes().unwrap();
        assert_eq!(bytes, &[10, 20, 30, 255, 40, 50, 60, 255]);
        assert_eq!(buf.descriptor().layout, ChannelLayout::Rgba);
        assert_eq!(buf.descriptor().alpha, Some(AlphaMode::Straight));
    }

    #[test]
    fn gray_to_grayalpha_add_alpha() {
        let data = [42, 99];
        let s = make_slice(&data, 2, 1, PixelDescriptor::GRAY8);
        let buf = s.to_with_alpha();
        let bytes = buf.as_contiguous_bytes().unwrap();
        assert_eq!(bytes, &[42, 255, 99, 255]);
        assert_eq!(buf.descriptor().layout, ChannelLayout::GrayAlpha);
    }

    #[test]
    fn rgba_drop_alpha() {
        let data = [10, 20, 30, 255, 40, 50, 60, 128];
        let s = make_slice(&data, 2, 1, PixelDescriptor::RGBA8);
        let buf = s
            .convert(ChannelLayout::Rgb, ChannelType::U8, GrayExpand::Broadcast)
            .unwrap();
        let bytes = buf.as_contiguous_bytes().unwrap();
        assert_eq!(bytes, &[10, 20, 30, 40, 50, 60]);
        assert_eq!(buf.descriptor().alpha, None);
    }

    #[test]
    fn gray_to_rgba_u16_combo() {
        // Gray U8 → RGBA U16: broadcast + add alpha + widen depth
        let data = [100];
        let s = make_slice(&data, 1, 1, PixelDescriptor::GRAY8);
        let buf = s
            .convert(ChannelLayout::Rgba, ChannelType::U16, GrayExpand::Broadcast)
            .unwrap();
        let bytes = buf.as_contiguous_bytes().unwrap();
        let v16 = 100u16 * 257;
        let expected: Vec<u8> = [v16, v16, v16, 65535]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        assert_eq!(bytes, &expected[..]);
    }

    #[test]
    fn grayalpha_to_rgba_broadcast() {
        // GrayAlpha U8 → RGBA U8: broadcast gray, keep alpha
        let data = [50, 200];
        let s = make_slice(&data, 1, 1, PixelDescriptor::GRAYA8);
        let buf = s
            .convert(ChannelLayout::Rgba, ChannelType::U8, GrayExpand::Broadcast)
            .unwrap();
        let bytes = buf.as_contiguous_bytes().unwrap();
        assert_eq!(bytes, &[50, 50, 50, 200]);
    }

    #[test]
    fn gray_to_rgb_broadcast() {
        let data = [77, 200];
        let s = make_slice(&data, 2, 1, PixelDescriptor::GRAY8);
        let buf = s
            .convert(ChannelLayout::Rgb, ChannelType::U8, GrayExpand::Broadcast)
            .unwrap();
        let bytes = buf.as_contiguous_bytes().unwrap();
        assert_eq!(bytes, &[77, 77, 77, 200, 200, 200]);
    }

    #[test]
    fn grayalpha_drop_alpha() {
        let data = [42, 128];
        let s = make_slice(&data, 1, 1, PixelDescriptor::GRAYA8);
        let buf = s
            .convert(ChannelLayout::Gray, ChannelType::U8, GrayExpand::Broadcast)
            .unwrap();
        let bytes = buf.as_contiguous_bytes().unwrap();
        assert_eq!(bytes, &[42]);
    }

    #[test]
    fn rgb_to_gray_rejected() {
        let data = [1, 2, 3];
        let s = make_slice(&data, 1, 1, PixelDescriptor::RGB8);
        let err = s
            .convert(ChannelLayout::Gray, ChannelType::U8, GrayExpand::Broadcast)
            .unwrap_err();
        assert_eq!(err, ConvertError::RgbToGray);
    }

    #[test]
    fn rgba_to_gray_rejected() {
        let data = [1, 2, 3, 4];
        let s = make_slice(&data, 1, 1, PixelDescriptor::RGBA8);
        let err = s
            .convert(
                ChannelLayout::GrayAlpha,
                ChannelType::U8,
                GrayExpand::Broadcast,
            )
            .unwrap_err();
        assert_eq!(err, ConvertError::RgbToGray);
    }

    #[test]
    fn bgra_depth_conversion() {
        // Bgra→Bgra with depth change is allowed (same layout, positional copy)
        let data = [10, 20, 30, 255]; // B=10, G=20, R=30, A=255
        let s = make_slice(&data, 1, 1, PixelDescriptor::BGRA8);
        let buf = s.to_u16();
        let bytes = buf.as_contiguous_bytes().unwrap();
        let expected: Vec<u8> = [10u16 * 257, 20u16 * 257, 30u16 * 257, 255u16 * 257]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        assert_eq!(bytes, &expected[..]);
    }

    #[test]
    fn bgra_cross_layout_rejected() {
        let data = [1, 2, 3, 4];
        let s = make_slice(&data, 1, 1, PixelDescriptor::BGRA8);
        let err = s
            .convert(ChannelLayout::Rgba, ChannelType::U8, GrayExpand::Broadcast)
            .unwrap_err();
        assert_eq!(err, ConvertError::UnsupportedLayout);
    }

    #[test]
    fn multi_row_with_stride() {
        // 2x2 RGB8 image
        let data = [
            1, 2, 3, 4, 5, 6, // row 0: pixels (1,2,3) and (4,5,6)
            7, 8, 9, 10, 11, 12, // row 1: pixels (7,8,9) and (10,11,12)
        ];
        let s = make_slice(&data, 2, 2, PixelDescriptor::RGB8);
        let buf = s.to_with_alpha();
        let bytes = buf.as_contiguous_bytes().unwrap();
        assert_eq!(
            bytes,
            &[
                1, 2, 3, 255, 4, 5, 6, 255, // row 0
                7, 8, 9, 255, 10, 11, 12, 255, // row 1
            ]
        );
    }

    #[test]
    fn preserves_metadata() {
        use crate::buffer::{ColorPrimaries, SignalRange, TransferFunction};
        use crate::color::WorkingColorSpace;

        let data = [42];
        let desc = PixelDescriptor {
            channel_type: ChannelType::U8,
            layout: ChannelLayout::Gray,
            transfer: TransferFunction::Srgb,
            primaries: ColorPrimaries::Bt709,
            signal_range: SignalRange::Full,
            alpha: None,
        };
        let s = make_slice(&data, 1, 1, desc).with_working_space(WorkingColorSpace::LinearSrgb);
        let buf = s.to_u16();
        assert_eq!(buf.descriptor().transfer, TransferFunction::Srgb);
        assert_eq!(buf.descriptor().primaries, ColorPrimaries::Bt709);
        assert_eq!(buf.descriptor().signal_range, SignalRange::Full);
        assert_eq!(buf.working_space(), WorkingColorSpace::LinearSrgb);
    }

    #[test]
    fn empty_image() {
        let data = [];
        let desc = PixelDescriptor::RGB8;
        let stride = desc.bytes_per_pixel() * 0;
        let s = PixelSlice::new(&data, 0, 0, stride, desc).unwrap();
        let buf = s.to_u16();
        assert_eq!(buf.width(), 0);
        assert_eq!(buf.height(), 0);
    }

    #[test]
    fn u16_roundtrip() {
        // U8→U16→U8 should preserve values (×257 then rounded)
        let data = [0, 1, 127, 128, 254, 255];
        let s = make_slice(&data, 6, 1, PixelDescriptor::GRAY8);
        let wide = s.to_u16();
        let narrow = wide.as_slice().to_u8();
        let bytes = narrow.as_contiguous_bytes().unwrap();
        assert_eq!(bytes, &data);
    }

    #[test]
    fn u16_roundtrip_all_u8_values() {
        // Verify U8→U16→U8 roundtrip for all 256 values
        let data: Vec<u8> = (0..=255).collect();
        let s = make_slice(&data, 256, 1, PixelDescriptor::GRAY8);
        let wide = s.to_u16();
        let narrow = wide.as_slice().to_u8();
        let bytes = narrow.as_contiguous_bytes().unwrap();
        assert_eq!(bytes, &data[..]);
    }

    #[test]
    fn u16_to_u8_rounding_not_truncation() {
        // 32767 is exactly halfway between 127 and 128 in U8 space.
        // With truncation (>>8): 32767 >> 8 = 127
        // With rounding: (32767 * 255 + 32768) >> 16 = 127 (correct: 32767/257 = 127.498)
        // Value 32896 = 128*257, should map to 128 exactly.
        // Value 32895 = 128*257 - 1, should map to 128 (rounds up from 127.996).
        let data: Vec<u8> = [32767u16, 32895, 32896, 33153]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let s = make_slice(&data, 4, 1, PixelDescriptor::GRAY16);
        let buf = s.to_u8();
        let bytes = buf.as_contiguous_bytes().unwrap();
        // 32767 / 257.0 = 127.498 → rounds to 127
        assert_eq!(bytes[0], 127);
        // 32895 / 257.0 = 127.996 → rounds to 128
        assert_eq!(bytes[1], 128);
        // 32896 / 257.0 = 128.0 → exactly 128
        assert_eq!(bytes[2], 128);
        // 33153 / 257.0 = 129.0 → exactly 129
        assert_eq!(bytes[3], 129);
    }

    // ── convert_explicit tests ──────────────────────────────────────────

    #[test]
    fn convert_explicit_forbid_alpha_removal() {
        let data = [10, 20, 30, 255];
        let s = make_slice(&data, 1, 1, PixelDescriptor::RGBA8);
        let err = s
            .convert_explicit(
                ChannelLayout::Rgb,
                ChannelType::U8,
                ConvertOptions {
                    gray_expand: GrayExpand::Broadcast,
                    alpha_policy: AlphaPolicy::Forbid,
                    depth_policy: DepthPolicy::Round,
                },
            )
            .unwrap_err();
        assert_eq!(err, ConvertError::AlphaRemovalForbidden);
    }

    #[test]
    fn convert_explicit_discard_if_opaque_succeeds() {
        let data = [10, 20, 30, 255, 40, 50, 60, 255];
        let s = make_slice(&data, 2, 1, PixelDescriptor::RGBA8);
        let buf = s
            .convert_explicit(
                ChannelLayout::Rgb,
                ChannelType::U8,
                ConvertOptions {
                    gray_expand: GrayExpand::Broadcast,
                    alpha_policy: AlphaPolicy::DiscardIfOpaque,
                    depth_policy: DepthPolicy::Round,
                },
            )
            .unwrap();
        assert_eq!(
            buf.as_contiguous_bytes().unwrap(),
            &[10, 20, 30, 40, 50, 60]
        );
    }

    #[test]
    fn convert_explicit_discard_if_opaque_fails() {
        let data = [10, 20, 30, 128]; // alpha = 128, not opaque
        let s = make_slice(&data, 1, 1, PixelDescriptor::RGBA8);
        let err = s
            .convert_explicit(
                ChannelLayout::Rgb,
                ChannelType::U8,
                ConvertOptions {
                    gray_expand: GrayExpand::Broadcast,
                    alpha_policy: AlphaPolicy::DiscardIfOpaque,
                    depth_policy: DepthPolicy::Round,
                },
            )
            .unwrap_err();
        assert_eq!(err, ConvertError::AlphaNotOpaque);
    }

    #[test]
    fn convert_explicit_discard_unchecked() {
        let data = [10, 20, 30, 128]; // alpha = 128
        let s = make_slice(&data, 1, 1, PixelDescriptor::RGBA8);
        let buf = s
            .convert_explicit(
                ChannelLayout::Rgb,
                ChannelType::U8,
                ConvertOptions {
                    gray_expand: GrayExpand::Broadcast,
                    alpha_policy: AlphaPolicy::DiscardUnchecked,
                    depth_policy: DepthPolicy::Round,
                },
            )
            .unwrap();
        assert_eq!(buf.as_contiguous_bytes().unwrap(), &[10, 20, 30]);
    }

    #[test]
    fn convert_explicit_forbid_depth_reduction() {
        let data: Vec<u8> = [32896u16].iter().flat_map(|v| v.to_ne_bytes()).collect();
        let s = make_slice(&data, 1, 1, PixelDescriptor::GRAY16);
        let err = s
            .convert_explicit(
                ChannelLayout::Gray,
                ChannelType::U8,
                ConvertOptions {
                    gray_expand: GrayExpand::Broadcast,
                    alpha_policy: AlphaPolicy::DiscardUnchecked,
                    depth_policy: DepthPolicy::Forbid,
                },
            )
            .unwrap_err();
        assert_eq!(err, ConvertError::DepthReductionForbidden);
    }

    #[test]
    fn try_add_alpha_returns_result() {
        let data = [10, 20, 30];
        let s = make_slice(&data, 1, 1, PixelDescriptor::RGB8);
        let buf = s.try_add_alpha().unwrap();
        assert_eq!(buf.as_contiguous_bytes().unwrap(), &[10, 20, 30, 255]);
    }

    #[test]
    fn try_widen_to_u16_returns_result() {
        let data = [100];
        let s = make_slice(&data, 1, 1, PixelDescriptor::GRAY8);
        let buf = s.try_widen_to_u16().unwrap();
        let bytes = buf.as_contiguous_bytes().unwrap();
        let expected: Vec<u8> = [100u16 * 257]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        assert_eq!(bytes, &expected[..]);
    }

    #[test]
    fn try_narrow_to_u8_round() {
        let data: Vec<u8> = [32896u16, 65535]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let s = make_slice(&data, 2, 1, PixelDescriptor::GRAY16);
        let buf = s.try_narrow_to_u8(DepthPolicy::Round).unwrap();
        assert_eq!(buf.as_contiguous_bytes().unwrap(), &[128, 255]);
    }

    #[test]
    fn try_narrow_to_u8_forbid() {
        let data: Vec<u8> = [32896u16].iter().flat_map(|v| v.to_ne_bytes()).collect();
        let s = make_slice(&data, 1, 1, PixelDescriptor::GRAY16);
        let err = s.try_narrow_to_u8(DepthPolicy::Forbid).unwrap_err();
        assert_eq!(err, ConvertError::DepthReductionForbidden);
    }

    #[test]
    fn try_narrow_to_u8_noop_on_u8() {
        let data = [42, 99];
        let s = make_slice(&data, 2, 1, PixelDescriptor::GRAY8);
        // Forbid should still succeed when no actual reduction needed
        let buf = s.try_narrow_to_u8(DepthPolicy::Forbid).unwrap();
        assert_eq!(buf.as_contiguous_bytes().unwrap(), &[42, 99]);
    }
}
