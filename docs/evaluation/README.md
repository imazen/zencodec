# Sequential guide learning protocol

A fresh Sol session receives rendered site sections in order, without repository
source, earlier design discussion, linked documentation, or compiler feedback.
Ordinary Rust knowledge is allowed. Each task has two submissions: the ordinary
visible view, then the same section with inline context/signatures expanded.
The coordinator compiles both preserved submissions afterward. A revised-site
replication uses a new session; the first learner is not reused as a fresh reader.

This is a protocol-limited agent study, not a sandbox proof or a human usability
success rate. Training knowledge cannot be removed. Record model/effort, revision,
exact intake, word count, attempts, compiler/runtime results and deviations. Do
not count compiler repair as first-attempt success. No source changes are made by
the learner and no missing fixture is skipped.

`tasks.json` freezes four coding tasks and two conceptual tasks. `oracles/`
contains exact-pixel, limit, source-failure and pre-write-disclosure checks.
`run.py` compiles a submission inside the existing prepared source export, under
its pinned dependencies. Run it through the standard resource wrapper. It records
failed attempts without altering them or weakening assertions.

For t5, score these separately: provisional absence is not final interpretation;
affected conversion/publication must wait or use an explicitly safe alternative;
late changes cannot retroactively repair published output; current informational
events are not a production resume barrier. For t6: output timing is not a retained
memory bound; caller/native ownership matters; staging and waiting need separate
admission; a cap can require runtime failure rather than guarantee completion.
Each criterion is 0/1, with the original answer retained for review. Report the
coding fraction and conceptual rubric separately; do not combine them into a
misleading overall percentage.

Each response also records confidence, missing information, guessed APIs and the
concept that took the most reasoning. The final learner report ranks confusion,
visual hierarchy and abstraction placement based only on material received.
Text-only site packets do not measure visual preference; the coordinator checks
rendered desktop/mobile layouts separately with Playwright.

Supplemental `t3_audit.rs` and `t4_audit.rs` checks were added after the baseline
learning assessment. They check the concrete source-error cause and independent
EXIF orientation without ICC. Run them with `just guide-learning-audit`; record
these results separately, without changing the frozen four-task score. A model
can pass an initial coding oracle while copying a debugging defect from a sample.
