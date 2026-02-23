//! Typed pixel buffer definitions.
//!
//! Uses `imgref::ImgVec` for 2D pixel data with typed pixels from the `rgb` crate.
//!
//! `PixelData` is a data container — it holds pixels and describes their layout,
//! but does **not** convert between formats. Format conversion requires
//! transfer function awareness that belongs in a dedicated conversion crate.
//! Use [`Decoder::decode_into()`](crate::Decoder::decode_into) to request
//! a specific format from the codec, which can convert correctly.

use alloc::vec::Vec;
use imgref::ImgVec;
use rgb::alt::BGRA;
use rgb::{Gray, Rgb, Rgba};

/// Grayscale pixel with alpha channel.
///
/// A simple two-component pixel type. Not from the `rgb` crate — we own this
/// type to avoid API instability in `rgb::alt::GrayAlpha`.
///
/// # Zero-copy limitation
///
/// `GrayAlpha<T>` does not implement `rgb::ComponentBytes`, so it cannot
/// produce a zero-copy [`PixelSlice`](crate::PixelSlice) via `From`. Use
/// [`PixelData::as_pixel_slice()`](crate::PixelData::as_pixel_slice) for
/// non-GrayAlpha variants, or [`PixelData::to_bytes()`](crate::PixelData::to_bytes)
/// for the buffer-copy path. The typed convenience methods on
/// [`EncoderConfig`](crate::EncoderConfig) (`encode_gray_alpha8` etc.) handle
/// this internally by routing through [`PixelData`](crate::PixelData).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(C)]
pub struct GrayAlpha<T> {
    /// Gray value.
    pub v: T,
    /// Alpha value.
    pub a: T,
}

impl<T> GrayAlpha<T> {
    /// Create a new gray+alpha pixel.
    pub const fn new(v: T, a: T) -> Self {
        Self { v, a }
    }
}

/// Decoded pixel data in a typed buffer.
///
/// The variant determines both the pixel format and precision.
/// Width and height are embedded in the `ImgVec`.
///
/// # Transfer function
///
/// `PixelData` does not track its transfer function — that metadata lives in
/// [`ImageInfo::cicp`](crate::ImageInfo) (or the ICC profile). The actual
/// transfer function depends on how the data was produced (codec-specific).
///
/// If you need a specific pixel format, use
/// [`Decoder::decode_into()`](crate::Decoder::decode_into) to request it
/// from the codec. Codecs can convert correctly because they know the source
/// transfer function. Post-hoc conversion between depth classes (u8/u16 ↔ f32)
/// requires transfer function math that this crate intentionally does not provide.
///
/// u16 variants use the full 0–65535 range regardless of source bit depth
/// (e.g. 10-bit AVIF values are scaled up, not left in 0–1023).
#[non_exhaustive]
pub enum PixelData {
    /// 8-bit RGB.
    Rgb8(ImgVec<Rgb<u8>>),
    /// 8-bit RGBA.
    Rgba8(ImgVec<Rgba<u8>>),
    /// 16-bit RGB. Full 0–65535 range regardless of source bit depth.
    Rgb16(ImgVec<Rgb<u16>>),
    /// 16-bit RGBA. Full 0–65535 range regardless of source bit depth.
    Rgba16(ImgVec<Rgba<u16>>),
    /// RGB f32. Values in [0.0, 1.0] for SDR; may exceed 1.0 for HDR.
    RgbF32(ImgVec<Rgb<f32>>),
    /// RGBA f32. Values in [0.0, 1.0] for SDR; may exceed 1.0 for HDR.
    RgbaF32(ImgVec<Rgba<f32>>),
    /// 8-bit grayscale.
    Gray8(ImgVec<Gray<u8>>),
    /// 16-bit grayscale. Full 0–65535 range regardless of source bit depth.
    Gray16(ImgVec<Gray<u16>>),
    /// Grayscale f32. Values in [0.0, 1.0] for SDR; may exceed 1.0 for HDR.
    GrayF32(ImgVec<Gray<f32>>),
    /// 8-bit BGRA (blue, green, red, alpha byte order).
    ///
    /// Native byte order for Windows/DirectX surfaces.
    Bgra8(ImgVec<BGRA<u8>>),
    /// 8-bit grayscale with alpha.
    GrayAlpha8(ImgVec<GrayAlpha<u8>>),
    /// 16-bit grayscale with alpha. Full 0–65535 range regardless of source bit depth.
    GrayAlpha16(ImgVec<GrayAlpha<u16>>),
    /// Grayscale + alpha f32.
    GrayAlphaF32(ImgVec<GrayAlpha<f32>>),
}

