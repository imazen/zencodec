# Policy resolution cost

Measured source: `5ab9bc5107eef9c06396c1c7c8d457c910e89847`.
Profiler/report script: `abbb960c9e522e8a036efc070320bd9358e27e67`.
Rust 1.99.1, zenbench 0.1.11, Linux x86_64, AMD Ryzen 9 7900X (`r7900x`).
Release profile uses thin LTO and debug information; no `target-cpu=native`.

## Workload and limits

Each iteration creates an operation, issues identities, validates three authored
alternatives, resolves one execution question, and maintains a bounded diagnostic
journal. All successful arms select the same alternative. Trait objects borrow
stack values; their construction does not allocate. There is no codec work,
metadata parsing, I/O, global storage reservation, concurrent contention or
production logging in this workload. It measures more than the indirect call.

The refusal arms intentionally perform different work: expected decline,
an application error with an eight-byte payload through error erasure, or no
admissible alternative. They are not equivalent workloads for ranking speed.
The [common implementation](../perf/io-validation/src/policy_benchmark.rs) asserts
the selected index or error outcome.

## Observations

The timing column is zenbench's median-of-three-passes mean; MAD is inter-pass
spread, not a confidence interval. The run had 60 rounds, including 20 noisy
rounds. Affinity was requested with `taskset -c 2,3` and the final thread mask was
`[2,3]`; zenbench labeled placement `UNVERIFIED_UNRESTRICTED` and did not verify
frequency. Treat these as exploratory local observations, not a regression gate
or evidence that one dispatch form is universally faster.

| Path | Mean ns | MAD ns | Allocation calls in 10,000 operations |
|---|---:|---:|---:|
| Direct configured preset | 89.5 | 0.1 | 0 |
| Closure delegating to preset | 90.2 | 0.6 | 0 |
| Borrowed resolver trait object | 94.2 | 0.8 | 0 |
| Error-erasing adapter, success | 90.6 | 0.8 | 0 |
| Expected decline | 92.8 | 0.7 | 50,000 |
| Erased application error | 118.3 | 0.4 | 60,000 |
| No admissible alternative | 81.1 | 0.1 | 30,000 |

Heaptrack observed 15 process-startup allocation calls in the zero-operation
control. Each successful 10,000-operation process had the same 15 total calls
and none under the operation stack. Failure counts above are attributed to
demangled operation stacks, excluding startup; positive error controls verify
that symbol attribution works. Reports preserve the native trace and bounded
rejection evidence. This is an allocation-call observation, not an operation RSS
bound, an OOM-survival guarantee, or a claim about arbitrary application callbacks.

The result supports resolving at operation/epoch boundaries without making every
successful decision allocate. It does not justify invoking policy per row,
packet, allocation or writer stall. Failure reporting still allocates; qualify
allocation-failure behavior before promising diagnostic survival under exhaustion.

## Reproduction and evidence

In the [prepared export](../perf/io-validation/README.md#prepare-an-export):

```sh
just io-policy-cost-build
just io-policy-bench 2,3
just io-policy-allocations /path/to/new-capture-directory
```

The allocation script needs heaptrack, heaptrack_print and rustfilt. It preserves
the executable, captures, allocation flamegraph stacks and full reports. Timing
and profiling ran serially, under the resource wrapper with a 12 GiB cap and four
build jobs. These receipts cover each whole command, including profiler overhead;
they must not be interpreted as per-operation memory:

```text
# Wall-clock benchmark
run-heavy: done rc=0 8s | peak-RSS 0.06GiB | min-avail 17276MiB | peak-load 0.65
# Heaptrack captures
run-heavy: done rc=0 1s | peak-RSS 0.09GiB | min-avail 18298MiB | peak-load 1.48
# Demangled stack analysis
run-heavy: done rc=0 0s | peak-RSS 0.05GiB | min-avail 16672MiB | peak-load 0.90
```

[Timing log](policy_cost_2026-10-25.log) and
[allocation counts](policy_allocations_2026-10-25.json) are committed. Larger raw
artifacts are at `/mnt/v/output/zencodec/policy-2026-10-25-r7900x/` on `r7900x`,
observed 2026-10-25; resolve with `ls` and verify `SHA256SUMS` there. The directory
contains both executables, all three zenbench evidence references and compressed
objects, heaptrack raw captures/reports/stacks, and complete command logs.
The zenbench objects' embedded executable hashes match the saved benchmark.

| Executable | SHA-256 |
|---|---|
| `policy-bench` | `50b1df909fe0e79c065ebcd5fa5763de75aef725e65b4ff18d4a5c50d8c2ffc8` |
| `heaptrack/policy_probe` | `42f566fb1b8293afb2a6eae0d8bc7762816385133a34a8c168cb698e62faa7a1` |

Zenbench object identities, in pass order:

```text
0b2c17e144f8b131373ab41c4a405af37706ee2379e34361eb98d439f2dbd990
294c2471687a4db7bce26716972e6f4b007d4de5334a8ce1b767ff24b56a39b8
56dcb6c3f7850b1a8cc7b24fc089262565ee799c554d4467b34c458a6b8580be
```
