# Retained presentation-map admission — 2026-10-30

Source: 37528d9dabd4e0fb1860ea13c053e0044fe265c5.
Private experiment only; no released API or codec implementation changes.

`PlanPool::prepare` checks the live owner limit and reserves bookkeeping before
calling the plan builder. The pool retains weak references; a RetainedPlan clone
or RetainedCursor keeps the same immutable plan and slot alive. A failed builder
leaves no occupied slot. Zero/full limits do not invoke the builder. Dropping the
pool leaves already issued owners valid.

The existing real AV1 delayed-epoch test now uses this pool and owned cursors.
Both one- and four-thread configurations still require actual delayed old-epoch
frames, unchanged temporal projections, and the control where the latest map
loses earlier presentations. Dropping the original lookup list retains six live
plans through cursors. Another prepare refuses before its builder. After cursor
completion/drop a separate replay owner still retains one slot; only dropping
that owner retires the last plan. Existing pixel/epoch assertions are unchanged.

Validation in an isolated source export with unchanged committed locks:

- Entire presentation-map suite: 12 passed, no ignored/filtered tests; doctests
  contain no runnable examples. Strict all-target Clippy passes.
- Native `presentation_maps_follow_delayed` control: passed; 51 other harness
  tests filtered, not claimed rerun. Both native thread configurations execute
  inside this control. Strict all-target harness Clippy passes.
- Explicit native harness package clean; compilation paths identify this export.
- Wrapper: run-heavy --mem 3G --jobs 2 -- timeout 900 bash run.sh; rc0,
  10 seconds, peak RSS 0.39 GiB, minimum available 13443 MiB, peak load 0.58.
  This is validation resource use, not native decode or map memory accounting.

The persistent recipe is `just presentation-epoch-check`, now including the
complete map suite before native controls. Full log:
/home/lilith/tmp/zencodec-map-owner-20261030/run.log on r7900x.
Recorded process exited and no cargo/rustc process remained after qualification.

Limits: admission counts distinct retained plans, not descriptor bytes, copied
segments, native pictures, source storage or the whole operation. Arc allocation
uses ordinary allocator behavior. The native fixture collects its decoded frames
before constructing maps; it proves map routing/lifetimes, not pre-decode resource
admission. A demuxer-to-frame presentation-only identity protocol remains open.
No automatic map retirement is inferred from decoder drain or finite coverage.
