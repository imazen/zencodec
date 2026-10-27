# Requirement scope follows effects, not question names

Design constraints for [mandatory admission](https://github.com/imazen/zencodec/issues/173).
The local admission implementation filters before callbacks; it does not yet
implement the scoped execution contract described here.

A reusable tenant policy should state requirements once. A question about source
replay should not discard a pending metadata requirement, and it should not have
to certify metadata itself. Conversely, labeling every unrelated question
NotApplicable would let an operation finish without ever enforcing that requirement.

## Check before the constrained effect

| Requirement | Enforcement deadline | Why final publication is insufficient |
|---|---|---|
| Allocation or retained-workspace cap | Before the allocation or retention transition | Refusing after allocating a large raster already violates the bound |
| Exact replay / source pass limit | Before each affected source read | A seekable source can change; an unknown pass count can exceed its initial lower bound |
| Required interpretation | Before publishing affected samples | Later metadata cannot repair pixels already delivered under the wrong interpretation |
| Metadata privacy | Before emitting the affected carrier | A final error cannot recall a private header or opaque packet already sent |
| Output-wait retention cap | Before waiting on the destination | Scheduling alone does not prove what buffers remain alive |
| Seek-back requirements | Before selecting/writing the dependent layout | A forward writer cannot repair a previously emitted head index |

The current workspace and output-wait limits have deliberately narrower scopes
than total operation memory. Do not turn their evidence into a whole-operation
bound. Discovery and preparation can allocate before a final interpretation
question; a future strict allocation contract must qualify those stages too.

## Pending requirements need authoritative state

The execution state should freeze the requirements and bind evidence to the
operation, retained source revision and prepared route. A nested operation or a
stale decision must not discharge its parent. Deferred evidence is permitted only
while the corresponding effect remains blocked. A new source/configuration
invalidates dependent evidence; unknown evidence cannot silently become proven.

`OperationReport` is not this state. It is a diagnostic ring, can omit old
records, and allows callers to append observations. Dropping records, cloning a
report or forwarding a child report must not authorize work. The correctness
state must remain private to the executing operation and survive report wrap.

NotApplicable needs an operation-level reason: the complete operation has no
such effect. Merely encountering a different question domain does not establish
that. A pure pixel result and an encoded destination have different effects;
any convenience API must derive that distinction rather than make callers
coordinate capability queries.

## Minimize intermediate questions

For encoding, prepare source replay, metadata treatment, the configured encoder,
destination access and output scheduling as a coherent route when the necessary
facts are available together. A single callback can then compare executable
routes with their combined effects and costs. Retain the selected objects and
execute them without rebuilding a different plan afterward.

Late streaming facts still require suspension points. Those questions share the
same resolver and authoritative requirements, but enforcement occurs at the
specific effect boundary. A generic Intermediate/Final flag is insufficient.

## Tests required before adopting this contract

Use the existing joint-plan experiment in
[policy/plan.rs](../perf/io-validation/src/policy/plan.rs) and its
[execution tests](../perf/io-validation/tests/policy_execution.rs) as a starting
point. They already exercise foreign-operation, changed-request and stale-context
refusal. Extend the executing native JPEG and IMI loops to cover:

- Deferred metadata while source planning proceeds, followed by refusal before
  the first affected output byte when opaque content remains unqualified.
- Allocation refusal before allocation, even when final output is held.
- Report wrap and nested callbacks without losing or discharging obligations.
- Source/route changes invalidating earlier evidence.
- Partial output and cancellation retaining accurate causes and committed-byte counts.
- One tenant policy across pixel-only, encode, transcode, render and collection
  operations, with explicit justification for every NotApplicable decision.

These are adoption gates, not completed coverage claims. The replay candidate
already demonstrates why source identity evidence must refer to the actual
retained input rather than a transferable promise.

## Fixed-state preparation experiment

`PendingPlan` in the joint-plan experiment now retains the execution selection
and two optional stage selections in private fields. Each accepted stage must
belong to the same operation, requirements and source/route context. Validation
precedes mutation. Finishing consumes this state and delegates final cost/effect
checks to `Plan::combine`; no execution admission exists for an incomplete plan.

This is deliberately conservative: it waits for Execution, Metadata and Fidelity
before permitting any of the experiment's execution effects. It is not a general
implementation of per-effect deadlines or operation-level NotApplicable.

The execution tests exercise real storage admission after report wrap, reject a
child's selection without removing the pending parent obligation, and reject
changed requirements, stale context and duplicates without overwriting accepted
stages. Native codec allocations remain outside this driver's accounting scope.

### Native JPEG witness

[requirement_scope.rs](../perf/jpeg-policy-validation/src/requirement_scope.rs)
retains the actual native metadata rewrite plan until all stages are admitted.
The same callback selects execution before metadata inspection, then metadata and
fidelity after inspection. Mandatory effects are never delegated to that callback.
Twenty intervening decisions evict the execution record without releasing its
pending obligations. Incomplete preparation and a changed source revision both
refuse before the destination receives a byte. Requiring exact metadata alongside
this destructive descriptive-metadata route refuses before the metadata callback.

The positive matrix covers sequential/progressive JPEG with header metadata,
metadata immediately before EOI, and split ICC chunks. The shared testkit supplies
actual GPS EXIF; the independent EXIF reader confirms GPS exists in the source
and is absent from the output. Orientation survives, compressed scan bytes are
identical, and libjpeg-turbo decodes identical pixels. Unknown XMP is removed.
ICC remains opaque and retained: this is not whole-file privacy qualification.

Run `just io-policy-engine-check` and `just jpeg-policy-check <artifacts>` in the
prepared export. Both tests and strict lint passed on 2026-10-27. Wrapper receipts:

- Pending plan: `rc=0 8s | peak-RSS 0.30GiB | min-avail 8571MiB | peak-load 0.51`.
- Native JPEG: `rc=0 8s | peak-RSS 0.57GiB | min-avail 8782MiB | peak-load 0.50`.

These include compilation/testing and are not codec memory or speed benchmarks.
The native witness excludes planning, source and destination allocation from its
fixed-state driver's accounting. Production effect deadlines, arbitrary opaque
metadata qualification and cross-operation NotApplicable remain unimplemented.
