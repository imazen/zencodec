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

The shared implementation and adversarial controls are qualified. The original
native publication-owner experiment below did not establish early workspace
admission or source metadata completeness. Subsequent application work adopted
a retained execution owner; see the current-source audit below. No native
workspace bound or general deferred-obligation mechanism is established. A
successful transcode decision does not authorize a separate bare decoder boundary.

## Original prototype obligations (historical)

```rust,ignore
// Adapter entry: freeze and check both sets before native preparation.
let legacy = request_policy;
let bound = policy.requirements();
legacy.check_workspace(None)?;
if let Some(bound) = bound {
    bound.check_workspace(None)?;
}
// Keep both snapshots for discovery and the later joint decision.
```

The shared `check_workspace` helper is implemented. It reports named typed
`UnmetRequirements` without invoking a callback or inventing an offer. Unknown
refuses every configured limit, including `u64::MAX`. A successful check leaves
the other requirements intact. The native owner must call it before creating
jobs or decoders and before conversion preparation; refusal at its later joint
question cannot undo workspace already used. That native ordering still needs
its own execution controls. The helper's tests and limits are recorded in the
[admission receipt](../benchmarks/workspace_admission_2026-10-28.json).

Final pixel interpretation is separate from source metadata completeness. The
prototype snapshots its output metadata expectation at the final boundary and
ignores ordinary later decode events. The generic decoder contract permits
later EXIF/XMP updates without changing the final pixels. Comparing the encoded
output only against that earlier expectation can therefore miss reported source
metadata. This is a source-level counterexample, not an executed native failure.

An executing production owner must qualify the metadata required by its treatment over
the same retained source, track relevant later metadata/color/orientation
changes, and fail before releasing output when that obligation cannot be met.
An extra scan cannot silently bypass input/replay admission. Copying no source
descriptive carriers may establish `DiscardDescriptive`, but does not remove
color/orientation obligations or certify arbitrary PII removal. `probe_full`'s
name alone is not proof: its trait default may return only a header probe.
Keep the [standalone final-boundary refusal](controlled-decoding.md) unchanged.

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

## Current application owner: source audit 2026-10-29

At [zenpipe 43301b4c](https://github.com/imazen/zenpipe/tree/43301b4cbad056bf5b30256b2063e3d31a5b3377),
`zencodecs::checked_transcode` has a retained `Execution` shared by sync and async
drivers. The historical missing-owner and ignored-event descriptions above must
not be read as its present implementation:

- `Execution::prepare` checks both legacy and bound workspace requirements
  against unknown evidence **before** preparing encoder jobs or starting the
  controlled decoder. Positive native workspace bounds remain unknown.
- `evidence::qualify` retains dimensions, format, color, orientation, density and
  descriptive metadata in an `Epoch` for the selected route. Complete-input
  qualification inspects actual inventory. PNG late metadata is conservatively
  refused; JPEG relies on the pinned full native metadata scan.
- After selection, both metadata events and completion call `revalidate` against
  the actual held decoder's current information. Later reported descriptions
  therefore do not silently disappear from the retained expectation.
- `Execution::finish` keeps the encoded result private while checking output
  limits, inspecting the actual output, qualifying its metadata and comparing
  it with the selected expectation. Publication has its own cancellation and
  partial-write contract; verification cannot roll back earlier external effects.

This is a source audit, not a new runtime or memory measurement. An arbitrary
codec can still violate reporting/completeness contracts, and these checks do
not certify opaque content as free of PII. Source acquisition, preparation,
verification and output-wait accounting must keep their distinct scopes.

The current unconditional EXIF/XMP epoch equality also refuses harmless late
changes on `DiscardDescriptive`. The [native candidate](../perf/late-metadata-validation/README.md)
allows only that discarded effect while keeping interpretation and preservation
checks strict. Its production adoption requires approval of the existing
blanket-refusal expectation. No caller capability query or extra callback is
needed to permit an effect the selected encoder never performs.

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
