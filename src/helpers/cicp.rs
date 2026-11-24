//! Diagnostics for raw H.273 values.
// Adapted from imazen/zenpixels fde59b96632557592cffa9b7301acd8db96ed078.

use core::fmt;
use zenpixels::Cicp;

/// Human-readable formatting of a raw CICP tuple.
pub struct CicpDisplay(pub Cicp);

impl fmt::Display for CicpDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} / {} / {} ({})",
            color_primaries_name(self.0.color_primaries),
            transfer_characteristics_name(self.0.transfer_characteristics),
            matrix_coefficients_name(self.0.matrix_coefficients),
            if self.0.full_range {
                "full range"
            } else {
                "limited range"
            },
        )
    }
}

/// Human-readable name for the color primaries code (ITU-T H.273 Table 2).
pub fn color_primaries_name(code: u8) -> &'static str {
    match code {
        0 => "Reserved",
        1 => "BT.709/sRGB",
        2 => "Unspecified",
        4 => "BT.470M",
        5 => "BT.601 (625)",
        6 => "BT.601 (525)",
        7 => "SMPTE 240M",
        8 => "Generic Film",
        9 => "BT.2020",
        10 => "XYZ",
        11 => "SMPTE 431 (DCI-P3)",
        12 => "Display P3",
        22 => "EBU Tech 3213",
        _ => "Unknown",
    }
}

/// Human-readable name for the transfer characteristics code (ITU-T H.273 Table 3).
pub fn transfer_characteristics_name(code: u8) -> &'static str {
    match code {
        0 => "Reserved",
        1 => "BT.709",
        2 => "Unspecified",
        4 => "BT.470M (Gamma 2.2)",
        5 => "BT.470BG (Gamma 2.8)",
        6 => "BT.601",
        7 => "SMPTE 240M",
        8 => "Linear",
        9 => "Log 100:1",
        10 => "Log 316:1",
        11 => "IEC 61966-2-4",
        12 => "BT.1361",
        13 => "sRGB",
        14 => "BT.2020 (10-bit)",
        15 => "BT.2020 (12-bit)",
        16 => "PQ (HDR)",
        17 => "SMPTE 428",
        18 => "HLG (HDR)",
        _ => "Unknown",
    }
}

/// Human-readable name for the matrix coefficients code (ITU-T H.273 Table 4).
pub fn matrix_coefficients_name(code: u8) -> &'static str {
    match code {
        0 => "Identity/RGB",
        1 => "BT.709",
        2 => "Unspecified",
        4 => "FCC",
        5 => "BT.470BG",
        6 => "BT.601",
        7 => "SMPTE 240M",
        8 => "YCgCo",
        9 => "BT.2020 NCL",
        10 => "BT.2020 CL",
        11 => "SMPTE 2085",
        12 => "Chroma NCL",
        13 => "Chroma CL",
        14 => "ICtCp",
        15 => "IPT-C2",
        16 => "YCgCo-Re",
        17 => "YCgCo-Ro",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h273_v4_assigned_primaries_transfer_and_matrix_codes() {
        // ITU-T H.273 (07/2024), Tables 2–4. Names here are the public diagnostic spellings.
        let primaries = [
            (1, "BT.709/sRGB"),
            (2, "Unspecified"),
            (4, "BT.470M"),
            (5, "BT.601 (625)"),
            (6, "BT.601 (525)"),
            (7, "SMPTE 240M"),
            (8, "Generic Film"),
            (9, "BT.2020"),
            (10, "XYZ"),
            (11, "SMPTE 431 (DCI-P3)"),
            (12, "Display P3"),
            (22, "EBU Tech 3213"),
        ];
        let transfers = [
            (1, "BT.709"),
            (2, "Unspecified"),
            (4, "BT.470M (Gamma 2.2)"),
            (5, "BT.470BG (Gamma 2.8)"),
            (6, "BT.601"),
            (7, "SMPTE 240M"),
            (8, "Linear"),
            (9, "Log 100:1"),
            (10, "Log 316:1"),
            (11, "IEC 61966-2-4"),
            (12, "BT.1361"),
            (13, "sRGB"),
            (14, "BT.2020 (10-bit)"),
            (15, "BT.2020 (12-bit)"),
            (16, "PQ (HDR)"),
            (17, "SMPTE 428"),
            (18, "HLG (HDR)"),
        ];
        let matrices = [
            (0, "Identity/RGB"),
            (1, "BT.709"),
            (2, "Unspecified"),
            (4, "FCC"),
            (5, "BT.470BG"),
            (6, "BT.601"),
            (7, "SMPTE 240M"),
            (8, "YCgCo"),
            (9, "BT.2020 NCL"),
            (10, "BT.2020 CL"),
            (11, "SMPTE 2085"),
            (12, "Chroma NCL"),
            (13, "Chroma CL"),
            (14, "ICtCp"),
            (15, "IPT-C2"),
            (16, "YCgCo-Re"),
            (17, "YCgCo-Ro"),
        ];
        for (code, expected) in primaries {
            assert_eq!(color_primaries_name(code), expected);
        }
        for (code, expected) in transfers {
            assert_eq!(transfer_characteristics_name(code), expected);
        }
        for (code, expected) in matrices {
            assert_eq!(matrix_coefficients_name(code), expected);
        }
        for range in [0, 1] {
            let raw = [255, 3, 222, range];
            let result = CicpDisplay(Cicp::try_from_bytes(raw).unwrap());
            let s = alloc::format!("{result}");
            assert_eq!(
                s,
                if range == 0 {
                    "Unknown / Unknown / Unknown (limited range)"
                } else {
                    "Unknown / Unknown / Unknown (full range)"
                }
            );
            assert_eq!(result.0.to_bytes(), raw);
        }
        assert!(Cicp::try_from_bytes([1, 13, 0, 2]).is_err());
    }
}
