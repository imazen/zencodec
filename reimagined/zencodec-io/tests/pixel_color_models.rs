//! PR160 color declarations through the codec's existing sample carriers.
use std::sync::Arc;
use zencodec::decode::{Accept, Sink};
use zencodec::encode::{EncodeCodec, EncodeSpec};
use zencodec::{ErrorKind, FrameHeader, Pass, Raster, Unsupported, copy_rows};
use zencodec_fixtures::rows::RowsCodec;
use zencodec_io::Collect;
use zenpixels::{
    AlphaMode, ChannelLayout, ChannelType, CieXy, ColorContext, ColorProfile, GammaCurve,
    ParametricColor, ParametricPrimaries, ParametricTransfer, PixelBuffer, PixelDescriptor,
    PixelSlice, PixelSliceMut, RenderingIntent, TransferFunction, WhitePoint,
};

#[test]
fn parametric_color_and_intent_survive_owned_frame_delivery() {
    let custom = ParametricColor::rgb(
        ParametricPrimaries::Custom {
            red: CieXy::new(0.67, 0.32),
            green: CieXy::new(0.28, 0.62),
            blue: CieXy::new(0.14, 0.08),
        },
        WhitePoint::Custom(CieXy::new(0.312, 0.329)),
        ParametricTransfer::Gamma(GammaCurve::from_encoding_exponent(0.47)),
    )
    .with_rendering_intent(RenderingIntent::AbsoluteColorimetric);
    for parametric in [
        custom,
        ParametricColor::DCI_P3,
        ParametricColor::PROPHOTO_RGB,
    ] {
        let context = Arc::new(ColorContext::from_parametric(parametric));
        let samples = vec![7, 29, 181, 251, 13, 67];
        let pixels = PixelBuffer::from_vec(samples.clone(), 2, 1, PixelDescriptor::RGB8_SRGB)
            .unwrap()
            .with_color_context(context.clone());
        let pointer = pixels.as_slice().row(0).as_ptr();
        let references = Arc::strong_count(&context);
        let encoding = pixels.encoding();
        assert_eq!(encoding.parametric(), Some(parametric));
        assert_eq!(
            encoding.rendering_intent(),
            Some(parametric.rendering_intent)
        );
        assert_eq!(encoding.icc(), None);
        assert_eq!(encoding.cicp(), None);
        assert_eq!(Arc::strong_count(&context), references);

        let raster = Raster::of(&pixels.as_slice());
        assert!(Arc::ptr_eq(raster.color.as_ref().unwrap(), &context));
        let mut collect = Collect::new();
        collect.begin(&FrameHeader::still(raster)).unwrap();
        collect.frame(pixels).unwrap();
        collect.end().unwrap();
        let frame = &collect.frames()[0];
        let kept = frame.pixels.packed().unwrap();
        assert_eq!(kept.as_slice().row(0).as_ptr(), pointer);
        assert_eq!(kept.as_slice().contiguous_bytes(), samples);
        for color in [
            frame.header.raster.color.as_ref().unwrap(),
            kept.color_context().unwrap(),
        ] {
            assert!(Arc::ptr_eq(color, &context));
            assert!(
                matches!(color.profile(), Some(ColorProfile::Parametric(p)) if *p == parametric)
            );
            assert_eq!(color.rendering_intent(), Some(parametric.rendering_intent));
        }
    }
}

