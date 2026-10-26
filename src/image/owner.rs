//! Keep a caller-defined immutable packed owner through shared image publication.
use super::*;
use zenpixels::{ColorContext, PixelDescriptor};

/// Immutable CPU-mappable packed pixels and optional scalar auxiliary planes.
///
/// Implementors retain the backing allocations, color context and any admission
/// charges. Every mapping must refer to the same immutable samples and declared
/// interpretation for the lifetime of this owner. Borrowed views cannot outlive
/// it. An image's clones keep this owner alive until the last image drops.
///
/// This is an adapter ownership mechanism, not another policy callback. Mapping
/// may fail with an original backend cause through [`super::OutputError::Mapping`].
/// A backend requiring a temporary component-plane lock uses
/// [`crate::frame::Picture`] instead.
pub trait PackedImageOwner: Send + Sync {
    /// Borrow the packed color pixels and their actual color context.
    fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>>;

    /// Borrow the scalar values for the supplied auxiliary index. Indices start
    /// at zero and correspond to the roles passed to `from_packed_owner`.
    /// `None` for a declared index is a typed missing-plane failure.
    fn auxiliary(&self, _index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
        Ok(None)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ValueLayout {
    width: u32,
    height: u32,
    stride: usize,
    channel: ChannelType,
}
impl From<ValueSlice<'_>> for ValueLayout {
    fn from(value: ValueSlice<'_>) -> Self {
        Self {
            width: value.width,
            height: value.height,
            stride: value.stride,
            channel: value.channel,
        }
    }
}

struct Held<P> {
    owner: P,
    width: u32,
    height: u32,
    stride: usize,
    descriptor: PixelDescriptor,
    color: Option<Arc<ColorContext>>,
    extras: Vec<(Meaning, ValueLayout)>,
}

impl ImageOutput {
    /// Retain a native or admitted packed owner without copying its samples.
    ///
    /// Roles and interpretation are frozen here. Construction checks every
    /// declared plane; later mappings check that layout and color interpretation
    /// still agree. Auxiliary planes keep their own dimensions and sample type.
    /// The owner remains responsible for immutable sample contents.
    ///
    /// A storage budget is enforced by the supplied owner. This constructor does
    /// not retroactively admit existing allocations or bound native workspace.
    /// Small owner/metadata allocations remain outside a managed payload budget.
    pub fn from_packed_owner<P: PackedImageOwner + 'static>(
        owner: P,
        auxiliaries: Vec<Auxiliary>,
        interpretation: ImageInterpretation,
    ) -> Result<Self, At<OutputError>> {
        let g = interpretation.geometry;
        g.validate().map_err(|e| fail(OutputError::Region(e)))?;
        if let Some(placement) = interpretation.placement {
            placement
                .visible(g)
                .map_err(|e| fail(OutputError::Placement(e)))?;
        }
        if interpretation.orientation.is_none()
            || interpretation.signal.is_some()
            || interpretation.presentation.is_some()
        {
            return Err(fail(OutputError::Interpretation));
        }
        let pixels = owner.pixels()?;
        let (width, height) = (pixels.width(), pixels.rows());
        if (width, height) != (g.region.width, g.region.height) {
            return Err(fail(OutputError::Geometry));
        }
        let descriptor = pixels.descriptor();
        let stride = pixels.stride();
        let color = pixels.color_context().cloned();
        let mut extras = Vec::new();
        extras.try_reserve_exact(auxiliaries.len()).map_err(|e| {
            fail(OutputError::Mapping(crate::frame::MapError::Backend(
                Box::new(e),
            )))
        })?;
        for (index, role) in auxiliaries.into_iter().enumerate() {
            let values = owner
                .auxiliary(index)?
                .ok_or_else(|| fail(OutputError::MissingPlane))?;
            extras.push((Meaning::Auxiliary(role), ValueLayout::from(values)));
        }
        Ok(Self {
            storage: Arc::new(Held {
                owner,
                width,
                height,
                stride,
                descriptor,
                color,
                extras,
            }),
            interpretation: Arc::new(interpretation),
        })
    }
}

impl<P: PackedImageOwner> Held<P> {
    fn values(&self, index: usize) -> Result<ValueSlice<'_>, At<OutputError>> {
        let values = self
            .owner
            .auxiliary(index)?
            .ok_or_else(|| fail(OutputError::MissingPlane))?;
        if ValueLayout::from(values) != self.extras[index].1 {
            return Err(fail(OutputError::Geometry));
        }
        Ok(values)
    }
}

struct HeldMap<'a, P> {
    held: &'a Held<P>,
    pixels: PixelSlice<'a>,
}
impl<P: PackedImageOwner> ImageStorage for Held<P> {
    fn facts(&self) -> SampleFacts {
        SampleFacts {
            packed: Some(self.descriptor),
            planes: 1 + self.extras.len(),
            auxiliaries: self.extras.len(),
            applied_spot_colors: self
                .extras
                .iter()
                .filter(|(meaning, _)| {
                    matches!(
                        meaning,
                        Meaning::Auxiliary(Auxiliary {
                            kind: AuxiliaryKind::SpotColor {
                                contribution: ColorContribution::Applied,
                                ..
                            },
                            ..
                        })
                    )
                })
                .count(),
        }
    }
    fn map(&self) -> Result<Box<dyn MappedImage + '_>, Failure> {
        let pixels = self.owner.pixels()?;
        if (pixels.width(), pixels.rows(), pixels.stride())
            != (self.width, self.height, self.stride)
        {
            return Err(fail(OutputError::Geometry));
        }
        let same_color = match (pixels.color_context(), self.color.as_ref()) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
            _ => false,
        };
        if pixels.descriptor() != self.descriptor || !same_color {
            return Err(fail(OutputError::Interpretation));
        }
        for index in 0..self.extras.len() {
            self.values(index)?;
        }
        Ok(Box::new(HeldMap { held: self, pixels }))
    }
}

