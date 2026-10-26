//! Native strip qualification with independently constructed sample bit patterns.
use crate::tiff_region::{self, Allowances, Rect, Selection};
use std::io::Cursor;
use tiff::{
    ColorType,
    encoder::{Compression, Predictor, TiffEncoder, TiffKind, TiffValue, colortype},
    tags::{SampleFormat, Tag},
};
use zencodec::source::SliceReadAt;
use zencodec_io::ReadAtReader;

const WIDTH: u32 = 35;
const HEIGHT: u32 = 29;

fn encoded<C: colortype::ColorType, K: TiffKind>(
    encoder: TiffEncoder<&mut Cursor<Vec<u8>>, K>,
    samples: &[C::Inner],
    compression: Compression,
    predictor: Predictor,
    orientation: u16,
) where
    [C::Inner]: TiffValue,
{
    let mut encoder = encoder
        .with_compression(compression)
        .with_predictor(predictor);
    let mut image = encoder.new_image::<C>(WIDTH, HEIGHT).unwrap();
    image.rows_per_strip(5).unwrap();
    image
        .encoder()
        .write_tag(Tag::Orientation, orientation)
        .unwrap();
    // The four-channel fixture explicitly declares straight alpha. A storage
    // layout alone cannot establish the fourth channel's meaning.
    if C::BITS_PER_SAMPLE.len() == 4 {
        image
            .encoder()
            .write_tag(Tag::ExtraSamples, &[2u16][..])
            .unwrap();
    }
    image.write_data(samples).unwrap();
}

fn qualify<C: colortype::ColorType>(
    samples: Vec<C::Inner>,
    original: Vec<u8>,
    color: ColorType,
    sample: SampleFormat,
) where
    [C::Inner]: TiffValue,
{
    let channels = usize::from(color.num_samples());
    let bpp = usize::from(color.bit_depth() / 8) * channels;
    assert_eq!(samples.len(), WIDTH as usize * HEIGHT as usize * channels);
    assert_eq!(original.len(), WIDTH as usize * HEIGHT as usize * bpp);
    for big in [false, true] {
        for (compression_index, compression) in [
            Compression::Uncompressed,
            Compression::Packbits,
            Compression::Lzw,
            Compression::Deflate(Default::default()),
        ]
        .into_iter()
        .enumerate()
        {
            for (predictor_index, predictor) in [
                Predictor::None,
                if sample == SampleFormat::IEEEFP {
                    Predictor::FloatingPoint
                } else {
                    Predictor::Horizontal
                },
            ]
            .into_iter()
            .enumerate()
            {
                let orientation = (compression_index * 2 + predictor_index + 1) as u16;
                let mut cursor = Cursor::new(Vec::new());
                if big {
                    encoded::<C, _>(
                        TiffEncoder::new_big(&mut cursor).unwrap(),
                        &samples,
                        compression,
                        predictor,
                        orientation,
                    );
                } else {
                    encoded::<C, _>(
                        TiffEncoder::new(&mut cursor).unwrap(),
                        &samples,
                        compression,
                        predictor,
                        orientation,
                    );
                }
                for region in [
                    Rect {
                        x: 13,
                        y: 4,
                        width: 6,
                        height: 12,
                    },
                    Rect {
                        x: 31,
                        y: 24,
                        width: 4,
                        height: 5,
                    },
                    Rect {
                        x: 0,
                        y: 0,
                        width: WIDTH,
                        height: HEIGHT,
                    },
                ] {
                    let expected: Vec<u8> = (region.y..region.y + region.height)
                        .flat_map(|y| {
                            let at = (y * WIDTH + region.x) as usize * bpp;
                            original[at..at + region.width as usize * bpp]
                                .iter()
                                .copied()
                        })
                        .collect();
                    let output = tiff_region::decode(
                        ReadAtReader::new(SliceReadAt::new(cursor.get_ref())),
                        Selection {
                            page: 0,
                            levels: &[],
                            region,
                        },
                        Allowances::default(),
                        || Ok(()),
                    )
                    .unwrap();
                    assert_eq!(output.color, color);
                    assert_eq!(output.sample, sample);
                    assert_eq!(output.orientation, u32::from(orientation));
                    assert_eq!(output.stride, region.width as usize * bpp);
                    assert_eq!(
                        output.bytes, expected,
                        "big={big} compression={compression:?} predictor={predictor:?}"
                    );
                    let allocation = output.bytes.as_ptr();
                    let raster = output.into_pixels().unwrap();
                    let pixels = raster.pixels.as_slice();
                    assert_eq!(pixels.as_strided_bytes().as_ptr(), allocation);
                    assert_eq!(pixels.width(), region.width);
                    assert_eq!(pixels.rows(), region.height);
                    assert_eq!(pixels.as_contiguous_bytes().unwrap(), expected);
                    assert_eq!(
                        raster.orientation,
                        zenpixels::Orientation::from_exif(orientation.try_into().unwrap()).unwrap()
                    );
                    assert_eq!(
                        pixels.descriptor().transfer(),
                        zenpixels::TransferFunction::Unknown
                    );
                    assert_eq!(
                        pixels.descriptor().primaries,
                        zenpixels::ColorPrimaries::Unknown
                    );
                    let mut copy = zenpixels_convert::RowConverter::new(
                        pixels.descriptor(),
                        pixels.descriptor(),
                    )
                    .unwrap();
                    let mut row = vec![0xa5; pixels.stride() + 13];
                    for y in 0..pixels.rows() {
                        copy.try_convert_row(pixels.row(y), &mut row, pixels.width())
                            .unwrap();
                        assert_eq!(&row[..pixels.row(y).len()], pixels.row(y));
                        assert_eq!(&row[pixels.row(y).len()..], &[0xa5; 13]);
                    }
                    let image = raster.into_image().unwrap();
                    image
                        .with_pixels(|pixels| {
                            assert_eq!(pixels.as_strided_bytes().as_ptr(), allocation);
                            assert_eq!(pixels.as_contiguous_bytes().unwrap(), expected);
                        })
                        .unwrap();
                    let mapped = image.map().unwrap();
                    assert_eq!(mapped.plane_count(), 1);
                    let plane = mapped.plane(0).unwrap().unwrap();
                    assert_eq!(plane.meaning, &crate::image_output::Meaning::Color);
                    assert!(plane.samples.code_encoding().is_none());
                    assert_eq!(plane.samples.stride_bytes(), region.width as usize * bpp);
                    assert!(mapped.plane(1).unwrap().is_none());
                }
            }
        }
    }
}

