# Selected-image facts and existing frame-policy cost

The selected-image policy addition increases the cost of the existing frame-only
history workload. This experiment compares the immediate pre-change revision
`7d72a8c740d28ed2d112dcb3e3758114a9c3ddb7` with
`419d5c19c715a412b54a811523081c718295c24d`, using the same locked dependencies and
unchanged `perf/frame-policy-cost.rs`. It measures no pixel decode or conversion.

Both binaries use Rust 1.99.1, thin LTO and debug level 1, without
`target-cpu=native`. Runs used CPUs 2,3 on a Ryzen 9 7900X. Zenbench reported
exploratory `UNVERIFIED_UNRESTRICTED` placement and unverified frequency. These
are median-of-three-pass means and inter-pass MAD, not formal non-inferiority
results. Each row has 60 rounds; each iteration constructs and returns a report.

| Presentations | Operation | Before image facts | With image facts |
|---|---|---:|---:|
| 1 | record | 112 ± 0 ns | 135 ± 0 ns |
| 1 | resolve | 115 ± 1 ns | 151 ± 1 ns |
| 1 | combined | 126 ± 1 ns | 164 ± 1 ns |
| 16 | record | 480 ± 5 ns | 609 ± 10 ns |
| 16 | resolve | 709 ± 3 ns | 911 ± 19 ns |
| 16 | combined | 752 ± 3 ns | 1037 ± 18 ns |
| 256 | record | 6.4 ± 0.1 µs | 8.1 ± 0.0 µs |
| 256 | resolve | 10.1 ± 0.2 µs | 13.4 ± 0.1 µs |
| 256 | combined | 10.8 ± 0.1 µs | 15.3 ± 0.3 µs |

Single fixed-work `perf stat` observations for one million record operations,
including process and workload setup:

| Counter | Before image facts | With image facts |
|---|---:|---:|
| instructions | 319,761,888 | 348,982,182 |
| cycles | 138,210,010 | 174,333,674 |
| cache misses | 32,994 | 31,972 |
| context switches | 1 | 1 |

A separate ten-million-operation combined capture collected 562 samples with
none lost. `memmove` accounts for 28.20% of sampled cycles. Its callers include
record construction/return, the harness's `black_box`, and report insertion.
This supports investigating record copies; it does not attribute the whole
regression to one field or establish an end-to-end codec cost.

The candidate stores selected-image facts once, but the older frame and boundary
selections repeat facts already present in `DecisionContext`. The next experiment
can retain those facts in the context and keep only the selected effect in
`Selection`, matching selected images. No diagnostic fact needs to be dropped.

## Evidence and reproduction

Full artifacts, binaries, counters, sampled stacks and content-addressed
zenbench objects are under
`/mnt/v/output/zencodec/platform-regions-20261026-run/image-policy-cost/` on
`r7900x`, observed 2026-10-26. Resolve the directory with `ls`; verify its
`SHA256SUMS`. [The receipt](image_policy_cost_2026-10-26.json) records source,
lockfile and executable hashes. `baseline-run` and `image-run` are the compared
wall-clock runs. An earlier baseline run in the checkout is preserved but
excluded because its runtime checkout metadata identifies the candidate revision.

Build with `just frame-cost-build` in isolated source exports. Save both binaries
before building the other revision, then run serially from separate evidence
directories using the resource wrapper and a deadline:

```sh
just --justfile /path/to/zencodec/justfile --working-directory /fresh/evidence \
  frame-cost-saved-bench /saved/benchmark 2,3
just --justfile /path/to/zencodec/justfile --working-directory /fresh/evidence \
  frame-cost-saved-profile /saved/probe /fresh/evidence/record.perf.csv record 1000000 2,3
```

Resource-wrapper receipts include the complete harness/profiler process. They
do not measure per-record heap usage or native decoder workspace:

```text
# baseline wall clock
run-heavy: done rc=0 14s | peak-RSS 0.08GiB | min-avail 13991MiB | peak-load 0.34
# selected-image wall clock
run-heavy: done rc=0 15s | peak-RSS 0.08GiB | min-avail 13807MiB | peak-load 1.35
# baseline counters
run-heavy: done rc=0 0s | peak-RSS 0.02GiB | min-avail 13781MiB | peak-load 1.11
# selected-image counters
run-heavy: done rc=0 0s | peak-RSS 0.02GiB | min-avail 14009MiB | peak-load 1.00
# selected-image sampled stacks
run-heavy: done rc=0 1s | peak-RSS 0.02GiB | min-avail 14092MiB | peak-load 0.73
```
