//! A profile-aware color stage writes directly into admitted destination rows.
//! Auxiliary mapping/publication and true CMYK color arithmetic are separate.
use crate::{jxl_packaged_storage, jxl_tests::fixture};
use std::sync::Arc;
use zencodec::{
    collection::{Cause, Composition, ImageRequest},
    image::ImageOutput,
    source::{ReadAtShim, SliceReadAt},
    storage::{SharedStorage, StorageBudget, StorageError},
};
use zenpixels::{ColorContext, PixelSliceMut};

#[cfg(feature = "cms_oracle")]
#[path = "jxl_color_oracle_tests.rs"]
mod oracle;

/// Retain the complete native image only when samples AND current signaling
/// already satisfy the plan. Otherwise the caller still owns the source.
fn retain_identity(
    image: &ImageOutput,
    plan: &OutputPlan,
) -> Result<Option<(ImageOutput, zenpixels_convert::output::OutputMetadata)>, Cause> {
    let unchanged = image.with_pixels(|pixels| {
        plan.borrow_rows(pixels.clone()).map(|borrowed| {
            borrowed.is_some_and(|view| {
                view.descriptor() == pixels.descriptor()
                    && view.color_context() == pixels.color_context()
            })
        })
    })??;
    Ok(unchanged.then(|| (image.clone(), plan.metadata().clone())))
}

struct ColorStage {
    data: SharedStorage<u32>,
    width: u32,
    rows: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    context: Arc<ColorContext>,
}
impl ColorStage {
    fn pixels(&self) -> PixelSlice<'_> {
        PixelSlice::new(
            bytemuck::cast_slice(self.data.as_slice()),
            self.width,
            self.rows,
            self.stride,
            self.descriptor,
        )
        .unwrap()
        .with_color_context(self.context.clone())
    }
}
impl zencodec::image::PackedImageOwner for ColorStage {
    fn pixels(&self) -> Result<PixelSlice<'_>, zencodec::At<zencodec::image::OutputError>> {
        Ok(ColorStage::pixels(self))
    }
}

// EXAMPLE: native_required_layout
use zencodec::At;
use zenpixels::{ColorOrigin, PixelDescriptor, PixelSlice};
use zenpixels_convert::{ConvertError, ConvertOptions, OutputPlan, OutputProfile};

fn plan_required_layout(
    source: &PixelSlice<'_>,
    origin: &ColorOrigin,
    required: PixelDescriptor,
) -> Result<OutputPlan, At<ConvertError>> {
    // ANCHOR: native_required_layout
    OutputPlan::new_with_descriptor(
        source.descriptor(),
        source.color_context().cloned(),
        origin,
        OutputProfile::SameAsOrigin,
        required,
        &ConvertOptions::forbid_lossy().with_clip_out_of_gamut(false),
        None,
    )
    // END: native_required_layout
}
// END EXAMPLE: native_required_layout

fn identity(pixels: &PixelSlice<'_>) -> OutputPlan {
    let profile = pixels
        .color_context()
        .unwrap()
        .icc
        .as_ref()
        .unwrap()
        .clone();
    plan_required_layout(pixels, &ColorOrigin::from_icc(profile), pixels.descriptor()).unwrap()
}

// A color-stage prototype, not a complete converted ImageOutput: a caller must
// still decide what to do with native auxiliary planes before publication.
fn convert(
    source: PixelSlice<'_>,
    plan: &mut OutputPlan,
    storage: &StorageBudget,
) -> Result<ColorStage, Cause> {
    let descriptor = plan.output_descriptor();
    let (width, rows) = (source.width(), source.rows());
    let stride = (width as usize)
        .checked_add(2)
        .and_then(|w| w.checked_mul(descriptor.bytes_per_pixel()))
        .ok_or(StorageError::SizeOverflow)?;
    let bytes = stride
        .checked_mul(rows as usize)
        .ok_or(StorageError::SizeOverflow)?;
    assert_eq!(bytes % 4, 0, "native fixture uses 32-bit channels");
    plan.prepare(width)?;
    let mut data = storage.filled(bytes / 4, 0xa5a5_a5a5u32)?;
    let destination = PixelSliceMut::new(
        bytemuck::cast_slice_mut(data.as_mut_slice()),
        width,
        rows,
        stride,
        descriptor,
    )?;
    let converted = plan.convert_rows(source, destination)?;
    let context = converted.color_context().unwrap().clone();
    assert_eq!(context.icc, plan.metadata().icc);
    assert_eq!(context.cicp, plan.metadata().cicp);
    Ok(ColorStage {
        data: data.freeze(),
        width,
        rows,
        stride,
        descriptor,
        context,
    })
}

