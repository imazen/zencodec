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

The README example and `check_all` Rustdoc currently overstate this as “every”
check. In particular the fixture-driven policy admission, rewrite, explicit
animation timing, inventory input-limit and error-envelope checks need separate
invocations. Do not add all of them blindly to `check_all`: callers must supply
format-specific expectations, malformed inputs and independent observations.

## Coverage and remaining work

| Contract | Existing evidence | Reusable-suite gap / next gate |
|---|---|---|
| Pixels, metadata, orientation, HDR, output timing | Default image helpers, reference/minimal codecs and dedicated mutation tests | Each native configuration still needs real fixtures and independent readback; a same-codec round trip can share a bug. |
| Incremental input and terminal state | `contracts.rs::check_incremental_decoder` compares chunked output, routing and cancellation | Its trip-after-two cancellation branch can complete before cancellation is observed. Add unconditional pre-cancel and one-shot stop-reason/no-effect controls, then invoke them across native codecs. This is not cancellation-latency evidence. |
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
