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
pub(crate) mod tests {
    use super::*;
    use moxcms::{CicpColorPrimaries, CicpProfile, ColorProfile, MatrixCoefficients, TransferCharacteristics};

    /// Build a minimal valid ICC profile with a cicp tag for testing.
    pub(crate) fn build_icc_with_cicp(cp: u8, tc: u8, mc: u8, fr: bool) -> alloc::vec::Vec<u8> {
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

    #[test]
    fn cicp_not_first_tag() {
        // cicp tag at index 2 (after two dummy tags).
        let mut data = alloc::vec![0u8; 300];
        data[0..4].copy_from_slice(&300u32.to_be_bytes());
        data[36..40].copy_from_slice(b"acsp");
        data[128..132].copy_from_slice(&3u32.to_be_bytes());
        // Tag 0: 'desc' at offset 168, size 12
        data[132..136].copy_from_slice(b"desc");
        data[136..140].copy_from_slice(&168u32.to_be_bytes());
        data[140..144].copy_from_slice(&12u32.to_be_bytes());
        // Tag 1: 'cprt' at offset 180, size 12
        data[144..148].copy_from_slice(b"cprt");
        data[148..152].copy_from_slice(&180u32.to_be_bytes());
        data[152..156].copy_from_slice(&12u32.to_be_bytes());
        // Tag 2: 'cicp' at offset 192, size 12
        data[156..160].copy_from_slice(b"cicp");
        data[160..164].copy_from_slice(&192u32.to_be_bytes());
        data[164..168].copy_from_slice(&12u32.to_be_bytes());
        // cicp tag data at offset 192
        data[192..196].copy_from_slice(b"cicp");
        data[200] = 9;  // BT.2020 primaries
        data[201] = 16; // PQ transfer
        data[202] = 0;  // Identity matrix
        data[203] = 1;  // full range
        assert_eq!(icc_extract_cicp(&data), Some((9, 16, 0, true)));
    }

    #[test]
    fn tag_count_zero() {
        let mut data = alloc::vec![0u8; 256];
        data[0..4].copy_from_slice(&256u32.to_be_bytes());
        data[36..40].copy_from_slice(b"acsp");
        data[128..132].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(icc_extract_cicp(&data), None);
    }

    // -----------------------------------------------------------------------
    // Cross-validation with moxcms: generate ICC profiles with moxcms,
    // serialize to bytes, verify our extractor agrees.
    // -----------------------------------------------------------------------

    /// Helper: generate ICC bytes from a moxcms ColorProfile and cross-validate.
    /// Reads expected values from the profile struct itself — no hardcoded assumptions.
    fn cross_validate_moxcms(profile: &ColorProfile) {
        let icc_bytes = profile.encode().expect("moxcms encode failed");

        // Derive expected from the profile's cicp field.
        let expected = profile.cicp.map(|c| {
            (c.color_primaries as u8, c.transfer_characteristics as u8,
             c.matrix_coefficients as u8, c.full_range)
        });

        // Our extractor must agree with what moxcms wrote.
        let our_result = icc_extract_cicp(&icc_bytes);

        // moxcms only writes cicp for Input/Display class with RGB/YCbCr/XYZ color space.
        // If the profile has cicp but the class/space combination prevents writing, we
        // expect None from our extractor even though the struct has Some.
        // For standard profiles (RGB Display), both should agree.
        if our_result != expected {
            // Check if moxcms intentionally skipped writing the cicp tag.
            // Re-parse and check if moxcms itself sees cicp.
            let parsed = ColorProfile::new_from_slice(&icc_bytes).expect("moxcms parse failed");
            let parsed_cicp = parsed.cicp.map(|c| {
                (c.color_primaries as u8, c.transfer_characteristics as u8,
                 c.matrix_coefficients as u8, c.full_range)
            });
            assert_eq!(
                our_result, parsed_cicp,
                "icc_extract_cicp disagrees with moxcms round-trip parse.\n\
                 ours={our_result:?}, moxcms_parsed={parsed_cicp:?}, struct_cicp={expected:?}"
            );
        }

        // Verify moxcms can round-trip.
        let parsed = ColorProfile::new_from_slice(&icc_bytes).expect("moxcms parse failed");
        if let (Some(ours), Some(theirs)) = (our_result, parsed.cicp) {
            assert_eq!(ours.0, theirs.color_primaries as u8, "primaries mismatch");
            assert_eq!(ours.1, theirs.transfer_characteristics as u8, "transfer mismatch");
            assert_eq!(ours.2, theirs.matrix_coefficients as u8, "matrix mismatch");
            assert_eq!(ours.3, theirs.full_range, "full_range mismatch");
        }
    }

    #[test]
    fn cross_validate_srgb() {
        cross_validate_moxcms(&ColorProfile::new_srgb());
    }

    #[test]
    fn cross_validate_display_p3() {
        cross_validate_moxcms(&ColorProfile::new_display_p3());
    }

    #[test]
    fn cross_validate_display_p3_pq() {
        cross_validate_moxcms(&ColorProfile::new_display_p3_pq());
    }

    #[test]
    fn cross_validate_bt2020_pq() {
        cross_validate_moxcms(&ColorProfile::new_bt2020_pq());
    }

    #[test]
    fn cross_validate_bt2020_hlg() {
        cross_validate_moxcms(&ColorProfile::new_bt2020_hlg());
    }

    #[test]
    fn cross_validate_bt2020_sdr() {
        cross_validate_moxcms(&ColorProfile::new_bt2020());
    }

    #[test]
    fn cross_validate_custom_cicp() {
        let cicp = CicpProfile {
            color_primaries: CicpColorPrimaries::Bt709,
            transfer_characteristics: TransferCharacteristics::Smpte2084,
            matrix_coefficients: MatrixCoefficients::Identity,
            full_range: false,
        };
        cross_validate_moxcms(&ColorProfile::new_from_cicp(cicp));
    }

    // -----------------------------------------------------------------------
    // Test against real ICC profiles from the zenpixels-convert bundle
    // (these are v4.0–v4.2, so should NOT have cicp tags).
    // -----------------------------------------------------------------------

    #[test]
    fn real_profiles_no_cicp() {
        extern crate std;
        let profile_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenpixels/zenpixels-convert/src/profiles");
        if !profile_dir.exists() {
            // Skip if sibling repo not present (CI)
            return;
        }
        for entry in std::fs::read_dir(&profile_dir).unwrap() {
            let entry: std::fs::DirEntry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e: &std::ffi::OsStr| e.to_str()) != Some("icc") {
                continue;
            }
            let data = std::fs::read(&path).unwrap();
            // These are all v4.0–v4.2 profiles — no cicp tag expected.
            assert_eq!(
                icc_extract_cicp(&data),
                None,
                "unexpected cicp tag in {}",
                path.display()
            );
            // But verify they parse as valid ICC (acsp signature present).
            assert_eq!(
                &data[36..40],
                b"acsp",
                "not a valid ICC profile: {}",
                path.display()
            );
        }
    }

