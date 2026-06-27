# zencodec-helpers

A generic runtime **top-K-verify** picker helper for the zen\* image codecs
(zenjpeg / zenwebp / zenjxl / zenavif / …).

## The gap this fills

An audit found that a per-codec picker's *top-K* path is unreachable at runtime.
Every wired picker (zenwebp `encoder/picker/runtime.rs`, zenjpeg
`encode/picker.rs`) takes a single masked argmin and uses the picked config
blindly — leaving the residual oracle gap (~2.4% on the JXL-lossy picker) on the
floor. The proven fix — **narrow by content, finalize by an RD check** — reached
~0.48% (≤1% MET) at K=3 in offline evaluation but had no runtime home.

`topk_verify::pick_top_k_verify` is that home. Generic over a codec's
`encode(cell) -> bytes` and `score(cell) -> quality` closures, it:

1. ranks the picker's predicted-cheapest cells (masked top-K),
2. **actually encodes + scores** the K cheapest, and
3. returns the **fewest-bytes** cell that **meets the quality target**
   (`VerifyOutcome::MetTarget`), or the best-quality cell if none do
   (`VerifyOutcome::BestEffort`).

`pick_top_3_verify` is the proven K=3 specialization. `K` is a const generic, so
the helper is allocation-free; ranking goes through this crate's own
`select_top_k` (a masked top-K selection over the picker's output slice — same
NaN / tie-break / mask-length contract as zenpredict's `argmin_masked`), so the
crate needs only zenpredict's **default** API.

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

`#![forbid(unsafe_code)]`; `no_std + alloc` on the runtime path (`std` opt-in).
The optional `parallel` feature encodes + scores the K candidates concurrently.

## Dependency note — zenpredict DEFAULT API only

This crate uses zenpredict's **default** API only — **no gated feature**
(`topk` / `advanced`). It calls `Predictor::predict` / `predict_transformed`
(the per-cell predicted-cost forward pass) and reuses the default decision-math
primitives `AllowedMask` / `ScoreTransform` / `ArgminOffsets`. The masked top-K
*selection* lives **here** (`select_top_k`), over that output slice, so nothing
needs to be added to zenpredict and the proven ≤1% path is entirely in the
consumer.

```toml
zenpredict = { version = "0.2.0", default-features = false, features = ["std"] }
```

zenpredict **0.2** is unpublished today (crates.io still ships 0.1.0 / v2-only),
so **this crate is excluded from the zencodec workspace** (zencodec's own CI
never tries to resolve it — verified `cargo check` builds only zencodec) and
pins the sibling `../../zenanalyze/zenpredict` (at `main`) via a build-local
`[patch.crates-io]` in its own `Cargo.toml`. Build/test it standalone:

```bash
cd zencodec-helpers
cargo test --all-features    # resolves ../../zenanalyze/zenpredict via the patch
```

Drop the patch and bump the dep to the real `>=0.2` once zenpredict 0.2 +
zenpredict-bake publish — the dep stays default-features-only either way.

## License

Apache-2.0 OR MIT.
