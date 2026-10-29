# Standalone harness lockfiles

Root Cargo.lock is ignored. Standalone qualification graphs instead commit
`dependencies.lock`, or named locks under `perf/io-validation/locks`, and copy
those files to Cargo.lock before locked commands. A root dependency change must
update these graphs too; root workspace tests do not check their freshness.

The 2026-10-29 freestanding fix added a direct libm dependency but initially
omitted these copies. Documentation CI 40938643876 failed before execution in
the IO examples, renderer and native spatial harness. The repair retains all
original tests and `--locked` gates.

All 13 current lockfiles pass `cargo metadata --locked --offline` in an export
outside workspace overrides after copying each to its corresponding Cargo.lock
and fetching the IO harness's pinned text sources with its existing setup tool.
The validation does not modify the copied locks. Wrapper: rc0, 1s, peak RSS
0.05 GiB. This checks graph resolution, not codec execution.

Changes are the direct libm edge (and libm 0.2.16 where absent), plus two explicit
updates:

- The IO root workspace snapshot still selected registry pixels 0.2.16. It now
  uses the existing root manifest's f23ac753 pixel pair. That converter requires
  lz4_flex 0.14.0 for its optional ICC database, replacing 0.13.1.
- The spatial harness advances its encoder/macros/SIMD group together to the
  qualified JXL float repair. Its native execution receipt is separate.

The historical `perf/streaming-regress` graph intentionally vendors zencodec
0.1.27. It does not consume today's root package and remains unchanged.
