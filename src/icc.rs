//! Lightweight ICC profile inspection.
//!
//! Extracts specific tags from ICC profile bytes without a full parse.
//! No dependencies beyond `core` — suitable for `no_std` environments.

/// Extract CICP (Coding-Independent Code Points) from an ICC profile's tag table.
///
/// Scans the ICC tag table for a `cicp` tag (ICC v4.4+, 12 bytes) and returns
/// the four CICP fields if found. Returns `None` for ICC v2 profiles (which
/// never contain cicp tags), profiles without a cicp tag, or malformed input.
///
/// This is a lightweight operation (~100ns) that reads only the 128-byte header
/// and tag table entries — no full profile parse required.
///
/// # Returns
///
/// `Some((color_primaries, transfer_characteristics, matrix_coefficients, full_range))`
/// if a valid cicp tag is found, `None` otherwise.
pub fn icc_extract_cicp(data: &[u8]) -> Option<(u8, u8, u8, bool)> {
    // ICC profiles: 128-byte header, then tag count at offset 128.
    if data.len() < 132 {
        return None;
    }
    // Validate ICC signature at offset 36.
    if data[36..40] != *b"acsp" {
        return None;
    }

    let tag_count = u32::from_be_bytes(data[128..132].try_into().ok()?) as usize;
    // Cap to prevent DoS from malformed tag count.
    let tag_count = tag_count.min(200);

    // Tag table starts at offset 132, each entry is 12 bytes:
    //   [0..4]  signature
    //   [4..8]  data offset from profile start
    //   [8..12] data size
    for i in 0..tag_count {
        let entry_offset = 132 + i * 12;
        let entry = data.get(entry_offset..entry_offset + 12)?;

        if entry[..4] != *b"cicp" {
            continue;
        }

        let data_offset = u32::from_be_bytes(entry[4..8].try_into().ok()?) as usize;
        let data_size = u32::from_be_bytes(entry[8..12].try_into().ok()?) as usize;

        if data_size < 12 {
            return None;
        }

        let tag_data = data.get(data_offset..data_offset + 12)?;

        // Tag data starts with type signature (should also be "cicp").
        if tag_data[..4] != *b"cicp" {
            return None;
        }
        // Bytes 4..8 are reserved (should be zero).
        // Bytes 8..12 are the four CICP fields.
        return Some((tag_data[8], tag_data[9], tag_data[10], tag_data[11] != 0));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid ICC profile with a cicp tag for testing.
    fn build_icc_with_cicp(cp: u8, tc: u8, mc: u8, fr: bool) -> alloc::vec::Vec<u8> {
        let mut data = alloc::vec![0u8; 256];
        // Profile size at offset 0.
        let size = data.len() as u32;
        data[0..4].copy_from_slice(&size.to_be_bytes());
        // 'acsp' signature at offset 36.
        data[36..40].copy_from_slice(b"acsp");
        // Tag count = 1 at offset 128.
        data[128..132].copy_from_slice(&1u32.to_be_bytes());
        // Tag entry at offset 132: signature='cicp', offset=144, size=12.
        data[132..136].copy_from_slice(b"cicp");
        data[136..140].copy_from_slice(&144u32.to_be_bytes());
        data[140..144].copy_from_slice(&12u32.to_be_bytes());
        // Tag data at offset 144: type='cicp', reserved=0, then 4 CICP bytes.
        data[144..148].copy_from_slice(b"cicp");
        // reserved bytes 148..152 are already 0
        data[152] = cp;
        data[153] = tc;
        data[154] = mc;
        data[155] = if fr { 1 } else { 0 };
        data
    }

    #[test]
    fn extract_cicp_srgb() {
        let icc = build_icc_with_cicp(1, 13, 0, true);
        assert_eq!(icc_extract_cicp(&icc), Some((1, 13, 0, true)));
    }

    #[test]
    fn extract_cicp_pq() {
        let icc = build_icc_with_cicp(9, 16, 0, true);
        assert_eq!(icc_extract_cicp(&icc), Some((9, 16, 0, true)));
    }

    #[test]
    fn extract_cicp_hlg() {
        let icc = build_icc_with_cicp(9, 18, 0, false);
        assert_eq!(icc_extract_cicp(&icc), Some((9, 18, 0, false)));
    }

    #[test]
    fn no_cicp_in_empty_profile() {
        assert_eq!(icc_extract_cicp(&[]), None);
    }

    #[test]
    fn no_cicp_in_short_data() {
        assert_eq!(icc_extract_cicp(&[0; 100]), None);
    }

    #[test]
    fn no_cicp_without_acsp_signature() {
        let mut icc = build_icc_with_cicp(1, 13, 0, true);
        icc[36..40].copy_from_slice(b"xxxx");
        assert_eq!(icc_extract_cicp(&icc), None);
    }

    #[test]
    fn no_cicp_when_tag_missing() {
        let mut data = alloc::vec![0u8; 256];
        let size = data.len() as u32;
        data[0..4].copy_from_slice(&size.to_be_bytes());
        data[36..40].copy_from_slice(b"acsp");
        // Tag count = 1 but tag is 'desc' not 'cicp'
        data[128..132].copy_from_slice(&1u32.to_be_bytes());
        data[132..136].copy_from_slice(b"desc");
        data[136..140].copy_from_slice(&144u32.to_be_bytes());
        data[140..144].copy_from_slice(&12u32.to_be_bytes());
        assert_eq!(icc_extract_cicp(&data), None);
    }

    #[test]
    fn no_cicp_when_tag_data_too_small() {
        let mut icc = build_icc_with_cicp(1, 13, 0, true);
        // Set tag data size to 8 (too small, need 12)
        icc[140..144].copy_from_slice(&8u32.to_be_bytes());
        assert_eq!(icc_extract_cicp(&icc), None);
    }

    #[test]
    fn no_cicp_when_data_offset_out_of_bounds() {
        let mut icc = build_icc_with_cicp(1, 13, 0, true);
        // Set data offset beyond profile
        icc[136..140].copy_from_slice(&999u32.to_be_bytes());
        assert_eq!(icc_extract_cicp(&icc), None);
    }

    #[test]
    fn no_cicp_when_tag_type_mismatch() {
        let mut icc = build_icc_with_cicp(1, 13, 0, true);
        // Corrupt the type signature in tag data
        icc[144..148].copy_from_slice(b"xxxx");
        assert_eq!(icc_extract_cicp(&icc), None);
    }

    #[test]
    fn malicious_tag_count_capped() {
        // Build a profile where the cicp tag is at index 201 (past the cap of 200).
        // Tag entries start at 132; each is 12 bytes. Tag at index 201 → offset 132 + 201*12 = 2544.
        // We need a buffer large enough to hold the cicp tag data too.
        const CICP_IDX: usize = 201;
        const ENTRY_OFFSET: usize = 132 + CICP_IDX * 12;
        const DATA_OFFSET: usize = ENTRY_OFFSET + 12;
        let buf_len = DATA_OFFSET + 12;
        let mut data = alloc::vec![0u8; buf_len];
        let size = data.len() as u32;
        data[0..4].copy_from_slice(&size.to_be_bytes());
        data[36..40].copy_from_slice(b"acsp");
        // Claim there are 202 tags.
        data[128..132].copy_from_slice(&202u32.to_be_bytes());
        // Put cicp tag at index 201.
        data[ENTRY_OFFSET..ENTRY_OFFSET + 4].copy_from_slice(b"cicp");
        data[ENTRY_OFFSET + 4..ENTRY_OFFSET + 8]
            .copy_from_slice(&(DATA_OFFSET as u32).to_be_bytes());
        data[ENTRY_OFFSET + 8..ENTRY_OFFSET + 12].copy_from_slice(&12u32.to_be_bytes());
        data[DATA_OFFSET..DATA_OFFSET + 4].copy_from_slice(b"cicp");
        data[DATA_OFFSET + 8] = 1;
        data[DATA_OFFSET + 9] = 13;
        data[DATA_OFFSET + 10] = 0;
        data[DATA_OFFSET + 11] = 1;
        // With an absurd claimed tag count, the cap prevents reaching index 201.
        data[128..132].copy_from_slice(&u32::MAX.to_be_bytes());
        // Cap of 200 means index 201 is never reached → returns None.
        assert_eq!(icc_extract_cicp(&data), None);
    }
}
