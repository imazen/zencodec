# Situational policy: choose a feasible execution plan

Proposal and private experiments, 2026-10-25. Native stored-request integration
is on main at `98db8a318d9d5bcaf1676a807dec34a3266302c9`.
This adds no public library API.
It extends the [checked caller contract](minimal-0.3.md#caller-contract-required-effects-preferences-and-checked-results).

An application often wants “use the fastest acceptable path within this budget,”
not “always stream.” The same policy should choose buffered execution for one
input, native sequential execution for another, or refuse when neither can meet
the request. A resolver should make that choice from operation-specific facts.

## Requirements, permission, preference

Keep three questions separate without making callers answer them repeatedly:

- What must hold? Correct pixel interpretation, required metadata effects, a
  resource cap with an explicit accounting scope, and any required output behavior.
- Which alternatives are allowed? Disk staging, another registered codec,
  a different representation, or omission of selected descriptive metadata.
  None of these permissions is implicit in a resource shortage.
- Among those alternatives, what is preferred for this situation? Completion
  speed, first useful output, shorter raw-storage retention, fewer range requests,
  lower transferred bytes, or avoiding disk access.

A named application policy can answer all three. A custom callback is an escape
hatch for conditional choices; ordinary callers still configure once and execute
with `Result`. A successful operation satisfies the chosen, explicitly permitted
contract and every invariant shared by the alternatives. The result records the
selection and any permitted omissions. It does not require a warning scan to
discover that a mandatory effect failed.

For example, keeping supplied copyright can be required, or it can be preferred
with omission explicitly permitted under stated conditions. If it is required,
the resolver cannot authorize an otherwise inadmissible “drop all EXIF” plan.
When malformed EXIF prevents both copyright retention and required GPS removal,
the operation refuses unless the request allows an alternative that can be
implemented and checked. The callback chooses policy; an editor performs the
actual rewrite, and the codec validates its output framing and interpretation.

## Streaming is several independent facts

An incremental byte interface, row delivery, bounded live raw storage, early
useful output, pausable upstream production, and pausable encoded output are
different properties. Describe the actual codec configuration and pipeline.
Do not infer the others from a `streaming` flag.

The following are hypothetical decisions, not measured memory results:

| Situation | Decision |
|---|---|
| Buffered execution fits and is preferred for this input | Use it; no unconditional streaming mandate. |
| The buffered path needs 10 GB, while a qualified native sequential path meets the cap | Choose the sequential path. |
| A streaming-shaped interface still retains 10 GB and exceeds the cap | Reject that candidate. Its interface shape does not satisfy the storage requirement. |
| Every available implementation needs 10 GB and the cap is lower | Refuse. Output staging and callbacks cannot remove a codec's required peak. |
| Raw memory fits, but waiting for a slow writer would retain it unnecessarily | Consider finish-first with independently admitted encoded staging and job concurrency. |
| Raw/intermediate state is bounded and upstream can pause | Output backpressure can be necessary to keep encoded storage bounded. |
| The required caller-owned raster is already resident | Include its ownership in the scope; changing encoder mode cannot erase that peak. |

Finishing before output can reduce raw **retention time** without reducing the
peak needed to finish. Nor does an encoder finishing release a raster its caller
still owns. Encoded staging needs its own capacity and failure policy; when it
fills, waiting, spilling, or aborting must follow the admitted plan. Arbitrary
encode-ahead is not a resource policy. CPU fairness stays separate from waiting
on a destination. See the [native retention control](../benchmarks/io_encoder_retention_2026-10-25.md)
and [issue 160](https://github.com/imazen/zencodec/issues/160).

The same reasoning applies to video: reference pictures, lookahead, reorder
queues, retained presentation frames, other selected tracks and configuration
epochs matter. Frame-at-a-time input does not establish bounded output latency
or raw retention. A gigapixel viewport similarly needs dependency/range facts
and view cancellation, not a universal requirement to stream the entire image.

## The resolver's authority

The shared boundary should obtain facts, construct permitted alternatives,
exclude candidates that violate required effects, ask the policy to choose,
then reserve and execute. Resolve at each relevant fact/admission boundary,
including discoveries during streaming, not per row, packet, allocation or writer
stall. A previous choice only remains valid for its unchanged fact scope. A closure can capture application
context without putting application types in the codec API.

The current private experiment uses a typed, fallible interface:

```rust
trait PolicyResolver {
    type Error: core::error::Error + Send + Sync + 'static;
    fn resolve(&mut self, question: Question<'_>)
        -> Result<Decision, zencodec::At<Self::Error>>;
}
```

This is implemented by closures and a configurable `PolicySet`. Questions borrow
typed facts and immutable alternatives. An answer selects an offered token, uses
the configured preset, or declines with a traced reason. `Err(At<E>)` preserves
an application failure distinct from an expected decline. The callback cannot
alter an offer or its requirements. Its answer is a selection, not a replacement
for resource admission or backend enforcement. The original `choose`/index
experiment remains a small historical control in `policy_resolution.rs`.

Production offers need facts bound to the actual input, configuration and source
version. Deferred choices must also be bound to that planning generation; stale
choices fail. Reserving after selection closes the capacity race: another job
may have taken space during the callback. A failed multi-resource reservation
must release any partial reservation before returning. Do not hold global budget
locks while calling application code.

There should be no general “ignore error and continue” response. A change in
input facts may justify a fresh bounded planning step before irreversible work.
It cannot justify replaying consumed non-replayable input, silently restarting
after output publication, or an unbounded negotiation loop. Probe/planning reads
and metadata parsing also need budgets. New video configuration facts may require
revalidation at an epoch boundary; they do not permit retroactive policy changes.

The [native JPEG qualification](policy-jpeg.md) shows why a successful header or
full-probe call cannot yet certify final interpretation. Color readiness,
complete metadata inventory and privacy admission are separate obligations.
The [suspension/disclosure experiment](policy-transcoding.md) binds the same
resolver to new facts, bounded holding and independent irreversible effects.
Production adoption requires [the mandatory barrier contract](https://github.com/imazen/zencodec/issues/166).

## What can actually be guaranteed

Keep these forms of evidence distinct:

- A qualified upper bound permits reservation before execution. The bound must
  cover the relevant configuration and ownership, including the largest
  indivisible step. It does not promise successful input/I/O or a wall-time bound.
- Complete runtime accounting can enforce a cap even when total need is unknown:
  reserve before each growth and fail before exceeding it. This promises bounded
  resource use or failure, not that admitted work will complete within the cap.
- An estimate can rank alternatives. It cannot prove that a hard cap will hold.
  Unknown cost must stay unknown. Unaccounted backend allocations cannot be hidden
  behind a driver-only budget.

Scope matters: codec workspace, retained input, decoded output, encoded staging,
cache and shared job capacity are not automatically process RSS. Shared storage
is charged once until its last owner releases it. A caller can retain unrelated
allocations outside the operation. A time estimate likewise cannot become a hard
deadline when a codec step is not preemptible.

Keep concrete execution planning in the I/O/pipeline layer. Keep
`resolve_color_emit` as a domain resolver, the metadata rewrite `Services` trait
as injected editing/compression mechanisms, and checked fidelity selection as
configuration selection. None currently proves all operation effects. Reuse
them behind a shared policy boundary. A single typed question enum can give the
application one place to express decisions across these domains, while the
planning/editing implementations retain their own responsibilities.
Async approval/remote negotiation should be added only with a consumer that needs
it; an async callback requirement does not belong in every core codec call.

## One typed interface, ordinary application code

A single resolver handling an enum of decision questions is the recommended
direction. Each variant carries the facts needed for that decision and concrete
alternatives. Applications can write `match`, `if` and ordinary helper functions;
they do not need a policy language or a separate callback for each option.

The executable vocabulary is below; public names and placement still need API
review. The question variants carry small facts and borrowed choices, not owned
copies of image data. The prototype's media facts contain primitives and do not
introduce a dependency from the core onto a media driver.

```rust,ignore
enum Decision {
    Choose(ChoiceId),
    UsePreset,
    Reject(zencodec::At<Decline>),
}
```

For example, one application resolver can choose location removal for an upload,
retain supplied attribution, choose bounded sequential execution for a large
input, and delegate every other decision to its selected preset. The metadata
choices describe their effects, including any lost attribution; the execution
choices describe storage evidence and output behavior. Application code can make
the conditions clear without reproducing codec capability negotiation.

`ChoiceId` should be an opaque token for an offered alternative, validated against
the current question and operation generation. One answer shape avoids returning
a metadata answer to an execution question. Selection cannot manufacture a codec
implementation or waive an independently required effect. Already-known input
facts should be supplied, rather than requiring callback-side probing. Report
unknown facts explicitly. For choices that eventually need a continuous value,
use a typed constrained proposal with validation; do not enumerate an unbounded
set or add an untyped payload escape hatch.

Presets, parameterized formulas and handwritten code should implement the same
resolution semantics. `UsePreset` invokes the configured fallback once; it must
not recurse into the overriding callback. Unknown future questions are refused
unless that fallback knows how to resolve them. A preset cannot silently weaken
an explicit hard requirement. Keep serialization on the preset/configuration
data; an arbitrary Rust callback need not be serializable. A named `Profile`
binds hard requirements and preset preferences together. Configuring the
`PolicySet` alone only ranks already-admissible alternatives: it cannot guarantee
privacy, preservation or a memory cap. `Request::new(format, profile)` stores
both halves once in the native experiment. Ordinary callers use `encode`;
specialists use `encode_with`, whose UsePreset answer uses that stored preset.

One hook does not require one giant global context. Question variants can borrow
the relevant small records without owning copies of pixels or metadata. The
resolver is called at decision points, not for every row, allocation or progress
event. Cancellation, error propagation, allocation enforcement and data transport
remain their own mechanisms. Coupled choices still need a coherent final plan:
independently accepted metadata and execution answers do not establish that their
combination is realizable.

A refusal of one combination is not proof that no feasible combination exists.
The private control demonstrates a selected 48-unit plan failing a 40-unit budget
while another 32-unit combination meets the same requirements. These are authored
logical costs. `Plan::combine` validates a plan; it does not search or silently
replay callbacks. A production planner should offer coherent combined alternatives
when domains interact, or carry the remaining constraints into later questions.
Bound any search and report planning exhaustion separately from an impossible
contract. Do not shift that coordination loop back to ordinary callers, or silently
undo a custom choice to make a different plan fit.

Place the shared question vocabulary where its dependencies fit. The core can
carry lightweight codec facts; it must not acquire a dependency on its I/O or
media drivers to name their decisions. An integration layer can translate those
typed questions into the single application-facing resolver. Feature-gated media
support should not force image-only users to configure media policy.

The [fallible resolver experiment and scenario matrix](policy-scenarios.md) now
exercise the heterogeneous question enum, configured profiles, scoped choice
tokens, traced errors, joint-plan validation, actual charged payload growth,
native QOI/BMP operations and real container packets. Native whole-operation
accounting and broader metadata/color/media effects remain qualification gates.

## Debugging and predictable failure

Keep the common boundary responsible for reports without sending every failure
back through application policy. Corrupt input, native codec failures, transport
failure and cancellation retain their causes. A policy decision cannot turn them
into successful output. Replanning requires new facts and an explicitly safe
boundary; it is not an automatic retry after an error.

| Outcome | Caller meaning |
|---|---|
| No admissible alternative | The required contract cannot be met by the offered implementations; the callback was not invoked. |
| Explicit decline | The application chose not to proceed; its reason and rule location survive. |
| Callback error | Decision-making itself failed; the original typed error and whereat frames survive. |
| Invalid/stale choice | The answer is not authorized for this question/source/epoch; no execution starts. |
| Admission failure | Capacity changed or reservation failed after selection; partial reservations roll back. |
| Native/I/O failure | Report stage and partial progress, with the native cause; successful policy selection did not promise execution success. |

The error record needs operation/question identity, domain, source/configuration
epoch, required effects and budget scope, rejected choices with typed reasons,
selection/fallback route, and publication progress. Prior successful selections
must survive a later failure. The native experiment retains all three decisions
and selected alternatives; the general engine has a bounded chronological journal
with a dropped-record count. Production correlation should add backend/configuration
identity and a caller-supplied request identifier at the integration boundary.
Do not use source offsets or transient token numbers as persistent resource identity.

Use bounded enum/numeric summaries by default. Error sources and custom whereat
contexts remain available for deliberate inspection and may contain application
data. Requiring `At<E>` does not force the callback to add an origin frame; the
boundary adds its own location. Typed and erased tests cover empty traces, capped
traces and native-cause lookup. `At::map_error` preserves frames, whereas a naive
`Box<dyn Error>` containing `At<E>` can hide `E` from source-chain traversal.
A reviewed erasure envelope must preserve both, instead of flattening to text.

Cancellation is checked before and after callback execution. An actual callback
error remains primary if cancellation also arrives; after a successful answer,
cancellation prevents admission. Arbitrary Rust callbacks cannot be preempted or
made allocation-bounded. Panics propagate, and the interrupted engine cannot be
reused. No unwind-catching or global callback lock is required. Error construction
still allocates; the prototype does not promise usable diagnostics after process
allocation failure.

For testing, inject typed facts, a deterministic resolver, storage admission and
the stop signal. Assert selected effects, rejection codes and emitted output,
not diagnostic wording. Record enough facts to reproduce the choice without
copying pixel payloads into logs. Replaying arbitrary callbacks can repeat side
effects: require deterministic callbacks when replay is part of the application
contract. A record is evidence of a past decision, not authority to reuse its token.

## How resolution can reduce the API

The saving comes from retiring duplicate ways to make the same decision. Adding
a callback beside every existing flag and setter increases the surface. Moving
all those flags into an untyped payload protocol only hides it. Typed question
variants are useful when they consolidate the actual decision and its validation.

The smaller consumer contract is: record an operation request, apply a reusable
policy, execute, and inspect the result when the selected alternative matters.
Keep ordinary config/job and `CodecSet` conveniences. They all use the same
resolution boundary; callers do not need a separate capability/check/set/read-back
sequence. A custom resolver is optional. Named policies handle ordinary requests,
and simple preferences can use an ordered list rather than a callback.

These are concrete ablation candidates, checked against source at `46fe941d`:

| Current surface | Replacement candidate | What must remain |
|---|---|---|
| `with_policy`, `try_set_policy`, `try_with_policy`, and erased `set_policy` / `try_set_policy` | One recorded request with automatic checked admission. Retire the unchecked path first; retire specialized application hooks only after the shared boundary replaces their enforcement. | Early validation of malformed input, rejection without partial mutation, typed execution errors. |
| `with_fidelity`, `resolved_target_fidelity`, `try_with_fidelity`, `check_fidelity_target`, `select_fidelity`; erased `try_fidelity_job` / `select_fidelity_job` | Express the required fidelity or permitted alternatives once. Use the common resolution path and return the selected target in operation facts. | Codec-specific tuning, approximate-target semantics, and validation against actual input. Config read-back is not proof of output fidelity. |
| `EncodePolicy::embed_icc/embed_exif/embed_xmp`, metadata retention policy, color-emission policy and a broad `strict` switch | One authoritative description of required metadata/rendering effects and allowed carrier alternatives, resolved together. Presets are recipes over that description. | Byte-exact versus semantic retention, explicit carrier requirements, pixel interpretation, and parser compatibility as a separate concern. |
| `with_prefer_streaming` plus `prefers_streaming`, followed by caller-selected execution paths | Select an admitted execution strategy from storage/latency/source facts. Retire the boolean negotiation when real callers can use that path. | Native row/session protocols, explicit early-output requirements, ownership, limits and codec-specific performance controls. |
| Per-option typed job setters and their erased forwarders | A framework-owned typed request, passed through generic and erased dispatch as the same data. Put fluent conveniences on the request rather than requiring every codec trait/shim to implement them. | Efficient static dispatch and the distinct execution hooks needed for borrowing, mutation, rows and media sessions. |

This is a candidate removal list, not an approved API change. The existing
checked setters remain the immediate safe migration path. Eliminating them later
requires a request boundary that actually retains and enforces every field;
`set_request` with a no-op default would recreate the present problem.

Three boundaries should become simpler:

- **Consumer:** requested effects, allowed alternatives and one execution result.
  Capability information remains useful for UI/discovery, without being required
  for correct execution. Common cases need no callback or manual prepare step.
- **Adapter:** construct input facts, supply transport/editing/storage mechanisms,
  and preserve the request and selected contract through dispatch. Metadata
  `Services`, `ReadAt` and async transport interfaces do real work and remain.
- **Implementor:** describe actual operation alternatives and execute resolved
  choices. Share request storage, invocation of the one resolver and reporting
  instead of independently interpreting application fallback policy in every codec.

Request fields still need typed semantics. The callback cannot eliminate the
types describing fidelity, metadata, limits, timing or pixel layout. A single
`resolve(anything)` method with a large new protocol would reduce method count
while increasing what callers must understand. Likewise, actual result facts
must describe execution, not merely echo the plan the callback selected.

Place reusable policy configuration at the application boundary, with mutable
decision state scoped to each operation. Do not serialize concurrent `CodecSet`
operations behind a shared mutable-callback lock. Fixed policies should resolve
without callbacks or allocation of a Cartesian product of every option. Obtain
only the relevant alternatives, under bounded planning work.

The resolver does not justify removing distinct image/video ownership and
execution mechanisms. One-shot versus borrowed strips, mutable pixel fast paths,
incremental byte demand and ordered media sessions still need their own measured
consolidation experiments. This proposal targets duplicated policy coordination.

The private native slice now passes the same stored fidelity/EXIF request through
concrete and `CodecSet` paths. Ordinary and custom-policy consumers preserve the
one-call path; mandatory, permissive and conflicting requests check actual output
and errors. The existing bitmap metadata setter can silently omit supplied EXIF,
while the checked boundary refuses required retention before encoding. This is
evidence for centralizing the request, not permission to delete production methods.
Inventory retired methods, replacement request fields and remaining backend hooks
together when preparing that API PR. No production API has been removed here.

## Executed experiment and remaining qualification

Run the `io-policy-engine-check`, `io-policy-native-check`, `io-policy-media-check`
and `io-policy-portability-check` recipes in the
[prepared export](../perf/io-validation/README.md#prepare-an-export). Media inputs
are explicit fixture arguments. The [scenario matrix](policy-scenarios.md) maps
each guarantee to mechanism tests, native evidence and remaining gaps.

[Cost measurements](../benchmarks/policy_cost_2026-10-25.md) include configured,
closure, borrowed trait-object and error-erased paths. The tested successful
operation paths did not allocate; failures allocated for traces and evidence.
These are bounded authored-choice workloads, not native codec speed claims.

Before freezing public types, qualify whole-operation bounds for named native
codec configurations, a bounded encoded spool with disk exhaustion behavior,
and ownership charges through retained media packets/frames. Expand actual
carrier/color/HDR/gain-map and animation conformance, and integrate viewport
cancellation/source changes with the policy boundary. The prototype deliberately
refuses unqualified native hard-memory requirements and metadata domains. Full
root freestanding compilation has an existing blocker in
[165](https://github.com/imazen/zencodec/issues/165); isolated policy-core and WASM
checks pass. Process-wide memory enforcement and diagnostic survival under OOM
are not promised. No 10 GB input was allocated or extrapolated from smaller tests.

Keep resource admission in
[161](https://github.com/imazen/zencodec/issues/161), output waiting in
[160](https://github.com/imazen/zencodec/issues/160), and the consumer contract in
[27](https://github.com/imazen/zencodec/issues/27).
