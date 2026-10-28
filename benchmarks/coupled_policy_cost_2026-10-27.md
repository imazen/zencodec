# Existing frame-policy cost after coupled conversion offers

This compares `7e4923e2` (before coupled conversion evidence) with `85c33cc8`
(conversion evidence and corrected lossless preference). Both execute the same
`perf/frame-policy-cost.rs` and the same lockfile. The workload publishes frame
facts and maintains a bounded report; it performs no decoding or conversion.

## Fixed-work profiles

`perf stat` captured one million operations per executable/mode, including
process and workload setup. These are single retired-instruction observations,
not elapsed-time ratios or confirmed non-inferiority results.

| Mode | Before | After |
|---|---:|---:|
| Record | 299,756,342 | 351,638,586 |
| Resolve | 414,214,504 | 452,353,520 |
| Combined | 486,255,956 | 523,039,480 |

Ten-million-operation combined profiles captured 474 and 513 cycle samples,
respectively, with zero lost samples. `memmove` self samples were 26.70% before
and 34.36% after. Its callers include report insertion, record construction and
the workload's `black_box`. This identifies copies as a profiling target, not a
proof that one field causes the difference or that a particular rewrite helps.

Heaptrack captured the combined workload at one and one million operations on
both binaries. All four captures recorded 16 allocation calls and printed a
75.46K peak heap. No allocation-count growth with record count was observed.
This includes runtime/workload setup, not just the policy API. It excludes stack
storage, multiple concurrent operations, decoder references and native workspace.
Heaptrack's own elapsed times and overhead-inclusive RSS are not speed or native
memory measurements.

## Exploratory wall comparison

Zenbench pair completed 45 invocations: three modes, before/after/A-A, five
interleaved blocks. Each invocation runs ten million operations and validates
the resulting report. Wall time includes Python/process startup, assertions and
supervisor polling/cleanup. Uniform hardware placement was verified; zero
invocations had sampled CPU load above 5%, but eight had heavy-process
observations. No observation or resource gate was removed.

| Mode | Exploratory paired latency change | A/A change |
|---|---:|---:|
| Record | +6.29% | −5.41% |
| Resolve | +10.73% | −0.11% |
| Combined | +10.66% | −0.02% |

These are zenbench's geometric means of block ratios, not confirmed effects.
The record A/A variation is material. This run cannot certify a speed regression,
an acceptable end-to-end codec overhead, or a performance improvement.

The result supports investigating internal record copies before introducing
heap allocation or changing caller-facing diagnostic access. It does not close
the runtime/memory release gate. The bounded report still retains the latest
16 decisions; no diagnostic or existing test expectation was reduced.

## Reproduction and provenance

[The receipt](coupled_policy_cost_2026-10-27.json) retains source, lockfile,
executable, workload, profiler, plan and packet hashes. All binaries, source
archives and captures are under this observed r7900x artifact root:

`/mnt/v/output/zencodec/platform-regions-20261026-run/policy-coupled-cost-20261027`.

Build each source in a **separate fresh target** with `frame-cost-probe-build`.
An initial shared-target candidate build returned the exact baseline binary;
it was excluded before profiling, then rebuilt in a fresh target. The build
recipe now requires a nonexistent target. Both qualified builds use Rust 1.99.1,
thin LTO, debug level 1 and two jobs, without `target-cpu=native`.

Use `frame-cost-saved-profile` for counters and `frame-cost-saved-sample` for
sampled stacks (CPUs 2,3 here); `frame-cost-saved-heap` records heaptrack captures.
`frame-cost-pair-plan` and `frame-cost-pair-run` use the
[retained-program runner](../perf/frame-policy-pair.py) and zenbench's unchanged
admission/observation machinery. A negative control supplies an invalid mode to
the real probe: the wrapper fails and emits no success token. The initial heap
report lookup assumed `.gz`; this installation writes `.zst`. Its original valid
capture was retained and parsed without repeating the capture.

Resource lines cover complete tool processes, not per-operation memory:

```text
before build: rc=0 28s | peak-RSS 0.45GiB | min-avail 11353MiB | peak-load 0.87
after build:  rc=0 27s | peak-RSS 0.45GiB | min-avail 11215MiB | peak-load 1.04
counters:     rc=0 1s | peak-RSS 0.02GiB | min-avail 11832MiB | peak-load 0.87
heap/stacks:  rc=0 4s | peak-RSS 0.09GiB | min-avail 11657MiB | peak-load 0.48
wall pair:    rc=0 42s | peak-RSS 0.04GiB | min-avail 11749MiB | peak-load 0.69
```
