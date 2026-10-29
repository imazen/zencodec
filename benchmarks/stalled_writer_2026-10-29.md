# Stalled async writer cancellation — 2026-10-29

Base `8d978e47`, source changes in this commit. No production API or implementation
changed. `just tokio-guide-check` runs the driver and quota suites plus strict
Clippy. The isolated ARM64 export uses the identical core/I/O/coroutine runtime
sources from `9bbd89f5`; the source diff against the base is empty for those
runtime directories. Root lock SHA256:
`47c1da49b6c7eb8b43ee81bcc76cbfc165c353c56a9a9751500b1b65c45c198e`.

43 driver tests and 3 quota tests pass, none ignored, plus strict Clippy. The new
three-case control holds a writer at prefix, tail or flush with Pending and no
self-wake. Tokio request abort and join must signal local Pulse and release the
encoder owner. Existing publication-finalization assertions remain unchanged.
Observed retained bytes are respectively empty, `prefix`, and `prefixtail`;
finish calls are 0/1/1 and flush polls 0/0/1. The fixture forbids shutdown of the
caller-owned writer. This verifies cleanup and effect ordering, not codec fidelity,
network disconnect handling, forced native interruption or cancellation latency.

Run-heavy: rc0, 69s, peak RSS 0.40 GiB, min available 14240 MiB, peak load 1.27;
3 GiB cap, two jobs, timeout900. These measure compilation and validation, not
encoder memory or throughput. Full stdout/stderr retained in
`~/tmp/native-pulse-diagnostics-20261029/stalled.log`; Cargo/rustc/rustdoc exits
verified. No executable archived.