#[test]
fn native_output_plan_all_layers_write_exact_color_into_admitted_storage() {
    let mut total = 0;
    for name in [
        "conformance_test_images/cmyk_layers.jxl",
        "multiple_layers_noise_spline.jxl",
    ] {
        let bytes = fixture(name);
        let mut source = ReadAtShim(SliceReadAt::new(&bytes));
        let (codecs, input_pool) = jxl_packaged_storage::factories(64, 64);
        let mut images = jxl_packaged_storage::open_admitted(
            &codecs,
            &mut source,
            Composition::Uncomposited,
            64,
            &input_pool,
        )
        .unwrap();
        let mut cursor = images.images().unwrap();
        while let Some(id) = images.next(&mut cursor).unwrap() {
            let image = images.decode(&id, &ImageRequest::new()).unwrap();
            let output_pool = StorageBudget::new(256 * 1024 * 1024, 1);
            let color = image
                .with_pixels(|pixels| {
                    let mut plan = identity(&pixels);
                    let color = convert(pixels.clone(), &mut plan, &output_pool).unwrap();
                    let view = color.pixels();
                    assert_ne!(
                        view.as_strided_bytes().as_ptr(),
                        pixels.as_strided_bytes().as_ptr()
                    );
                    assert_eq!(view.descriptor(), pixels.descriptor());
                    assert_eq!(view.color_context(), pixels.color_context());
                    for y in 0..pixels.rows() {
                        assert_eq!(view.row(y), pixels.row(y));
                        let active =
                            pixels.width() as usize * pixels.descriptor().bytes_per_pixel();
                        let all = bytemuck::cast_slice::<u32, u8>(color.data.as_slice());
                        assert!(
                            all[y as usize * color.stride + active
                                ..(y as usize + 1) * color.stride]
                                .iter()
                                .all(|b| *b == 0xa5)
                        );
                    }
                    color
                })
                .unwrap();
            jxl_packaged_storage::assert_charges(&input_pool, &[&image]);
            assert_eq!(
                (output_pool.reserved_bytes(), output_pool.reserved_objects()),
                (color.stride * color.rows as usize, 1)
            );
            drop(image);
            assert_eq!(
                (input_pool.reserved_bytes(), input_pool.reserved_objects()),
                (0, 0)
            );
            let held = color.data.clone();
            drop(color);
            assert_eq!(output_pool.reserved_bytes(), held.charged_bytes());
            drop(held);
            assert_eq!(
                (output_pool.reserved_bytes(), output_pool.reserved_objects()),
                (0, 0)
            );
            total += 1;
        }
    }
    assert_eq!(total, 9);
}

#[test]
fn native_output_plan_admission_and_interpretation_refusal_release_destination() {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let (codecs, input_pool) = jxl_packaged_storage::factories(64, 64);
    let mut images = jxl_packaged_storage::open_admitted(
        &codecs,
        &mut source,
        Composition::Uncomposited,
        64,
        &input_pool,
    )
    .unwrap();
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    let image = images.decode(&id, &ImageRequest::new()).unwrap();
    image
        .with_pixels(|pixels| {
            let mut plan = identity(&pixels);
            for pool in [
                StorageBudget::new(0, 1),
                StorageBudget::new(256 * 1024 * 1024, 0),
            ] {
                let error = convert(pixels.clone(), &mut plan, &pool).err().unwrap();
                assert!(zencodec::find_cause::<StorageError>(error.as_ref()).is_some());
                assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
            }
            let pool = StorageBudget::new(256 * 1024 * 1024, 1);
            let changed = pixels
                .clone()
                .with_color_context(Arc::new(ColorContext::default()));
            let error = convert(changed, &mut plan, &pool).err().unwrap();
            assert_eq!(
                zencodec::find_cause::<zenpixels_convert::ConvertError>(error.as_ref()),
                Some(&zenpixels_convert::ConvertError::SourceInterpretationChanged)
            );
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
            let color = convert(pixels, &mut plan, &pool).unwrap();
            assert_eq!(pool.reserved_objects(), 1);
            drop(color);
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
        })
        .unwrap();
    jxl_packaged_storage::assert_charges(&input_pool, &[&image]);
}

