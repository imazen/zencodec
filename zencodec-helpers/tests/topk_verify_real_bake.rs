//! End-to-end runtime proof: a REAL ZNPR bake → `Predictor::predict` → the
//! generic [`pick_top_k_verify`] helper → min-bytes-meeting-target pick.
//!
//! The synthetic unit tests in `topk_verify.rs` feed hand-written prediction
//! slices; this test closes the loop the way a codec actually would — it mints
//! a model with `zenpredict-bake`, runs a forward pass through the DEFAULT-API
//! `Predictor::predict` to get the per-cell predicted log-bytes, and drives the
//! verify helper over that real output. This exercises the self-contained
//! `select_top_k` selection on an actual `Predictor` slice — no gated zenpredict
//! API involved.
#![cfg(feature = "topk-verify")]

use zencodec_helpers::topk_verify::{
    QualityDirection, VerifyConfig, VerifyOutcome, pick_top_3_verify, pick_top_k_verify,
};
use zenpredict::{Activation, AllowedMask, Model, Predictor, ScoreTransform, WeightDtype};
use zenpredict_bake::{BakeLayer, BakeRequest};

/// Bake a 1-input → N-cell linear model whose output cell `i` is
/// `weights[i] * x + biases[i]`. With `x = 1.0` the outputs are exactly
/// `weights[i] + biases[i]`, letting the test set each cell's predicted
/// log-bytes directly. 16-aligned by virtue of being a fresh `Vec`.
fn bake_per_cell_costs(pred_log_bytes: &[f32]) -> Vec<u8> {
    let n = pred_log_bytes.len();
    let scaler_mean = [0.0f32];
    let scaler_scale = [1.0f32];
    // 1 input → n outputs: weight row is the per-cell coefficient on x; bias 0.
    let weights: Vec<f32> = pred_log_bytes.to_vec();
    let biases = vec![0.0f32; n];
    let layers = [BakeLayer {
        in_dim: 1,
        out_dim: n,
        activation: Activation::Identity,
        dtype: WeightDtype::F32,
        weights: &weights,
        biases: &biases,
    }];
    BakeRequest::builder(0, 0, &scaler_mean, &scaler_scale, &layers)
        .bake()
        .expect("bake a tiny identity model")
}

#[test]
fn real_bake_drives_top_k_verify_min_bytes_pick() {
    // Picker predicts log-bytes for 5 cells. Predicted-cheapest order:
    // cells 0,1,2,3,4 (ascending). The model emits exactly these on x=1.0.
    let pred_log_bytes = [1.0f32, 2.0, 3.0, 4.0, 5.0];
    let bytes = bake_per_cell_costs(&pred_log_bytes);
    let model = Model::from_bytes(&bytes).expect("load bake");
    let mut predictor = Predictor::new(&model);
    let out = predictor.predict(&[1.0]).expect("forward pass");
    assert_eq!(out.len(), 5, "one output per cell");

    // Verify with K=3: encode cells {0,1,2}. Quality: only 1,2 meet target 80.
    // Actual bytes: cell 1 = 500, cell 2 = 400 → min-bytes target-meeter = 2.
    let actual_bytes = [100u64, 500, 400, 300, 200];
    let actual_quality = [60.0f32, 85.0, 90.0, 95.0, 99.0];
    let mask_data = [true; 5];
    let mask = AllowedMask::new(&mask_data);
    let cfg = VerifyConfig::log_bytes(out.len(), 80.0);

    let mut encoded_cells = Vec::new();
    let outcome = pick_top_k_verify::<3, _, _>(
        out,
        &mask,
        &cfg,
        |cell| {
            encoded_cells.push(cell);
            actual_bytes[cell]
        },
        |cell| actual_quality[cell],
    )
    .expect("verify runs");

    match outcome {
        VerifyOutcome::MetTarget(m) => {
            assert_eq!(m.cell, 2, "min-bytes target-meeter among the top-3");
            assert_eq!(m.bytes, 400);
            assert_eq!(m.quality, 90.0);
        }
        other => panic!("expected MetTarget(cell 2), got {other:?}"),
    }
    // Only the 3 predicted-cheapest cells were ever encoded — the verify budget
    // held against a real model output.
    assert_eq!(encoded_cells, vec![0, 1, 2]);
}

