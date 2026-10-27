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
