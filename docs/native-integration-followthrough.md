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
