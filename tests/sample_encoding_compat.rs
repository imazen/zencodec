//! Differential guard for the explicitly retained compatibility type.
use zencodec::frame::{SampleEncoding as Legacy, SampleEncodingError as LegacyError};
use zenpixels::{
    ChannelType,
    sample::{SampleEncoding as Canonical, SampleEncodingError},
};

fn legacy_error(error: LegacyError) -> u8 {
    match error {
        LegacyError::UnsupportedStorage => 0,
        LegacyError::InvalidBitDepth => 1,
        LegacyError::InvalidBitShift => 2,
        _ => panic!("new compatibility error requires migration review"),
    }
}
fn canonical_error(error: SampleEncodingError) -> u8 {
    match error {
        SampleEncodingError::UnsupportedStorage => 0,
        SampleEncodingError::InvalidBitDepth => 1,
        SampleEncodingError::InvalidBitShift => 2,
        _ => panic!("new canonical error requires migration review"),
    }
}

#[test]
fn all_integer_depth_and_shift_inputs_have_identical_contracts() {
    for storage in [ChannelType::U8, ChannelType::U16] {
        for bits in 0..=u8::MAX {
            for shift in 0..=u8::MAX {
                match (
                    Legacy::new(storage, bits, shift),
                    Canonical::new(storage, bits, shift),
                ) {
                    (Ok(old), Ok(new)) => {
                        assert_eq!(old.storage(), new.storage());
                        assert_eq!(old.code_bits(), new.code_bits());
                        assert_eq!(old.bit_shift(), new.bit_shift());
                        assert_eq!(
                            old.bytes_per_sample(),
                            if storage == ChannelType::U8 { 1 } else { 2 }
                        );
                    }
                    (Err(old), Err(new)) => assert_eq!(legacy_error(old), canonical_error(new)),
                    pair => panic!("contract mismatch for {storage:?}/{bits}/{shift}: {pair:?}"),
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
                legacy_error(Legacy::new(ChannelType::F32, bits, shift).unwrap_err()),
                0
            );
            assert_eq!(
                canonical_error(Canonical::new(ChannelType::F32, bits, shift).unwrap_err()),
                0
            );
        }
    }
    let canonical = Canonical::new(ChannelType::U8, 8, 0).unwrap();
    assert_eq!(Legacy::U8.storage(), canonical.storage());
    assert_eq!(Legacy::U8.code_bits(), canonical.code_bits());
    assert_eq!(Legacy::U8.bit_shift(), canonical.bit_shift());
}
