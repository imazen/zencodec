# Actual frame-policy and decision-history cost

This measures the shared `resolve_frame` and `OperationReport` implementation,
not the earlier private policy prototype. Baseline: `2a30127303ebaf28a7faaeeb0ac67346e329b2ac`.
The revision containing this report replaces full-array rotation with a circular
history; the public methods and latest-16 retention contract stay unchanged.

## Workload and limitations

`perf/frame-policy-cost.rs` constructs one 10-bit YUV420 interpretation with
1920×1080 geometry, then varies presentation timing and packet provenance.
It allocates no image raster and runs no decoder. `record` constructs and appends
real frame records; `resolve` invokes the shared preset resolver and consumes its
result; `combined` resolves and appends. Each benchmark iteration creates and
returns a report, including in the resolve-only control. The controls validate
retained facts, chronology and omitted-record counts before timing.

Rust 1.99.1, release/bench thin LTO and debug level 1, AMD Ryzen 9 7900X,
Linux x86_64. No `target-cpu=native`. Task affinity was CPUs 2,3, also reported
by zenbench. This is zenbench's exploratory `UNVERIFIED_UNRESTRICTED` mode:
frequency was not verified. Values are median-of-three-pass means, with
inter-pass MAD, not a confirmed non-inferiority result. The passes run within
one process. Small differences in the controls should not be interpreted as
independent algorithm improvements.

| Presentations per iteration | Operation | Rotating history | Circular history |
|---|---|---:|---:|
| 1 | record | 112 ± 1 ns | 111 ± 0 ns |
| 1 | resolve | 114 ± 0 ns | 114 ± 0 ns |
| 1 | combined | 125 ± 1 ns | 126 ± 3 ns |
| 16 | record | 570 ± 1 ns | 478 ± 6 ns |
| 16 | resolve | 683 ± 2 ns | 702 ± 2 ns |
| 16 | combined | 782 ± 5 ns | 750 ± 1 ns |
| 256 | record | 68.3 ± 6.0 µs | 6.4 ± 0.1 µs |
| 256 | resolve | 9.8 ± 0.0 µs | 10.0 ± 0.1 µs |
| 256 | combined | 69.8 ± 4.6 µs | 10.8 ± 0.1 µs |

Each row has 60 rounds across the three passes. These are batch times, not
per-frame decode times. Full codecs, realistic application callbacks,
multi-threaded ticket contention, architectures other than this CPU, and
report formatting cost were not benchmarked here.

## Profiler evidence

One million record-only iterations, including fixed process/workload setup:

| `perf stat` counter | Rotating history | Circular history |
|---|---:|---:|
| instructions | 1,870,475,710 | 319,897,311 |
| cycles | 2,286,417,768 | 136,089,959 |
| cache misses | 52,954 | 30,229 |
| context switches | 5 | 0 |

The separate five-million-iteration DWARF sample attributed 89.24% of baseline
sampled cycles to the rotation stack, including the repeated record copies.
The revised sample has no rotation stack. It still includes record construction,
timing normalization, one retained-record copy and the harness's black boxes.
The two captures contain 1,283 and 131 samples respectively; neither lost samples.
Do not treat the smaller revised capture as precise attribution among the
remaining small functions. Counter runs are single observations, not replicates.

## Reproduction and artifacts

Use an owned export outside workspace dependency overrides, with this tree's
locked dependencies. Run serially through the host resource wrapper and a
deadline; set `TMPDIR` to a persistent disk scratch directory.

```sh
just frame-cost-build
just frame-cost-bench 2,3
just frame-cost-profile /fresh/path/stat.csv record 1000000 2,3
just frame-cost-sample /fresh/path/perf.data record 5000000 2,3
just frame-cost-sample-report /fresh/path/perf.data
```

The fixed-work profiler commands refuse to overwrite outputs. `frame-cost-check`
validates all three modes and runs strict Clippy. The added history test compares
every append with an independent recent-history queue across 1,024 records,
checks clone independence, chronological debug formatting and saturated omission
counting. The existing retention and native-frame tests remain unchanged.

Raw evidence is at `/mnt/v/output/zencodec/frame-policy-2026-10-25/` on
`r7900x` (observed 2026-10-25; resolve with `ls`, verify `SHA256SUMS`). It preserves
both benchmark/probe executables, source hashes, timing logs, perf counters and
captures, sampled stacks and all six content-addressed zenbench objects with
their evidence references. Zenbench's embedded executable hashes match the
saved binaries. Its transient attempt files have been moved into the object
store by zenbench; the finalized objects are the retained measurements.

| Executable | SHA-256 |
|---|---|
| baseline benchmark | `f8f0f3de996e635d1dfe3883e0925b5eef95c94fcf79e2071aa65f09a2ab0503` |
| circular benchmark | `5160b5cf4f14e5de6b38537dfe063bfaa4a2e568d066edf82372eb8dcb75525b` |
| baseline fixed-work probe | `8ccc0cf8d59f21d8a5a81f1a5b9764aa74bce145c9cdcbbd241c77ca5cb89e5a` |
| circular fixed-work probe | `2b74c97888fcf9417cbff9713df1951b7adf2aeeae2a2ff056b884701464337f` |

Resource-wrapper receipts cover complete commands, including harness/profiler
overhead. They do not measure codec memory or establish per-frame heap bounds:

```text
# baseline wall clock; three-minute command deadline
run-heavy: done rc=0 15s | peak-RSS 0.07GiB | min-avail 16502MiB | peak-load 0.97
# circular wall clock; same deadline
run-heavy: done rc=0 13s | peak-RSS 0.07GiB | min-avail 16457MiB | peak-load 0.85
# baseline fixed-work counters; two-minute deadline
run-heavy: done rc=0 1s | peak-RSS 0.02GiB | min-avail 16630MiB | peak-load 0.59
# circular fixed-work counters; same deadline
run-heavy: done rc=0 0s | peak-RSS 0.02GiB | min-avail 16598MiB | peak-load 0.72
# baseline sampled stacks; two-minute deadline
run-heavy: done rc=0 3s | peak-RSS 0.03GiB | min-avail 16597MiB | peak-load 0.62
# circular sampled stacks; same deadline
run-heavy: done rc=0 1s | peak-RSS 0.02GiB | min-avail 16604MiB | peak-load 0.72
```

All used a 12 GiB hard process cap and four build jobs. These receipts are
process observations; the implementation's fixed history contains no growing
allocation regardless of the number of presented frames.

## Native and workspace verification

The unchanged native AV1 matrix passes with the circular report: 8/10/12-bit
mono, YUV420/422/444 and identity-matrix GBR, with independent FFmpeg/libdav1d
sample comparison after re-encoding. Refusal, cancellation, typed causes,
packet epochs and caught callback unwinds also pass. Artifacts are under
`/mnt/v/output/zencodec/av1-policy-2026-10-25/journal-ring/`. The first attempt
failed to create that directory before encoding; provisioning the fresh owned
directory allowed the same unmodified test to run.

The all-feature workspace suite, strict policy/media Clippy, no-default-feature
checks and public API snapshots pass. No public snapshot changed. Receipts:

```text
# native AV1 matrix plus strict harness Clippy
run-heavy: done rc=0 15s | peak-RSS 0.87GiB | min-avail 15519MiB | peak-load 0.93
# cargo test --workspace --all-features --locked
run-heavy: done rc=0 14s | peak-RSS 0.41GiB | min-avail 15664MiB | peak-load 1.40
# public API snapshots
run-heavy: done rc=0 4s | peak-RSS 0.26GiB | min-avail 15995MiB | peak-load 1.45
```