#[test]
fn real_bake_exp_transform_ranks_in_linear_byte_space() {
    // Two cells whose predicted LOG-bytes are close (3.0 vs 3.1) but whose
    // linear bytes differ; the Exp transform must rank in linear space. With a
    // per-cell byte offset that flips the order, the cheaper *effective* cell
    // wins. This pins that the helper honors ScoreTransform::Exp + offsets via
    // the real argmin path.
    let pred_log_bytes = [3.0f32, 3.1];
    let bytes = bake_per_cell_costs(&pred_log_bytes);
    let model = Model::from_bytes(&bytes).unwrap();
    let mut predictor = Predictor::new(&model);
    let out = predictor.predict(&[1.0]).unwrap();

    // e^3.0 ≈ 20.09, e^3.1 ≈ 22.20. A +5 byte offset on cell 0 only
    // (per_output) makes cell 0 ≈ 25.09 > cell 1 ≈ 22.20 → cell 1 is predicted
    // cheaper. Both meet quality, so the helper should verify cell 1 first and
    // (since cell 1 also has fewer actual bytes) return it.
    let per_output = [5.0f32, 0.0];
    let offsets = zenpredict::ArgminOffsets {
        uniform: 0.0,
        per_output: Some(&per_output),
    };
    let cfg = VerifyConfig {
        cost_range: (0, 2),
        transform: ScoreTransform::Exp,
        offsets: Some(&offsets),
        target_quality: 50.0,
        direction: QualityDirection::HigherIsBetter,
    };
    let actual_bytes = [30u64, 20];
    let actual_quality = [99.0f32, 99.0];
    let mask_data = [true; 2];
    let mask = AllowedMask::new(&mask_data);

    let mut order = Vec::new();
    let outcome = pick_top_k_verify::<2, _, _>(
        out,
        &mask,
        &cfg,
        |c| {
            order.push(c);
            actual_bytes[c]
        },
        |c| actual_quality[c],
    )
    .unwrap();
    // Predicted-cheapest-first encode order must put cell 1 before cell 0.
    assert_eq!(order, vec![1, 0], "Exp+offset ranks cell 1 cheaper");
    match outcome {
        VerifyOutcome::MetTarget(m) => assert_eq!(m.cell, 1),
        other => panic!("expected MetTarget(cell 1), got {other:?}"),
    }
}

#[test]
fn pick_top_3_alias_works_against_real_bake() {
    let pred_log_bytes = [1.0f32, 2.0, 3.0, 4.0];
    let bytes = bake_per_cell_costs(&pred_log_bytes);
    let model = Model::from_bytes(&bytes).unwrap();
    let mut predictor = Predictor::new(&model);
    let out = predictor.predict(&[1.0]).unwrap();

    let actual_bytes = [500u64, 400, 300, 200];
    let actual_quality = [85.0f32, 86.0, 70.0, 60.0];
    let mask_data = [true; 4];
    let mask = AllowedMask::new(&mask_data);
    let cfg = VerifyConfig::log_bytes(out.len(), 80.0);
    let outcome =
        pick_top_3_verify(out, &mask, &cfg, |c| actual_bytes[c], |c| actual_quality[c]).unwrap();
    // top-3 = {0,1,2}; meeters {0,1}; min bytes → cell 1 (400 < 500).
    match outcome {
        VerifyOutcome::MetTarget(m) => assert_eq!(m.cell, 1),
        other => panic!("expected MetTarget(cell 1), got {other:?}"),
    }
}
