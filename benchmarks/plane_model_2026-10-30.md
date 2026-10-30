# Byte-backed component ablation — 2026-10-30

Private source: `perf/plane-ablation`; native witness:
`perf/av1-policy-validation/src/component_storage_native.rs`.
Core baseline `ab8a3db3`; pixels `f23ac753`; AVIF `94116d26` with the committed
import-only patch; rav1d `b19aec65`. No production storage API is added.

Seven model controls pass for typed raw borrowing, stepped P010 chroma, exact
float NaN/signed-zero and signed-wide values, mixed-depth planes, unaligned
storage, explicit endianness, and empty/overflow geometry. Strict Clippy and
WASM also pass. Initial standalone receipt:
`run-heavy: done rc=0 1s | peak-RSS 0.15GiB | min-avail 13791MiB | peak-load 0.30`.

The eight focused native-harness tests pass (seven shared model controls plus
one native matrix). The native matrix covers 8/10/12-bit 4:2:0 and 4:4:4 AV1.
Every row retains the native address and exact words; every active code matches
native extraction after decoder drop. This proves no repacking at the tested
borrow boundary, not process-wide allocation behavior or codec correctness.
`run-heavy: done rc=0 3s | peak-RSS 0.40GiB | min-avail 13698MiB | peak-load 0.25`.

After separating the reusable model module from its standalone crate wrapper,
the final standalone tests/strict Clippy/WASM and full native-harness all-target
strict Clippy pass:
`run-heavy: done rc=0 7s | peak-RSS 0.87GiB | min-avail 12854MiB | peak-load 0.30`.
The native runtime matrix also used the final shared module.

The first native compilation failed because its encoder submodule still lacked
the canonical SampleEncoding import. The patch adds that import; the failed log
is retained, with no skipped assertions. Native graph metadata proves one core,
I/O, media and pixel family. The lock changes only zenavif/zenavif-parse source
IDs from the pinned git tree to its local patched export; package versions remain
unchanged. This does not establish portable landing of that sibling patch.

Exact exports, graphs and complete logs are retained under
`~/tmp/zencodec-plane-model-20261030/` and
`~/tmp/zencodec-plane-native-20261030/`. These wrapper receipts describe gate
resources, not benchmark results. No executable archives were created.

Not qualified: native TIFF/JXL scalar carriage, unified image/frame ownership,
component grids/odd-origin crop phase, foreign-endian generic float loading,
mutable mapping, performance, compile-time deltas or production API migration.
