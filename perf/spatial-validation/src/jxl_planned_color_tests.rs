//! A profile-aware color stage writes directly into admitted destination rows.
//! Auxiliary mapping/publication and true CMYK color arithmetic are separate.
use crate::{jxl_packaged_storage, jxl_tests::fixture};
use std::sync::Arc;
use zencodec::{
    collection::{Cause, Composition, ImageRequest},
    source::{ReadAtShim, SliceReadAt},
    storage::{SharedStorage, StorageBudget, StorageError},
};
use zenpixels::{ColorContext, ColorOrigin, PixelDescriptor, PixelSlice, PixelSliceMut};
use zenpixels_convert::{ConvertOptions, OutputPlan, OutputProfile};

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

fn identity(pixels: &PixelSlice<'_>) -> OutputPlan {
    let profile = pixels
        .color_context()
        .unwrap()
        .icc
        .as_ref()
        .unwrap()
        .clone();
    OutputPlan::new(
        pixels.descriptor(),
        pixels.color_context().cloned(),
        &ColorOrigin::from_icc(profile),
        OutputProfile::SameAsOrigin,
        pixels.descriptor().pixel_format(),
        &ConvertOptions::forbid_lossy(),
        None,
    )
    .unwrap()
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
            drop(source);
        }
        assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    }
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
