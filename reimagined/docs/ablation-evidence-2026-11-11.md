# What the older design earned, and what can be ablated

The useful simplification is to remove repeated mechanisms, hidden copies and
unnecessary handoffs while keeping the obligations their tests established.
A low trait count is not performance evidence. A fast unchecked decode cannot
stand in for checked metadata, bounded ownership or source-version admission.

This investigation covers the older main surface at `119f3133` and the
reimagined document candidate. PR208/209 are reviewed alternatives, not measured
native replacement cohorts. The fresh harness compares the older root drivers
and the reimagined drivers in one binary. Native codec migrations, framework
rewrites and releases are outside this phase.

## Decision ledger

| Responsibility / mechanism | What the evidence establishes | Decision for the reimagining |
|---|---|---|
| Borrowed resident-input path | Existing slice driver avoids input buffering. The memcpy fixture exposes copying costs directly. | Retain a borrowed slice route. Do not send resident data through an async read buffer merely for API uniformity. |
| Row streaming and output draining | Native streaming rollout retained byte parity and recorded throughput gains on several PNG/PNM workloads. It also caught duplicate conversion/copy work. | Retain streaming where native work benefits. Allow a provided one-shot adapter for codecs that need whole images. |
| Tokio execution strategy | Historical benchmarks include both copies and worker handoffs. Fresh controls distinguish inline, batched pools and one buffered handoff. | Keep execution outside core; choose inline/batched/resident paths by workload. Do not make every strip cross a pool boundary. |
| Work quota and backpressure | Work quota is independent of output production; cancellation/publication tests require boundaries even when no bytes are produced. | Retain a scheduling quota. Its value is a tuning choice, not an output-buffer size or cancellation-latency guarantee. |
| Coroutine authoring | The existing helper copies into coroutine storage and delivers rows through retained handoffs. The synthetic comparison measures that path. | Keep optional for parser/control flow. It must not be the mandatory pixel path, and it cannot replace a synchronous Read/Seek stack without an explicit adaptation cost. |
| Typed versus erased entry paths | Type hierarchy alone does not allocate. Stack-erased and heap-boxed dispatch are different experiments. | Prefer one semantic protocol; retain a cheap borrowed/direct adapter if useful. Do not duplicate the public protocol family just to avoid a box. |
| Config/job/preparation layers | Shared configuration, retained controls and native preparation are separate lifetime responsibilities. Retained preparation reduced observed allocation calls; runtime non-inferiority was not fully established. | Consolidate constructors and provided helpers; retain one actual prepared native owner and its controls. Never probe one owner then silently reconstruct another for execution. |
| Mandatory requirements versus callbacks | Successful historical resolver paths allocated zero times in the measured workload. Refusal/native controls show that callbacks cannot weaken inherited limits or select unproved strict outcomes. | Present one application policy entry. Preserve bound requirements, explicit unknown evidence and refusal before effects. Resolve at operation/interpretation boundaries rather than per row. |
| Bounded reports and diagnostic records | Fixed-work profiling found material memmove activity. The evidence does not isolate one field or confirm an end-to-end codec regression. | Remove repeated copies/duplicate facts internally. Preserve typed reasons and bounded history; fewer diagnostic facts require a separate correctness decision. |
| Pixel storage and interpretation | Canonical encoding/grid/owner experiments preserve tested sample bits, borrowed addresses, signed registration and owner lifetime. They do not establish allocation or speed equivalence for every codec. | Consolidate duplicate representations and owner/mapping mechanisms around zenpixels. Keep storage, encoding, color/alpha, sampling grid and presentation as distinct facts. |
| Retained collections and native selections | TIFF directory/ROI witness and scoped-ID controls show that repeated browsing needs retained identity and native geometry. | Keep Document, exact level/ROI/channel identity, source revision and registered geometry. Containment, resolution and presentation cannot be collapsed into one numbered frame. |
| Source/range admission | Existing regressions caught prefetch rereads, allocation after consumption and stale/partial publication. Range reuse improved actual native transport read counts. | Retain positional requests, bounded prefetch/alternatives and exact source failures. The driver can share one planner; it must not impose a full download. |
| Error/cancellation/publication guards | Native and adversarial controls caught swallowed errors, success after cancellation, missing rows and output after finalization. | Retain these guards at the actual trust/effect boundary. Consolidating their implementation is reasonable; deleting their guarantees to improve the positive benchmark is not. |
| Native lifetime and thread constraints | Main collection factories permit an opened collection/source to borrow and need not make the native operation Send. The new core currently requires Send + static owners; the synthetic benchmark cannot qualify that restriction. | Put worker Send/static requirements on adapters that offload, unless a codec cohort proves the stronger core restriction acceptable. Do not silently discard thread-affine or borrowing backends. |
| Independent output-storage ownership | Retained images/clones must keep their resource charges, and replaced color may be released while auxiliaries remain. | Keep accounted lifetimes and owner groups. A plain Vec or an input-window cap is not a replacement for that contract. |