impl PixelData {
    /// Pixel format descriptor for this variant.
    ///
    /// Returns a descriptor based on the channel type and layout. The transfer
    /// function is [`Unknown`](crate::TransferFunction::Unknown) because
    /// `PixelData` does not track its transfer function — that metadata lives
    /// in [`ImageInfo::cicp`](crate::ImageInfo) or the ICC profile.
    ///
    /// To get a descriptor with the correct transfer function, resolve it
    /// from CICP metadata:
    ///
    /// ```ignore
    /// let desc = pixels.descriptor();
    /// let tf = info.transfer_function(); // derives from CICP
    /// let resolved = desc.with_transfer(tf);
    /// ```
    pub fn descriptor(&self) -> crate::buffer::PixelDescriptor {
        use crate::buffer::PixelDescriptor;
        match self {
            PixelData::Rgb8(_) => PixelDescriptor::RGB8,
            PixelData::Rgba8(_) => PixelDescriptor::RGBA8,
            PixelData::Rgb16(_) => PixelDescriptor::RGB16,
            PixelData::Rgba16(_) => PixelDescriptor::RGBA16,
            PixelData::RgbF32(_) => PixelDescriptor::RGBF32,
            PixelData::RgbaF32(_) => PixelDescriptor::RGBAF32,
            PixelData::Gray8(_) => PixelDescriptor::GRAY8,
            PixelData::Gray16(_) => PixelDescriptor::GRAY16,
            PixelData::GrayF32(_) => PixelDescriptor::GRAYF32,
            PixelData::Bgra8(_) => PixelDescriptor::BGRA8,
            PixelData::GrayAlpha8(_) => PixelDescriptor::GRAYA8,
            PixelData::GrayAlpha16(_) => PixelDescriptor::GRAYA16,
            PixelData::GrayAlphaF32(_) => PixelDescriptor::GRAYAF32,
        }
    }

    /// Image width in pixels.
    pub fn width(&self) -> u32 {
        match self {
            PixelData::Rgb8(img) => img.width() as u32,
            PixelData::Rgba8(img) => img.width() as u32,
            PixelData::Rgb16(img) => img.width() as u32,
            PixelData::Rgba16(img) => img.width() as u32,
            PixelData::RgbF32(img) => img.width() as u32,
            PixelData::RgbaF32(img) => img.width() as u32,
            PixelData::Gray8(img) => img.width() as u32,
            PixelData::Gray16(img) => img.width() as u32,
            PixelData::GrayF32(img) => img.width() as u32,
            PixelData::Bgra8(img) => img.width() as u32,
            PixelData::GrayAlpha8(img) => img.width() as u32,
            PixelData::GrayAlpha16(img) => img.width() as u32,
            PixelData::GrayAlphaF32(img) => img.width() as u32,
        }
    }

    /// Image height in pixels.
    pub fn height(&self) -> u32 {
        match self {
            PixelData::Rgb8(img) => img.height() as u32,
            PixelData::Rgba8(img) => img.height() as u32,
            PixelData::Rgb16(img) => img.height() as u32,
            PixelData::Rgba16(img) => img.height() as u32,
            PixelData::RgbF32(img) => img.height() as u32,
            PixelData::RgbaF32(img) => img.height() as u32,
            PixelData::Gray8(img) => img.height() as u32,
            PixelData::Gray16(img) => img.height() as u32,
            PixelData::GrayF32(img) => img.height() as u32,
            PixelData::Bgra8(img) => img.height() as u32,
            PixelData::GrayAlpha8(img) => img.height() as u32,
            PixelData::GrayAlpha16(img) => img.height() as u32,
            PixelData::GrayAlphaF32(img) => img.height() as u32,
        }
    }

    /// Whether this pixel data has an alpha channel.
    pub fn has_alpha(&self) -> bool {
        matches!(
            self,
            PixelData::Rgba8(_)
                | PixelData::Rgba16(_)
                | PixelData::RgbaF32(_)
                | PixelData::Bgra8(_)
                | PixelData::GrayAlpha8(_)
                | PixelData::GrayAlpha16(_)
                | PixelData::GrayAlphaF32(_)
        )
    }