macro_rules! integer_case {
    ($name:ident, $codec:ident, $ty:ty, $color:expr, $sample:ident) => {
        #[test]
        fn $name() {
            // Extrema, high bits and byte-order asymmetry, in every channel and
            // at strip boundaries. Expected bytes never come from a decoder.
            let count = WIDTH * HEIGHT * u32::from($color.num_samples());
            let values: Vec<$ty> = (0..count)
                .map(|i| match i % 7 {
                    0 => <$ty>::MIN,
                    1 => <$ty>::MAX,
                    2 => 0,
                    3 => 1,
                    _ => (u64::from(i).wrapping_mul(0x0102_0304_0506_0709)) as $ty,
                })
                .collect();
            let bytes = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
            qualify::<colortype::$codec>(values, bytes, $color, SampleFormat::$sample);
        }
    };
}

integer_case!(gray_u8, Gray8, u8, ColorType::Gray(8), Uint);
integer_case!(gray_i8, GrayI8, i8, ColorType::Gray(8), Int);
integer_case!(gray_u16, Gray16, u16, ColorType::Gray(16), Uint);
integer_case!(gray_i16, GrayI16, i16, ColorType::Gray(16), Int);
integer_case!(gray_u32, Gray32, u32, ColorType::Gray(32), Uint);
integer_case!(gray_i32, GrayI32, i32, ColorType::Gray(32), Int);
integer_case!(gray_u64, Gray64, u64, ColorType::Gray(64), Uint);
integer_case!(gray_i64, GrayI64, i64, ColorType::Gray(64), Int);
integer_case!(rgb_u8, RGB8, u8, ColorType::RGB(8), Uint);
integer_case!(rgb_u16, RGB16, u16, ColorType::RGB(16), Uint);
integer_case!(rgb_u32, RGB32, u32, ColorType::RGB(32), Uint);
integer_case!(rgb_u64, RGB64, u64, ColorType::RGB(64), Uint);
integer_case!(rgba_u8, RGBA8, u8, ColorType::RGBA(8), Uint);
integer_case!(rgba_u16, RGBA16, u16, ColorType::RGBA(16), Uint);
integer_case!(rgba_u32, RGBA32, u32, ColorType::RGBA(32), Uint);
integer_case!(rgba_u64, RGBA64, u64, ColorType::RGBA(64), Uint);

macro_rules! float_case {
    ($name:ident, $codec:ident, $ty:ty, $color:expr, $bits:expr) => {
        #[test]
        fn $name() {
            let patterns = $bits;
            let count = WIDTH * HEIGHT * u32::from($color.num_samples());
            let values: Vec<$ty> = (0..count)
                .map(|i| <$ty>::from_bits(patterns[i as usize % patterns.len()]))
                .collect();
            let bytes = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
            qualify::<colortype::$codec>(values, bytes, $color, SampleFormat::IEEEFP);
        }
    };
}
// +0, -0, signed infinities, distinct NaN payloads, subnormals, extrema,
// asymmetric finite values. Byte equality preserves all representations.
const F32: [u32; 12] = [
    0,
    0x8000_0000,
    0x7f80_0000,
    0xff80_0000,
    0x7fc0_1234,
    0xffc0_5678,
    1,
    0x8000_0001,
    0x7f7f_ffff,
    0xff7f_ffff,
    0x3f80_0102,
    0xbf12_3456,
];
const F64: [u64; 12] = [
    0,
    0x8000_0000_0000_0000,
    0x7ff0_0000_0000_0000,
    0xfff0_0000_0000_0000,
    0x7ff8_1234_5678_9abc,
    0xfff8_1234_5678_9abc,
    1,
    0x8000_0000_0000_0001,
    0x7fef_ffff_ffff_ffff,
    0xffef_ffff_ffff_ffff,
    0x3ff0_0102_0304_0506,
    0xbfe1_2345_6789_abcd,
];
float_case!(gray_f32, Gray32Float, f32, ColorType::Gray(32), F32);
float_case!(gray_f64, Gray64Float, f64, ColorType::Gray(64), F64);
float_case!(rgb_f32, RGB32Float, f32, ColorType::RGB(32), F32);
float_case!(rgb_f64, RGB64Float, f64, ColorType::RGB(64), F64);
float_case!(rgba_f32, RGBA32Float, f32, ColorType::RGBA(32), F32);
float_case!(rgba_f64, RGBA64Float, f64, ColorType::RGBA(64), F64);
