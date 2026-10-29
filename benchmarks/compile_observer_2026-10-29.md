# Compile observer qualification — 2026-10-29

**Compile-time confirmation remains unqualified.** Zenbench 0.1.11 Confirm
passed the resident-worker resource control, but invalidated every invocation
of the owned short-lived-child control. Repeating the full compile study before
resolving this limitation would not establish clean-host evidence.

| Control | Completed invocations | Unclean during-work observations | Affected invocations |
|---|---:|---:|---:|
| Ten CPU intervals in one process | 15 | 0 | 0 |
| Ten CPU intervals in successive owned child processes | 15 | 151 | 15 |

All child commands succeeded. Both cases retained five blocks, an identical
candidate and A/A control, 1% margins and 95% family confidence. Resident results
were equivalent; child-process results were invalid on resource observations.
No heavy process was reported in either case. No competing workload was injected;
these are resource-classification controls, **not throughput measurements** or
proof that arbitrary foreign work is detected. Aggregate exit status was 2.

Source inspection of published 0.1.11 explains the limitation conservatively:
`src/bin/support/pair.rs` marks leader startup/exit transitions, but
`SystemMonitor::snapshot_excluding_processes` in `src/platform.rs` subtracts
currently live descendants. Its documentation explicitly treats unassignable
CPU from exited workers as potential foreign load. The result is consistent
with that limitation; this experiment does not independently attribute every
OS counter tick or claim globally idle hardware.

Reproduce through the resource wrapper:

```sh
just compile-observer-prepare "$HOME/tmp/owned-worker-control"
zenbench pair "$HOME/tmp/owned-worker-control/plan.json" --mode confirm --dry-run
zenbench pair "$HOME/tmp/owned-worker-control/plan.json" --mode confirm \
  --output "$HOME/tmp/owned-worker-control/attempt"
```

The exact [control source](../perf/compile-observer-control.py), frozen worker
hash, per-phase counts and all unclean CPU-load observations are in the
[receipt](compile_observer_2026-10-29.json). The first plan omitted a required
revision field and was refused before worker execution; it is retained separately.
The corrected control ran on ARM Linux with the published 0.1.11 executable.
Wrapper: rc2, 88s, peak RSS 0.02 GiB, minimum available 14880 MiB, peak load 0.74.

## Portable compile plans

`compile-time-prepare` now writes relative source dependencies and declared input
paths, so a frozen study can move to an eligible measurement host without editing
its inputs. The prepared current-main study retained identical SHA256 for all
1,340 files after relocation; the moved plan passes zenbench Confirm dry-run.
The existing seven runner/report controls pass. `compile-time-confirm` explicitly
selects Confirm and keeps its packet separate from Iterate. The historical report
continues to reject non-Iterate input; no previous observation is reclassified.

The relocated current-main consumer also compiled successfully on ARM using the
frozen lock and offline runner; the target was removed by the existing cleanup.
That single build is a portability smoke check, not a compile-time comparison.
Wrapper: rc0, 11s, peak RSS 0.33 GiB, minimum available 14467 MiB.

The full current-main comparison has **not run**. Required next evidence is an
observer that can account for owned descendant exits while retaining foreign-load,
PID-reuse, missing-accounting and observation-limit controls. Keep the existing
strict gate; do not subtract guessed CPU or accept A/A as a cleanliness proof.

Accounting qualification is tracked in
[zenbench issue 44](https://github.com/imazen/zenbench/issues/44), separately from
[issue 42](https://github.com/imazen/zenbench/issues/42)'s retained diagnostics.
