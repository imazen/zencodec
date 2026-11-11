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
| Raw row encoder | Early strip admission and original output-schedule callback | Pass: existing raw/native staging-admission controls |
| Retained typed/erased job | Plan once; constructor after route admission | Pass: planned_row_encode temporal/erasure controls |
| U8/U16 padded raster | Exact active samples; padding excluded; borrowed planned route preserved | Pass: new native row_spike matrix plus borrowed-pointer controls |
| Multipass | Pass range, finality, replay identity/hash/snapshot and source-pass limits | Pass: shared replay/finality controls; native IMI graph blocked |
| Sequential low-memory output | Live available only for streaming timing; cancellation at waits | Pass: native JPEG schedules and shared publication cancellation |
| Buffered encoder | Finish-first drops raw owners before final writer waits | Pass: native owner-lifetime and shared replay finish-first controls |
| Seek-back file | Prefix-relative patches, typed errors, cancellation before restore/flush | Pass: shared patch controls and native JPEG disk/prefix controls |
| Strided source / changing declaration | No wrong rows; reject callback/source mutations before next effect | Pass: native mutation and shared source-revalidation controls |
| Late metadata / privacy | Discovery allowed; final unknown obligations refuse | Pass: deferred/final decision controls; whole-container PII not certified |
| Conflicting policy sources | Both enforced; no last-writer-wins replacement | Pass: conflict witness and 4,096 intersection pairs |
| Caller callback | Cannot bypass requirements; may override preferences | Pass: bound-policy and callback preference controls |
| Policy direct / delegated | Same admitted choices and named unmet facts | Pass: frozen-selector differential and weak-admission direct-call witness |
| Cancellation / timeout / callback error | Original typed cause, terminal failure and confirmed byte counts | Pass: existing typed failure/cancellation/unwind suites |
| Tokio / erased ownership | Existing Send and cancellation/publication contracts remain intact | Pass: existing shared/native Tokio controls; external HTTP E2E not run |
| Whole-raster-only job | Explicit refusal unless a real one-shot plan/executor is implemented | Pass: explicit existing planning refusal; no one-shot fallback added |
| Collection/video/conversion policies | No accidental loss of unrelated requirements | Pass: all-nine-domain matrix and workspace suites; native video not rerun |

## Current costs to measure

Count shared and removed production lines separately from tests. Preserve all
existing tests. Record actual pointer/read/constructor effects rather than infer
zero-copy or allocation savings from names. Wall-clock throughput and whole-process
peak memory are not established by source inspection or test duration.

## Caller experiment: state requirements once

The row driver already accepts one policy argument. Its deprecated
`Options::with_policy_set` is an additional independent source; the lower-level
`resolve_*` helpers still take an independent preset argument too.

```rust,ignore
// Current retained-job path: one policy is enough for new calling code.
let policy = PolicySet::new().require_output_schedule(OutputSchedule::FinishFirst);
row_encode::encode_job(job, Raster::new(pixels), file, policy, options, pulse)?;
```

An application can choose situationally without being allowed to waive its
mandatory requirements:

```rust,ignore
let policy = BoundPolicy::new(requirements, |question: Question<'_>| {
    match question {
        Question::EncodeRoute { choices, .. } => choose_for_this_request(choices),
        _ => Ok(Decision::UsePreset),
    }
});
row_encode::encode_job(job, source, writer, policy, options, pulse)?;
```

The private `transcode` prototype in `spike_matrix.rs` demonstrates the proposed
single-policy helper signature. It delegates to the real framework and is
**not a new exported API**. The proposed shape for the eight public `resolve_*`
helpers removes their independent `preset` argument; it does not replace the
borrowed facts, retained plans or typed offers with an untyped option bag.

```rust,ignore
// Existing helper:
resolve_transcode(boundary, offers, &mut bound_policy, older_policy, pulse)?;
// Proposed modern helper, after explicitly migrating the older requirements:
resolve_transcode(boundary, offers, &mut combined_policy, pulse)?;
```

Simply deleting `older_policy` is unsafe. For example, an older
`PreserveReported` requirement and a newer `DiscardDescriptive` requirement
currently refuse together. Keeping only the newer policy succeeds. This
counterexample is executable in the spike.

The private intersection prototype uses these rules:

| Fields | Combination |
| --- | --- |
| Workspace, output-wait, source-pass limits | Smaller configured maximum |
| Extended range, exact replay | Either requirement remains mandatory |
| Permission for opaque profiles or raw RAM staging | Both must permit |
| Required metadata treatment or output schedule | Equal, absent, or an explicit conflict |
| Lossless and assumed-siting preferences | Bound policy retains current precedence |

