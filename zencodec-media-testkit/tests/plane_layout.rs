//! Public frame/image interchange must preserve per-plane interpretation and storage.
use std::sync::Arc;
use zencodec::{
    frame::{Frame, FrameGeometry, FrameInterpretation, FrameTiming, Presentation},
    image::{ImageOutput, OutputError, Row, Samples},
    operation::ImageFacts,
};
use zencodec_media::frame::{OwnedPlane, OwnedPlanes};
use zenpixels::{
    ChannelType,
    plane::{FloatEncoding, PlanarLayout, Plane, PlaneEncoding, PlaneRole, PlaneSpec},
    sample::{ChromaSiting, ChromaSubsampling, SampleEncoding},
};

fn code(storage: ChannelType, bits: u8, shift: u8) -> PlaneEncoding {
    PlaneEncoding::Code(SampleEncoding::new(storage, bits, shift).unwrap())
}
fn image(
    planes: Vec<(OwnedPlane, PlaneEncoding)>,
    specs: Vec<PlaneSpec>,
    sub: ChromaSubsampling,
) -> ImageOutput {
    let layout = PlanarLayout::new(specs, sub, ChromaSiting::Unknown).unwrap();
    let owner = Arc::new(OwnedPlanes::from_encoded_planes(planes).unwrap());
    ImageOutput::from_frame(Frame::new(
        owner,
        Arc::new(FrameInterpretation::new(FrameGeometry::full(3, 2), layout)),
        FrameTiming::default(),
        Presentation::default(),
    ))
}

#[test]
fn mixed_depth_alpha_keeps_native_words_and_no_false_uniform_depth() {
    let y = code(ChannelType::U16, 10, 6);
    let a = code(ChannelType::U8, 8, 0);
    let data = vec![
        0u16,
        512 << 6,
        1023 << 6,
        0xbeef,
        1 << 6,
        511 << 6,
        1022 << 6,
    ];
    let pointer = data.as_ptr();
    let output = image(
        vec![
            (OwnedPlane::U16(Plane::new(data, 3, 2, 4).unwrap()), y),
            (
                OwnedPlane::U8(Plane::new(vec![0, 128, 255, 99, 1, 127, 254], 3, 2, 4).unwrap()),
                a,
            ),
        ],
        vec![
            PlaneSpec::new(PlaneRole::Luma, y),
            PlaneSpec::new(PlaneRole::Alpha, a),
        ],
        ChromaSubsampling::Mono,
    );
    assert_eq!(ImageFacts::new(&output).frame.unwrap().encoding, None);
    let mapped = output.map().unwrap();
    let luma = mapped.plane(0).unwrap().unwrap();
    let Samples::Codes(luma) = luma.samples else {
        panic!("integer luma")
    };
    let Some(Row::Words16(first)) = luma.row(0) else {
        panic!("u16 luma")
    };
    assert_eq!(first.as_ptr(), pointer);
    assert_eq!(luma.code(1, 0), Some(512));
    assert_eq!(luma.code(2, 1), Some(1022));
    let alpha = mapped.plane(1).unwrap().unwrap();
    let Some(Row::Bytes(row)) = alpha.samples.row(1) else {
        panic!("u8 alpha")
    };
    assert_eq!(row, &[1, 127, 254]);
    assert_eq!(
        alpha.samples.code_encoding(),
        Some(SampleEncoding::new(ChannelType::U8, 8, 0).unwrap())
    );
}

#[test]
fn float_mapping_keeps_scale_offset_excursions_and_nan_bits_without_conversion() {
    let encoding = FloatEncoding::new(255.0, -128.0).unwrap();
    let e = PlaneEncoding::Float(encoding);
    let payload_nan = f32::from_bits(0x7fc01234);
    let data = vec![-128.0, 0.0, 127.0, 777.0, -129.0, 128.0, payload_nan];
    let pointer = data.as_ptr();
    let output = image(
        vec![(OwnedPlane::F32(Plane::new(data, 3, 2, 4).unwrap()), e)],
        vec![PlaneSpec::new(PlaneRole::Luma, e)],
        ChromaSubsampling::Mono,
    );
    assert!(ImageFacts::new(&output).frame.unwrap().encoding.is_none());
    let mapped = output.map().unwrap();
    let sample = mapped.plane(0).unwrap().unwrap();
    assert_eq!(sample.samples.stride_bytes(), 16);
    assert_eq!(sample.samples.code_encoding(), None);
    let Samples::Float {
        plane,
        encoding: actual,
    } = sample.samples
    else {
        panic!("float plane")
    };
    assert_eq!(actual, encoding);
    assert_eq!(plane.row(0).unwrap().as_ptr(), pointer);
    assert_eq!(
        plane
            .row(1)
            .unwrap()
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>(),
        [
            (-129.0f32).to_bits(),
            128.0f32.to_bits(),
            payload_nan.to_bits()
        ]
    );
}

#[test]
fn odd_width_semiplanar_chroma_counts_components_and_retains_order() {
    for role in [PlaneRole::CbCr, PlaneRole::CrCb] {
        let e = code(ChannelType::U8, 8, 0);
        let output = image(
            vec![
                (OwnedPlane::U8(Plane::new(vec![0; 6], 3, 2, 3).unwrap()), e),
                (
                    OwnedPlane::U8(Plane::new(vec![10, 20, 30, 40], 4, 1, 4).unwrap()),
                    e,
                ),
            ],
            vec![PlaneSpec::new(PlaneRole::Luma, e), PlaneSpec::new(role, e)],
            ChromaSubsampling::Yuv420,
        );
        let mapped = output.map().unwrap();
        let chroma = mapped.plane(1).unwrap().unwrap();
        assert_eq!(chroma.meaning, &zencodec::image::Meaning::Component(role));
        assert_eq!(chroma.samples.dimensions(), (4, 1));
        let Some(Row::Bytes(row)) = chroma.samples.row(0) else {
            panic!("chroma bytes")
        };
        assert_eq!(row, &[10, 20, 30, 40]);
    }
}

#[test]
fn per_plane_owner_encoding_contradiction_is_a_typed_refusal() {
    let actual = code(ChannelType::U16, 10, 0);
    let claimed = code(ChannelType::U16, 12, 0);
    let output = image(
        vec![(
            OwnedPlane::U16(Plane::new(vec![0; 6], 3, 2, 3).unwrap()),
            actual,
        )],
        vec![PlaneSpec::new(PlaneRole::Luma, claimed)],
        ChromaSubsampling::Mono,
    );
    let mapped = output.map().unwrap();
    let error = match mapped.plane(0) {
        Err(error) => error,
        Ok(_) => panic!("must refuse contradiction"),
    };
    let Some(OutputError::Component {
        expected_encoding,
        actual_encoding,
        ..
    }) = zencodec::find_cause::<OutputError>(&error)
    else {
        panic!("typed component cause")
    };
    assert_eq!(*expected_encoding, claimed);
    assert_eq!(*actual_encoding, actual);
}
