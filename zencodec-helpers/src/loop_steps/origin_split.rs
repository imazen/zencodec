//! Canonical train/val/test split for picker work — the ONE Rust source of
//! truth, a faithful port of `zenmetrics/scripts/picker/origin_split.py`.
//!
//! Rule (set by the user 2026-06-26): the split is by ORIGIN image, by the last
//! digit of the origin's numeric id, and EVERY sizing/crop/encode derivative of
//! an origin inherits the origin's bucket — so no derivative ever leaks across
//! the split:
//!
//! ```text
//! last digit of origin id in {0,2,4,6,8} -> Train
//!                           in {1,3,5}     -> Val
//!                           in {7,9}       -> Test
//! ```
//!
//! Origin-level (NOT per-rendition) and deterministic (NOT a seeded shuffle) —
//! reproducible across blind sessions with zero state. Use this everywhere
//! (`omni_merge`, corpus segmentation, eval); do NOT re-implement the rule.

/// Which split bucket an origin (and all its renditions) belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Split {
    Train,
    Val,
    Test,
}

impl Split {
    /// Lowercase label matching the Python (`"train" | "val" | "test"`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Val => "val",
            Self::Test => "test",
        }
    }
}

/// Strip the directory + a trailing `.scale{W}x{H}(.png)` rendition suffix to
/// the origin stem. Mirrors the Python `_REND` regex (case-insensitive,
/// optional `.png`).
///
/// ```
/// use zencodec_helpers::loop_tools::origin_split::origin_stem;
/// assert_eq!(origin_stem("/data/o_1004.scale48x64.png"), "o_1004");
/// assert_eq!(origin_stem("v2_src0001.png.scale256x250.png"), "v2_src0001.png");
/// ```
pub fn origin_stem(name: &str) -> &str {
    let base = name.rsplit('/').next().unwrap_or(name);
    strip_rendition_suffix(base)
}

/// Remove one trailing `.scale<digits>x<digits>` optionally followed by
/// `.png`/`.PNG`. Done by hand (no regex dep) — the suffix grammar is fixed.
fn strip_rendition_suffix(base: &str) -> &str {
    // Peel an optional trailing ".png"/".PNG".
    let no_png = base
        .strip_suffix(".png")
        .or_else(|| base.strip_suffix(".PNG"))
        .unwrap_or(base);
    // Find the last ".scale" and verify it's followed by <digits>x<digits>
    // running to the end of `no_png`.
    if let Some(pos) = no_png.rfind(".scale") {
        let after = &no_png[pos + ".scale".len()..];
        if is_dims(after) {
            return &base[..pos];
        }
    }
    base
}

/// `true` iff `s` is exactly `<digits>x<digits>` (the WxH rendition tail).
fn is_dims(s: &str) -> bool {
    let Some((w, h)) = s.split_once('x') else {
        return false;
    };
    !w.is_empty()
        && !h.is_empty()
        && w.bytes().all(|b| b.is_ascii_digit())
        && h.bytes().all(|b| b.is_ascii_digit())
}

