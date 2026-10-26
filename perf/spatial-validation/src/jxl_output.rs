//! Interpret actual requested output, not merely the embedded source profile.
use super::{
    image_items::{ImageGeometry, Rect},
    image_output::{
        Auxiliary, AuxiliaryKind, ColorContribution, ImageInterpretation, ImageOutput,
        SourcePrecision, Values,
    },
    jxl_items::Plane,
    tiff_region::Failure,
};
use std::{io::Error, sync::Arc};
use zenjxl_decoder::api::{
    ExtraChannel, JxlBasicInfo, JxlBitDepth, JxlColorProfile, JxlColorType, JxlExtraChannelDetails,
    JxlPixelFormat,
};
use zenpixels::{
    AlphaMode, ChannelLayout, ChannelType, ColorContext, ColorPrimaries, Orientation, PixelBuffer,
    PixelBufferParts, PixelDescriptor, TransferFunction,
};

/// Retain native source facts without treating them as an output profile or
/// as metadata approved for passthrough.
pub struct JxlSourceFacts {
    pub header: JxlBasicInfo,
    pub embedded_color: JxlColorProfile,
    pub frame_index: usize,
    pub frame_name: String,
    pub is_last: bool,
    pub frame_timecode: Option<u32>,
    pub extra_details: Vec<JxlExtraChannelDetails>,
}

pub fn adopt(
    planes: Vec<Plane>,
    source: JxlSourceFacts,
    format: &JxlPixelFormat,
    profile: &JxlColorProfile,
    timing: Option<zencodec::frame::FrameTiming>,
    spot_colors_rendered: bool,
) -> Result<ImageOutput, Failure> {
    let info = &source.header;
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
    if source.extra_details.len() != info.extra_channels.len() {
        return Err(Error::other("extra-channel interpretation count differs from source").into());
    }
    for (ec, details) in info.extra_channels.iter().zip(&source.extra_details) {
        let plane = planes
            .next()
            .ok_or_else(|| Error::other("missing requested native extra channel"))?;
        if plane.channels != 1 {
            return Err(Error::other("extra plane has multiple channels").into());
        }
        let kind = match ec.ec_type {
            ExtraChannel::Alpha => AuxiliaryKind::Alpha(alpha_mode(ec.alpha_associated)),
            ExtraChannel::Depth => AuxiliaryKind::Depth,
            ExtraChannel::SpotColor => AuxiliaryKind::SpotColor {
                linear_rgba: details
                    .spot_color
                    .ok_or_else(|| Error::other("spot channel is missing source RGBA"))?,
                contribution: if spot_colors_rendered {
                    ColorContribution::Applied
                } else {
                    ColorContribution::Separate
                },
            },
            ExtraChannel::SelectionMask => AuxiliaryKind::SelectionMask,
            ExtraChannel::Black => AuxiliaryKind::Black,
            ExtraChannel::CFA => AuxiliaryKind::Cfa {
                channel: details
                    .cfa_channel
                    .ok_or_else(|| Error::other("CFA channel is missing source index"))?,
            },
            ExtraChannel::Thermal => AuxiliaryKind::Thermal,
            other => AuxiliaryKind::Unknown(other as u32),
        };
        extras.push((
            Auxiliary {
                kind,
                name: ec.name.clone(),
                source_precision: match details.bit_depth {
                    JxlBitDepth::Int { bits_per_sample } => SourcePrecision::Integer {
                        bits: bits_per_sample,
                        signed: false,
                    },
                    JxlBitDepth::Float {
                        bits_per_sample,
                        exponent_bits_per_sample,
                    } => SourcePrecision::Float {
                        bits: bits_per_sample,
                        exponent_bits: exponent_bits_per_sample,
                    },
                },
                source_dim_shift: ec.dim_shift,
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
    let mut interpretation = ImageInterpretation::packed(geometry, orientation);
    interpretation.timing = timing;
    interpretation.source_details.insert(source);
    ImageOutput::packed(pixels, extras, interpretation)
}

fn alpha_mode(associated: bool) -> AlphaMode {
    if associated {
        AlphaMode::Premultiplied
    } else {
        AlphaMode::Straight
    }
}
