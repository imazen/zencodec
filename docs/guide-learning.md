# What a reader can build from the guide

Assessment started 2026-10-25. The task definitions and native oracles were
committed in `ef35079b` before grading. The baseline is `75f37b78`; the revised
site is `d4280dff`. The [protocol](evaluation/README.md) and
[attempt data](../benchmarks/guide_learning_2026-10-25.csv) are reproducible.

## Method and limits

Two fresh GPT-6.1-Sol sessions use high reasoning effort and the same task order.
Each receives one rendered site section, submits an attempt, then receives its
expanded inline context. Earlier packets remain learned context. Neither reader
gets repository source, linked pages or compiler feedback. The coordinator
compiles preserved attempts against the pinned QOI/BMP and JPEG harnesses.

This is one model session per presentation, not a human usability rate or a
population comparison. The tasks are selected, ordinary Rust knowledge is
allowed, and model training knowledge cannot be removed. The second site was
revised using the first reader's findings; this is a diagnostic replication,
not an independent generalization benchmark. Text packets measure access to
information, not visual preference. Desktop/mobile screenshots are reviewed
separately. Packet word counts include preserved hyperlinks.

The baseline tool audit found only packet reads and answer writes after one
initial instruction-file listing and unsuccessful `git status` in its non-repo
assessment directory. That initial command violated the intake protocol but
obtained no repository contents. The restriction is procedural, not a sandbox.

## Comparison

The fresh revised reader passed **4/4 initial coding attempts**, compared with
**2/4** for the baseline. Both readers passed **4/4 expanded attempts**. All four
revised expanded Rust submissions are byte-identical to their initial attempts;
extra context confirmed their code instead of repairing it. Both initial and
expanded revised t3/t4 submissions also pass the separate concrete-cause and
independent-orientation audits.

Both readers satisfy **4/4 late-metadata criteria** and **4/4 ownership criteria**
on each view. The [rubric and matching excerpts](../benchmarks/guide_learning_rubric_2026-10-25.json)
record these coordinator judgments separately from the executed coding oracles.
The revised tool audit found only supplied-file reads, own-answer existence and
word-count checks, and answer writes; no repository, network or compiler intake.
[Exact coding submissions](evaluation/submissions/README.md) are committed,
including the failures, without formatting or repair.

| Coding exposure | Baseline | Revised |
| --- | ---: | ---: |
| Initial section view | 2/4 | 4/4 |
| After inline expansion, no compiler feedback | 4/4 | 4/4 |

The revised reader ranks registry use least confusing, while positioned decode
is now the largest remaining assembly question. It distinguishes the usable
public ICC guard from optional private coordination. Both readers recommend
keeping config/job, registry and source/driver/sink roles; missing setup does
not justify merging them. Both identify the streaming barrier and whole-job
ownership qualification as production work beyond documentation.

```
revised grading and audits: rc=0 3s | peak-RSS 0.23GiB | min-avail 19185MiB | peak-load 0.44
```

After the measured comparison, the guide names ResourceLimits at its use site
and states CollectSink's full-raster ownership, success-only return and typed
error propagation. The code and frozen comparison packets remain unchanged.
The contrast follow-up changes only the expanded code background. Scores and
intake/height measurements refer to the recorded revisions, not subsequent prose.

## Baseline results

The initial views yielded **2/4** passing coding tasks. Expanded views yielded
**4/4**, without compiler feedback or repair of the preserved submissions.

| Task | Initial | Expanded | What mattered |
| --- | --- | --- | --- |
| Registry detection and dimensions | Compile failure | Pass | Invented root-level codec imports; actual configurations live in zenbitmaps. Dimension accessors were guessed correctly but never taught in the packet. |
| Inclusive pixel limit | Pass | Pass | The chain was visible; setup imports were inferred from earlier packets. Equality semantics were missing from the text. |
| Positioned decode and source failure | Pass | Pass | Earlier signatures made the hidden factory and sink setup inferable. |
| JPEG rewrite and pre-write ICC refusal | Compile failure | Pass | Wrong policy import and invented Plan::write_to. Expansion revealed the actual preset and chunk writer. |

The native checks cover both registered formats, unknown input, exact and
over-limit pixel counts, complete row equality, a failing positioned source,
baseline/progressive JPEG scan equality, independent decoded pixels, removal
of the descriptive fixture, and zero output for refused header/trailing ICC.
They are selected contracts, not complete codec or privacy qualification.
In particular, a stringified source error passes the original source-failure
gate; preservation of its concrete cause needs a separate check.

Both conceptual answers satisfy all four prewritten criteria: **4/4 for late
metadata, 4/4 for backpressure**. Expansion adds no information to these two
sections. These are coordinator rubric judgments, separate from executable
coding results. The reader distinguishes pending absence from final color,
information events from enforceable suspension, output availability from raw
storage release, and a memory ceiling from successful completion.

```
baseline grading: rc=0 2s | peak-RSS 0.22GiB | min-avail 19363MiB | peak-load 0.74
```

