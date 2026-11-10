# Structural critique: complexity belongs at the boundary that needs it

This is a design review, not an implemented namespace migration. It changes no
public signatures, defaults, pixel behavior or compatibility promises.

Audited sources: main `1fb997bb`, [PR208 at 23ca16ff](https://github.com/imazen/zencodec/pull/208)
and [the separate PR209 spike at fffac137](https://github.com/imazen/zencodec/pull/209).
Most examples and the proposed cleanup below concern the unpublished review
surface, not crates.io. Historical design notes are evidence of earlier decisions,
not proof that a current implementation has the same gaps.

## Judgment

The data model has necessary complexity. The entry points expose too much of
its organization and history. In particular, another alias path named `advanced`
does not establish a simple application API, and a two-line convenience is not
simple if its caller must independently check whether configuration took effect.

The target is **no unrelated concepts required for the task at hand**. It is not
zero concepts for every task, nor fewer structs at the expense of lost meaning.
A basic decode should not require learning replay, policy tickets or color
carrier planning. A sequential codec should not need an animation implementation.
A layered-image browser really does need identity, hierarchy and composition.

The obvious first changes are a curated root, one canonical path per role,
clear required/preferred method names, and elimination of misleading name
collisions. Facade enforcement and optional authoring protocols need deeper
review; hiding their names cannot fix their semantics.

## Findings, in decision order

| Priority | Finding | Recommended action | Kind of change |
| --- | --- | --- | --- |
| P0 | Short and checked operations use different enforcement paths | Make the ordinary facade the trusted path; retain explicit advanced escape hatches | Behavioral/API decision, with native controls |
| P1 | `advanced` aliases entire existing modules | Establish actual consumer, codec-author and adapter surfaces; choose defining paths and visibility | Namespace/API review |
| P1 | Root exports mix everyday operations, parsing machinery and diagnostic taxonomy | Freeze a small root allowlist and move specialist access under owned domains | API migration |
| P1 | Reusable builders and byte acquisition both use `Request` | Reserve request/demand terminology for protocol messages; name reusable intent as options | Naming/API review |
| P1 | Same `SourcePrecision` name means different facts | Distinguish original sample representation from mutable modeled capacity | Paired naming review; preserve all fields |
| P1 | `with_output` means a requirement, while nearby APIs express preferences | Use `require_*` versus `prefer_*` consistently | Small, explicit API migration |
| P1 | Policy permissions, hard requirements and callback choice look interchangeable | Present one application policy entry; retain their separate enforcement responsibilities | Presentation plus policy review |
| P2 | Simple codec authors inherit execution and animation plumbing | Prototype a provided one-shot adapter over the existing contracts | Bounded real-codec spike |
| P2 | `CodePlane` names overlapping but non-equivalent views | Decide the canonical storage/reader boundary before merging or renaming | Interchange review |
| P2 | Several finish/push/receive protocols have similar verbs | Document state and ownership by role; consolidate implementations where equivalent | Internal refactor first |

P0 is not a claim that the checked path proves an arbitrary backend correct.
It means callers should get enforcement of stated requirements and a typed
refusal when evidence is unavailable, without a capability-query handshake.

## 1. Make the short path the semantic center

Current code already offers the right application shape:

```rust,ignore
let image = codecs.decode(bytes)?;
let encoded = codecs.encode(ImageFormat::Png, image.pixels())?;
```

The problem is what happens after a caller configures it.
[`CodecSet::decode_job`](https://github.com/imazen/zencodec/blob/23ca16ff/src/set.rs)
uses a best-effort policy setter; `checked_decode_job` uses its checked sibling.
`EncodeRequest` similarly prepares jobs through checked configuration.
The source comments explicitly preserve the older convenience behavior.
A caller should not have to discover that distinction from implementation notes.

The existing checked spelling makes the contrast concrete:

```rust,ignore
let encoded = EncodeRequest::new(ImageFormat::Png)
    .with_fidelity(Fidelity::Lossless).encode(&codecs, image.pixels())?;
```

**Recommendation:** route ordinary application operations through the checked
machinery when requirements are supplied. Native defaults remain ordinary defaults;
absence of a strict requirement should permit supported successful routes.
Unsupported *required* behavior should return a named error. Advisory preferences
may fall back and expose the actual outcome. Do not silently retry another codec
or reinterpret samples after an executing route has failed.

This needs before/after controls for inherited versus per-call settings, missing
native support, empty clearing, actual metadata inspection, unchanged source
pixels and error causes. It is not an automatic replacement of one method call.
A checked setter confirms the configured effect; it does not certify an achieved
quality score or whole-container privacy.

The parameter requiring sophistication should be introduced when the story needs it:

```rust,ignore
// Proposed naming only; EncodeOptions is not an exported type today.
let options = EncodeOptions::new(ImageFormat::Png).require_fidelity(Fidelity::Lossless);
let encoded = options.encode(&codecs, image.pixels())?;
```

`EncodeRequest` is the existing reusable builder. Renaming it should not add a
second builder with a second implementation. A custom callback remains optional.
Optional per-call progress/cancellation remains `Option<&dyn Pulse>`; consumers
must not manufacture `NoPulse` to perform an ordinary operation.

## 2. A namespace must reduce the visible choices

The reviewed root has **80 explicit public re-export bindings and 20 public
modules**, counting both inline and out-of-line declarations in `src/lib.rs`.
This counts bindings, including crates/constants/functions, not all descendant
items or 80 independent concepts. The issue is the mixture of responsibilities:
`CodecSet` appears beside ICC extraction, gain-map carrier constants, `Retention`,
`Unsupported`, error classification kinds, and incremental dispatch machinery.

[`advanced.rs`](https://github.com/imazen/zencodec/blob/23ca16ff/src/advanced.rs)
currently contains:

```rust
pub use crate::{collection, decode, encode, frame, source, storage};
```

Those aliases preserve trait identity, which is good. They leave all old surfaces
visible and alias consumer output types along with implementation machinery.
They also omit policy/operation, output-patch and implementation-helper contracts.
Consequently they do not yet form a complete authoring boundary.

A target organization to review, not a promise that these imports work today:

```text
zencodec
  CodecSet, ImageFormat, ImageInfo, Metadata, ResourceLimits
  Orientation, Pulse, SharedPulse, At, CodecSetError
  encode       application options, fidelity and completed encoded output
  decode       application options, completed pixels and result details
  image        selected-output meaning, placement and auxiliary semantics
  collection   application discovery/selection and lazy browsing
  prelude      only traits needed for ordinary concrete-config conveniences
  advanced
    encode     config/job/encoder authoring; plan, rows, erased extensions
    decode     config/job/decoder authoring; rows, incremental, controlled, erased
    collection backend/factory/cursor authoring
    adapter    prepared offers, policy questions, publication and state guards
    source     byte acquisition contracts
    storage    accounted ownership mechanisms
    helpers    codec implementation utilities
  errors       detailed classifications and inspection
```

The root list is a proposed allowlist, not a target item count. Some names already
have published paths. Existing domain modules may remain canonical where that is
clearer; the decision must specify which path the guide and Rustdoc teach.
A compatibility path must not become a competing recommended API.
Do not create a duplicate trait family behind the new spelling.

Runtime drivers stay in `zencodec-io`, the runtime-free coroutine authoring aid in
`zencodec-coroutine`, and rendering in `zencodec-render`. Do not pull Tokio or CMS
execution into core to make imports appear shorter. Modules separate audiences;
Cargo features should separate dependencies/platform facilities, not create a
feature combination for every conceptual submodule.

## 3. Give names a consistent grammar

| Word | Meaning to reserve |
| --- | --- |
| Options | Reusable application intent; constructing it performs no codec work |
| Job | One retained configured native owner before execution |
| Plan | Prepared effects of that exact job, with uncertainty explicit |
| Declaration | Caller/native claims, not verified sample evidence |
| Request / Demand | A protocol message asking the next participant to act |
| View | Borrowed data with a bounded lifetime |
| Storage / Owner | The backing allocation or native mapping owner |
| Summary | Bounded diagnostic data; cannot substitute for the retained target |

Apply this first to the overlapping names with concrete consequences:

| Current names | Why confusing | Proposed direction |
| --- | --- | --- |
| `DecodeRequest` / `EncodeRequest`, versus `InputRequest` and `RowDemand` | Reusable configuration and live protocol messages share a word | `DecodeOptions` / `EncodeOptions`; keep `InputRequest` / `RowDemand` as protocol messages. Review the separate zencodecs transcode builder on the same rule. |
| `DecodeRequest::with_output`, `preferring_output` | The first is strict despite the neutral spelling | `require_output` / `prefer_output`; preserve exact current behavior |
| `image::SourcePrecision`, `zenpixels::precision::SourcePrecision` | Original integer/signed/float representation versus a capacity model updated by composition/conversion | Consider `SourceSampleFormat` and `PrecisionCapacity`; this needs paired review, not an alias or field-dropping conversion |
| `image::CodePlane`, `zenpixels::sample::CodePlane` | One exposes enclosing storage; the other can read a stepped component such as Cb within P010 | Prefer one shared implementation where sufficient. If both roles remain, a name such as `ComponentStorage` must distinguish the aggregate storage from a component-code reader. |
| `OutputInfo`, `DecodeOutput`, `ImageOutput` | Prediction, completed packed decode and selected rich image look like interchangeable outputs | Keep result types qualified; consider `DecodeOutputHint` for prediction. Do not merge their ownership/completeness semantics. |
| `FrameFacts` / `ImageFacts` versus full interpretation/targets | A small copied record can be mistaken for evidence sufficient to execute | Prefer `FrameSummary` / `ImageSummary` for diagnostic snapshots, or equally explicit documentation; never replace target equality with summary equality |
| `strict()` presets | Native feature restriction, strict parsing and exact preservation are different promises | Describe each preset by its domain/effects; `DecodePolicy::strict` currently disables color extraction and several accepted input features, not just tolerant parsing |
| Root `Retention`, `Unsupported`, `PolicyKind` | Their domain is absent at the point of discovery | Put them under their owning domain instead of adding more root synonyms |

These are candidates, not approved renames. The highest-value changes are the
first three rows and removal of ambiguous root imports. Avoid changing every
`Output` to `Result`, every `Frame` to `Image`, or every error name for symmetry.
The review should choose one vocabulary before the breaking target ships.

Several documentation corrections are already clear from source: the config
trait still mentions a per-format encode trait although `Encoder` accepts a
type-erased `PixelSlice`; the root introduction still attributes multi-format
conveniences to zencodecs while also presenting CodecSet. The tutorial should
state the actual division: core registry/operations versus additional concrete
pipeline integration. These are factual cleanup, not reasons for another API.

The current prelude imports generic and erased encode/decode traits, row and
animation interfaces, and control traits. A narrow consumer prelude plus an
explicit authoring prelude would reduce completion/import noise. Merely moving
that same broad list under a new name would not achieve this.

## 4. One application policy entry, several necessary responsibilities

`DecodePolicy`/`EncodePolicy` are native feature permissions. `MetadataPolicy`
filters metadata. `ColorEmitPolicy` chooses carrier treatment. `PolicySet` contains
mandatory offer requirements and preferences. `PolicyResolver` is application
choice; `BoundPolicy` keeps hard requirements outside replaceable callback
wrappers. `OperationPolicy` is the accepted sealed interface.

These are not seven interchangeable policy objects. Presenting them without
explaining their roles makes the caller coordinate implementation detail.
Conversely, folding all seven into one global callback would transfer that
coordination to every codec and weaken the meaning of configuration.

The application should normally supply one configured policy or no override.
A callback should see typed, executable, compliant alternatives only when a
situational decision is needed. Missing late evidence permits discovery where
safe, followed by another decision before affected output. It does not authorize
publication under unknown hard requirements.

PR209 is useful evidence here, not authorization to drop a parameter: removing
an older policy can turn conflicting retention requirements into success.
Eager intersection also changes when conflicting policies refuse discovery.
Keep the operation binding and the current direct-call safeguard while reviewing
single-policy syntax. Require fewer concepts at entry without erasing obligations.

## 5. Make simple codec authoring an actual supported case

The reviewed typed one-shot hierarchy has three traits. Counting required
members gives ten methods and six associated types across `EncoderConfig`,
`EncodeJob` and `Encoder`. That includes a required animation-construction
method even for a still-only codec. Most other methods have defaults; the count
is a description of this surface, not a measure of implementation correctness.

The first authoring story should implement actual encoding, store common job
configuration once, preserve Pulse and native errors, and explicitly refuse
unsupported requirements. It should not begin with a list of every optional
protocol that exists.

A provided one-shot adapter over the current hierarchy is a useful bounded
experiment. It must reuse those contracts, not introduce a second codec API or
a blanket implementation that prevents later custom implementations. Defaults
must not claim metadata retention, progress responsiveness or workspace bounds
that the wrapped native codec cannot establish.

Add row support when a codec can push useful strips; add retained planning when
startup must bind color/metadata before headers; add encoder-driven replay when
native analysis actually revisits input. One-shot support must remain usable.
Splitting the existing trait into many extension traits is not an obvious fix:
it can increase erased forwarding, registration constraints and migration work.
Prove the adapter with a small real still codec and native JPEG before changing
the whole hierarchy.

Thread mobility belongs to the boundary that needs it. Typed `Encoder` does not
require `Send`; registry/worker routes impose their own bounds. Review the
existing animation-associated `Send` bound separately. Moving a non-Send native
owner into a wrapper does not make it portable; construct thread-bound state
inside its worker. No task needs to learn Tokio to implement synchronous encoding.

## 6. Keep these distinctions even when the names are inconvenient

- Current pixel interpretation versus original source metadata/provenance.
- Significant native code bits/shift versus canonical U16 values, modeled
  capacity, and verified current exactness. Resize/filter/CMS can end exactness.
- Packed samples, planar/interleaved components, and arbitrary auxiliary values.
  A color layout is not the meaning of a depth/spot/thermal plane.
- Coded size, visible crop, canvas placement, render size and chroma registration.
  Do not silently clip a raw layer to make it fit a display convenience.
- A picture's storage versus repeated presentations and exact track edit programs.
  Pages, SubIFDs, layers, animation and video are not all frame indices.
- Input acquisition/replay versus output timing, staging and memory held while
  a writer blocks. Streaming byte availability does not prove low raw memory.
- Desired behavior, qualified evidence, and the immutable owner that will execute.
  Bounded reports do not retain or prove everything about that owner.

These should become required learning only in the relevant authoring/adapter
module. The internal framework must still enforce them for ordinary callers.

## 7. Obvious cleanup versus design work

**Safe first pass, with no behavior change:** choose canonical documentation paths;
lead with actual CodecSet one-shots; label every example's role and status;
separate current signatures from proposed ones; remove generic root imports from
examples where a domain-qualified name is clearer. Document strict requirements
beside advisory preferences. Give protocol examples their startup/error/terminal
states. Do not show deprecated setters as the normal next step.

**Small but public decisions:** builder names, `require_output`, source-format
naming, narrower preludes, and the root/module visibility allowlist. Review and
migrate these together. Aliases can bridge existing published paths, but adding
multiple equally prominent spellings for the unpublished target defeats the aim.

**Keep as explicit design gates:** trusted checked facade, simple-codec adapter,
planar still-encoding input, rich image/frame ownership, precision-policy
integration and the broader page/render convenience. A new catch-all `Media`
object, universal options bag or second policy framework is not required by
this critique.

## 8. Acceptance for the structural pass

Use these tasks as the review script. Import/type counts are warning signals;
correct pixels, ownership, effects and error behavior remain the gate.

The historical [June ablation report](public-api/ABLATION-zencodec.md) used a
symbol inventory and treated specification membership as a keep criterion.
That is not sufficient architectural justification. Each exposed concept should
have an identifiable caller task or native requirement and an assigned layer.

| Task | Must be obvious | Complexity that must stay optional |
| --- | --- | --- |
| Decode uploaded bytes | One registry call, owned output, optional control | Byte windows, erased traits, policy tickets |
| Encode ordinary pixels | Destination format and actual output/error | Native carrier planning and finish patches |
| Require metadata or lossless mode | One options object; success or a named refusal | Capability queries and manual post-set coordination |
| Resize a 10-bit source then encode | Current values and allowed loss are explicit | An edit-log engine or a claim that source depth proves exactness |
| Browse TIFF pages/SubIFDs and JXL layers | Lazy discovery and explicit selection/render semantics | Video send/receive machinery for a still-image application |
| Decode video / render edits | Correct presentation/crop and unsupported-operation errors | Packet/mux details for callers using the provided renderer |
| HTTP streaming or local seek-back | Existing drivers own transport/state/publication | Runtime scheduling policy inside every codec |
| Add a synchronous one-shot codec | One coherent authoring path with reusable job plumbing | Animation/replay/async implementations or invented evidence |

For each change, require a real successful path, an unsupported required path,
optional Pulse cancellation/timeout, original typed error propagation and the
relevant padded/precision/native parity controls. Reuse testkit coverage. Do not
turn this into an unbounded all-codec rewrite or treat a prettier import list as
qualification of behavior that was never tested.

This pass should finish with one module map, one naming/migration table and a
small set of compiled caller/codec examples. Public API snapshots should expose
all changes for review. The current document supplies the critique and candidate
map; it does not claim those structural changes are implemented.

## Source pointers

- [Root exports, encode/decode modules and prelude](https://github.com/imazen/zencodec/blob/23ca16ff/src/lib.rs)
- [Current advanced aliases](https://github.com/imazen/zencodec/blob/23ca16ff/src/advanced.rs)
- [CodecSet ordinary and checked paths](https://github.com/imazen/zencodec/blob/23ca16ff/src/set.rs)
- [Reusable operations and required output](https://github.com/imazen/zencodec/blob/23ca16ff/src/operation.rs)
- [Encoder authoring hierarchy](https://github.com/imazen/zencodec/blob/23ca16ff/src/traits/encoding.rs)
- [Native mapped-code and source-format wrappers](https://github.com/imazen/zencodec/blob/23ca16ff/src/image.rs)
- [Shared capacity model](https://github.com/imazen/zenpixels/blob/76005fc/zenpixels/src/precision.rs)
- [Policy/driver spike, scenarios and limits](https://github.com/imazen/zencodec/blob/fffac137/docs/row-policy-spike.md)
