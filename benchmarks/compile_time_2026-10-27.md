# Cold consumer compile observations, 2026-10-27

These are exploratory observations, not a confirmed performance comparison.
Zenbench completed all 60 fresh builds with A/A controls and verified uniform
CPU-class placement. CPU-load observations exceeded 5% in all 60; 17 had
heavy-process observations. No sample was discarded and no gate was relaxed.
The packet counts heavy processes but does not identify their owners; these
observations do not establish that the host was free of interference.

| Minimal consumer | Published 0.1.26 | Main `a20e4dd3` | API review `85c33cc8` |
|---|---:|---:|---:|
| Debug, default | 1.96 s | 2.29 s | 2.66 s |
| Debug, std | 1.99 s | 2.28 s | 2.69 s |
| Release, default | 2.35 s | 2.90 s | 3.64 s |

Each cell is the median of five builds with an empty unique target directory.
Dependencies are prefetched, the OS file cache is warm, incremental compilation
and compiler wrappers are off, and Cargo has two jobs. Rust 1.99.1 on Ryzen 9
7900X. This minimal consumer re-exports zencodec; it does not compile codecs,
zenpixels-convert, CMS, tests or the guide, nor measure incremental rebuilding.
The review revision includes coupled frame conversion evidence and its lossless
preference correction. All three graphs resolve zenpixels 0.2.16. Main adds
how-far relative to published; review adds spin relative to main.

Cargo's zencodec unit medians isolate that compilation unit:

| Unit | Published | Main | API review |
|---|---:|---:|---:|
| Debug, default | 0.40 s | 0.69 s | 1.08 s |
| Debug, std | 0.40 s | 0.70 s | 1.10 s |
| Release, default | 0.71 s | 1.25 s | 1.98 s |

The observations show additional compiler work in the review API, but do not
attribute it to individual abstractions or establish a controlled regression
against the [earlier measurement](compile_time_2026-10-26.md). They do not justify
an API or feature split without profiling the relevant consumer workload.

`run-heavy: done rc=0 180s | peak-RSS 0.30GiB | min-avail 11360MiB | peak-load 1.98`

This is the experiment wrapper's resource line, not native codec memory. The
[receipt](compile_time_2026-10-27.json) separately retains per-build Cargo maximum
RSS from `/usr/bin/time`, ranges, source revisions and evidence hashes.

Reproduce with `just compile-time-prepare ROOT MAIN_SHA IDEAL_SHA`, then
`just compile-time-run ZENBENCH ROOT` under the resource wrapper. Use a fresh
ROOT. `just compile-time-report ROOT LOG RECEIPT` reads retained evidence only.
The [runner](../perf/compile-time.py) and [reporter](../perf/compile-time-report.py)
retain locked graphs, archives, timing pages, A/A controls and failures.

Artifact root on r7900x:
`/mnt/v/output/zencodec/platform-regions-20261026-run/compile-time-current-20261027`.
The packet is `attempt/packet.json.gz`; the measured runner and toolchain are
retained alongside it. This does not close runtime, operation-memory, complete
application compile-time or conversion-stack cost validation.

## Runner identity correction (2026-10-28)

The original core preparation recipe referenced the mutable checkout script,
while this report expects `runner-measured.py` in the experiment directory.
Preparation now freezes that file before creating the plan, matching the adapter
experiment. Every case hashes the frozen runner and every arm executes it. This
corrects future reproducibility; it neither changes nor upgrades the retained
2026-10-27 observations. A new release-candidate timing run is still required.
