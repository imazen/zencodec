# Native integration follow-through

Observation: 2026-10-28. This record distinguishes tested native revisions from
integration experiments. None of these review branches is a published release.

## Native encoding and operation control

The shared declaration hooks let a configured job plan before construction and
receive the current pixel color at encoder startup. The row driver retains that
job, admits the route, and compares the observed startup plan before reading rows
or publishing output. Its signatures and limits are in
[`row_encode`](../zencodec-io/src/row_encode.rs).

The coordinator independently queried these GitHub runs and PR heads:

| Adapter | Tested revision | CI | Review head |
|---|---|---|---|
| JPEG declaration hooks | `af5fd093d8998f35ce8a5d16b0dd36adfb7c7921` | [Success](https://github.com/imazen/zenjpeg/actions/runs/40603221339) | [PR 246](https://github.com/imazen/zenjpeg/pull/246), `e2b4c2d1112ea4a247f7ddc530d0f675f0a37149` |
| PNG declaration hooks | `e2afc5017cd8a08f7bf4df3ef5bb259eba83e225` | [Success](https://github.com/imazen/zenpng/actions/runs/40603224490) | [PR 47](https://github.com/imazen/zenpng/pull/47), same revision |
| Bitmap Pulse migration | `866b088487bacffe6b443df06e7b2344b84ed3e1` | [Success](https://github.com/imazen/zenbitmaps/actions/runs/40603386897) | [PR 34](https://github.com/imazen/zenbitmaps/pull/34), same revision |

JPEG's two subsequent commits change documentation and manifest comments only;
the coordinator inspected the complete comparison. CI success is attributed to
the tested revision, not to an unexecuted run on the later head. Bitmap control
migration does not establish native declaration-hook support.

The normal JPEG dependency graph needs the matching `ultrahdr-core` review pin
`6aa34974fdbb80f2b9a59fa9c8f8110ce47af0a8`. A dependency's own Cargo patches do not
propagate to its consumers. Standalone consumer validation must repeat necessary
patches and inspect the resolved graph; a successful workspace build alone does
not establish one shared trait version.

## Pipeline validation still in progress

The first zenpipe integration batch compiled the new I/O library. Its pipeline
tests failed to compile because the operators require an owned, `'static`
upstream; a slice-borrowing source did not satisfy that requirement. Three further
steps failed while compiling the migrated dispatch path. Repairs are written;
their next execution is a separate gate.

An owned shared-byte source can avoid an additional encoded-input copy, but
retains that input for its lifetime. It is not evidence of bounded streaming
memory. Likewise, a bounded row queue says nothing about native codec workspace.

The run receipt was `rc=1`, 47 seconds, peak RSS 0.63 GiB, minimum available
memory 6308 MiB and peak load 1.53. These are compilation/test resource readings,
not codec throughput or pipeline memory measurements.

The root zenpipe test targets are blocked by an unmigrated zengif dependency.
The combined `stop_and_limits` target requires unmigrated WebP/GIF adapters.
The separate I/O test member does not replace these gates: both remain untested.

## Contract decisions exposed by integration

These are open design items, not promises of implemented support:

| Caller need | Current boundary | Next qualification |
|---|---|---|
| Registry job to planned destination | `CodecSet::encode_job` returns an erased job; `row_encode::encode_job` requires a typed `EncodeJob` whose executor implements `Encoder`. | Exercise a shared erased-job bridge, including native errors and every startup method, before choosing a convenience API. |
| Retain one operation control across owned adapters | `CodecSet::pulse()` exposes a borrowed `&dyn Pulse`. | Check ownership and cancellation/progress identity through the complete pipeline; avoid asking callers to coordinate duplicate controls. |
| Sample meaning through decode, processing and encode | `frame::SampleEncoding` currently duplicates the unpublished zenpixels type. Original source precision is not proof of current sample precision after edits. | Validate the paired pixel release and mutation invalidation before unifying types or admitting lower-depth encoding. |
| Tokio output with policy admission | The planned row driver is synchronous. A blocking bridge is an implementation option, with a retained thread during waits. | Test cancellation, slow writers, bounded queues and failure propagation; compare with a direct async driver before prescribing the bridge. |
| Efficient HTTP region access | Whole-source JPEG/PNG decoding cannot qualify tiled selective fetching. | Run a collection-backed source with a tiled codec, source-version checks and the existing transport simulator. |
| Lower peak memory under slow output | Output timing does not state retained uncompressed memory. | Compare live backpressure with compressed staging for actual codec configurations; include whole-raster and replay acquisition. |

The [implementation record](ideal-api.md) remains the wider scope checklist.
Native video delay/late metadata, TIFF collection packaging, complete workspace
admission, and full-container privacy guarantees are not closed by these native
encoding CI results.

## Second pipeline batch: failures now reach native execution

The subsequent integration batch executed 14 command gates: six passed and eight
failed. The Tokio bridge and modeled HTTP tests passed, as did the I/O library
check/lint and two output/image-job targets. These do not establish whole-pipeline
correctness. The failing gates exposed:

- A JPEG controlled decode overflowing the default test-thread stack, including
  the checked transcode path. Increasing the stack is not the acceptance gate.
- Native JPEG timeout cancellation reported as `Cancelled` instead of `TimedOut`.
- A supplied `Rotate90` orientation without an EXIF blob returning as `Identity`.
- A required output-wait bound refusing without named unmet requirements when
  the `PolicySet` is passed as the callback rather than installed on the operation.
- Remaining example lifetimes/imports and a custom cancellation fixture that
  still implemented the older control interface.

The original failures remain recorded; repairs must pass the same assertions.
Unmigrated GIF/WebP and the root test graph remain separate coverage gaps.

### One requirement set, including diagnostics

The fourth failure has a source-confirmed explanation in
[`policy::choose`](../src/operation/policy.rs): the operation's preset filters
offers before the callback and fills `DecisionRecord::unmet_requirements`.
A second `PolicySet` used as the callback can independently decline a choice,
but `Decision::Decline` carries only the predefined reason and trace. That path
does not populate the named-requirement record. Refusal still happens; the
diagnostic and admission behavior depend on where the same configuration is put.

The intended replacement should bind mandatory requirements once, before route
selection, and let the callback choose among those admitted routes. Tests must
compare direct built-in policy use with a custom callback, retaining requirement
names and typed causes without exposing metadata payloads. This is an API design
item in [issue 173](https://github.com/imazen/zencodec/issues/173), not a reason to
weaken the integration test or silently move the caller's requirements.

## Qualified repairs and paired pixel proposal

JPEG [PR 246](https://github.com/imazen/zenjpeg/pull/246) now contains the
stack-construction and stop-reason repairs at
`3d2c8d5b72036e1a6635a3ce364d56ed524f5f7b`. Independent before/after executables
were built in separate target directories and preserved with SHA-256 hashes.
At a 1 MiB thread stack, the old build aborts and the repaired build passes
[`stack_demand`](https://github.com/imazen/zenjpeg/blob/3d2c8d5b72036e1a6635a3ce364d56ed524f5f7b/zenjpeg/tests/stack_demand.rs):
baseline subsampling variants, progressive and grayscale inputs agree across
one-shot, streaming and controlled decoding. A prior shared-target experiment
was invalid and is excluded from the comparison.

The subsequent broad native tests, two strict Clippy configurations, format
check and fuzz compilation passed. Receipt: `rc=0`, 223 seconds, peak RSS
1.90 GiB, minimum available memory 4898 MiB, peak load 3.02. This is a build/test
resource reading. A preceding broad run exceeded its memory cap; that incomplete
run is not a passing gate or a valid peak-RSS measurement. The consumer pipeline
still needs to rerun against the pushed repair; orientation-only metadata remains
outside this repair's scope.

The paired pixel proposal is
[zenpixels PR 95](https://github.com/imazen/zenpixels/pull/95), with approved
documentation corrections and API snapshots pushed at
`0c96114e81cbecf086952fc75e5bf696e5d81cd9`. It distinguishes source ancestry from
current-sample exactness, carries bounded `ExactDepth` evidence, and invalidates
that evidence on mutable access. Native code-plane conversion handles code depth
and placement explicitly. This remains a review proposal; native encoder adoption
and the coordinated dependency graph are not yet qualified.

Its follow-up checks passed: precision evidence in both tested feature sets,
strict workspace Clippy, the tests whose documentation changed, strict rustdoc,
layout probes, and API snapshot regeneration/checking. The x86-64 structural
comparison against the proposal's base is:

| Type | Base size | Proposal size | Alignment |
|---|---:|---:|---:|
| `PixelSlice` | 48 bytes | 56 bytes | 8 bytes |
| `PixelSliceMut` | 48 bytes | 48 bytes | 8 bytes |
| `PixelBuffer` | 64 bytes | 72 bytes | 8 bytes |
| `CodePlane` | 56 bytes | 64 bytes | 8 bytes |

These are `size_of`/`align_of` results, not heap or process memory measurements.
`ExactDepth` and `Option<ExactDepth>` each occupy four bytes; enclosing alignment
accounts for the larger object-size changes. Other targets and throughput remain
unmeasured. The standalone layout build/run receipts were 0.24 GiB and 0.18 GiB
peak RSS, respectively; those compiler/process readings are separate from the
struct sizes above.

## Third pipeline batch and bounded coroutine delivery

The third pipeline batch ran each test in a separate process: 256 of 264 tests
passed, and 11 of 19 command gates passed. These denominators cover this batch,
not the release requirements. The receipt was `rc=1`, 306 seconds, peak RSS
0.65 GiB, minimum available memory 6107 MiB and peak load 1.99. The remaining
failures include orientation carriage, named policy refusals, the metadata
verification witness, and PNG delivery/cropping. Passing library and transport
tests do not close those failures.

PNG exposed a distinction between a bounded buffer loan and bounded delivery
per decoder step. The coroutine split a large native batch into small sink loans,
but delivered all of them before returning control to a pull consumer. Its queue
could reach the unchanged 64-object cap even though every loan fit.

Core commit `4be34b03` retains the native batch and returns after one sink loan.
The next step delivers the next strip without consuming more input or polling
the producer. The regression uses a one-slot sink, 129 rows, distinct source and
destination strides, provisional/final passes, and a producer that finishes with
rows still pending. Both controls fail against the previous implementation;
all coroutine tests and strict Clippy pass with the repair. Cancellation between
strips remains terminal. The complete test/catalog receipt was `rc=0`, 6 seconds,
peak RSS 0.26 GiB, minimum available memory 6419 MiB and peak load 0.89.

This bounds delivery, not native memory: the pending native batch remains owned
until its last strip is delivered. Handwritten decoders do not acquire this
coroutine-specific guarantee. Native PNG and the complete consumer pipeline must
qualify the change separately. Core CI and fuzz checks passed at `79676215`;
the documentation workflow's native JPEG harness still fails, so Pages deployment
remains blocked even though the guide, TIFF/JXL and AV1 qualification jobs pass.

The checked-policy migration also exposes fields that native adapters currently
refuse. Acceptance must be implemented and tested in those adapters; replacing a
checked setter with the deprecated best-effort setter would restore silent
policy loss. Native capability work and named policy diagnostics are separate
acceptance gates.

The [policy-binding experiment](policy-binding-experiment.md) exercises a concrete
binding outside callback wrappers. Its five controls and strict Clippy pass; it
adds no production API and does not close the deferred-obligation or duplicate
preset-placement gaps.

## Aligned native graph and registry destinations

The native repairs are now pushed on review branches: JPEG `8672e549`, PNG
`5422b6d2`, and bitmaps `9ad4582b`, all using core `79676215`. The JPEG/PNG batch
passed its orientation controls, native suites, strict lint and fuzz builds.
The bitmap all-features build initially selected published zencodec 0.1.26 for
its own dependency despite another graph edge using 0.2.0. Raising the required
floor to 0.2.0 fixed that split. Its separate rerun passed all nine command gates:
`rc=0`, 135 seconds, peak RSS 0.34 GiB, minimum available memory 5817 MiB and peak
load 1.54. These are local qualification results; review-head CI is a separate gate.

[Zenpipe PR 106](https://github.com/imazen/zenpipe/pull/106) publishes the matching
Pulse migration at `874a3856`. Its fourth integration batch passed checked
transcoding, Pulse retention, metadata services, native PNG strip delivery and
crop controls, and direct orientation carriage. File, PNG-to-PNG, disk-spool and
Tokio examples executed. Two policy-placement diagnostic controls and ImageJob
orientation remained failing. Subsequent processing tests exposed orientation-node
composition and resize draining failures, tracked without weakening the pixel
oracle in [zenpipe issue 107](https://github.com/imazen/zenpipe/issues/107).

Core adapter commit `f627b02b` adds `encode_dyn_job` and
`encode_dyn_job_to_destination`. A registry job now enters the same planned row
loop as a typed job. The private executor preserves boxed native causes, and
planning/startup use the retained configured job. Every existing planned-row
control now runs through both typed and erased paths; separate controls check
registry-to-writer execution, forward rejection and seek-back patch application.
The std I/O suite and strict Clippy pass. Test/catalog receipt: `rc=0`, 9 seconds,
peak RSS 0.30 GiB, minimum available memory 6002 MiB, peak load 1.09. Site source
and full browser checks pass (`rc=0`, 34 seconds, peak RSS 0.30 GiB). The new
[code-first journey](https://imazen.github.io/zencodec/code.html#adapter-registry-output)
ships with source-extracted signatures and expandable examples; deployment still
requires the native JPEG harness repair. Native zenpipe adoption of these new
entry points remains a separate gate. The already measured pipeline used the
older pinned core, not these new adapter functions.

The paired pixel proposal's 32-bit fixture-overflow repair is pushed at
`0bdc0db5`. The local i686 conversion suite, including its zero-allocation control,
passes with overflow checks enabled. Full exact-head CI is
[run 40625957534](https://github.com/imazen/zenpixels/actions/runs/40625957534).
No crate has been published by this work.

## Exact-output controls for the memory experiment

The sixth zenpipe batch independently decoded all seven successful memory-matrix
outputs with the `png` crate and compared every sample against the authored
2048 × 2048 RGB8 fixture. All seven match exactly. Effort 2 with live output
refuses at policy admission and produces no output; its refusal-process memory
must not be presented as an encoding result. These controls use the preserved
measured executable, SHA-256
`bdc74846f9dd469fc25a1f003d026bf5ffa3a84cf2883121ba842ab3c1f8ab94`,
and fixture SHA-256
`2d3cb5fd8276d290b1a05b61274b6b81015f7b7202e168291343946ab41b1aab`.

For the buffering configuration (effort 2), heaptrack reports peak heap of
63.82 MB with memory staging and 63.78 MB with disk staging. Corresponding
`/usr/bin/time -v` maximum RSS values are 84,768 and 85,136 KiB. Disk staging
does not remove the native encoder's retained image in this experiment.
For effort 0, live output reports 17.58 MB peak heap and 21,516 KiB maximum RSS,
but heaptrack attributes two full compressed-input allocations to the controlled
decode path. Lower memory than the buffering route does not establish bounded
network streaming. These are individual memory observations, not throughput
measurements or whole-operation upper bounds.

The control batch receipt is `rc=1`, 16 seconds, peak RSS 0.65 GiB, minimum
available memory 4586 MiB and peak load 0.45. Its failure is explicit: the two
ImageJob composition controls still fail (three of five ImageJob tests pass).
The standalone consumer's two tests pass with one shared zencodec and zenpixels
instance after pinning zenpixels `86d0e2d3`, which supplies the guide's
`ConvertOptions::retain_channels` method. Native codec pins remain those in the
preceding section; no adoption of the new erased row driver is implied.

The paired zenpixels proposal at `0bdc0db5` now passes its full CI run linked
above, including i686, Linux ARM64, macOS ARM64 and Windows ARM64. Core
`534ee4ea` likewise passes [CI](https://github.com/imazen/zencodec/actions/runs/40627250712)
and [fuzz checks](https://github.com/imazen/zencodec/actions/runs/40627250793).
Its [documentation workflow](https://github.com/imazen/zencodec/actions/runs/40627250847)
passes guide execution, browser checks, TIFF/JXL and AV1 qualification; native
JPEG qualification still fails and prevents deployment.

## JPEG harness migration: current-core local qualification

The harness now pins the qualified native graph above and calls
`EncodeJob::plan_encode` / `Encoder::begin_declared` directly. The historical
native patch is retained as measurement provenance, but the test runner no longer
applies it. The 952 planning/startup combinations and the existing pixel, refusal,
ownership and cancellation assertions are preserved. Stop-only test fixtures use
the explicit `StopOnly` adapter at Pulse boundaries.

An isolated export of core integration commit `d9c84dff` passed all 47 JPEG
controls and strict all-target Clippy with `native-preflight` enabled, plus the
three preparation-profiler parser tests. Receipt: `rc=0`, 27 seconds, peak RSS
0.65 GiB, minimum available memory 4962 MiB, peak load 1.21. This qualifies the
harness on the current row driver, including the preceding erased-job addition;
it is not a native adoption test of those erased entry points or a performance
measurement. GitHub documentation CI must still pass before deployment is claimed.

The site initially refused the new dependency pin because its generated
integration signature snapshot still named the old zenpipe revision. Re-extracting
from `874a3856` changes the revision only: the displayed checked-transcode
signatures are identical. The snapshot/manifest equality gate remains intact.

## Merged JPEG baseline and documentation qualification

GitHub main at `cf7b051ba9c198ff7b1cd222552e9d17a29b0513` contains the pinned
JPEG `8672e549` commit. The coordinator verified ancestry through GitHub's compare
API. [PR 240](https://github.com/imazen/zenjpeg/pull/240) (controlled decoding),
[PR 245](https://github.com/imazen/zenjpeg/pull/245) (extended JPEG capability),
[PR 246](https://github.com/imazen/zenjpeg/pull/246) (native planning hooks) and
[PR 248](https://github.com/imazen/zenjpeg/pull/248) (metadata feature gate) are
merged. Remaining carrier-policy work must be qualified on that combined main;
the existing harness pin still identifies its tested source exactly.

The combined main's [CI run](https://github.com/imazen/zenjpeg/actions/runs/40630953425)
fails its public API snapshot and all five platform test jobs. Each platform's
explicit extended-JPEG configuration, with other defaults disabled, reaches four
failing gain-map cases in `mpf_bounds`. The failures include the deliberate
metadata-disabled refusal to classify XMP container items. This requires resolving
the feature/test contract; a merged commit is not evidence of a passing matrix.
Fuzz and coverage workflows pass on that same main revision.

Zencodec integration `2fe20905` passes [CI](https://github.com/imazen/zencodec/actions/runs/40635981071),
[fuzz checks](https://github.com/imazen/zencodec/actions/runs/40635981079) and
[documentation qualification](https://github.com/imazen/zencodec/actions/runs/40635981082),
including native JPEG, TIFF/JXL, AV1, executable examples and browser checks.
The deployment job is skipped on the review branch; these results do not claim a
GitHub Pages publication.

## JPEG feature repair and independent policy replay

JPEG main now contains the approved feature-matrix repair at
[`74ce6cee`](https://github.com/imazen/zenjpeg/commit/74ce6ceeebb85f637f5c33709fd4afd2a0a6821f).
The four original gain-map assertions remain enabled with `metadata`; without
that feature, separate controls require `UnsupportedFeature` for the same
fixtures. The API snapshot also includes the merged declaration hooks. The
coordinator verified that this revision is four commits ahead of `cf7b051b`,
with no diverging commits.

Local tests and strict Clippy pass in both metadata configurations, as does the
format check. The final repair-check receipt is `rc=0`, 315 seconds, peak RSS
0.76 GiB and minimum available memory 3515 MiB. The preceding broad batch failed
on a gated-import lint and a separate proposed carrier-policy change; it is not
a passing batch. The lint is repaired. The carrier change remains unmerged:
it refuses an existing test's malformed ICC blob under `strip_all()`. Profile
recognition failure alone must not be treated as proof that discarding color
information is safe. That compatibility boundary needs further qualification.

The independent policy review replayed its original four proposed safety
assertions against core `e8f16683`; only the cancellation argument changed to
Pulse. All four pass, together with nine current-behavior controls. The
coordinator inspected the per-step results and verified that review head
`d420712a` changes none of the tested core, I/O, testkit or root manifest files.
Direct callback selection cannot choose an excluded route, retained choices
keep their original indices, and admission refusals name unmet requirements.
The receipt is `rc=0`, 12 seconds, peak RSS 0.45 GiB, minimum available memory
4213 MiB and peak load 0.97. This is test resource usage, not policy overhead.

The replay still observes three costs with the reference codec and recording
encoder: a 786,432-byte raster requirement mismatch incurs 786,880 requested
allocation bytes before refusal; none of 64 row pushes borrows the original
raster; and an eight-object staging budget refuses after eight one-byte output
chunks. These measurements do not establish native codec memory bounds. The
two-placement diagnostic gap and cross-operation requirement scope remain open;
the [policy-binding experiment](policy-binding-experiment.md) describes the
unqualified ownership boundary.

Core `d420712a` passes [CI](https://github.com/imazen/zencodec/actions/runs/40638206274),
[fuzz checks](https://github.com/imazen/zencodec/actions/runs/40638206291) and
[documentation qualification](https://github.com/imazen/zencodec/actions/runs/40638206353).
These pinned-graph checks do not qualify the all-main consumer graph: shared
CI substitutes zencodec main 0.1.27 for the 0.2 dependency needed by merged JPEG.
Coordinated core integration remains tracked in
[issue 159](https://github.com/imazen/zencodec/issues/159) and
[PR 168](https://github.com/imazen/zencodec/pull/168).

## Pipeline orientation repair and remaining adapter gates

Zenresize main
[`5b4e14e3`](https://github.com/imazen/zenresize/commit/5b4e14e33a1cd49bd9e08287858d2f534526ac25)
fixes stale padding samples during oriented row output. The two regression
controls fail before the repair and pass afterward; all 21 golden tests pass
without regenerating their 24 baselines. Default, no-default library and
`pretty-safe` suites, strict Clippy and formatting pass locally. The full
[CI matrix](https://github.com/imazen/zenresize/actions/runs/40644463135) also
passes on this revision.

The local before/after batch reports `rc=1` because it includes the two expected
pre-fix failures: 157 seconds, peak RSS 0.46 GiB, minimum available memory
3594 MiB, peak load 2.02. Each repaired test/configuration command succeeds.
The receipt is not a throughput or native memory-bound measurement.

The zenpipe adapter stack is pushed at
[`66a5f34b`](https://github.com/imazen/zenpipe/commit/66a5f34b8070007d4573d7d991a632287274c0e6),
on [PR 108](https://github.com/imazen/zenpipe/pull/108). Its existing pins do not
yet include that zenresize fix; the pin change needs a separate consumer replay.
The two policy-placement diagnostic controls remain failing and unchanged.

Orientation support is also inconsistent across zenresize's output methods:
`with_orientation` affects `next_output_row`, while the other row-output
methods remain unqualified for that setting. This is tracked separately in
[zenresize issue 26](https://github.com/imazen/zenresize/issues/26), rather than
being treated as resolved by the padding repair.

SIMD compatibility validation remains pending. Its first batch stopped before
the intended tests because the sibling dependency did not match the committed
lock and a probe requested a tier outside the adapter's configured support.
Matching empty result inventories from failed commands do not establish parity;
the rerun requires both test commands to succeed and nonempty matching outcomes.

### SIMD compatibility qualified on the committed graph

Zenpipe main
[`1b788757`](https://github.com/imazen/zenpipe/commit/1b788757076d239769ead495dab77f67936a5214)
adds the private compatibility helper without moving the committed SIMD
dependency version. Both before and after runs pass 596 tests and retain the
same six ignored documentation examples. All 602 recorded outcomes match;
the ignored examples are not counted as passing coverage. Strict Clippy passes
for x86-64, with library compilation/lint checks for aarch64 and WASM.

Against the newer SIMD source, the helper matches `mul_add_relaxed` bit for bit
on 6,561 vector triples on each of scalar and x86-64-v3. This does not establish
runtime NEON equivalence or qualify the whole sibling-main dependency graph.
The batch receipt is `rc=0`, 260 seconds, peak RSS 0.77 GiB, minimum available
memory 2770 MiB and peak load 6.50; no throughput claim is made.

## Malformed ICC bytes: compatibility is not color evidence

Native source review found that `zenpixels::icc::profile_color_space` reads a
header field after checking only that the first 20 bytes exist. It can return a
color-space name for truncated or structurally inconsistent data. It must not
be used as an ICC validity test. Likewise, a profile that cannot be identified
as a known profile is not thereby safe to discard.

The native lane's lcms 2.17 experiment rejects several malformed fixtures but
opens others, including some inconsistent declared-size/tag-bound cases.
Those observations describe one reader and version. They do not establish that
every reader rejects a given byte sequence or that removing it preserves color.
The proposed generalized malformed-profile stripping exception remains unmerged.

The existing JPEG `encoding_with_policy_strips_metadata` test explicitly uses
the deprecated best-effort setter. Its compatibility expectation remains
unchanged. The next implementation experiment must distinguish that legacy
behavior from checked policy admission, without weakening authoritative
`ColorContext` handling—even when identical ICC bytes also appear in job
metadata. Required controls include setter ordering, failed setter atomicity,
unknown and Display P3 interpretation, and refusal before output. No production
API repair or new ICC validity guarantee is claimed here.
