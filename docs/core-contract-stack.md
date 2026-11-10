# Core contract implementation stack

The owner authorized this five-layer series on 2026-11-10. Each layer has one
review target and shares the preceding implementation; it is not five competing
API proposals. Core remains unpublished pending the normal release gates.

| Layer | Review | Implementation / qualification |
|---|---|---|
| 1. Current pixel/color/fidelity baseline | PR201 (retargeted to main) | Core/render/fuzz select pixel76005fc9, including fallible allocations. Gray/HLG/ICC authority and source-depth work are retained. Generation A is a distinct fidelity request. Core all-feature library/integration tests 1435 pass, doctests32 pass/39 existing ignores; strict all-target Clippy and no-default core check pass. Render8 integration tests +1 doctest and strict Clippy pass. |
| 2. Complete frame interpretation and checked lending | Stacked frame PR | Current color/alpha, explicit mapped encoding and shared validation; direct/image/lent consumers share refusal checks. Core1440 tests +33 doctests pass/39 existing ignores; strict all-target Clippy, core no-default and renderer controls pass. |
| 3. Evidence and late decisions | Next stacked PR | Reuse retained planning and OperationPolicy; implementation follows layer2. |
| 4. Exact media presentation | Next stacked PR | Rebase the reviewed timeline/default-refusal delta; implementation follows layer3. |
| 5. Shared conformance, migration and guide | Next stacked PR | Reusable contract controls and final public surface/docs. |

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