    /// Cross-validate against saucecontrol/Compact-ICC-Profiles — the most
    /// widely embedded profiles in web images (Chrome, Firefox, imagemagick).
    /// All 44 profiles are v2.1 or v4.2 (pre-cicp), so our extractor must
    /// return None for every one. Also verify moxcms agrees.
    ///
    /// Clones the repo to a temp dir so this works on any platform with git.
    #[test]
    fn compact_icc_profiles_no_cicp() {
        extern crate std;
        use std::process::Command;

        let tmp = std::env::temp_dir().join("zencodec-compact-icc-profiles");
        let profile_dir = tmp.join("profiles");

        if !profile_dir.exists() {
            let status = Command::new("git")
                .args(["clone", "--depth", "1",
                       "https://github.com/saucecontrol/Compact-ICC-Profiles.git"])
                .arg(&tmp)
                .status();
            match status {
                Ok(s) if s.success() => {}
                _ => return, // git not available or network error — skip
            }
        }

        let mut count = 0;
        for entry in std::fs::read_dir(&profile_dir).unwrap() {
            let entry: std::fs::DirEntry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e: &std::ffi::OsStr| e.to_str()) != Some("icc") {
                continue;
            }
            let data = std::fs::read(&path).unwrap();
            let name = path.file_name().unwrap().to_string_lossy();

            // Our extractor must return None.
            assert_eq!(
                icc_extract_cicp(&data),
                None,
                "false positive cicp in compact profile: {name}"
            );

            // moxcms must also parse successfully and agree (no cicp).
            if let Ok(profile) = ColorProfile::new_from_slice(&data) {
                assert!(
                    profile.cicp.is_none(),
                    "moxcms found cicp in compact profile: {name}"
                );
            }
            // Some v2 micro/nano profiles may not parse in moxcms — that's OK.
            count += 1;
        }
        assert!(count >= 40, "expected at least 40 compact profiles, found {count}");
    }
}
