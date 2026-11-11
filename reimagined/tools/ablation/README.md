# Driver mechanism ablation

This standalone tool compares the root/main API and the reimagined API in one
binary. It reuses the existing SYN1 hand-written/coroutine fixture, translating
only the hand-written decoder to the new protocol. Noise/patch input bytes and
one strided backing sink are shared. The distinct pixel revisions stay pinned;
the sample-copy work is identical but the complete semantic surfaces are not.

Timed work includes decoder construction, output allocation, row copies and
drop. Runtime creation, fixture generation and correctness checks are outside
timing. Stack-erased and boxed older arms are distinct; erased pointers are
black-boxed to resist devirtualization. The direct A/A arm is an unchanged
control. Both APIs use a 64 KiB read size and identical step quotas. The quota
sweep separates scheduling from strip size. Resident one-hop decoding has its
own arm; it is not a substitute for a streaming upload.

Before measurement, the tool checks exact active samples, final completion and
padded-row storage over the same paths. The translated decoder checks current
requests after consumption and emits the new protocol's FrameDone event.
This establishes only these valid synthetic workloads, not native codec,
metadata/policy, error-lifecycle, storage-budget or network equivalence.

From `reimagined/`, use `just ablation-check` and `just ablation-build` under
the resource wrapper with an isolated output directory. Run the resulting
`drivers-*` executable with `run.py BINARY NEW_ARTIFACT_DIRECTORY -- FLAGS` to
retain full stdout/stderr, compiler flags and source/binary SHA-256 identities.
Use `--format=json`; the JSON includes all resource and reliability flags.
Do not drop flagged processes or disable admission to obtain acceptance.
