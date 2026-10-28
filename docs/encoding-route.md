# Joint encoding-route admission

The review API adds `operation::resolve_encode_route` for one retained configured
`EncodeJob` and complete acquisition × output alternatives. It reuses
`EncodeDeclaration`, `EncodePlan`, `OutputScheduleOffer` and `OperationPolicy`.
It does not implement source acquisition, native preparation, replay execution or
publication. Those belong to the driver; native integration remains unqualified
by this source-only change.

## Caller policy

```rust
use core::convert::Infallible;
use zencodec::{At, operation::{
    BoundPolicy, Decision, OutputSchedule, PolicySet, Question,
}};

let requirements = PolicySet::new()
    .with_max_source_passes(1)
    .require_exact_replay(true)
    .allow_raw_memory_staging(false)
    .require_output_schedule(OutputSchedule::FinishFirst);
let policy = BoundPolicy::new(requirements,
    |question: Question<'_>| -> Result<Decision, At<Infallible>> {
        match question {
            Question::EncodeRoute { plan, choices, .. } => {
                // Only mandatory-admitted routes appear here. The plan is the
                // retained effective job's plan, including borrowed carriers.
                let _native_input_contract = plan.input;
                let _number_of_admitted_routes = choices.len();
                Ok(Decision::UsePreset)
            }
            _ => Ok(Decision::UsePreset),
        }
    });
```

Pass that policy to an integrating driver. This combination permits direct row
streaming or a borrowed raster when the retained native plan proves one pass; it
refuses direct multipass reads and raw RAM staging. It does not force a codec to
become one-pass or secretly change quality.
A caller can relax its own requirements explicitly, before starting an operation.

## Adapter boundary

```rust
use zencodec::{At, Pulse, encode::{EncodeDeclaration, EncodePlan}, operation::{
    DecisionRecord, EncodeEvidence, EncodeRouteOffer, OperationPolicy, PolicyFailure,
    PolicySet, resolve_encode_route,
}};

fn admit_retained_routes<R: OperationPolicy + ?Sized>(
    declaration: &EncodeDeclaration<'_>,
    plan: &EncodePlan,
    evidence: EncodeEvidence,
    prepared_offers: &[EncodeRouteOffer],
    policy: &mut R,
    legacy_requirements: PolicySet,
    pulse: &dyn Pulse,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>> {
    resolve_encode_route(declaration, plan, evidence, prepared_offers,
        policy, legacy_requirements, pulse)
}
```

The implementor creates `EncodeEvidence::new(checked_encoder_offer)` once for the
retained job. Its `metadata` and `retains_opaque_profile` fields default to unknown;
fill them only from qualified effective-job evidence. This common value reaches
all admission checks, the callback and the report as `facts.evidence`. Acquisition
and output choices contain no metadata/profile fields, so selecting a route
cannot select a different treatment for the same job. Workspace and range remain
route-dependent.

The implementor prepares these offers from retained private slots. For example,
an actual held immutable `PixelSlice` can supply
`InputAcquisition::BorrowedRaster` plus `ReplayEvidence::ImmutableBorrow`.
A caller-implementable source flag cannot supply that evidence. Private raw RAM
can supply `MemorySnapshot` plus `PrivateSnapshot`; disk/positional sources with
hash or identity checks supply `VerifiedHash` or `VerifiedIdentity`. The latter
two never establish exact samples across repeated reads. When the retained native
plan proves at most one pass (`Rows`, `WholeRaster`, or `Replay` with maximum one),
no replay occurs and the requirement needs no snapshot or replay evidence. There
is no caller-supplied `NotReplayed` flag. Unknown acquisition still refuses even
for one pass or with no policy.

The driver snapshots legacy and bound requirements at operation entry and calls
`check_workspace` on both before any covered native preparation. It prepares the
same configured job, complete source declaration and destination access, with
admitted storage reservations for the executable alternatives. The joint resolver
snapshots its policy inputs before its callback, enforces their conjunction, and
returns the original index without cloning a filtered offer list. At most 64
alternatives are accepted by the shared selector.

After selection the driver retains the chosen slot, revalidates source and
output access, and consumes the same job once. It compares `begin_declared`'s
complete plan against the retained plan, including ICC bytes, input protocol,
output timing and output phase. It must validate sample-depth assertions on
actual supplied rows and enforce pass/final-pass protocol during execution.
A decision record is not a transferable proof and cannot authorize reconstruction,
reconfiguration, retry or another source. Preserve the record and original typed
source/native/control error if execution fails.

## Evidence and limits

