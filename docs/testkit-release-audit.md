# Testkit release audit — 2026-10-29

The testkit is not a complete conformance specification for the current API.
This source audit uses `5a7e3b45`. A green codec `check_all` is useful evidence
for its selected default fixtures, not certification of every public contract.
The older [integration matrix](testkit-contract-coverage.md) covers a narrower
campaign; its adapter counts are not current whole-suite coverage.

## What the one-call path actually runs

`zencodec-testkit/src/lib.rs::check_all` directly calls 16 distinct check helpers.
The three public-check source modules (`lib.rs`, `contracts.rs`, `adversarial.rs`)
define 33 `check_*` functions besides `check_all`. These are lexical inventory
counts, **not a coverage fraction**: the incremental capability helper invokes
`check_incremental_contract`, which invokes `check_incremental_decoder`.
Other helpers also compose checks. Capability-dependent cases and first-error
returns mean a direct call is not proof that every branch executed.

The README example and `check_all` Rustdoc now explicitly describe a subset.
In particular the fixture-driven policy admission, rewrite, explicit
animation timing, inventory input-limit and error-envelope checks need separate
invocations. Do not add all of them blindly to `check_all`: callers must supply
format-specific expectations, malformed inputs and independent observations.

## Coverage and remaining work

| Contract | Existing evidence | Reusable-suite gap / next gate |
|---|---|---|
| Pixels, metadata, orientation, HDR, output timing | Default image helpers, reference/minimal codecs and dedicated mutation tests | Each native configuration still needs real fixtures and independent readback; a same-codec round trip can share a bug. |
| Incremental input and terminal state | `contracts.rs::check_incremental_decoder` compares chunked output, routing and cancellation | Unconditional pre-cancel checks now preserve cancellation/timeout reasons, forbid all sink effects and require terminal state; mutation controls reproduce the former gap. Mid-operation checkpoint frequency and native adapter invocations remain separate gates. This is not cancellation-latency evidence. |
| Mandatory requirements before effects | Core `tests/collection_workspace.rs`, renderer discovery tests, native admission harnesses | No common image testkit helper accepts an effect-counted factory and exercises every bound/legacy/callback route against each codec. Keep the existing core controls; add adapter fixtures without inventing workspace bounds. |
| Retained plans, Rows/Replay, typed/erased job ownership | Production driver tests and `perf/native-contract-validation` | Promote the proven native controls into a reusable caller-supplied source/encoder harness. Cover plan-once/start-once, min/max passes, exact replay, stride, owner cleanup and output scheduling together. |
| TIFF/JXL collections and rendering | Spatial/layer harnesses and core placement oracle | No generic collection conformance helper covering item dependencies, SubIFDs, raw versus rendered pixels, oversized placement and native composition. Manifest-based expected inventory plus independent rendering is required. |
| Precision and provenance | Core admission and paired zenpixels verifier tests | No generic native encoder proof that canonical depth evidence matches its quantization/packing rule. AVIF coded depth alone cannot certify losslessness. |
| Media queues, timestamps, lifetime and mux output | Separate `zencodec-media-testkit/src/checks.rs`: decoder/encoder, demux/seek, mux honesty and animation-over-video helpers | This is a separate suite, not included by image `check_all`. Native codec/container/configuration coverage must be recorded separately from toy implementations. |
| Async/WASM execution and memory | I/O cancellation/publication tests; model-backed media lab | No browser-native shared Pulse bridge qualification, global Rayon budget proof or whole-operation peak-memory guarantee. Browser model results cannot fill these cells. |
| Allocation failure | Fallible-capacity/resource-limit regressions | No systematic allocation-failure injection. Overflow rejection is not evidence for every allocation failure point. |

## Ship criteria

Before calling a codec/configuration qualified, record the exact source graph,
features, fixtures, helpers invoked (including opt-in helpers), expected refusals,
independent oracle and failing-first mutations. A capability flag is not a
replacement for running the applicable check. Unsupported configurations must
produce their documented typed refusal, not disappear behind an implicit skip.

First strengthen cancellation and effect-ordering coverage, then extract the
already-proven replay and collection controls. Keep model scheduling, UI polish
and benchmark tuning out of the core release gate unless they expose a missing
contract. Testkit publication separately depends on releasing I/O; see
[package dependency gates](release-candidate.md#package-dependency-audit-2026-10-29).

## Pulse diagnostic integration

The testkit's development-only `how-far-really` instrumentation wraps the actual
reference incremental decoder with `DiagnosticPulse`, `PulseTree` and `Profiler`.
It records cancellation request and operation return separately, finishes with
the observed result outcome, requires retained stop reasons and complete traces,
and detects `StopOutcomeMismatch` in a deliberately faulty wrapper. It uses
structured diagnostic kinds, not display strings or timing thresholds.

Native follow-through must add an outer operation span even when no checkpoint
occurs, and one span per worker so a busy worker cannot mask a silent worker.
Record cancellation return only after cleanup and worker joins. Inspect
`UnobservedCancellation`, `StopGap`, `ReportGap`, `AbandonedPhase`,
`CountMismatch` and incomplete-evidence counters alongside explicit expected
work. A missing report is not automatically a violation for an operation that
promises no reporting. Gap durations and interruption latency are measurements
with scheduling interference, not deterministic CI limits. Instrument any
external replacement stop source too; otherwise it may bypass the observer.

Qualification and retained failing-first evidence:
[pre-cancellation receipt](../benchmarks/testkit_precancellation_2026-10-29.md).

## Native JPEG follow-through

`just native-pulse-diagnostics-check` runs the strengthened incremental helper
on baseline and progressive native JPEG fixtures, plus direct native decode with
DiagnosticPulse. Both modes preserve one-shot Cancelled and TimedOut at early,
middle and final observed checkpoints. This exercises native code, not only the
shared Fused wrapper. The fixture is 97x65; it is not full JPEG configuration or
all-checkpoint coverage.

The successful traces contain 15 cancellation checks for baseline and 57 for
progressive, but only one progress report of 65 units in each mode. Intermediate
reporting therefore remains a concrete UX gap, tracked in
[zenjpeg issue 251](https://github.com/imazen/zenjpeg/issues/251). Positive report
counts alone do not establish useful progress granularity. No latency bound or
optimal polling frequency is inferred from these instrumented runs.

See [native diagnostic evidence](../benchmarks/native_pulse_diagnostics_2026-10-29.md).
