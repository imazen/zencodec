//! Executable core-contract guide. Included by the media testkit.
#![allow(dead_code)]
use std::sync::Arc;
use zencodec::{
    At,
    frame::{Frame, FrameInterpretation, MapError},
    operation::{Decision, FrameConversionOffer, Question},
};

// EXAMPLE: frame_inspect

// ANCHOR: frame_inspect
pub fn inspect(frame: &Frame) -> Result<usize, MapError> {
    // Current context, native word layout and alpha association travel together.
    // This borrows sample storage; it does not crop, convert, resize or render.
    frame.with_planar(None, |view| view.planes().len())
}
// END: frame_inspect
// END EXAMPLE: frame_inspect

// EXAMPLE: frame_policy_target

// ANCHOR: frame_policy_target
#[derive(Debug)]
pub struct MissingCurrentColor;
impl std::fmt::Display for MissingCurrentColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this conversion requires known current color")
    }
}
impl std::error::Error for MissingCurrentColor {}

pub fn choose_current_target(question: Question<'_>) -> Result<Decision, At<MissingCurrentColor>> {
    let Question::FrameRoute { choices, .. } = question else {
        return Ok(Decision::UsePreset);
    };
    for choice in choices.iter() {
        let Some(conversion) = &choice.value().conversion else {
            continue;
        };
        if conversion.encoder_input.color_context.is_some() {
            return Ok(Decision::choose(choice));
        }
    }
    Err(At::wrap(MissingCurrentColor))
}
// END: frame_policy_target
// END EXAMPLE: frame_policy_target

// EXAMPLE: frame_conversion

// ANCHOR: frame_conversion
pub fn conversion(input: &Frame, target: Arc<FrameInterpretation>) -> FrameConversionOffer {
    // Bind this to the actual prepared converter, held through policy/execution.
    // Defaults make no sample/range-preservation or workspace guarantee.
    FrameConversionOffer::new(target, input.timing())
}
// END: frame_conversion
// END EXAMPLE: frame_conversion

mod implementor {
    // EXAMPLE: frame_native_loan
    // ANCHOR: frame_native_loan
    use zencodec::frame::MappedPlanes;
    use zenpixels::{
        ChannelType,
        plane::{AnyPlane, PlaneEncoding, PlaneRef},
        sample::SampleEncoding,
    };
    pub struct Gray10Loan<'a> {
        pub samples: &'a [u16],
        pub width: usize,
        pub height: usize,
        pub stride_words: usize,
    }
    impl MappedPlanes for Gray10Loan<'_> {
        fn plane_count(&self) -> usize {
            1
        }
        fn plane(&self, index: usize) -> Option<AnyPlane<'_>> {
            if index != 0 {
                return None;
            }
            PlaneRef::new(self.samples, self.width, self.height, self.stride_words)
                .ok()
                .map(AnyPlane::U16)
        }
        fn encoding(&self, index: usize) -> Option<PlaneEncoding> {
            (index == 0)
                .then(|| PlaneEncoding::Code(SampleEncoding::new(ChannelType::U16, 10, 0).unwrap()))
        }
    }
    // END: frame_native_loan
    // END EXAMPLE: frame_native_loan
}

#[test]
fn ordinary_use_and_prepared_conversion_share_the_exact_target() {
    use zencodec::frame::{FrameGeometry, FrameTiming, Presentation};
    use zencodec_media::frame::{OwnedPlane, OwnedPlanes};
    use zenpixels::{
        ChannelType, Cicp, ColorContext,
        plane::{PlanarLayout, Plane, PlaneEncoding, PlaneRole, PlaneSpec},
        sample::{ChromaSiting, ChromaSubsampling, SampleEncoding},
    };
    let encoding = SampleEncoding::new(ChannelType::U16, 10, 0).unwrap();
    let signal = Arc::new(
        FrameInterpretation::new(
            FrameGeometry::full(3, 1),
            PlanarLayout::new(
                vec![PlaneSpec::new(
                    PlaneRole::Luma,
                    PlaneEncoding::Code(encoding),
                )],
                ChromaSubsampling::Mono,
                ChromaSiting::Unknown,
            )
            .unwrap(),
        )
        .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::SRGB))),
    );
    let frame = Frame::new(
        Arc::new(
            OwnedPlanes::new(
                vec![OwnedPlane::U16(
                    Plane::new(vec![0, 577, 1023], 3, 1, 3).unwrap(),
                )],
                encoding,
            )
            .unwrap(),
        ),
        signal.clone(),
        FrameTiming::default(),
        Presentation::default(),
    );
    assert_eq!(inspect(&frame).unwrap(), 1);
    assert!(Arc::ptr_eq(
        &conversion(&frame, signal.clone()).encoder_input,
        &signal
    ));
    zencodec_media_testkit::check_frame_interchange(&frame).unwrap();
}
