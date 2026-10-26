# Cold consumer compile observations, 2026-10-26

These are **exploratory observations, not a qualified performance comparison**.
Zenbench pair completed all 60 fresh invocations, verified uniform CPU-class
placement and checked successful compilation. Every invocation had an observed
CPU-load sample above 5%; 19 had a heavy-process observation. No observation was
discarded and no gate was relaxed. The retained packet contains the A/A controls.

| Minimal consumer | Published 0.1.26 | Main `a20e4dd3` | API review `219a545a` |
|---|---:|---:|---:|
| Debug, default features | 1.98 s | 2.28 s | 2.61 s |
| Debug, `std` | 1.97 s | 2.31 s | 2.65 s |
| Release, default features | 2.35 s | 2.94 s | 3.56 s |

Cells are medians of five builds. Each build starts with a new empty target,
after dependency fetching; the OS file cache remains warm. The consumer only
re-exports zencodec. Rust 1.99.1, Ryzen 9 7900X, two Cargo jobs, incremental off,
no compiler wrapper or target-cpu override. This excludes codec implementations,
CMS, native validation harnesses and site generation. It does not measure an
incremental application rebuild or the complete deployed transcode stack.

Cargo's zencodec unit timings (also medians) isolate that compilation unit:

| Unit | Published | Main | API review |
|---|---:|---:|---:|
| Debug, default | 0.41 s | 0.70 s | 1.05 s |
| Debug, `std` | 0.41 s | 0.71 s | 1.05 s |
| Release, default | 0.72 s | 1.25 s | 1.90 s |

The main graph adds how-far; the review graph additionally adds spin. All three
resolve zenpixels 0.2.16 and matching versions of the common dependencies.
The measured review changes add observable compiler work in this small consumer;
these data do not identify a particular API or justify a feature split. Source
volume is not a compile-time profile. The earlier October 15 run used another
machine and overlapping work and is not the baseline for this comparison.

`run-heavy: done rc=0 175s | peak-RSS 0.29GiB | min-avail 10942MiB | peak-load 1.52`

This wrapper covers the complete compile experiment. Individual Cargo maximum
RSS comes from `/usr/bin/time`; the [receipt](compile_time_2026-10-26.json) keeps
its medians/ranges separately. Neither is native codec memory.

Reproduce using `just compile-time-prepare ROOT MAIN_SHA IDEAL_SHA`, then
`just compile-time-run ZENBENCH ROOT`, under the resource wrapper. Use a fresh
ROOT. `just compile-time-report ROOT LOG RECEIPT` reads existing evidence only.
The [runner](../perf/compile-time.py) and [reporter](../perf/compile-time-report.py)
preserve source archives, locked graphs, Cargo timings and failed runs.

Artifact root on r7900x:
`/mnt/v/output/zencodec/platform-regions-20261026-run/compile-time-1`.
The receipt records full source revisions, packet/runner/toolchain hashes and
log location. A copy of the measured runner is retained as `runner-measured.py`.
