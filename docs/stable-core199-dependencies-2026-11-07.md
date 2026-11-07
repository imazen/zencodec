# Frozen core PR 199 stable dependencies

Base: `88fc5871`. Require moxcms 0.9.2 for the existing test oracle; retain
paired pixels b6b12bbe, garb 58ce9cad and the existing linear source. No
implementation, fixture, threshold or public signature changes.

`just metadata-check` passes: 1,464 workspace/all-feature tests and doctests
pass, with 39 existing ignored doctests unchanged. Formatting, strict
workspace/all-target/all-feature Clippy, and the existing host-target
zenmeta no-default-feature matrix pass. This is not a freestanding-target
qualification. The full check took 16 seconds, run-heavy peak RSS 0.31 GiB,
minimum available memory 16,356 MiB, peak load 2.27.

The default library/integration gate separately passed 1,323 tests with no
ignores. The exact all-feature seed is
`perf/stable_core199_2026-11-07.lock`. This candidate remains local for the
core owner's integration; the original PR head and main are unchanged.