#[test]
fn native_output_plan_shared_pool_counts_source_and_destination_together() {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let decode = |pool: &StorageBudget| {
        let mut source = ReadAtShim(SliceReadAt::new(&bytes));
        let (codecs, _) = jxl_packaged_storage::factories(64, 64);
        let mut images = jxl_packaged_storage::open_admitted(
            &codecs,
            &mut source,
            Composition::Uncomposited,
            64,
            pool,
        )
        .unwrap();
        let mut cursor = images.images().unwrap();
        let id = images.next(&mut cursor).unwrap().unwrap();
        images
            .decode(&id, &ImageRequest::new())
            .unwrap()
            .into_output()
    };
    let measured = StorageBudget::new(256 * 1024 * 1024, 64);
    let source = decode(&measured);
    let native_bytes = measured.reserved_bytes();
    let native_objects = measured.reserved_objects();
    let extra = source
        .with_pixels(|pixels| {
            let mut plan = identity(&pixels);
            convert(pixels, &mut plan, &measured).unwrap()
        })
        .unwrap();
    let extra_bytes = extra.data.charged_bytes();
    drop(extra);
    drop(source);
    assert_eq!(
        (measured.reserved_bytes(), measured.reserved_objects()),
        (0, 0)
    );

    for sufficient in [false, true] {
        let capacity = native_bytes + if sufficient { extra_bytes } else { 0 };
        let pool = StorageBudget::new(capacity, native_objects + 1);
        let source = decode(&pool);
        let result = source
            .with_pixels(|pixels| {
                let mut plan = identity(&pixels);
                convert(pixels, &mut plan, &pool)
            })
            .unwrap();
        if sufficient {
            let color = result.unwrap();
            assert_eq!(
                (pool.reserved_bytes(), pool.reserved_objects()),
                (native_bytes + extra_bytes, native_objects + 1)
            );
            drop(source);
            assert_eq!(
                (pool.reserved_bytes(), pool.reserved_objects()),
                (extra_bytes, 1)
            );
            drop(color);
        } else {
            let error = result
                .err()
                .expect("source-only pool admitted another full color plane");
            assert!(zencodec::find_cause::<StorageError>(error.as_ref()).is_some());
            jxl_packaged_storage::assert_charges(&pool, &[&source]);
            let plan = source.with_pixels(|pixels| identity(&pixels)).unwrap();
            let (retained, _) = retain_identity(&source, &plan).unwrap().unwrap();
            drop(source);
            assert_eq!(pool.reserved_bytes(), native_bytes);
            jxl_packaged_storage::assert_charges(&pool, &[&retained]);
            drop(retained);
        }
        assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    }
}

