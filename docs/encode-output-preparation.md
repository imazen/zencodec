# Output planning must precede native startup

The checked request-to-destination path is still incomplete. Eagerly beginning
every candidate encoder before offering a configuration/schedule is not an
acceptable implementation: it moves native allocation ahead of admission and
keeps every candidate's native state alive during the policy callback.

## Native evidence

The [probe](../perf/jpeg-policy-validation/src/bin/encode_preparation.rs) uses
2048×2048 RGB8 sRGB declarations, with no source raster, pixel reads, or output
publication. Each `prepare`/`construct`/`begin` process retains either one or four
jobs simultaneously. Their quality targets are 75 through 78. The independent
`refuse` case uses the default quality, one encoder, a zero workspace requirement,
and a source/writer that panic if called; refusal happens at the policy boundary.

The native dependency is zenjpeg `4535ec499d88b04f58882df03e61761526c35756`.
The debug binary is identified in the [complete measured data](../benchmarks/encode_preparation_2026-10-27.json).

| Native configuration | Stage | Candidates | Peak heap bytes |
|---|---|---:|---:|
| Fixed-table sequential | Prepared jobs | 1 | 75,487 |
| Fixed-table sequential | Prepared jobs | 4 | 77,479 |
| Fixed-table sequential | Constructed encoders | 1 | 84,185 |
| Fixed-table sequential | Constructed encoders | 4 | 110,697 |
| Fixed-table sequential | Begun encoders | 1 | 1,836,421 |
| Fixed-table sequential | Begun encoders | 4 | 7,118,213 |
| Fixed-table sequential | Refused workspace requirement | 1 | 1,925,732 |
| Progressive, optimized tables | Prepared jobs | 1 | 75,493 |
| Progressive, optimized tables | Prepared jobs | 4 | 77,485 |
| Progressive, optimized tables | Constructed encoders | 1 | 84,191 |
| Progressive, optimized tables | Constructed encoders | 4 | 110,703 |
| Progressive, optimized tables | Begun encoders | 1 | 14,573,843 |
| Progressive, optimized tables | Begun encoders | 4 | 58,070,699 |
| Progressive, optimized tables | Refused workspace requirement | 1 | 14,663,988 |

These are whole-process heap peaks from allocation events, including startup
and bookkeeping. They are not RSS, native-only totals, elapsed-time benchmarks,
or bounds for other dimensions/configurations. Reservations need not fault every
page. No peak is extrapolated to a different image size or codec.

