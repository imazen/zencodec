//! Native CMYK carriage versus output conversion: qualify the public entry point.
use crate::{jxl_packaged_storage, jxl_tests::fixture};
use std::sync::Arc;
use zencodec::{
    collection::{Composition, ImageRequest},
    image::ImageOutput,
    source::{ReadAtShim, SliceReadAt},
    storage::StorageBudget,
};
use zenpixels::{ColorOrigin, PixelBuffer, PixelBufferParts, PixelFormat, PixelSlice};
use zenpixels_convert::{
    Cicp, ConvertError,
    cms_moxcms::MoxCms,
    output::{OutputProfile, finalize_for_output_with},
};

fn native_cmyk(view: Composition) -> (ImageOutput, StorageBudget) {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let (codecs, storage) = jxl_packaged_storage::factories(64, 64);
    let mut images =
        jxl_packaged_storage::open_admitted(&codecs, &mut source, view, 64, &storage).unwrap();
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    let image = images.decode(&id, &ImageRequest::new()).unwrap();
    (image.into_output(), storage)
}

/// The public finalizer currently requires an owned PixelBuffer. Deliberately
/// retain prefix, row padding and a suffix so a contiguous-only copy is caught.
fn stage_for_finalizer(pixels: PixelSlice<'_>) -> PixelBuffer {
    let bpp = pixels.descriptor().bytes_per_pixel();
    let stride = (pixels.width() as usize + 2) * bpp;
    let mut parts = PixelBufferParts::new(
        vec![0xa5; bpp + stride * pixels.rows() as usize + bpp],
        pixels.width(),
        pixels.rows(),
        stride,
        pixels.descriptor(),
    );
    parts.offset = bpp;
    parts.color_context = pixels.color_context().cloned();
    let mut buffer = PixelBuffer::try_from_parts(parts).unwrap();
    for y in 0..pixels.rows() {
        buffer
            .as_slice_mut()
            .row_mut(y)
            .copy_from_slice(pixels.row(y));
    }
    buffer
}

fn icc(buffer: &PixelBuffer) -> Arc<[u8]> {
    buffer
        .color_context()
        .unwrap()
        .icc
        .as_ref()
        .unwrap()
        .clone()
}

#[test]
fn native_cmyk_finalization_identity_preserves_samples_and_current_profile() {
    let (image, storage) = native_cmyk(Composition::Uncomposited);
    let buffer = image.with_pixels(stage_for_finalizer).unwrap();
    assert_eq!(
        buffer.descriptor().pixel_format(),
        PixelFormat::CmykF32Inverted
    );
    let profile = icc(&buffer);
    assert_eq!(&profile[16..20], b"CMYK");
    let origin = ColorOrigin::from_icc(profile.clone());
    let original = buffer.as_slice().as_strided_bytes().to_vec();
    for target in [
        OutputProfile::SameAsOrigin,
        OutputProfile::Icc(profile.clone()),
    ] {
        let ready =
            finalize_for_output_with(&buffer, &origin, target, PixelFormat::CmykF32Inverted, None)
                .unwrap();
        let converted = ready.pixels();
        assert_eq!(converted.descriptor(), buffer.descriptor());
        assert_eq!(
            converted.color_context().unwrap().icc,
            Some(profile.clone())
        );
        assert_eq!(ready.metadata().icc, Some(profile.clone()));
        assert_eq!(ready.metadata().cicp, None);
        for y in 0..converted.rows() {
            assert_eq!(converted.row(y), buffer.as_slice().row(y));
        }
        assert_eq!(buffer.as_slice().as_strided_bytes(), original);
        jxl_packaged_storage::assert_charges(&storage, &[&image]);
    }
}

#[test]
fn native_cmyk_finalization_does_not_convert_float_cmyk_with_moxcms() {
    let (image, storage) = native_cmyk(Composition::Uncomposited);
    let buffer = image.with_pixels(stage_for_finalizer).unwrap();
    let origin = ColorOrigin::from_icc(icc(&buffer));
    let original = buffer.as_slice().as_strided_bytes().to_vec();
    for cms in [
        None,
        Some(&MoxCms as &dyn zenpixels_convert::cms::PluggableCms),
    ] {
        let error = finalize_for_output_with(
            &buffer,
            &origin,
            OutputProfile::Named(Cicp::SRGB),
            PixelFormat::RgbF32,
            cms,
        )
        .err()
        .expect("float CMYK conversion has not been qualified by this backend");
        assert!(
            matches!(error.error(), ConvertError::NeedsCms { .. }),
            "{error:?}"
        );
        assert_eq!(buffer.as_slice().as_strided_bytes(), original);
        jxl_packaged_storage::assert_charges(&storage, &[&image]);
    }
}

#[test]
fn native_cmyk_finalization_copies_are_outside_the_decode_output_pool() {
    let (image, storage) = native_cmyk(Composition::Uncomposited);
    let original_address = image
        .with_pixels(|p| p.as_strided_bytes().as_ptr() as usize)
        .unwrap();
    let buffer = image.with_pixels(stage_for_finalizer).unwrap();
    let ready = finalize_for_output_with(
        &buffer,
        &ColorOrigin::from_icc(icc(&buffer)),
        OutputProfile::SameAsOrigin,
        PixelFormat::CmykF32Inverted,
        None,
    )
    .unwrap();
    let staged_address = buffer.as_slice().as_strided_bytes().as_ptr() as usize;
    let finalized_address = ready.pixels().as_strided_bytes().as_ptr() as usize;
    assert_ne!(original_address, staged_address);
    assert_ne!(original_address, finalized_address);
    assert_ne!(staged_address, finalized_address);
    jxl_packaged_storage::assert_charges(&storage, &[&image]);
    drop(image);
    assert_eq!(
        (storage.reserved_bytes(), storage.reserved_objects()),
        (0, 0)
    );
    for y in 0..buffer.height() {
        assert_eq!(buffer.as_slice().row(y), ready.pixels().row(y));
    }
}

#[test]
fn native_cmyk_finalization_origin_cannot_replace_missing_current_profile() {
    let (image, storage) = native_cmyk(Composition::Uncomposited);
    let buffer = image.with_pixels(stage_for_finalizer).unwrap();
    let origin = ColorOrigin::from_icc(icc(&buffer));
    let mut parts = buffer.into_parts();
    parts.color_context = None;
    let buffer = PixelBuffer::try_from_parts(parts).unwrap();
    let original = buffer.as_slice().as_strided_bytes().to_vec();
    let error = finalize_for_output_with(
        &buffer,
        &origin,
        OutputProfile::SameAsOrigin,
        PixelFormat::CmykF32Inverted,
        Some(&MoxCms),
    )
    .err()
    .expect("provenance is not the current sample interpretation");
    assert!(
        matches!(error.error(), ConvertError::NeedsCms { .. }),
        "{error:?}"
    );
    assert_eq!(buffer.as_slice().as_strided_bytes(), original);
    jxl_packaged_storage::assert_charges(&storage, &[&image]);
}