/// The origin's numeric id — the LEADING stem (after an optional `o_` /
/// `v2_src` prefix), or `None` if there's no leading numeric stem. Leading-stem
/// is robust to crop labels (`o_1004_c25_tl`), trailing dimensions
/// (`1003_..._4000x3000`), and descriptive suffixes — all of which a
/// trailing-number rule would wrongly grab.
///
/// ```
/// use zencodec_helpers::loop_tools::origin_split::origin_id;
/// assert_eq!(origin_id("o_1004.scale48x64.png"), Some("1004"));
/// assert_eq!(origin_id("v2_src0009.png.scale512x500.png"), Some("0009"));
/// assert_eq!(origin_id("1003_general_oceanfront_4000x3000.sdr.png"), Some("1003"));
/// assert_eq!(origin_id("untitled.png"), None);
/// ```
pub fn origin_id(name: &str) -> Option<&str> {
    let base = origin_stem(name);
    // Patterns tried in order, mirroring Python `_STEM_PATS`:
    //   ^o_(\d+) | ^v2_src(\d+) | ^(\d+)
    for prefix in ["o_", "v2_src"] {
        if let Some(rest) = base.strip_prefix(prefix) {
            let digits = leading_digits(rest);
            if !digits.is_empty() {
                return Some(digits);
            }
        }
    }
    let digits = leading_digits(base);
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// The longest leading run of ASCII digits in `s` (possibly empty).
fn leading_digits(s: &str) -> &str {
    let end = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    &s[..end]
}

/// Last digit of the origin's (leading) numeric id, or `None`.
pub fn origin_id_last_digit(name: &str) -> Option<u8> {
    origin_id(name).and_then(|id| id.bytes().last().map(|b| b - b'0'))
}

/// `Some(Split)` per the canonical rule, or `None` for an unsplittable name
/// (no numeric origin id).
///
/// ```
/// use zencodec_helpers::loop_tools::origin_split::{split_of, Split};
/// assert_eq!(split_of("o_1004.scale48x64.png"), Some(Split::Train)); // 4
/// assert_eq!(split_of("o_1003.scale72x96.png"), Some(Split::Val));   // 3
/// assert_eq!(split_of("o_1007.scale72x96.png"), Some(Split::Test));  // 7
/// ```
pub fn split_of(name: &str) -> Option<Split> {
    match origin_id_last_digit(name)? {
        0 | 2 | 4 | 6 | 8 => Some(Split::Train),
        1 | 3 | 5 => Some(Split::Val),
        7 | 9 => Some(Split::Test),
        _ => None, // unreachable: a single decimal digit is 0..=9
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The exact case table from the Python self-test — asserts byte-for-byte
    // parity with the canonical rule.
    #[test]
    fn matches_python_self_test_cases() {
        let cases: &[(&str, Split)] = &[
            ("o_1004.scale48x64.png", Split::Train),
            ("/data/o_1004.scale48x64.png", Split::Train),
            ("o_1003.scale72x96.png", Split::Val),
            ("o_1007.scale72x96.png", Split::Test),
            ("v2_src0001.png.scale256x250.png", Split::Val), // 1
            ("v2_src0002.png.scale49x64.png", Split::Train), // 2
            ("v2_src0009.png.scale512x500.png", Split::Test), // 9
            ("1000", Split::Train),
            ("1005", Split::Val),
            ("1009", Split::Test),
            // leading-stem robustness (cases a trailing-number rule gets wrong):
            ("o_1004_c25_tl.scale48x64.png", Split::Train), // o_1004, not crop 25
            ("1003_general_oceanfront_4000x3000.sdr.png", Split::Val), // 1003, not 3000
            (
                "9736_gen_products_brass-clock_p0497_1024x1024.sdr.png",
                Split::Train,
            ), // 9736→6
        ];
        for (name, want) in cases {
            assert_eq!(split_of(name), Some(*want), "split_of({name})");
        }
    }

    #[test]
    fn origin_stem_strips_only_real_rendition_suffix() {
        assert_eq!(origin_stem("o_1004.scale48x64.png"), "o_1004");
        assert_eq!(origin_stem("o_1004.scale48x64"), "o_1004");
        // ".scale" not followed by WxH must NOT be stripped.
        assert_eq!(
            origin_stem("photo.scalefactor.png"),
            "photo.scalefactor.png"
        );
        // descriptive name with a trailing dimension is not a rendition suffix.
        assert_eq!(
            origin_stem("1003_general_oceanfront_4000x3000.sdr.png"),
            "1003_general_oceanfront_4000x3000.sdr.png"
        );
    }

    #[test]
    fn unsplittable_when_no_leading_numeric_id() {
        assert_eq!(origin_id("untitled.png"), None);
        assert_eq!(split_of("untitled.png"), None);
        assert_eq!(origin_id_last_digit("untitled.png"), None);
    }

    #[test]
    fn no_derivative_leaks_across_split() {
        // Every rendition / crop of the same origin lands in the same bucket.
        let renditions = [
            "o_1004.scale48x64.png",
            "o_1004.scale256x256.png",
            "o_1004_c25_tl.scale48x64.png",
            "/abs/path/o_1004.scale4096x4096.png",
        ];
        for r in renditions {
            assert_eq!(
                split_of(r),
                Some(Split::Train),
                "{r} must inherit origin bucket"
            );
        }
    }
}
