use super::{
    component_grid::{Axis, Grid},
    component_owner::*,
    component_storage::{Order, Plane},
    fixtures::*,
};
use std::{
    error::Error as StdError,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zenavif::media::Av1VideoDecoder;
use zencodec::{
    frame::{FrameTiming, MappedPlanes, PlaneSamples, Presentation, Subsampling},
    image::Meaning,
};
use zenpixels::{PixelDescriptor, PixelSlice};

type Cause = Box<dyn StdError + Send + Sync>;
struct Av1(zencodec::frame::Frame);
struct Av1Map<'a> {
    native: Box<dyn MappedPlanes + 'a>,
    grids: Vec<Grid>,
}
impl Owner for Av1 {
    fn map(&self, _pulse: Option<&dyn zencodec::Pulse>) -> Result<Box<dyn Mapping + '_>, Cause> {
        let i = self.0.interpretation();
        let grids = i
            .roles
            .iter()
            .enumerate()
            .map(|(n, _)| grid(n, i.subsampling))
            .collect();
        Ok(Box::new(Av1Map {
            native: self.0.map()?,
            grids,
        }))
    }
}
fn grid(index: usize, sub: Subsampling) -> Grid {
    let axis = if index > 0 && sub == Subsampling::Yuv420 {
        Axis::new(1, 4, 2)
    } else {
        Axis::new(0, 1, 1)
    }
    .unwrap();
    Grid::Axes { x: axis, y: axis }
}
impl Mapping for Av1Map<'_> {
    fn plane_count(&self) -> usize {
        self.native.plane_count()
    }
    fn plane(
        &self,
        index: usize,
        _pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Option<Samples<'_>>, Cause> {
        let Some(p) = self.native.plane(index) else {
            return Ok(None);
        };
        let bytes = match p.as_strided_samples() {
            PlaneSamples::U8(s) => s,
            PlaneSamples::U16(s) => bytemuck::cast_slice(s),
        };
        let encoding = p.encoding();
        Ok(Some(Samples::Component {
            plane: Plane::new(
                bytes,
                encoding.storage(),
                Order::Native,
                p.width(),
                p.height(),
                p.stride_bytes(),
                encoding.storage().byte_size(),
            )
            .unwrap(),
            encoding: Some(encoding),
            grid: self.grids[index],
        }))
    }
}
#[test]
fn component_owner_retains_native_picture_across_presentations() {
    for bits in [8, 10, 12] {
        let s = settings(bits, Subsampling::Yuv420, false);
        let input = frame(&s, 0);
        let (config, packets) = encode(s, &[input]);
        let mut decoder = Av1VideoDecoder::new().unwrap();
        let source = decode(&mut decoder, &config, &packets, None)
            .unwrap()
            .remove(0);
        drop(decoder);
        let weak = Arc::downgrade(source.picture());
        let mapping = source.map().unwrap();
        let mut expected = Vec::new();
        let mut specs = Vec::new();
        for n in 0..mapping.plane_count() {
            let p = mapping.plane(n).unwrap();
            expected.push(
                (0..p.height())
                    .map(|y| {
                        (0..p.width())
                            .map(|x| p.code(x, y).unwrap())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
            );
            specs.push(Spec {
                meaning: Meaning::Component(source.interpretation().roles[n]),
                dimensions: (p.width(), p.height()),
                representation: Representation::Component {
                    channel: p.encoding().storage(),
                    encoding: Some(p.encoding()),
                    grid: grid(n, Subsampling::Yuv420),
                },
            });
        }
        drop(mapping);
        let presentation = Frame {
            image: Image::new(Box::new(Av1(source)), specs.into_boxed_slice()),
            timing: FrameTiming::default(),
            flags: Presentation::KEYFRAME,
        };
        let replay = presentation.represent(
            FrameTiming::default().with_duration_ticks(40),
            Presentation::SHOW_EXISTING,
        );
        assert!(replay.image.same_storage(&presentation.image));
        assert!(presentation.flags.keyframe);
        assert!(replay.flags.show_existing);
        assert_eq!(replay.timing.duration_ticks, Some(40));
        drop(presentation);
        assert!(weak.upgrade().is_some());
        assert_eq!(replay.image.planes().len(), 3);
        let map = replay.image.map(None).unwrap();
        assert_eq!(map.plane_count(), 3);
        for (n, rows) in expected.iter().enumerate() {
            let p = map.plane(n).unwrap().unwrap();
            assert!(matches!(p.meaning, Meaning::Component(_)));
            let Samples::Component {
                plane,
                encoding: Some(encoding),
                ..
            } = p.samples
            else {
                panic!()
            };
            let codes = plane.codes(encoding).unwrap();
            for (y, row) in rows.iter().enumerate() {
                for (x, value) in row.iter().enumerate() {
                    assert_eq!(codes.code(x, y).unwrap(), *value);
                }
            }
        }
        assert!(map.plane(expected.len()).unwrap().is_none());
        drop(map);
        drop(replay);
        assert!(weak.upgrade().is_none());
    }
}

#[derive(Clone, Copy)]
enum Fault {
    None,
    Missing,
    Changed,
    Backend,
    Panic,
    MapPanic,
}
struct Packed {
    bytes: [u8; 6],
    fault: Fault,
    calls: Arc<AtomicUsize>,
}
struct PackedMap<'a>(&'a Packed);
impl Owner for Packed {
    fn map(&self, _pulse: Option<&dyn zencodec::Pulse>) -> Result<Box<dyn Mapping + '_>, Cause> {
        assert!(
            !matches!(self.fault, Fault::MapPanic),
            "map must not be called"
        );
        Ok(Box::new(PackedMap(self)))
    }
}
impl Mapping for PackedMap<'_> {
    fn plane_count(&self) -> usize {
        1
    }
    fn plane(
        &self,
        _: usize,
        _pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<Option<Samples<'_>>, Cause> {
        self.0.calls.fetch_add(1, Ordering::Relaxed);
        match self.0.fault {
            Fault::Panic | Fault::MapPanic => panic!("mapping callback panic"),
            Fault::Missing => Ok(None),
            Fault::Backend => Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "private backend detail",
            ))),
            Fault::None | Fault::Changed => Ok(Some(Samples::Packed(
                PixelSlice::new(
                    &self.0.bytes,
                    if matches!(self.0.fault, Fault::Changed) {
                        1
                    } else {
                        2
                    },
                    1,
                    6,
                    PixelDescriptor::RGB8_SRGB,
                )
                .unwrap(),
            ))),
        }
    }
}
fn packed(fault: Fault, calls: Arc<AtomicUsize>) -> Image {
    Image::new(
        Box::new(Packed {
            bytes: [1, 2, 3, 4, 5, 6],
            fault,
            calls,
        }),
        vec![Spec {
            meaning: Meaning::Color,
            dimensions: (2, 1),
            representation: Representation::Packed {
                descriptor: PixelDescriptor::RGB8_SRGB,
                color: None,
            },
        }]
        .into_boxed_slice(),
    )
}
#[test]
fn component_owner_packed_and_native_use_same_mapping_interface() {
    let image = packed(Fault::None, Arc::default());
    let map = image.map(None).unwrap();
    let p = map.plane(0).unwrap().unwrap();
    assert!(matches!(p.meaning, Meaning::Color));
    let Samples::Packed(pixels) = p.samples else {
        panic!()
    };
    assert_eq!(pixels.row(0), [1, 2, 3, 4, 5, 6]);
}
#[test]
fn component_owner_mapping_failures_are_typed_and_terminal() {
    for fault in [Fault::Missing, Fault::Changed, Fault::Backend] {
        let calls = Arc::new(AtomicUsize::new(0));
        let image = packed(fault, calls.clone());
        let map = image.map(None).unwrap();
        let error = match map.plane(0) {
            Err(e) => e,
            Ok(_) => panic!("failure swallowed"),
        };
        match fault {
            Fault::Missing => assert!(matches!(error, Error::Missing(0))),
            Fault::Changed => assert!(matches!(error, Error::Changed(0))),
            Fault::Backend => {
                assert_eq!(
                    error
                        .source()
                        .unwrap()
                        .downcast_ref::<std::io::Error>()
                        .unwrap()
                        .kind(),
                    std::io::ErrorKind::TimedOut
                );
                assert!(!error.to_string().contains("private backend detail"));
                assert!(!format!("{error:?}").contains("private backend detail"));
            }
            Fault::None | Fault::Panic | Fault::MapPanic => unreachable!(),
        }
        assert!(matches!(map.plane(0), Err(Error::Terminal)));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn component_owner_mapping_unwind_remains_terminal() {
    let calls = Arc::new(AtomicUsize::new(0));
    let image = packed(Fault::Panic, calls.clone());
    let map = image.map(None).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = map.plane(0);
        }))
        .is_err()
    );
    assert!(matches!(map.plane(0), Err(Error::Terminal)));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

struct TimedOut;
impl enough::Stop for TimedOut {
    fn check(&self) -> Result<(), enough::StopReason> {
        Err(enough::StopReason::TimedOut)
    }
}
#[test]
fn component_owner_optional_pulse_refuses_before_mapping() {
    let calls = Arc::new(AtomicUsize::new(0));
    let image = packed(Fault::MapPanic, calls.clone());
    let pulse = zencodec::how_far::StopOnly::new(TimedOut);
    assert!(matches!(
        image.map(Some(&pulse)),
        Err(Error::Stopped(enough::StopReason::TimedOut))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