#[test]
fn native_output_plan_identity_retains_all_native_planes_and_last_owner_charge() {
    use crate::output_tests::mapped_planes;
    let mut total = 0;
    for name in [
        "conformance_test_images/cmyk_layers.jxl",
        "multiple_layers_noise_spline.jxl",
    ] {
        let bytes = fixture(name);
        let mut source = ReadAtShim(SliceReadAt::new(&bytes));
        let (codecs, pool) = jxl_packaged_storage::factories(64, 64);
        let mut images = jxl_packaged_storage::open_admitted(
            &codecs,
            &mut source,
            Composition::Uncomposited,
            64,
            &pool,
        )
        .unwrap();
        let mut cursor = images.images().unwrap();
        let mut held = Vec::new();
        let mut expected = Vec::new();
        while let Some(id) = images.next(&mut cursor).unwrap() {
            let image = images
                .decode(&id, &ImageRequest::new())
                .unwrap()
                .into_output();
            expected.push(mapped_planes(&image));
            let before = (pool.reserved_bytes(), pool.reserved_objects());
            let plan = image.with_pixels(|pixels| identity(&pixels)).unwrap();
            let (retained, metadata) = retain_identity(&image, &plan).unwrap().unwrap();
            assert!(std::ptr::eq(
                image.interpretation(),
                retained.interpretation()
            ));
            let original_map = image.map().unwrap();
            let retained_map = retained.map().unwrap();
            assert_eq!(original_map.plane_count(), retained_map.plane_count());
            for index in 0..original_map.plane_count() {
                let original = original_map.plane(index).unwrap().unwrap();
                let retained = retained_map.plane(index).unwrap().unwrap();
                assert_eq!(original.meaning, retained.meaning);
                let zencodec::image::Row::Bytes(a) = original.samples.row(0).unwrap() else {
                    panic!("native F32 bytes")
                };
                let zencodec::image::Row::Bytes(b) = retained.samples.row(0).unwrap() else {
                    panic!("native F32 bytes")
                };
                assert_eq!(a.as_ptr(), b.as_ptr());
            }
            retained
                .with_pixels(|pixels| {
                    let context = pixels.color_context().unwrap();
                    assert_eq!(metadata.icc, context.icc);
                    assert_eq!(metadata.cicp, context.cicp);
                })
                .unwrap();
            drop(original_map);
            drop(retained_map);
            drop(image);
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), before);
            held.push(retained);
            total += 1;
        }
        drop(images);
        drop(bytes);
        drop(codecs);
        jxl_packaged_storage::assert_charges(&pool, &held.iter().collect::<Vec<_>>());
        for (image, samples) in held.iter().zip(expected) {
            assert_eq!(mapped_planes(image), samples);
        }
        drop(held);
        assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    }
    assert_eq!(total, 9);
}

#[test]
fn native_output_plan_owner_retention_requires_matching_signaling_too() {
    use zencodec::image::{ImageGeometry, ImageInterpretation, Rect};
    use zenpixels::{Cicp, Orientation, PixelBuffer, PixelFormat};
    let pixels = PixelBuffer::from_vec(vec![7, 19, 121], 1, 1, PixelDescriptor::RGB8_SRGB).unwrap();
    let geometry = ImageGeometry {
        canvas_width: 1,
        canvas_height: 1,
        region: Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
    };
    let image = ImageOutput::packed(
        pixels,
        vec![],
        ImageInterpretation::packed(geometry, Orientation::Identity),
    )
    .unwrap();
    for format in [PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let plan = OutputPlan::new(
            PixelDescriptor::RGB8_SRGB,
            None,
            &ColorOrigin::assumed(),
            OutputProfile::Named(Cicp::SRGB),
            format,
            &ConvertOptions::forbid_lossy(),
            None,
        )
        .unwrap();
        image
            .with_pixels(|pixels| {
                let borrowed = plan.borrow_rows(pixels.clone()).unwrap();
                assert_eq!(borrowed.is_some(), format == PixelFormat::Rgb8);
                if let Some(view) = borrowed {
                    assert_eq!(
                        view.as_strided_bytes().as_ptr(),
                        pixels.as_strided_bytes().as_ptr()
                    );
                    assert_eq!(view.color_context().unwrap().cicp, Some(Cicp::SRGB));
                }
            })
            .unwrap();
        assert!(retain_identity(&image, &plan).unwrap().is_none());
        // Declining owner reuse must leave the original source available to the
        // conversion/relabeling adapter, with its original interpretation.
        image
            .with_pixels(|pixels| {
                assert_eq!(pixels.row(0), &[7, 19, 121]);
                assert!(pixels.color_context().is_none());
            })
            .unwrap();
    }
}