#[test]
fn cmyk_and_cmyka_keep_strided_samples_and_icc_intent() {
    let context = Arc::new(
        ColorContext::from_icc(Arc::<[u8]>::from(&b"opaque CMYK ICC"[..]))
            .with_rendering_intent(RenderingIntent::Saturation),
    );
    for descriptor in [
        PixelDescriptor::cmyk(ChannelType::U8),
        PixelDescriptor::cmyk_inverted(ChannelType::U8),
        PixelDescriptor::cmyka(ChannelType::U8, AlphaMode::Straight),
        PixelDescriptor::cmyka_inverted(ChannelType::U8, AlphaMode::Straight),
    ] {
        let (width, height) = (3, 5);
        let row_bytes = width as usize * descriptor.bytes_per_pixel();
        let stride = row_bytes + 7;
        // There is no trailing padding on the last row.
        let mut samples = vec![0xe9; stride * (height as usize - 1) + row_bytes];
        let mut expected = Vec::new();
        for y in 0..height as usize {
            for x in 0..row_bytes {
                let sample = ((y * 73 + x * 19) % 251) as u8;
                samples[y * stride + x] = sample;
                expected.push(sample);
            }
        }
        let original = samples.clone();
        let slice = PixelSlice::new(&samples, width, height, stride, descriptor)
            .unwrap()
            .with_color_context(context.clone());
        let mut collect = Collect::new();
        collect
            .begin(&FrameHeader::still(Raster::of(&slice)))
            .unwrap();
        collect.rows(0, slice.sub_rows(0, 2), Pass::Final).unwrap();
        collect.rows(2, slice.sub_rows(2, 3), Pass::Final).unwrap();
        collect.end().unwrap();
        let frame = &collect.frames()[0];
        let kept = frame.pixels.packed().unwrap();
        assert_eq!(kept.descriptor(), descriptor);
        assert_eq!(kept.as_slice().contiguous_bytes(), expected);
        let color = frame.header.raster.color.as_ref().unwrap();
        assert!(Arc::ptr_eq(color, &context));
        assert_eq!(color.icc(), Some(&b"opaque CMYK ICC"[..]));
        assert_eq!(color.rendering_intent(), Some(RenderingIntent::Saturation));
        assert_eq!(samples, original);
    }
}

#[test]
fn opposite_ink_polarity_requires_a_declared_conversion() {
    for (a, b) in [
        (PixelDescriptor::CMYK8, PixelDescriptor::CMYK8_INVERTED),
        (
            PixelDescriptor::cmyka(ChannelType::U8, AlphaMode::Straight),
            PixelDescriptor::cmyka_inverted(ChannelType::U8, AlphaMode::Straight),
        ),
    ] {
        for (native, wanted) in [(a, b), (b, a)] {
            assert_eq!(
                Accept::require(&[wanted]).choose(native, &[]),
                Err(Unsupported::PixelFormat)
            );
            assert_eq!(Accept::prefer(&[wanted]).choose(native, &[]), Ok(native));
            let row_bytes = 3 * native.bytes_per_pixel();
            let samples = vec![11; row_bytes];
            let src = PixelSlice::new(&samples, 3, 1, row_bytes, native).unwrap();
            let mut canary = vec![0xe9; row_bytes];
            let mut dst = PixelSliceMut::new(&mut canary, 3, 1, row_bytes, wanted).unwrap();
            let error = copy_rows(&src, &mut dst).unwrap_err();
            assert_eq!(error.error().kind(), ErrorKind::Protocol);
            assert!(canary.iter().all(|b| *b == 0xe9));
        }
    }
}

#[test]
fn new_pixel_models_do_not_imply_codec_support() {
    for descriptor in [
        PixelDescriptor::CMYK8,
        PixelDescriptor::CMYK8_INVERTED,
        PixelDescriptor::cmyka(ChannelType::U8, AlphaMode::Straight),
        PixelDescriptor::new(
            ChannelType::F32,
            ChannelLayout::Oklch,
            None,
            TransferFunction::Unknown,
        ),
        PixelDescriptor::new(
            ChannelType::F32,
            ChannelLayout::Hsl,
            None,
            TransferFunction::Srgb,
        ),
    ] {
        let spec = EncodeSpec::new(Raster::packed(3, 2, descriptor));
        let error = match RowsCodec.encoder(&spec) {
            Err(error) => error,
            Ok(_) => panic!("fixture accepted unsupported format {descriptor:?}"),
        };
        assert_eq!(
            error.error().kind(),
            ErrorKind::Unsupported(Unsupported::PixelFormat)
        );
    }
}
