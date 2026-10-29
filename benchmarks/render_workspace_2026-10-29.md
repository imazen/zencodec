# Collection and render workspace admission

Base: `0010885919c6ac782a2937cacdad0ef8e5a0e8f4`.
Tests ran on zen-arm-big in a source export outside workspace overrides, stable
Rust 1.96, run-heavy 3G/jobs2, serialized, debug information/incremental disabled.
Renderer uses its committed dependencies.lock; core uses the checkout Cargo.lock.
No dependency versions were changed.

Three new failing-first controls reproduced effects before workspace admission:

- `required_workspace_refuses_before_collection_factory`: factory executed.
- `late_legacy_workspace_refuses_before_discovery_without_losing_owner`:
  discovery succeeded despite unknown required workspace.
- `required_workspace_refuses_before_projection_for_bound_and_legacy_policies`:
  projection reached the zero-capacity storage pool instead of returning the
  named workspace refusal. The fixed test covers bound, legacy, and their
  separate compatibility entry point.

Fix: check both applicable requirements against unknown workspace before native
collection effects and before render color planning. Existing collection owners
survive request-local refusal. Opening refuses before the factory. Input owners
constructed by callers remain outside this boundary. Unconstrained paths retain
ordinary success; final policy resolution still enforces other requirements.
No API signature changed and no native workspace bound was invented.

Validation commands are retained as `just render-workspace-check`:
renderer tests (8 integration controls and 1 doctest), strict all-target renderer
Clippy, no-default-features renderer check, 680 core unit tests, and strict core
library Clippy all pass. All previous assertions remain unchanged.

The first post-fix aggregate stopped after successful renderer tests/Clippy
because the exported root omitted its ignored Cargo.lock. Copying the checkout
lock repaired orchestration; the remaining core and no-default gates passed.
This was not a codec or assertion failure.

Run-heavy receipts:

- Before collection: rc101, 17s, peak-RSS 0.36GiB, min-avail 14359MiB, load 0.44.
- Before render: rc101, 0s, peak-RSS 0.03GiB, min-avail 14681MiB, load 0.45.
- Renderer pass / missing root lock: rc101, 15s, peak-RSS 0.34GiB,
  min-avail 14313MiB, load 0.53.
- Remaining gates: rc0, 68s, peak-RSS 0.48GiB, min-avail 14281MiB, load 1.24.

These are validation resource measurements, not codec memory bounds or speed
comparisons. No new native TIFF/JXL composition, WASM runtime or cross-platform
qualification is claimed. Logs: `~/tmp/render-workspace-20261029/{before,
before-render,after,core}.log` on zen-arm-big; copies retained on r7900x.

The two affected compiler catalogs were regenerated; exported signatures did
not change. All 26 site source tests pass in a fresh source export. An initial
check against the old generated site failed its exact catalog comparison; a
subsequent in-place rebuild correctly rejected stale Rustdoc revision identity.
The isolated site build preserves those guards and does not claim a new browser
or full HTML Rustdoc run. Catalog receipt: rc0, 22s, peak-RSS 0.35GiB,
min-avail 14241MiB, load 1.20. All remote validation processes exited; the owned
Cargo target was deleted, with logs retained and no executables archived.