#[test]
fn native_output_owner_keeps_auxiliaries_and_interpretation_through_color_replacement() {
    use zencodec::{
        image::{Row, Samples},
        operation::{ImageFacts, PolicySet, resolve_image},
    };
    let mut total = 0;
    for (name, view, count) in [
        (
            "conformance_test_images/cmyk_layers.jxl",
            Composition::Uncomposited,
            4,
        ),
        (
            "multiple_layers_noise_spline.jxl",
            Composition::Uncomposited,
            5,
        ),
        (
            "conformance_test_images/spot.jxl",
            Composition::Composited,
            1,
        ),
        ("extra_channels.jxl", Composition::Composited, 1),
    ] {
        let bytes = fixture(name);
        let mut source = ReadAtShim(SliceReadAt::new(&bytes));
        let (codecs, pool) = jxl_packaged_storage::factories(64, 64);
        let mut images =
            jxl_packaged_storage::open_admitted(&codecs, &mut source, view, 64, &pool).unwrap();
        let mut cursor = images.images().unwrap();
        for _ in 0..count {
            let id = images
                .next(&mut cursor)
                .unwrap()
                .expect("required native selection");
            let image = images
                .decode(&id, &ImageRequest::new())
                .unwrap()
                .into_output();
            let before = ImageFacts::new(&image);
            let input_charge = (pool.reserved_bytes(), pool.reserved_objects());
            let auxiliary_bytes = {
                let map = image.map().unwrap();
                (1..map.plane_count())
                    .map(|index| {
                        let plane = map.plane(index).unwrap().unwrap();
                        plane.samples.stride_bytes() * plane.samples.dimensions().1
                    })
                    .sum::<usize>()
            };
            let color_pool = StorageBudget::new(256 * 1024 * 1024, 1);
            let color = image
                .with_pixels(|pixels| {
                    let mut plan = identity(&pixels);
                    convert(pixels, &mut plan, &color_pool).unwrap()
                })
                .unwrap();
            let original = image.clone();
            let replaced = image.replace_packed_color(color).unwrap();
            assert!(std::ptr::eq(
                original.interpretation(),
                replaced.interpretation()
            ));
            let after = ImageFacts::new(&replaced);
            assert_eq!(
                (
                    before.planes,
                    before.auxiliaries,
                    before.applied_spot_colors
                ),
                (after.planes, after.auxiliaries, after.applied_spot_colors)
            );
            {
                let a = original.map().unwrap();
                let b = replaced.map().unwrap();
                assert_eq!(a.plane_count(), b.plane_count());
                for index in 0..a.plane_count() {
                    let a = a.plane(index).unwrap().unwrap();
                    let b = b.plane(index).unwrap().unwrap();
                    assert_eq!(a.meaning, b.meaning);
                    assert_eq!(a.samples.dimensions(), b.samples.dimensions());
                    if let (Samples::Pixels(a), Samples::Pixels(b)) = (&a.samples, &b.samples) {
                        assert_eq!(a.descriptor(), b.descriptor());
                        assert_eq!(a.color_context(), b.color_context());
                    }
                    for y in 0..a.samples.dimensions().1 {
                        let (Some(Row::Bytes(a)), Some(Row::Bytes(b))) =
                            (a.samples.row(y), b.samples.row(y))
                        else {
                            panic!()
                        };
                        assert_eq!(a, b);
                        assert_eq!(a.as_ptr() == b.as_ptr(), index != 0);
                    }
                }
            }
            assert_eq!(
                (pool.reserved_bytes(), pool.reserved_objects()),
                input_charge
            );
            drop(original);
            assert_eq!(
                (pool.reserved_bytes(), pool.reserved_objects()),
                (auxiliary_bytes, before.auxiliaries)
            );
            let published = resolve_image(
                replaced,
                &mut PolicySet::new(),
                PolicySet::new(),
                &zencodec::Unstoppable,
            )
            .unwrap();
            assert_eq!(color_pool.reserved_objects(), 1);
            drop(published);
            assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
            assert_eq!(
                (color_pool.reserved_bytes(), color_pool.reserved_objects()),
                (0, 0)
            );
            total += 1;
        }
    }
    assert_eq!(total, 11);
}

