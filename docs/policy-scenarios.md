# Policy resolver qualification

2026-10-25. Private experiments under `perf/io-validation`, based on main
`057ea889`. No published interface has changed. This is the scenario inventory
and evidence record for the [policy design](policy-resolution.md), including
the limits of what each experiment establishes.

## Traced callback and configured resolver

The executable interface in
[`policy/resolver.rs`](../perf/io-validation/src/policy/resolver.rs) is:

```rust
trait PolicyResolver {
    type Error: core::error::Error + Send + Sync + 'static;
    fn resolve(&mut self, question: Question<'_>)
        -> Result<Decision, zencodec::At<Self::Error>>;
}
```

`Question` has typed execution, metadata and fidelity variants. Closures and
the configurable `PolicySet` implement the same trait. The predefined decisions
are choose an offered token, use the configured preset, and decline with a
reason. `Decision::reject` captures the rule's source location. A callback error
keeps its concrete cause and trace; an expected decline has a different code.

The engine validates requirements and offered choices, invokes the callback,
delegates to the preset at most once, then validates the returned token. Question
tickets are unique across engine instances and checked for overflow. Tickets
do not authorize resource use: reservation and execution are separate boundaries.

`whereat 0.1.5` is already re-exported by zencodec. Its `At::map_error` preserves
the trace. Its `Error::source` delegates to the wrapped error's `source`, so the
framework failure must expose the callback error as its source. The typed and
erased tests verify that `find_cause` still reaches the concrete application
error. Simply nesting wrappers or keeping only Display text would lose useful
information. The pinned `StopReason` is typed data rather than an `Error`; it
remains available in the failure cause without pretending it is downcastable.

The callback can use `At::wrap` without an origin frame. Requiring `At<E>` does
not force correct tracing inside application code, and trace allocation can
fail. The boundary adds its own frame and never discards the original cause.
Custom trace contexts and native causes may contain application data; only the
framework's bounded numeric/enum record is intended as a safe default summary.

## Scenario matrix

“Control passed” means an executable mechanism test using authored alternatives;
it does not certify a codec's memory bounds or actual metadata effects.

| Area | Cases and required behavior | Evidence/status |
|---|---|---|
| One interface | One closure handles metadata, execution and fidelity; configured object works directly or as fallback | Control passed: `policy_engine` |
| Error provenance | Original cause, origin frame and erasure frame survive; expected decline is distinct from callback failure | Control passed: `policy_engine` |
| Preset delegation | Callback runs once, preset does not recurse; unknown questions refuse | Control passed: `policy_engine` |
| Choice identity | Reused token fails across calls, engine instances, source revisions, plan revisions and media epochs | Control passed: `policy_engine` |
| Invalid requests/offers | Empty choices, wrong question family, unknown effect bits, non-finite fidelity after a valid first choice | Control passed: `policy_engine` |
| Required effects | No admissible alternative refuses before application code; callback cannot weaken requirements | Control passed: `policy_engine`, `policy_resolution` |
| Resource evidence | Upper/enforced bound versus estimate/unknown; exact cap; driver-only accounting; resident input lower bound | Control passed: `policy_engine`; payload allocation enforcement passed in `policy_execution`; native whole-operation accounting remains unqualified |
| Planning bounds | Candidate limit, decision limit, ticket exhaustion; no arithmetic wrap | Control passed: `policy_engine` |
| Input/output irreversibility | Non-replayable input excludes rewind; published bytes exclude restart/prefix changes; safe continuation remains selectable | Control passed: `policy_engine` |
| Cancellation | Before callback and after successful callback; no selected operation escapes the stop | Control passed: `policy_engine` |
| Callback panic | Panic is not converted to a policy refusal; reuse of interrupted engine refuses | Control passed: `policy_engine` |
| Diagnostic retention | Fixed-capacity chronological journal reports dropped entries; public failure summary omits native callback payload | Control passed: `policy_engine`; bounded failure evidence records exact options, costs and per-option rejection reasons |
| Shared reservation | Another job consumes space during selection; partial RAM reservation rolls back when disk admission fails | Unified `policy_execution` control passed; partial reservation rollback and simultaneous contenders |
| Coupled choices | Individually valid metadata/execution selections exceed a combined budget or conflict; selected plan becomes stale before start | Control passed: `policy_execution`; conservative summed bounds and source/plan/epoch checks |
| Unknown growth | Actual reservation before each growth, exact-cap failure, release on failure/cancellation | Actual payload control passed: `policy_execution`; allocator overhead and native codec storage are outside this scope |
| Native codec path | Same policy through concrete config and CodecSet/erased jobs, strided pixels, unsupported metadata, native failures | Pending integration |
| Metadata reality | Actual packets, malformed/opaque EXIF, retention/removal conflict, missing carrier, independent output inspection | Pending integration |
| Output failure | Slow/short/failing writer, partial artifact, no automatic retry, raw/staging lifetimes | Existing retention control; unified diagnostics pending |
| Transport and viewport | Reused simulator, range/source version changes, view cancellation and disk/resident controls | Existing transport model; unified resolver integration pending |
| Media | Real packet/frame facts, epoch changes, late decisions, retained frames and multiple tracks | Existing media evidence; unified resolver integration pending |
| Concurrency | Independent mutable resolvers, shared immutable preset configuration, unique operation/question tickets | Concurrent control passed: `policy_engine`; no shared callback mutex |
| Portability | Actual root types on WASM; isolated resolver on a freestanding target | Passed as described below; full root freestanding graph blocked by issue 165 |
| Cost | Static versus erased dispatch, allocation/error-report cost | Pending measurement |

