# Metadata and publication belong to one transcode route

```rust
let requirements = PolicySet::new()
    .require_metadata(MetadataTransfer::DiscardDescriptive)
    .allow_opaque_profiles(false)
    .require_output_schedule(OutputSchedule::FinishFirst);
```

These requirements can now be admitted together by `resolve_transcode` when the
same retained `TranscodeOffer` supplies the metadata, profile and output-schedule
evidence. The caller still passes one `PolicySet` or `BoundPolicy`. The adapter
keeps the actual configured encoder, conversion and publication mechanism
through the decision; it does not reconstruct them from the report.

```rust
// Adapter evidence, filled only for a route it will actually execute.
offer.output_schedule = Some(OutputScheduleOffer::new(OutputSchedule::FinishFirst));
// No memory bound is supplied by that statement.
```

`TranscodeOffer::new` defaults this field to `None`. An existing adapter therefore
continues to refuse a required schedule until it qualifies its route. A required
output-wait bound also refuses when retention is unknown, even for finish-first.
Mandatory admission runs before either direct callback selection or built-in
preferences. Missing evidence and contradictory evidence remain distinct named
refusals.

The shared implementation and adversarial controls are qualified. The native
publication-owner experiment is now qualified as an isolated prototype; its
public convenience API is not adopted. This change does
not add a high-level writer convenience, establish a native workspace bound or
implement deferred requirements across arbitrary question domains. A successful
transcode decision does not authorize a separate bare decoder boundary.

## What remains to prove in the executing adapter

A finished `EncodeOutput` may retain spare vector capacity and arbitrary backend
extensions. Releasing the encoder alone does not establish the retained memory
while writing. The general experiment moves the encoded vector and drops backend extensions
before a writer is introduced. It retains an arbitrary Pulse during publication
and therefore offers no finite wait-memory bound. A test-only fixed-array/NoPulse
variant explores a narrower ownership model; its layout calculation is not a
measured process or stack bound and its copying is unsuitable as the general API.

Production candidates should reuse existing admitted storage and staging, accept
runtime limits, and preserve `Pulse` and typed partial-write errors. A borrowed
control can itself retain data: adding `&pulse` must not turn an owner-layout
calculation into an unsupported whole-route wait bound. Unknown remains unknown.
Disk staging needs separate accounting for replay storage and blocking spool I/O.
Neither mechanism certifies arbitrary opaque metadata as PII-free.

## Executed shared-contract checks

Four new controls failed before wiring schedule evidence into joint admission
and pass afterward. Workspace tests and strict all-target Clippy also pass.
The controls cover direct choice and preset delegation, original offer indices,
unknown and contradictory schedule/wait evidence, unchanged strict workspace
refusal, and continued refusal at a separate final decode boundary.

On the validation host, the Rust value layout of `TranscodeOffer` changed from
88 to 136 bytes. `OutputScheduleOffer` remained 48 bytes and `FrameRouteOffer`
264 bytes. These are `size_of` results, not heap allocation, stack-peak, process
memory or throughput measurements. No throughput comparison was run.

Reproduce with `just transcode-composition-check` and `just policy-layout-check`.
The [receipt](../benchmarks/joint_transcode_2026-10-28.json) records source hashes,
commands, scope and the build/test resource reading.

## Executed native publication experiment

The [standalone reproduction](../perf/publication-validation/README.md) retains
the exact source patches and commands. It passed seven unit and 23 native
controls, with the 16 ordinary outcomes unchanged, and strict Clippy with and
without the feature and with async enabled. The same prepared route admits
metadata and finish-first requirements. Arbitrary-Pulse wait retention stays
unknown; every finite wait cap refuses before selection. Cancellation, partial
writes, flush failure, callback unwind, output overflow and postcondition
refusal retain their existing effects and typed causes. No final API spelling
or whole-operation obligation ledger is established by this experiment.