#[test]
fn native_output_owner_required_descriptor_preserves_premultiplication() {
    let bytes = fixture("conformance_test_images/alpha_premultiplied.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let (codecs, pool) = jxl_packaged_storage::factories(64, 64);
    let mut images = jxl_packaged_storage::open_admitted(
        &codecs,
        &mut source,
        Composition::Composited,
        64,
        &pool,
    )
    .unwrap();
    let image = images
        .first_image(&ImageRequest::new())
        .unwrap()
        .unwrap()
        .into_output();
    let plan = image.with_pixels(|pixels| identity(&pixels)).unwrap();
    let charged = (pool.reserved_bytes(), pool.reserved_objects());
    let (retained, metadata) = retain_identity(&image, &plan).unwrap().unwrap();
    assert_eq!(
        crate::output_tests::mapped_planes(&retained),
        crate::output_tests::mapped_planes(&image)
    );
    assert_eq!(metadata.icc, plan.metadata().icc);
    assert_eq!(metadata.cicp, plan.metadata().cicp);
    #[allow(deprecated)]
    {
        assert!(metadata.hdr.is_none());
        assert!(plan.metadata().hdr.is_none());
    }
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), charged);
    drop(retained);
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), charged);
    let destination = StorageBudget::new(256 * 1024 * 1024, 1);
    let owner = image
        .with_pixels(|pixels| {
            assert_eq!(
                pixels.descriptor().alpha(),
                Some(zenpixels::AlphaMode::Premultiplied)
            );
            let error = OutputPlan::new(
                pixels.descriptor(),
                pixels.color_context().cloned(),
                &ColorOrigin::from_icc(pixels.color_context().unwrap().icc.clone().unwrap()),
                OutputProfile::SameAsOrigin,
                pixels.descriptor().pixel_format(),
                &ConvertOptions::forbid_lossy(),
                None,
            )
            .unwrap_err();
            // PixelFormat chooses straight alpha. SameAsOrigin means the profile,
            // not unchanged association. Preserve the actual refusal as a contract
            // probe for the format-only convenience's established behavior.
            assert!(matches!(
                error.error(),
                zenpixels_convert::ConvertError::NeedsCms { .. }
            ));
            assert_eq!(destination.reserved_bytes(), 0);
            let mut plan = identity(&pixels);
            let borrowed = plan.borrow_rows(pixels.clone()).unwrap().unwrap();
            assert_eq!(
                borrowed.as_strided_bytes().as_ptr(),
                pixels.as_strided_bytes().as_ptr()
            );
            assert_eq!(borrowed.descriptor(), pixels.descriptor());
            assert_eq!(borrowed.color_context(), pixels.color_context());
            let color = convert(pixels.clone(), &mut plan, &destination).unwrap();
            for y in 0..pixels.rows() {
                assert_eq!(color.pixels().row(y), pixels.row(y));
                let active = pixels.width() as usize * pixels.descriptor().bytes_per_pixel();
                assert!(
                    bytemuck::cast_slice::<u32, u8>(color.data.as_slice())
                        [y as usize * color.stride + active..(y as usize + 1) * color.stride]
                        .iter()
                        .all(|b| *b == 0xa5)
                );
            }
            color
        })
        .unwrap();
    let original = image.clone();
    let derived = image.replace_packed_color(owner).unwrap();
    assert_eq!(
        &crate::output_tests::mapped_planes(&original)[1..],
        &crate::output_tests::mapped_planes(&derived)[1..]
    );
    original
        .with_pixels(|a| {
            derived
                .with_pixels(|b| {
                    assert_eq!(a.descriptor(), b.descriptor());
                    assert_eq!(a.color_context(), b.color_context());
                    for y in 0..a.rows() {
                        assert_eq!(a.row(y), b.row(y));
                    }
                })
                .unwrap()
        })
        .unwrap();
    drop(original);
    let retained = derived.clone();
    drop(derived);
    assert_eq!(destination.reserved_objects(), 1);
    drop(retained);
    assert_eq!(
        (destination.reserved_bytes(), destination.reserved_objects()),
        (0, 0)
    );
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
}