Heaptrack's timestamp-sampled Massif export missed these short-lived peaks.
The runner now replays every interpreted allocation/free event and cross-checks
the result against `heaptrack_print`'s independently reported rounded peak. The
event semantics follow the [upstream analyzer](https://github.com/KDE/heaptrack/blob/master/src/analyze/accumulatedtracedata.cpp).
The first sampled summary is marked invalid alongside its preserved raw data;
the committed data comes from the corrected full rerun.

```text
profile: rc=0 3s | peak-RSS 0.14GiB | min-avail 8726MiB | peak-load 0.15
native contracts + lint: rc=0 1s | peak-RSS 0.53GiB | min-avail 8869MiB | peak-load 0.12
```

## The existing native plan is not yet the adapter plan

[Executing controls](../perf/jpeg-policy-validation/src/preparation.rs) show
that `JpegEncoderConfig::inner().resolve_plan()` returns identical plans before
and after `with_prefer_streaming(true)`, while the actual row encoders differ.
The wrapper applies its effective configuration later. At heights 32 and 64,
the preferred configuration streams; at height 8 it correctly refuses a
header-only streaming claim. Explicit progressive/native settings override the
streaming preference.

Therefore the common planning hook must describe the retained **effective job**,
including checked fidelity, policy, density, native overrides and source
interpretation. Reading a native template's plan from outside the adapter is
not sufficient.

## Required contract before the destination API lands

```text
request.prepare(codecs)
    → retain lightweight jobs
    → plan actual input + destination access without native pixel/workspace startup
    → admit mandatory requirements and choose a coupled route
    → drop unselected jobs
    → start the selected job once
    → validate the promised route before source reads / output publication
    → execute the selected input and output protocols
```

- Planning must distinguish row push, borrowed raster and replay requirements.
  Output byte availability does not prove the input can be supplied once.
- Bind width, height, descriptor and current color context, plus forward versus
  seek-back destination access. Revalidate retained source/destination facts
  after application callbacks.
- Unknown native workspace, retained output-wait memory or privacy evidence
  cannot satisfy a hard requirement. Refuse before the unadmitted native startup;
  a successfully checked pixel-storage pool is not a native allocation bound.
- Retain the selected prepared job. Do not reconstruct it, silently alter its
  configuration, or begin all candidates to discover their costs.
- The codec's start result must agree with the plan before any affected output.
  Cancellation, late source changes and native startup errors keep their causes;
  failure must not trigger a fallback or a second encode.
- Use the same requirement resolver through preparation and execution. Do not
  add a second independent preset in the destination options or weaken the
  request when adapting it to the row driver.

The method name and public plan representation remain under implementation
review. This evidence rules out eager `begin` as the planning primitive; it does
not yet qualify a replacement hook across JPEG, PNG, video, or multipass codecs.
No zenpixels changes were needed for these probes. This does not establish the
final cross-codec zenpixels requirement.

Reproduce with `just jpeg-preparation-build`, `just jpeg-preparation-check`, then
`just jpeg-preparation-profile EXECUTABLE NEW_OUTPUT_DIRECTORY SOURCE_REVISION`
in the prepared standalone export, under the normal resource wrapper. Raw and
interpreted captures are preserved under the run directory; the compact JSON
contains capture hashes. Track admission in [issue 173](https://github.com/imazen/zencodec/issues/173)
and output scheduling in [issue 160](https://github.com/imazen/zencodec/issues/160).

## Native preflight implementation experiment

The [pinned native patch](../perf/jpeg-policy-validation/native-preflight.patch)
adds an experimental effective-job primitive inside zenjpeg:

```rust
let job = config.job();
let timing = job.plan_rows(width, height, descriptor, current_color_context, access)?;
// Retain this exact job; a shared checked adapter must freeze it across policy.
let mut encoder = job.encoder()?;
let observed = encoder.begin_output(width, height, descriptor, access)?;
assert_eq!(timing, observed); // The eventual adapter must return an error here.
```

The patch uses the actual adapter configuration, metadata handling and native
builder. Geometry normalization and streaming eligibility are shared with
startup. It performs no native pixel/coefficient startup; cloning configuration
and preparing metadata can still allocate. It returns byte-availability timing,
not privacy evidence, a workspace bound or permission to apply backpressure.
This method is an experiment in an isolated source export, not a shipped method
on zencodec or the pinned unpatched zenjpeg dependency.

The [executing controls](../perf/jpeg-policy-validation/src/native_preflight.rs)
compare planning with actual startup over 952 combinations: 17 configurations,
two widths, seven heights, RGB/gray input and forward/seek-back destinations.
They cover 4:4:4, 4:2:2 and 4:2:0, progressive/sequential, fixed/optimized Huffman,
streaming preferences, and native RGB/XYB overrides. Repeated planning preserves
the same strided encode bytes. Invalid dimensions, canvas mismatch, resource
limits, oversized EXIF, contradictory current color context and cancellation
refuse during planning. The existing native policy suite and independent JPEG
reader checks also pass with the patch applied.

```text
native preflight + existing native suite + strict lint + probe build:
rc=0 17s | peak-RSS 0.69GiB | min-avail 8303MiB | peak-load 0.66
```

The replacement public contract is still open: timing alone omits input replay
requirements, total memory evidence and effect-specific policy obligations.
The checked adapter must own/freeze the planned job and current source facts;
calling a query and then mutating the job is not an admitted execution route.
Target-quality search, MPF output and other codecs are not qualified here.
No zenpixels source changes were required; current descriptor and ColorContext
types suffice for this JPEG timing experiment.

Reproduce from a synchronized standalone framework export:

```sh
just jpeg-preflight-setup EXPORT PINNED_JPEG_REPOSITORY NEW_NATIVE_DIRECTORY
# In EXPORT, using its patched native dependency:
just jpeg-preflight-check
just jpeg-policy-check NEW_ORACLE_DIRECTORY
just jpeg-preflight-build
# In the source checkout (retains the exact patch and profiling runner):
just jpeg-preflight-profile EXPORT/perf/jpeg-policy-validation/target/debug/encode_preparation NEW_CAPTURE_DIRECTORY SOURCE_REVISION
```

Run these commands under the resource wrapper. Setup verifies current framework
sources, exports the immutable native revision, checks/applies the patch, and
retains the resolved graph and patch identity. It never edits the native working
checkout. The patch replaces the disabled optional sibling zenpredict path with
its pinned git source to make the isolated export resolvable; that feature is
not part of this qualification.

### Allocation measurement of the implementation

The [corrected event-level profiler](../perf/jpeg-policy-validation/profile-preparation.py)
measured the implemented patch with direct native jobs at the default quality,
2048×2048 RGB8, retaining all jobs/encoders simultaneously. These cases use the
same configuration within each row, with no registry or quality alternatives.
The [complete results](../benchmarks/native_encode_preflight_2026-10-27.json)
include the binary and native patch hashes.

| Configuration | Jobs | Prepared peak heap | Planned peak heap | Begun peak heap |
|---|---:|---:|---:|---:|
| Fixed-table sequential | 1 | 75,464 B | 75,464 B | 1,835,642 B |
| Fixed-table sequential | 4 | 76,686 B | 76,683 B | 7,120,068 B |
| Progressive, optimized | 1 | 75,464 B | 75,464 B | 14,573,930 B |
| Progressive, optimized | 4 | 76,692 B | 76,689 B | 58,072,178 B |

Whole-process peaks include startup and argument strings; equal peaks do not
prove zero planning allocations. In particular, planning clones configuration
and prepares metadata, and this measurement has no supplied metadata. It shows
that this implementation avoids the measured native pixel-workspace startup;
it does not establish a general constant-memory guarantee or a workspace limit.
No source pixels were read and no encoded bytes were published. This is not a
wall-clock performance measurement.

```text
profile: rc=0 2s | peak-RSS 0.13GiB | min-avail 8608MiB | peak-load 0.49
```
