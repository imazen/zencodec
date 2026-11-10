# Core contract implementation stack

Current review target: [PR208 against main](https://github.com/imazen/zencodec/pull/208).
The full selected native runtime/lint gates and guide/reference/browser checks
pass. [Review order and remaining scopes](review-batch.md) and the final dated
qualification below supersede the intermediate failure reports retained here.

The owner authorized this five-layer series on 2026-11-10. Each layer has one
review target and shares the preceding implementation; it is not five competing
API proposals. Core remains unpublished pending the normal release gates.

| Layer | Review | Implementation / qualification |
|---|---|---|
| 1. Current pixel/color/fidelity baseline | PR201 (retargeted to main) | Core/render/fuzz select pixel76005fc9, including fallible allocations. Gray/HLG/ICC authority and source-depth work are retained. Generation A is a distinct fidelity request. Core all-feature library/integration tests 1435 pass, doctests32 pass/39 existing ignores; strict all-target Clippy and no-default core check pass. Render8 integration tests +1 doctest and strict Clippy pass. |
| 2. Complete frame interpretation and checked lending | [PR204](https://github.com/imazen/zencodec/pull/204) | Current color/alpha, explicit mapped encoding and shared validation; direct/image/lent consumers share refusal checks. Core1440 tests +33 doctests pass/39 existing ignores; strict all-target Clippy, core no-default and renderer controls pass. |
| 3. Evidence and late decisions | [PR205](https://github.com/imazen/zencodec/pull/205) | Runtime conversion choices retain the exact current target; report snapshots stay payload-free. Shared controls inspect opaque targets, reject context/alpha/timing drift, preserve ordinary successful admission and verify report cleanup. |
| 4. Exact media presentation | [PR206](https://github.com/imazen/zencodec/pull/206) | Ordered exact timelines, default-refusing typed/erased mux admission, known-program fallback refusal, separate presentation facts and complete conformance comparison. |
| 5. Shared conformance, migration and guide | [PR208](https://github.com/imazen/zencodec/pull/208) | Reusable direct/image/planar and full session comparisons; real AV1/TIFF witnesses, compiled code-first page and compiler/Rustdoc reference. |

Layer1's independent source-export gate is `just core-stack-check OUTPUT TARGET REVISION`
under run-heavy. Both early renderer dependency-resolution failures are retained:
its seed lock held beta.2 SIMD while the new converter requires stable0.9.31.
Updating the three locked SIMD packages together resolves that conflict. The
full rerun passes: rc0,39s,peak RSS0.49GiB. No numerical tolerance was relaxed.
The root lock is untracked; the exact resolved locks and commands are retained
with the run receipt. Renderer dependencies.lock contains the qualified seed.

Generation-A selection uses `Fidelity::zensim_a(score)` /
`LossyTarget::ApproxZensimA(score)`. It never aliases generation B or certifies a
measured result. Legacy best-effort setters still report their resolved native
quality; checked target selection rejects substitutions. The testkit asks about
both generations so codecs cannot silently advertise one as the other.

The deliberately frozen native comparison graphs remain separate evidence.
Their historical Mox CMYK failure is not fixed or reclassified by this baseline.
Native route qualification and final site/compiler snapshots remain explicit
work in the stack; runtime/core success is not a whole-repository CI claim.

Layer2 adds a retained `ColorContext` and alpha association to current frame
interpretation. Original container claims remain provenance. `Frame::map` checks
geometry and exact owner encoding before returning a loan; `with_planar` lends a
canonical view without copying sample storage. Missing context stays unknown.
The lifetime doctest rejects an escaping view. Adversarial controls cover
contradictory encoding, invalid alpha, pre-effect geometry refusal, native-return
Pulse cancellation and original native error priority. Source-export rerun passes
rc0,35s,peak RSS0.49GiB; earlier failed fixture/lint runs remain retained.

Layer3 removes the redundant `push_converted` target parameter. The conversion
offer owns that immutable target and timing once. Runtime target equality uses
all current color/alpha facts; bounded `FrameFacts` equality is diagnostic only.
`FrameRouteSummary` and `FrameConversionFacts` retain no opaque profile owner.
The isolated full gate passes with the existing exact-depth, replay, mandatory
metadata/workspace and late-boundary controls included; no source-depth hint is
promoted into a losslessness certificate. Native AV1 witness source is migrated
to the same signature; its separate graph/runtime qualification is still open.

Layer4 rebases PR188's declaration/default-refusal delta while retaining current
optional-Pulse forwarding. The animation adapter now refuses known presentation
work it cannot execute; elementary codec configuration is explicitly separate
from the retained track program. Unknown/unedited/empty edited declarations stay
distinct. The full isolated gate passes; this is shared contract admission, not
proof that MP4 edits render correctly on every native backend.

Layer5's minimal current graph passes native AV1/TIFF controls and strict Clippy.
Two AV1 fixtures (odd 5x3 U8 and gradient U16/10-bit) match independent FFmpeg
active sample bytes exactly after dropping the decoder. Native guard lending is
borrowed; the TIFF two-page adapter adopts decoded vectors. Both run the same
public testkit helper. These witnesses do not cover general AV1 session migration,
TIFF SubIFDs, arbitrary collection composition or edited MP4 playback.

The 14-conformance isolated rerun passes core1451 tests,33 doctests/39 existing
ignores, renderer8 integration+1 doctest, native2 tests, strict Clippy and core
no-default check: rc0,43s,peak RSS0.49GiB. The first native run passed its runtime
checks but failed strict Clippy on three deprecated TIFF fixture calls; their
modern equivalents retain every assertion. All original failures remain retained.
The compiler catalog and all-eight-crate strict Rustdoc reference also generate.

### Remaining integration gate before landing the complete stack

The full native roots now select the same current pixel source as core. Their
selected no-run builds pass; runtime and strict lint qualification remain
separate gates. The preserved historical numerical receipts retain their
original source identities and bounds. See the dated native-cohort update below
for exact target coverage and remaining CI failures.

No crate has been published and main is unchanged by this review series.


Final combined gate: [committed receipt](../benchmarks/core_contract_stack_2026-11-10.json).
Core1451 tests,33 doctests/39 existing ignores, renderer8+1, native2, real-ICC3,
all strict Clippy gates and no-default core check pass. The compiler catalog,
all-eight-crate strict Rustdoc,28 site checks and full browser suite pass,
including the new page at390/768/1440px. run-heavy rc0,152s,peak RSS0.98GiB.
The effective export's1011 tracked blobs match public94ac77d6 byte-for-byte;
its ICC lock was resolved before testing and then committed in that descendant.

The new browser assertion first caught a768px overflow. The fix contains bare
code blocks without hiding any examples; assertions remain unchanged. The actual
real-profile tool also now selects the current pixel source and passes all3
controls. Four older Documentation native jobs remain blocked on the historical
cohorts; [issue207](https://github.com/imazen/zencodec/issues/207) is the explicit
pre-landing integration gate. PR208 remains draft pending full native qualification; the stack does not
claim release readiness or complete native edited-playback correctness.


Caller simplicity follow-up: the homepage now leads with the actual two-line
CodecSet pixel conversion, independently compiled and round-trip tested. The
older journey reference remains at journeys.html, detailed frame contracts at
advanced.html, and migration notes stay there. Dark styling is the default.
`zencodec::advanced` supplies aliases for existing authoring contracts; it adds no
new protocol or adapter implementation. A unified short render operation across
still images, collections, animation and video remains an explicit convenience
gap. That gap must not be presented as solved by a plane-count example.


## Review readiness correction (2026-11-10)

Before the native repairs below, the complete stack was **not ready for final review or merge**. At the start of this pass, its local
shared/native-mapping gates passed, but four full Documentation jobs failed on
PR208: guide examples, JPEG, AV1 and TIFF/JXL. Those older pixel cohorts lacked
current plane APIs. The update below records the subsequent compile repair and
remaining runtime/lint failures. Issue207 is unfinished required integration, not a waived release
gate. Previous "ready for API review" language did not make this clear enough.

The twelve open PRs are not twelve independent release review requests:

- PR201/204/205/206/208 are one five-layer implementation stack. Hold final review
  until its complete native validation is green; intermediate heads include
  known failures subsequently repaired by descendants.
- PR199/200 are earlier overlapping work. Reconcile their remaining useful
  content before marking them superseded; do not ask for duplicate reviews.
- PR188/189/196/158/157 are separate presentation experiments, publication work,
  assessment mapping, optional HDR capability and C2PA work respectively.
  Green checks alone do not certify that each proposal's design was reviewed.

All currently failing proposals are held in draft. Main and publication remain
unchanged. The next release-critical work is the actual cohort migration and full
native tests, rather than more API surface or presentation changes.


## Native cohort update (2026-11-10)

PR208 commit `c83798f6` synchronizes fifteen standalone manifests with the root
pixel/converter and garb sources, updates their tracked seed locks, and migrates
native harness ownership/exhaustiveness calls. Codec and linear-sRGB revisions
stay fixed. The converter requires MoxCMS0.9.2; the affected JPEG/spatial locks
therefore move from0.9.1 to0.9.2. Existing pixel assertions and quality bounds
are unchanged. `just validation-cohort-check` now detects source-pin drift.

The isolated no-run gate at `49a12cc7` passes all five selected native roots:
guide I/O, JPEG, AV1, TIFF/JXL and LCMS. Its I/O selection covered seven of the
eight CI integration targets plus both bins; `policy_execution` was omitted.
The committed runner now includes all eight, and its source test compares the
selection with CI. That corrected selection has not been rerun locally.
The other roots compiled their libraries and tests; this is not an all-target
strict lint or runtime result. The wrapper reports rc0,74s,peak RSS0.72GiB.
The small [compile receipt](../benchmarks/native_cohort_compile_2026-11-10.json)
records the exact source and command coverage.

CI on `c83798f6` confirms core platform tests, MSRV, WASM, lint, fuzz,
current native frame interchange and the guide/browser job pass. Its concrete
remaining failures are:

- Guide examples execute successfully, then strict lint fails on deprecated
  scalar edit-delay calls in pinned zenwebm/zenmp4 dependencies. Migrating those
  callers must preserve the edited-presentation limitations; a scalar is not a
  complete timeline.
- JPEG reports67 passing controls and one failure in
  `public_guide_executes_all_transport_and_policy_recipes`: its end-to-end checked
  transcode returns a Plan-stage error. This remains a runtime blocker.
- AV1 strict lint fails on the edit-list example's deprecated scalar accessor
  and two redundant Arc conversions. Its no-run success did not qualify examples
  or strict lint.

The TIFF/JXL job was still running at this update. No full-runtime pass is
claimed for it or the separate LCMS root. Historical Mox CMYK precision failures
remain unwaived. At that intermediate checkpoint the five-layer stack remained draft; the final
consolidation and qualification update below supersedes that status.
The next bounded gate is one replay of the exact failing commands with existing
assertions and frozen codec kernels, stopping on substantive correctness failures.


## Consolidated review qualification (2026-11-10)

The complete implementation is now reviewed through **PR208 against main**;
PR201/204/205/206 are superseded layer views whose commits remain intact.
PR199/200/189 are incorporated predecessors. PR188's declaration changes are
incorporated while its remaining native playback experiments stay preserved
for issue187. [Review order and all proposal/issue dispositions](review-batch.md).

All previously failing native roots now pass their selected **runtime and strict
all-target Clippy** gates: all eight I/O integration targets and both bins;
complete JPEG68 and AV170 controls; complete TIFF/JXL149 controls. Native JPEG
preflight also passes on its feature-enabled graph. The separate LCMS control
passes the strict0.0013 accuracy bound and all-target lint after the shared test
change. Existing independent float/Gamma22 checks, source-transport controls and
the exact transport-model CSV comparison pass. This is scope-specific native
qualification, not proof that every codec configuration or edited timeline works.

The JPEG guide now explicitly provides MoxCms where authoritative ICC conversion
needs a CMS; the original failure retains its typed NeedsCms cause. Frozen muxer
functions retain scalar compatibility via four count-checked, function-scoped
lint annotations, with upstream/derived hashes in sources.json. Their runtime
source and known edit-list limitations are unchanged. A separate diagnostic's
scalar use is likewise explicitly marked compatibility-only.

The owner approved replacing the failing Mox accuracy expectation with an
explicit known-discrepancy regression: every direct-backend/finalizer sample,
source/context/storage assertion, finite check and polarity control remains.
The original independent0.0013 comparison is still computed and reported as
unqualified (937/786432 violations,max0.006689250469207764). LCMS retains the
strict passing reference gate. [zenpixels157](https://github.com/imazen/zenpixels/issues/157)
tracks minimal upstream-report preparation; no MoxCMS issue was opened.

Compiler catalogs, all-eight-crate strict Rustdoc and the full source/browser
site gate pass after the publication-manifest update. Core runtime source is
unchanged from the passing Linux x64/ARM, Windows x64/ARM, macOS ARM, i686,
MSRV,WASM,lint and fuzz CI at c83798f6; only I/O's publication manifest changed.
No platform claim relies on an unexecuted new backend path.

[Exact commands, source identities and resource receipts](../benchmarks/review_batch_2026-11-10.json)
retain the original failures separately. Main remains unchanged. This batch is
for API review, not approval to publish or a claim that all roadmap issues close.