## Commands and observed limits

Run `just io-policy-engine-check` in the
[prepared export](../perf/io-validation/README.md#prepare-an-export).
The initial engine test suite and warning-free Clippy passed:

```text
run-heavy: done rc=0 0s | peak-RSS 0.17GiB | min-avail 18454MiB | peak-load 0.70
```

This observation includes compilation. It is not a per-operation memory bound
or a performance comparison. Alternatives currently carry authored resource
and effect claims. A dishonest backend can still lie about those claims; native
output tests and actual accounting must qualify them. No policy callback can
prevent arbitrary trusted application code from blocking, allocating or panicking.
The engine checks cancellation at its boundaries but does not preempt Rust code.

## Portability and concurrent use

`just io-policy-portability-check io-policy-engine-check` passes the root-linked
consumer on `wasm32-unknown-unknown`, the isolated resolver on
`thumbv7em-none-eabihf`, host execution of the isolated consumer, and Clippy for
both checks. The isolated mode compiles the existing `src/fidelity.rs` directly
with whereat 0.1.5 and enough 0.4.5; it does not invent replacement fidelity types.
The native harness retains its separately pinned enough version. The checked
ticket issuer uses pointer-width atomics and refuses exhaustion without wrapping.

The concurrent control runs separate mutable callbacks with shared immutable
preset configuration. Operation identities and question tickets remain distinct.
Additional error controls cover missing origin frames, whereat's frame cap, and
a callback failure arriving with cancellation. Callback failure remains primary;
after a successful callback, cancellation wins before selecting an operation.

```text
run-heavy: done rc=0 1s | peak-RSS 0.19GiB | min-avail 18454MiB | peak-load 0.34
```

**Full freestanding integration is blocked by existing code**, not qualified by
the isolated check. The root zenpixels dependency enables its default std
feature. Turning that edge off in a disposable export exposes 12 gain-map float
operations that also fail on the freestanding target. Root production code was
left unchanged; [issue 165](https://github.com/imazen/zencodec/issues/165) records
the reproduction and required math/feature qualification. A successful WASM
check with std available is not a std-free dependency-graph proof.

Policy should centralize anticipated refusals. Corrupt input, transport failure,
cancelled work, resource exhaustion and implementation defects remain distinct
failures, with their original causes. They must not all be relabeled as policy
denials for the sake of one error funnel.

## Joint decisions and resource admission

`policy/plan.rs` combines execution, metadata and fidelity decisions from one
operation, request and source/plan/epoch context. Missing and duplicate questions
refuse. It checks resource sums with checked arithmetic and revalidates context
at start. This experiment conservatively adds selected stage bounds as though
they overlap; it does not yet exploit disjoint stage lifetimes. Resource evidence
must be bounded before this admission path will execute.

`policy_execution.rs` reuses the existing shared `Storage`/`Transfer` mechanism.
Two simultaneous contenders cannot spend the same capacity. A failed disk
reservation releases the provisional RAM reservation. Actual transfer payloads
reserve before allocation, reject allocator capacity larger than the charge,
and remain charged across clones until their final owner releases them.
Cancellation, incomplete input and changed epochs release in-flight storage.
Disk **reservation** is tested here; no disk spool is claimed by this control.
The scope is payload capacity, excluding codec internals, allocator overhead,
control objects, and RSS. Whole-operation admission needs those additional
charges and qualified codec bounds.

Failure evidence is allocated only on error, holds at most 64 candidate records,
and reports omitted records if that allocation fails or the candidate limit was
exceeded. Each candidate exposes its numeric/enum costs and typed reasons such
as unknown disk cost, missing effects or an invalid accounting scope. Callback
payloads remain accessible only through their native error/trace interfaces.

`just io-policy-engine-check io-policy-portability-check` passed after adding the
joint plan and actual payload controls:

```text
run-heavy: done rc=0 1s | peak-RSS 0.19GiB | min-avail 18499MiB | peak-load 0.11
```
