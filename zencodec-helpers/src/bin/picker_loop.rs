//! `picker-loop` — the consistent per-codec picker-training-loop CLI.
//!
//! Thin IO wrapper over [`zencodec_helpers::loop_tools`]: it reads/writes TSVs
//! with the `csv` crate and delegates every decision to the library so the
//! logic is the same one the unit tests exercise. Subcommands:
//!
//! ```text
//! picker-loop merge --omni A.tsv [--omni B.tsv ...] --features F.tsv \
//!     --metric-col score_ssim2_gpu --out-pareto P.tsv
//!         Concatenate omni sidecars (dedup), inner-join to features on
//!         variant_name, emit the trainer's PARETO rows. Prints row/config/
//!         size-class summary + dropped-variant count.
//!
//! picker-loop split --pareto P.tsv [--out-train T.tsv --out-val V.tsv --out-test E.tsv]
//!         Label each row train/val/test by the canonical origin split and
//!         (optionally) write the three partitions. Prints per-bucket counts.
//!
//! picker-loop validate-cells --omni A.tsv [--omni B.tsv ...] --grammar jxl-lossy-dense
//!         Run the parse / roundtrip / max-deviation gate over the distinct
//!         cell-ids. Exits non-zero on any failure (the test_lossy_dense_parse
//!         contract).
//!
//! picker-loop topk-eval --pareto P.tsv --target Q [--ks 1,2,3,5] [--metric-dir higher|lower]
//!         Compute the top-K-verify oracle gap per K from a PARETO TSV (groups
//!         rows by image_path×size_class as the cell space, reach = metric
//!         meets target). Prints the per-K mean/p50/p90/p99/max % + hit-rate.
//! ```

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use zencodec_helpers::loop_tools::feature_prep::feature_columns;
use zencodec_helpers::loop_tools::grammars::JxlLossyDense;
use zencodec_helpers::loop_tools::omni_merge::{ColumnRecord, FeatureRow, MergeBuilder, ParetoRow};
use zencodec_helpers::loop_tools::origin_split::{Split, split_of};
use zencodec_helpers::loop_tools::parse_config::validate_cell_ids;
use zencodec_helpers::loop_tools::topk_eval::{Row, evaluate_topk_verify};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let rest = &args[1..];
    let result = match cmd {
        "merge" => cmd_merge(rest),
        "split" => cmd_split(rest),
        "validate-cells" => cmd_validate(rest),
        "topk-eval" => cmd_topk_eval(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => Err(format!("unknown subcommand {other:?}\n\n{USAGE}")),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE, // gate failed (validate-cells)
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "picker-loop <merge|split|validate-cells|topk-eval> [opts]  (see module docs)";

// --- tiny flag parsing (no clap dep) -----------------------------------------

/// Collect `--flag value` (repeatable) and `--flag value` (single) from args.
struct Flags {
    multi: std::collections::HashMap<String, Vec<String>>,
}
impl Flags {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut multi: std::collections::HashMap<String, Vec<String>> = Default::default();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            let Some(name) = a.strip_prefix("--") else {
                return Err(format!("expected --flag, got {a:?}"));
            };
            let val = args
                .get(i + 1)
                .ok_or_else(|| format!("--{name} needs a value"))?;
            multi.entry(name.to_string()).or_default().push(val.clone());
            i += 2;
        }
        Ok(Self { multi })
    }
    fn one(&self, name: &str) -> Option<&str> {
        self.multi
            .get(name)
            .and_then(|v| v.last())
            .map(String::as_str)
    }
    fn req(&self, name: &str) -> Result<&str, String> {
        self.one(name)
            .ok_or_else(|| format!("--{name} is required"))
    }
    fn all(&self, name: &str) -> Vec<String> {
        self.multi.get(name).cloned().unwrap_or_default()
    }
}

// --- TSV helpers -------------------------------------------------------------