| Requirement | Evidence admitted at the joint question |
|---|---|
| Maximum source passes | Maximum external traversals derived from the actual acquisition and native plan; unknown refuses any finite cap |
| Exact replay | A retained native plan proving at most one pass; otherwise an actual immutable raster borrow or private raw snapshot, with hash/identity/unknown refusing |
| No raw RAM staging | Known acquisition other than `MemorySnapshot`; compressed staging is independent |
| Metadata | Exact requested `MetadataTransfer` qualified for the effective job |
| Opaque profiles | Explicit `Some(false)` when exclusion is required |
| Workspace | Qualified additional codec/conversion workspace including preparation/verification, excluding managed stores and source storage |
| Extended range | Explicit complete-route preservation plus confirmed lossless encoder fidelity |
| Output schedule | The selected route's `OutputScheduleOffer` |
| Output wait memory | Explicit qualified bound covering all route-retained memory while the destination waits |

A direct route uses the native maximum; staged acquisition reads the external
source once even if the native maximum is unknown. Zero rejects every route that
reads source pixels. A staged one-pass claim does not prove native termination or
bound internal computation. Freezing a first traversal does not prove that a
mutable external source represented an atomic historical snapshot.

`FinalPass` restricts which native pass can emit bytes. `OutputTiming` counts rows
within an emitting pass. Neither implies a retained-memory bound. Live scheduling
requires streaming timing; a whole-raster plan cannot claim streaming. A raw disk
row store cannot directly satisfy a whole-raster protocol. Strict wait limits
still require evidence under finish-first scheduling.

A one-pass exemption never clears source-pass limits, structural compatibility,
metadata, workspace or output obligations. In particular, cap zero still refuses.
One external staging traversal does not prove one native pass; a staged route
with unknown or multiple native passes still needs exact replay evidence.

Invalid shared plans refuse the whole question with `DecisionCode::InvalidOffers`
and the original typed `EncodePlanError` directly in the cause chain, even when
no alternatives were supplied. A valid plan with an incompatible acquisition/output
alternative instead filters that alternative and records
`PolicyRequirement::EncodeRoute`; other compatible original slots remain selectable.

`UnmetRequirements` uses `u16` masks. Existing named requirements remain in order;
`SourcePasses`, `ExactReplay`, `RawMemoryStaging` and structural `EncodeRoute`
compatibility are appended. Other question domains cannot discharge these new
obligations. Discovery may continue, but bare final interpretation adoption
refuses them. No requirements are cleared to make a subsequent question pass.

## Migration and cost

Existing non-multipass/default call paths are unchanged by this module. Integrators
replace separate replay and output choices with this joint question, preserving
legacy and bound requirements. There is no second multipass job hierarchy or
fake transcode offer. No fields are added to existing caller-constructed structs;
new offers have constructors with unknown evidence defaults. Custom callbacks
can handle the new non-exhaustive question or delegate to the preset.

One configuration is shared by all alternatives, so lossless preference cannot
rank its acquisition methods differently. Metadata and fidelity are facts of the
actual effective job; accepting a setter alone is not output qualification.

The common selector retains bounded copyable facts and scans offers through a
bit mask. The question borrows the ICC-bearing plan; reports contain no ICC
payload, source owner, path or callback text. This adds route compatibility and
mandatory-evidence checks; it does not allocate a raster or clone a carrier per
offer. No runtime, throughput or memory measurements were performed. Retained
jobs and source reservations are still the driver's responsibility, which is the
main integration obligation this API cannot make automatic.

## Qualification

This change was prepared source-only. Rust formatting/parser checks are the only
executed checks; compilation, tests, Clippy, native parity and driver admission
order remain unrun. The regression source covers caps 0/1/2/unknown, immutable
versus weaker replay evidence, coupled choices, original slot preservation,
callback exclusion, bound+legacy conjunction, metadata/workspace/range evidence,
raw versus compressed staging, final-phase/timing, wrong-domain refusal, plan-derived one-pass exemption, typed
invalid-plan causes and shared-job metadata admission across route alternatives.

Run these commands serially in an authorized heavy slot from the integrated
workspace (capture full logs under the operator's durable scratch directory):

```sh
cargo test -p zencodec --lib operation::encoding_route
cargo test -p zencodec --lib operation::policy
cargo test --workspace --all-features
cargo test -p zencodec --doc
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Apply the build host's resource controls to each command. Standalone native driver
qualification must additionally prove refusal before preparation/source reads,
retained job identity through startup, exact sample parity, failed-fill handling,
original causes and cancellation before final destination effects.
