//! Feature preparation — the `feat_<name>` column selection + the parameter-
//! free `log1p` transform decision.
//!
//! Ports the shared logic of `configs/picker_config_common.py`:
//!   - `keep_features()` — intersect a desired feature list with the columns a
//!     features TSV actually has (a bake must never reference a column the
//!     extractor didn't emit);
//!   - `feature_transforms()` — the `log1p` map restricted to heavy-tailed,
//!     strictly-positive features that are ALSO in the keep-set. `log1p` is
//!     parameter-free (applied before StandardScaler, baked into the model so
//!     inference matches) — corpus-stable, unlike winsor/clip which need
//!     per-corpus `[p1,p99]` params.
//!
//! These are data-prep decisions, not codec-specific — every codec's loop wants
//! the identical keep + transform discipline so a feature added to one picker
//! is treated the same in all.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A feature transform the trainer applies before the scaler and bakes into the
/// model JSON. Only the parameter-free ones live here; corpus-parameterized
/// transforms (winsor / clip_then_log1p) are deliberately excluded — they need
/// `[p1,p99]` stats this layer doesn't have.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureTransform {
    /// `log1p(x)` — compresses a heavy positive tail to near-Gaussian. The only
    /// transform applied by default.
    Log1p,
}

impl FeatureTransform {
    pub fn name(self) -> &'static str {
        match self {
            Self::Log1p => "log1p",
        }
    }
}

/// The canonical heavy-tailed, strictly-positive feature names that get
/// `log1p`. Measured on `imazen26_train_features_2026-06-22`: `pixel_count`
/// tail 9352×, `laplacian_variance` 365×, `luma_kurtosis` 367×, the chroma
/// horiz/vert/peak sharpness family 80-180×. Kept in lockstep with
/// `picker_config_common._LOG1P_FEATURES`.
pub const LOG1P_FEATURES: &[&str] = &[
    "feat_pixel_count",
    "feat_variance",
    "feat_laplacian_variance",
    "feat_laplacian_variance_p50",
    "feat_laplacian_variance_p75",
    "feat_high_freq_energy_ratio",
    "feat_dct_compressibility_y",
    "feat_dct_compressibility_uv",
    "feat_cb_horiz_sharpness",
    "feat_cb_vert_sharpness",
    "feat_cb_peak_sharpness",
    "feat_cr_horiz_sharpness",
    "feat_cr_vert_sharpness",
    "feat_cr_peak_sharpness",
    "feat_luma_kurtosis",
];

/// Intersect a desired feature list with the columns actually present in a
/// features TSV header, preserving the *desired* order (so the bake's input
/// vector layout is stable regardless of the TSV's column order).
///
/// `wanted` is the codec's curated feature list; `available` is the set of
/// column names the features TSV carries (typically every header cell starting
/// `feat_`, but the caller passes the full header set and this filters). A
/// feature in `wanted` but absent from `available` is dropped — a bake must
/// never reference a column the extractor didn't emit.
pub fn keep_features<'a>(wanted: &[&'a str], available: &BTreeSet<String>) -> Vec<&'a str> {
    wanted
        .iter()
        .copied()
        .filter(|w| available.contains(*w))
        .collect()
}

/// The `log1p` transform map for the keep-set: every [`LOG1P_FEATURES`] member
/// that survived [`keep_features`]. Mirrors `feature_transforms()` — restricted
/// to features actually present so the bake never declares a transform for a
/// column the model has no input for.
///
/// Pass `disable = true` (the `PICKER_NO_TRANSFORMS=1` A/B knob) to get an empty
/// map.
pub fn feature_transforms(keep: &[&str], disable: bool) -> BTreeMap<String, FeatureTransform> {
    if disable {
        return BTreeMap::new();
    }
    let keep_set: BTreeSet<&str> = keep.iter().copied().collect();
    LOG1P_FEATURES
        .iter()
        .filter(|f| keep_set.contains(**f))
        .map(|f| (f.to_string(), FeatureTransform::Log1p))
        .collect()
}

/// Pull the `feat_`-prefixed column names out of a TSV header row, in header
/// order. The convenience used by [`omni_merge`](super::omni_merge) and the CLI
/// when the caller hasn't curated a `wanted` list and just wants "all feature
/// columns."
pub fn feature_columns(header: &[String]) -> Vec<String> {
    header
        .iter()
        .filter(|c| c.starts_with("feat_"))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn keep_features_intersects_preserving_wanted_order() {
        let wanted = ["feat_b", "feat_a", "feat_missing", "feat_c"];
        let available = set(&["feat_a", "feat_b", "feat_c", "feat_unused"]);
        // wanted order kept; feat_missing dropped; feat_unused not requested.
        assert_eq!(
            keep_features(&wanted, &available),
            ["feat_b", "feat_a", "feat_c"]
        );
    }

    #[test]
    fn transforms_restricted_to_present_keep_features() {
        // pixel_count + laplacian_variance are log1p features; edge_density is
        // not. Only the present log1p ones get a transform.
        let keep = [
            "feat_pixel_count",
            "feat_edge_density",
            "feat_laplacian_variance",
        ];
        let t = feature_transforms(&keep, false);
        assert_eq!(t.get("feat_pixel_count"), Some(&FeatureTransform::Log1p));
        assert_eq!(
            t.get("feat_laplacian_variance"),
            Some(&FeatureTransform::Log1p)
        );
        assert!(
            !t.contains_key("feat_edge_density"),
            "non-tail feature untouched"
        );
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn disable_yields_empty_map() {
        let keep = ["feat_pixel_count"];
        assert!(feature_transforms(&keep, true).is_empty());
    }

    #[test]
    fn log1p_feature_not_in_keep_is_not_declared() {
        // A log1p-eligible feature that wasn't kept must NOT get a transform
        // (the model has no input for it).
        let keep = ["feat_edge_density"]; // no log1p features kept
        assert!(feature_transforms(&keep, false).is_empty());
    }

    #[test]
    fn feature_columns_extracts_feat_prefixed() {
        let header: Vec<String> = ["image_path", "width", "feat_a", "height", "feat_b"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(feature_columns(&header), ["feat_a", "feat_b"]);
    }
}
