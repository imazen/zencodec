//! Native reconstruction into admitted packed storage, then ordinary publication.
use super::{
    collection::Collection,
    image_items::ImageRequest,
    jxl_items::{Limits, complete, pixel_decoder},
    jxl_layers::JxlLayers,
    jxl_tests::fixture,
    output_tests::mapped_planes,
};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use zencodec::{
    At, SharedPulse, NoPulse,
    collection::Cause,
    enough::{Stop, StopReason},
    image::{
        Auxiliary, ImageInterpretation, ImageOutput, Meaning, OutputError, PackedImageOwner,
        ValueSlice,
    },
    operation::{Decision, Decline, PolicySet, Question, resolve_image},
    storage::{SharedStorage, StorageBudget, StorageError},
};
use zenjxl_decoder::api::{JxlColorType, JxlDecoderOptions, JxlOutputBuffer};
use zenpixels::{ChannelLayout, ChannelType, ColorContext, PixelDescriptor, PixelSlice};

#[derive(Clone, Copy)]
struct Layout {
    width: u32,
    height: u32,
    stride: usize,
    channels: usize,
}
struct Owner {
    planes: Vec<(SharedStorage<u8>, Layout)>,
    descriptor: PixelDescriptor,
    color: Option<Arc<ColorContext>>,
}
impl PackedImageOwner for Owner {
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
        let (storage, layout) = &self.planes[0];
        let mut pixels = PixelSlice::new(
            storage.as_slice(),
            layout.width,
            layout.height,
            layout.stride,
            self.descriptor,
        )
        .map_err(|e| {
            At::wrap(OutputError::Mapping(zencodec::frame::MapError::Backend(
                Box::new(e),
            )))
        })?;
        if let Some(color) = &self.color {
            pixels = pixels.with_color_context(color.clone());
        }
        Ok(pixels)
    }
    fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        self.planes
            .get(index + 1)
            .map(|(storage, layout)| {
                ValueSlice::new(
                    storage.as_slice(),
                    layout.width,
                    layout.height,
                    layout.stride,
                    ChannelType::F32,
                )
            })
            .transpose()
    }
}

// Interpretation comes from the already qualified native adapter, kept separate
// from this allocation experiment. No color or auxiliary meaning is invented.
struct Template {
    descriptor: PixelDescriptor,
    color: Option<Arc<ColorContext>>,
    auxiliaries: Vec<Auxiliary>,
    interpretation: ImageInterpretation,
}
fn reference(bytes: &[u8]) -> (ImageOutput, Template, usize) {
    let mut native = JxlLayers::open(
        bytes,
        Limits {
            images: 64,
            frame_headers: Some(64),
        },
        SharedPulse::default(),
    )
    .unwrap();
    let mut cursor = native.images().unwrap();
    let id = native.next(&mut cursor).unwrap().unwrap();
    let image = native.decode(&id, &ImageRequest::new()).unwrap();
    let (descriptor, color) = image
        .with_pixels(|p| (p.descriptor(), p.color_context().cloned()))
        .unwrap();
    let map = image.map().unwrap();
    let mut auxiliaries = Vec::new();
    for index in 1..map.plane_count() {
        let plane = map.plane(index).unwrap().unwrap();
        let Meaning::Auxiliary(role) = plane.meaning else {
            panic!("native auxiliary role missing")
        };
        auxiliaries.push(role.clone());
    }
    let size = mapped_planes(&image).iter().map(|p| p.bytes.len()).sum();
    let template = Template {
        descriptor,
        color,
        auxiliaries,
        interpretation: image.interpretation().clone(),
    };
    drop(map);
    (image, template, size)
}

fn reconstruct(
    bytes: &[u8],
    template: &Template,
    budget: &StorageBudget,
    stop: SharedPulse,
) -> Result<(ImageOutput, Vec<usize>), Cause> {
    let options = JxlDecoderOptions::default()
        .with_coalescing(false)
        .with_adjust_orientation(false)
        .with_render_spot_colors(false)
        .with_dither_u8(false)
        .with_stop(Arc::new(stop));
    let (mut decoder, mut input) = pixel_decoder(bytes, options)?;
    let mut format = decoder.current_pixel_format().clone();
    format.color_type = match template.descriptor.layout() {
        ChannelLayout::Gray => JxlColorType::Grayscale,
        ChannelLayout::GrayAlpha => JxlColorType::GrayscaleAlpha,
        ChannelLayout::Rgb => JxlColorType::Rgb,
        ChannelLayout::Rgba => JxlColorType::Rgba,
        ChannelLayout::Cmyk => JxlColorType::Cmyk,
        other => panic!("unqualified fixture layout: {other:?}"),
    };
    decoder.set_pixel_format(format);
    let format = decoder.current_pixel_format().clone();
    let frame = complete(decoder.process(&mut input)?)?;
    let header = frame.frame_header();
    let (width, height) = header.size;
    let padding = format.color_type.samples_per_pixel() * 4;
    let mut allocations = Vec::new();
    let mut layouts = Vec::new();
    for channels in std::iter::once(format.color_type.samples_per_pixel()).chain(
        format
            .extra_channel_format
            .iter()
            .filter(|f| f.is_some())
            .map(|_| 1),
    ) {
        let stride = width
            .checked_mul(channels)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(padding))
            .ok_or(StorageError::SizeOverflow)?;
        let size = stride
            .checked_mul(height)
            .ok_or(StorageError::SizeOverflow)?;
        allocations.push(budget.filled(size, 0xa5u8)?);
        layouts.push(Layout {
            width: u32::try_from(width)?,
            height: u32::try_from(height)?,
            stride,
            channels,
        });
    }
    let addresses = allocations
        .iter()
        .map(|p| p.as_slice().as_ptr() as usize)
        .collect();
    let mut output: Vec<_> = allocations
        .iter_mut()
        .zip(&layouts)
        .map(|(data, layout)| {
            JxlOutputBuffer::new_with_stride(
                data.as_mut_slice(),
                height,
                width * layout.channels * 4,
                layout.stride,
            )
        })
        .collect();
    let decoder = complete(frame.process(&mut input, &mut output)?)?;
    drop(output);
    drop(decoder);
    let owner = Owner {
        planes: allocations
            .into_iter()
            .zip(layouts)
            .map(|(p, l)| (p.freeze(), l))
            .collect(),
        descriptor: template.descriptor,
        color: template.color.clone(),
    };
    Ok((
        ImageOutput::from_packed_owner(
            owner,
            template.auxiliaries.clone(),
            template.interpretation.clone(),
        )?,
        addresses,
    ))
}

