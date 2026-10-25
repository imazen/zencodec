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

`Question` has typed execution, metadata, fidelity, media-track and discovery-boundary variants. Closures and
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
| Native codec path | Same policy through concrete config and CodecSet/erased jobs, strided pixels, unsupported metadata, native failures | Passed for two pinned QOI/BMP configurations in `policy_native`; other codecs and whole-operation memory bounds remain unqualified |
| Metadata reality | Actual packets, malformed/opaque EXIF, retention/removal conflict, missing carrier, independent output inspection | Extracted EXIF controls pass with kamadak-exif inspection; whole-file privacy and carrier roundtrips beyond QOI/BMP are not qualified |
| Output failure | Slow/short/failing writer, partial artifact, no automatic retry, raw/staging lifetimes | Short/interrupted/zero/failing Write diagnostics passed in `policy_native`; the native retention control now executes a resolver-selected finish-first or sequential schedule |
| Transport and viewport | Reused simulator, range/source version changes, view cancellation and disk/resident controls | Native MP4/WebM/IVF decisions agree over resident, disk and reused network model; changed-source admission controls pass; viewport-specific adapter remains unqualified |
| Media | Real packet/frame facts, epoch changes, late decisions, retained frames and multiple tracks | Native multi-track packet checks and toy-session epoch/frame-ownership controls pass; native decoded-video allocation bounds remain unqualified |
| Late interpretation | Suspend before effects, bounded waiting, CICP/finality facts, independent commitments, stale receipts, typed callback failure | Native QOI ordering and post-barrier encoding pass; late discovery is authored. [Scope and evidence](policy-transcoding.md#qualification-status) |
| Opaque disclosure | Syntax is not privacy evidence; qualified regeneration, optional omission, required retention conflict, exact inspected bytes | Native XML/RDF parsing passes for one orientation-only schema. Arbitrary ICC, nested content and whole-file PII are unqualified. [Scope and evidence](policy-transcoding.md#qualification-status) |
| Concurrency | Independent mutable resolvers, shared immutable preset configuration, unique operation/question tickets | Concurrent control passed: `policy_engine`; no shared callback mutex |
| Portability | Actual root types on WASM; isolated resolver on a freestanding target | Passed as described below; full root freestanding graph blocked by issue 165 |
| Cost | Static versus erased dispatch, allocation/error-report cost | [Measured workload](../benchmarks/policy_cost_2026-10-25.md): success paths 89.5–94.2 ns, no operation allocations; failure paths allocate for causes, traces and evidence. Not a codec or I/O benchmark. |

The admission matrix exercises all 64 choice positions over 2,080 combinations
of budgets, required effects, replayability and published output. It checks that
more budget cannot remove a choice, and that stronger requirements or less
replay freedom cannot add one. These are invariants of the selection mechanism;
they do not establish the truth of a backend's resource or effect claims.

## Commands and observed limits

### Accounting-scope regression, 2026-10-25

Review against `c949d3f5` found that a disk-only operation cap accepted a
driver-scoped offer. The scope predicate checked whether a RAM cap existed,
so `ram: None, disk: Some(0), accounting: Operation` admitted an offer with
`accounting: Driver`. A claimed zero driver disk cost says nothing about codec
or editor disk use. This is a private-prototype defect, not a measured violation
by a native codec or a change to the released API.

`accounting_scope_applies_to_disk_caps_without_a_ram_cap` failed on that exact
case before the fix. Scope now applies whenever either resource has a cap. The
test crosses RAM absent/present, disk absent/zero/positive, and both required
and available scopes. A scope rejection occurs before the callback and retains
its typed rejection reason. Explicit driver-only requests and uncapped requests
remain permitted. The evidence values in this test are authored inputs.

Validation: `just io-policy-engine-check io-policy-native-check
io-policy-stream-check` in the prepared export passed, including Clippy. This
covers selection, joint admission, native bitmap integration, disclosure and
stream suspension controls; it does not qualify whole-operation codec storage.

```text
run-heavy: done rc=0 2s | peak-RSS 0.22GiB | min-avail 18279MiB | peak-load 0.46
```

This is the test process receipt, not a codec memory bound.

### Earlier qualification

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

Joint validation is not a solver. An explicit control rejects a 48-unit selected
plan under a 40-unit cap, then admits a different 32-unit combination with the
same requirements before any irreversible work. Costs here are logical fixture
inputs. The prototype does not automatically backtrack or replay a callback.
Production questions with coupled effects need coherent offered plans or carried
constraints, and a distinct bounded-search exhaustion outcome.

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

## Native metadata, pixels and output failures

`just io-policy-native-check` exercises QOI and BMP from the recorded zenbitmaps
pin through concrete configurations and CodecSet's erased configurations. The
same resolver selects EXIF disposition, fidelity and execution. Both paths
produce identical encoded bytes for the strided RGBA fixture and decode to its
exact samples. This is a two-configuration vertical slice, not a codec fleet
certification. Native memory bounds are unknown, so a hard operation RAM cap
refuses before parsing or invoking the callback.

The test reproduces the existing QOI setter's successful omission of supplied
EXIF. The checked operation refuses required rights retention before calling
the codec or writing output. If omission is allowed, it proceeds explicitly.
Preserving existing rights is different from requiring rights to be present:
empty input satisfies preservation without inventing an attribution.

Extracted EXIF filtering uses the existing zenmeta rewrite processor. Independent
kamadak-exif inspection confirms copyright and orientation retention, GPS IFD
removal, and removal of the fixture's camera identifiers, timestamps and
thumbnail fields. Exact preservation conflicts with GPS removal when the packet
contains GPS or cannot be validated. An opaque malformed packet can still be
retained byte-for-byte when privacy removal is not required and a carrier exists.
These are packet-scoped statements: arbitrary private data in unknown tags,
XMP, other carriers, ICC, HDR/gain-map and pixel-content privacy are not certified.
The native wrapper refuses unqualified color/orientation/XMP inputs explicitly.

Interrupted and short writes complete without changing bytes. Write-zero and
injected native I/O errors retain their typed cause, exact accepted-byte count,
and incomplete-output status. They do not invoke the resolver again or restart
encoding. Accepted bytes mean the Write contract, not disk durability or receipt
by a remote peer. Codec pixel-limit failures remain codec errors, not policy
declines.

Two interface findings need to shape the public design:

- A metadata preference alone is not a privacy guarantee. A named privacy profile
  must install required effects as well as the resolver's preferences. Otherwise
  ordinary fallback could keep an opaque packet when filtering is unavailable.
- Generic boxed errors do not automatically preserve a whereat envelope's inner
  type or trace. `At::source` skips its wrapped error and delegates to that error's
  source. The qualified bitmap adapter explicitly downcasts its known
  `At<CodecError>` envelope and maps it into the outer report, keeping the trace.
  A general erased interface needs an explicit trace-preserving boundary.

```text
run-heavy: done rc=0 0s | peak-RSS 0.20GiB | min-avail 18489MiB | peak-load 0.18
```

## Media and the reused network model

The media-track question is resolved once per track/configuration epoch. The
adapter copies known packet facts into the shared vocabulary without making it
depend on the media crate. Unknown random-access status and missing timestamps
remain unknown. A required track cannot be dropped; optional audio omission is
an explicit permission and callback decision. The returned selection is bound
to source revision, track and epoch and is checked on subsequent packets.

Native MP4 (AV1/Opus), WebM (VP9/Opus), and IVF (AV1) fixtures produce the same
packet bytes/timing and track descriptions over a resident slice, FileReadAt,
and the reused IMI transport model, at read sizes 7 and 65,536. Observations:

| Fixture | Recorded events | Policy decisions | Explicitly omitted audio packets |
|---|---:|---:|---:|
| AV1/Opus MP4 | 123 | 2 | 81 |
| VP9/Opus WebM | 163 | 2 | 101 |
| AV1 IVF | 42 | 1 | 0 |

The omission column is a separate, explicitly permitted run. Default runs keep
every track. These are packet/container controls, not new native-video decoding
or live-network throughput measurements. Frame ownership uses the existing toy
session as a protocol control: an old frame retains its samples and interpretation
after reset and an epoch change, while a packet from the new epoch invalidates
the old policy selection. Cancellation leaves the unaccepted packet untouched.

The three fixture paths are required inputs to `just io-policy-media-check`;
missing fixtures fail. Source generation and oracle provenance remain in the
[existing media experiment](../perf/io-validation/README.md#video-containers).
The two protocol controls run in Documentation CI; native fixture integration
is an explicit local command, not silently skipped in that CI job.

```text
run-heavy: done rc=0 1s | peak-RSS 0.21GiB | min-avail 18506MiB | peak-load 0.19
```

## Named profiles bind intent to preferences

The `Profile` control now constructs EXIF-web and exact-EXIF configurations
containing both requirements and a `PolicySet`. Custom code can return UsePreset;
that delegation never removes the profile's hard requirements. Unknown native
memory cost still refuses a configured hard cap. The native boundary also rejects
requirements outside its operation domain: a still-image function cannot silently
ignore a media-track requirement just because none of its questions handles it.
This is why the final consumer API should bind the profile to the request once,
instead of exposing the prototype's separate requirements/preset arguments.

Native callback facts now use the actual input descriptor and strided byte view.
That view's length is not a claim about the caller's entire backing allocation.

```text
run-heavy: done rc=0 1s | peak-RSS 0.20GiB | min-avail 18479MiB | peak-load 0.14
```

The native experiment now stores both halves in `Request::new(format, profile)`.
`request.encode(...)` uses the configured resolver; `request.encode_with(...)`
uses application code with that same configured fallback. `via_registry()` sends
the same request through `CodecSet`. Native QOI/BMP controls verify identical
output across these paths, the selected nondefault preset, and refusal of required
rights retention before invoking a callback when the format has no carrier.
This is a private consumer-shape experiment, not a newly supported library API.

The operation report retains selected metadata/fidelity/execution alternatives
and all three bounded decision records on success and on later failure. A
callback failure at the second question retains the first selection and original
application trace; a writer failure retains all selections and accepted-byte
progress. Failed operation reports are boxed; successful reports do not require
that box. A local closure can borrow non-Send application state. The callback
itself is not required to be Send merely because its error is erasable.

```text
run-heavy: done rc=0 1s | peak-RSS 0.22GiB | min-avail 16587MiB | peak-load 0.62
```

## Reproducible cost workload

`just io-policy-cost-build` builds the shared policy workload for zenbench and
heaptrack. Successful arms all execute the same validation and select the same
alternative: direct configured preset, callback delegation, a borrowed trait
object, and the error-erasing callback adapter. Separate refusal arms exercise
expected decline, an erased application error carrying data, and no admissible
alternative. Each iteration includes operation creation, identity issuance and
the bounded diagnostic journal; none includes codec work or real I/O. These
controls can measure dispatch/report costs without changing the enforced contract.

The [measured report](../benchmarks/policy_cost_2026-10-25.md) records source and
executable hashes, raw-data locations, commands, resource-wrapper receipts, timing
uncertainty and allocation attribution. Successful paths did not allocate in this
workload; arbitrary callbacks, native codecs and failure reporting are outside
that claim. The measurements support a hook at operation/epoch boundaries.

## Output-waiting policy is exercised, not inferred from timing flags

The existing native QOI retention control now runs a schedule selected by the
unified resolver. The adapter knows whether its explicit wrapper retains the
raster: it chooses finish-first in that case and direct sequential output for
the non-retaining configuration. Both encounter a genuinely pending async writer,
retain zero wrapper-owned raw rows at the stall, and preserve exact encoded bytes.
The callback runs once; stalls do not re-enter it. The original contrasting
control remains intact. Whole-operation memory, bounded staging and native
codec ownership qualification are still required before generalizing this choice.

```text
run-heavy: done rc=0 2s | peak-RSS 0.19GiB | min-avail 16447MiB | peak-load 0.77
```
