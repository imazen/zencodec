//! Check the actual public output against mismatched native plane contracts.
use std::sync::Arc;
use zencodec::{
    At,
    frame::{
        ChromaSiting, Frame, FrameGeometry, FrameInterpretation, FrameTiming, MapError,
        MappedPlanes, Picture, PlaneRole, Presentation, SampleEncoding, Subsampling,
    },
    image::{ImageOutput, OutputError, Row},
};
use zencodec_media::frame::{OwnedPlane, OwnedPlanes, PlaneData};
use zenpixels::ChannelType;

fn encoding(bits: u8) -> SampleEncoding {
    SampleEncoding::new(ChannelType::U16, bits, 0).unwrap()
}
fn frame(picture: Arc<dyn Picture>, width: u32, bits: u8) -> ImageOutput {
    let interpretation = FrameInterpretation::new(
        FrameGeometry::full(width, 2),
        Subsampling::Mono,
        ChromaSiting::Unknown,
        encoding(bits),
        vec![PlaneRole::Luma],
    );
    ImageOutput::from_frame(Frame::new(
        picture,
        Arc::new(interpretation),
        FrameTiming::default(),
        Presentation::default(),
    ))
}
fn words() -> Arc<dyn Picture> {
    Arc::new(
        OwnedPlanes::new(
            vec![OwnedPlane {
                data: PlaneData::U16(vec![0, 512, 1023, 65535, 1, 511, 1022]),
                width: 3,
                height: 2,
                stride_bytes: 8,
            }],
            encoding(10),
        )
        .unwrap(),
    )
}

#[test]
fn native_codes_keep_active_rows_and_refuse_conflicting_claims() {
    let native = words();
    let correct = frame(native.clone(), 3, 10);
    let mapping = correct.map().unwrap();
    let plane = mapping.plane(0).unwrap().unwrap();
    assert_eq!(plane.samples.code_encoding(), Some(encoding(10)));
    let Some(Row::Words16(row)) = plane.samples.row(1) else {
        panic!("native words were reinterpreted");
    };
    assert_eq!(row, &[1, 511, 1022]);
    assert!(mapping.plane(1).unwrap().is_none());
    for (width, bits) in [(3, 12), (2, 10), (2, 12)] {
        let incorrect = frame(native.clone(), width, bits);
        let mapped = incorrect.map().unwrap();
        let error = match mapped.plane(0) {
            Err(error) => error,
            Ok(_) => panic!("conflicting native interpretation was accepted"),
        };
        let OutputError::Component {
            index,
            expected_dimensions,
            actual_dimensions,
            expected_encoding,
            actual_encoding,
        } = error.error()
        else {
            panic!("wrong refusal: {error:?}");
        };
        assert_eq!(*index, 0);
        assert_eq!(*expected_dimensions, (width as usize, 2));
        assert_eq!(*actual_dimensions, (3, 2));
        assert_eq!(*expected_encoding, encoding(bits));
        assert_eq!(*actual_encoding, encoding(10));
    }
}

#[derive(Debug)]
struct NativeError(u32);
impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native mapping {}", self.0)
    }
}
impl std::error::Error for NativeError {}
struct Refusing;
impl Picture for Refusing {
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
        Err(MapError::Backend(Box::new(At::wrap(NativeError(73)).at())))
    }
}

#[test]
fn mapping_error_retains_native_cause_through_shared_output() {
    let image = frame(Arc::new(Refusing), 3, 10);
    let error = match image.map() {
        Err(error) => error,
        Ok(_) => panic!("native mapping refusal was hidden"),
    };
    assert_eq!(zencodec::find_cause::<NativeError>(&error).unwrap().0, 73);
    assert!(zencodec::find_cause::<At<NativeError>>(&error).is_some());
}

#[test]
fn missing_planes_and_invalid_canvas_refuse_before_exposing_samples() {
    let missing = OwnedPlanes::new(vec![], encoding(10)).unwrap();
    let output = frame(Arc::new(missing), 3, 10);
    assert!(
        matches!(output.map(), Err(error) if matches!(error.error(), OutputError::MissingPlane))
    );
    let invalid = frame(Arc::new(Refusing), 0, 10);
    // Geometry is rejected before entering the deliberately refusing backend.
    assert!(matches!(invalid.map(), Err(error) if matches!(error.error(), OutputError::Region(_))));
}
