//! Interpret actual requested output, not merely the embedded source profile.
use super::{
    image_items::{ImageGeometry, Rect},
    image_output::{Auxiliary, AuxiliaryKind, ImageOutput, Values},
    jxl_items::Plane,
    tiff_region::Failure,
};
use std::{io::Error, sync::Arc};
use zenjxl_decoder::api::{
    ExtraChannel, JxlBasicInfo, JxlColorProfile, JxlColorType, JxlPixelFormat,
};
use zenpixels::{
    AlphaMode, ChannelLayout, ChannelType, ColorContext, ColorPrimaries, Orientation, PixelBuffer,
    PixelBufferParts, PixelDescriptor, TransferFunction,
};

pub fn adopt(
    planes: Vec<Plane>,
    info: &JxlBasicInfo,
    format: &JxlPixelFormat,
    profile: &JxlColorProfile,
) -> Result<ImageOutput, Failure> {
    let layout = match format.color_type {
        JxlColorType::Grayscale => ChannelLayout::Gray,
        JxlColorType::GrayscaleAlpha => ChannelLayout::GrayAlpha,
        JxlColorType::Rgb => ChannelLayout::Rgb,
        JxlColorType::Rgba => ChannelLayout::Rgba,
        JxlColorType::Bgra => ChannelLayout::Bgra,
        JxlColorType::Cmyk => ChannelLayout::Cmyk,
        JxlColorType::Bgr => return Err(Error::other("BGR interpretation is unqualified").into()),
    };
    let alpha = if layout.has_alpha() {
        let channel = info
            .extra_channels
            .iter()
            .find(|ec| ec.ec_type == ExtraChannel::Alpha)
            .ok_or_else(|| Error::other("output alpha has no source declaration"))?;
        Some(alpha_mode(channel.alpha_associated))
    } else {
        None
    };
    let icc = match profile {
        JxlColorProfile::Icc(bytes) => bytes.clone(),
        JxlColorProfile::Simple(encoding) => encoding
            .maybe_create_profile()?
            .ok_or_else(|| Error::other("native output color cannot be represented by ICC"))?,
    };
    let mut planes = planes.into_iter();
    let color = planes
        .next()
        .ok_or_else(|| Error::other("missing native color plane"))?;
    let (width, height) = (u32::try_from(color.width)?, u32::try_from(color.height)?);
    let descriptor =
        PixelDescriptor::new(ChannelType::F32, layout, alpha, TransferFunction::Unknown)
            .with_primaries(ColorPrimaries::Unknown);
    let mut parts = PixelBufferParts::new(color.bytes, width, height, color.stride, descriptor);
    parts.color_context = Some(Arc::new(ColorContext::from_icc(Arc::<[u8]>::from(icc))));
    let pixels = PixelBuffer::try_from_parts(parts).map_err(|e| e.without_buffer())?;
    let mut extras = Vec::new();
    extras.try_reserve_exact(info.extra_channels.len())?;
    for ec in &info.extra_channels {
        let plane = planes
            .next()
            .ok_or_else(|| Error::other("missing requested native extra channel"))?;
        if plane.channels != 1 {
            return Err(Error::other("extra plane has multiple channels").into());
        }
        let kind = match ec.ec_type {
            ExtraChannel::Alpha => AuxiliaryKind::Alpha(alpha_mode(ec.alpha_associated)),
            ExtraChannel::Depth => AuxiliaryKind::Depth,
            ExtraChannel::SpotColor => AuxiliaryKind::SpotColor,
            ExtraChannel::SelectionMask => AuxiliaryKind::SelectionMask,
            ExtraChannel::Black => AuxiliaryKind::Black,
            ExtraChannel::CFA => AuxiliaryKind::Cfa,
            ExtraChannel::Thermal => AuxiliaryKind::Thermal,
            other => AuxiliaryKind::Unknown(other as u32),
        };
        extras.push((
            Auxiliary {
                kind,
                name: ec.name.clone(),
                source_bits: ec.bits_per_sample,
                source_dim_shift: ec.dim_shift,
                unexposed_native_parameters: matches!(
                    ec.ec_type,
                    ExtraChannel::SpotColor | ExtraChannel::CFA
                ),
            },
            Values::new(
                plane.bytes,
                u32::try_from(plane.width)?,
                u32::try_from(plane.height)?,
                plane.stride,
                ChannelType::F32,
            )?,
        ));
    }
    if planes.next().is_some() {
        return Err(Error::other("unidentified native extra plane").into());
    }
    let geometry = ImageGeometry {
        canvas_width: u32::try_from(info.coded_size.0)?,
        canvas_height: u32::try_from(info.coded_size.1)?,
        region: Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    };
    let orientation = Orientation::from_exif(info.orientation as u8)
        .ok_or_else(|| Error::other("invalid JXL orientation"))?;
    Ok(ImageOutput::packed(pixels, extras, geometry, orientation))
}

fn alpha_mode(associated: bool) -> AlphaMode {
    if associated {
        AlphaMode::Premultiplied
    } else {
        AlphaMode::Straight
    }
}
