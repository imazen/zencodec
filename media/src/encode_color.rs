//! Packed RGB/gray to native unsigned components for video encoding.
//!
//! This scalar conversion preserves the input transfer and primaries. It does
//! not interpret ICC or apply an EOTF. Chroma decimation is explicitly a
//! phase-aware triangle filter in source-encoded RGB, with edge replication.

use crate::{
    color::{ChromaLocation, ColorError, Subsampling, YuvView},
    display::OutOfRange,
    plane::{Plane, Samples},
};
use std::fmt;
use zenpixels::{
    AlphaMode, ChannelLayout, ChannelType, Cicp, PixelSlice, SignalRange, sample::SampleEncoding,
};

/// Video without an auxiliary alpha stream needs an explicit alpha policy.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum AlphaHandling {
    /// Reject any nonopaque pixel. Hidden RGB is never silently exposed.
    RequireOpaque,
    /// Source-over against an opaque RGB matte in the same encoded color
    /// space. This is encoded-domain composition, not linear-light blending.
    CompositeEncoded([f32; 3]),
}

/// Owned, tightly packed, LSB-aligned U16 words carrying 8–16 significant bits.
/// Component order and color remain attached to the frame, not its planes.
#[derive(Debug)]
pub struct YuvBuffer {
    data: [Vec<u16>; 3],
    width: usize,
    height: usize,
    plan: RgbToYuv,
}

impl YuvBuffer {
    pub fn view(&self) -> YuvView<'_> {
        let (sx, sy) = shifts(self.plan.sampling);
        let encoding =
            SampleEncoding::new(ChannelType::U16, self.plan.bits, 0).expect("validated encoding");
        let plane = |i: usize, w, h| {
            Plane::new(Samples::U16(self.data[i].as_slice()), w, h, w * 2, encoding)
                .expect("owned geometry")
        };
        let chroma = (self.plan.sampling != Subsampling::Monochrome).then(|| {
            [
                plane(1, self.width.div_ceil(sx), self.height.div_ceil(sy)),
                plane(2, self.width.div_ceil(sx), self.height.div_ceil(sy)),
            ]
        });
        YuvView::new(
            plane(0, self.width, self.height),
            chroma,
            self.plan.sampling,
            self.plan.location,
            self.plan.color,
        )
        .expect("owned components")
    }
}

/// Prepared RGB → YCbCr (or GBR identity) transform. Integer quantization is
/// nearest, ties upward, clamped to the storage code range; no dithering.
#[derive(Clone, Copy, Debug)]
pub struct RgbToYuv {
    bits: u8,
    sampling: Subsampling,
    location: ChromaLocation,
    color: Cicp,
    weights: [f64; 3],
    clipping: OutOfRange,
}

impl RgbToYuv {
    pub fn new(
        bits: u8,
        sampling: Subsampling,
        location: ChromaLocation,
        color: Cicp,
        clipping: OutOfRange,
    ) -> Result<Self, EncodeColorError> {
        if !(8..=16).contains(&bits) {
            return Err(ColorError::UnsupportedPrecision(bits).into());
        }
        if matches!(sampling, Subsampling::Yuv420 | Subsampling::Yuv422)
            && location == ChromaLocation::Unknown
        {
            return Err(ColorError::UnknownChromaLocation.into());
        }
        let (kr, kb) = match color.matrix_coefficients {
            0 if sampling != Subsampling::Yuv444 => {
                return Err(ColorError::IdentitySubsampling.into());
            }
            0 => (0.0, 0.0),
            1 => (0.2126, 0.0722),
            5 | 6 => (0.299, 0.114),
            9 => (0.2627, 0.0593),
            code => return Err(ColorError::UnsupportedMatrix(code).into()),
        };
        Ok(Self {
            bits,
            sampling,
            location,
            color,
            weights: [kr, 1.0 - kr - kb, kb],
            clipping,
        })
    }

