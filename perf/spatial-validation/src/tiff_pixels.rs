//! Native sample adoption, without numerical or display conversion.
use super::tiff_region::{Failure, Region};
use std::{fmt, sync::Arc};
use tiff::{ColorType, tags::SampleFormat};
use zenpixels::{
    AlphaMode, ChannelLayout, ChannelType, ColorContext, ColorPrimaries, Orientation, PixelBuffer,
    PixelBufferParts, PixelDescriptor, TransferFunction,
};

#[derive(Debug, PartialEq, Eq)]
pub enum InterpretationError {
    Samples,
    Alpha,
    Orientation,
}

impl fmt::Display for InterpretationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Samples => "unqualified native sample representation",
            Self::Alpha => "TIFF extra sample does not establish alpha association",
            Self::Orientation => "TIFF orientation is outside 1..8",
        })
    }
}
impl std::error::Error for InterpretationError {}

#[derive(Debug)]
pub struct StoredRaster {
    pub pixels: PixelBuffer,
    pub geometry: super::image_items::ImageGeometry,
    /// Still unapplied: the buffer is in stored coordinates.
    pub orientation: Orientation,
}
impl StoredRaster {
    pub fn into_image(self) -> Result<super::image_output::ImageOutput, Failure> {
        super::image_output::ImageOutput::packed(
            self.pixels,
            Vec::new(),
            super::image_output::ImageInterpretation::packed(self.geometry, self.orientation),
        )
    }
}

impl Region {
    /// Move the native allocation into the common pixel owner. Interpretation
    /// comes from this selected directory, not a root/page/global default.
    pub fn into_pixels(self) -> Result<StoredRaster, Failure> {
        let channel = match (self.sample, self.color.bit_depth()) {
            (SampleFormat::Uint, 8) => ChannelType::U8,
            (SampleFormat::Uint, 16) => ChannelType::U16,
            (SampleFormat::Uint, 32) => ChannelType::U32,
            (SampleFormat::Uint, 64) => ChannelType::U64,
            (SampleFormat::Int, 8) => ChannelType::I8,
            (SampleFormat::Int, 16) => ChannelType::I16,
            (SampleFormat::Int, 32) => ChannelType::I32,
            (SampleFormat::Int, 64) => ChannelType::I64,
            (SampleFormat::IEEEFP, 32) => ChannelType::F32,
            (SampleFormat::IEEEFP, 64) => ChannelType::F64,
            _ => return Err(InterpretationError::Samples.into()),
        };
        let layout = match self.color {
            ColorType::Gray(_) => ChannelLayout::Gray,
            ColorType::GrayA(_) => ChannelLayout::GrayAlpha,
            ColorType::RGB(_) => ChannelLayout::Rgb,
            ColorType::RGBA(_) => ChannelLayout::Rgba,
            _ => return Err(InterpretationError::Samples.into()),
        };
        let alpha = if layout.has_alpha() {
            Some(match self.extra_samples.as_deref() {
                Some([1]) => AlphaMode::Premultiplied,
                Some([2]) => AlphaMode::Straight,
                _ => return Err(InterpretationError::Alpha.into()),
            })
        } else {
            if self.extra_samples.as_ref().is_some_and(|v| !v.is_empty()) {
                return Err(InterpretationError::Alpha.into());
            }
            None
        };
        let descriptor = PixelDescriptor::new(channel, layout, alpha, TransferFunction::Unknown)
            .with_primaries(ColorPrimaries::Unknown);
        let orientation = Orientation::from_exif(
            u8::try_from(self.orientation).map_err(|_| InterpretationError::Orientation)?,
        )
        .ok_or(InterpretationError::Orientation)?;
        let mut parts =
            PixelBufferParts::new(self.bytes, self.width, self.height, self.stride, descriptor);
        parts.color_context = self
            .icc
            .map(|icc| Arc::new(ColorContext::from_icc(Arc::<[u8]>::from(icc))));
        let pixels = PixelBuffer::try_from_parts(parts).map_err(|e| e.without_buffer())?;
        Ok(StoredRaster {
            pixels,
            geometry: self.geometry,
            orientation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collection::Collection,
        image_items::{Composition, ImageRequest},
        tiff_items::TiffCollection,
        tiff_region::{self, Allowances, Rect, Selection},
    };
    use std::io::Cursor;
    use tiff::{
        encoder::{TiffEncoder, colortype},
        tags::Tag,
    };

    fn region() -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        }
    }

