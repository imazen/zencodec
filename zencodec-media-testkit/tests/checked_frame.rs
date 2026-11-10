//! Shared direct/image/lent mapping contract before codec-specific adoption.
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use zencodec::{
    CodecErrorExt, ErrorCategory,
    frame::{
        Frame, FrameGeometry, FrameInterpretation, FrameTiming, MapError, MappedPlanes, Picture,
        Presentation,
    },
    image::ImageOutput,
};
use zencodec_media::frame::{OwnedPlane, OwnedPlanes};
use zenpixels::hdr::DiffuseWhite;
use zenpixels::{
    AlphaMode, ChannelType, Cicp, ColorContext,
    plane::{
        AnyPlane, LayoutError, PlanarLayout, Plane, PlaneEncoding, PlaneRef, PlaneRole, PlaneSpec,
    },
    sample::{ChromaSiting, ChromaSubsampling, SampleEncoding},
};

fn encoding(bits: u8) -> PlaneEncoding {
    PlaneEncoding::Code(SampleEncoding::new(ChannelType::U16, bits, 0).unwrap())
}
fn interpretation() -> FrameInterpretation {
    FrameInterpretation::new(
        FrameGeometry::full(3, 1),
        PlanarLayout::new(
            vec![PlaneSpec::new(PlaneRole::Luma, encoding(10))],
            ChromaSubsampling::Mono,
            ChromaSiting::Unknown,
        )
        .unwrap(),
    )
}
fn frame(picture: Arc<dyn Picture>, interpretation: FrameInterpretation) -> Frame {
    Frame::new(
        picture,
        Arc::new(interpretation),
        FrameTiming::default(),
        Presentation::default(),
    )
}
fn owned() -> Arc<dyn Picture> {
    Arc::new(
        OwnedPlanes::new(
            vec![OwnedPlane::U16(
                Plane::new(vec![0, 512, 1023], 3, 1, 3).unwrap(),
            )],
            SampleEncoding::new(ChannelType::U16, 10, 0).unwrap(),
        )
        .unwrap(),
    )
}

#[test]
fn context_and_alpha_lending_preserve_odd_coded_geometry_strides_and_payloads() {
    let e = PlaneEncoding::Code(SampleEncoding::new(ChannelType::U16, 10, 6).unwrap());
    let a = PlaneEncoding::Code(SampleEncoding::new(ChannelType::U8, 8, 0).unwrap());
    let layout = PlanarLayout::new(
        vec![
            PlaneSpec::new(PlaneRole::Luma, e),
            PlaneSpec::new(PlaneRole::CrCb, e),
            PlaneSpec::new(PlaneRole::Alpha, a),
        ],
        ChromaSubsampling::Yuv420,
        ChromaSiting::Left,
    )
    .unwrap();
    let y = vec![0u16; 19];
    let y_pointer = y.as_ptr() as usize;
    let owner = Arc::new(
        OwnedPlanes::from_encoded_planes(vec![
            (OwnedPlane::U16(Plane::new(y, 5, 3, 7).unwrap()), e),
            (
                OwnedPlane::U16(Plane::new(vec![0u16; 14], 6, 2, 8).unwrap()),
                e,
            ),
            (
                OwnedPlane::U8(Plane::new(vec![255u8; 17], 5, 3, 6).unwrap()),
                a,
            ),
        ])
        .unwrap(),
    );
    let context = Arc::new(
        ColorContext::from_icc(&b"private profile payload"[..])
            .with_diffuse_white(DiffuseWhite::BT2408),
    );
    let mut geometry = FrameGeometry::full(5, 3);
    geometry.crop = zencodec::frame::Crop {
        x: 1,
        y: 1,
        width: 3,
        height: 2,
    };
    geometry.render = Some((9, 6));
    let signal = FrameInterpretation::new(geometry, layout)
        .with_color_context(context.clone())
        .with_alpha_mode(AlphaMode::Straight);
    assert!(!format!("{signal:?}").contains("private profile payload"));
    let f = frame(owner.clone(), signal);
    f.with_planar(None, |view| {
        assert_eq!((view.width(), view.height()), (5, 3));
        assert_eq!(view.alpha_mode(), Some(AlphaMode::Straight));
        assert_eq!(view.color_context(), context.as_ref());
        assert_eq!(view.layout().siting(), ChromaSiting::Left);
        assert_eq!(
            view.planes()[0].as_strided_bytes().as_ptr() as usize,
            y_pointer
        );
        assert_eq!(view.planes()[1].stride_bytes().unwrap(), 16);
        assert_eq!(view.planes()[1].width(), 6);
    })
    .unwrap();
    assert_eq!(f.interpretation().geometry, geometry);
    let image = ImageOutput::from_frame(f.clone());
    assert_eq!(image.map().unwrap().plane_count(), 3);
    let replay = f.represent(FrameTiming::default(), Presentation::SHOW_EXISTING);
    assert!(f.shares_picture(&replay));
    assert!(Arc::ptr_eq(f.interpretation(), replay.interpretation()));
}

