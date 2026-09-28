//! EBML primitive boundary tests: VINT size fields, element IDs, signed ints.

use std::io::Cursor;
use zencodec_media::ebml as e;

fn encode_size(v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    e::write_size(&mut out, Some(v)).unwrap();
    out
}

#[test]
fn vint_size_boundaries() {
    // Legal widths: 1 byte holds 0..=126 (127 is all-ones reserved).
    assert_eq!(encode_size(0), [0x80]);
    assert_eq!(encode_size(126), [0xFE]);
    // 127 collides with the len-1 sentinel → 2 bytes.
    assert_eq!(encode_size(127), [0x40, 0x7F]);
    assert_eq!(encode_size(16_382), [0x7F, 0xFE]);
    // 16383 collides with the len-2 sentinel → 3 bytes.
    assert_eq!(encode_size(16_383), [0x20, 0x3F, 0xFF]);
    assert_eq!(encode_size(2_097_150), [0x3F, 0xFF, 0xFE]);
    assert_eq!(encode_size(2_097_151), [0x10, 0x1F, 0xFF, 0xFF]);
}

#[test]
fn vint_size_max_and_overflow() {
    // Largest legal: 2^56-2 at 8 bytes.
    let max = encode_size((1u64 << 56) - 2);
    assert_eq!(max.len(), 8);
    assert_eq!(max[0], 0x01);
    assert_eq!(max[7], 0xFE);
    // 2^56-1 is the unknown-size sentinel — must refuse, not silently write it.
    let mut out = Vec::new();
    assert!(e::write_size(&mut out, Some((1u64 << 56) - 1)).is_err());
    assert!(e::write_size(&mut out, Some(u64::MAX)).is_err());
}

#[test]
fn vint_unknown_size_sentinel() {
    let mut out = Vec::new();
    e::write_size(&mut out, None).unwrap();
    assert_eq!(out, [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn element_id_widths() {
    // Width is set by the marker-bit position, not the value's magnitude.
    let cases: &[(u64, &[u8])] = &[
        (0xA3, &[0xA3]),                         // SimpleBlock: 1 byte
        (0x4286, &[0x42, 0x86]),                 // EBMLVersion: 2 bytes
        (0x23E383, &[0x23, 0xE3, 0x83]),         // DefaultDuration: 3 bytes
        (0x1A45DFA3, &[0x1A, 0x45, 0xDF, 0xA3]), // EBML: 4 bytes
    ];
    for (id, want) in cases {
        let mut out = Vec::new();
        e::write_id(&mut out, *id).unwrap();
        assert_eq!(&out[..], *want, "id {id:#x}");
    }
}

#[test]
fn element_header_roundtrip() {
    let mut out = Vec::new();
    e::write_element(&mut out, e::id::TIMESTAMP_SCALE, &[0x0F, 0x42, 0x40]).unwrap();
    let h = e::read_element_header(&mut Cursor::new(&out)).unwrap();
    assert_eq!(h.id, e::id::TIMESTAMP_SCALE);
    assert_eq!(h.size, Some(3));
    assert_eq!(h.header_len as usize, out.len() - 3);
}

#[test]
fn element_header_malformed() {
    // Zero first byte: no marker bit in 8 bytes.
    assert!(e::read_element_header(&mut Cursor::new(&[0x00, 0x40])).is_err());
    // Truncated mid-field.
    assert!(e::read_element_header(&mut Cursor::new(&[0x1A, 0x45])).is_err());
    // Unknown size decodes as None (caller decides legality).
    let h = e::read_element_header(&mut Cursor::new(&[
        0x1F, 0x43, 0xB6, 0x75, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ]))
    .unwrap();
    assert_eq!(h.id, e::id::CLUSTER);
    assert_eq!(h.size, None);
}

fn encode_int(v: i64) -> Vec<u8> {
    let mut out = Vec::new();
    e::write_int(&mut out, e::id::REFERENCE_BLOCK, v).unwrap();
    // Strip 1-byte id + size; return payload.
    let h = e::read_element_header(&mut Cursor::new(&out)).unwrap();
    let start = h.header_len as usize;
    out[start..start + h.size.unwrap() as usize].to_vec()
}

#[test]
fn signed_int_minimal_width() {
    assert_eq!(encode_int(0), [0x00]);
    assert_eq!(encode_int(127), [0x7F]);
    assert_eq!(encode_int(128), [0x00, 0x80]);
    assert_eq!(encode_int(-1), [0xFF]);
    assert_eq!(encode_int(-128), [0x80]);
    assert_eq!(encode_int(-129), [0xFF, 0x7F]);
    assert_eq!(encode_int(32_767), [0x7F, 0xFF]);
    assert_eq!(encode_int(32_768), [0x00, 0x80, 0x00]);
    assert_eq!(encode_int(-32_768), [0x80, 0x00]);
    assert_eq!(encode_int(-32_769), [0xFF, 0x7F, 0xFF]);
    // i64 extremes fill all 8 bytes.
    assert_eq!(encode_int(i64::MIN).len(), 8);
    assert_eq!(encode_int(i64::MAX).len(), 8);
}