These are recommendations for this fork, not approval to delete established
APIs, change defaults or weaken tests. Structural consolidation and performance
qualification remain separate axes.

## What the archived numbers can and cannot decide

- [Driver overhead v2](../../zencodec-io/benchmarks/driver_overhead_v2_2026-10-14.md)
  records resident slice, buffered pump, coroutine and Tokio paths. Its M4 Pro
  placement is explicitly exploratory; pool-thread allocations are excluded from
  its per-thread allocation column. It does not measure generic Tokio overhead
  or this fork.
- [Streaming rollout](../../benchmarks/streaming_regress_2026-10-14.md) retained
  like-for-like encoded bytes and found repeated conversions/copies in native
  PNG. Its timing is exploratory and includes code-placement controls. The
  unchanged-code variation must not become a universal regression threshold.
- [Policy resolver cost](../../benchmarks/policy_cost_2026-10-25.md) records
  successful direct/closure/borrowed-erased decisions at 89.5/90.2/94.2 ns and no
  operation-stack allocation calls in that workload. This is historical local
  evidence, not a new measurement or permission to resolve policy per row.
- [Coupled frame-policy profiling](../../benchmarks/coupled_policy_cost_2026-10-27.md)
  and [selected-image profiling](../../benchmarks/image_policy_cost_2026-10-26.md)
  locate copies as an investigation target. Their flagged/inconclusive timings
  do not authorize deleting facts, reports or policy boundaries.
- [Retained decode timing](../../benchmarks/retained_decode_runtime_2026-10-29.json)
  is invalidated. The [later ARM reuse run](../../benchmarks/retained_reuse_runtime_2026-10-29.json)
  has clean recorded observations but three inconclusive comparisons and one
  equivalent comparison; it does not establish non-inferiority across all cells.
  The [heaptrack comparison](../../benchmarks/retained_reuse_heap_2026-10-29.json)
  does establish fewer recorded preparation allocation calls on its frozen inputs.
- [Core compile cost](../../benchmarks/compile_core_2026-10-29.md) compares whole
  graphs with different pixel versions and functionality, with observer flags.
  It cannot assign a compile-time cost to Config/Job traits or prove a gain from
  object-safe dispatch. Fresh matched consumer builds remain a separate question.
- [Frame/plane ablation](../../docs/frame-plane-ablation.md) and the linked native
  controls support semantic consolidation, while explicitly leaving broader
  native/auxiliary adoption and performance open.

## Fresh benchmark boundary

Harness: [tools/ablation](../tools/ablation/README.md), frozen source
`718bf91b65ac46b9c0074a8f5e83321cd13b5698`. The old core/I/O/coroutine inputs are
this workspace's unchanged main files; the reimagined inputs are the document
candidate plus the Tokio module-wiring correction. The two pixel revisions and
all dependencies are pinned in its standalone lock. Both use the same resolved
Tokio version in one executable.

All cells use authored SYN1 RGBA8 noise/patch bytes and the same Vec-backed sink.
The sink writes active rows and preserves padding. Decoder construction, output
allocation, copying and drop are timed. Fixture generation, checksum/stride
qualification and runtime construction are excluded. No native compression,
network waits, CMS, checked policy, output leases, document discovery or rendering
is timed. The whole APIs are not semantically interchangeable.

