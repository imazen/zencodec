//! Omni-merge — join a sweep's omni TSV rows (concatenated across box
//! sidecars) with the rendition features TSV on `variant_name`, emitting the
//! trainer's PARETO rows.
//!
//! Ports `omni_to_pareto.py`. Two reusable pieces the per-codec loops all need:
//!
//!   1. **Cross-box concatenation** ([`MergeBuilder::add_omni_tsv`]) — a fleet
//!      sweep writes one omni TSV per box (or per chunk); they share a schema
//!      and concatenate. This dedups on the cell identity so a chunk re-run that
//!      overlaps doesn't double-count.
//!   2. **Feature join** ([`MergeBuilder::join_features`]) — inner-join the omni
//!      rows to the features TSV on `variant_name` (derived from `image_path`
//!      via [`variant_of`]), attaching `width`, `height`, and the `feat_*`
//!      columns, computing `size_class`, the stable `config_id`, and the
//!      per-(image, size_class) `effective_max_<metric>`.
//!
//! The omni TSV schema (from the unified sweep):
//! `image_path, codec, q, knob_tuple_json={"cell","fp","plan"}, encoded_bytes,
//!  encode_ms, …, score_<metric>`. The `config_name` is the plan cell-id
//! extracted from `knob_tuple_json`.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::variant::{SizeClass, variant_of};

/// A single omni row's `column -> value` accessor. The returned borrow is tied
/// to `&self`, which sidesteps the higher-ranked-lifetime trap a bare
/// `Fn(&str) -> Option<&str>` bound hits (the value borrows the *row*, not the
/// *column name*, so a `for<'a>` closure bound can't express it). Implemented
/// for `&[(&str, &str)]` (literal column tables, used by tests) and for
/// [`ColumnRecord`] (a row + header index, used by the CLI over a TSV row).
pub trait Record {
    fn get(&self, column: &str) -> Option<&str>;
}

impl Record for &[(&str, &str)] {
    fn get(&self, column: &str) -> Option<&str> {
        self.iter().find(|(k, _)| *k == column).map(|(_, v)| *v)
    }
}

/// A [`Record`] over one TSV row: the row's cells plus a `column -> index` map.
/// The CLI builds one per row, borrowing both from the parsed sidecar.
pub struct ColumnRecord<'a> {
    /// The row's cell values, in header order.
    pub cells: &'a [String],
    /// Shared `column name -> cell index` lookup (built once per sidecar).
    pub index: &'a alloc::collections::BTreeMap<&'a str, usize>,
}

impl Record for ColumnRecord<'_> {
    fn get(&self, column: &str) -> Option<&str> {
        self.index
            .get(column)
            .and_then(|&i| self.cells.get(i))
            .map(String::as_str)
    }
}

/// One fully-merged PARETO row — the trainer's input record. Field names match
/// the parquet columns `omni_to_pareto.py` writes.
#[derive(Clone, Debug, PartialEq)]
pub struct ParetoRow {
    /// The `variant_name` (renamed to `image_path` in the trainer's parquet).
    pub image_path: String,
    pub size_class: SizeClass,
    pub width: u32,
    pub height: u32,
    /// Stable integer id for `config_name` (sorted-unique order).
    pub config_id: i64,
    /// The plan cell-id (`knob_tuple_json["cell"]`).
    pub config_name: String,
    pub q: f64,
    pub bytes: i64,
    /// The target metric value (named `metric` here; the trainer's column is
    /// conventionally `zensim` regardless of which metric fills it).
    pub metric: f64,
    pub encode_ms: f64,
    pub total_ms: f64,
    /// Best achievable metric for this (image, size_class) — the per-row
    /// reachability ceiling.
    pub effective_max_metric: f64,
}

/// A raw omni row, post-parse, pre-join. The identity `(image_path, config_name,
/// q)` dedups cross-box overlap.
#[derive(Clone, Debug)]
struct OmniRow {
    image_path: String,
    config_name: String,
    q: f64,
    bytes: i64,
    metric: f64,
    encode_ms: f64,
}

/// A feature row keyed on `variant_name`: dims + the `feat_*` values.
#[derive(Clone, Debug)]
pub struct FeatureRow {
    pub variant_name: String,
    pub width: u32,
    pub height: u32,
    /// `feat_<name>` → value, in declared order is not preserved here (the
    /// trainer reads by name); a `Vec` of pairs keeps it light.
    pub feats: Vec<(String, f64)>,
}

