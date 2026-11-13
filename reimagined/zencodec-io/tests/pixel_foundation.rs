//! Interoperation with the pinned zenpixels 0.3 foundation, through real drivers.
use std::sync::Arc;
use zencodec::decode::{DecodeSpec, Sink};
use zencodec::encode::{EncodeCodec, EncodeEvent, EncodeInput, EncodeSpec};
use zencodec::{FrameHeader, Metadata, Raster};
use zencodec_fixtures::rows::{RowsCodec, bt601_full, planar_raster, yuv420};
use zencodec_io::{Collect, decode, encode};
use zenpixels::plane::{AlignedSamples, AnyPlane, AnyPlaneBuf, PlanarBuffer, PlaneBuf};
use zenpixels::{
    Cicp, ColorContext, ColorProfile, PixelBuffer, PixelDescriptor, PixelSlice, RowAlign,
};

#[test]
fn borrowed_encoding_and_owned_delivery_keep_the_selected_profile() {
    // Deliberately opaque: file CICP must not replace an unrecognized ICC.
    let icc: Arc<[u8]> = Arc::from(&b"unrecognized ICC payload"[..]);
    let context = Arc::new(ColorContext::from_cicp(Cicp::DISPLAY_P3).with_icc(icc.clone()));
    let metadata = Metadata::none()
        .with_icc(icc.clone())
        .with_cicp(Cicp::DISPLAY_P3);
    assert_eq!(metadata.icc.as_deref(), Some(&*icc));
    assert_eq!(metadata.cicp, Some(Cicp::DISPLAY_P3));
    assert!(matches!(context.profile(), Some(ColorProfile::Icc(_))));
    assert_eq!(context.cicp(), None);

    let samples: Vec<u8> = (0..45).map(|i| (i * 17) as u8).collect();
    let pixels = PixelBuffer::from_vec(samples.clone(), 5, 3, PixelDescriptor::RGB8_SRGB)
        .unwrap()
        .with_color_context(context.clone());
    let pointer = pixels.as_slice().row(0).as_ptr();
    let references = Arc::strong_count(&context);
    for _ in 0..8 {
        let encoding = pixels.encoding();
        let copied = encoding;
        assert_eq!(encoding.descriptor(), PixelDescriptor::RGB8_SRGB);
        assert_eq!(copied.icc(), Some(&*icc));
        assert_eq!(copied.cicp(), None);
        assert!(std::ptr::eq(copied.color_context().unwrap(), &*context));
        assert_eq!(Arc::strong_count(&context), references);
    }

    let mut collect = Collect::new();
    collect
        .begin(&FrameHeader::still(Raster::of(&pixels.as_slice())))
        .unwrap();
    collect.frame(pixels).unwrap();
    collect.end().unwrap();
    let frames = collect.into_frames();
    let kept = frames[0].pixels.packed().unwrap();
    assert_eq!(kept.as_slice().row(0).as_ptr(), pointer);
    assert!(Arc::ptr_eq(kept.color_context().unwrap(), &context));
    assert_eq!(kept.as_slice().contiguous_bytes(), samples);
}

