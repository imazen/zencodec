# Native JPEG Pulse diagnostics — 2026-10-29

Core base `9bbd89f5`, changes in the enclosing commit. Native JPEG pin
`7c5a9501270210d6ef534c23221ff2660508b7cf`; paired pixels `f23ac753`.
Existing JPEG harness pins and features are unchanged. Resolution adds only
how-far-along 0.1.0, how-far-really 0.1.0, critical-section 1.2.0 and the local
zencodec-testkit 0.2.0 package. No existing package identity changed.

```sh
just native-pulse-diagnostics-check "$HOME/tmp/zencodec-native-pulse"
```

Two tests passed, strict test Clippy passed. They exercise:

- Shared incremental conformance on baseline and progressive 97x65 fixtures,
  including chunked pixels, pre-cancellation/timeout, no sink effects and terminal
  behavior. Existing helper assertions were not relaxed.
- Native direct decode, separately from the erased wrapper. First observe a
  successful baseline, then inject a one-shot stop at checkpoint 0, count/2 and
  count-1, with both Cancelled and TimedOut. Twelve selected interruption cases
  preserve the reason and leave no StopOutcomeMismatch diagnostic.
- Explicit outer operation lifetime, completed diagnostic owners, no dropped
  spans/incidents and no active spans at return. Successful decodes must produce
  at least one actual progress report.

Observed native trace counts, not a performance model:

| Fixture | Cancellation checks | Progress reports | Reported units |
|---|---:|---:|---:|
| Baseline JPEG | 15 | 1 | 65 |
| Progressive JPEG | 57 | 1 | 65 |

Reporting occurs only at completion on these fixtures. This does not establish
useful intermediate progress; [zenjpeg issue 251](https://github.com/imazen/zenjpeg/issues/251)
tracks that gap. Fourteen full JSON traces (two successful and twelve interrupted)
are retained under `~/tmp/native-pulse-diagnostics-20261029/artifacts` on the
qualification and coordinating hosts. Trace source locations identify native
checkpoint/report sites. Timings are instrumented, not controlled benchmarks;
there is no claimed responsiveness bound, throughput improvement or whole-process
memory bound. Cancellation is injected at a native checkpoint, not requested by
an independent external clock, so no external observation-latency claim follows.

The first native gate passed before adding trace retention and the positive
report assertion: run-heavy rc=0, 90s, peak RSS 0.68 GiB, min available 14066 MiB,
peak load 1.53. Final strengthened gate: rc=0, 8s, peak RSS 0.34 GiB, min available
14479 MiB, peak load 0.58. Both use ARM64, a 3 GiB cap, two jobs and timeout 900s;
these are validation-wrapper resource figures. Full logs are retained in the
same run directory. No executable archive is retained.

Limits: only the pinned default baseline/progressive fixtures and selected
checkpoints, not every mode, worker count or input. No claim about rendering,
encoder buffering, video, TIFF/JXL or other codec responsiveness. The testkit's
separate faulty-wrapper controls validate detection of swallowed stops and reason
collapse; the native harness itself does not mutate native code.