    fn alpha_file(alpha: Option<u16>) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new(&mut bytes).unwrap();
        let mut image = encoder.new_image::<colortype::RGBA8>(2, 1).unwrap();
        if let Some(alpha) = alpha {
            image
                .encoder()
                .write_tag(Tag::ExtraSamples, &[alpha][..])
                .unwrap();
        }
        image
            .write_data(&[8, 17, 29, 47, 89, 131, 193, 251])
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn native_alpha_association_is_required_and_preserved() {
        for alpha in [None, Some(0), Some(1), Some(2), Some(3)] {
            let output = tiff_region::decode(
                Cursor::new(alpha_file(alpha)),
                Selection {
                    page: 0,
                    levels: &[],
                    region: region(),
                },
                Allowances::default(),
                || Ok(()),
            )
            .unwrap();
            let result = output.into_pixels();
            match alpha {
                Some(1) | Some(2) => {
                    let raster = result.unwrap();
                    assert_eq!(
                        raster.pixels.descriptor().alpha(),
                        Some(if alpha == Some(1) {
                            AlphaMode::Premultiplied
                        } else {
                            AlphaMode::Straight
                        })
                    );
                    assert_eq!(
                        raster.pixels.as_slice().row(0),
                        &[8, 17, 29, 47, 89, 131, 193, 251]
                    );
                }
                _ => assert_eq!(
                    result.unwrap_err().downcast_ref::<InterpretationError>(),
                    Some(&InterpretationError::Alpha)
                ),
            }
        }
    }

    #[test]
    fn selected_directories_keep_their_own_icc_and_unapplied_orientation() {
        let mut bytes = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new_big(&mut bytes).unwrap();
        let profiles = [b"opaque page one".as_slice(), b"opaque page two".as_slice()];
        for (i, profile) in profiles.into_iter().enumerate() {
            let mut image = encoder.new_image::<colortype::RGB8>(2, 1).unwrap();
            image.encoder().write_tag(Tag::IccProfile, profile).unwrap();
            image
                .encoder()
                .write_tag(Tag::Orientation, if i == 0 { 6u16 } else { 1 })
                .unwrap();
            image.write_data(&[8, 17, 29, 47, 89, 131]).unwrap();
        }
        let mut images = TiffCollection::open(
            Cursor::new(bytes.into_inner()),
            Composition::Composited,
            Allowances::default(),
        )
        .unwrap();
        let mut cursor = images.images().unwrap();
        let first = images.next(&mut cursor).unwrap().unwrap();
        let second = images.next(&mut cursor).unwrap().unwrap();
        assert!(images.next(&mut cursor).unwrap().is_none());
        for (id, index) in [(&second, 1), (&first, 0), (&second, 1)] {
            let output = images.decode(id, &ImageRequest::default()).unwrap();
            output
                .with_pixels(|pixels| {
                    assert_eq!(
                        pixels.color_context().unwrap().icc.as_deref(),
                        Some(profiles[index])
                    );
                    assert_eq!(pixels.row(0), &[8, 17, 29, 47, 89, 131]);
                    assert_eq!(pixels.descriptor().primaries, ColorPrimaries::Unknown);
                    assert_eq!(pixels.descriptor().transfer(), TransferFunction::Unknown);
                })
                .unwrap();
            assert_eq!(
                output.interpretation().orientation,
                Orientation::from_exif(if index == 0 { 6 } else { 1 })
            );
        }
    }

    #[test]
    fn malformed_or_over_budget_icc_is_not_reported_as_absent() {
        for malformed in [false, true] {
            let mut bytes = Cursor::new(Vec::new());
            let mut encoder = TiffEncoder::new(&mut bytes).unwrap();
            let mut image = encoder.new_image::<colortype::RGB8>(2, 1).unwrap();
            if malformed {
                image
                    .encoder()
                    .write_tag(Tag::IccProfile, "wrong TIFF field type")
                    .unwrap();
            } else {
                image
                    .encoder()
                    .write_tag(Tag::IccProfile, &[7u8; 128][..])
                    .unwrap();
            }
            image.write_data(&[8, 17, 29, 47, 89, 131]).unwrap();
            let result = tiff_region::decode(
                Cursor::new(bytes.into_inner()),
                Selection {
                    page: 0,
                    levels: &[],
                    region: region(),
                },
                Allowances {
                    tag: 64,
                    ..Allowances::default()
                },
                || Ok(()),
            );
            let error = result.unwrap_err();
            assert!(error.downcast_ref::<tiff::TiffError>().is_some(), "{error}");
        }
    }
}
