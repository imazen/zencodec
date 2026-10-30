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


## Owned passes across a complete program

Source d3d3424980689533ea58be39d22fd50fc89ea545 integrates the same private
Pass<D> into baked_presentation's --owned-driver mode. Program admission precedes
Plan construction, one immutable owner survives all media/gap segments, and a
new native decoder/cursor handles each media segment. All source packets are
replayed and native EOF is drained for each pass; there is no implicit early
metadata completion. Black gap fill is explicit in the example, not a driver
default. Weak picture controls require native pictures to be released after
encoder disposal; the application-owned black frame is deliberately excluded.
The last program owner releases its pool slot after all segments complete.

The twelve generated MP4s are byte-identical to the earlier buffered pump-files
reference. Independent FFmpeg decode/probe passes all twelve sample/timing
comparisons for identity, clip, repeat, gap, dwell and double speed, each in
progressive and fragmented MP4. Expected raw samples and the oracle assertions
remain unchanged. The original retained-edit player disagreements are separate
and remain failures; this route bakes edits into samples/timestamps.

Initial source d0b26811 generated files, then strict example Clippy stopped on
the private Mapping error field never being inspected outside tests. Rather than
suppress it, the mapping error now implements core::error::Error and ProjectionError
exposes it through source(), preserving typed diagnosis for consumers. The
initial failure remains in program.log. Corrected program-fixed.log records:
12 map tests, three native owner/driver controls, example and all-target strict
Clippy, twelve independent playback cases and twelve byte-identity comparisons.
Other native tests remain filtered. Both committed locks are unchanged.

The isolated run explicitly cleaned/rebuilt the native harness. Wrapper rc0,
13 seconds, peak RSS 0.39 GiB, minimum available 13524 MiB, peak load 0.54.
These are validation resources, not a performance or whole-operation memory bound.
Logs: /home/lilith/tmp/zencodec-map-owner-20261030/{program,program-fixed}.log
on r7900x. Artifacts under the prior presentation-playback-20261030 root:
owned-program-fixed-files and owned-program-fixed-oracle (results.json,
versions.json, raw output, probe and decoder diagnostics). The initial
owned-program-files directory is preserved. Reproduce with just presentation-owned.

Remaining contract scope: the example still owns compressed source/output packet
vectors; native construction/configuration admission is external to Pass::new.
It has no situational gap/acquisition callback, no final metadata completeness
proof, and no automatic live-session map replacement. No new published API or
zenpixels change is introduced by this integration.


## Policy after projection and before encoder output

Source 593b11c1411691aa766ad1a302a6986478b50976 adds the private Publication<R>
owner. It checks bound unknown workspace before the example prepares plans or
native contexts, retains the same operation policy/report across all passes and
gaps, and calls shared resolve_frame on the projected frame before downstream
output. State is poisoned before policy/output callbacks and restored only after
success and the post-output Pulse check. Default formatting is bounded; source()
retains policy/application/downstream errors.

A real AV1 clip control sees callback PTS 1000 ms rather than source PTS 40 ms.
Default selection preserves exact samples. Explicit decline produces no output;
DiscardDescriptive requirements refuse before callback invocation; an application
PermissionDenied error remains recoverable through the full driver error chain.
Every failure is terminal and releases the native pass's map slot. Zero and
u64::MAX workspace limits both refuse unknown evidence. The report remains
accessible after refusal; it records decisions, not a full output effect journal.

All twelve owned-program MP4s still match the buffered reference byte for byte and
pass the independent sample/timing oracle, including filled gaps. Complete map
suite 12/12, four selected native owner/driver/policy controls, strict example and
all-target Clippy pass. Fifty-one other native harness tests are not rerun.
The isolated export explicitly rebuilt the harness; both locks remain unchanged.
Wrapper rc0, 13 seconds, peak RSS 0.39 GiB, minimum available 13477 MiB,
peak load 0.28. These are validation resources only. Full log:
/home/lilith/tmp/zencodec-map-owner-20261030/policy.log on r7900x. Artifacts:
owned-policy-files and owned-policy-oracle under the prior persistent artifact root.
Reproduction remains just presentation-owned and presentation-epoch-check.

This implements the existing frame-publication question after projection, not
a new strategy resolver. It does not prove complete-container metadata removal,
late metadata discharge, whole-operation resources, or automatic policy selection
of gap/acquisition strategies. Failure cannot roll back frames already emitted.
