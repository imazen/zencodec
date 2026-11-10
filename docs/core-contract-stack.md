# Core contract implementation stack

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

The legacy full AV1 harness still selects pixel fde59b96. A fresh locked build
against this core fails with six core diagnostics: absent PlanarSlice and new
layout/stride checking APIs. Its frozen spatial Mox/LCMS comparisons likewise
remain historical cohorts, not current-core qualification. Do not change an old
numeric receipt's source identity or relax the preserved Mox CMYK bound. Migrate
these standalone consumer graphs explicitly before claiming whole-repository CI
or enabling a broad codec rollout. The new minimal graph demonstrates the frame
contract against real native storage while preserving that limitation.

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
pre-landing integration gate. PR208 is ready for API review; the stack does not
claim release readiness or complete native edited-playback correctness.