fn read_tsv(path: &str) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .from_path(path)
        .map_err(|e| format!("open {path}: {e}"))?;
    let header: Vec<String> = rdr
        .headers()
        .map_err(|e| format!("{path} header: {e}"))?
        .iter()
        .map(str::to_string)
        .collect();
    let mut rows = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| format!("{path} row: {e}"))?;
        rows.push(rec.iter().map(str::to_string).collect());
    }
    Ok((header, rows))
}

fn col_index(header: &[String], name: &str) -> Option<usize> {
    header.iter().position(|c| c == name)
}

// --- subcommands -------------------------------------------------------------

fn cmd_merge(args: &[String]) -> Result<bool, String> {
    let f = Flags::parse(args)?;
    let omnis = f.all("omni");
    if omnis.is_empty() {
        return Err("at least one --omni is required".into());
    }
    let features_path = f.req("features")?;
    let metric_col = f.one("metric-col").unwrap_or("score_ssim2_gpu");
    let out_pareto = f.req("out-pareto")?;

    let mut builder = MergeBuilder::new();
    for omni in &omnis {
        let (header, rows) = read_tsv(omni)?;
        let header_set: BTreeSet<String> = header.iter().cloned().collect();
        // Build the shared `column -> cell index` once per sidecar; each row's
        // ColumnRecord borrows it + the row's cells.
        let index: std::collections::BTreeMap<&str, usize> = header
            .iter()
            .enumerate()
            .map(|(i, c)| (c.as_str(), i))
            .collect();
        let records = rows.iter().map(|row| ColumnRecord {
            cells: row,
            index: &index,
        });
        let n = builder
            .add_omni_rows(&header_set, metric_col, records)
            .map_err(|e| format!("{omni}: {e:?}"))?;
        eprintln!("omni {omni}: +{n} rows");
    }

    let features = read_feature_rows(features_path)?;
    let (rows, dropped) = builder.join_features(&features);
    if dropped > 0 {
        eprintln!("WARNING: {dropped} sweep variants had no feature row (dropped)");
    }
    write_pareto(out_pareto, &rows)?;

    let n_cfg = rows
        .iter()
        .map(|r| &r.config_name)
        .collect::<BTreeSet<_>>()
        .len();
    let sizes: BTreeSet<&str> = rows.iter().map(|r| r.size_class.label()).collect();
    let (mn, mx) = rows
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), r| {
            (a.min(r.metric), b.max(r.metric))
        });
    println!(
        "pareto: {} rows, {} configs, sizes={:?}, metric range [{:.1},{:.1}] -> {}",
        rows.len(),
        n_cfg,
        sizes,
        if mn.is_finite() { mn } else { 0.0 },
        if mx.is_finite() { mx } else { 0.0 },
        out_pareto
    );
    Ok(true)
}

fn read_feature_rows(path: &str) -> Result<Vec<FeatureRow>, String> {
    let (header, rows) = read_tsv(path)?;
    let vn = col_index(&header, "variant_name")
        .ok_or_else(|| format!("{path} lacks 'variant_name' column"))?;
    let wi = col_index(&header, "width");
    let hi = col_index(&header, "height");
    let feat_cols = feature_columns(&header);
    let feat_idx: Vec<(String, usize)> = feat_cols
        .iter()
        .filter_map(|c| col_index(&header, c).map(|i| (c.clone(), i)))
        .collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for row in &rows {
        let name = row[vn].clone();
        if !seen.insert(name.clone()) {
            continue; // drop_duplicates("variant_name")
        }
        let width = wi.and_then(|i| row[i].parse().ok()).unwrap_or(0u32);
        let height = hi.and_then(|i| row[i].parse().ok()).unwrap_or(0u32);
        let feats = feat_idx
            .iter()
            .map(|(c, i)| (c.clone(), row[*i].parse::<f64>().unwrap_or(0.0)))
            .collect();
        out.push(FeatureRow {
            variant_name: name,
            width,
            height,
            feats,
        });
    }
    Ok(out)
}

