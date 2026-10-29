# Shared registry snapshot allocation counts

Compared `88531042` with `d6a47664` using the same updated
`perf/api-usability-validation/src/main.rs` probe and its recorded dependency lock.
Each revision was exported outside the workspace with an independent Cargo target.
Command: `cargo run --manifest-path perf/api-usability-validation/Cargo.toml --locked -- --registry-only`.
Dhat counts allocation calls and requested bytes; this is not a throughput,
peak-live-memory or native codec workspace measurement.

[Raw counters](registry_snapshot_2026-10-29.tsv) cover 0, 1, 4 and 16 pairs of
reference decoder/encoder registrations. Empty-registry cloning stays allocation-free.
Populated clones drop from two allocations (32/128/512 bytes) to zero.
Controlled startup drops from four allocations to two, with 384 requested bytes
at every registration count after the change. Previously it requested
480/576/960 bytes for 1/4/16 pairs; empty startup was two allocations, 448 bytes.
This is session creation before format detection, not native decoder execution.

Registry construction adds one allocation and 88 requested bytes in every case,
including the empty registry. Mutation after cloning now copies shared lists on
write; it does not clone native configurations. Per-set limits, policy and pulse
remain independent. No public signatures or codec/zenpixels changes are required.

`just registry-snapshot-check` passed 17 codec-set tests, two registry tests and
strict core all-feature/all-target Clippy. Existing config-clone counts remain
unchanged, and the new control verifies that extending a snapshot does not mutate
the original registration lists. Serialized wrapper for both allocation probes
and these checks: `rc=0 35s | peak-RSS 0.44GiB | min-avail 13610MiB`.
Full logs are retained under `~/tmp/zencodec-registry-snapshot-20261029/`.

Native JPEG: `just jpeg-decode-admission-check` passed all seven admission tests
and strict native-harness Clippy on the same candidate. Wrapper:
`rc=0 28s | peak-RSS 0.71GiB | min-avail 13548MiB`. This preserves native
metadata/extras, exact pixels, typed causes and controlled output admission;
it is not a native throughput measurement.