#[test]
fn native_output_plan_adding_alpha_preserves_native_color_bits_with_or_without_cms() {
    use zenpixels::PixelFormat;
    use zenpixels_convert::{cms::PluggableCms, cms_moxcms::MoxCms};
    let bytes = fixture("conformance_test_images/animation_spline.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let (codecs, pool) = jxl_packaged_storage::factories(64, 64);
    let mut images = jxl_packaged_storage::open_admitted(
        &codecs,
        &mut source,
        Composition::Composited,
        64,
        &pool,
    )
    .unwrap();
    let image = images
        .first_image(&ImageRequest::new())
        .unwrap()
        .unwrap()
        .into_output();
    image
        .with_pixels(|pixels| {
            assert_eq!(pixels.descriptor().pixel_format(), PixelFormat::RgbF32);
            let profile = pixels.color_context().unwrap().icc.clone().unwrap();
            for cms in [None, Some(&MoxCms as &dyn PluggableCms)] {
                let mut plan = OutputPlan::new(
                    pixels.descriptor(),
                    pixels.color_context().cloned(),
                    &ColorOrigin::from_icc(profile.clone()),
                    OutputProfile::SameAsOrigin,
                    PixelFormat::RgbaF32,
                    &ConvertOptions::forbid_lossy(),
                    cms,
                )
                .unwrap();
                assert!(plan.borrow_rows(pixels.clone()).unwrap().is_none());
                let destination_pool = StorageBudget::new(256 * 1024 * 1024, 1);
                let color = convert(pixels.clone(), &mut plan, &destination_pool).unwrap();
                let view = color.pixels();
                assert_eq!(view.color_context().unwrap().icc, Some(profile.clone()));
                for y in 0..pixels.rows() {
                    for (source, destination) in pixels
                        .row(y)
                        .as_chunks::<12>()
                        .0
                        .iter()
                        .zip(view.row(y).as_chunks::<16>().0.iter())
                    {
                        assert_eq!(
                            &destination[..12],
                            source,
                            "format-only conversion changed native RGB bits"
                        );
                        assert_eq!(&destination[12..], &1.0f32.to_ne_bytes());
                    }
                }
                drop(color);
                assert_eq!(
                    (
                        destination_pool.reserved_bytes(),
                        destination_pool.reserved_objects()
                    ),
                    (0, 0)
                );
            }
        })
        .unwrap();
}

#[derive(Debug)]
struct InjectedRow(usize);
impl std::fmt::Display for InjectedRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("injected conversion failure")
    }
}
impl std::error::Error for InjectedRow {}

struct FailingCms {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    unwind: bool,
}
impl zenpixels_convert::cms::PluggableCms for FailingCms {
    fn build_source_transform(
        &self,
        _: zenpixels::ColorProfileSource<'_>,
        _: zenpixels::ColorProfileSource<'_>,
        _: zenpixels::PixelFormat,
        _: zenpixels::PixelFormat,
        _: &ConvertOptions,
    ) -> Option<
        Result<
            Box<dyn zenpixels_convert::cms::RowTransformMut>,
            zencodec::At<zenpixels_convert::cms::CmsPluginError>,
        >,
    > {
        Some(Ok(Box::new(Self {
            calls: self.calls.clone(),
            unwind: self.unwind,
        })))
    }
}
impl zenpixels_convert::cms::RowTransformMut for FailingCms {
    fn prepare(
        &mut self,
        _: u32,
    ) -> Result<(), zencodec::At<zenpixels_convert::cms::CmsPluginError>> {
        Ok(())
    }
    fn transform_row(&mut self, _: &[u8], _: &mut [u8], _: u32) {
        panic!("fallible hook must be used")
    }
    fn try_transform_row(
        &mut self,
        _: &[u8],
        destination: &mut [u8],
        _: u32,
    ) -> Result<(), zencodec::At<zenpixels_convert::cms::CmsPluginError>> {
        let row = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        destination.fill(0x33);
        if row == 2 {
            assert!(
                !self.unwind,
                "controlled CMS unwind after destination writes"
            );
            return Err(zencodec::At::wrap(
                zenpixels_convert::cms::CmsPluginError::new(InjectedRow(row)),
            ));
        }
        Ok(())
    }
}

