//! Public frame/image interchange must preserve per-plane interpretation and storage.
use std::sync::Arc;
use zencodec::{
    frame::{Frame, FrameGeometry, FrameInterpretation, FrameTiming, Presentation},
    image::{ImageOutput, Row, Samples},
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
    let has_alpha = layout.roles().any(|role| role == PlaneRole::Alpha);
    let mut interpretation = FrameInterpretation::new(FrameGeometry::full(3, 2), layout);
    if has_alpha {
        interpretation = interpretation.with_alpha_mode(zenpixels::AlphaMode::Straight);
    }
    ImageOutput::from_frame(Frame::new(
        owner,
        Arc::new(interpretation),
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
        let Samples::Codes(codes) = &chroma.samples else {
            panic!("integer chroma")
        };
        assert_eq!(codes.components(), 2);
        assert_eq!(codes.code(1, 0), None);
        assert_eq!(codes.component(0).unwrap().code(1, 0), Some(30));
        assert_eq!(codes.component(1).unwrap().code(1, 0), Some(40));
        assert!(codes.component(2).is_err());
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
    let error = match output.map() {
        Err(error) => error,
        Ok(_) => panic!("must refuse contradiction"),
    };
    let Some(zencodec::frame::MapError::Component {
        expected_encoding,
        actual_encoding,
        ..
    }) = zencodec::find_cause::<zencodec::frame::MapError>(&error)
    else {
        panic!("typed component cause")
    };
    assert_eq!(*expected_encoding, claimed);
    assert_eq!(*actual_encoding, Some(actual));
}

#[test]
fn policy_facts_distinguish_mixed_depth_and_plane_order() {
    let make = |bits, reversed| {
        let mut specs = vec![
            PlaneSpec::new(PlaneRole::Luma, code(ChannelType::U16, bits, 0)),
            PlaneSpec::new(PlaneRole::Alpha, code(ChannelType::U8, 8, 0)),
        ];
        if reversed {
            specs.reverse();
        }
        let interpretation = FrameInterpretation::new(
            FrameGeometry::full(3, 2),
            PlanarLayout::new(specs, ChromaSubsampling::Mono, ChromaSiting::Unknown).unwrap(),
        );
        zencodec::operation::FrameFacts::new(&interpretation, FrameTiming::default())
    };
    assert_eq!(make(10, false).encoding, None);
    assert_eq!(make(12, false).encoding, None);
    assert_ne!(make(10, false), make(12, false));
    assert_ne!(make(10, false), make(10, true));
}

#[test]
fn gray_renderer_refuses_alpha_only_before_writing() {
    use zencodec_media::adapt::{Gray8, RenderFrame};
    use zenpixels::{PixelDescriptor, PixelSliceMut};
    let encoding = code(ChannelType::U8, 8, 0);
    let frame = Frame::new(
        Arc::new(
            OwnedPlanes::from_encoded_planes(vec![(
                OwnedPlane::U8(Plane::new(vec![1, 2], 2, 1, 2).unwrap()),
                encoding,
            )])
            .unwrap(),
        ),
        Arc::new(FrameInterpretation::new(
            FrameGeometry::full(2, 1),
            PlanarLayout::new(
                vec![PlaneSpec::new(PlaneRole::Alpha, encoding)],
                ChromaSubsampling::Mono,
                ChromaSiting::Unknown,
            )
            .unwrap(),
        )),
        FrameTiming::default(),
        Presentation::default(),
    );
    let mut bytes = [99u8; 2];
    let mut canvas = PixelSliceMut::new(&mut bytes, 2, 1, 2, PixelDescriptor::GRAY8_SRGB).unwrap();
    assert!(Gray8::default().render(&frame, &mut canvas).is_err());
    assert_eq!(bytes, [99, 99]);
}
