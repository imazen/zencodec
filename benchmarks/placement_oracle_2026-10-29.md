# Placement arithmetic oracle

Production base: `7972cea7241a3b359e78be173bd3566b7823d832` (production source
unchanged by subsequent collection test maintenance). No production changes.

`tests/placement_oracle.rs` enumerates every sample of selected small layers and
compares the returned overlap against independent i128 point membership in the
presentation canvas. It covers positive/negative placement on each edge,
fully off-canvas results, oversized layers and crops with nonzero local origins.
It separately checks i64-min off-canvas placement, i64-max whole-layer overflow,
and the largest valid positive origin for a u32-max layer with a one-pixel crop.
The small-case set is not exhaustive over all dimensions or native formats.

Both tests and strict Clippy pass via `just placement-oracle-check`, in an
isolated export on zen-arm-big, stable Rust 1.96, existing root Cargo.lock,
run-heavy 3G/jobs2, debug info/incremental disabled. Receipt:
`rc=0 77s | peak-RSS 0.40GiB | min-avail 14236MiB | peak-load 1.40`.
Full log: `~/tmp/render-workspace-20261029/placement.log` on zen-arm-big and
r7900x. These figures measure validation resources, not codec throughput or
workspace bounds. No new native composition, reference-reuse or video
qualification follows. Native evidence and remaining gaps are indexed in
[the rendering contract](../docs/render-contract.md#canvas-and-raw-layer-boundaries).
