# zencodec-helpers

Shared infrastructure that makes high-quality, *consistent* per-codec picker
**loops** and the picker **runtime** easy across the zen\* image codecs
(zenjpeg / zenwebp / zenjxl / zenavif / …). Two independent, feature-gated
halves.

## 1. Runtime: top-K-verify picker (`topk-verify` feature, default)

The key gap an audit found: a per-codec picker's *top-K* path is unreachable at
runtime. Every wired picker (zenwebp, zenjpeg) takes a single masked argmin and
uses the picked config blindly — leaving the residual oracle gap (~2.4% on the
JXL-lossy picker) on the floor. The proven fix — **narrow by content, finalize
by an RD check** — reached ~0.48% (≤1% MET) at K=3 in offline evaluation but had
no runtime home.

`topk_verify::pick_top_k_verify` is that home. Generic over a codec's
`encode(cell) -> bytes` and `score(cell) -> quality` closures, it:

1. ranks the picker's predicted-cheapest cells (masked top-K),
2. **actually encodes + scores** the K cheapest, and
3. returns the **fewest-bytes** cell that **meets the quality target**
   (`VerifyOutcome::MetTarget`), or the best-quality cell if none do
   (`VerifyOutcome::BestEffort`).

`pick_top_3_verify` is the proven K=3 specialization. `K` is a const generic, so
the helper is allocation-free; ranking goes through
`zenpredict::argmin::argmin_masked_top_k_in_range`.

```rust,ignore
use zencodec_helpers::topk_verify::{pick_top_3_verify, VerifyConfig, VerifyOutcome};
use zenpredict::AllowedMask;

let mask = AllowedMask::new(&allowed);                       // codec constraints
let cfg  = VerifyConfig::log_bytes(predictions.len(), 80.0); // log-bytes picker, target 80
let outcome = pick_top_3_verify(
    &predictions, &mask, &cfg,
    |cell| my_codec::encode(cell).len() as u64, // actually encode candidate `cell`
    |cell| my_codec::score(cell),               // score that encode's quality
)?;
if let VerifyOutcome::MetTarget(best) = outcome {
    // re-emit / cache `best.cell`'s bitstream — fewest bytes meeting target.
}
```

`VerifyConfig` carries the cost sub-range, the `ScoreTransform` (`Exp` for
log-bytes pickers), optional per-cell byte offsets, the target, and a
`QualityDirection` (higher-is-better for zensim/SSIM2, lower-is-better for
butteraugli distance).

## 2. Offline: training-loop glue (`loop-tools` feature)

A Rust port of the per-codec picker-training-loop steps that were ad-hoc Python
in `zenmetrics/scripts/picker/`, so every codec's loop runs the **identical**
shape. Library (`zencodec_helpers::loop_tools`) + a `picker-loop` CLI.

| step | module | replaces |
|------|--------|----------|
| omni-merge across box sidecars | `omni_merge` | `omni_to_pareto.py` |
| `variant_name` + `size_class` | `variant` | `variant_of` / `size_class` |
| `feat_<name>` select + `log1p` | `feature_prep` | `picker_config_common.py` |
| `parse_config_name` robustness | `parse_config` + `grammars` | per-codec config parsers |
| train/val/test origin split | `origin_split` | `origin_split.py` |
| top-K-verify oracle-gap | `topk_eval` | `evaluate_topk_verify` |

```text
picker-loop merge  --omni A.tsv [--omni B.tsv …] --features F.tsv \
                   --metric-col score_ssim2_gpu --out-pareto P.tsv
picker-loop split  --pareto P.tsv [--out-train T.tsv --out-val V.tsv --out-test E.tsv]
picker-loop validate-cells --omni A.tsv --grammar jxl-lossy-dense
picker-loop topk-eval --pareto P.tsv --target 80 --ks 1,2,3,5 [--metric-dir higher|lower]
```

`origin_split` and `topk_eval` are faithful, unit-tested ports — `origin_split`
matches the canonical Python self-test table byte-for-byte; `topk_eval`'s
percentiles use numpy's linear interpolation so its per-K report lines up with
the trainer's. They are the **offline twins** of the runtime helper: same
ranking, same finalize — so the K the loop validates is the K the runtime uses.

## Dependency note — read before depending on `topk-verify`

The runtime half consumes `zenpredict`'s masked top-K argmin
(`argmin_masked_top_k::<K>`), which lives behind zenpredict's **`advanced`**
feature — present only in the in-tree **0.2.0** (crates.io still ships 0.1.0 /
v2-only), and explicitly "NOT YET STABILIZED" there (a concurrent zenpredict PR
is stabilizing exactly this API; the stabilized form lifts the core top-K out of
`advanced`, so enabling `advanced` is a safe superset against both states). This
crate is designed against the masked-top-K shape (K output indices, ascending
best-first).

Because `zenpredict 0.2` is unpublished, **this crate is excluded from the
zencodec workspace** (so zencodec's own CI never tries to resolve it) and pins
the sibling via a build-local `[patch.crates-io]` in its own `Cargo.toml`.
Build/test it standalone:

```bash
cd zencodec-helpers
cargo test --all-features    # resolves ../../zenanalyze/zenpredict via the patch
```

Drop the patch and bump the dep to the real `>=0.2` once zenpredict 0.2 + 
zenpredict-bake publish, then promote this crate into the workspace `members`.

## License

Apache-2.0 OR MIT.