This is the grading process receipt, not a codec memory bound. The wrapper
succeeds after recording failed submissions; individual compiler statuses are
in the CSV and complete logs.

## What changed

- The full example is the primary copy action. Independent imports/setup and
  continuation expansion remains in source order; source links stay visible.
- Compact neutral code panels replace repeated dark bars and explanatory
  framing. Build-time Rust grammar highlighting distinguishes calls, types,
  keywords and literals without a browser highlighter dependency.
- Registration names the concrete codec crate, constructors and output
  dimension methods. The pixel-limit lesson states the inclusive boundary.
- JPEG policy selection is in the visible code. A checked public-plan example
  precedes the private resolver adapter, whose internals expand on demand.
- Six samples propagate typed driver errors with `?`. An independent source
  timeout regression first failed on the old samples, then passed after the
  fix. This was a teaching defect, not a missing library error facility.

At a 1440 × 1100 viewport, initial section heights changed as follows. These
include prose, code and signatures, not just the code panel. Rounded display
values are from captured DOM measurements.

| Section | Baseline CSS px | Revised CSS px |
| --- | ---: | ---: |
| Registry entry | 1064 | 708 |
| Pixel-limit chapter | 1978 | 1661 |
| Positioned-I/O chapter | 1787 | 1457 |
| JPEG policy steps 3–4 | 2291 | 1175 |
| Late metadata | 1281 | 1139 |
| Output ownership | 880 | 904 |

The ownership section grows because it now states explicitly that a cap may
require refusal. Less space is useful only when the contract remains clear.

## Where the abstraction belongs

The baseline reader ranked the simple registry task most confusing and least
conceptually complex. Late metadata was least ambiguous in its explanation
but most complex to implement. Those are different problems.

Keep CodecSet, per-request jobs and positioned drivers. The first three tasks
do not establish that those layers should merge. Teach imports, results and
contracts where the operation is introduced; reserve backend associated types
and state-machine construction for the implementor path.

A fixed ICC refusal can use the existing plan directly. A shared resolver earns
its place when requirements span carriers, transcode routes and changing facts.
The caller supplies required effects and a preset or typed fallible callback.
Adapters own question construction, validation, reservations, scoped receipts
and barriers before effects. Requiring callers to construct Prepared or Engine
would move coordination into the public API instead of removing it.

Retire unchecked or duplicative caller coordination only after the replacement
enforces it: capability/check/set/read-back loops, ambiguous best-effort setters,
and streaming booleans used as memory guarantees. Keep optional discovery,
native row/session interfaces, result descriptors, and distinct image/video
ownership. A callback cannot authorize false finality, manufacture a memory
bound, recover published bytes, or silently weaken a requirement.

The production gaps remain [scoped metadata finality and suspension](https://github.com/imazen/zencodec/issues/166),
[checked caller requests](https://github.com/imazen/zencodec/issues/27),
[storage-aware waiting](https://github.com/imazen/zencodec/issues/160) and
[admission across owners](https://github.com/imazen/zencodec/issues/161).
Documentation clarity does not implement these mechanisms.

Raw packets, answers, compiler logs, control submissions and screenshots are
retained under `output/zencodec/guide-learning-2026-10-25/` in the experiment
artifact store. The source definitions, grading code and compact measurements
are in this repository. The [artifact pointer](../benchmarks/guide_learning_artifacts_2026-10-25.json)
records the complete manifest hash, file count and pinned assessment revisions.
No production public API changed in this assessment.

## Supplemental semantic audit

The frozen oracles intentionally remain unchanged. A later audit checks two
contracts separately: the original concrete source-error cause must survive,
and a no-ICC JPEG with EXIF orientation 6 must preserve that orientation while
removing Artist, as read by the independent EXIF reader. Both coordinator
controls pass. The baseline expanded positioned-source submission fails the
cause check because it copied the old sample's string conversion; its JPEG
submission passes the orientation check. Thus 4/4 on the frozen expanded tasks
is not a blanket statement that every production contract passed.

```
semantic audit: rc=0 2s | peak-RSS 0.22GiB | min-avail 19364MiB | peak-load 0.10
```

## Reading the scope correctly

The coding tasks qualify consumer and adapter call sites. They do not measure
writing a new codec implementation, dependency installation from crates.io, a
native video reconfiguration pipeline, or a human beginner's onboarding time.
The guide labels implemented repository interfaces separately from private
prototypes; implemented-on-main does not imply that a release contains them.

The revised JPEG section is shorter initially but longer with every detail
expanded: 4017 versus 3456 CSS px (rounded) at the recorded desktop viewport.
It now includes both a complete usable public example and the optional adapter
experiment. Keeping that extra context available is preferable to hiding the
policy needed for correctness or presenting private machinery as application
setup. The initial coding packets total 1140 words versus 1368 in the baseline,
including link lists. These are measured intake sizes, not reader time estimates.