fn write_pareto(path: &str, rows: &[ParetoRow]) -> Result<(), String> {
    if let Some(parent) = PathBuf::from(path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut w = csv::WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(path)
        .map_err(|e| format!("create {path}: {e}"))?;
    w.write_record([
        "image_path",
        "size_class",
        "width",
        "height",
        "config_id",
        "config_name",
        "q",
        "bytes",
        "metric",
        "encode_ms",
        "total_ms",
        "effective_max_metric",
    ])
    .map_err(|e| e.to_string())?;
    for r in rows {
        w.write_record([
            r.image_path.as_str(),
            r.size_class.label(),
            &r.width.to_string(),
            &r.height.to_string(),
            &r.config_id.to_string(),
            r.config_name.as_str(),
            &r.q.to_string(),
            &r.bytes.to_string(),
            &r.metric.to_string(),
            &r.encode_ms.to_string(),
            &r.total_ms.to_string(),
            &r.effective_max_metric.to_string(),
        ])
        .map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_split(args: &[String]) -> Result<bool, String> {
    let f = Flags::parse(args)?;
    let pareto = f.req("pareto")?;
    let (header, rows) = read_tsv(pareto)?;
    let ip =
        col_index(&header, "image_path").ok_or_else(|| format!("{pareto} lacks 'image_path'"))?;
    let mut buckets: std::collections::HashMap<Split, Vec<Vec<String>>> = Default::default();
    let mut unsplittable = 0usize;
    for row in &rows {
        match split_of(&row[ip]) {
            Some(s) => buckets.entry(s).or_default().push(row.clone()),
            None => unsplittable += 1,
        }
    }
    let count = |s: Split| buckets.get(&s).map(Vec::len).unwrap_or(0);
    println!(
        "split: train={} val={} test={} unsplittable={}",
        count(Split::Train),
        count(Split::Val),
        count(Split::Test),
        unsplittable
    );
    for (flag, s) in [
        ("out-train", Split::Train),
        ("out-val", Split::Val),
        ("out-test", Split::Test),
    ] {
        if let Some(out) = f.one(flag) {
            write_rows(
                out,
                &header,
                buckets.get(&s).map(Vec::as_slice).unwrap_or(&[]),
            )?;
        }
    }
    Ok(true)
}

fn write_rows(path: &str, header: &[String], rows: &[Vec<String>]) -> Result<(), String> {
    if let Some(parent) = PathBuf::from(path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut w = csv::WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(path)
        .map_err(|e| format!("create {path}: {e}"))?;
    w.write_record(header).map_err(|e| e.to_string())?;
    for r in rows {
        w.write_record(r).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_validate(args: &[String]) -> Result<bool, String> {
    let f = Flags::parse(args)?;
    let omnis = f.all("omni");
    if omnis.is_empty() {
        return Err("at least one --omni is required".into());
    }
    let grammar = f.one("grammar").unwrap_or("jxl-lossy-dense");
    // Collect distinct cell-ids from the knob_tuple_json column.
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for omni in &omnis {
        let (header, rows) = read_tsv(omni)?;
        let ki = col_index(&header, "knob_tuple_json")
            .ok_or_else(|| format!("{omni} lacks 'knob_tuple_json'"))?;
        for row in &rows {
            let cell: serde_json::Value = serde_json::from_str(&row[ki])
                .map_err(|e| format!("{omni}: bad knob_tuple_json: {e}"))?;
            if let Some(c) = cell.get("cell").and_then(|v| v.as_str()) {
                ids.insert(c.to_string());
            }
        }
    }
    let report = match grammar {
        "jxl-lossy-dense" => validate_cell_ids(&JxlLossyDense, ids.iter().cloned()),
        other => return Err(format!("unknown grammar {other:?} (have: jxl-lossy-dense)")),
    };
    println!("unique cell-ids: {}", report.n_cells);
    println!(
        "parse failures: {}  {:?}",
        report.parse_failures.len(),
        &report.parse_failures.iter().take(5).collect::<Vec<_>>()
    );
    println!(
        "max-dev violations: {}  {:?}",
        report.max_deviation_violations.len(),
        &report
            .max_deviation_violations
            .iter()
            .take(5)
            .collect::<Vec<_>>()
    );
    println!(
        "roundtrip failures: {}  {:?}",
        report.roundtrip_failures.len(),
        &report.roundtrip_failures.iter().take(5).collect::<Vec<_>>()
    );
    let ok = report.ok();
    println!("RESULT: {}", if ok { "PASS" } else { "FAIL" });
    Ok(ok)
}

fn cmd_topk_eval(args: &[String]) -> Result<bool, String> {
    let f = Flags::parse(args)?;
    let pareto = f.req("pareto")?;
    let target: f32 = f
        .req("target")?
        .parse()
        .map_err(|_| "bad --target".to_string())?;
    let ks: Vec<usize> = f
        .one("ks")
        .unwrap_or("1,2,3,5")
        .split(',')
        .map(|s| s.trim().parse().map_err(|_| format!("bad K {s:?}")))
        .collect::<Result<_, _>>()?;
    let higher = f.one("metric-dir").unwrap_or("higher") != "lower";

    let (header, rows) = read_tsv(pareto)?;
    let need = |c: &str| col_index(&header, c).ok_or_else(|| format!("{pareto} lacks {c:?}"));
    let (ip, sc, by, mt) = (
        need("image_path")?,
        need("size_class")?,
        need("bytes")?,
        need("metric")?,
    );

    // Group rows into a cell space per (image_path, size_class). Each group's
    // cells are its rows; reach = metric meets target; bytes -> log-bytes.
    let mut groups: std::collections::BTreeMap<(String, String), Vec<(f64, f64)>> =
        Default::default();
    for row in &rows {
        let key = (row[ip].clone(), row[sc].clone());
        let bytes: f64 = row[by].parse().map_err(|_| "bad bytes".to_string())?;
        let metric: f64 = row[mt].parse().map_err(|_| "bad metric".to_string())?;
        groups
            .entry(key)
            .or_default()
            .push((bytes.max(1.0), metric));
    }

    // For a real picker the "predicted" cost would come from the model; here we
    // have only actuals, so predicted == actual (this measures the *oracle
    // structure* of the corpus — the K=n_reach→0% / K=1 floor — not a specific
    // model's gap; the trainer supplies model predictions for the model-gap).
    let mut pred_store: Vec<Vec<f32>> = Vec::new();
    let mut actual_store: Vec<Vec<f32>> = Vec::new();
    let mut reach_store: Vec<Vec<bool>> = Vec::new();
    for cells in groups.values() {
        let logs: Vec<f32> = cells.iter().map(|(b, _)| (b.ln()) as f32).collect();
        let reach: Vec<bool> = cells
            .iter()
            .map(|(_, m)| {
                if higher {
                    *m >= target as f64
                } else {
                    *m <= target as f64
                }
            })
            .collect();
        actual_store.push(logs.clone());
        pred_store.push(logs);
        reach_store.push(reach);
    }
    let max_cells = actual_store.iter().map(Vec::len).max().unwrap_or(0);
    let all_mask = vec![true; max_cells];
    let eval_rows: Vec<Row> = (0..actual_store.len())
        .map(|i| Row {
            pred_log_bytes: &pred_store[i],
            actual_log_bytes: &actual_store[i],
            reach: &reach_store[i],
        })
        .collect();
    let stats = evaluate_topk_verify(&eval_rows, &all_mask, &ks);

    println!(
        "topk-eval target={target} dir={} groups={}",
        if higher { "higher" } else { "lower" },
        stats.first().map(|s| s.n_rows).unwrap_or(0)
    );
    println!("K  mean%   p50%   p90%   p99%   max%   hit%  mean_verified");
    for s in &stats {
        println!(
            "{:<2} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>5.1} {:>6.2}",
            s.k,
            s.mean_pct,
            s.p50_pct,
            s.p90_pct,
            s.p99_pct,
            s.max_pct,
            100.0 * s.hit_rate,
            s.mean_verified
        );
    }
    Ok(true)
}