    /// Convert one borrowed packed image. Supports RGB/RGBA/BGRA/gray/gray-alpha
    /// U8, native-endian U16, and native-endian F32, full-range only. U16 RGB
    /// follows zenpixels' full 0..65535 contract; low-bit native components
    /// must not be mislabeled as packed RGB16. Straight or opaque alpha only.
    ///
    /// Source transfer/primaries must match the target CICP. ICC-bearing input
    /// is rejected: resolve it through a CMS before this numeric operation.
    /// `max_bytes` bounds the returned component allocation; no full RGB float
    /// intermediate is allocated. Failure returns no partially converted frame.
    pub fn convert(
        &self,
        source: &PixelSlice<'_>,
        alpha: AlphaHandling,
        max_bytes: usize,
        stop: Option<&dyn enough::Stop>,
    ) -> Result<YuvBuffer, EncodeColorError> {
        check_stop(stop)?;
        let descriptor = source.descriptor();
        if !matches!(
            descriptor.channel_type(),
            ChannelType::U8 | ChannelType::U16 | ChannelType::F32
        ) || !matches!(
            descriptor.layout(),
            ChannelLayout::Rgb
                | ChannelLayout::Rgba
                | ChannelLayout::Bgra
                | ChannelLayout::Gray
                | ChannelLayout::GrayAlpha
        ) || descriptor.signal_range != SignalRange::Full
            || matches!(
                descriptor.alpha(),
                Some(AlphaMode::Premultiplied | AlphaMode::Undefined)
            )
        {
            return Err(EncodeColorError::UnsupportedPixels);
        }
        if descriptor.transfer().to_cicp() != Some(self.color.transfer_characteristics)
            || descriptor.primaries.to_cicp() != Some(self.color.color_primaries)
            || source.color_context().is_some_and(|c| c.icc.is_some())
        {
            return Err(EncodeColorError::ColorMismatch);
        }
        if let AlphaHandling::CompositeEncoded(matte) = alpha
            && matte
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(EncodeColorError::InvalidMatte);
        }
        let (width, height) = (source.width() as usize, source.rows() as usize);
        if width == 0 || height == 0 {
            return Err(ColorError::EmptyFrame.into());
        }
        let (sx, sy) = shifts(self.sampling);
        let (cw, ch) = (width.div_ceil(sx), height.div_ceil(sy));
        let y_count = width
            .checked_mul(height)
            .ok_or(EncodeColorError::AllocationLimit)?;
        let c_count = if self.sampling == Subsampling::Monochrome {
            0
        } else {
            cw.checked_mul(ch)
                .ok_or(EncodeColorError::AllocationLimit)?
        };
        let total = c_count
            .checked_mul(2)
            .and_then(|c| y_count.checked_add(c))
            .and_then(|v| v.checked_mul(2))
            .ok_or(EncodeColorError::AllocationLimit)?;
        if total > max_bytes {
            return Err(EncodeColorError::AllocationLimit);
        }
        let mut output = YuvBuffer {
            data: [Vec::new(), Vec::new(), Vec::new()],
            width,
            height,
            plan: *self,
        };
        for (data, count) in output.data.iter_mut().zip([y_count, c_count, c_count]) {
            data.try_reserve_exact(count)
                .map_err(|_| EncodeColorError::AllocationLimit)?;
            data.resize(count, 0);
        }
        for y in 0..height {
            check_stop(stop)?;
            for x in 0..width {
                let rgb = pixel(source, x, y, alpha, self.clipping)?;
                let values = self.components(rgb);
                output.data[0][y * width + x] = self.quantize(values[0], false);
            }
        }
        if c_count != 0 {
            let center_x = self.location == ChromaLocation::Center;
            let center_y = matches!(self.location, ChromaLocation::Center | ChromaLocation::Left);
            for y in 0..ch {
                check_stop(stop)?;
                for x in 0..cw {
                    let mut rgb = [0.0; 3];
                    for (py, wy) in taps(y, sy, center_y, height) {
                        if wy == 0.0 {
                            continue;
                        }
                        for (px, wx) in taps(x, sx, center_x, width) {
                            if wx == 0.0 {
                                continue;
                            }
                            let p = pixel(source, px, py, alpha, self.clipping)?;
                            for c in 0..3 {
                                rgb[c] += p[c] * wx * wy;
                            }
                        }
                    }
                    let values = self.components(rgb);
                    for (c, value) in values.into_iter().enumerate().skip(1) {
                        output.data[c][y * cw + x] =
                            self.quantize(value, self.color.matrix_coefficients != 0);
                    }
                }
            }
        }
        Ok(output)
    }

    fn components(&self, rgb: [f64; 3]) -> [f64; 3] {
        if self.color.matrix_coefficients == 0 {
            return [rgb[1], rgb[2], rgb[0]];
        }
        let [kr, _, kb] = self.weights;
        let [r, g, b] = rgb;
        // Difference form preserves the neutral axis and exact +/- 0.5
        // saturated chroma boundaries. Subtracting a rounded luma estimate
        // instead can move an exact quantization tie to the wrong side.
        let rg = r - g;
        let bg = b - g;
        [
            g + kr * rg + kb * bg,
            0.5 * bg - kr / (2.0 * (1.0 - kb)) * rg,
            0.5 * rg - kb / (2.0 * (1.0 - kr)) * bg,
        ]
    }

    fn quantize(&self, value: f64, chroma: bool) -> u16 {
        let max = f64::from((1_u32 << self.bits) - 1);
        let k = f64::from(1_u32 << (self.bits - 8));
        let (offset, span) = if chroma {
            (
                128.0 * k,
                if self.color.full_range {
                    max
                } else {
                    224.0 * k
                },
            )
        } else if self.color.full_range {
            (0.0, max)
        } else {
            (16.0 * k, 219.0 * k)
        };
        (value * span + offset).round().clamp(0.0, max) as u16
    }
}

