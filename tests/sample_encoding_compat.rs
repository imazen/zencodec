//! Canonical sample encoding crosses core mapping and conversion boundaries directly.
use zencodec::frame::{PlaneSamples, PlaneView};
use zenpixels::{
    ChannelType,
    sample::{CodePlane, SampleEncoding, SampleEncodingError},
};

#[test]
fn all_integer_depth_and_shift_inputs_have_identical_contracts() {
    for (storage, width) in [(ChannelType::U8, 8u16), (ChannelType::U16, 16)] {
        for bits in 0..=u8::MAX {
            for shift in 0..=u8::MAX {
                let result = SampleEncoding::new(storage, bits, shift);
                if bits == 0 || u16::from(bits) > width {
                    assert_eq!(result, Err(SampleEncodingError::InvalidBitDepth));
                } else if u16::from(bits) + u16::from(shift) > width {
                    assert_eq!(result, Err(SampleEncodingError::InvalidBitShift));
                } else {
                    let encoding = result.unwrap();
                    assert_eq!(encoding.storage(), storage);
                    assert_eq!(encoding.code_bits(), bits);
                    assert_eq!(encoding.bit_shift(), shift);
                    assert_eq!(encoding.storage().byte_size(), usize::from(width / 8));
                    // Same checked value feeds native mapping and conversion without
                    // reconstructing a second crate's representation.
                    if storage == ChannelType::U16 {
                        let words = [0xaaaa, 0xffff, 0xdead, 0x5555, 0, 0xbeef];
                        let native =
                            PlaneView::new(PlaneSamples::U16(&words), 2, 2, 6, encoding).unwrap();
                        let convert =
                            CodePlane::from_u16(&words, 2, 2, 6, native.encoding()).unwrap();
                        for y in 0..2 {
                            for x in 0..2 {
                                let expected = ((u32::from(words[y * 3 + x]) >> shift)
                                    & ((1u32 << bits) - 1))
                                    as u16;
                                assert_eq!(native.code(x, y), Some(expected));
                                assert_eq!(convert.code(x, y), Some(expected));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn floating_storage_refuses_and_u8_convenience_is_equivalent() {
    for bits in 0..=u8::MAX {
        for shift in 0..=u8::MAX {
            assert_eq!(
                SampleEncoding::new(ChannelType::F32, bits, shift),
                Err(SampleEncodingError::UnsupportedStorage)
            );
        }
    }
    let encoding = SampleEncoding::new(ChannelType::U8, 8, 0).unwrap();
    let bytes = [0, 255, 17, 128];
    let native = PlaneView::new(PlaneSamples::U8(&bytes), 2, 2, 2, encoding).unwrap();
    let convert = CodePlane::from_u8(&bytes, 2, 2, 2, native.encoding()).unwrap();
    for y in 0..2 {
        for x in 0..2 {
            assert_eq!(native.code(x, y), Some(u16::from(bytes[y * 2 + x])));
            assert_eq!(native.code(x, y), convert.code(x, y));
        }
    }
}