#[test]
fn row_alignment_and_minimal_final_row_keep_encoded_samples() {
    let (width, height, descriptor) = (3, 5, PixelDescriptor::RGB8_SRGB);
    let row_bytes = width as usize * descriptor.bytes_per_pixel();
    let samples: Vec<u8> = (0..row_bytes * height as usize)
        .map(|i| (i * 37) as u8)
        .collect();
    let tight = PixelSlice::new(&samples, width, height, row_bytes, descriptor).unwrap();
    let expected = encode(&RowsCodec, EncodeSpec::for_pixels(&tight), tight, None).unwrap();
    for align in [
        RowAlign::Tight,
        RowAlign::Cache64,
        RowAlign::PixelCache64,
        RowAlign::Samples(7),
    ] {
        let mut pixels: PixelBuffer<(), AlignedSamples<u8>> =
            PixelBuffer::try_new_aligned(width, height, descriptor, align).unwrap();
        for y in 0..height {
            pixels
                .as_slice_mut()
                .row_mut(y)
                .copy_from_slice(&samples[y as usize * row_bytes..(y as usize + 1) * row_bytes]);
        }
        let slice = pixels.as_slice();
        let bytes = encode(&RowsCodec, EncodeSpec::for_pixels(&slice), slice, None).unwrap();
        assert_eq!(bytes, expected, "{align:?}");
        let decoded = decode(&RowsCodec, &bytes, &DecodeSpec::new(), None).unwrap();
        assert_eq!(
            decoded.frames[0]
                .pixels
                .packed()
                .unwrap()
                .as_slice()
                .contiguous_bytes(),
            samples
        );
    }

    let stride = descriptor
        .try_aligned_stride(width, RowAlign::Cache64)
        .unwrap();
    let mut minimal = vec![0xe9; stride * (height as usize - 1) + row_bytes];
    for y in 0..height as usize {
        minimal[y * stride..y * stride + row_bytes]
            .copy_from_slice(&samples[y * row_bytes..(y + 1) * row_bytes]);
    }
    let slice = PixelSlice::new(&minimal, width, height, stride, descriptor).unwrap();
    assert!(slice.try_row_with_stride(height - 1).is_none());
    let bytes = encode(&RowsCodec, EncodeSpec::for_pixels(&slice), slice, None).unwrap();
    assert_eq!(bytes, expected);
    for y in 0..height as usize - 1 {
        assert!(
            minimal[y * stride + row_bytes..(y + 1) * stride]
                .iter()
                .all(|b| *b == 0xe9)
        );
    }
}

#[test]
fn owned_planar_buffer_lends_strided_odd_edges_to_the_codec() {
    let plane = |width: usize, height: usize, stride: usize, seed: u8| {
        let mut samples = vec![0xe9; stride * height];
        for y in 0..height {
            for x in 0..width {
                samples[y * stride + x] = ((y * 73 + x * 19 + seed as usize) % 251) as u8;
            }
        }
        AnyPlaneBuf::U8(PlaneBuf::new(samples, width, height, stride).unwrap())
    };
    let picture = PlanarBuffer::new(
        vec![plane(5, 3, 8, 1), plane(3, 2, 5, 2), plane(3, 2, 7, 3)],
        (*yuv420()).clone(),
        5,
        3,
        (*bt601_full()).clone(),
        None,
    )
    .unwrap();
    let pointers: Vec<_> = picture
        .as_slice()
        .planes()
        .map(|p| {
            let AnyPlane::U8(p) = p else {
                panic!("U8 witness")
            };
            p.row(0).unwrap().as_ptr()
        })
        .collect();
    let mut encoder = RowsCodec
        .encoder(&EncodeSpec::new(planar_raster(5, 3)))
        .unwrap();
    assert!(matches!(
        encoder.step(EncodeInput::None, None).unwrap().event,
        EncodeEvent::NeedRows { y: 0, .. }
    ));
    let step = encoder
        .step(EncodeInput::Planes(picture.as_slice()), None)
        .unwrap();
    assert_eq!(step.rows, 3);
    assert_eq!(step.event, EncodeEvent::Done);
    let bytes = encoder.output().take();
    let decoded = decode(
        &RowsCodec,
        &bytes,
        &DecodeSpec::new().with_planar(true),
        None,
    )
    .unwrap();
    assert_eq!(
        decoded.frames[0].header.raster.color.as_deref(),
        Some(picture.as_slice().color_context())
    );
    let collected = decoded.frames[0].pixels.planar().unwrap();
    for (i, (original, actual)) in picture
        .as_slice()
        .planes()
        .zip(&collected.planes)
        .enumerate()
    {
        let (AnyPlane::U8(original), AnyPlane::U8(actual)) = (original, actual.view()) else {
            panic!("U8 witness")
        };
        assert_eq!(original.row(0).unwrap().as_ptr(), pointers[i]);
        for (want, got) in original.rows().zip(actual.rows()) {
            assert_eq!(got, want);
        }
        for row in original.strided_samples().chunks_exact(original.stride()) {
            assert!(row[original.width()..].iter().all(|b| *b == 0xe9));
        }
    }
}