fn shifts(sampling: Subsampling) -> (usize, usize) {
    match sampling {
        Subsampling::Yuv420 => (2, 2),
        Subsampling::Yuv422 => (2, 1),
        _ => (1, 1),
    }
}

// A normalized triangle with radius two on a subsampled axis. Co-sited taps
// are 1/4,1/2,1/4; centered taps are 1/8,3/8,3/8,1/8. Duplicate edge samples
// retain their weights. Never rebase phase at an odd source boundary.
fn taps(index: usize, spacing: usize, centered: bool, len: usize) -> [(usize, f64); 4] {
    if spacing == 1 {
        return [(index, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)];
    }
    let base = index * 2;
    let points = [
        base.saturating_sub(1),
        base,
        base.saturating_add(1).min(len - 1),
        base.saturating_add(2).min(len - 1),
    ];
    let weights = if centered {
        [0.125, 0.375, 0.375, 0.125]
    } else {
        [0.25, 0.5, 0.25, 0.0]
    };
    std::array::from_fn(|i| (points[i], weights[i]))
}

fn pixel(
    source: &PixelSlice<'_>,
    x: usize,
    y: usize,
    alpha: AlphaHandling,
    clipping: OutOfRange,
) -> Result<[f64; 3], EncodeColorError> {
    let d = source.descriptor();
    let row = source.row(y as u32);
    let bytes = d.channel_type().byte_size();
    let count = d.channels();
    let offset = x * count * bytes;
    let read = |channel| {
        let start = offset + channel * bytes;
        match d.channel_type() {
            ChannelType::U8 => f64::from(row[start]) / 255.0,
            ChannelType::U16 => {
                f64::from(u16::from_ne_bytes(
                    row[start..start + 2].try_into().expect("validated word"),
                )) / 65535.0
            }
            ChannelType::F32 => f64::from(f32::from_ne_bytes(
                row[start..start + 4].try_into().expect("validated float"),
            )),
            _ => unreachable!("validated storage"),
        }
    };
    let mut rgb = match d.layout() {
        ChannelLayout::Rgb | ChannelLayout::Rgba => [read(0), read(1), read(2)],
        ChannelLayout::Bgra => [read(2), read(1), read(0)],
        _ => [read(0); 3],
    };
    let a = if d.layout().has_alpha() {
        read(count - 1)
    } else {
        1.0
    };
    if !a.is_finite() || !(0.0..=1.0).contains(&a) || rgb.iter().any(|v| !v.is_finite()) {
        return Err(EncodeColorError::InvalidPixel);
    }
    if rgb.iter().any(|v| !(0.0..=1.0).contains(v)) {
        if clipping == OutOfRange::Reject {
            return Err(EncodeColorError::InvalidPixel);
        }
        rgb = rgb.map(|v| v.clamp(0.0, 1.0));
    }
    if a != 1.0 {
        match alpha {
            AlphaHandling::RequireOpaque => return Err(EncodeColorError::NonopaquePixel),
            AlphaHandling::CompositeEncoded(matte) => {
                for c in 0..3 {
                    rgb[c] = rgb[c] * a + f64::from(matte[c]) * (1.0 - a);
                }
            }
        }
    }
    Ok(rgb)
}

#[derive(Debug)]
#[non_exhaustive]
pub enum EncodeColorError {
    Components(ColorError),
    UnsupportedPixels,
    ColorMismatch,
    InvalidMatte,
    InvalidPixel,
    NonopaquePixel,
    AllocationLimit,
    Cancelled,
}
impl From<ColorError> for EncodeColorError {
    fn from(e: ColorError) -> Self {
        Self::Components(e)
    }
}
impl fmt::Display for EncodeColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Components(e) => e.fmt(f),
            Self::UnsupportedPixels => f.write_str("unsupported packed storage, range, layout or alpha mode"),
            Self::ColorMismatch => f.write_str("packed pixels require color conversion or ICC resolution before component encoding"),
            Self::InvalidMatte => f.write_str("encoded matte must have finite RGB components in [0,1]"),
            Self::InvalidPixel => f.write_str("packed pixel is nonfinite or outside the permitted domain"),
            Self::NonopaquePixel => f.write_str("video conversion requires an explicit policy for nonopaque pixels"),
            Self::Cancelled => f.write_str("native component conversion cancelled"),
            Self::AllocationLimit => f.write_str("native component allocation exceeds the byte budget or address space"),
        }
    }
}
impl std::error::Error for EncodeColorError {}

fn check_stop(stop: Option<&dyn enough::Stop>) -> Result<(), EncodeColorError> {
    if let Some(stop) = stop {
        stop.check().map_err(|_| EncodeColorError::Cancelled)?;
    }
    Ok(())
}
