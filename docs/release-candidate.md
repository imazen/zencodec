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
replay-driver matrix with strict Clippy on `a39616ba`. The separate native final
integration gate passes its runtime controls but still needs its fixture-only
Clippy correction qualified. See [the receipts](../benchmarks/replay_driver_2026-10-28.md).

## Required before merging this batch

1. Complete the native retained-job integration/Clippy gate on the corrected
   Undefined-slot fixture. Preserve the established opaque-slot contract.
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
   The subsequent configured-job/reference update has fresh CI in progress.
5. Review the exact public API diff, deprecated replacements, README, changelog
   and supported-mode matrix. Record remaining unmeasured performance/memory/
   compile-time costs explicitly; validation-wrapper RSS is not an encoder bound.

Merge only qualified commits. Unbuilt native proposals may be reviewed in draft
PRs with their missing gates explicit, and no test
failure is hidden with an ignore, feature exclusion or weaker assertion.

## Separate application integration gate

zenpipe/zencodecs full-profile qualification still needs the prepared HEIC/RAW
Pulse migrations, the coordinated dependency graph, and resolution of the AVIF
orientation expectations without changing its established displayed-pixel
behavior. Its publication adapter subset passed; that does not prove the full
format profile. Do not mark that application release ready from zencodec CI.
Any finding requiring a core contract change returns to this candidate review.

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
