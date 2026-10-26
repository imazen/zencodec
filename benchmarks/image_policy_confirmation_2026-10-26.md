# Policy-history confirmation after CPU coordination

The three saved executables and sampling plan are unchanged from the
[initial cost investigation](image_policy_cost_2026-10-26.md). This measures
policy-history machinery, not native decoding or conversion.

Neither additional attempt qualifies a timing comparison:

| Attempt | Baseline rounds with rejected resource observations | Outcome |
|---|---:|---|
| Coordinated idle sessions, normal benchmark process priority | 41 / 450 | Zenbench exit 2; candidates not started |
| Six identified idle interfaces temporarily suspended; default run-heavy priority | 72 / 450 | Zenbench exit 2; candidates not started |

The fixed plan still requires five independent processes, 30 rounds per group,
a 1% margin and 95% family confidence. No resource threshold was changed. The
normal-priority attempt retained its cgroup memory cap. The second attempt used
[`quiet-command.py`](../perf/quiet-command.py), with a deadline and pidfd watchdog;
all six interfaces were verified resumed afterward. No scheduler or other system
service was stopped. Diagnostic-only tracing additionally observed brief CPU
activity from other idle interfaces and the scheduler. It does not prove that
those observations are harmless, or identify every interferer in the timed run.

```text
run-heavy: done rc=2 24s | peak-RSS 0.08GiB | min-avail 13185MiB | peak-load 0.45
run-heavy: done rc=2 25s | peak-RSS 0.08GiB | min-avail 13168MiB | peak-load 0.81
```

## Fixed-work instruction profiles

`perf stat` separately measured one million operations per executable and mode,
including process and workload setup. These are instruction counts from one
capture per cell, not speed ratios or confirmed non-inferiority results.

| Mode | Before image facts (`7d72a8c7`) | Image facts (`419d5c19`) | Facts stored once (`fe5f5205`) |
|---|---:|---:|---:|
| Record | 320,257,047 | 349,379,967 | 235,222,213 |
| Resolve | 440,138,094 | 520,377,583 | 435,171,489 |
| Combined | 476,263,172 | 563,423,678 | 477,341,462 |

The candidate performs fewer retired instructions than the image-facts version
in all three measured workloads. The combined count remains above the earlier
baseline. This supports the existing decision to retain shared diagnostic facts
once; elapsed-time impact and current-head cost still need qualified measurement.
Cycles, cache misses and context switches are retained in the raw profiles, but
the rejected timing runs cannot establish a speed claim.

```text
run-heavy: done rc=0 0s | peak-RSS 0.02GiB | min-avail 13255MiB | peak-load 0.19
```

The [receipt](image_policy_confirmation_2026-10-26.json) binds commands, executable
hashes, plans, profiles, logs and invalid result files. The retained artifact
directories on `r7900x` are
`/mnt/v/output/zencodec/platform-regions-20261026-run/image-policy-cost/confirm-coordinated`
and `confirm-idle-interfaces`; resolve them with `ls` before reuse. Progress files
record failure and completed/required populations, rather than leaving these
attempts labeled running. Resource receipts describe entire profiling processes,
not per-operation heap allocation or codec workspace.

`just quiet-command-controls` exercises restoration after success, command
failure, timeout and parent SIGKILL using only a test-owned sleeping process.
The guard requires explicit, already-coordinated PIDs and must not be used to
suspend active work or bypass a live scheduling claim.
