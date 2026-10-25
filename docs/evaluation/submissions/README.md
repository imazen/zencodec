# Preserved reader submissions

These are exact, uncorrected model answers from the sequential guide study.
Some intentionally remain uncompilable; their failures are part of the result.
They are assessment data, not examples recommended for application use. The
baseline positioned-read submissions also reproduce the old guide's error
stringification defect. Do not format, repair or import these into a library.

The [report](../../guide-learning.md) identifies the site revisions, exposure
protocol and limitations. [Recorded attempts](../../../benchmarks/guide_learning_2026-10-25.csv)
include compiler status, packet size and subjective confidence. The current
recommended implementations remain in [examples](../../examples/journeys.rs)
and [JPEG examples](../../examples/jpeg_policy.rs).

After preparing a source export with the existing pinned harness dependencies,
run an individual preserved attempt through `guide-learning-grade` (or
`guide-learning-audit` for the separate t3/t4 checks), under the resource wrapper:

```sh
just guide-learning-grade t1 docs/evaluation/submissions/baseline/t1-visible.rs EXPORT OUTPUT
```

The grader records a compilation failure without altering the submission.
Inspect `OUTPUT/result.json`; the coordinator's collection process exit code
is not the candidate's success status. Full text packets, reader notes and logs
remain in the artifact collection identified in the report.
