# Two bounded repaired-source measurement reruns

## Missing and limits first

Confirmed latency, a zenbench cleanliness verdict, full native conformance and the later PR211 changes remain unmeasured. These are external perf diagnostics, including counter overhead. Allocation calls do not establish peak native workspace. The old erased bitmap adapter copies decoded RGB pixels; its difference from native cannot be assigned to trait design.

No requested cells are missing: tiny native completed 180/180 paired cells and 36/36 allocation cells; large synthetic completed 40/40 paired cells. No measured block was dropped or repeated.

The first controller stopped during calibration on an arm-omitting filename collision, before any measured pair or plan. Its record and all 11 counter captures remain. The guarded repair reused the completed native calibration and then ran the frozen plan once. No failed timing was promoted or retried for acceptance.

## Identity and method

Measurement controller source: `02a8aa35de6fe75020a422625087c876ddbd9a35` on review/reimagined-documents ([PR210](https://github.com/imazen/zencodec/pull/210)). Codec/driver implementations remain those of `3bfa754857020586c9d88cbeb11259186b5ea4d1`; no later [PR211](https://github.com/imazen/zencodec/pull/211) changes were imported.

Native probe: retained 739bc0df compiled inputs, unchanged bytes. Synthetic probe: 283510da shape/qualification CLI over unchanged repaired codec/driver code. Dependencies remain locked. No target-cpu=native. Source hashes stayed unchanged during both measured studies.

Host: Ryzen 9 7900X. Both studies acquired the legacy benchmark rendezvous and retained published zenbench exclusive lease; zenbench exec verified a homogeneous performance hard cpuset, CPUs 0-23. run-heavy limited memory to 4G and jobs to 2; Rayon/OMP pools were explicitly 2.

Each process creates the same fixture/runtime and performs one warm decode. Subtract a separately launched zero-extra process from the extra-decode process, then divide by the recorded count. CPU is inherited user + system time across threads. Five blocks use seed 20261111 and shuffled cells with alternating zero/extra order. All counters, negative adjusted values, loads, failures and observations survive.

Calibrate each cell by fixed doubling until adjusted external duration reaches 500 ms; the identical A/A arm uses its native count. Counts freeze before the five blocks. Heaptrack uses independent zero/1,000-extra captures with the same warm operation, summing allocation-cost stack leaves over all threads.

A/A spread below is the maximum absolute native-versus-identical-A/A difference among the five matching blocks, independently for wall and CPU. It is an observed variation bound, not a confidence interval. An absolute contrast no larger than that spread is unresolved. Exceeding it is still diagnostic evidence, not confirmed latency acceptance.

## Measurement 1: tiny native RGB8

Wall / CPU and A/A spread are microseconds per decode. N is extra decodes per process in each of five blocks. Allocation calls are per decode from zero/1,000-extra heaptrack subtraction. Borrowed control uses &NoPulse. AA rows execute the unchanged native arm.

| Codec / size | Arm | N | Wall / CPU µs | Calls / decode | A/A spread wall / CPU µs |
|---|---|---:|---:|---:|---:|
| qoi 8² | native | 5,120,000 | 0.159339 / 0.159306 | 1 | 0.000630 / 0.000624 |
| qoi 8² | native_probed | 5,120,000 | 0.161632 / 0.161603 | 1 | 0.000630 / 0.000624 |
| qoi 8² | borrowed_dyn | 5,120,000 | 0.164140 / 0.164116 | 1 | 0.000630 / 0.000624 |
| qoi 8² | next_slice | 2,560,000 | 0.246588 / 0.246480 | 2 | 0.000630 / 0.000624 |
| qoi 8² | older_erased | 2,560,000 | 0.296095 / 0.296037 | 5 | 0.000630 / 0.000624 |
| qoi 8² | native_AA | 5,120,000 | 0.158951 / 0.158923 | 1 | 0.000630 / 0.000624 |
| qoi 63² | native | 80,000 | 6.908578 / 6.901925 | 1 | 0.150855 / 0.125012 |
| qoi 63² | native_probed | 80,000 | 6.913703 / 6.912618 | 1 | 0.150855 / 0.125012 |
| qoi 63² | borrowed_dyn | 80,000 | 6.893283 / 6.891845 | 1 | 0.150855 / 0.125012 |
| qoi 63² | next_slice | 80,000 | 6.977954 / 6.976370 | 2 | 0.150855 / 0.125012 |
| qoi 63² | older_erased | 80,000 | 8.518794 / 8.517252 | 5 | 0.150855 / 0.125012 |
| qoi 63² | native_AA | 80,000 | 6.895011 / 6.893772 | 1 | 0.150855 / 0.125012 |
| bmp 8² | native | 5,120,000 | 0.183088 / 0.183059 | 1 | 0.004062 / 0.004072 |
| bmp 8² | native_probed | 2,560,000 | 0.228479 / 0.228433 | 1 | 0.004062 / 0.004072 |
| bmp 8² | borrowed_dyn | 5,120,000 | 0.188785 / 0.188739 | 1 | 0.004062 / 0.004072 |
| bmp 8² | next_slice | 2,560,000 | 0.316922 / 0.316877 | 2 | 0.004062 / 0.004072 |
| bmp 8² | older_erased | 2,560,000 | 0.329938 / 0.329878 | 5 | 0.004062 / 0.004072 |
| bmp 8² | native_AA | 5,120,000 | 0.184418 / 0.184386 | 1 | 0.004062 / 0.004072 |
| bmp 63² | native | 320,000 | 2.003363 / 2.002746 | 1 | 0.032887 / 0.032378 |
| bmp 63² | native_probed | 320,000 | 2.041380 / 2.041066 | 1 | 0.032887 / 0.032378 |
| bmp 63² | borrowed_dyn | 320,000 | 1.999968 / 1.999509 | 1 | 0.032887 / 0.032378 |
| bmp 63² | next_slice | 320,000 | 2.131722 / 2.131281 | 2 | 0.032887 / 0.032378 |
| bmp 63² | older_erased | 160,000 | 3.639416 / 3.638354 | 5 | 0.032887 / 0.032378 |
| bmp 63² | native_AA | 320,000 | 1.985931 / 1.985630 | 1 | 0.032887 / 0.032378 |
| jpeg 8² | native | 160,000 | 4.471244 / 4.470280 | 30 | 0.062013 / 0.062806 |
| jpeg 8² | native_probed | 160,000 | 4.945356 / 4.944603 | 32 | 0.062013 / 0.062806 |
| jpeg 8² | borrowed_dyn | 160,000 | 4.517041 / 4.516228 | 30 | 0.062013 / 0.062806 |
| jpeg 8² | next_slice | 160,000 | 5.020212 / 5.018995 | 33 | 0.062013 / 0.062806 |
| jpeg 8² | older_erased | 160,000 | 5.122000 / 5.121110 | 36 | 0.062013 / 0.062806 |
| jpeg 8² | native_AA | 160,000 | 4.480154 / 4.479270 | 30 | 0.062013 / 0.062806 |
| jpeg 63² | native | 40,000 | 19.446874 / 19.443020 | 30 | 2.463406 / 2.466825 |
| jpeg 63² | native_probed | 40,000 | 18.733625 / 18.727285 | 32 | 2.463406 / 2.466825 |
| jpeg 63² | borrowed_dyn | 40,000 | 18.414778 / 18.411090 | 30 | 2.463406 / 2.466825 |
| jpeg 63² | next_slice | 40,000 | 18.856636 / 18.853370 | 33 | 2.463406 / 2.466825 |
| jpeg 63² | older_erased | 40,000 | 18.897977 / 18.894235 | 36 | 2.463406 / 2.466825 |
| jpeg 63² | native_AA | 40,000 | 18.494031 / 18.490205 | 30 | 2.463406 / 2.466825 |

Derived differences, in **nanoseconds** per decode. Each bullet gives wall / CPU, followed by the corresponding A/A spread and classification. The boxed-step contrast includes admission, sink/protocol checks and dispatch as well as its box; it does not isolate a single allocation.

- **qoi 8²:** boxed_step_path: +84.956 / +84.877 ns; A/A 0.630 / 0.624 ns; wall exceeds observed A/A spread; diagnostic only, CPU exceeds observed A/A spread; diagnostic only; extra_header_probe: +2.293 / +2.298 ns; A/A 0.630 / 0.624 ns; wall exceeds observed A/A spread; diagnostic only, CPU exceeds observed A/A spread; diagnostic only.
- **qoi 63²:** boxed_step_path: +64.251 / +63.752 ns; A/A 150.855 / 125.012 ns; wall unresolved, CPU unresolved; extra_header_probe: +5.126 / +10.693 ns; A/A 150.855 / 125.012 ns; wall unresolved, CPU unresolved.
- **bmp 8²:** boxed_step_path: +88.443 / +88.444 ns; A/A 4.062 / 4.072 ns; wall exceeds observed A/A spread; diagnostic only, CPU exceeds observed A/A spread; diagnostic only; extra_header_probe: +45.391 / +45.374 ns; A/A 4.062 / 4.072 ns; wall exceeds observed A/A spread; diagnostic only, CPU exceeds observed A/A spread; diagnostic only.
- **bmp 63²:** boxed_step_path: +90.342 / +90.215 ns; A/A 32.887 / 32.378 ns; wall exceeds observed A/A spread; diagnostic only, CPU exceeds observed A/A spread; diagnostic only; extra_header_probe: +38.017 / +38.320 ns; A/A 32.887 / 32.378 ns; wall exceeds observed A/A spread; diagnostic only, CPU exceeds observed A/A spread; diagnostic only.
- **jpeg 8²:** boxed_step_path: +74.856 / +74.392 ns; A/A 62.013 / 62.806 ns; wall exceeds observed A/A spread; diagnostic only, CPU exceeds observed A/A spread; diagnostic only; extra_header_probe: +474.112 / +474.323 ns; A/A 62.013 / 62.806 ns; wall exceeds observed A/A spread; diagnostic only, CPU exceeds observed A/A spread; diagnostic only.
- **jpeg 63²:** boxed_step_path: +123.011 / +126.085 ns; A/A 2463.406 / 2466.825 ns; wall unresolved, CPU unresolved; extra_header_probe: -713.249 / -715.735 ns; A/A 2463.406 / 2466.825 ns; wall unresolved, CPU unresolved.

## Measurement 2: 1024×1024 RGBA8, 16-row strips, quota 64

Means across all five shuffled blocks. Times are microseconds per decode; CPU sums worker threads. These process-profile means are not uninstrumented latency estimates or a confirmation verdict.

| Route | N | Wall µs | CPU µs | Kernel µs | Context switches / decode |
|---|---:|---:|---:|---:|---:|
| direct | 8,000 | 96.791 | 96.777 | 0.232 | 0.001375 |
| older_tokio_inline | 4,000 | 164.411 | 164.318 | 0.666 | 0.003200 |
| next_tokio_inline | 8,000 | 97.770 | 97.738 | 0.179 | 0.003275 |
| older_tokio_pool | 1,000 | 740.835 | 746.023 | 448.430 | 132.040400 |
| next_tokio_pool | 1,000 | 837.390 | 843.251 | 465.253 | 132.036200 |
| next_buffered_one_hop | 8,000 | 107.489 | 107.569 | 5.718 | 2.002800 |
| older_read_at | 8,000 | 99.215 | 99.192 | 0.001 | 0.002200 |
| next_read_at | 8,000 | 96.375 | 96.361 | 0.193 | 0.001250 |

The shortest retained measured extra-operation loop was 546.925 ms. The receipt includes per-cell minima. Raw summary.json files retain ranges and all measured blocks.

## Commands and resources

Executed from reimagined/:

```sh
flock --timeout 300 "${XDG_CACHE_HOME:-$HOME/.cache}/zenbench/zenbench.lock" /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/binaries/lease /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/binaries/executor exec --cpu-policy performance python3 -- tools/native-slice/bounded-rerun.py tiny-native /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/binaries/native /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/tiny-native
flock --timeout 300 "${XDG_CACHE_HOME:-$HOME/.cache}/zenbench/zenbench.lock" /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/binaries/lease /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/binaries/executor exec --cpu-policy performance python3 -- tools/native-slice/bounded-rerun.py tiny-native /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/binaries/native /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/tiny-native --resume-calibration
flock --timeout 300 "${XDG_CACHE_HOME:-$HOME/.cache}/zenbench/zenbench.lock" /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/binaries/lease /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/binaries/executor exec --cpu-policy performance python3 -- tools/native-slice/bounded-rerun.py large-synthetic /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/binaries/synthetic /mnt/v/output/zencodec/reimagined-rerun-2026-11-12/large-synthetic
```

The initial tiny-native command stopped in calibration; the resume command continued that same study. No extra measured study was launched.

```text
build.log: run-heavy: done rc=0 3s | peak-RSS 0.20GiB | min-avail 13108MiB | peak-load 0.55
build-final.log: run-heavy: done rc=0 3s | peak-RSS 0.20GiB | min-avail 12936MiB | peak-load 0.21
clippy.log: run-heavy: done rc=0 8s | peak-RSS 0.32GiB | min-avail 12572MiB | peak-load 0.80
tiny-native.log: run-heavy: done rc=1 4s | peak-RSS 0.03GiB | min-avail 13092MiB | peak-load 1.85
tiny-native-resume.log: run-heavy: done rc=0 203s | peak-RSS 0.10GiB | min-avail 11970MiB | peak-load 1.84
large-synthetic.log: run-heavy: done rc=0 47s | peak-RSS 0.02GiB | min-avail 13074MiB | peak-load 1.82
```

These wrapper peaks cover the profiled process trees, not a codec workspace bound.

Frozen binary SHA256:

```text
native db2587fa3f9adca51dd116eb98cfac71911fc26166d572a96397024c8b54b3b2
synthetic 9ceb081d066956fb0a6bf662024e7268d417786153e5aba9afc6090d54f1bcd5
lease a239eb1b3f6fa0c652c43e7415228c0092086274d1d1f083eef119b9602fa052
executor da3358cc46a622194c63e5664aa666eeaa42e2cbe620e84ae217fb63287d3ccc
```

Raw artifacts: /mnt/v/output/zencodec/reimagined-rerun-2026-11-12

Sources, locked dependency identities and complete process counters are retained in run.json, binaries.json, the native prior receipts and the captured logs. Original calibration failure and counters remain in run.before-calibration-repair.json and the original tiny-native log.

The requested two studies are complete. No merge, codec migration, new design work, browser measurement or optional page update was performed.