/// Errors a merge can surface (schema problems, not data-quality warnings).
#[derive(Clone, Debug, PartialEq)]
pub enum MergeError {
    /// The omni TSV lacked a required column.
    MissingOmniColumn(String),
    /// The configured metric score column wasn't present.
    MissingMetricColumn(String),
    /// `knob_tuple_json` didn't carry a `"cell"` key.
    NoCellInKnobTuple(String),
    /// A numeric field failed to parse.
    BadNumber { column: String, value: String },
}

/// Accumulates omni rows across box sidecars, then joins to features.
#[derive(Default)]
pub struct MergeBuilder {
    /// Dedup map keyed on `(image_path, config_name, q-bits)` → row. Last write
    /// wins (a re-run's row replaces an earlier identical-identity one).
    rows: BTreeMap<(String, String, u64), OmniRow>,
}

impl MergeBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many distinct omni rows have accumulated.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Ingest one omni sidecar's parsed rows. `header` is the TSV header;
    /// `records` yields each row as a [`Record`] (`column -> value` lookup —
    /// the caller owns TSV reading; the CLI uses the `csv` crate, this stays
    /// IO-agnostic so it is unit-testable). `metric_col` is the score column to
    /// read into `metric` (e.g. `score_ssim2_gpu`).
    ///
    /// Identical `(image_path, config_name, q)` across sidecars is deduped
    /// (last-write-wins), so overlapping chunk re-runs don't double-count.
    pub fn add_omni_rows<I, R>(
        &mut self,
        header: &BTreeSet<String>,
        metric_col: &str,
        records: I,
    ) -> Result<usize, MergeError>
    where
        I: IntoIterator<Item = R>,
        R: Record,
    {
        for col in ["image_path", "knob_tuple_json", "encoded_bytes", "q"] {
            if !header.contains(col) {
                return Err(MergeError::MissingOmniColumn(col.to_string()));
            }
        }
        if !header.contains(metric_col) {
            return Err(MergeError::MissingMetricColumn(metric_col.to_string()));
        }
        let mut added = 0;
        for row in records {
            let image_path = row.get("image_path").unwrap_or("").to_string();
            let knob = row.get("knob_tuple_json").unwrap_or("");
            let config_name = cell_from_knob_tuple(knob)
                .ok_or_else(|| MergeError::NoCellInKnobTuple(knob.to_string()))?
                .to_string();
            let q = parse_f64(row.get("q").unwrap_or(""), "q")?;
            let bytes = parse_f64(row.get("encoded_bytes").unwrap_or(""), "encoded_bytes")? as i64;
            let metric = parse_f64(row.get(metric_col).unwrap_or(""), metric_col)?;
            // encode_ms is optional (the Python defaults it to 0.0).
            let encode_ms = match row.get("encode_ms") {
                Some(s) if !s.is_empty() => parse_f64(s, "encode_ms")?,
                _ => 0.0,
            };
            let key = (image_path.clone(), config_name.clone(), q.to_bits());
            self.rows.insert(
                key,
                OmniRow {
                    image_path,
                    config_name,
                    q,
                    bytes,
                    metric,
                    encode_ms,
                },
            );
            added += 1;
        }
        Ok(added)
    }

    /// Inner-join the accumulated omni rows to `features` on `variant_name`,
    /// producing the trainer's PARETO rows. Omni rows whose variant has no
    /// feature row are dropped (reported via the returned `dropped` count).
    ///
    /// Returns `(rows, dropped)` where `dropped` is the number of distinct
    /// variants with no matching feature row — the Python's WARNING count.
    pub fn join_features(self, features: &[FeatureRow]) -> (Vec<ParetoRow>, usize) {
        let by_variant: BTreeMap<&str, &FeatureRow> = features
            .iter()
            .map(|f| (f.variant_name.as_str(), f))
            .collect();

        // Stable config_id: sorted-unique config_name → index, computed over
        // the JOINED set (matches the Python, which indexes post-merge).
        let mut joined: Vec<(OmniRow, &FeatureRow)> = Vec::new();
        let mut dropped_variants: BTreeSet<String> = BTreeSet::new();
        for (_, row) in self.rows {
            let vname = variant_of(&row.image_path).to_string();
            match by_variant.get(vname.as_str()) {
                Some(fr) => joined.push((row, *fr)),
                None => {
                    dropped_variants.insert(vname);
                }
            }
        }

        let mut cfg_names: Vec<&str> = joined.iter().map(|(r, _)| r.config_name.as_str()).collect();
        cfg_names.sort_unstable();
        cfg_names.dedup();
        let cfg_index: BTreeMap<&str, i64> = cfg_names
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, i as i64))
            .collect();

        // First pass: build rows without effective_max; second pass fills it
        // per (variant, size_class).
        let mut rows: Vec<ParetoRow> = joined
            .iter()
            .map(|(r, fr)| {
                let size_class = SizeClass::from_dims(fr.width, fr.height);
                ParetoRow {
                    image_path: variant_of(&r.image_path).to_string(),
                    size_class,
                    width: fr.width,
                    height: fr.height,
                    config_id: cfg_index[r.config_name.as_str()],
                    config_name: r.config_name.clone(),
                    q: r.q,
                    bytes: r.bytes,
                    metric: r.metric,
                    encode_ms: r.encode_ms,
                    total_ms: r.encode_ms,
                    effective_max_metric: f64::NEG_INFINITY,
                }
            })
            .collect();

        // effective_max_metric = max metric per (image_path, size_class).
        let mut max_by_group: BTreeMap<(String, &'static str), f64> = BTreeMap::new();
        for r in &rows {
            let e = max_by_group
                .entry((r.image_path.clone(), r.size_class.label()))
                .or_insert(f64::NEG_INFINITY);
            if r.metric > *e {
                *e = r.metric;
            }
        }
        for r in &mut rows {
            r.effective_max_metric = max_by_group[&(r.image_path.clone(), r.size_class.label())];
        }

        // Deterministic output order: (image_path, config_id, q).
        rows.sort_by(|a, b| {
            a.image_path
                .cmp(&b.image_path)
                .then(a.config_id.cmp(&b.config_id))
                .then(a.q.partial_cmp(&b.q).unwrap_or(core::cmp::Ordering::Equal))
        });
        (rows, dropped_variants.len())
    }
}