All 4,096 generated policy pairs preserve final transcode choices and unmet
facts when intersection is representable. Contradictory pairs explicitly fail.
There is a remaining semantic decision: eager intersection rejects a conflict
at configuration time, while the current two-source model can still allow
`ContinueReading` before final adoption. The spike tests this difference; it
does not silently choose a new discovery contract.

## Shared execution, distinct startup guarantees

```text
supplied encoder -- begin_output --> observed row timing -- output policy --+
                                                                          |
retained job ----- plan_encode ---> actual plan --------- joint route ------+
                                                                          |
                                              start / validate retained plan
                                                                          |
                                          one row or replay execution loop
                                                                          |
                                release source, rows, plan and native encoder
                                                                          |
                           one finish / staging / tail / patch / flush path
```

`StartupPlan::ObservedRows` and `StartupPlan::Declared` are private. The supplied
encoder no longer fabricates an `EncodePlan::rows` as though it were preflight
evidence. It keeps the existing output-schedule question and early strip
allocation. Retained jobs still plan once before construction and can borrow
an immutable raster without copying. Supplied encoders keep their existing
copying behavior in this experiment, isolating execution consolidation from
an additional ownership/performance change.

`Source`, `Encoder`, `EncodeJob`, typed/erased entry points, destination APIs and
`Option<&dyn Pulse>` signatures remain unchanged. No `Send` requirement was
added. The common filler also reports row progress on the supplied-encoder
path; checkpoint counts are not a stable contract. It adds per-row validation
work there, so allocation parity alone cannot establish throughput parity.

Whole-raster-only jobs still refuse in this row executor. One-shot fallback,
privacy evidence, codec workspace bounds, collection pagination and native
video muxing are not manufactured by sharing this loop.

## Mandatory admission experiment

Hard offer predicates now live in `PolicySet::unmet`. Built-in selection reuses
them and handles preferences separately. The old selector is retained only as
a test oracle; it is not compiled into the runtime.

The framework's `UsePreset` route selects preferences over already-admitted
choices. Direct public `OperationPolicy::decide` calls still validate their own
set: a callback could pass a question admitted under a weaker policy. Passing a
`PolicySet` directly can therefore perform a second bounded scan. Removing that
scan would need a private trusted dispatch/proof interface; this spike favors
one predicate implementation over adding that interface prematurely. Original
choice indices, tickets and refusal categories remain covered.

## Results and limits

- The frozen selector and candidate agree on **184,320** generated cases:
  nine domains, 4,096 configuration bit patterns and five admitted masks. These
  are synthetic facts, including unknown evidence; they do not certify native
  metadata/workspace claims.
- Full workspace all-feature tests, doctests, strict all-target Clippy and the
  core no-default check pass. Existing cancellation, timeout, source mutation,
  replay/finality, disk staging, patch/flush, callback and Tokio controls remain.
- All **69** native JPEG-harness tests pass, including native JPEG/PNG progress
  controls and the new raw/typed/erased matrix: RGB/gray, U8/U16, padded rows,
  baseline/progressive JPEG. The new matrix requires byte-identical routes;
  independent djpeg reads JPEG, and png 0.18.1 verifies exact active PNG samples
  including U16 byte order. Existing quality bounds/assertions are unchanged.
- The native IMI public-driver graph **does not resolve**: its frozen
  `archmage >=0.9.30, <=0.9.31-beta.1` conflicts with the current pixel graph's
  stable 0.9.31 requirement. Native replay requalification is blocked; existing
  shared replay controls pass, but cannot substitute for that native gate.
- Counting source before the first `#[cfg(test)]` gives **190 fewer row-driver
  lines** and **57 fewer policy lines** than PR208. This counts comments/blanks
  consistently and excludes the temporary differential oracle and new tests.
- Paired heaptrack controls use identical native test/oracle source and dependency
  graph against PR208 and this candidate. Both repeats in each arm report
  **9,672 allocation calls**, **1,620 temporary allocations**, and **13.10M peak
  heap** (heaptrack's rounded display). This is the complete small native control,
  including harness/codec/readers, not isolated driver overhead or a bound for
  larger images. There is **no throughput or peak-RSS improvement claim**.

The recommendation is to review shared execution as an independent refactor.
The hard-predicate consolidation is viable with the direct-call safeguard.
Single-policy signatures need an explicit conflict/migration decision first;
none of the old policy arguments or exports were removed in this spike.
