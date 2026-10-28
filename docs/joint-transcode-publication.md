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
publication-owner experiment is still private and unqualified. This change does
not add a high-level writer convenience, establish a native workspace bound or
implement deferred requirements across arbitrary question domains. A successful
transcode decision does not authorize a separate bare decoder boundary.

## What remains to prove in the executing adapter

A finished `EncodeOutput` may retain spare vector capacity and arbitrary backend
extensions. Releasing the encoder alone does not establish the retained memory
while writing. The private experiment copies verified bytes into a fixed owner
and drops the original output before a writer is introduced. That makes a narrow
ownership witness; its fixed capacity, stack use and copying are unsuitable as
the proposed general API.

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