/// Extract the `"cell"` field from a `knob_tuple_json` string without a full
/// JSON parser dependency in the hot path — the value is always a quoted string
/// under the `"cell"` key. Returns the cell-id slice, or `None` if absent.
///
/// The loop-tools CLI uses `serde_json` for robustness; this lightweight
/// extractor keeps the core merge logic dependency-light and is what the unit
/// tests exercise. It handles the canonical shape `{"cell":"...","fp":...}`.
pub fn cell_from_knob_tuple(knob_tuple_json: &str) -> Option<&str> {
    let key = "\"cell\"";
    let start = knob_tuple_json.find(key)? + key.len();
    let rest = knob_tuple_json[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn parse_f64(s: &str, column: &str) -> Result<f64, MergeError> {
    s.trim().parse::<f64>().map_err(|_| MergeError::BadNumber {
        column: column.to_string(),
        value: s.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(cols: &[&str]) -> BTreeSet<String> {
        cols.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cell_from_knob_tuple_extracts_cell() {
        assert_eq!(
            cell_from_knob_tuple(r#"{"cell":"vd-e7_zen_def","fp":12345,"plan":"x"}"#),
            Some("vd-e7_zen_def")
        );
        assert_eq!(
            cell_from_knob_tuple(r#"{ "cell" : "jp3_tr14_small_420" }"#),
            Some("jp3_tr14_small_420")
        );
        assert_eq!(cell_from_knob_tuple(r#"{"fp":1}"#), None);
    }

    // The `Record` item type for the slice impl.
    type Rec<'a> = &'a [(&'a str, &'a str)];

    #[test]
    fn missing_required_column_errors() {
        let mut b = MergeBuilder::new();
        let h = header(&["image_path", "q", "encoded_bytes", "score_x"]); // no knob_tuple_json
        let err = b
            .add_omni_rows(&h, "score_x", core::iter::empty::<Rec>())
            .unwrap_err();
        assert_eq!(
            err,
            MergeError::MissingOmniColumn("knob_tuple_json".to_string())
        );
    }

    #[test]
    fn missing_metric_column_errors() {
        let mut b = MergeBuilder::new();
        let h = header(&["image_path", "q", "encoded_bytes", "knob_tuple_json"]);
        let err = b
            .add_omni_rows(&h, "score_ssim2_gpu", core::iter::empty::<Rec>())
            .unwrap_err();
        assert_eq!(
            err,
            MergeError::MissingMetricColumn("score_ssim2_gpu".to_string())
        );
    }

    #[test]
    fn dedups_cross_box_overlap_last_write_wins() {
        let mut b = MergeBuilder::new();
        let h = header(&[
            "image_path",
            "q",
            "encoded_bytes",
            "knob_tuple_json",
            "score_x",
        ]);
        let knob = r#"{"cell":"c0"}"#;
        // Box 1 row.
        let row1: Rec = &[
            ("image_path", "/d/o_1000.png"),
            ("q", "50"),
            ("encoded_bytes", "1000"),
            ("knob_tuple_json", knob),
            ("score_x", "80.0"),
        ];
        b.add_omni_rows(&h, "score_x", [row1]).unwrap();
        // Box 2 re-run, same identity, different bytes — should replace.
        let row2: Rec = &[
            ("image_path", "/d/o_1000.png"),
            ("q", "50"),
            ("encoded_bytes", "900"),
            ("knob_tuple_json", knob),
            ("score_x", "81.0"),
        ];
        b.add_omni_rows(&h, "score_x", [row2]).unwrap();
        assert_eq!(b.len(), 1, "identical identity deduped");
        let feats = [FeatureRow {
            variant_name: "o_1000".to_string(),
            width: 100,
            height: 100,
            feats: alloc::vec![("feat_a".to_string(), 1.0)],
        }];
        let (rows, dropped) = b.join_features(&feats);
        assert_eq!(dropped, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bytes, 900, "last write won");
        assert_eq!(rows[0].metric, 81.0);
    }

    #[test]
    fn join_computes_size_class_config_id_and_effective_max() {
        let mut b = MergeBuilder::new();
        let h = header(&[
            "image_path",
            "q",
            "encoded_bytes",
            "knob_tuple_json",
            "score_x",
            "encode_ms",
        ]);
        // Two configs on one variant, two q's — effective_max picks the best.
        let add = |b: &mut MergeBuilder, cell: &str, q: &str, by: &str, sc: &str| {
            let knob = alloc::format!(r#"{{"cell":"{cell}"}}"#);
            let row: Rec = &[
                ("image_path", "/d/o_1004.scale256x256.png"),
                ("q", q),
                ("encoded_bytes", by),
                ("knob_tuple_json", &knob),
                ("score_x", sc),
                ("encode_ms", "5.0"),
            ];
            b.add_omni_rows(&h, "score_x", [row]).unwrap();
        };
        add(&mut b, "cA", "50", "1000", "70.0");
        add(&mut b, "cB", "90", "2000", "95.0");
        let feats = [FeatureRow {
            variant_name: "o_1004.scale256x256".to_string(),
            width: 256,
            height: 256,
            feats: alloc::vec![],
        }];
        let (rows, dropped) = b.join_features(&feats);
        assert_eq!(dropped, 0);
        assert_eq!(rows.len(), 2);
        // 256×256 = 65536 ≤ 65536 → small.
        assert!(rows.iter().all(|r| r.size_class == SizeClass::Small));
        // effective_max = 95.0 across both rows of the variant.
        assert!(
            rows.iter()
                .all(|r| (r.effective_max_metric - 95.0).abs() < 1e-9)
        );
        // config_id is stable sorted: cA=0, cB=1.
        let ca = rows.iter().find(|r| r.config_name == "cA").unwrap();
        let cb = rows.iter().find(|r| r.config_name == "cB").unwrap();
        assert_eq!(ca.config_id, 0);
        assert_eq!(cb.config_id, 1);
        assert_eq!(ca.total_ms, 5.0); // total_ms defaults to encode_ms
    }

    #[test]
    fn variant_with_no_feature_row_is_dropped_and_counted() {
        let mut b = MergeBuilder::new();
        let h = header(&[
            "image_path",
            "q",
            "encoded_bytes",
            "knob_tuple_json",
            "score_x",
        ]);
        let row: Rec = &[
            ("image_path", "/d/o_9999.png"),
            ("q", "50"),
            ("encoded_bytes", "1000"),
            ("knob_tuple_json", r#"{"cell":"c0"}"#),
            ("score_x", "80.0"),
        ];
        b.add_omni_rows(&h, "score_x", [row]).unwrap();
        // features TSV doesn't have o_9999.
        let feats = [FeatureRow {
            variant_name: "o_1000".to_string(),
            width: 10,
            height: 10,
            feats: alloc::vec![],
        }];
        let (rows, dropped) = b.join_features(&feats);
        assert!(rows.is_empty());
        assert_eq!(dropped, 1, "the unmatched variant is reported");
    }
}
