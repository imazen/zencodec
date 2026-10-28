# Release candidate landing gates

The immediate deliverable is a reviewable zencodec release candidate. The larger
codec-agnostic image/video/transcoding goal remains open. Do not label the entire
codec family or every native mode qualified from this candidate's evidence.

## Freeze

Use main plus PR175's joint route and production Rows/Replay driver. The only
additional API candidate in this batch is `EncodeJob::into_dyn_job`: an additive
provided method using the existing adapter, needed to retain native job-specific
configuration through shared dispatch. No new policy domains, proof framework,
source abstraction or rendering redesign enters this landing batch.

Keep conservative explicit refusals for unknown evidence. Native AT_FINISH
continues to retain compressed output; this is not a streaming memory guarantee.
The provided erasure method remains unqualified until its executable gates pass.

## Required before merging this batch

1. Complete the native retained-job integration/Clippy gate on the corrected
   Undefined-slot fixture. Preserve the established opaque-slot contract.
2. Execute the corrected native/public-driver matrix, including direct configured
   job erasure and its core ownership tests. Keep the previous failed run and all
   no-effect, exact-byte, cleanup and typed-cause assertions. Audit one core/pixel
   dependency family before building. Finish strict Clippy.
3. Refresh API snapshots/catalogs and run the full site browser suite on the
   candidate's own server. Shared-driver controls and the isolated browser suite
   already pass on `03b91e5d`; new API signatures require fresh capture.
4. Require green PR CI on Linux, Windows x64/ARM64, macOS ARM64, i686, WASM,
   MSRV, Clippy, format and API/semver gates. The first Windows run found a
   fixture's HOME assumption; the USERPROFILE correction is awaiting CI.
5. Review the exact public API diff, deprecated replacements, README, changelog
   and supported-mode matrix. Record remaining unmeasured performance/memory/
   compile-time costs explicitly; validation-wrapper RSS is not an encoder bound.

Merge only qualified commits. Unbuilt native proposals remain local, and no test
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