Fourteen arms cover a direct A/A control, older typed/stack-erased/boxed slice
routes, the new boxed slice route, positioned reads, the older coroutine, both
inline Tokio drivers, both pool drivers and the new resident one-hop route.
Three image/strip shapes are joined by a quota sweep at 1, 64 and unlimited
steps. Every timed path is checked for exact active samples and completion before
measurement. A separate control verifies that the translated decoder updates its
requests immediately after consuming bytes; stale requests would distort I/O.

The captured reimagined Tokio file was not declared in lib.rs. Its earlier
all-feature gate did not compile it. The wiring correction now compiles and
executes these positive paths; it does not qualify future-drop cancellation,
transcode publication or async document driving.

The fresh fixed plan uses zenbench 0.1.11, five processes, 30 rounds per group,
95% family confidence and its unmodified resource gates. Auto placement resolves
to uniform hardware on the observed Ryzen 9 7900X. Frequency is unverified.
Every process and every flag is retained. Raw comparisons to the direct arm are
mechanism costs with different work, not release-regression verdicts for equal
functionality.

## Fresh results and limits

The fixed timing run completed all five processes and all 900 paired rounds.
**Confirmation is invalid**: 177 rounds had resource flags (162 heavy-process,
15 CPU-load observations), and all five processes are marked unreliable. No
process or round was removed, no gate was relaxed, and the run was not repeated
until it happened to pass. Source hashes stayed unchanged. The observed flags
lack process identities; a later process listing cannot identify their cause.

The following are descriptive means across all five processes, including the
flagged observations. They locate promising mechanism questions, not confirmed
speedups or regressions:

| 1024×1024 RGBA, 16-row strips | Mean µs |
|---|---:|
| Direct | 97.117 |
| Older typed slice | 98.537 |
| Older stack-erased slice | 99.745 |
| Older boxed slice | 99.696 |
| New boxed slice | 98.771 |
| Older resident-capable ReadAt | 99.586 |
| New buffered ReadAt | 232.608 |
| Older Tokio inline | 167.428 |
| New Tokio inline | 234.060 |
| Older Tokio pool | 640.869 |
| New Tokio pool | 732.788 |
| New resident one-hop | 110.818 |

The 256×256 one-row quota sweep records new pool means of 1,638.586 µs at
quota1, 91.071 µs at quota64 and 69.349 µs at unlimited. The resident one-hop
route records 19.587/18.080/17.644 µs in those groups. This compares different
scheduling/residency choices; it does not justify removing fairness or streaming.
All timings remain subject to the invalid confirmation above.

A source-verified distinction explains the ReadAt paths: the older driver's
`source.as_slice()` fast path routes resident input to the borrowed slice
loop. The new ReadAt trait has no resident borrow, so its driver allocates and
fills a window even when the concrete source is a byte slice. The new explicit
slice route is already fast in this workload. Preserve that fast-path choice;
API uniformity alone must not introduce a copy.

### Allocation calls: separate heaptrack measurement

The separate release probe profiles all threads. For each arm it executes the
same warm operation, then captures a zero-operation process and a process with
1,000 additional operations. The table is the exact difference in total
allocation-cost flamegraph leaves. It includes the output Vec and runtime
activity that scales with work; it is not a native workspace or latency bound.

| Arm, 256×256 / 4-row / quota64 | Additional calls in 1,000 operations |
|---|---:|
| Direct | 1,000 |
| Older typed slice | 1,000 |
| Older stack-erased slice | 1,000 |
| Older boxed slice | 2,000 |
| New boxed slice | 2,000 |
| Older resident-capable ReadAt | 1,000 |
| New buffered ReadAt | 4,000 |
| Older coroutine | 12,000 |
| Older Tokio inline | 3,000 |
| New Tokio inline | 5,000 |
| Older Tokio pool | 21,013 |
| New Tokio pool | 11,013 |
| New resident one-hop | 3,000 |