impl<P: PackedImageOwner> MappedImage for HeldMap<'_, P> {
    fn plane_count(&self) -> usize {
        1 + self.held.extras.len()
    }
    fn plane(&self, index: usize) -> Result<Option<Plane<'_>>, At<OutputError>> {
        if index == 0 {
            return Ok(Some(Plane {
                meaning: &Meaning::Color,
                samples: Samples::Pixels(self.pixels.clone()),
            }));
        }
        let Some((meaning, _)) = self.held.extras.get(index - 1) else {
            return Ok(None);
        };
        Ok(Some(Plane {
            meaning,
            samples: Samples::Values(self.held.values(index - 1)?),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use core::sync::atomic::{AtomicU8, Ordering};

    struct ChangingOwner {
        mode: Arc<AtomicU8>,
        pixels: Vec<u8>,
        values: Vec<u8>,
    }
    #[derive(Debug)]
    struct Backend(&'static str);
    impl fmt::Display for Backend {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }
    impl core::error::Error for Backend {}
    impl PackedImageOwner for ChangingOwner {
        fn pixels(&self) -> Result<PixelSlice<'_>, At<OutputError>> {
            let mode = self.mode.load(Ordering::SeqCst);
            if mode == 6 {
                return Err(fail(OutputError::Mapping(crate::frame::MapError::Backend(
                    Box::new(At::wrap(Backend("private backend payload"))),
                ))));
            }
            let descriptor = if mode == 2 {
                PixelDescriptor::RGB8_SRGB.with_transfer(zenpixels::TransferFunction::Linear)
            } else {
                PixelDescriptor::RGB8_SRGB
            };
            let mut pixels = PixelSlice::new(
                &self.pixels,
                if mode == 1 { 1 } else { 2 },
                2,
                if mode == 7 { 12 } else { 9 },
                descriptor,
            )
            .unwrap();
            if mode == 3 {
                pixels = pixels.with_color_context(Arc::new(ColorContext::from_icc(vec![1, 2, 3])));
            }
            Ok(pixels)
        }
        fn auxiliary(&self, index: usize) -> Result<Option<ValueSlice<'_>>, At<OutputError>> {
            if index > 0 || self.mode.load(Ordering::SeqCst) == 4 {
                return Ok(None);
            }
            ValueSlice::new(
                &self.values,
                if self.mode.load(Ordering::SeqCst) == 5 {
                    1
                } else {
                    2
                },
                2,
                12,
                ChannelType::F32,
            )
            .map(Some)
        }
    }
    fn roles() -> Vec<Auxiliary> {
        vec![Auxiliary {
            kind: AuxiliaryKind::Depth,
            name: String::new(),
            source_precision: SourcePrecision::Float {
                bits: 32,
                exponent_bits: 8,
            },
            source_dim_shift: 0,
        }]
    }
    fn interpretation() -> ImageInterpretation {
        ImageInterpretation::packed(
            ImageGeometry {
                canvas_width: 2,
                canvas_height: 2,
                region: Rect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                },
            },
            Orientation::from_exif(1).unwrap(),
        )
    }
    fn owner(mode: u8) -> (ChangingOwner, Arc<AtomicU8>) {
        let mode = Arc::new(AtomicU8::new(mode));
        (
            ChangingOwner {
                mode: mode.clone(),
                pixels: vec![7; 32],
                values: vec![0; 24],
            },
            mode,
        )
    }
    #[test]
    fn packed_owner_refuses_drift_and_keeps_backend_cause() {
        let (storage, mode) = owner(0);
        let image = ImageOutput::from_packed_owner(storage, roles(), interpretation()).unwrap();
        for changed in 1..=7 {
            mode.store(changed, Ordering::SeqCst);
            let error = image.map().err().expect("changed mapping was admitted");
            match changed {
                1 | 5 | 7 => assert!(matches!(error.error(), OutputError::Geometry)),
                2 | 3 => assert!(matches!(error.error(), OutputError::Interpretation)),
                4 => assert!(matches!(error.error(), OutputError::MissingPlane)),
                6 => {
                    assert!(crate::find_cause::<At<Backend>>(&error).is_some());
                    assert!(!alloc::format!("{error}").contains("private backend payload"));
                    assert!(!alloc::format!("{error:?}").contains("private backend payload"));
                }
                _ => unreachable!(),
            }
            mode.store(0, Ordering::SeqCst);
            let map = image.map().unwrap();
            let plane = map.plane(1).unwrap().unwrap();
            assert_eq!(plane.samples.dimensions(), (2, 2));
            assert_eq!(plane.samples.stride_bytes(), 12);
            assert!(map.plane(2).unwrap().is_none());
        }
    }
    #[test]
    fn packed_owner_checks_all_declared_planes_at_construction() {
        for mode in [1, 4, 6] {
            let (storage, _) = owner(mode);
            assert!(ImageOutput::from_packed_owner(storage, roles(), interpretation()).is_err());
        }
        for (width, height, stride, len) in [
            (0, 2, 12, 24),
            (2, 0, 12, 24),
            (2, 2, 7, 24),
            (2, 2, 12, 19),
        ] {
            assert!(
                ValueSlice::new(&[0; 24][..len], width, height, stride, ChannelType::F32).is_err()
            );
        }
        let values = ValueSlice::new(&[0; 20], 2, 2, 12, ChannelType::F32).unwrap();
        assert_eq!(values.as_strided_bytes().len(), 20);
    }
}
