# A minimal 0.3: explicit guarantees, small execution contracts

Proposal, 2026-10-25. Source reviewed: `a9af44d1445404f5bd5daca8351fe85cbf1ba22b`.
The workspace manifest is still 0.1.27; “0.3” here names a proposed coordinated
API target, not a version bump, approved removal, or publication. The existing
[queued breaking changes](../CHANGELOG.md#queued-breaking-changes-3) and
[metadata boundary](zenmeta-design.md#next-breaking-release-target) remain the
migration record. This proposal adds priorities and experiments; it does not
silently replace that record.

The aim is fewer independent promises and implementation paths. Keep the useful
consumer calls and the current byte/session protocols. Concentrate changes at
admission, interpretation, storage ownership, and independent-image selection.
“All modern needs” means a contract that can express these workflows and refuse
unsupported configurations accurately. It does not mean every codec implements
every feature or that the core contains a transport, scheduler, CMS, and editor.

## What current main actually has

- `CodecSet` registration, detection, probe/decode/encode/transcode and job escape
  hatches; config/job conveniences. The [compiled examples](examples/journeys.rs)
  exercise these through native QOI/BMP. Keep them.
- `IncrementalDecode` and driver-level `ByteStepper` reuse `Input`/`Request`.
  `ReadAt` serves synchronous sources; `AsyncSource` serves async adapters.
  Positional disk reads and prefetched-range reuse have already landed.
- Rows use caller-provided storage, with a separate opt-in provisional-pass path.
  `StreamingDecode` instead yields borrowed codec-owned strips from input bound
  at construction. Those ownership models are not interchangeable for free.
- Video sessions, demux/mux contracts, owned frames, rational durations, clocks,
  configuration epochs, drain/reset and output patching already exist. Pages and
  pyramid enumeration do not become available merely because sequence and
  supplement flags can describe their existence.
- Checked `try_set_policy`/`try_with_policy` already exist alongside best-effort
  setters. A smaller release can retire the ambiguous alternative rather than
  invent another checked setter.
- `IoLimits`, checked buffer-policy selection and a CPU work quota are still
  proposals. The private admission implementation is useful evidence, not a fix
  to the existing production driver. See [161](https://github.com/imazen/zencodec/issues/161).

The [previous ablation report](public-api/ABLATION-zencodec.md) is historical.
Its June caller counts are not a current migration proof. Being named in a spec
also does not establish that an interface earns its implementation cost.

## The release boundary

| Owner | Minimal responsibility |
|---|---|
| `zenpixels` | Strided packed/planar views, storage/sample precision, current color/alpha interpretation, and conversion primitives. No parallel sample vocabulary in zencodec. |
| `zenpixels-convert` | Conversion planning and execution: layout, transfer, gamut, depth, alpha and explicit CMS/HDR paths. Adapters compose these with codec contracts; zencodec's core does not gain a mandatory conversion dependency. |
| `zencodec` | Image config/job/convenience calls, registry, incremental byte demand and image output contracts, checked operation requirements/results, lightweight pixel-interpretation metadata, standard gain-map contracts and time vocabulary. |
| `zencodec-io` | Slice/file/stream/async drivers, bounded input admission, work scheduling and explicit output staging policy. HTTP implementation and cache policy can remain adapters. |
| `zencodec-media` | Packets/tracks, independently owned pictures, sessions, demux/mux, time/epoch/seek and patch contracts. Keep it optional. |
| `zenmeta` | Rich EXIF/XMP/MakerNote editing, filtering and audits. Codec framing remains in codecs; codec-facing request/result types do not depend on zenmeta's editing tree. |
| `zenpipe` / application | Resize, conversion, composition, viewport priorities, transport/cache choices, shared scheduling and cross-codec selection policy. |

Do not move the format-selection engine into the core merely because it is
`alloc`-only. The duplication findings in
[122](https://github.com/imazen/zencodec/issues/122) warrant deduplication;
shipping a default ranking of formats is a separate product-policy decision.
Custom-format allowlists must work, but that does not by itself require a new
core `FormatSet`: current `ImageFormat` already has equality and hashing, including
Custom definitions. Test a consumer allowlist over that identity first
([121](https://github.com/imazen/zencodec/issues/121)).

### Conversion at the checked boundary

The consumer requests a destination interpretation and permitted loss; the
pipeline adapter qualifies the source, builds a zenpixels-convert plan and
checks that the selected encoder can signal the converted pixels. A policy
resolver can choose among executable routes, or refuse. It cannot make an
unknown source interpretation trustworthy, turn an unsupported CMS transform
into a relabel, or authorize contradictory old metadata over new pixels.

The [native JPEG recipes](pixel-conversion.md) demonstrate buffer conversion
for display and prepared row conversion before encoding. These still allocate
whole-image destinations. The production extension must count those destinations
alongside the input, codec state, frame queues and encoded output. Row conversion
alone does not establish bounded streaming. Keep high-level composition in
zenpipe / application adapters and expose explicit conversion as an operation;
do not silently widen the current CodecSet::transcode contract.

## Caller contract: required effects, preferences, and checked results

The primary consumer path should be **record intent -> execute -> success or a
typed error**. Capability queries are optional discovery for UI and format
selection. They are not a prerequisite for safely calling an operation, and
query-then-set-then-read-back is not how a caller establishes correctness.

A builder can remain infallible when it only records a request. It must retain
every supplied requirement. The shared execution boundary validates and resolves
the whole request automatically, using the actual codec configuration and input;
it must not discard a field because a backend does not implement its setter.
Malformed values can fail earlier. An optional prepare operation uses that same
validation path and binds the resolved configuration; it is not a second,
weaker boolean checklist or a guarantee that later input/I/O cannot fail.

Use two semantic classes, not a matrix callers have to reconstruct:

- A **requirement** must hold for successful completion. An unsupported or
  conflicting requirement produces an error. A plain method that appears to
  select an output or set an exact value should have this meaning.
- An explicitly named **preference/hint** permits documented alternatives.
  Outcome information says what was selected or ignored. A warning/report must
  never downgrade a failed requirement into successful output.

Defaults and named presets are complete, documented requests. Ordinary callers
choose one preset and execute; specialists override selected requirements.
Result reports are useful for inspection and authorized approximation, but
callers need not inspect them to discover whether their requirements held.

Metadata needs distinct promises even when presets make them convenient:

| Requested effect | Meaning of successful completion |
|---|---|
| Retain specified supplied fields | Their required semantics are present in the destination. A target lacking the carrier must refuse. Absent source fields are not invented; requiring presence is a separate explicit request. |
| Preserve selected packet bytes exactly | Those bytes remain identical. A necessary orientation rewrite, carrier limitation or conflict with correct interpretation is an error unless the caller permits that change. |
| Remove specified fields | Those fields are absent within the declared operation scope. An opaque record that prevents proving removal must be dropped when allowed, or cause refusal. |
| Retain what the target supports | This explicitly permissive request may omit unsupported metadata and reports omissions. It must not be named “exact preservation.” |
| Preserve rendering interpretation | The encoded interpretation matches the current pixels, including authoritative color and residual orientation. Descriptive metadata cannot override that interpretation. |

Removing selected fields from a `Metadata` record is not a whole-file privacy or
anonymity guarantee. Thumbnails, auxiliary images, opaque packets and identifying
ICC text need the appropriate codec/editor scope. If required copyright retention
and required private-tag removal cannot both be established for malformed EXIF,
return an error; silently dropping the entire EXIF packet would satisfy only one
requirement. Current `Metadata::filtered` intentionally drops such a packet under
partial policies, so its result alone cannot certify both requirements.

The existing `PreserveExact` name is too strong for the complete encode operation:
filtering may reconcile orientation, and a codec may omit unsupported carriers.
The breaking design should name semantic retention, byte-exact packet retention,
and permissive retention distinctly. Do not silently change the published
variant's behavior during migration.

The same rule applies outside metadata. Required lossless coding, orientation
application, frame selection, canvas size, loop count and gain-map delivery must
be fulfilled or refused. An approximate quality target remains approximate; a
preferred streaming mode remains optional. A crop optimization hint may be ignored,
whereas selecting a particular output region must produce that region or fail.
A setter/getter match proves only configuration selection, not content-dependent
losslessness or achieved perceptual quality.

Validate what is knowable before consuming non-replayable input or emitting bytes.
Rejected mutable configuration must remain unchanged. Content-dependent and I/O
failures can still happen later. For streaming output, a successful begin is not
successful completion: late failure invalidates the partial artifact. Drivers
should stage/commit or abort where their destination supports it, with separately
admitted staging capacity. A raw writer cannot promise rollback. Do not automatically
retry a failed streaming operation after publication or without replayable input.

Keep caller recovery typed and small: invalid/conflicting request, unsupported
requirement, resource limit, and execution failure (input/I/O/cancellation), with
the failed requirement and native cause available as structured detail. A policy
that permits alternatives can let the framework select among them before
irreversible work; the caller should not write a capability-coordination loop.

The implementation must centralize request storage, admission and metadata/color
resolution, then give backends resolved input. Generic, erased, registry, one-shot,
row and animation paths use that same machinery. Default no-op setters are not
an implementation of required effects. Conformance tests must check actual output
with independent metadata/pixel oracles where applicable, not just returned flags
or the backend's own report.

First tests for this boundary: an adapter that ignores requested metadata; a target
with no requested carrier; malformed EXIF requiring both removal and retention;
conflicting ICC/pixel interpretation; unsupported loop/frame/gain-map settings;
and failures before and after output starts. Required cases must either produce
the specified result or return a typed error. Permissive counterparts must report
the actual deviation. Run the same fixtures through generic and erased paths.
These are proposed acceptance tests, not claims of new completed native runs.

The [private qualification matrix](policy-scenarios.md) now supplies native
QOI/BMP typed/registry evidence, extracted EXIF checks with an independent reader,
and callback/writer failures with retained decisions. Color/HDR/gain-map and
animation effect guarantees still require their native qualification. No public
setter has been removed by these experiments.

Requirements can be conditional without becoming unchecked: specify permitted
alternatives and choose among them using operation-specific facts. The
[situational policy proposal and executable resolver experiment](policy-resolution.md)
separate selection from reservation, distinguish bounds from estimates, and
explain why streaming shape alone cannot decide memory or output-wait policy.
The tested shape uses `Result<Decision, At<E>>`, one configured fallback, and
profiles that bind requirements with preferences. A stored request supplies the
one-call path; custom code remains optional. Expected refusal, application failure
and native failure retain distinct codes, causes and traces.

## Remove or replace in the breaking migration

| Current surface | Proposed treatment | Reason and migration gate |
|---|---|---|
| Best-effort `DecodeJob::with_policy`, `EncodeJob::with_policy`, erased `set_policy` | Retire for correctness requirements; use the existing checked setters. Split parser compatibility, metadata return and content admission. | Current defaults may ignore the policy. Rejection must leave a mutable job unchanged. Metadata extraction must not disable interpretation needed for correct pixels. |
| Best-effort `with_fidelity` / `with_lossless`, and exact-looking default no-op options such as loop count or frame selection | Store required intent and validate at execution; explicitly name permissive tuning hints. | Configuration read-back alone cannot certify the requested output. Preserve approximate quality semantics without allowing silent target substitution. |
| Caller-facing unfiltered `with_metadata` / `set_metadata`; owned EXIF editing and `Metadata::filtered` in core | Move editing/filtering implementation to zenmeta. Give the codec validated, explicit metadata/interpretation input through a reviewed replacement. | These are also implementor primitives today: deleting them without replacing that boundary breaks `with_metadata_policy`. Preserve simple byte carriage, cheap orientation/color reading and ISO gain-map interpretation without zenmeta. |
| Millisecond-only `push_frame` requirement and lossy duration getters | Make `FrameDuration` the primary contract. Keep exact millisecond constructors/conveniences over it. | `push_frame_timed` already exists and reports actual duration. Preserve cumulative quantization, explicit exactness, zero-duration semantics and rejection without state advancement. |
| Animation `render_next_frame_to_sink -> Option<OutputInfo>` | Return frame timing/index as well as output facts. | Current signature loses timing on this path; already queued in the changelog. Use the same timing authority as the borrowed/owned frame paths. |
| `BufferPolicy::for_pattern(...) -> Self` | Checked selection returning `Result`, with private validated configuration where useful. | Sequential selection currently picks a 64 KiB window regardless of a smaller cap; backtrack selection can silently clamp below the codec requirement. Refuse impossible admission before I/O. |
| `DriveOptions` as the whole resource policy; gap-only coalescing | Retain performance hints separately from byte/descriptor/in-flight limits. | `read_size` and `max_gap` cannot bound live storage or overfetch. Bounds must include a codec's minimum contiguous request and retained/in-flight windows. |
| Deprecated ICC helpers, threading aliases, ambiguous gain-map fraction/format aliases | Remove after compile-tested consumer migration. | The replacements are already named in the queued-breaking list. Do not remove useful color queries or standard gain-map data with the aliases. |

The current `CodecSet::transcode` is intentionally still-only and refuses
sequence, auxiliary-image and incompatible-descriptor cases. Keep those guards.
It still calls the best-effort `job.set_policy` for color; migrating the facade
to the checked boundary is part of this work. Do not make a color-converting,
resizing, flattening pipeline an implicit side effect of this small convenience.
The QOI/BMP descriptor refusal is an adapter finding in
[zenbitmaps 28](https://github.com/imazen/zenbitmaps/issues/28#issuecomment-6392289457),
not evidence that the guard should disappear.

## Candidates that need an experiment before removal

1. **Independent one-shot, push, pull-strip and incremental implementations.**
   Aim for one native core per codec with shared wrappers for conveniences.
   Compare that with the current paths on sequential, progressive, whole-frame
   and tiled codecs. A `StreamingDecode` wrapper over push callbacks may need
   buffering, suspension or another allocation; do not promise free replacement.
   A whole-file bridge remains a useful integration tool, with its buffering
   disclosed and forbidden when a bounded operation requires native resumption.
2. **`Encoder::encode_from`.** Its callback returns `usize`; it cannot directly
   return an input error or distinguish one from EOF. Prototype a fallible row
   provider and an adapter over `push_rows`. Keep a separate native pull hook
   only if measured replay/allocation behavior needs it. Do not discard source
   errors or imply that a synchronous callback can suspend for network input.
3. **`Encoder::encode_srgba8`.** This permits destructive adaptation of mutable
   input; `encode(PixelSlice)` is not an equivalent replacement. Compare a typed
   mutable pixel-view entry point before removing the specialization.
4. **Incremental construction only through `Box<DynIncrementalDecoder>`.**
   Measure small independent-tile workloads before adding a typed factory or
   another required associated type. Preserve a generic path where it pays,
   with erasure at the registry boundary rather than mandatory per-step boxing.
5. **Two step facades.** `ByteStepper` already adapts image decoding and demuxing.
   Keep it in the driver layer unless a concrete consumer proves that merging it
   into `IncrementalDecode` removes real work without coupling unrelated outputs.

These are source-based design candidates, not measured performance defects.
Names and trait counts alone are not the acceptance criteria.

## The small additions that earn their place

**Operation admission.** Keep static capabilities for discovery. A checked result
for the exact backend, configuration, input interpretation and requested operation
must disclose native versus whole-file fallback, input access/lookback, required
pixel replay, output timing/patches and retention facts. Estimates remain estimates;
unknown memory or retention does not become a guaranteed bound. Existing
`AccessPattern`, `OutputTiming`, `MuxTiming` and resource estimates are inputs to
that answer, not competing replacement enums.

Separate requirements from hints. Required precision, alpha, metadata, exact timing,
ROI work bounds and output layout either hold or are refused explicitly. Tuning
preferences may be ignored with an observable outcome. Content-dependent fidelity
can require analysis and a result-side report; a config-time match cannot prove
losslessness for arbitrary input ([104](https://github.com/imazen/zencodec/issues/104)).
Admission can be staged after required headers arrive through the existing byte
protocol. It must not require an upfront scan of an entire remote object. Recheck
facts that change during discovery or at a page/configuration epoch. The
[native JPEG probes](policy-jpeg.md) can return a partial ICC as successful
metadata, so method names and HeaderReady cannot certify interpretation.

**Scoped finality and suspension.** Keep pending/provisional/final/unavailable
facts distinct and name the scope that is final. A mandatory decision barrier
must precede affected conversion, encoder setup and publication. Waiting needs
bounded admitted storage; replay or a fresh media segment is an alternative only
when implemented and permitted. The same typed resolver handles new relevant
facts. Complete disclosure inventory remains separate from rendering authority.
Do not encode this as another ignorable Event variant. See
[166](https://github.com/imazen/zencodec/issues/166) and the
[private suspension controls](policy-transcoding.md).

**One encode interpretation boundary.** Validate the submitted pixels' current
color, precision, alpha and orientation against the actual target. Keep source
provenance and descriptive metadata separate. Resolve realizable carriers before
irreversible output, then report what was written. Reuse the existing color
resolver/zenpixels finalization work; prove its seams with
[154](https://github.com/imazen/zencodec/issues/154). HDR sample support and optional
mastering/light metadata are separate capabilities; [158](https://github.com/imazen/zencodec/pull/158)
implements that distinction and still needs review/landing. Interpretation
availability, final pixels, and fully validated file completion are separate facts.

**Small I/O controls.** The first review should implement the already proposed
`IoLimits` (live input bytes, descriptors, in-flight requests), typed driver-owned
limit failures, checked `BufferPolicy`, and a CPU-fairness `WorkQuota`. A quota
cannot preempt an indivisible native call. Keep two explicit output policies:

- Verified sequential operation with bounded raw/intermediate state and a pausable
  input: bounded output-coupled progress.
- Buffered or unknown retention: admit encoded staging and job concurrency, finish
  the useful unit to release raw state, then allow the writer to wait.

Both paths account for caller-owned raw pixels and independently bound staging.
There is no universal `streaming = bounded memory` inference
([160](https://github.com/imazen/zencodec/issues/160)).

A [nostdio-style bridge](https://docs.rs/nostdio/0.2.0/nostdio/) belongs at an
actual synchronous pull-parser boundary. Keep the byte-demand protocol for
network suspension, range completion and driver admission; adding another
Read/Seek family to the core does not supply those semantics.

**Storage ownership that survives handoff.** Input cache eviction, packet acceptance,
decoder reset and presentation completion are different release events. Current
`PacketPayload` is Vec or an Arc slice and cannot carry an arbitrary reservation
owner. Prototype an owner-retaining payload integration, or a conservative explicit
budget transfer, before promising global accounting. Keep direct borrowed/reused
buffers for disk and resident bytes. `Frame`/`Picture` already preserve picture
ownership across reset; do not replace them with mandatory copied rasters.
A CPU mapping seam is not evidence of a working GPU import/export adapter. An
actual backend must validate lifetime, synchronization and copy behavior. The
[WebCodecs resource model](https://www.w3.org/TR/webcodecs/#raw-media-memory-model-reference-counting) is useful
comparison material for shared storage lifetime, not an API to copy wholesale.

**Independent image selection.** Prototype one optional collection/selection
extension for pages and alternate representations: stable item identities,
per-item info/metadata, incremental enumeration, level/preview relationships,
and a selected region's coordinate system. Do not require a known page count or
pretend page discovery is always local/constant-cost. Enumeration and decoding
should use the existing byte-demand protocol. A selected region can emit ordinary
rows; independent tiles can complete in arbitrary order at the application layer
without weakening the ordered final-row contract for one decoded image.

Use the IMI explorer and TIFF as the two consumers before choosing public names.
A complete dependency plan is distinct from `requested()` alternatives, and
includes dictionaries/references/checksums. Separate object version from viewport
or seek generation. Reuse verified cached bytes across view cancellation while
rejecting stale view delivery. Mixed pages must retain their own dimensions,
precision, orientation and interpretation. The design also needs an incremental
page writer; whole-collection accumulation is not an acceptable implicit default.
This is the remaining substantive image-family gap in
[1](https://github.com/imazen/zencodec/issues/1) and
[131](https://github.com/imazen/zencodec/issues/131).

Source precision and signed/float scientific data belong first in zenpixels.
Current media `PlaneSamples` supports U8/U16 only; do not claim arbitrary float
or scientific-plane interchange from the existing media tests. The duplicated
media `SampleEncoding` should converge on the canonical zenpixels type after
released-dependency and consumer checks. Arbitrary science schemas, viewer
geometry policy and codec-specific tuning remain extensions. If bounded-error
near-lossless fidelity is shared, define its exact sample domain, per-channel
units and floating-point edge cases with real encoders before freezing it; an
approximate perceptual target is not that guarantee.

## Next experiments, in order of decisions they unlock

| Work | Actual implementation/test | Decision gate |
|---|---|---|
| 1. Production input admission | Integrate the private planner/reservations into sync and async drivers; exact-cap EOF, oversized minimum request, lookback, overflow, allocation failure, cancellation and window/batch reuse tests. | No read or allocation starts beyond admitted capacity. Unsupported requirements refuse before consumption. Borrowed-slice path gains no mandatory allocation. Submit the small I/O API review here. |
| 2. Encoder retention and output policy | Run sequential and buffered native configurations with slow/short/failing writers, caller-owned rasters, admitted encoded RAM/disk staging and concurrent jobs. Retain the existing ownership control. | Choose policy by measured storage lifetime; no deadlock when staging fills. Record time to raw release, exact output, allocator peak, occupancy and throughput. CPU fairness remains independent. |
| 3. Explorer over a real transport | Reuse the IMI simulator for deterministic plans, then serve actual IMI/TIFF selections over a fault-injecting HTTP adapter. Test wrong/missing ranges, ignored Range/200, object changes, decompression, retry, rapid pan/zoom and cancellation. Include disk and resident-source controls. | Same selected pixels and interpretation; no stale publication; bounded overfetch/storage; no forced full raster or full remote-file scan. Decide collection/selection and complete-plan shape from both codecs. |
| 4. Canonical codec execution | Adapt JPEG sequential/progressive, PNG/interlaced, a whole-frame codec, and tiled decode to the candidate common execution path. Compare current native convenience/pull paths and full strided pixel/color/metadata outcomes. | Keep or remove execution hooks based on allocations, copies, first useful rows, total work and actual migration code. No representative codec silently changes pixels or buffers a frame to satisfy the abstraction. |
| 5. Complete media pipeline | Rebase/execute zenpipe 86 with native AV1 and MP4/WebM/IVF, packet/frame ownership, delayed/reordered output, configuration changes, seek/reset and retained frames. Include an unread second track and a stalled destination. | Bounded aggregate ownership with no head-of-line storage starvation; all selected tracks preserved; timestamps and independent-reader output match. Qualify image-animation bridges separately from byte-fed input. |
| 6. Checked interpretation/migration | Cross-path tests for unknown/conflicting ICC/CICP, odd YUV crop origins, 8/10/12/16-bit and supported float input, alpha, gain maps, orientation, exact/quantized duration and metadata privacy. Compile real consumers against each removal. | No silent relabeling, precision loss, timing loss or discarded required component. Optional failures are typed. No local path patch hides duplicate zencodec/zenpixels versions. |

Use the existing [harness](../perf/io-validation/README.md), testkits and codec
fixtures. Synthetic controls isolate mechanisms; native codecs and independent
readers qualify behavior. The simulator's calculated costs do not replace
transport measurements. Use allocator/profiler evidence for memory/copy claims;
set performance regression budgets after collecting reproducible baselines,
not from guessed percentages.

Do not schedule the native-codec tests as competing heavy jobs on the same
machine. These are separate experiments; they can share fixtures and run serially.

## Current PR evidence and release gate

Rechecked 2026-10-25: zenrav1e [44](https://github.com/imazen/zenrav1e/pull/44)
remains open at `9057c4ba`; zenpipe [86](https://github.com/imazen/zenpipe/pull/86)
at `330eea4b`; WebP [105](https://github.com/imazen/zenwebp/pull/105) at `6e94c8ec`;
JXL [20](https://github.com/imazen/zenjxl/pull/20) at `56184931`. The existing
[native qualification](../benchmarks/io_contracts_2026-10-24.md) covers the AV1
pin; it does not qualify the complete zenpipe network pipeline. Frame-at-a-time
animation is not proof of byte-incremental input. zenpixels [76](https://github.com/imazen/zenpixels/pull/76)
is merged; that alone does not prove that every consumer has migrated or that
its types are published.

The minimum useful next merge is the small input-admission review, with native
retention and collection experiments proceeding against private candidates.
Freeze the broader breaking signatures after those consumer paths exercise them.
A smaller driver repair release can ship independently; do not present it as
complete modern image/media coverage.

Before the coordinated release: review resolved public signatures, finish the
queued metadata/timing removals, compile the real dependency graph without
workspace overrides, check default/no_std/WASM and supported platform builds,
and run independent codec oracles. Keep optional transports, rich editors and
concrete codecs out of the core dependency graph. Publication remains a separate
approval. No new public library API has been implemented by this document.

## Reader check on the proposed boundary

The [sequential guide assessment](guide-learning.md) used two fresh Sol sessions,
four native coding tasks and two contract questions. Improving local context and
separating implemented JPEG policy from private adapter machinery changed
initial coding passes from 2/4 to 4/4. Both readers regarded config/job,
CodecSet and source/driver/sink separation as useful. This small model study
supports better entry points and disclosure, not removing those layers.

The remaining abstraction change is enforcement behind the operation boundary:
callers state requirements and a preset or fallible resolver; adapters construct
questions, validate choices, reserve resources and guard effects. Codec authors
supply qualified facts. Prepared data, engine construction and authorization
receipts should not become mandatory caller ceremony. A fixed ICC refusal still
has a simpler current public-plan implementation. Retire old coordination paths
only when the checked request replaces their enforcement across real routes.
Production streaming barriers and ownership qualification remain the gates above.

The [comparison with established codec interfaces](api-review-2026-10-25.md)
checks this boundary against explicit progress, buffer ownership, metadata
ordering and image-region contracts, and records the accounting-scope regression
found while reviewing the private resolver.

The [independent Argon review and audit](api-review-2026-10-25.md#independent-argon-review-and-disposition)
favor domain-specific offers and coherent transcode routes behind that same
consumer boundary. They do not justify a second application callback, a universal
codec error, or treating calibrated estimates as hard resource bounds. A common
item/level/region selector needs viewport qualification before adding more decode
trait families. These remain proposed refinements; the reproduced dyn encoder
default mismatch was fixed separately without changing public signatures.