#[test]
fn owned_jxl_output_admission_survives_mapping_conversion_clones_and_reconstruction() {
    for name in [
        "conformance_test_images/cmyk_layers.jxl",
        "named_frame_test.jxl",
    ] {
        check_owner(name);
    }
}
fn check_owner(name: &str) {
    let bytes = fixture(name);
    let (expected, template, size) = reference(&bytes);
    let budget = StorageBudget::new(size, 8);
    let (image, addresses) =
        reconstruct(&bytes, &template, &budget, SharedPulse::default()).unwrap();
    assert_eq!(budget.reserved_bytes(), size);
    assert_eq!(mapped_planes(&image), mapped_planes(&expected));
    image
        .with_pixels(|pixels| {
            assert_eq!(pixels.as_strided_bytes().as_ptr() as usize, addresses[0]);
            let conversion =
                zenpixels_convert::RowConverter::new(pixels.descriptor(), pixels.descriptor());
            if pixels.descriptor().layout() == ChannelLayout::Cmyk {
                let error = conversion
                    .err()
                    .expect("CMYK path must retain its current typed CMS refusal");
                assert!(matches!(
                    error.error(),
                    zenpixels_convert::ConvertError::NeedsCms { .. }
                ));
            } else {
                let mut copy = conversion.unwrap();
                let active = pixels.width() as usize * pixels.descriptor().bytes_per_pixel();
                let mut row = vec![0xa5; active + 13];
                for y in 0..pixels.rows() {
                    copy.try_convert_row(pixels.row(y), &mut row, pixels.width())
                        .unwrap();
                    assert_eq!(&row[..active], pixels.row(y));
                    assert_eq!(&row[active..], &[0xa5; 13]);
                }
            }
        })
        .unwrap();
    let map = image.map().unwrap();
    for (index, address) in addresses.iter().copied().enumerate().skip(1) {
        let plane = map.plane(index).unwrap().unwrap();
        let zencodec::image::Samples::Values(values) = plane.samples else {
            panic!("expected scalar values")
        };
        assert_eq!(values.as_strided_bytes().as_ptr() as usize, address);
    }
    drop(map);
    let clone = image.clone();
    drop(image);
    let error = reconstruct(&bytes, &template, &budget, SharedPulse::default())
        .err()
        .unwrap();
    assert!(zencodec::find_cause::<StorageError>(error.as_ref()).is_some());
    assert_eq!(budget.reserved_bytes(), size);
    assert_eq!(mapped_planes(&clone), mapped_planes(&expected));
    drop(clone);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    let (next, _) = reconstruct(&bytes, &template, &budget, SharedPulse::default()).unwrap();
    assert_eq!(mapped_planes(&next), mapped_planes(&expected));
    drop(next);
    assert_eq!(budget.reserved_bytes(), 0);
}

#[derive(Clone)]
struct StopWithAllocation {
    budget: StorageBudget,
    observed: Arc<AtomicBool>,
}
impl Stop for StopWithAllocation {
    fn check(&self) -> Result<(), StopReason> {
        if self.budget.reserved_bytes() > 0 {
            self.observed.store(true, Ordering::SeqCst);
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[test]
fn owned_jxl_output_refusal_native_cancellation_and_policy_release_all_charges() {
    let bytes = fixture("conformance_test_images/cmyk_layers.jxl");
    let (_, template, size) = reference(&bytes);
    for budget in [StorageBudget::new(size - 1, 8), StorageBudget::new(size, 1)] {
        let error = reconstruct(&bytes, &template, &budget, SharedPulse::default())
            .err()
            .unwrap();
        assert!(zencodec::find_cause::<StorageError>(error.as_ref()).is_some());
        assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    }
    let budget = StorageBudget::new(size, 8);
    let stop = StopWithAllocation {
        budget: budget.clone(),
        observed: Arc::new(AtomicBool::new(false)),
    };
    let error = reconstruct(&bytes, &template, &budget, SharedPulse::new(zencodec::how_far::StopOnly::new(stop.clone())))
        .err()
        .unwrap();
    assert!(
        stop.observed.load(Ordering::SeqCst),
        "native control must observe admitted output"
    );
    assert_eq!(
        zencodec::find_cause::<zenjxl_decoder::api::Error>(error.as_ref())
            .unwrap()
            .stop_reason(),
        Some(StopReason::Cancelled)
    );
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));

    let (image, _) = reconstruct(&bytes, &template, &budget, SharedPulse::default()).unwrap();
    let mut resolver = |question: Question<'_>| -> Result<Decision, At<Infallible>> {
        assert!(matches!(question, Question::Image { .. }));
        assert_eq!(budget.reserved_bytes(), size);
        Ok(Decision::decline(Decline::ApplicationRule))
    };
    let error = resolve_image(image, &mut resolver, PolicySet::new(), &NoPulse).unwrap_err();
    assert!(zencodec::find_cause::<Decline>(&error).is_some());
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}
