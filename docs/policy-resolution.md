# Situational policy: choose a feasible execution plan

Proposal and private experiment, 2026-10-25. Reviewed against main
`c6943f6cd40ec1cb9c1338a2ecdb385d467c02d1`. This adds no public library API.
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
then reserve and execute. Resolve once at a meaningful planning boundary, not
per row, packet, allocation or writer stall. A closure can capture application
context without putting application types in the codec API.

The executable experiment uses this deliberately small interface:

```rust
trait PolicyResolver {
    fn choose(&mut self, alternatives: &[&Offer]) -> Option<usize>;
}
```

This is a private test shape, not the proposed final public signature. An offer
contains a strategy, memory/disk bounds and two metadata effects. `None` refuses;
an invalid index fails. Indices exist only within one synchronous call. The
callback cannot alter an offer or its requirements. Its answer is a selection,
not a replacement for resource admission or backend enforcement.

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

Start with concrete execution selection in the I/O/pipeline layer. Keep
`resolve_color_emit` as a domain resolver, the metadata rewrite `Services` trait
as injected editing/compression mechanisms, and checked fidelity selection as
configuration selection. None currently proves all operation effects. Reuse
them behind the checked boundary instead of wrapping every knob in one universal
policy-event enum. A future application policy object can compose typed selectors.
Async approval/remote negotiation should be added only with a consumer that needs
it; an async callback requirement does not belong in every core codec call.

## Executed experiment and remaining qualification

Run `just io-policy-resolution-check` in the prepared
[standalone harness](../perf/io-validation/README.md#prepare-an-export).
The [implementation](../perf/io-validation/src/policy_resolution.rs) reuses the
existing shared reservation model; [tests](../perf/io-validation/tests/policy_resolution.rs)
exercise conditional small/large decisions, a misleading streaming shape,
already-resident input, unknown/estimated costs, required metadata versus permitted
omission, invalid/refused choices and capacity taken during resolution. Failure
to reserve disk rolls back the RAM reservation; successful reservations remain
held until the admitted plan is dropped.

The decision tests and warning-free Clippy passed. Resource-wrapper observation
for that command, including compilation:

```text
run-heavy: done rc=0 1s | peak-RSS 0.13GiB | min-avail 18643MiB | peak-load 0.09
```

These are authored logical costs, not codec measurements. No 10 GB buffer was
allocated. The experiment covers synchronous finite alternatives and conservative
upper-bound reservation only. It does not implement native cost discovery,
runtime-accounted unknown growth, disk spooling, metadata editing, deferred/stale
choices, process-wide memory enforcement or a production codec resolver.

Before choosing public types, connect selection to the native encoder-retention
experiment and independently checked metadata output. Exercise caps immediately
below/at/above real requirements, exhausted staging, cancellation, writer failure,
non-replayable input and shared storage across jobs. Reuse the transport simulator
for allowed range plans and viewport changes, then validate the selected plans
against real HTTP and disk behavior. Keep resource admission in
[161](https://github.com/imazen/zencodec/issues/161), output waiting in
[160](https://github.com/imazen/zencodec/issues/160), and the consumer contract in
[27](https://github.com/imazen/zencodec/issues/27).