    /// Borrow pixel data as a [`PixelSlice`](crate::buffer::PixelSlice).
    ///
    /// Returns `None` for GrayAlpha variants (our `GrayAlpha<T>` type
    /// doesn't implement `rgb::ComponentBytes`, so we can't get a byte
    /// slice without copying).
    pub fn as_pixel_slice(&self) -> Option<crate::buffer::PixelSlice<'_>> {
        // The From<ImgRef> impls use convention-based descriptors (sRGB for u8,
        // linear for f32). Override with self.descriptor() which preserves the
        // transfer-agnostic Unknown from decoded pixel data.
        let desc = self.descriptor();
        let slice: crate::buffer::PixelSlice<'_> = match self {
            PixelData::Rgb8(img) => img.as_ref().into(),
            PixelData::Rgba8(img) => img.as_ref().into(),
            PixelData::Rgb16(img) => img.as_ref().into(),
            PixelData::Rgba16(img) => img.as_ref().into(),
            PixelData::RgbF32(img) => img.as_ref().into(),
            PixelData::RgbaF32(img) => img.as_ref().into(),
            PixelData::Gray8(img) => img.as_ref().into(),
            PixelData::Gray16(img) => img.as_ref().into(),
            PixelData::GrayF32(img) => img.as_ref().into(),
            PixelData::Bgra8(img) => img.as_ref().into(),
            // GrayAlpha types don't implement ComponentBytes
            PixelData::GrayAlpha8(_) | PixelData::GrayAlpha16(_) | PixelData::GrayAlphaF32(_) => {
                return None;
            }
        };
        Some(slice.with_descriptor(desc))
    }

    /// Get the raw pixel data as a byte vector.
    ///
    /// Returns the raw bytes of the pixel buffer in its native format.
    /// No format conversion is performed.
    pub fn to_bytes(&self) -> Vec<u8> {
        use rgb::ComponentBytes;
        match self {
            PixelData::Rgb8(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::Rgba8(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::Rgb16(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::Rgba16(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::RgbF32(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::RgbaF32(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::Gray8(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::Gray16(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::GrayF32(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::Bgra8(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.as_bytes().to_vec()
            }
            PixelData::GrayAlpha8(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.iter().flat_map(|p| [p.v, p.a]).collect()
            }
            PixelData::GrayAlpha16(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.iter()
                    .flat_map(|p| {
                        let mut b = [0u8; 4];
                        b[..2].copy_from_slice(&p.v.to_ne_bytes());
                        b[2..].copy_from_slice(&p.a.to_ne_bytes());
                        b
                    })
                    .collect()
            }
            PixelData::GrayAlphaF32(img) => {
                let (buf, _, _) = img.as_ref().to_contiguous_buf();
                buf.iter()
                    .flat_map(|p| {
                        let mut b = [0u8; 8];
                        b[..4].copy_from_slice(&p.v.to_ne_bytes());
                        b[4..].copy_from_slice(&p.a.to_ne_bytes());
                        b
                    })
                    .collect()
            }
        }
    }

    /// Whether this pixel data is grayscale (no color channels).
    pub fn is_grayscale(&self) -> bool {
        matches!(
            self,
            PixelData::Gray8(_)
                | PixelData::Gray16(_)
                | PixelData::GrayF32(_)
                | PixelData::GrayAlpha8(_)
                | PixelData::GrayAlpha16(_)
                | PixelData::GrayAlphaF32(_)
        )
    }

    // --- Zero-copy borrowing accessors (as_*) ---
    //
    // Return `None` if the variant doesn't match. No conversion.

    /// Borrow as RGB8 if that's the native format.
    pub fn as_rgb8(&self) -> Option<imgref::ImgRef<'_, Rgb<u8>>> {
        match self {
            PixelData::Rgb8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as RGBA8 if that's the native format.
    pub fn as_rgba8(&self) -> Option<imgref::ImgRef<'_, Rgba<u8>>> {
        match self {
            PixelData::Rgba8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as BGRA8 if that's the native format.
    pub fn as_bgra8(&self) -> Option<imgref::ImgRef<'_, BGRA<u8>>> {
        match self {
            PixelData::Bgra8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as Gray8 if that's the native format.
    pub fn as_gray8(&self) -> Option<imgref::ImgRef<'_, Gray<u8>>> {
        match self {
            PixelData::Gray8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as RGB16 if that's the native format.
    pub fn as_rgb16(&self) -> Option<imgref::ImgRef<'_, Rgb<u16>>> {
        match self {
            PixelData::Rgb16(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as RGBA16 if that's the native format.
    pub fn as_rgba16(&self) -> Option<imgref::ImgRef<'_, Rgba<u16>>> {
        match self {
            PixelData::Rgba16(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as RGB f32 if that's the native format.
    pub fn as_rgb_f32(&self) -> Option<imgref::ImgRef<'_, Rgb<f32>>> {
        match self {
            PixelData::RgbF32(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as RGBA f32 if that's the native format.
    pub fn as_rgba_f32(&self) -> Option<imgref::ImgRef<'_, Rgba<f32>>> {
        match self {
            PixelData::RgbaF32(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as Gray16 if that's the native format.
    pub fn as_gray16(&self) -> Option<imgref::ImgRef<'_, Gray<u16>>> {
        match self {
            PixelData::Gray16(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as Gray f32 if that's the native format.
    pub fn as_gray_f32(&self) -> Option<imgref::ImgRef<'_, Gray<f32>>> {
        match self {
            PixelData::GrayF32(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as GrayAlpha8 if that's the native format.
    pub fn as_gray_alpha8(&self) -> Option<imgref::ImgRef<'_, GrayAlpha<u8>>> {
        match self {
            PixelData::GrayAlpha8(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as GrayAlpha16 if that's the native format.
    pub fn as_gray_alpha16(&self) -> Option<imgref::ImgRef<'_, GrayAlpha<u16>>> {
        match self {
            PixelData::GrayAlpha16(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    /// Borrow as GrayAlpha f32 if that's the native format.
    pub fn as_gray_alpha_f32(&self) -> Option<imgref::ImgRef<'_, GrayAlpha<f32>>> {
        match self {
            PixelData::GrayAlphaF32(img) => Some(img.as_ref()),
            _ => None,
        }
    }

    // --- Consuming extractors (try_into_*) ---
    //
    // Return `None` if the variant doesn't match. No conversion.

    /// Extract as owned RGB8 without conversion.
    pub fn try_into_rgb8(self) -> Option<ImgVec<Rgb<u8>>> {
        match self {
            PixelData::Rgb8(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned RGBA8 without conversion.
    pub fn try_into_rgba8(self) -> Option<ImgVec<Rgba<u8>>> {
        match self {
            PixelData::Rgba8(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned BGRA8 without conversion.
    pub fn try_into_bgra8(self) -> Option<ImgVec<BGRA<u8>>> {
        match self {
            PixelData::Bgra8(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned Gray8 without conversion.
    pub fn try_into_gray8(self) -> Option<ImgVec<Gray<u8>>> {
        match self {
            PixelData::Gray8(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned RGB16 without conversion.
    pub fn try_into_rgb16(self) -> Option<ImgVec<Rgb<u16>>> {
        match self {
            PixelData::Rgb16(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned RGBA16 without conversion.
    pub fn try_into_rgba16(self) -> Option<ImgVec<Rgba<u16>>> {
        match self {
            PixelData::Rgba16(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned RGB f32 without conversion.
    pub fn try_into_rgb_f32(self) -> Option<ImgVec<Rgb<f32>>> {
        match self {
            PixelData::RgbF32(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned RGBA f32 without conversion.
    pub fn try_into_rgba_f32(self) -> Option<ImgVec<Rgba<f32>>> {
        match self {
            PixelData::RgbaF32(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned Gray16 without conversion.
    pub fn try_into_gray16(self) -> Option<ImgVec<Gray<u16>>> {
        match self {
            PixelData::Gray16(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned Gray f32 without conversion.
    pub fn try_into_gray_f32(self) -> Option<ImgVec<Gray<f32>>> {
        match self {
            PixelData::GrayF32(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned GrayAlpha8 without conversion.
    pub fn try_into_gray_alpha8(self) -> Option<ImgVec<GrayAlpha<u8>>> {
        match self {
            PixelData::GrayAlpha8(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned GrayAlpha16 without conversion.
    pub fn try_into_gray_alpha16(self) -> Option<ImgVec<GrayAlpha<u16>>> {
        match self {
            PixelData::GrayAlpha16(img) => Some(img),
            _ => None,
        }
    }

    /// Extract as owned GrayAlpha f32 without conversion.
    pub fn try_into_gray_alpha_f32(self) -> Option<ImgVec<GrayAlpha<f32>>> {
        match self {
            PixelData::GrayAlphaF32(img) => Some(img),
            _ => None,
        }
    }

    /// Convert to RGB8 by reference, allocating a new buffer.
    ///
    /// 16-bit values are downscaled with proper rounding. Float values are
    /// clamped to [0.0, 1.0]. Gray is expanded with R=G=B. RGBA/BGRA variants
    /// discard alpha.
    pub fn to_rgb8(&self) -> ImgVec<Rgb<u8>> {
        match self {
            PixelData::Rgb8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                ImgVec::new(buf.into_owned(), w, h)
            }
            PixelData::Rgba8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| Rgb {
                        r: p.r,
                        g: p.g,
                        b: p.b,
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::Gray8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| {
                        let v = p.value();
                        Rgb { r: v, g: v, b: v }
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::Gray16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| {
                        let v = u16_to_u8(p.value());
                        Rgb { r: v, g: v, b: v }
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::GrayF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| {
                        let v = (p.value().clamp(0.0, 1.0) * 255.0) as u8;
                        Rgb { r: v, g: v, b: v }
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::Rgb16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| Rgb {
                        r: u16_to_u8(p.r),
                        g: u16_to_u8(p.g),
                        b: u16_to_u8(p.b),
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::Rgba16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| Rgb {
                        r: u16_to_u8(p.r),
                        g: u16_to_u8(p.g),
                        b: u16_to_u8(p.b),
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::RgbF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| Rgb {
                        r: (p.r.clamp(0.0, 1.0) * 255.0) as u8,
                        g: (p.g.clamp(0.0, 1.0) * 255.0) as u8,
                        b: (p.b.clamp(0.0, 1.0) * 255.0) as u8,
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::RgbaF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| Rgb {
                        r: (p.r.clamp(0.0, 1.0) * 255.0) as u8,
                        g: (p.g.clamp(0.0, 1.0) * 255.0) as u8,
                        b: (p.b.clamp(0.0, 1.0) * 255.0) as u8,
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::Bgra8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| Rgb {
                        r: p.r,
                        g: p.g,
                        b: p.b,
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::GrayAlpha8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| {
                        let v = p.v;
                        Rgb { r: v, g: v, b: v }
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::GrayAlpha16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| {
                        let v = u16_to_u8(p.v);
                        Rgb { r: v, g: v, b: v }
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
            PixelData::GrayAlphaF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgb: Vec<Rgb<u8>> = buf
                    .iter()
                    .map(|p| {
                        let v = (p.v.clamp(0.0, 1.0) * 255.0) as u8;
                        Rgb { r: v, g: v, b: v }
                    })
                    .collect();
                ImgVec::new(rgb, w, h)
            }
        }
    }

    /// Convert to RGB8, consuming self.
    ///
    /// Avoids a clone when the data is already Rgb8.
    pub fn into_rgb8(self) -> ImgVec<Rgb<u8>> {
        match self {
            PixelData::Rgb8(img) => img,
            other => other.to_rgb8(),
        }
    }

    /// Convert to RGBA8 by reference, allocating a new buffer.
    ///
    /// Gray is expanded with R=G=B, A=255. RGB gets A=255 added.
    /// 16-bit values are downscaled with proper rounding.
    /// Float values are clamped to [0.0, 1.0].
    pub fn to_rgba8(&self) -> ImgVec<Rgba<u8>> {
        match self {
            PixelData::Rgba8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                ImgVec::new(buf.into_owned(), w, h)
            }
            PixelData::Rgb8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| Rgba {
                        r: p.r,
                        g: p.g,
                        b: p.b,
                        a: 255,
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::Gray8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| {
                        let v = p.value();
                        Rgba {
                            r: v,
                            g: v,
                            b: v,
                            a: 255,
                        }
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::Gray16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| {
                        let v = u16_to_u8(p.value());
                        Rgba {
                            r: v,
                            g: v,
                            b: v,
                            a: 255,
                        }
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::GrayF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| {
                        let v = (p.value().clamp(0.0, 1.0) * 255.0) as u8;
                        Rgba {
                            r: v,
                            g: v,
                            b: v,
                            a: 255,
                        }
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::Rgba16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| Rgba {
                        r: u16_to_u8(p.r),
                        g: u16_to_u8(p.g),
                        b: u16_to_u8(p.b),
                        a: u16_to_u8(p.a),
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::Rgb16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| Rgba {
                        r: u16_to_u8(p.r),
                        g: u16_to_u8(p.g),
                        b: u16_to_u8(p.b),
                        a: 255,
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::RgbF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| Rgba {
                        r: (p.r.clamp(0.0, 1.0) * 255.0) as u8,
                        g: (p.g.clamp(0.0, 1.0) * 255.0) as u8,
                        b: (p.b.clamp(0.0, 1.0) * 255.0) as u8,
                        a: 255,
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::RgbaF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| Rgba {
                        r: (p.r.clamp(0.0, 1.0) * 255.0) as u8,
                        g: (p.g.clamp(0.0, 1.0) * 255.0) as u8,
                        b: (p.b.clamp(0.0, 1.0) * 255.0) as u8,
                        a: (p.a.clamp(0.0, 1.0) * 255.0) as u8,
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::Bgra8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| Rgba {
                        r: p.r,
                        g: p.g,
                        b: p.b,
                        a: p.a,
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::GrayAlpha8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| Rgba {
                        r: p.v,
                        g: p.v,
                        b: p.v,
                        a: p.a,
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::GrayAlpha16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| Rgba {
                        r: u16_to_u8(p.v),
                        g: u16_to_u8(p.v),
                        b: u16_to_u8(p.v),
                        a: u16_to_u8(p.a),
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
            PixelData::GrayAlphaF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let rgba: Vec<Rgba<u8>> = buf
                    .iter()
                    .map(|p| Rgba {
                        r: (p.v.clamp(0.0, 1.0) * 255.0) as u8,
                        g: (p.v.clamp(0.0, 1.0) * 255.0) as u8,
                        b: (p.v.clamp(0.0, 1.0) * 255.0) as u8,
                        a: (p.a.clamp(0.0, 1.0) * 255.0) as u8,
                    })
                    .collect();
                ImgVec::new(rgba, w, h)
            }
        }
    }

    /// Convert to RGBA8, consuming self.
    ///
    /// Avoids a clone when the data is already Rgba8.
    pub fn into_rgba8(self) -> ImgVec<Rgba<u8>> {
        match self {
            PixelData::Rgba8(img) => img,
            other => other.to_rgba8(),
        }
    }

    /// Convert to Gray8 by reference, allocating a new buffer.
    ///
    /// RGB variants use BT.601 luminance (0.299R + 0.587G + 0.114B).
    /// RGBA/BGRA ignore alpha. 16-bit values are downscaled with proper rounding.
    /// Float values are clamped to [0.0, 1.0].
    pub fn to_gray8(&self) -> ImgVec<Gray<u8>> {
        match self {
            PixelData::Gray8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                ImgVec::new(buf.into_owned(), w, h)
            }
            PixelData::Gray16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let gray: Vec<Gray<u8>> = buf
                    .iter()
                    .map(|p| Gray::new(u16_to_u8(p.value())))
                    .collect();
                ImgVec::new(gray, w, h)
            }
            PixelData::GrayF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let gray: Vec<Gray<u8>> = buf
                    .iter()
                    .map(|p| Gray::new((p.value().clamp(0.0, 1.0) * 255.0) as u8))
                    .collect();
                ImgVec::new(gray, w, h)
            }
            PixelData::GrayAlpha8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let gray: Vec<Gray<u8>> = buf.iter().map(|p| Gray::new(p.v)).collect();
                ImgVec::new(gray, w, h)
            }
            PixelData::GrayAlpha16(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let gray: Vec<Gray<u8>> = buf.iter().map(|p| Gray::new(u16_to_u8(p.v))).collect();
                ImgVec::new(gray, w, h)
            }
            PixelData::GrayAlphaF32(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let gray: Vec<Gray<u8>> = buf
                    .iter()
                    .map(|p| Gray::new((p.v.clamp(0.0, 1.0) * 255.0) as u8))
                    .collect();
                ImgVec::new(gray, w, h)
            }
            PixelData::Rgb8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let gray: Vec<Gray<u8>> = buf
                    .iter()
                    .map(|p| Gray::new(rgb_to_luma(p.r, p.g, p.b)))
                    .collect();
                ImgVec::new(gray, w, h)
            }
            PixelData::Rgba8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let gray: Vec<Gray<u8>> = buf
                    .iter()
                    .map(|p| Gray::new(rgb_to_luma(p.r, p.g, p.b)))
                    .collect();
                ImgVec::new(gray, w, h)
            }
            PixelData::Bgra8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let gray: Vec<Gray<u8>> = buf
                    .iter()
                    .map(|p| Gray::new(rgb_to_luma(p.r, p.g, p.b)))
                    .collect();
                ImgVec::new(gray, w, h)
            }
            other => {
                // Fall back through Rgb8 for remaining formats.
                let rgb = other.to_rgb8();
                let (buf, w, h) = rgb.as_ref().to_contiguous_buf();
                let gray: Vec<Gray<u8>> = buf
                    .iter()
                    .map(|p| Gray::new(rgb_to_luma(p.r, p.g, p.b)))
                    .collect();
                ImgVec::new(gray, w, h)
            }
        }
    }

    /// Convert to Gray8, consuming self.
    ///
    /// Avoids a clone when the data is already Gray8.
    pub fn into_gray8(self) -> ImgVec<Gray<u8>> {
        match self {
            PixelData::Gray8(img) => img,
            other => other.to_gray8(),
        }
    }

    /// Convert to BGRA8 by reference, allocating a new buffer.
    ///
    /// RGB/RGBA variants have channels reordered. Higher-precision formats
    /// are clamped/truncated to 8-bit.
    pub fn to_bgra8(&self) -> ImgVec<BGRA<u8>> {
        match self {
            PixelData::Bgra8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                ImgVec::new(buf.into_owned(), w, h)
            }
            PixelData::Rgba8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let bgra: Vec<BGRA<u8>> = buf
                    .iter()
                    .map(|p| BGRA {
                        b: p.b,
                        g: p.g,
                        r: p.r,
                        a: p.a,
                    })
                    .collect();
                ImgVec::new(bgra, w, h)
            }
            PixelData::Rgb8(img) => {
                let (buf, w, h) = img.as_ref().to_contiguous_buf();
                let bgra: Vec<BGRA<u8>> = buf
                    .iter()
                    .map(|p| BGRA {
                        b: p.b,
                        g: p.g,
                        r: p.r,
                        a: 255,
                    })
                    .collect();
                ImgVec::new(bgra, w, h)
            }
            other => {
                // Fall back through RGBA for all other formats.
                let rgba = other.to_rgba8();
                let (buf, w, h) = rgba.as_ref().to_contiguous_buf();
                let bgra: Vec<BGRA<u8>> = buf
                    .iter()
                    .map(|p| BGRA {
                        b: p.b,
                        g: p.g,
                        r: p.r,
                        a: p.a,
                    })
                    .collect();
                ImgVec::new(bgra, w, h)
            }
        }
    }

    /// Convert to BGRA8, consuming self.
    ///
    /// Avoids a clone when the data is already Bgra8.
    pub fn into_bgra8(self) -> ImgVec<BGRA<u8>> {
        match self {
            PixelData::Bgra8(img) => img,
            other => other.to_bgra8(),
        }
    }
}

/// BT.601 luminance from 8-bit RGB. Matches JPEG's grayscale conversion.
fn rgb_to_luma(r: u8, g: u8, b: u8) -> u8 {
    // Fixed-point: 0.299*256=77, 0.587*256=150, 0.114*256=29 (sum=256)
    ((77u32 * r as u32 + 150u32 * g as u32 + 29u32 * b as u32) >> 8) as u8
}

/// Convert 16-bit to 8-bit with proper rounding.
///
/// Uses `(v * 255 + 32768) >> 16` which maps 0->0 and 65535->255 exactly.
#[inline]
fn u16_to_u8(v: u16) -> u8 {
    ((v as u32 * 255 + 32768) >> 16) as u8
}

impl core::fmt::Debug for PixelData {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let variant = match self {
            PixelData::Rgb8(_) => "Rgb8",
            PixelData::Rgba8(_) => "Rgba8",
            PixelData::Rgb16(_) => "Rgb16",
            PixelData::Rgba16(_) => "Rgba16",
            PixelData::RgbF32(_) => "RgbF32",
            PixelData::RgbaF32(_) => "RgbaF32",
            PixelData::Gray8(_) => "Gray8",
            PixelData::Gray16(_) => "Gray16",
            PixelData::GrayF32(_) => "GrayF32",
            PixelData::Bgra8(_) => "Bgra8",
            PixelData::GrayAlpha8(_) => "GrayAlpha8",
            PixelData::GrayAlpha16(_) => "GrayAlpha16",
            PixelData::GrayAlphaF32(_) => "GrayAlphaF32",
        };
        write!(
            f,
            "PixelData::{}({}x{})",
            variant,
            self.width(),
            self.height()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn dimensions_and_alpha() {
        let img = ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 100], 10, 10);
        let data = PixelData::Rgb8(img);
        assert_eq!(data.width(), 10);
        assert_eq!(data.height(), 10);
        assert!(!data.has_alpha());

        let data = PixelData::Rgba8(ImgVec::new(
            vec![
                Rgba {
                    r: 0u8,
                    g: 0,
                    b: 0,
                    a: 255
                };
                4
            ],
            2,
            2,
        ));
        assert!(data.has_alpha());

        let data = PixelData::Bgra8(ImgVec::new(
            vec![
                BGRA {
                    b: 0,
                    g: 0,
                    r: 0,
                    a: 255
                };
                4
            ],
            2,
            2,
        ));
        assert!(data.has_alpha());

        let data = PixelData::Gray8(ImgVec::new(vec![Gray::new(0u8); 4], 2, 2));
        assert!(!data.has_alpha());

        let data = PixelData::GrayF32(ImgVec::new(vec![Gray::new(0.0f32); 4], 2, 2));
        assert!(!data.has_alpha());
        assert_eq!(data.width(), 2);
        assert_eq!(data.height(), 2);
    }

    #[test]
    fn gray_alpha8_has_alpha() {
        let data = PixelData::GrayAlpha8(ImgVec::new(vec![GrayAlpha::new(128, 200); 4], 2, 2));
        assert!(data.has_alpha());
        assert_eq!(data.width(), 2);
        assert_eq!(data.height(), 2);
    }

    #[test]
    fn to_bytes_rgb8() {
        let data = PixelData::Rgb8(ImgVec::new(vec![Rgb { r: 1, g: 2, b: 3 }; 1], 1, 1));
        assert_eq!(data.to_bytes(), vec![1, 2, 3]);
    }

    #[test]
    fn to_bytes_gray8() {
        let data = PixelData::Gray8(ImgVec::new(vec![Gray::new(42u8); 2], 2, 1));
        assert_eq!(data.to_bytes(), vec![42, 42]);
    }

    #[test]
    fn to_bytes_rgba8() {
        let data = PixelData::Rgba8(ImgVec::new(
            vec![
                Rgba {
                    r: 1,
                    g: 2,
                    b: 3,
                    a: 4
                };
                1
            ],
            1,
            1,
        ));
        assert_eq!(data.to_bytes(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn gray_alpha_debug() {
        let data = PixelData::GrayAlpha8(ImgVec::new(vec![GrayAlpha::new(0u8, 0); 6], 3, 2));
        assert_eq!(alloc::format!("{:?}", data), "PixelData::GrayAlpha8(3x2)");
    }

    #[test]
    fn debug_format() {
        let data = PixelData::Rgb8(ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 6], 3, 2));
        assert_eq!(alloc::format!("{:?}", data), "PixelData::Rgb8(3x2)");

        let data = PixelData::GrayF32(ImgVec::new(vec![Gray::new(0.0f32); 6], 3, 2));
        assert_eq!(alloc::format!("{:?}", data), "PixelData::GrayF32(3x2)");
    }

    #[test]
    fn as_pixel_slice_rgb8() {
        let data = PixelData::Rgb8(ImgVec::new(
            vec![
                Rgb {
                    r: 10,
                    g: 20,
                    b: 30,
                },
                Rgb {
                    r: 40,
                    g: 50,
                    b: 60,
                },
            ],
            2,
            1,
        ));
        let slice = data.as_pixel_slice().unwrap();
        assert_eq!(slice.width(), 2);
        assert_eq!(slice.rows(), 1);
        assert_eq!(slice.descriptor(), crate::buffer::PixelDescriptor::RGB8);
        assert_eq!(slice.row(0), &[10, 20, 30, 40, 50, 60]);
    }

    #[test]
    fn as_pixel_slice_gray_alpha_returns_none() {
        let data = PixelData::GrayAlpha8(ImgVec::new(vec![GrayAlpha::new(0u8, 0)], 1, 1));
        assert!(data.as_pixel_slice().is_none());

        let data = PixelData::GrayAlpha16(ImgVec::new(vec![GrayAlpha::new(0u16, 0)], 1, 1));
        assert!(data.as_pixel_slice().is_none());

        let data = PixelData::GrayAlphaF32(ImgVec::new(vec![GrayAlpha::new(0.0f32, 0.0)], 1, 1));
        assert!(data.as_pixel_slice().is_none());
    }

    #[test]
    fn as_accessors_match() {
        let rgb8 = PixelData::Rgb8(ImgVec::new(vec![Rgb { r: 1, g: 2, b: 3 }], 1, 1));
        assert!(rgb8.as_rgb8().is_some());
        assert!(rgb8.as_rgba8().is_none());
        assert!(rgb8.as_rgb16().is_none());
        assert!(rgb8.as_gray8().is_none());

        let rgba16 = PixelData::Rgba16(ImgVec::new(
            vec![Rgba {
                r: 1u16,
                g: 2,
                b: 3,
                a: 4,
            }],
            1,
            1,
        ));
        assert!(rgba16.as_rgba16().is_some());
        assert!(rgba16.as_rgba8().is_none());
        assert!(rgba16.as_rgb16().is_none());

        let gf32 = PixelData::GrayF32(ImgVec::new(vec![Gray::new(0.5f32)], 1, 1));
        assert!(gf32.as_gray_f32().is_some());
        assert!(gf32.as_gray8().is_none());
        assert!(gf32.as_rgb_f32().is_none());

        let ga8 = PixelData::GrayAlpha8(ImgVec::new(vec![GrayAlpha::new(128u8, 255)], 1, 1));
        assert!(ga8.as_gray_alpha8().is_some());
        assert!(ga8.as_gray8().is_none());
    }

    #[test]
    fn try_into_accessors_match() {
        let rgb8 = PixelData::Rgb8(ImgVec::new(vec![Rgb { r: 1, g: 2, b: 3 }], 1, 1));
        assert!(rgb8.try_into_rgb8().is_some());

        let rgb8 = PixelData::Rgb8(ImgVec::new(vec![Rgb { r: 1, g: 2, b: 3 }], 1, 1));
        assert!(rgb8.try_into_rgba8().is_none());

        let rf32 = PixelData::RgbF32(ImgVec::new(
            vec![Rgb {
                r: 0.5f32,
                g: 0.5,
                b: 0.5,
            }],
            1,
            1,
        ));
        assert!(rf32.try_into_rgb_f32().is_some());

        let rf32 = PixelData::RgbF32(ImgVec::new(
            vec![Rgb {
                r: 0.5f32,
                g: 0.5,
                b: 0.5,
            }],
            1,
            1,
        ));
        assert!(rf32.try_into_rgb8().is_none());

        let ga16 = PixelData::GrayAlpha16(ImgVec::new(vec![GrayAlpha::new(1000u16, 2000)], 1, 1));
        assert!(ga16.try_into_gray_alpha16().is_some());
    }

    #[test]
    fn as_pixel_slice_all_non_gray_alpha() {
        // Verify all non-GrayAlpha variants return Some.
        let cases: Vec<PixelData> = vec![
            PixelData::Rgb8(ImgVec::new(vec![Rgb { r: 0, g: 0, b: 0 }], 1, 1)),
            PixelData::Rgba8(ImgVec::new(
                vec![Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 0,
                }],
                1,
                1,
            )),
            PixelData::Rgb16(ImgVec::new(
                vec![Rgb {
                    r: 0u16,
                    g: 0,
                    b: 0,
                }],
                1,
                1,
            )),
            PixelData::Rgba16(ImgVec::new(
                vec![Rgba {
                    r: 0u16,
                    g: 0,
                    b: 0,
                    a: 0,
                }],
                1,
                1,
            )),
            PixelData::RgbF32(ImgVec::new(
                vec![Rgb {
                    r: 0.0f32,
                    g: 0.0,
                    b: 0.0,
                }],
                1,
                1,
            )),
            PixelData::RgbaF32(ImgVec::new(
                vec![Rgba {
                    r: 0.0f32,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }],
                1,
                1,
            )),
            PixelData::Gray8(ImgVec::new(vec![Gray::new(0u8)], 1, 1)),
            PixelData::Gray16(ImgVec::new(vec![Gray::new(0u16)], 1, 1)),
            PixelData::GrayF32(ImgVec::new(vec![Gray::new(0.0f32)], 1, 1)),
            PixelData::Bgra8(ImgVec::new(
                vec![BGRA {
                    b: 0,
                    g: 0,
                    r: 0,
                    a: 0,
                }],
                1,
                1,
            )),
        ];
        for data in cases {
            assert!(
                data.as_pixel_slice().is_some(),
                "as_pixel_slice returned None for {:?}",
                data
            );
        }
    }
}
