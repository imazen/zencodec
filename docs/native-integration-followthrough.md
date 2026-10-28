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
