# Fused native-return cancellation

Base: `68d3ccd7ea571b58662489ee41dc8fc6584d00a7`.
Failing-first test commit: `d5a14736`.

A native step or resume could signal cancellation and return success. Fused
accepted that success without checking Pulse again. Three regression tests fail
on the base: raw completion, controlled completion and controlled resume.

The fix polls after successful native returns, before accepting state changes.
Native errors and retained sink/interpretation refusals retain their precedence.
The wrapper stays terminal after cancellation, even when the next call supplies
None; completion details remain unavailable. Cached successful Done is unchanged.
Already-performed sink effects cannot be rolled back, and native work is not
preempted by this boundary.

The fixtures exercise both Cancelled and TimedOut using a one-shot signal, plus
simultaneous native-error controls. Two initial fixture compilation errors
(StopOnly ownership/lifetime) are retained separately from the valid failing-first
run; neither is counted as behavioral evidence.

ARM64 Rust 1.96.0, serialized run-heavy 3G/jobs2 with a 1800-second timeout:

- Focused regression: 3 passed.
- Workspace all-features/all-targets and doctests: 1433 passed, 0 failed,
  39 existing ignored tests. The focused three are repeated within this total.
- Workspace all-features/all-targets strict Clippy: passed.
- Wrapper: rc0, 187s, peak RSS 0.47 GiB, minimum available 14194 MiB,
  peak load 1.83. This measures validation resources, not codec workspace.

Commands: `just fused-cancellation-check`, then
`cargo test --workspace --all-features --all-targets --locked`,
`cargo test --workspace --all-features --doc --locked`, and
`cargo clippy --workspace --all-features --all-targets --locked -- -D warnings`.
The all-target gate also executes exploratory benchmark targets; their timings
are not used as performance evidence.

Full logs, unchanged fixtures and runner are retained under
`~/tmp/fused-post-call-20261029/` (`before.log`, `before-fixed-fixture.log`,
`before-final.log`, `after.log`). Native checkpoint/progress qualification is
separate: see the how-far DiagnosticPulse harness and
`just native-pulse-diagnostics-check ARTIFACTS`.

Documentation: all eight compiler catalogs regenerated with matching local/export
source fingerprints. Eight-crate strict Rustdoc and all 26 guide source checks
pass (rc0, 34s, peak RSS 0.34 GiB, minimum available 14137 MiB). The initial
reference invocation lacked GITHUB_SHA in the archive and failed before building;
its log is retained. The rerun supplies the recorded source revision explicitly.
No browser layout or native checkpoint rerun is claimed for this wrapper change.