#[test]
fn native_output_plan_late_backend_failure_or_unwind_drops_unpublished_storage() {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let (codecs, input_pool) = jxl_packaged_storage::factories(64, 64);
    let mut images = jxl_packaged_storage::open_admitted(
        &codecs,
        &mut source,
        Composition::Uncomposited,
        64,
        &input_pool,
    )
    .unwrap();
    let mut cursor = images.images().unwrap();
    let id = images.next(&mut cursor).unwrap().unwrap();
    let image = images.decode(&id, &ImageRequest::new()).unwrap();
    image
        .with_pixels(|pixels| {
            let original = pixels.as_strided_bytes().to_vec();
            for unwind in [false, true] {
                let cms = FailingCms {
                    calls: Arc::default(),
                    unwind,
                };
                let mut plan = OutputPlan::new(
                    pixels.descriptor(),
                    pixels.color_context().cloned(),
                    &ColorOrigin::assumed(),
                    OutputProfile::Icc(Arc::from(&b"fault injection target"[..])),
                    pixels.descriptor().pixel_format(),
                    &ConvertOptions::forbid_lossy(),
                    Some(&cms),
                )
                .unwrap();
                let pool = StorageBudget::new(256 * 1024 * 1024, 1);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    convert(pixels.clone(), &mut plan, &pool)
                }));
                if unwind {
                    assert!(result.is_err());
                } else {
                    let error = result.unwrap().err().expect("partial output was returned");
                    assert_eq!(
                        zencodec::find_cause::<InjectedRow>(error.as_ref())
                            .unwrap()
                            .0,
                        2
                    );
                }
                assert_eq!(cms.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
                assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
                assert_eq!(pixels.as_strided_bytes(), original);
            }
        })
        .unwrap();
    jxl_packaged_storage::assert_charges(&input_pool, &[&image]);
}

#[test]
fn native_output_plan_exact_samples_never_require_a_cms() {
    let bytes = fixture("conformance_test_images/animation_spline.jxl");
    let mut source = ReadAtShim(SliceReadAt::new(&bytes));
    let (codecs, pool) = jxl_packaged_storage::factories(64, 64);
    let mut images = jxl_packaged_storage::open_admitted(
        &codecs,
        &mut source,
        Composition::Composited,
        64,
        &pool,
    )
    .unwrap();
    let image = images
        .first_image(&ImageRequest::new())
        .unwrap()
        .unwrap()
        .into_output();
    image
        .with_pixels(|pixels| {
            let profile = pixels.color_context().unwrap().icc.clone().unwrap();
            let current = pixels.color_context().cloned();
            let descriptor = pixels.descriptor();
            let mut identity = OutputPlan::new_preserving_samples(
                descriptor,
                current.clone(),
                &ColorOrigin::assumed(),
                OutputProfile::Icc(profile.clone()),
                descriptor,
            )
            .unwrap();
            identity.prepare(pixels.width()).unwrap();
            let borrowed = identity.borrow_rows(pixels.clone()).unwrap().unwrap();
            assert_eq!(borrowed.row(0).as_ptr(), pixels.row(0).as_ptr());
            for row in 0..pixels.rows() {
                assert_eq!(borrowed.row(row), pixels.row(row));
            }
            let output = zenpixels::PixelFormat::RgbaF32
                .descriptor()
                .with_primaries(descriptor.primaries)
                .with_transfer(descriptor.transfer());
            let mut expanded = OutputPlan::new_preserving_samples(
                descriptor,
                current,
                &ColorOrigin::assumed(),
                OutputProfile::Icc(profile),
                output,
            )
            .unwrap();
            let storage = StorageBudget::new(256 * 1024 * 1024, 1);
            let converted = convert(pixels.clone(), &mut expanded, &storage).unwrap();
            let view = converted.pixels();
            for y in 0..pixels.rows() {
                for (before, after) in pixels
                    .row(y)
                    .as_chunks::<12>()
                    .0
                    .iter()
                    .zip(view.row(y).as_chunks::<16>().0.iter())
                {
                    assert_eq!(before, &after[..12]);
                    assert_eq!(&after[12..], &1f32.to_ne_bytes());
                }
            }
            drop(converted);
            assert_eq!(
                (storage.reserved_bytes(), storage.reserved_objects()),
                (0, 0)
            );
        })
        .unwrap();
}
