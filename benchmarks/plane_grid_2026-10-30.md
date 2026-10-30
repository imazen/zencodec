# Exact grid and cropped native-view ablation — 2026-10-30

Private candidate builds on `8b407751`; native/pixel graph and import-only AVIF
patch are unchanged from [the storage experiment](plane_model_2026-10-30.md).
`perf/plane-ablation/src/grid.rs` defines exact signed origins, positive spacing,
explicit unknown registration and checked rebasing. `Plane::window` retains
underlying storage strides and active sample identity.

`just plane-ablation-check` and `just plane-ablation-native-check` pass from
exports outside workspace path overrides: 12 standalone model tests, 13 focused
native-harness tests, both strict all-target Clippy gates and standalone WASM.
The native matrix still covers 8/10/12-bit 4:2:0 and 4:4:4. It now verifies every
sample and exact coordinate in a borrowed view following an odd luma crop (1,3),
after decoder drop. Expected centers use a separate direct source-coordinate
formula. The existing complete-row pointer and code-identity controls remain.

The graph/lock is unchanged. Source compilation paths for the updated model and
native harness appear in the full log. No tests were skipped or assertions
relaxed; the 52 unrelated native tests remain filtered by this focused recipe.

`run-heavy: done rc=0 3s | peak-RSS 0.34GiB | min-avail 13638MiB | peak-load 0.61`.
Full command/log: `~/tmp/zencodec-grid-model-20261030/`; updated source exports:
`~/tmp/zencodec-plane-model-20261030/` and
`~/tmp/zencodec-plane-native-20261030/`. Prior source revisions are committed;
prior logs remain separate. No executable archive was created.

This is representation and registration evidence, not rendering parity,
independent codec correctness, performance or whole-process memory measurement.
It does not choose a reconstruction kernel or its halo. Resize/affine grids,
TIFF/JXL scalar/grid adoption and unified ownership/presentation remain open.