The typed and stack-erased older paths have identical allocation-call growth.
Boxing adds observed allocation calls independently of vtable dispatch. Both
boxed slice APIs have the same growth here. The new pool route records fewer
calls than the older pool route, while its inline and generic ReadAt paths record
more. Thus there is no evidence for a blanket claim that the smaller API is
cheaper. Preserve the new session ownership idea, and investigate mandatory
boxing/input-buffer routing before adding more layers.

The profiler's distinct operation-count controls include background runtime
setup; the pool differences are deliberately reported as 21,013 and 11,013,
not rounded into an exact per-decode invariant. Captures, reports, stack leaves
and binary hashes are retained. No per-operation memory figure was inferred
from allocation counts.

### Receipts and reproduction

[Small committed receipt](../benchmarks/ablation_2026-11-11.json) retains scalar
results, source/binary identities and all quality flags. Full raw artifacts,
profiler captures and preserved probe binary live at
`/mnt/v/output/zencodec/reimagined-ablation-2026-11-11/`. The timing binary is
preserved there as well; the exact source/lock fingerprint is in `confirm/run.json`.
The runtime Git metadata resolves the enclosing zen checkout, so it is not used
as build provenance; use the explicit jj revision and SHA-256 manifest instead.

```text
# Fixed five-process timing study; invalid, not promoted
run-heavy: done rc=2 162s | peak-RSS 0.16GiB | min-avail 10628MiB | peak-load 1.60
# Heaptrack capture and analysis across all 13 arms
run-heavy: done rc=0 4s | peak-RSS 0.09GiB | min-avail 10469MiB | peak-load 0.07
```

These resource lines cover the complete validation/profile commands, not codec
memory. The benchmark/data collection phase stops here: it establishes a
repeatable comparison and concrete allocation/routing observations, while clean
native timing confirmation remains open. Fixing the host observer or running a
cross-host native campaign is a separate phase.

## Proposed migration order

First simplify names/entry points and share internal execution while keeping the
existing conformance behavior. Next preserve resident and streaming fast paths
through the single protocol, with scheduling chosen at the driver boundary.
Then consolidate sample/owner representations under native bit/stride/lifetime
controls. Migrate richer policy, storage and document adapters against their
actual semantic acceptance matrix before considering retirement of older types.
No trait removal follows merely from a faster synthetic decoder.

## Published resident-input repair

The forward import from PR211 (`ba8f7082`) adds `ReadAt::as_slice` and
`AsyncSource::as_slice`, routes blocking and inline resident sources through
borrowed sub-slices, and reuses initialized sequential-window storage. The
existing document driver and its `read_next` access remain intact. The imported
blocking/async controls, existing document controls, doctests and strict workspace
Clippy pass. These checks establish behavior on the fixture codecs, not a native
codec performance result. The earlier timing/allocation receipt still describes
the pre-repair source; measurements below must identify their own source.

The independent resident-input control confirms pointer identity through the
slice, positioned-slice and inline Tokio drivers, including ranges and yields.
It also fails on the imported PR211 source: a driver-only Pulse raised during a
step could be ignored until the next batch, or entirely on `Done`. The repaired
slice runner checks the async Pulse before and after each successful native step,
keeping native errors primary. Its synchronous instantiation supplies a no-op
closure. Direct assignment of a zero public step quota now makes progress rather
than yielding forever. These are driver-boundary checks; they do not preempt a
native call.

## Resident repair and native trials

The [readable API/cost page](resident-input.html) and
[machine-readable receipt](../benchmarks/resident_input_2026-11-11.json)
record the follow-through. The repaired synthetic source is `e9066b12`; the
native trial source is `739bc0df`. Both timing studies retained unchanged source
through execution. Earlier receipts still describe their original compiled inputs.

