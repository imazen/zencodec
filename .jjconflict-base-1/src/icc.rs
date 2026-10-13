//! Lightweight ICC profile inspection.
//!
//! This module is a thin compatibility shim. The real implementation lives in
//! [`zenpixels::icc`] (enabled via the `icc` feature, already required by
//! zencodec). Prefer calling `zenpixels::icc::extract_cicp` directly — it
//! returns a typed [`zenpixels::Cicp`] instead of a tuple and comes with
//! richer identification helpers (`identify_common`, `is_common_srgb`).

/// Extract CICP (Coding-Independent Code Points) from an ICC profile's tag table.
///
/// Scans the ICC tag table for a `cicp` tag (ICC v4.4+, 12 bytes) and returns
/// the four CICP fields if found. Returns `None` for ICC v2 profiles (which
/// never contain cicp tags), profiles without a cicp tag, or malformed input.
///
/// # Returns
///
/// `Some((color_primaries, transfer_characteristics, matrix_coefficients, full_range))`
/// if a valid cicp tag is found, `None` otherwise.
///
/// # Deprecated
///
/// This wrapper is kept for backwards compatibility. Prefer
/// [`zenpixels::icc::extract_cicp`], which returns a typed
/// [`zenpixels::Cicp`] and lives alongside richer ICC identification
/// helpers. This function will be removed in the next breaking release.
#[deprecated(
    since = "0.1.16",
    note = "use zenpixels::icc::extract_cicp — returns a typed Cicp instead of a tuple"
)]
pub fn icc_extract_cicp(data: &[u8]) -> Option<(u8, u8, u8, bool)> {
    let c = zenpixels::icc::extract_cicp(data)?;
    Some((
        c.color_primaries,
        c.transfer_characteristics,
        c.matrix_coefficients,
        c.full_range,
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(deprecated)]
    use super::*;

    /// Build a minimal valid ICC profile with a cicp tag for testing.
    ///
    /// Shared test helper — used by `info.rs` tests and the local shim test.
    pub(crate) fn build_icc_with_cicp(cp: u8, tc: u8, mc: u8, fr: bool) -> alloc::vec::Vec<u8> {
        let mut data = alloc::vec![0u8; 256];
        let size = data.len() as u32;
        data[0..4].copy_from_slice(&size.to_be_bytes());
        data[36..40].copy_from_slice(b"acsp");
        data[128..132].copy_from_slice(&1u32.to_be_bytes());
        data[132..136].copy_from_slice(b"cicp");
        data[136..140].copy_from_slice(&144u32.to_be_bytes());
        data[140..144].copy_from_slice(&12u32.to_be_bytes());
        data[144..148].copy_from_slice(b"cicp");
        data[152] = cp;
        data[153] = tc;
        data[154] = mc;
        data[155] = if fr { 1 } else { 0 };
        data
    }

    /// Shim sanity check — delegating behavior is covered exhaustively in
    /// zenpixels::icc::tests. Here we just confirm the tuple mapping.
    #[test]
    fn shim_round_trip() {
        let icc = build_icc_with_cicp(9, 16, 0, true);
        assert_eq!(icc_extract_cicp(&icc), Some((9, 16, 0, true)));
    }

    #[test]
    fn shim_none_on_empty() {
        assert_eq!(icc_extract_cicp(&[]), None);
    }
}
