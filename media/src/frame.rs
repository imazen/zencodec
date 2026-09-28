//! Materialize native components as packed RGB without guessing display color.

use crate::{
    color::{ColorError, YuvToRgb, YuvView},
    display::{DisplayConversion, DisplayError, OutOfRange},
};
use std::{fmt, sync::Arc};
use zenpixels::{ColorContext, PixelBuffer, PixelFormat};

/// Packed output precision. Integer output spans the entire storage range;
/// F32 keeps finite out-of-range matrix results when no display conversion runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RgbStorage {
    U8,
    U16,
    F32,
}

/// Render one full frame or checked crop. Matrix reconstruction and optional
/// display conversion happen row by row. The output carries the *resulting*
/// RGB CICP, including unknown raw codes when no display interpretation is made.
///
/// Integer packing rounds to nearest, ties upward, without dithering. Clipping
/// is explicit. U16 is normalized 0..65535, never an untagged 10/12-bit code.
/// `max_bytes` covers the returned packed allocation plus one RGB f64 row;
/// backend frame ownership and allocator bookkeeping are additional.
pub fn to_rgb(
    source: YuvView<'_>,
    storage: RgbStorage,
    clipping: OutOfRange,
    display: Option<&DisplayConversion>,
    max_bytes: usize,
    stop: Option<&dyn enough::Stop>,
) -> Result<PixelBuffer, FrameError> {
    check_stop(stop)?;
    let matrix = YuvToRgb::new(source).map_err(FrameError::Color)?;
    let color = if let Some(display) = display {
        if matrix.output_color() != display.input_color() {
            return Err(FrameError::ColorMismatch);
        }
        display.output_color()
    } else {
        matrix.output_color()
    };
    let (format, size) = match storage {
        RgbStorage::U8 => (PixelFormat::Rgb8, 1),
        RgbStorage::U16 => (PixelFormat::Rgb16, 2),
        RgbStorage::F32 => (PixelFormat::RgbF32, 4),
    };
    let width = matrix.width();
    let height = matrix.height();
    let bytes = width
        .checked_mul(height)
        .and_then(|v| v.checked_mul(3 * size))
        .ok_or(FrameError::Limit)?;
    let scratch = width
        .checked_mul(core::mem::size_of::<[f64; 3]>())
        .ok_or(FrameError::Limit)?;
    if bytes.checked_add(scratch).is_none_or(|n| n > max_bytes) {
        return Err(FrameError::Limit);
    }
    let (w, h) = (
        u32::try_from(width).map_err(|_| FrameError::Limit)?,
        u32::try_from(height).map_err(|_| FrameError::Limit)?,
    );
    let mut output = Vec::new();
    output
        .try_reserve_exact(bytes)
        .map_err(|_| FrameError::Allocation)?;
    let mut row = Vec::new();
    row.try_reserve_exact(width)
        .map_err(|_| FrameError::Allocation)?;
    row.resize(width, [0.0; 3]);
    for y in 0..height {
        check_stop(stop)?;
        matrix
            .write_row_f64(y, &mut row)
            .map_err(FrameError::Color)?;
        for &rgb in &row {
            let rgb = if let Some(display) = display {
                display
                    .convert(rgb.map(|v| v as f32))
                    .map_err(FrameError::Display)?
                    .map(f64::from)
            } else {
                rgb
            };
            for mut value in rgb {
                if !value.is_finite() {
                    return Err(FrameError::NonFinite);
                }
                if storage != RgbStorage::F32 {
                    if !(0.0..=1.0).contains(&value) && clipping == OutOfRange::Reject {
                        return Err(FrameError::OutOfRange);
                    }
                    value = value.clamp(0.0, 1.0);
                }
                match storage {
                    RgbStorage::U8 => output.push((value * 255.0 + 0.5).floor() as u8),
                    RgbStorage::U16 => output
                        .extend_from_slice(&((value * 65535.0 + 0.5).floor() as u16).to_ne_bytes()),
                    RgbStorage::F32 => output.extend_from_slice(&(value as f32).to_ne_bytes()),
                }
            }
        }
    }
    check_stop(stop)?;
    let buffer = PixelBuffer::from_vec(output, w, h, color.to_descriptor(format))
        .map_err(|_| FrameError::Allocation)?;
    Ok(buffer.with_color_context(Arc::new(ColorContext::from_cicp(color))))
}

fn check_stop(stop: Option<&dyn enough::Stop>) -> Result<(), FrameError> {
    if let Some(stop) = stop {
        stop.check().map_err(FrameError::Stopped)?;
    }
    Ok(())
}

#[derive(Debug)]
#[non_exhaustive]
pub enum FrameError {
    Color(ColorError),
    Display(DisplayError),
    ColorMismatch,
    Limit,
    Allocation,
    NonFinite,
    OutOfRange,
    Stopped(enough::StopReason),
}
impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RGB frame conversion: {self:?}")
    }
}
impl std::error::Error for FrameError {}
