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

The native failure control also cancels before publication and after the first
accepted two-byte chunk. Both preserve the timeout reason and the exact accepted
prefix. A writer failing after three bytes preserves its original BrokenPipe
error and leaves exactly that output prefix. These counts are observed at the
test destination; this witness does not add a production failure-report API.
The expanded native suite and strict lint passed:
`rc=0 3s | peak-RSS 0.53GiB | min-avail 9276MiB | peak-load 0.29`.

## Late discarded descriptions: native candidate

The [application experiment](../perf/late-metadata-validation/README.md) finds a
safe-success candidate narrower than general deferred obligations: when the
retained route already discards descriptive metadata, late EXIF/XMP byte changes
need not invalidate it. Actual PNG output is identical to the clean-input route,
including independent pixel readback; late rendering orientation must survive
or cause refusal. Other interpretation changes and metadata preservation remain
strict. No additional callback is needed to reapprove an effect the retained
route cannot perform.

The 29-test native candidate and strict Clippy pass, but an unchanged legacy test
requires blanket rejection. Production remains unchanged pending approval of that
specific contract change. This does not authorize unknown opaque profile retention
or discharge arbitrary requirements at unrelated boundaries.

## Current-main admission audit (2026-10-29)

At main `ab29b04c`, `operation::policy::choose` filters offers against both the
bound requirements and legacy preset before constructing callback choices.
Original offer indices and retained owners survive filtering; `UsePreset`
ranks the same admitted set. A direct callback choice cannot bypass these
requirements. Failed admission records named Unknown/Contradicted evidence.

`just operation-policy-check` passes in an isolated export: 12 policy tests,
6 request tests and 45 operation unit tests. These include the original-index
63 control, all-excluded refusal before callback, required-vs-preferred output,
stale/nested tickets, callback error causes, cancellation and owner cleanup.
Wrapper rc0, 22s, peak RSS 0.46 GiB, minimum available 13805 MiB. The initial
export lacked the ignored root Cargo.lock and refused under --locked; validation
then used the inspected root lock (SHA256
`8201e9dd6075c980b9825edbe4513205ae29981860a7a5aeca588f10ed8f46d1`).
No dependency resolution or test threshold was relaxed.

This closes the direct-selection bypass described in issue 173, not its wider
scoped-obligation design. Unrelated questions still cannot certify absent
metadata/workspace facts. Deferred success requires the retained execution
owner and effect deadlines described above; callback filtering alone is not
native metadata completeness or an operation-wide resource guarantee.