The native trial uses actual QOI/BMP decoding from zenbitmaps `9d71b356` and
JPEG from the retained [PR260](https://github.com/imazen/zenjpeg/pull/260)
cohort at `6d6539df`. Two benchmark-only bitmap accessors expose the existing
QOI parser and move out its private pixel Cow. The source verifier compares all
184 exported blobs with the pinned Git tree and permits only those two exact
additions. No native kernel or sibling checkout changed. The methods and
remaining limits are in [the trial README](../tools/native-slice/README.md).

Every native route preserves dimensions, descriptor and sample bits for 8, 63
and 256 square RGB8 witnesses. Native output pointers survive ownership
extraction and PixelBuffer adoption. A separate `&[u8]` / `&dyn Pulse` entry
point accepts a stack-borrowing observer whose `share()` refuses, reaches native
checks, and retains native timeout causes. JPEG reports progress through it;
the raw bitmap APIs qualify cancellation only. These success-path frontends are
not full codec ports or policy/admission replacements.

Heaptrack reports these additional calls over 1,000 native decodes (256 square
RGB8, same warm operation in both processes, all threads). Borrowed cost arms
use `&NoPulse`; the reporting observer is a separate behavioral control:

| Route | QOI | BMP | JPEG |
|---|---:|---:|---:|
| Native | 1000 | 1000 | 39000 |
| Native plus a separate header probe | 1000 | 1000 | 41000 |
| Borrowed typed, `&dyn Pulse` | 1000 | 1000 | 39000 |
| Borrowed dyn, `&dyn Pulse` | 1000 | 1000 | 39000 |
| Reimagined slice | 2000 | 2000 | 42000 |
| Reimagined resident ReadAt | 2000 | 2000 | 42000 |
| Reimagined inline Tokio | 2000 | 2000 | 42000 |
| Existing erased one-shot | 5000 | 5000 | 45000 |

The new loop improves on the older erased binding, but still requires a decoder
box; JPEG's separate probe adds two calls per operation. The borrowed dyn control
matches native allocation counts. This is allocation-call evidence, not latency,
peak live heap, total process memory or a qualified native workspace bound.
It does not establish equivalent metadata/policy surfaces.

The repaired synthetic paths also changed as predicted: resident ReadAt fell
from 4000 to 2000 calls per 1000 decodes, and inline Tokio from 5000 to 3000.
The older typed and stack-erased paths remain at 1000, boxed old/new slice paths
at 2000. Pool counts include runtime startup effects; the complete exact counts
remain in the receipt, including the non-rounded 11013 new-pool calls.

Both new timing runs are **inconclusive**, with no process dropped or gate
relaxed. The repaired synthetic run completed five processes and 900 rounds;
196 rounds were flagged (182 heavy-process, 14 CPU-load). The native run
completed five processes and 1350 rounds; 325 were flagged for heavy processes.
Every native process is unreliable. Descriptive means are retained in the
receipt but cannot authorize performance acceptance. No repeated search for a
favorable run was performed.

Resource-wrapper receipts:

```text
synthetic timing: rc=2 171s | peak-RSS 0.16GiB | min-avail 10831MiB | peak-load 1.57
synthetic heaptrack: rc=0 4s | peak-RSS 0.09GiB | min-avail 10917MiB | peak-load 0.95
native timing: rc=2 131s | peak-RSS 0.10GiB | min-avail 11024MiB | peak-load 1.19
native heaptrack: rc=0 9s | peak-RSS 0.10GiB | min-avail 10611MiB | peak-load 0.39
```

The local resident path should borrow both the file and `&dyn Pulse`, without
requiring a box, runtime or owned control. Keep the range/streaming mechanism and
retained document semantics as useful mechanisms, and preserve checked admission
and publication obligations. An object-safe resident override need not recreate
the old parallel typed/dyn hierarchy. The cost control is not yet that complete
API: `DecodeSpec` still stores SharedPulse, `WholeImageDecode::decode` has no
Pulse argument, and a `'static` Decoder cannot retain a native parser borrowing
caller input between steps. Do not remove the existing fast path until the
borrowed replacement meets those obligations and has valid latency evidence.

Raw identities, rejected timings, source fingerprints, all-thread captures and
logs live at `/mnt/v/output/zencodec/reimagined-resident-2026-11-11/`, with the
receipt identifying their source and binary hashes. Full codec conformance,
malformed-input behavior, color/orientation/metadata policy, workspace bounds,
document-driver borrowing and nonresident worker copies remain outside this gate.

## Wall and process CPU accounting

The [CPU/wall receipt](../benchmarks/wall_cpu_2026-11-11.json) adds external
`perf stat` accounting over the **same retained probe binaries**, whose SHA-256
identities match the allocation studies. No codec or measured loop was rebuilt.
The accounting orchestrator is `25514d68`. Five fixed, shuffled blocks cover 24
cases, including unchanged native and synthetic A/A controls. Each case pairs a
process performing 10,000 extra direct in-memory decodes with a separate process
performing zero extra decodes; both create the same fixture/runtime and perform
one warm decode. All 240 captures and 120 adjusted pairs survive.

CPU in these tables means process user plus system CPU, including worker
threads. Perf inherited task-clock is retained separately; it is not silently
substituted for the user/system total. Wall, user and system setup costs are
subtracted separately. Small negative adjusted system-time values remain in the
data as independent-control noise; they are not physical negative work.

Mean microseconds per native 256-square RGB8 decode, **wall / CPU**:

| Codec | Native | Borrowed dyn + NoPulse | Reimagined boxed | Existing erased |
|---|---:|---:|---:|---:|
| QOI | 112.13 / 112.11 | 112.07 / 112.04 | 112.37 / 112.35 | 136.91 / 136.89 |
| BMP | 28.16 / 28.15 | 28.17 / 28.16 | 28.40 / 28.39 | 52.81 / 52.80 |
| JPEG | 187.47 / 187.43 | 186.57 / 186.54 | 186.68 / 186.65 | 186.85 / 186.82 |

These are profiler means, not confirmation or an API-equivalence result. The
identical JPEG native A/A control averages 185.80 wall / 185.78 CPU, while the
other native control averages 187.47 / 187.43. That variation is larger than
the small differences among the borrowed/boxed/old JPEG bindings. The complete
per-case ranges and all raw blocks remain available; no favorable subset is used.

Synthetic 256-square RGBA8, four-row strips, step quota 64; microseconds/decode:

| Route | Wall | CPU total | Kernel CPU |
|---|---:|---:|---:|
| Direct | 6.20 | 6.20 | -0.03 |
| Old inline Tokio | 11.42 | 11.42 | -0.04 |
| New inline Tokio | 6.45 | 6.44 | -0.02 |
| Old Tokio pool | 68.06 | 69.28 | 42.13 |
| New Tokio pool | 65.74 | 65.96 | 40.96 |
| New resident one hop | 15.01 | 15.06 | 7.34 |

Worker handoffs consume substantial kernel CPU on this workload. New pool and
one-hop routes record means of 12.00218 and 2.00094 context switches per decode.
CPU totals can exceed wall time because threads can execute concurrently. These
observations support making worker scheduling optional; they do not quantify
streaming uploads, other batch sizes, concurrent workloads or native codec pools.

The complete diagnostic holds the published zenbench exclusive lease and runs
through verified `zenbench exec` CPU placement: homogeneous detected class,
hard cpuset CPUs 0–23. It uses run-heavy with two jobs and a 2 GiB ceiling.
The first launcher rejected Auto policy before running the program; the next
attempt refused an output directory before launching any probe. Both logs are
preserved, followed by the one complete fixed plan. No data was removed to gain
acceptance. The diagnostic has no cleanliness/latency acceptance verdict, and
does not replace or repair the earlier rejected Confirm runs.

```text
run-heavy: done rc=0 98s | peak-RSS 0.03GiB | min-avail 10634MiB | peak-load 0.95
```

Raw perf counters, schedules, startup controls, binary copies and command logs
live at `/mnt/v/output/zencodec/reimagined-wall-cpu-2026-11-11/`. The receipt
distinguishes wall, user, system, total CPU and task-clock rather than inferring
CPU usage from wall time or allocation counts.

Perf counter collection itself adds overhead, particularly for paths with many
context switches. The process-profile wall means are therefore kept separate
from the earlier zenbench means and are not uninstrumented latency estimates.
