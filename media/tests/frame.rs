use zencodec_media::{
    color::{ChromaLocation, Subsampling, YuvView},
    display::{DisplayConversion, DisplayTransfer, OutOfRange},
    frame::{FrameError, RgbStorage, to_rgb},
    plane::{Plane, Samples},
};
use zenpixels::{ChannelType, Cicp, sample::SampleEncoding};

fn identity<'a>(codes: &'a [u16], bits: u8, color: Cicp) -> YuvView<'a> {
    let plane = Plane::new(
        Samples::U16(codes),
        codes.len(),
        1,
        codes.len() * 2,
        SampleEncoding::new(ChannelType::U16, bits, 0).unwrap(),
    )
    .unwrap();
    YuvView::new(
        plane,
        Some([plane, plane]),
        Subsampling::Yuv444,
        ChromaLocation::Unknown,
        color,
    )
    .unwrap()
}

#[test]
fn native_10_and_12_bit_rgb_expand_without_losing_the_low_codes() {
    for bits in [10, 12] {
        let max = (1u32 << bits) - 1;
        let codes: Vec<_> = (0..=max).map(|x| x as u16).collect();
        let color = Cicp::new(9, 16, 0, true);
        let view = identity(&codes, bits, color);
        let buffer = to_rgb(
            view,
            RgbStorage::U16,
            OutOfRange::Reject,
            None,
            1_000_000,
            None,
        )
        .unwrap();
        assert_eq!(buffer.as_slice().color_context().unwrap().cicp, Some(color));
        assert_eq!(buffer.descriptor().transfer().to_cicp(), Some(16));
        assert_eq!(buffer.descriptor().primaries.to_cicp(), Some(9));
        for (sample, rgb) in buffer
            .as_slice()
            .row(0)
            .as_chunks::<6>()
            .0
            .iter()
            .enumerate()
        {
            let expected = ((sample as u32 * 65535 + max / 2) / max) as u16;
            for pair in rgb.as_chunks::<2>().0 {
                assert_eq!(u16::from_ne_bytes(*pair), expected);
            }
        }
    }
}

#[test]
fn display_conversion_checks_source_color_and_records_resulting_color() {
    let codes = [0, 512, 1023];
    let linear = Cicp::new(1, 8, 0, true);
    let srgb = Cicp::new(1, 13, 0, true);
    let display = DisplayConversion::new(
        linear,
        DisplayTransfer::linear(100.0).unwrap(),
        srgb,
        DisplayTransfer::srgb(100.0).unwrap(),
        OutOfRange::Reject,
    )
    .unwrap();
    let packed = to_rgb(
        identity(&codes, 10, linear),
        RgbStorage::U8,
        OutOfRange::Reject,
        Some(&display),
        1000,
        None,
    )
    .unwrap();
    assert_eq!(
        packed.as_slice().row(0),
        &[0, 0, 0, 188, 188, 188, 255, 255, 255]
    );
    assert_eq!(packed.as_slice().color_context().unwrap().cicp, Some(srgb));
    assert!(matches!(
        to_rgb(
            identity(&codes, 10, srgb),
            RgbStorage::U8,
            OutOfRange::Reject,
            Some(&display),
            1000,
            None
        ),
        Err(FrameError::ColorMismatch)
    ));
}

#[test]
fn unknown_color_stays_unknown_and_memory_bound_includes_row_scratch() {
    let codes = [0, 1, 1023];
    let color = Cicp::new(222, 223, 0, true);
    let view = identity(&codes, 10, color);
    assert!(matches!(
        to_rgb(view, RgbStorage::U8, OutOfRange::Reject, None, 80, None),
        Err(FrameError::Limit)
    ));
    let packed = to_rgb(view, RgbStorage::U8, OutOfRange::Reject, None, 81, None).unwrap();
    assert_eq!(packed.as_slice().color_context().unwrap().cicp, Some(color));
    assert_eq!(
        packed.descriptor().transfer(),
        zenpixels::TransferFunction::Unknown
    );
    let floats = to_rgb(view, RgbStorage::F32, OutOfRange::Reject, None, 108, None).unwrap();
    let middle = f32::from_ne_bytes(floats.as_slice().row(0)[12..16].try_into().unwrap());
    assert!((middle - 1.0 / 1023.0).abs() < 1e-9);
}
