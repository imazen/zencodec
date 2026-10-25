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
| Resource evidence | Upper/enforced bound versus estimate/unknown; exact cap; driver-only accounting; resident input lower bound | Control passed: `policy_engine`; runtime enforcement still needs integration |
| Planning bounds | Candidate limit, decision limit, ticket exhaustion; no arithmetic wrap | Control passed: `policy_engine` |
| Input/output irreversibility | Non-replayable input excludes rewind; published bytes exclude restart/prefix changes; safe continuation remains selectable | Control passed: `policy_engine` |
| Cancellation | Before callback and after successful callback; no selected operation escapes the stop | Control passed: `policy_engine` |
| Callback panic | Panic is not converted to a policy refusal; reuse of interrupted engine refuses | Control passed: `policy_engine` |
| Diagnostic retention | Fixed-capacity chronological journal reports dropped entries; public failure summary omits native callback payload | Control passed: `policy_engine`; detailed rejection evidence remains to add |
| Shared reservation | Another job consumes space during selection; partial RAM reservation rolls back when disk admission fails | Earlier control passed: `policy_resolution`; unified engine integration pending |
| Coupled choices | Individually valid metadata/execution selections exceed a combined budget or conflict; selected plan becomes stale before start | Pending integration |
| Unknown growth | Actual reservation before each growth, exact-cap failure, release on failure/cancellation | Pending integration |
| Native codec path | Same policy through concrete config and CodecSet/erased jobs, strided pixels, unsupported metadata, native failures | Pending integration |
| Metadata reality | Actual packets, malformed/opaque EXIF, retention/removal conflict, missing carrier, independent output inspection | Pending integration |
| Output failure | Slow/short/failing writer, partial artifact, no automatic retry, raw/staging lifetimes | Existing retention control; unified diagnostics pending |
| Transport and viewport | Reused simulator, range/source version changes, view cancellation and disk/resident controls | Existing transport model; unified resolver integration pending |
| Media | Real packet/frame facts, epoch changes, late decisions, retained frames and multiple tracks | Existing media evidence; unified resolver integration pending |
| Concurrency | Independent mutable resolvers, shared immutable preset configuration, unique tickets, no lock held across callbacks | Pending concurrent control |
| Portability/cost | no_std/WASM compile, static versus erased dispatch, allocation/error-report cost | Pending measurement |

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

Policy should centralize anticipated refusals. Corrupt input, transport failure,
cancelled work, resource exhaustion and implementation defects remain distinct
failures, with their original causes. They must not all be relabeled as policy
denials for the sake of one error funnel.