#[test]
fn unknown_context_is_not_invented_from_source_claims() {
    let f = frame(
        owned(),
        interpretation().with_color(zencodec::frame::ColorClaim::new(
            Some(Cicp::SRGB),
            None,
            Some(Cicp::SRGB),
        )),
    );
    f.with_planar(None, |view| {
        assert_eq!(view.color_context(), &ColorContext::default())
    })
    .unwrap();
}

struct Native {
    encoding: Option<PlaneEncoding>,
    calls: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    fail: bool,
}
struct Loan<'a>(&'a Native);
impl MappedPlanes for Loan<'_> {
    fn plane_count(&self) -> usize {
        1
    }
    fn plane(&self, index: usize) -> Option<AnyPlane<'_>> {
        (index == 0).then(|| AnyPlane::U16(PlaneRef::new(&[0, 512, 1023][..], 3, 1, 3).unwrap()))
    }
    fn encoding(&self, index: usize) -> Option<PlaneEncoding> {
        if index == 0 { self.0.encoding } else { None }
    }
}
#[derive(Debug)]
struct NativeFailure;
impl std::fmt::Display for NativeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native payload")
    }
}
impl std::error::Error for NativeFailure {}
impl Picture for Native {
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.stopped.store(true, Ordering::SeqCst);
        if self.fail {
            Err(MapError::Backend(Box::new(zencodec::At::wrap(
                NativeFailure,
            ))))
        } else {
            Ok(Box::new(Loan(self)))
        }
    }
}

#[test]
fn missing_and_contradictory_encoding_refuse_before_any_consumer_callback() {
    for actual in [None, Some(encoding(12))] {
        let f = frame(
            Arc::new(Native {
                encoding: actual,
                calls: Arc::new(AtomicUsize::new(0)),
                stopped: Arc::new(AtomicBool::new(false)),
                fail: false,
            }),
            interpretation(),
        );
        assert!(matches!(f.with_planar(None,|_|panic!("must not lend")),
            Err(MapError::Component{index:0,actual_encoding,..}) if actual_encoding==actual));
        assert!(f.map().is_err());
        assert!(ImageOutput::from_frame(f).map().is_err());
    }
}

#[test]
fn invalid_alpha_and_geometry_refuse_before_mapping() {
    for mode in 0..3 {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut signal = interpretation();
        if mode == 0 {
            signal.alpha_mode = Some(AlphaMode::Undefined);
        }
        if mode == 1 {
            signal.alpha_mode = Some(AlphaMode::Straight);
        }
        if mode == 2 {
            signal.geometry.crop.x = u32::MAX;
        }
        let f = frame(
            Arc::new(Native {
                encoding: Some(encoding(10)),
                calls: calls.clone(),
                stopped: Arc::new(AtomicBool::new(false)),
                fail: false,
            }),
            signal,
        );
        assert!(matches!(
            f.map(),
            Err(MapError::Layout(LayoutError::AlphaInterpretation) | MapError::InvalidGeometry)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

struct Control(Arc<AtomicBool>);
impl enough::Stop for Control {
    fn check(&self) -> Result<(), enough::StopReason> {
        if self.0.load(Ordering::SeqCst) {
            Err(enough::StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}

#[test]
fn post_mapping_timeout_preserves_reason_and_native_failure_priority() {
    for fail in [false, true] {
        let stopped = Arc::new(AtomicBool::new(false));
        let control = zencodec::how_far::StopOnly::new(Control(stopped.clone()));
        let f = frame(
            Arc::new(Native {
                encoding: Some(encoding(10)),
                calls: Arc::new(AtomicUsize::new(0)),
                stopped,
                fail,
            }),
            interpretation(),
        );
        let error = f
            .with_planar(Some(&control), |_| panic!("stopped mapping must not lend"))
            .unwrap_err();
        if fail {
            assert!(zencodec::find_cause::<NativeFailure>(&error).is_some());
        } else {
            assert_eq!(
                error.error_category(),
                Some(ErrorCategory::Stopped(enough::StopReason::TimedOut))
            );
        }
        assert!(!format!("{error:?}").contains("native payload"));
    }
}
