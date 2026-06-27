//! `variant_name` derivation — the join key between a sweep's omni TSV and the
//! rendition features TSV.
//!
//! Ports `variant_of()` from `zenmetrics/scripts/picker/omni_to_pareto.py`: the
//! features TSV is keyed on `variant_name` = the rendition's basename with a
//! `.png`/`.PNG` extension stripped, while the omni TSV carries a full
//! `image_path`. Both sides must derive the key the same way or the inner join
//! silently drops rows — so this is the ONE place the rule lives.

/// Derive the `variant_name` join key from an image path.
///
/// Strips the directory and one trailing `.png` / `.PNG` extension. A path with
/// any other extension (or none) keeps its full basename — matching the Python,
/// which only special-cases `.png`.
///
/// ```
/// use zencodec_helpers::loop_tools::variant::variant_of;
/// assert_eq!(variant_of("/data/o_1004.scale48x64.png"), "o_1004.scale48x64");
/// assert_eq!(variant_of("o_1004.scale48x64.PNG"), "o_1004.scale48x64");
/// assert_eq!(variant_of("thumb.jpg"), "thumb.jpg"); // non-png basename kept whole
/// ```
pub fn variant_of(image_path: &str) -> &str {
    let base = image_path.rsplit('/').next().unwrap_or(image_path);
    base.strip_suffix(".png")
        .or_else(|| base.strip_suffix(".PNG"))
        .unwrap_or(base)
}

/// Size class from a pixel count, matching the trainer's `size_class()` buckets
/// (the `<=` boundaries are deliberate — they match the Python exactly).
///
/// Per the calibration discipline these four buckets bracket fixed-overhead vs
/// per-pixel cost: tiny ≤ 64×64, small ≤ 256×256, medium ≤ 1024×1024, large
/// above.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SizeClass {
    Tiny,
    Small,
    Medium,
    Large,
}

impl SizeClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    /// Classify by total pixel count (`width * height`).
    pub fn from_pixels(pixels: u64) -> Self {
        if pixels <= 64 * 64 {
            Self::Tiny
        } else if pixels <= 256 * 256 {
            Self::Small
        } else if pixels <= 1024 * 1024 {
            Self::Medium
        } else {
            Self::Large
        }
    }

    /// Classify from dimensions.
    pub fn from_dims(width: u32, height: u32) -> Self {
        Self::from_pixels(u64::from(width) * u64::from(height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_of_strips_dir_and_png() {
        assert_eq!(
            variant_of("/data/sub/o_1004.scale48x64.png"),
            "o_1004.scale48x64"
        );
        assert_eq!(variant_of("o_1004.scale48x64.PNG"), "o_1004.scale48x64");
        assert_eq!(variant_of("o_1004.scale48x64"), "o_1004.scale48x64");
        // double-png basename: only the outer extension is stripped (Python parity).
        assert_eq!(
            variant_of("v2_src0001.png.scale49x64.png"),
            "v2_src0001.png.scale49x64"
        );
        assert_eq!(variant_of("thumb.jpg"), "thumb.jpg");
    }

    #[test]
    fn size_class_buckets_match_python_boundaries() {
        assert_eq!(SizeClass::from_dims(64, 64), SizeClass::Tiny); // 4096 ≤ 4096
        assert_eq!(SizeClass::from_dims(64, 65), SizeClass::Small); // 4160 > 4096
        assert_eq!(SizeClass::from_dims(256, 256), SizeClass::Small); // 65536 ≤ 65536
        assert_eq!(SizeClass::from_dims(257, 256), SizeClass::Medium);
        assert_eq!(SizeClass::from_dims(1024, 1024), SizeClass::Medium); // = 1048576
        assert_eq!(SizeClass::from_dims(1025, 1024), SizeClass::Large);
        assert_eq!(SizeClass::from_dims(4096, 4096), SizeClass::Large);
    }
}
