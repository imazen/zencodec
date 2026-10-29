# Release candidate landing gates

The immediate deliverable is a reviewable zencodec release candidate. The larger
codec-agnostic image/video/transcoding goal remains open. Do not label the entire
codec family or every native mode qualified from this candidate's evidence.

## Freeze

Use main plus PR175's joint route and production Rows/Replay driver. The retained-job API candidate in this batch is `EncodeJob::into_dyn_job`: an additive
provided method using the existing adapter, needed to retain native job-specific
configuration through shared dispatch. No new policy domains, proof framework,
source abstraction or rendering redesign enters this landing batch.

The subsequent caller convenience decision makes borrowed per-call controls
`Option<&dyn Pulse>`. `None` does not disable retained job cancellation or
mandatory policy checks. Unknown optional facts do not prevent default success;
unknown evidence for an explicit requirement still refuses before affected
effects. A situational callback may act on later facts while those effects remain
withheld under admitted storage. See [the contract and current native
qualification](optional-pulse.md).

Keep explicit refusals for unproven required evidence. Native AT_FINISH
continues to retain compressed output; this is not a streaming memory guarantee.
The provided erasure method passes core ownership controls and the native public
replay-driver matrix with strict Clippy on `a39616ba`. The separate native final integration gate and strict library Clippy now pass
on the optional-control graph at `6ab547aa`; its Undefined-slot native/shared
controls preserve the existing slot bytes and row-padding invariance. See [the optional-control receipt](../benchmarks/optional_pulse_2026-10-29.json) and
[the public-driver receipt](../benchmarks/replay_driver_2026-10-28.md).

## Required before merging this batch

1. **Passed on `6ab547aa`, receipts pushed as IMI `daa9c13d`:** native retained-job,
   encode/row/one-shot controls and strict library Clippy, including the corrected
   Undefined-slot native/shared controls. Broader feature/platform qualification
   retains its own scope; preserve the established opaque-slot contract.
2. **Passed on `a39616ba`:** corrected native/public-driver matrix, direct
   configured-job erasure, core ownership tests and strict Clippy. The graph audit
   confirms one core/pixel family. Previous failures and all no-effect, exact-byte,
   cleanup and typed-cause assertions are retained.
3. **Passed on `c910076c`:** refreshed API snapshots and eight catalogs,
   eight-crate Rustdoc HTML, 24 site/source controls and the full browser suite on
   its own server. Generated snapshots are committed unchanged from that export.
4. Inspect PR CI on Linux, Windows x64/ARM64, macOS ARM64, i686, WASM,
   MSRV, Clippy, format and API/semver gates. Locally qualified related changes
   may merge without waiting for CI under the owner's current instruction;
   investigate failures promptly. Publication still requires all release gates
   green. The first Windows run found a
   fixture's HOME assumption; the USERPROFILE correction passes on `d7b5a159`.
   The complete CI, fuzz and documentation workflows pass on `b45cadb2`,
   including the required Windows ARM64, macOS ARM64 and i686 jobs. Later
   guide/harness-pin changes preserve those library sources and pass their
   local native/browser gates on `9b606366`.
5. Review the exact public API diff, deprecated replacements, README, changelog
   and supported-mode matrix. Record remaining unmeasured performance/memory/
   compile-time costs explicitly; validation-wrapper RSS is not an encoder bound.

Merge only qualified commits. Unbuilt native proposals may be reviewed in draft
PRs with their missing gates explicit, and no test
failure is hidden with an ignore, feature exclusion or weaker assertion.

## Separate application integration gate

zenpipe/zencodecs `f0322466` integrates the qualified HEIC/RAW Pulse adapters.
All five portable roots pass source/family checks; the complete configured API
feature profile compiles and passes strict library Clippy. API generation and
freshness checks pass. The core guide's new consumer pin passes 48 native
JPEG/conversion scenarios, strict Clippy, 24 source tests and the full browser
suite on `9b606366`; see [the receipt](../benchmarks/guide_integration_2026-10-29.json).

Two AVIF encode-orientation carrier assertions remain unresolved: the tests
require stored tags while native still encoding bakes pixels. Neither assertion
was weakened. The independent consumer bug dropping the decode OrientationHint
is fixed with exact-pixel controls. Complete application/platform runtime and
Apple DNG extraction coverage remain separate. Do not mark that application
release ready from core CI. Any finding requiring a core contract change
returns to candidate review.

## Remaining measurements

The older [companion compile study](../benchmarks/compile_adapters_2026-10-27.md)
is exploratory: all samples recorded background CPU load, six recorded heavy
processes, and supported features differ between baselines. It is not a clean
comparison of this final candidate. Final-candidate compile time, native replay
throughput, total workspace and whole-operation memory costs remain unmeasured;
validation-wrapper receipts do not establish those bounds. Paired pixel changes
and their migration costs remain documented in the precision contracts.

## Publication

A mergeable release candidate is not publication authorization. Review README
with the owner, obtain explicit release approval, run all-target and doc tests,
verify the coordinated published dependency graph, and require green release CI
before the tag, GitHub release and crate publication sequence.

## Work discipline for this landing

One integration owner, no new exploratory lanes, one serialized qualification
batch per candidate, and focused reruns only for actual failures. Preserve all
reports and source commits. Maintain a bounded failure list in the owning docs;
do not spend remaining capacity repeating already-qualified unchanged matrices.

A fresh outer-consumer resolution on 2026-10-29 rejected the registry requirement
`zenpixels >=0.2.17, <0.4.0` (Cargo reported 0.3.0 yanked). The candidate's root
patch selects `283655c5`; downstream roots must provide that patch until the
paired pixel release is published. Benchmark consumers now carry the immutable
source patch explicitly. A successful workspace build does not qualify an
unpatched crates.io consumer graph.

## PR175 compile observation

[The 45-build comparison](../benchmarks/compile_candidate_2026-10-29.md) completed.
Load flags prevent a confirmed performance claim; retained medians describe only
the measured core/io/media consumer, not crates.io or full native applications.
