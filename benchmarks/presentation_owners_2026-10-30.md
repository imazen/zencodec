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


## Session-bound presentation identity

Source 6678866bcde9e6021cf50469227bc78ba10e7503 adds a second native control.
Two AV1 decoders use identical configuration/epoch and compressed input. All
packet ordinals equal 7 and offsets equal 1000; frame output confirms these
values. Separate retained cursors project to origins 0 and 1000 ms. Interleaved
input and reversed session drain order preserve exact samples, shared picture
owners, three presentations per session and 40 ms durations. Both one- and
four-thread decoder configurations execute. Dropping both sessions releases
both map slots. The existing delayed-epoch control remains unchanged.

This establishes session ownership as a usable presentation identity for
independent replay operations, without adding a mandatory field to codecs.
It does not establish mid-session map replacement, arbitrary random access,
metadata completion, or resource admission for two simultaneous native contexts.
The output collection retains frames in this fixture.

Initial source 6fc95811 failed compilation because the test omitted the
Presentation import. No native control ran on that source. The correction only
qualifies that type path; assertions and dependency locks remain unchanged.
Initial log: /home/lilith/tmp/zencodec-map-owner-20261030/sessions.log on r7900x;
wrapper rc101, 6 seconds, peak RSS 0.39 GiB.

Corrected isolated export: all 12 map tests, both selected native presentation
owner controls and both strict all-target Clippy gates pass. Fifty other native
harness tests are filtered and not claimed rerun. Explicit package clean ensures
native harness compilation from the export. Wrapper rc0, 10 seconds, peak RSS
0.39 GiB, minimum available 13510 MiB, peak load 0.51. Full log:
/home/lilith/tmp/zencodec-map-owner-20261030/sessions-fixed.log on r7900x.
`just presentation-epoch-check` now runs both native owner controls.


## Owned presentation pump

Source 3f4e23e028bafc46811d8d850f9520b09c7a7033 adds the reusable private Pass<D>
over the existing native send/drain pumps. Projection occurs inside their output
callback, so dropping a frame does not break the native ReceivePending progress
contract. State is taken before pumping and restored only after successful send;
error/unwind and finish drop the decoder/cursor. No driver frame queue exists.
The projection preserves frame presentation flags and packet provenance while
changing only timestamps/duration and retaining the original picture owner.

The new native test exercises five cases: successful clipping of the first frame,
output BrokenPipe coincident with timeout, incomplete finite coverage,
pre-cancellation, and timeout after the first output. Success checks exact native
samples and presentation times. Errors retain typed causes; post-publication
cancellation delivers exactly one frame, pre-cancellation none. Every case
releases the map slot and refuses subsequent finish with Finished. Coverage
failure can occur after earlier frames were delivered; no rollback is promised.

Initial source 66575711 failed compilation due to the test's missing SessionError
import; no native test ran. The corrected source adds that import and the two
cancellation controls. Failed log: /home/lilith/tmp/zencodec-map-owner-20261030/driver.log.

Isolated corrected run: 12 map tests and three selected native owner/driver tests
pass, with 51 other native tests filtered. Both strict all-target Clippy gates
pass. Native harness explicitly rebuilt from the export; committed locks
unchanged. Wrapper rc0, 10 seconds, peak RSS 0.39 GiB, minimum available 13473 MiB,
peak load 0.65. Full log: /home/lilith/tmp/zencodec-map-owner-20261030/driver-fixed.log
on r7900x. Reproduction remains `just presentation-epoch-check`.

This is single-segment execution, not the complete presentation API. The caller
supplies an already admitted/configured decoder. The driver drains to native EOF
rather than using finite coverage as an early-completion signal. Source metadata
obligations, whole-operation workspace, multi-segment acquisition and gap policy
are not inferred from these results. Native calls and blocking downstream
callbacks still require their own cooperative interruption.
