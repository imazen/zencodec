# Core and I/O/media companion compilation, 2026-10-27

These are exploratory observations, not a confirmed performance comparison.
Zenbench completed 45 interleaved fresh builds with A/A controls and uniform
CPU-class placement. All 45 had CPU-load observations above 5%; six had
heavy-process observations. No samples were discarded or gates relaxed.

| Median of five builds | Main `a20e4dd3` | Review `ec5d5089` |
|---|---:|---:|
| Debug, no requested std | 2.44 s | 2.86 s |
| Debug, available std features | 2.44 s | 2.95 s |
| Release, available std features | 3.19 s | 4.20 s |

The consumer re-exports zencodec, zencodec-io and zencodec-media. **Main media has
no std feature; review media's std feature includes its new output and staging
drivers.** The runner enables std only on packages exposing it and retains every
manifest. This measures the expanded companion surface, not identical supported
operations or an isolated driver regression. No renderer, codecs, CMS, native
harnesses or application monomorphization is included. Incremental rebuild costs
remain unmeasured. The remote main SHA was checked during this run.

Each invocation has a new empty target; dependencies are prefetched and the OS
file cache is warm. Rust 1.99.1 on Ryzen 9 7900X, two Cargo jobs, incremental and
compiler wrappers disabled. Common dependency versions match, including
zenpixels 0.2.16; review adds spin 0.12.3. No dependency source patch is used.

Cargo compilation-unit medians:

| Case / crate | Main | Review |
|---|---:|---:|
| Debug std / zencodec | 0.86 s | 1.37 s |
| Debug std / zencodec-io | 0.16 s | 0.26 s |
| Debug std / zencodec-media | 0.18 s | 0.27 s |
| Release std / zencodec | 1.53 s | 2.55 s |
| Release std / zencodec-io | 0.31 s | 0.58 s |
| Release std / zencodec-media | 0.25 s | 0.52 s |

These units can overlap; do not sum them into wall time. They locate compiler
work at crate granularity, not individual API features. They do not justify
changing the contract or moving a particular method without further profiling.

Per-build maximum RSS is captured by `/usr/bin/time`. Debug std medians are
247880 KiB on main and 305820 KiB on review; release std medians are 257056 and
315848 KiB. These are compiler-process measurements, not codec workspace bounds.

`run-heavy: done rc=0 157s | peak-RSS 0.30GiB | min-avail 12552MiB | peak-load 1.56`

The [receipt](compile_adapters_2026-10-27.json) retains all medians/ranges, source
SHAs, feature manifests, artifact hashes and quality flags. Raw evidence on
r7900x: `/mnt/v/output/zencodec/platform-regions-20261026-run/compile-adapters-20261027-2`.
The first preparation attempt is preserved alongside it: it correctly failed
when requesting main media's nonexistent std feature, before any timed build.

Reproduce with `just compile-adapters-prepare ROOT MAIN_SHA REVIEW_SHA`,
`just compile-time-run ZENBENCH ROOT`, and
`just compile-adapters-report ROOT LOG RECEIPT`. Run preparation and measurement
serially under run-heavy. The runner is frozen into each new ROOT before building;
source archives, lockfiles, Cargo timing pages, A/A observations and failures
remain available. The report reads existing evidence and does not rebuild.
