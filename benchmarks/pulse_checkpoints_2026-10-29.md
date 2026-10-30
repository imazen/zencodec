# Shared Pulse checkpoint conformance

Base: `8499408519c6d9753742f5f5f259f558606d34d4`. New testkit API:
`pulse::check_checkpoints`, `pulse::Case`, `pulse::Limits`. No production codec
signature changes. The existing diagnostic dependencies move from testkit's
dev dependencies to its normal dependencies so downstream tests can use the
helper; codecs still consume testkit as a dev dependency. Cargo.lock is unchanged.

ARM64 Rust 1.96.0, serialized run-heavy 3G/jobs2, timeout900.
`just pulse-checkpoints-qualification` reproduces the final command sequence:
all-target testkit tests, doctests, strict all-target Clippy, eight generated
catalogs, strict Rustdoc and site source controls. Final receipt: rc0, 98s,
peak RSS 0.35 GiB, minimum available 14364 MiB, peak load 2.36.
186 Rust tests/doctests passed, zero failed or ignored; 26 site source checks
passed. Eight catalog fingerprints match the source. No browser run is claimed.
These resource receipts describe validation, not codec performance or RAM bounds.

Six new controls exercise complete checkpoint enumeration with both reasons,
swallowed one-shot stops with observable failure traces, timeout collapse,
missing/excessive checks, dropped diagnostics/observer failure, and an abandoned
child that started work. The helper checks actual returned outcomes, never the
expected outcome, when recording diagnostics. Codec-specific work and pixel
assertions belong in the operation closure/trace observer.

Earlier attempts remain recorded: strict Clippy rejected a nested conditional
(rc101, 32s, peak RSS 0.36 GiB); the focused repair passed (rc0, 4s, 0.23 GiB).
The initial abandonment fixture dropped a child before it started and therefore
incorrectly expected rejection (rc101, 16s, 0.32 GiB). The corrected fixture
starts the child; the helper now explicitly checks terminal non-abandoned tree
states. Its first compilation needed the Report trait import (rc101, 4s,
0.27 GiB). No pre-existing assertion, skip or behavioral expectation changed.
Logs: `~/tmp/pulse-checkpoints-20261029/`; no executable archive retained.

Native JPEG adoption and qualification are pending. This does not certify
useful progress, parallel-worker spans, report/finish-triggered cancellation,
async lifecycle, latency or all possible thread schedules. The synchronous
closure must join workers before return; use a timeout around the test command.

The first PR185 native-JPEG CI job refused its stale standalone lock before
compilation. Regeneration in the isolated export adds only how-far-along and
how-far-really edges to that lock's zencodec-testkit package. The API-usability
harness also adds critical-section 1.2.0 and the two diagnostic packages 0.1.0.
Both graphs pass locked offline metadata resolution; no pre-existing package
identity or codec pin changes. Native runtime CI requalification is separate.
The historical io-validation workspace lock is an input to a recipe that
explicitly resolves current core dependencies; it is not an unchanged-lock gate.

## Native graph preflight

A separate JPEG progress candidate failed compilation because its PNG dev
edge selected zenpixels 0.2.16 alongside the core's 0.3.1. The new
`scripts/check-codec-graph.py` reads resolved Cargo metadata and checks pixel
source identity at core-family and converter interfaces. It reproduces that
failure (7 interface consumers, 1 mismatch), accepts the repaired graph (7,
zero mismatches), and accepts this repository's isolated JPEG harness (10,
zero mismatches). Five Python controls cover a legacy unrelated pixel branch,
native and converter splits, equal-version/different-source identity and missing
resolved input. The JPEG CI/local recipe now invokes the gate before compilation.

The gate does not infer every Rust type-sharing relationship. For example,
zensim-regress consumes PNG output without depending directly on the core family;
that boundary still needed source review and lock alignment. Use metadata from
the actual selected features/target and retain compilation/runtime gates.
Accepting a lock is not proof that a fresh unlocked resolution chooses the same
compatible flexible edges. No pre-existing package versions were upgraded.
