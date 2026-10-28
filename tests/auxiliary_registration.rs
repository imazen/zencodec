use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use zencodec::{
    At,
    image::{
        Auxiliary, AuxiliaryGrid, AuxiliaryImageOwner, AuxiliaryKind, ImageGeometry,
        ImageInterpretation, ImageOutput, OutputError, PackedImageOwner, Rect, Samples,
        SourcePrecision, ValueSlice, Values,
    },
};
use zenpixels::{ChannelType, Orientation, PixelBuffer, PixelDescriptor, PixelSlice};

struct Extra {
    bytes: [u8; 6],
    width: u32,
    aligned: Arc<AtomicBool>,
}
impl AuxiliaryImageOwner for Extra {
    fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        if index != 0 {
            return Ok(None);
        }
        ValueSlice::new(&self.bytes, self.width, 2, 3, ChannelType::U8).map(|v| {
            Some(v.with_grid(if self.aligned.load(Ordering::SeqCst) {
                AuxiliaryGrid::ColorSamples
            } else {
                AuxiliaryGrid::Unknown
            }))
        })
    }
}
struct Owner {
    extra: Arc<Extra>,
    separate: bool,
}
impl PackedImageOwner for Owner {
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
        Ok(PixelSlice::new(&[3; 18], 2, 2, 9, PixelDescriptor::RGB8_SRGB).unwrap())
    }
    fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        self.extra.auxiliary(index)
    }
    fn retain_auxiliaries(&self) -> Result<Option<Arc<dyn AuxiliaryImageOwner>>, At<OutputError>> {
        Ok(self
            .separate
            .then(|| self.extra.clone() as Arc<dyn AuxiliaryImageOwner>))
    }
}
fn role() -> Auxiliary {
    Auxiliary {
        kind: AuxiliaryKind::Depth,
        name: String::new(),
        source_precision: SourcePrecision::Integer {
            bits: 8,
            signed: false,
        },
        source_dim_shift: 0,
    }
}
fn interpretation() -> ImageInterpretation {
    ImageInterpretation::packed(
        ImageGeometry {
            canvas_width: 9,
            canvas_height: 12,
            region: Rect {
                x: 3,
                y: 7,
                width: 2,
                height: 2,
            },
        },
        Orientation::Identity,
    )
}
fn owner(width: u32, aligned: bool, separate: bool) -> (Owner, Arc<AtomicBool>) {
    let flag = Arc::new(AtomicBool::new(aligned));
    (
        Owner {
            extra: Arc::new(Extra {
                bytes: [17, 18, 99, 19, 20, 98],
                width,
                aligned: flag.clone(),
            }),
            separate,
        },
        flag,
    )
}
fn grid(image: &ImageOutput) -> AuxiliaryGrid {
    let map = image.map().unwrap();
    let plane = map.plane(1).unwrap().unwrap();
    let Samples::Values(values) = plane.samples else {
        panic!("auxiliary became color");
    };
    values.grid()
}

#[test]
fn matching_dimensions_and_source_reduction_never_infer_registration() {
    for width in [1, 2] {
        let (owner, _) = owner(width, false, false);
        let image = ImageOutput::from_packed_owner(owner, vec![role()], interpretation()).unwrap();
        assert_eq!(grid(&image), AuxiliaryGrid::Unknown);
        let values =
            Values::new(vec![17, 18, 99, 19, 20, 98], width, 2, 3, ChannelType::U8).unwrap();
        assert_eq!(values.grid(), AuxiliaryGrid::Unknown);
        assert_eq!(values.as_slice().grid(), AuxiliaryGrid::Unknown);
        let pixels = PixelBuffer::from_vec(vec![3; 12], 2, 2, PixelDescriptor::RGB8_SRGB).unwrap();
        let image = ImageOutput::packed(pixels, vec![(role(), values)], interpretation()).unwrap();
        assert_eq!(grid(&image), AuxiliaryGrid::Unknown);
    }
}

#[test]
fn known_registration_checks_shape_for_owned_and_native_planes_without_copying() {
    for width in [1, 2] {
        let (owner, _) = owner(width, true, false);
        let address = owner.extra.bytes.as_ptr();
        let native = ImageOutput::from_packed_owner(owner, vec![role()], interpretation());
        let values = Values::new(vec![17, 18, 99, 19, 20, 98], width, 2, 3, ChannelType::U8)
            .unwrap()
            .with_grid(AuxiliaryGrid::ColorSamples);
        let pixels = PixelBuffer::from_vec(vec![3; 12], 2, 2, PixelDescriptor::RGB8_SRGB).unwrap();
        let owned = ImageOutput::packed(pixels, vec![(role(), values)], interpretation());
        if width == 1 {
            for result in [native, owned] {
                assert!(matches!(
                    result.unwrap_err().error(),
                    OutputError::AuxiliaryGrid {
                        index: 0,
                        color_dimensions: (2, 2),
                        auxiliary_dimensions: (1, 2),
                        ..
                    }
                ));
            }
        } else {
            let native = native.unwrap();
            assert_eq!(grid(&native), AuxiliaryGrid::ColorSamples);
            assert_eq!(grid(&owned.unwrap()), AuxiliaryGrid::ColorSamples);
            let map = native.map().unwrap();
            let plane = map.plane(1).unwrap().unwrap();
            let Samples::Values(values) = plane.samples else {
                panic!();
            };
            assert_eq!(values.as_strided_bytes().as_ptr(), address);
            assert_eq!(values.as_strided_bytes(), &[17, 18, 99, 19, 20, 98]);
        }
    }
}

#[test]
fn registration_is_frozen_through_clones_and_both_color_replacement_owners() {
    for initially_aligned in [false, true] {
        for separate in [false, true] {
            let (source, flag) = owner(2, initially_aligned, separate);
            let original =
                ImageOutput::from_packed_owner(source, vec![role()], interpretation()).unwrap();
            let copy = original.clone();
            let (replacement, _) = owner(2, false, false);
            let replaced = original.replace_packed_color(replacement).unwrap();
            let expected = if initially_aligned {
                AuxiliaryGrid::ColorSamples
            } else {
                AuxiliaryGrid::Unknown
            };
            assert_eq!(grid(&copy), expected);
            assert_eq!(grid(&replaced), expected);
            flag.store(!initially_aligned, Ordering::SeqCst);
            for image in [&copy, &replaced] {
                let error = match image.map() {
                    Err(error) => error,
                    Ok(_) => panic!("changed auxiliary registration escaped"),
                };
                assert!(
                    matches!(error.error(), OutputError::AuxiliaryGrid { index: 0, expected: e, actual, .. } if *e == expected && *actual != expected)
                );
            }
        }
    }
}
