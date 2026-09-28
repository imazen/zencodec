use zencodec_media::plane::{Plane, PlaneError, Samples};
use zenpixels::ChannelType;
use zenpixels::sample::SampleEncoding;

#[test]
fn native_and_shifted_codes_keep_their_interpretation_without_copying() {
    for bits in [8, 10, 12, 16] {
        for shift in [0, 16 - bits] {
            let max = ((1_u32 << bits) - 1) as u16;
            let words = [0, max << shift, 0xabcd, (max / 2) << shift, 0];
            // Two rows of two samples, one padding word only after row zero.
            let plane = Plane::new(
                Samples::U16(&words),
                2,
                2,
                6,
                SampleEncoding::new(ChannelType::U16, bits, shift).unwrap(),
            )
            .unwrap();
            assert_eq!(plane.code(1, 0).unwrap(), max);
            assert_eq!(plane.code(0, 1).unwrap(), max / 2);
            assert_eq!(plane.stride_bytes(), 6);
            let Samples::U16(row) = plane.row(1).unwrap() else {
                panic!("storage must remain U16")
            };
            assert_eq!(row.as_ptr(), words[3..].as_ptr());
            assert_eq!(row.len(), 2);
            assert_eq!(plane.encoding().code_bits(), bits);
        }
    }
}

#[test]
fn invalid_geometry_is_rejected_without_reading_pixels() {
    let words = [0_u16; 4];
    let make = |w, h, stride| {
        Plane::new(
            Samples::U16(&words),
            w,
            h,
            stride,
            SampleEncoding::new(ChannelType::U16, 16, 0).unwrap(),
        )
    };
    assert_eq!(make(2, 2, 3).unwrap_err(), PlaneError::MisalignedStride);
    assert_eq!(make(3, 1, 4).unwrap_err(), PlaneError::ShortStride);
    assert_eq!(make(2, 3, 4).unwrap_err(), PlaneError::ShortBuffer);
    assert_eq!(
        make(1, usize::MAX, 4).unwrap_err(),
        PlaneError::GeometryOverflow
    );
    assert_eq!(
        Plane::new(
            Samples::U16(&words),
            1,
            1,
            2,
            SampleEncoding::new(ChannelType::U8, 8, 0).unwrap()
        )
        .unwrap_err(),
        PlaneError::StorageMismatch
    );
}

#[test]
fn zero_area_never_computes_a_phantom_row_offset() {
    let empty = Plane::new(
        Samples::U16(&[]),
        0,
        usize::MAX,
        usize::MAX - 1,
        SampleEncoding::new(ChannelType::U16, 16, 0).unwrap(),
    )
    .unwrap();
    assert!(empty.row(usize::MAX - 1).unwrap().is_empty());
    assert_eq!(empty.code(0, 0), Err(PlaneError::OutsidePlane));
    let no_rows = Plane::new(
        Samples::U8(&[]),
        10,
        0,
        0,
        SampleEncoding::new(ChannelType::U8, 8, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(no_rows.row(0).unwrap_err(), PlaneError::OutsidePlane);
}
