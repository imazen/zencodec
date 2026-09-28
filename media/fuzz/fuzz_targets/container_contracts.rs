#![no_main]
use libfuzzer_sys::fuzz_target;
use zencodec_media::{
    ivf::IvfReader,
    plane::{Plane, Samples},
    time::{Rounding, TimeBase, Timestamp},
};
use zenpixels::{ChannelType, sample::SampleEncoding};

fuzz_target!(|data: &[u8]| {
    if let Ok(mut reader) = IvfReader::new(data, 65536) {
        let mut previous_offset = 0;
        for _ in 0..64 {
            match reader.next_packet() {
                Ok(Some(packet)) => {
                    assert!(!packet.data.is_empty());
                    assert!(packet.data.len() <= 65536);
                    assert!(packet.byte_offset > previous_offset);
                    previous_offset = packet.byte_offset;
                }
                Ok(None) => {
                    assert!(reader.next_packet().unwrap().is_none());
                    break;
                }
                Err(_) => {
                    assert!(reader.next_packet().is_err());
                    break;
                }
            }
        }
    }
    if data.len() >= 24 {
        let ticks = i64::from_le_bytes(data[..8].try_into().unwrap());
        let word = |offset| u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        if let (Ok(a), Ok(b)) = (
            TimeBase::new(word(8), word(12)),
            TimeBase::new(word(16), word(20)),
        ) {
            let original = Timestamp::new(ticks, a);
            assert_eq!(original.rescale(a, Rounding::Nearest).unwrap(), original);
            if let Ok(floor) = original.rescale(b, Rounding::Floor) {
                assert!(!floor.compare(original).is_gt());
            }
            if let Ok(ceil) = original.rescale(b, Rounding::Ceil) {
                assert!(!ceil.compare(original).is_lt());
            }
        }
        if let Ok(encoding) = SampleEncoding::new(ChannelType::U8, data[20], data[21]) {
            if let Ok(plane) = Plane::new(
                Samples::U8(data),
                word(8) as usize,
                word(12) as usize,
                word(16) as usize,
                encoding,
            ) {
                assert!(plane.row(plane.height()).is_err());
                assert!(plane.code(plane.width(), 0).is_err());
                if plane.width() > 0 && plane.height() > 0 {
                    let maximum = (1_u16 << encoding.code_bits()) - 1;
                    assert!(plane.code(plane.width() - 1, plane.height() - 1).unwrap() <= maximum);
                }
            }
        }
    }
});
