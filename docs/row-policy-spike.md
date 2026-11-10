# Spike: one row executor, one policy, one admission pass

This is a separate experiment based on the qualified PR208 contract. It is not
a release proposal or permission to merge. PR208 remains unchanged.

## Questions and stopping conditions

1. Can raw encoders and retained typed/erased jobs share row execution and
   publication without losing their different admission boundaries?
2. Can operation entry points take one bound policy without silently dropping
   an older independent requirement? What is the caller migration?
3. Can mandatory admission be evaluated once, with built-in selection handling
   preferences only, while preserving direct policy use and refusal diagnostics?

The deliverable is executable candidate code, a separate review PR, and a
scenario ledger that distinguishes passing controls from unsupported paths.
Qualification uses the existing I/O and policy conformance suites, focused native
JPEG/PNG and multipass controls where their pinned graphs compile, and explicit
comparisons of policy choices/refusals. A native integration failure is a finding,
not permission to replace it with a scripted codec or change its assertions.

## Boundaries that must survive

- A supplied encoder has already been constructed. Its `begin_output` reports
  observed timing; it cannot certify a configured-job plan or metadata retention.
- A retained job supplies its real plan before construction. Requirements must
  refuse before preparation when workspace evidence is unknown. Startup must
  validate the same retained plan; no reconstruction or invented pass counts.
- Mandatory requirements from independently configured sources are conjunctive.
  An application callback sees compliant executable alternatives only.
- Discovery can defer evidence; final adoption cannot waive obligations. A late
  callback can choose among newly proven alternatives, not make unknown facts true.
- Live output is situational: it can retain source and native raw storage across
  writer waits. Finish-first releases those owners before publication but retains
  compressed output in bounded memory or a distinct disk spool. Neither implies
  a whole-operation memory bound.
- Unknown metadata/workspace/opaque-profile evidence stays unknown. Raster borrowing,
  output timing, precision hints and successful setters do not prove those facts.

## Scenario ledger

| Scenario | Required observation | Evidence |
| --- | --- | --- |
| Raw row encoder | Early strip admission and original output-schedule callback | pending |
| Retained typed/erased job | Plan once; constructor after route admission | pending |
| U8/U16 padded raster | Exact active samples; padding excluded; borrowed planned route preserved | pending |
| Multipass | Pass range, finality, replay identity/hash/snapshot and source-pass limits | pending |
| Sequential low-memory output | Live available only for streaming timing; cancellation at waits | pending |
| Buffered encoder | Finish-first drops raw owners before final writer waits | pending |
| Seek-back file | Prefix-relative patches, typed errors, cancellation before restore/flush | pending |
| Strided source / changing declaration | No wrong rows; reject callback/source mutations before next effect | pending |
| Late metadata / privacy | Discovery allowed; final unknown obligations refuse | pending |
| Conflicting policy sources | Both enforced; no last-writer-wins replacement | pending |
| Caller callback | Cannot bypass requirements; may override preferences | pending |
| Policy direct / delegated | Same admitted choices and named unmet facts | pending |
| Cancellation / timeout / callback error | Original typed cause, terminal failure and confirmed byte counts | pending |
| Tokio / erased ownership | Existing Send and cancellation/publication contracts remain intact | pending |
| Whole-raster-only job | Explicit refusal unless a real one-shot plan/executor is implemented | pending |
| Collection/video/conversion policies | No accidental loss of unrelated requirements | pending |

## Current costs to measure

Count shared and removed production lines separately from tests. Preserve all
existing tests. Record actual pointer/read/constructor effects rather than infer
zero-copy or allocation savings from names. Wall-clock throughput and whole-process
peak memory are not established by source inspection or test duration.
